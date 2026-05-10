//! Bridge MQTT bidirectionnel Pi5 ↔ NanoPi.
//!
//! Reproduit exactement la section `connection nanopi-venus-bridge` de mosquitto.conf :
//!
//!   • `N/c0619ab9929a/#`               NanoPi → Pi5  (QoS 0)
//!   • `W/c0619ab9929a/#`               Pi5 → NanoPi  (QoS 1)
//!   • `R/c0619ab9929a/#`               Pi5 → NanoPi  (QoS 1)
//!   • `santuario/#`                    NanoPi → Pi5  (QoS 0)
//!   • `santuario/{heat,heatpump,...}/#` Pi5 → NanoPi  (QoS 0)
//!   • `shellypro2pm-ec62608840a4/#`    bidirectionnel (QoS 0)
//!
//! Métriques exposées sur http://127.0.0.1:8084/metrics (JSON).
//!
//! Usage : mqtt-bridge --config /etc/daly-bms/config.toml

mod bridge;
mod config;
mod metrics;

use anyhow::Result;
use axum::{routing::get, Json, Router};
use clap::Parser;
use metrics::BridgeMetrics;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tracing::info;

// =============================================================================
// CLI
// =============================================================================

#[derive(Parser)]
#[command(name = "mqtt-bridge", about = "Bridge MQTT Pi5 ↔ NanoPi pour DalyBMS")]
struct Cli {
    #[arg(long, default_value = "/etc/daly-bms/config.toml")]
    config: PathBuf,

    #[arg(long, default_value = "127.0.0.1:8084")]
    metrics_addr: SocketAddr,
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
                .unwrap_or_else(|_| "mqtt_bridge=info".into()),
        )
        .init();

    let cfg = config::load(&cli.config)?;
    info!(
        local  = %cfg.local_host,
        remote = %cfg.remote_host,
        "Bridge MQTT démarré"
    );

    let metrics = Arc::new(BridgeMetrics::new());

    // ── Serveur HTTP métriques ────────────────────────────────────────────────
    {
        let m = Arc::clone(&metrics);
        let app = Router::new()
            .route("/metrics", get(move || {
                let m2 = Arc::clone(&m);
                async move { Json(m2.snapshot()) }
            }))
            .route("/health", get(|| async { "ok" }));
        let addr = cli.metrics_addr;
        tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(addr).await
                .expect("Bind HTTP métriques bridge");
            info!("HTTP métriques bridge sur http://{addr}/metrics");
            axum::serve(listener, app).await.expect("Serveur métriques bridge");
        });
    }

    // ── Notification systemd ready ────────────────────────────────────────────
    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]);

    // ── Lancement des deux demi-bridges en parallèle ──────────────────────────
    let cfg = Arc::new(cfg);
    let (r1, r2) = tokio::join!(
        bridge::run_nanopi_to_local(Arc::clone(&cfg), Arc::clone(&metrics)),
        bridge::run_local_to_nanopi(Arc::clone(&cfg), Arc::clone(&metrics)),
    );
    r1?;
    r2?;

    Ok(())
}
