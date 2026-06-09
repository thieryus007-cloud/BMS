//! # daly-bms-server
//!
//! Serveur principal : charge la configuration, ouvre le port série,
//! démarre le polling et expose l'API Axum (REST + WebSocket).
//!
//! ## Démarrage
//! ```bash
//! DALY_CONFIG=/etc/daly-bms/config.toml daly-bms-server
//! ```

mod autodetect;
mod config;
mod ats;
mod console;
mod et112;
mod freshness;
mod irradiance;
mod shelly;
mod tasmota;
mod state;
mod api;
mod bridges;
mod dashboard;
mod dashboards;
mod monitor;
mod redb_writes;

// Allocator jemalloc à la place de glibc malloc.
// Cf. commit accompagnant : observation 18 mai 2026 d'une fragmentation
// heap qui faisait croître le RSS à chaque utilisation du dashboard
// historique (+15-20 Mo par burst, non libérés). malloc_trim manuel
// libérait 18 Mo confirmant le diagnostic. jemalloc rend les pages au
// kernel agressivement → RSS retombe au baseline après chaque burst.
#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use crate::bridges::{alerts, mqtt};
use crate::config::AppConfig;
use crate::state::{AppState, LogBuffer, LogEntry};
use daly_bms_core::bus::{BmsConfig, DalyBusManager, DalyPort};
use daly_bms_core::poll::{poll_loop, PollConfig, PollErrorKind};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use tracing::{error, info, warn};
use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_appender::rolling;

// =============================================================================
// Supervision des tâches critiques (audit robustesse §2)
// =============================================================================

/// Lance une tâche critique longue durée. Si son future se termine (retour) —
/// ce qui ne doit jamais arriver pour une boucle de polling/bridge — on log une
/// erreur fatale et on arrête le process pour forcer un redémarrage propre par
/// systemd (`Restart=on-failure`). Les panics sont déjà fatals via
/// `panic = "abort"` (profil release) ; ce helper couvre les retours propres.
///
/// NB : à n'utiliser QUE pour les boucles externes longue durée — jamais pour
/// les tâches transitoires par-snapshot (qui se terminent normalement).
fn spawn_critical<F>(fut: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::task::spawn(async move {
        fut.await;
        error!(
            "Une tâche critique s'est terminée de façon inattendue — \
             arrêt du process pour redémarrage par systemd"
        );
        // Laisse le temps aux logs de partir avant l'exit.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        std::process::exit(1);
    });
}

// =============================================================================
// Couche tracing → buffer web
// =============================================================================

struct WebLogLayer {
    buffer: LogBuffer,
}

struct MsgVisitor(String);

impl tracing::field::Visit for MsgVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" { self.0 = value.to_string(); }
        else if !self.0.is_empty()   { self.0.push_str(&format!(" {}={}", field.name(), value)); }
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" { self.0 = format!("{:?}", value).trim_matches('"').to_string(); }
        else if !self.0.is_empty()   { self.0.push_str(&format!(" {}={:?}", field.name(), value)); }
    }
}

impl<S: Subscriber> Layer<S> for WebLogLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let level = event.metadata().level().to_string().to_uppercase();
        let mut v = MsgVisitor(String::new());
        event.record(&mut v);
        let entry = LogEntry {
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            level,
            message: v.0,
        };
        if let Ok(mut buf) = self.buffer.lock() {
            if buf.len() >= 200 { buf.pop_front(); }
            buf.push_back(entry);
        }
    }
}

// =============================================================================
// Arguments CLI du serveur
// =============================================================================

#[derive(Debug)]
struct ServerArgs {
    /// Port série explicite (ex: COM3, /dev/ttyUSB0)
    port:      Option<String>,
    /// Adresses BMS pour le mode hardware (ex: 0x01,0x02)
    bms_addrs: Vec<u8>,
    /// `--check-config` : valide la config (parse + bornes) puis sort —
    /// dry-run pour CI/déploiement (audit 2026-06 §12).
    check_config: bool,
}

impl ServerArgs {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().collect();

        let port = args.windows(2)
            .find(|w| w[0] == "--port" || w[0] == "-p")
            .map(|w| w[1].clone());

        let bms_addrs = args.windows(2)
            .find(|w| w[0] == "--bms")
            .map(|w| Self::parse_addresses(&w[1]))
            .unwrap_or_default();

        let check_config = args.iter().any(|a| a == "--check-config");

        Self { port, bms_addrs, check_config }
    }

    fn parse_addresses(s: &str) -> Vec<u8> {
        s.split(',')
            .filter_map(|s| {
                let s = s.trim();
                if s.starts_with("0x") || s.starts_with("0X") {
                    u8::from_str_radix(&s[2..], 16).ok()
                } else {
                    s.parse::<u8>().ok()
                }
            })
            .collect()
    }
}

// =============================================================================
// Log cleanup
// =============================================================================

/// Supprime les fichiers de log plus vieux que `keep_days` jours dans `dir`.
fn cleanup_old_logs(dir: &str, keep_days: u64) {
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(keep_days * 24 * 3600);
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "log")
            || path.to_string_lossy().contains("daly-bms.log")
        {
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    if modified < cutoff {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }
}

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = ServerArgs::parse();

    // ── Configuration ──────────────────────────────────────────────────────────
    let config_from_file;
    let mut config = match AppConfig::load_default() {
        Ok(c) => {
            config_from_file = true;
            c
        }
        Err(e) => {
            eprintln!("Config non trouvée ({}) — utilisation des valeurs par défaut", e);
            config_from_file = false;
            AppConfig {
                serial:      config::SerialConfig::default(),
                api:         config::ApiConfig::default(),
                logging:     config::LoggingConfig::default(),
                mqtt:        config::MqttConfig::default(),
                metrics_store: config::MetricsStoreConfig::default(),
                alerts:      config::AlertsConfig::default(),
                read_only:   config::ReadOnlyConfig::default(),
                bms:         Vec::new(),
                et112:       config::Et112Config::default(),
                irradiance:  None,
                ats:         None,
                tasmota:     config::TasmotaConfig::default(),
                shelly:      config::ShellyConfig::default(),
            }
        }
    };

    // ── Override port série depuis CLI ─────────────────────────────────────────
    if let Some(ref port) = args.port {
        config.serial.port = port.clone();
    }
    // Override adresses BMS hardware depuis CLI
    if !args.bms_addrs.is_empty() {
        config.serial.addresses = args.bms_addrs.iter()
            .map(|a| format!("{:#04x}", a))
            .collect();
    }

    // ── Validation des bornes (audit 2026-06 §12) ─────────────────────────────
    // Échec = message nommant le champ fautif, avant tout démarrage de tâche.
    // Ne s'applique qu'à une config chargée depuis un fichier : les valeurs
    // par défaut internes sont saines par construction.
    if config_from_file {
        config.validate()?;
    }

    // `--check-config` : dry-run (parse + validation) pour CI/déploiement.
    if args.check_config {
        if config_from_file {
            println!("Config OK");
            return Ok(());
        }
        anyhow::bail!("--check-config : aucun fichier de config trouvé");
    }

    // ── Flags d'auto-détection ────────────────────────────────────────────────
    // Actifs uniquement si aucun fichier config ET aucun argument CLI fourni.
    let auto_detect_port    = args.port.is_none()      && (!config_from_file || config.serial.port.is_empty());
    let auto_discover_addrs = args.bms_addrs.is_empty() && !config_from_file;

    // ── Logging ────────────────────────────────────────────────────────────────
    let log_level  = config.logging.level.clone();
    let log_dir    = config.logging.log_dir.clone();
    let keep_days  = config.logging.log_keep_days;
    let log_buffer: LogBuffer = Arc::new(Mutex::new(VecDeque::new()));

    // Rolling file appender (daily, kept for `keep_days` days).
    // The guard must live for the duration of the program to flush pending writes.
    let _file_guard = if !log_dir.is_empty() {
        match std::fs::create_dir_all(&log_dir) {
            Ok(_) => {
                // Clean up log files older than keep_days before starting.
                cleanup_old_logs(&log_dir, keep_days);
                let file_appender = rolling::daily(&log_dir, "daly-bms.log");
                let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
                let file_layer = tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(non_blocking);
                tracing_subscriber::registry()
                    .with(
                        tracing_subscriber::EnvFilter::try_from_default_env()
                            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&log_level)),
                    )
                    .with(tracing_subscriber::fmt::layer())
                    .with(file_layer)
                    .with(WebLogLayer { buffer: log_buffer.clone() })
                    .init();
                Some(guard)
            }
            Err(e) => {
                // Fall back to console-only logging if the log dir can't be created.
                tracing_subscriber::registry()
                    .with(
                        tracing_subscriber::EnvFilter::try_from_default_env()
                            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&log_level)),
                    )
                    .with(tracing_subscriber::fmt::layer())
                    .with(WebLogLayer { buffer: log_buffer.clone() })
                    .init();
                eprintln!("Warning: cannot create log dir {log_dir}: {e} — file logging disabled");
                None
            }
        }
    } else {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&log_level)),
            )
            .with(tracing_subscriber::fmt::layer())
            .with(WebLogLayer { buffer: log_buffer.clone() })
            .init();
        None
    };

    info!(
        version = env!("CARGO_PKG_VERSION"),
        api = %config.api.bind,
        "DalyBMS Server démarrage"
    );

    // ── AlertEngine (SQLite + règles ESS) ─────────────────────────────────────
    let alert_engine = if !config.alerts.db_path.is_empty() {
        // Créer le répertoire parent si nécessaire
        if let Some(parent) = std::path::Path::new(&config.alerts.db_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match alerts::AlertEngine::new(config.alerts.clone()) {
            Ok(e) => {
                info!(db = %config.alerts.db_path, "AlertEngine initialisé");
                Some(e)
            }
            Err(e) => {
                warn!("AlertEngine init échoué : {} — alertes désactivées", e);
                None
            }
        }
    } else {
        info!("AlertEngine désactivé (alerts.db_path vide)");
        None
    };

    // ── metrics-store redb (seule TSDB, cf. `docs/metriques-redb-architecture.md`) ──
    let metrics_store = if config.metrics_store.enabled {
        let opts = metrics_store::Options {
            cache_bytes: config.metrics_store.cache_mb * 1024 * 1024,
            writer_queue_depth: config.metrics_store.queue_depth,
            writer: metrics_store::WriterConfig::default(),
        };
        match open_metrics_store_with_quarantine(&config.metrics_store.db_path, opts) {
            Ok(s) => {
                info!(
                    db_path = %config.metrics_store.db_path,
                    "metrics-store ouvert (écriture redb activée)"
                );
                if config.metrics_store.maintenance_interval_hours > 0 {
                    let policy = metrics_store::TierPolicy {
                        raw_retention_days: config.metrics_store.raw_retention_days,
                        hourly_retention_days: config.metrics_store.hourly_retention_days,
                        daily_retention_days: config.metrics_store.daily_retention_days,
                    };
                    // Le JoinHandle est conservé (binding nommé) : le dropper
                    // détacherait la tâche sans l'annuler, mais on évite ainsi
                    // le lint let_underscore_future et on garde la poignée.
                    let _maintenance_handle = s.spawn_maintenance(
                        policy,
                        config.metrics_store.maintenance_interval_hours,
                    );
                    info!(
                        interval_h = config.metrics_store.maintenance_interval_hours,
                        "metrics-store: maintenance tiered planifiée"
                    );
                }
                Some(s)
            }
            Err(e) => {
                warn!("metrics-store ouverture échouée : {e} — écriture redb désactivée");
                None
            }
        }
    } else {
        None
    };

    // ── État partagé ───────────────────────────────────────────────────────────
    // metrics-store (redb) est la seule TSDB (lecture + écriture via le Writer du shim).
    let metrics_store_arc = metrics_store.as_ref().map(|s| std::sync::Arc::new(s.clone()));
    let state = AppState::new(
        config.clone(),
        log_buffer,
        alert_engine.clone(),
        metrics_store_arc,
    );

    // ── Bridges en arrière-plan ─────────────────────────────────────────────────

    // Map adresse RS485 → index topic MQTT (ex: 0x28 → "1", 0x29 → "2")
    // Permet d'écrire santuario/bms/1/venus au lieu de santuario/bms/40/venus
    let mqtt_addr_map: std::collections::HashMap<u8, String> = config.bms
        .iter()
        .enumerate()
        .filter_map(|(i, bms_cfg)| {
            let addr = bms_cfg.parsed_address()?;
            let idx  = bms_cfg.mqtt_index.unwrap_or((i + 1) as u8);
            Some((addr, idx.to_string()))
        })
        .collect();

    if config.mqtt.enabled {
        spawn_critical({
            let (s, c, m) = (state.clone(), config.mqtt.clone(), mqtt_addr_map);
            async move { mqtt::run_mqtt_bridge(s, c, m).await }
        });
    }
    if let Some(ref engine) = alert_engine {
        spawn_critical({
            let (s, e) = (state.clone(), engine.clone());
            async move { alerts::run_alert_engine(s, e).await }
        });
    }

    // ── Venus OS MQTT subscriber (données D-Bus) ───────────────────────────────
    if config.mqtt.enabled {
        info!("Démarrage Venus OS MQTT subscriber");
        let state_venus = state.clone();
        let mqtt_cfg = config.mqtt.clone();
        spawn_critical(async move {
            mqtt::start_venus_mqtt_subscriber(state_venus, mqtt_cfg).await
        });
    }

    // ── Tasmota MQTT subscriber ────────────────────────────────────────────────
    if !config.tasmota.devices.is_empty() {
        info!(
            count = config.tasmota.devices.len(),
            "Démarrage Tasmota MQTT subscriber"
        );
        let state_ta = state.clone();
        let devs_ta  = config.tasmota.devices.clone();
        let mqtt_ta  = config.mqtt.clone();
        spawn_critical(async move {
            tasmota::run_tasmota_mqtt_loop(
                devs_ta,
                mqtt_ta,
                move |snap| {
                    let s = state_ta.clone();
                    tokio::spawn(async move { s.on_tasmota_snapshot(snap).await });
                },
            )
            .await;
        });
    }

    // ── Shelly Pro 2PM MQTT subscriber ────────────────────────────────────────
    if !config.shelly.devices.is_empty() {
        info!(
            count = config.shelly.devices.len(),
            "Démarrage Shelly Pro 2PM MQTT subscriber"
        );
        let state_sh  = state.clone();
        let devs_sh   = config.shelly.devices.clone();
        let mqtt_sh   = config.mqtt.clone();
        let client_sh = state.shelly_client.clone();
        spawn_critical(async move {
            shelly::run_shelly_mqtt_loop(
                devs_sh,
                mqtt_sh,
                client_sh,
                move |snap| {
                    let s = state_sh.clone();
                    tokio::spawn(async move { s.on_shelly_snapshot(snap).await });
                },
            )
            .await;
        });
    }

    // ── Mode HARDWARE ──────────────────────────────────────────────────────────
    // TEMPORAIRE — audit fuite mémoire phase 2.9 : flag DALY_DISABLE_RS485 pour
    // bypasser TOUT le polling RS485 (BMS + ET112 + ATS + irradiance). Permet
    // d'isoler si la fuite vient des poll loops ou du runtime tokio/hyper.
    if std::env::var("DALY_DISABLE_RS485").is_ok() {
        tracing::warn!("Polling RS485 désactivé via DALY_DISABLE_RS485 (debug fuite mémoire)");
    } else {
        // ── 1. Résoudre le port (auto-détection ou config) ───────────────────
        // find_daly_port() retourne le port déjà ouvert pour éviter la double
        // ouverture Windows ("Accès refusé" si on ouvre, ferme, puis rouvre).
        let (resolved_port, pre_opened_port) = if auto_detect_port {
            info!("Port non spécifié — détection automatique en cours...");
            match autodetect::find_daly_port(config.serial.baud).await {
                Some((name, port)) => (name, Some(port)),
                None => {
                    error!("Aucun Daly BMS détecté sur les ports série disponibles.");
                    warn!("Relancez avec --port COMx pour forcer un port.");
                    warn!("Démarrage en mode API-seule (pas de données BMS).");
                    (String::new(), None)
                }
            }
        } else {
            (config.serial.port.clone(), None)
        };

        if !resolved_port.is_empty() {
            // Réutiliser le port pré-ouvert (auto-détect) ou en ouvrir un nouveau
            let dal_port = match pre_opened_port {
                Some(p) => Ok(p),
                None    => DalyPort::open(&resolved_port, config.serial.baud, 500),
            };
            match dal_port {
                Ok(port) => {
                    info!("Port série {} ouvert à {} baud — bus RS485 unifié", resolved_port, config.serial.baud);
                    state.polling_active.store(true, Ordering::Relaxed);

                    // Rendre le port disponible pour les commandes d'écriture
                    state.set_port(port.clone()).await;

                    // ── Bus RS485 partagé avec ET112 et PRALRAN ────────────────
                    let shared_bus = port.shared_bus();

                    // ── ET112 sur bus unifié ───────────────────────────────────
                    if !config.et112.devices.is_empty() {
                        info!(
                            count = config.et112.devices.len(),
                            "Démarrage polling ET112 (bus RS485 unifié)"
                        );
                        let state_et     = state.clone();
                        let state_et_err = state.clone();
                        let bus_et    = shared_bus.clone();
                        let et112_cfg = config.et112.clone();
                        spawn_critical(async move {
                            et112::run_et112_poll_loop(
                                bus_et,
                                et112_cfg.devices,
                                std::time::Duration::from_millis(et112_cfg.poll_interval_ms),
                                move |snap| {
                                    let s = state_et.clone();
                                    tokio::spawn(async move { s.on_et112_snapshot(snap).await });
                                },
                                move |addr, name, res| {
                                    let s = state_et_err.clone();
                                    let name = name.to_string();
                                    tokio::spawn(async move {
                                        match res {
                                            Ok(()) => s.record_rs485_success(addr, "ET112", &name).await,
                                            Err(msg) => s.record_rs485_error(addr, "ET112", &name, &msg).await,
                                        }
                                    });
                                },
                            )
                            .await;
                        });
                    }

                    // ── PRALRAN irradiance sur bus unifié ──────────────────────
                    if let Some(irrad_cfg) = config.irradiance.clone() {
                        info!(
                            addr = %irrad_cfg.address,
                            "Démarrage polling irradiance PRALRAN (bus RS485 unifié)"
                        );
                        let state_irrad     = state.clone();
                        let state_irrad_err = state.clone();
                        let bus_irrad   = shared_bus.clone();
                        spawn_critical(async move {
                            irradiance::run_irradiance_poll_loop(
                                bus_irrad,
                                irrad_cfg,
                                move |snap| {
                                    let s = state_irrad.clone();
                                    tokio::spawn(async move { s.on_irradiance_snapshot(snap).await });
                                },
                                move |addr, name, res| {
                                    let s = state_irrad_err.clone();
                                    let name = name.to_string();
                                    tokio::spawn(async move {
                                        match res {
                                            Ok(()) => s.record_rs485_success(addr, "PRALRAN", &name).await,
                                            Err(msg) => s.record_rs485_error(addr, "PRALRAN", &name, &msg).await,
                                        }
                                    });
                                },
                            )
                            .await;
                        });
                    }

                    // ── ATS CHINT sur bus unifié ──────────────────────────────────
                    if let Some(ats_cfg) = config.ats.clone() {
                        if ats_cfg.enabled {
                            info!(
                                addr = ats_cfg.address,
                                name = %ats_cfg.name,
                                "Démarrage polling ATS CHINT (bus RS485 unifié)"
                            );
                            let state_ats     = state.clone();
                            let state_ats_err = state.clone();
                            let bus_ats   = shared_bus.clone();
                            state.set_ats_bus(shared_bus.clone()).await;
                            spawn_critical(async move {
                                ats::run_ats_poll_loop(
                                    bus_ats,
                                    ats_cfg,
                                    move |snap| {
                                        let s = state_ats.clone();
                                        tokio::spawn(async move { s.on_ats_snapshot(snap).await });
                                    },
                                    move |addr, name, res| {
                                        let s = state_ats_err.clone();
                                        let name = name.to_string();
                                        tokio::spawn(async move {
                                            match res {
                                                Ok(()) => s.record_rs485_success(addr, "ATS", &name).await,
                                                Err(msg) => s.record_rs485_error(addr, "ATS", &name, &msg).await,
                                            }
                                        });
                                    },
                                )
                                .await;
                            });
                        }
                    }

                    // ── 2. Résoudre les adresses BMS (auto-découverte ou config) ──
                    let addresses = if auto_discover_addrs {
                        info!("Découverte automatique des BMS sur le bus RS485 (0x01..0x04)...");
                        let mgr_tmp = DalyBusManager::new(port.clone(), vec![]);
                        let found = mgr_tmp.discover(1, 4).await;
                        if found.is_empty() {
                            warn!("Aucun BMS découvert — polling désactivé.");
                            state.polling_active.store(false, Ordering::Relaxed);
                        }
                        found
                    } else {
                        config.bms_addresses()
                    };

                    if !addresses.is_empty() {
                        let devices: Vec<BmsConfig> = addresses
                            .iter()
                            .map(|&addr| {
                                let mut bms = BmsConfig::new(addr);
                                bms.cell_count        = config.serial.default_cell_count;
                                bms.temp_sensor_count = config.serial.default_temp_sensors;
                                // Surcharges depuis [[bms]] si présentes
                                if let Some(dev_cfg) = config.bms.iter()
                                    .find(|b| b.parsed_address() == Some(addr))
                                {
                                    if let Some(n)  = &dev_cfg.name           { bms.name = n.clone(); }
                                    if let Some(c)  = dev_cfg.capacity_ah      { bms.installed_capacity_ah    = c; }
                                    if let Some(mc) = dev_cfg.max_charge_a     { bms.max_charge_current_a    = mc; }
                                    if let Some(md) = dev_cfg.max_discharge_a  { bms.max_discharge_current_a = md; }
                                }
                                bms
                            })
                            .collect();

                        info!("Polling de {} BMS : {:?}", devices.len(),
                              devices.iter().map(|d| format!("{:#04x}", d.address)).collect::<Vec<_>>());

                        // Map adresse → nom pour enrichir les stats RS485.
                        let bms_names: std::collections::BTreeMap<u8, String> = devices
                            .iter()
                            .map(|d| (d.address, d.name.clone()))
                            .collect();

                        let manager  = Arc::new(DalyBusManager::new(port, devices));
                        let poll_cfg = PollConfig {
                            interval_ms: config.serial.poll_interval_ms,
                            ..Default::default()
                        };
                        let state_poll = state.clone();
                        let state_err  = state.clone();

                        // Channels MPSC pour découpler les callbacks sync de poll_loop
                        // des appels async sur AppState. Évite N × `tokio::spawn` par
                        // snapshot (qui alloue ~500 bytes par task) qui s'accumulent
                        // sous rétention jemalloc — audit fuite mémoire phase 2.6.
                        // Cap 64 = ~10 s de buffer à 6 Hz (3 BMS × 2 Hz).
                        let (snap_tx, mut snap_rx) =
                            tokio::sync::mpsc::channel::<daly_bms_core::types::BmsSnapshot>(64);
                        let (err_tx, mut err_rx) =
                            tokio::sync::mpsc::channel::<(u8, String, String)>(64);

                        // Consumer task : await sur les 2 channels et appelle state.
                        let state_consumer = state.clone();
                        spawn_critical(async move {
                            loop {
                                tokio::select! {
                                    Some(snap) = snap_rx.recv() => {
                                        let addr = snap.address;
                                        let name = snap.name.clone();
                                        state_consumer.record_rs485_success(addr, "BMS", &name).await;
                                        state_consumer.on_snapshot(snap).await;
                                    }
                                    Some((addr, name, err_msg)) = err_rx.recv() => {
                                        state_consumer.record_rs485_error(addr, "BMS", &name, &err_msg).await;
                                    }
                                    else => break,
                                }
                            }
                        });

                        let _ = state_poll;
                        let _ = state_err;

                        spawn_critical(async move {
                            poll_loop(
                                manager,
                                poll_cfg,
                                move |snap| {
                                    // try_send : non-bloquant, drop si plein.
                                    // Pas d'allocation supplémentaire (slot pré-alloué).
                                    let _ = snap_tx.try_send(snap);
                                },
                                move |addr, kind, msg| {
                                    let name = bms_names
                                        .get(&addr)
                                        .cloned()
                                        .unwrap_or_else(|| format!("BMS {:#04x}", addr));
                                    let err_tag = match kind {
                                        PollErrorKind::Timeout => "timeout",
                                        PollErrorKind::Crc     => "crc",
                                        PollErrorKind::Serial  => "timeout",
                                        PollErrorKind::Other   => "other",
                                    };
                                    let err_msg = format!("{}: {}", err_tag, msg);
                                    let _ = err_tx.try_send((addr, name, err_msg));
                                },
                            )
                            .await;
                        });
                    }
                }
                Err(e) => {
                    error!("Impossible d'ouvrir {} : {:?}", resolved_port, e);
                    warn!("Démarrage en mode API-seule (pas de données BMS).");
                }
            }
        }
    }

    // ── Agent de monitoring Pi5 (+ watchdog sd_notify + métriques tokio→VM) ──
    // TEMPORAIRE — audit fuite mémoire phase 2.8 : commenter pour isoler la
    // contribution de monitor_agent (fork ps/systemctl/df toutes les 30s) et
    // watchdog_agent (TCP probes toutes les 15s). À restaurer après diagnostic.
    if std::env::var("DALY_DISABLE_MONITOR").is_err() {
        monitor::spawn_all(state.clone());
    } else {
        tracing::warn!("Monitor + watchdog désactivés via DALY_DISABLE_MONITOR (debug fuite mémoire)");
    }

    // ── Export de la fraîcheur des sources (audit 2026-06 §18) ────────────────
    // Toutes les 30 s : âge de la dernière donnée par source →
    // `source_last_update_age_seconds{source=...}` dans metrics-store.
    // Transforme les pannes silencieuses (« la donnée ne se rafraîchit
    // plus ») en signal alertable (âge > N × intervalle de polling).
    if let Some(store) = &state.metrics_store {
        let writer = store.writer();
        let freshness_state = state.clone();
        spawn_critical(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                ticker.tick().await;
                let ts = chrono::Utc::now().timestamp_millis();
                for (source, age) in freshness_state.freshness.ages_seconds() {
                    let sample =
                        metrics_store::Sample::new("source_last_update_age_seconds", ts, age)
                            .with_label("source", &source);
                    // try_write : jamais bloquant ; pendant l'arrêt gracieux
                    // le canal est fermé → échec silencieux attendu.
                    let _ = writer.try_write(sample);
                }
            }
        });
    }

    // ── Serveur HTTP Axum ──────────────────────────────────────────────────────
    let shutdown_state = state.clone();
    let router = api::build_router(state);
    let addr: SocketAddr = config.api.bind.parse()?;

    info!("API  → http://{}", addr);
    info!("WS   → ws://{}/ws/bms/stream", addr);

    // Notifie systemd que le service est prêt (Type=notify)
    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    // ── Arrêt gracieux SIGTERM/SIGINT (audit 2026-06 §5) ──────────────────────
    // Le service redémarre tous les jours (RuntimeMaxSec) : sans arrêt propre,
    // le batch metrics en file (fenêtre 250 ms + backlog) était perdu à chaque
    // restart. Drain HTTP borné à 10 s : des WebSockets ouvertes ne doivent
    // pas suspendre l'arrêt jusqu'au SIGKILL systemd (TimeoutStopSec 90 s).
    let (shut_tx, shut_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        let _ = shut_tx.send(true);
    });
    let mut graceful_rx = shut_rx.clone();
    let mut deadline_rx = shut_rx;
    let serve = axum::serve(listener, router).with_graceful_shutdown(async move {
        let _ = graceful_rx.changed().await;
    });
    tokio::select! {
        res = serve => { res?; }
        _ = async {
            let _ = deadline_rx.changed().await;
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        } => {
            warn!("Arrêt : connexions HTTP/WS encore ouvertes après 10 s — fermeture forcée");
        }
    }

    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Stopping]);
    if let Some(store) = &shutdown_state.metrics_store {
        info!("Arrêt gracieux : flush du batch metrics-store…");
        store.shutdown();
    }
    info!("Arrêt gracieux terminé");
    Ok(())
}

/// Ouvre la base metrics-store ; si l'ouverture échoue sur une **corruption
/// avérée** (coupure brutale pendant un fsync, bitflip…), met le fichier en
/// quarantaine (`<db>.corrupt.<timestamp>`) puis recrée une base vide
/// (audit 2026-06 §15). Avant : le service tournait sans historique jusqu'à
/// intervention manuelle. Le fichier en quarantaine est conservé pour
/// autopsie/récupération — jamais supprimé automatiquement.
///
/// Les échecs NON liés à une corruption (verrou « Database already open »
/// d'une seconde instance, permissions, disque plein) ne déclenchent JAMAIS
/// la quarantaine : on ne déplace pas une base saine.
fn open_metrics_store_with_quarantine(
    db_path: &str,
    opts: metrics_store::Options,
) -> anyhow::Result<metrics_store::MetricsStore> {
    let path = std::path::Path::new(db_path);
    let err = match metrics_store::MetricsStore::open(path, opts.clone()) {
        Ok(s) => return Ok(s),
        Err(e) => e,
    };
    if !path.exists() || !metrics_store::MetricsStore::open_error_is_corruption(&err) {
        return Err(err);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let quarantine = format!("{db_path}.corrupt.{ts}");
    error!(
        "metrics-store : base corrompue ({err:#}) — quarantaine vers {quarantine} \
         puis recréation d'une base vide"
    );
    std::fs::rename(path, &quarantine)
        .map_err(|re| anyhow::anyhow!("quarantaine impossible ({re}) ; erreur d'origine : {err:#}"))?;
    let store = metrics_store::MetricsStore::open(path, opts)?;
    warn!(
        "metrics-store recréé vide — l'historique corrompu est conservé dans {quarantine} \
         (autopsie : docs/metriques-redb-architecture.md)"
    );
    Ok(store)
}

/// Résout au premier SIGTERM (systemd stop/restart) ou SIGINT (Ctrl-C).
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::select! {
                _ = term.recv() => {}
                r = tokio::signal::ctrl_c() => { let _ = r; }
            }
        }
        Err(e) => {
            // Improbable ; on retombe sur SIGINT seul plutôt que d'échouer.
            warn!("Inscription SIGTERM impossible ({e}) — arrêt gracieux sur SIGINT uniquement");
            let _ = tokio::signal::ctrl_c().await;
        }
    }
    info!("Signal d'arrêt reçu — fermeture gracieuse en cours");
}
