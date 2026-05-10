//! Broker MQTT Rust — remplace Mosquitto/Docker sur Pi5.
//!
//! Basé sur rumqttd (même famille que rumqttc déjà utilisé dans le workspace).
//! Fonctionnalités préservées vs Mosquitto :
//!   - MQTT 3.1.1, QoS 0/1/2, retained messages, persistence disque
//!   - Listener TCP  :1883
//!   - Listener WS   :9001 (WebSocket MQTT pour dashboard JS)
//!   - Anonymous (réseau local)
//!   - Messages jusqu'à 1 MB
//!
//! Métriques exposées sur http://127.0.0.1:8082/metrics (JSON) pour le dashboard.
//!
//! Usage : mqtt-broker --config /etc/daly-bms/mqtt-broker.toml

use anyhow::{Context, Result};
use axum::{routing::get, Json, Router};
use chrono::Utc;
use clap::Parser;
use rumqttd::{Broker, Config};
use serde::Serialize;
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};
use tracing::{info, warn};

// =============================================================================
// CLI
// =============================================================================

#[derive(Parser)]
#[command(name = "mqtt-broker", about = "Broker MQTT Rust (rumqttd) pour DalyBMS")]
struct Cli {
    #[arg(long, default_value = "/etc/daly-bms/mqtt-broker.toml")]
    config: PathBuf,

    #[arg(long, default_value = "127.0.0.1:8082")]
    metrics_addr: SocketAddr,
}

// =============================================================================
// Métriques partagées
// =============================================================================

#[derive(Default)]
struct BrokerMetrics {
    messages_rx:  AtomicU64,
    messages_tx:  AtomicU64,
    connections:  AtomicU64,
    start_epoch:  AtomicU64,
}

#[derive(Serialize)]
struct MetricsResponse {
    status:       &'static str,
    uptime_secs:  u64,
    messages_rx:  u64,
    messages_tx:  u64,
    connections:  u64,
    timestamp:    String,
}

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mqtt_broker=info,rumqttd=info".into()),
        )
        .init();

    // ── Chargement de la config ───────────────────────────────────────────────
    let config_str = std::fs::read_to_string(&cli.config)
        .with_context(|| format!("Lecture config : {}", cli.config.display()))?;
    let config: Config = toml::from_str(&config_str)
        .with_context(|| format!("Parse config TOML : {}", cli.config.display()))?;

    info!("Broker MQTT démarré — TCP :1883, WS :9001");
    info!("Persistence : {}", config.router.dir.display());

    // ── Métriques partagées ───────────────────────────────────────────────────
    let metrics = Arc::new(BrokerMetrics::default());
    metrics.start_epoch.store(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        Ordering::Relaxed,
    );

    // ── Broker rumqttd ────────────────────────────────────────────────────────
    let mut broker = Broker::new(config);

    // Lien "monitor" pour compter les messages en transit
    let (mut link_tx, mut link_rx) = broker
        .link("__metrics__")
        .context("Création du lien monitor")?;

    // Abonnement wildcard pour compter tous les messages
    link_tx.subscribe("#").context("Subscribe #")?;

    // link_rx.recv() est synchrone → spawn_blocking pour ne pas bloquer le runtime
    let metrics_clone = Arc::clone(&metrics);
    tokio::task::spawn_blocking(move || {
        loop {
            match link_rx.recv() {
                Ok(Some(_notif)) => {
                    metrics_clone.messages_rx.fetch_add(1, Ordering::Relaxed);
                }
                Ok(None) | Err(_) => break,
            }
        }
    });

    // ── Serveur HTTP métriques ────────────────────────────────────────────────
    let metrics_http = Arc::clone(&metrics);
    let start_time = Instant::now();

    let app = Router::new().route(
        "/metrics",
        get(move || {
            let m = Arc::clone(&metrics_http);
            let elapsed = start_time.elapsed().as_secs();
            async move {
                Json(MetricsResponse {
                    status:      "running",
                    uptime_secs: elapsed,
                    messages_rx: m.messages_rx.load(Ordering::Relaxed),
                    messages_tx: m.messages_tx.load(Ordering::Relaxed),
                    connections: m.connections.load(Ordering::Relaxed),
                    timestamp:   Utc::now().to_rfc3339(),
                })
            }
        }),
    )
    .route("/health", get(|| async { "ok" }));

    let metrics_addr = cli.metrics_addr;
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(metrics_addr)
            .await
            .expect("Bind HTTP métriques");
        info!("HTTP métriques sur http://{metrics_addr}/metrics");
        axum::serve(listener, app).await.expect("Serveur métriques");
    });

    // ── Notification systemd ready ────────────────────────────────────────────
    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]);

    // ── Démarrage broker (bloquant) ───────────────────────────────────────────
    tokio::task::spawn_blocking(move || {
        broker.start().expect("Erreur fatale broker MQTT");
    })
    .await
    .context("Broker MQTT thread")?;

    Ok(())
}
