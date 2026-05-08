use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod bus;
mod config;
mod http_clients;
mod live_ws;
mod logic;
mod monitoring;
mod mqtt;
mod persist;
mod types;

use bus::AppBus;
use types::EnergyState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // --- Logging ---
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    info!("energy-manager starting");

    // --- Config ---
    let cfg = config::load()?;
    info!("Config loaded — portal_id={}, mqtt={}:{}",
        cfg.victron.portal_id, cfg.mqtt.host, cfg.mqtt.port);

    // --- Shared state ---
    let state: Arc<RwLock<EnergyState>> = Arc::new(RwLock::new(EnergyState::default()));

    // --- Bus ---
    let (bus, receivers) = AppBus::new();

    // --- MQTT topics ---
    let topics = mqtt::topics::all_subscriptions(
        &cfg.victron.portal_id,
        cfg.victron.vebus_instance,
        cfg.victron.mppt1_instance,
        cfg.victron.mppt2_instance,
        cfg.victron.pvinverter_instance,
        cfg.victron.smartshunt_instance,
    );

    // --- Spawn MQTT client ---
    mqtt::client::spawn(&cfg.mqtt, topics, bus.clone(), receivers.mqtt_out_rx).await?;

    // --- Spawn persist MQTT watcher (retained topics at startup) ---
    spawn_persist_watcher(bus.clone(), state.clone());

    // --- LG ThinQ client ---
    let lg_client = http_clients::lg_thinq::spawn_poller(
        cfg.lg_thinq.clone(),
        bus.clone(),
        state.clone(),
    ).await;
    let lg_arc = lg_client.map(Arc::new);

    // --- Open-Meteo ---
    http_clients::open_meteo::spawn(
        cfg.open_meteo.clone(),
        bus.clone(),
        state.clone(),
    ).await;

    // --- Logic modules ---
    let vic = Arc::new(cfg.victron.clone());

    logic::inverter::spawn(vic.clone(), bus.clone(), state.clone()).await;
    logic::smartshunt::spawn(vic.clone(), bus.clone(), state.clone()).await;
    logic::irradiance::spawn(bus.clone(), state.clone(), cfg.solar.bms_server_url.clone()).await;
    logic::tasmota::spawn(vic.clone(), bus.clone(), state.clone()).await;
    logic::switch_ats::spawn(bus.clone(), state.clone()).await;
    logic::platform::spawn(cfg.platform.clone(), bus.clone()).await;
    logic::charge_current::spawn(vic.clone(), cfg.charge_current.clone(), bus.clone(), state.clone()).await;
    logic::solar_power::spawn(vic.clone(), cfg.solar.clone(), bus.clone(), state.clone()).await;
    logic::deye_command::spawn(vic.clone(), cfg.deye.clone(), bus.clone(), state.clone()).await;
    logic::water_heater::spawn(cfg.water_heater.clone(), lg_arc.clone(), bus.clone(), state.clone()).await;
    logic::meteo::spawn(cfg.solar.clone(), bus.clone(), state.clone()).await;
    logic::victron_keepalive::spawn(cfg.victron.portal_id.clone(), bus.clone()).await;

    // --- Live WebSocket + REST server ---
    let bind     = cfg.api.bind.clone();
    let live_tx  = bus.live.clone();
    let srv_state = state.clone();
    let srv_lg    = lg_arc.clone();
    tokio::spawn(async move {
        live_ws::server::serve(&bind, live_tx, srv_state, srv_lg).await;
    });

    // --- Module monitoring Pi5 (métriques système + tokio → VictoriaMetrics) ---
    monitoring::spawn(cfg.water_heater.vm_url.clone());

    info!("energy-manager fully started");

    // Notifie systemd que le service est prêt (Type=notify)
    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]);

    // Heartbeat watchdog systemd (doit arriver avant WatchdogSec=60)
    tokio::spawn(async {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            ticker.tick().await;
            let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]);
        }
    });

    // Wait forever (all work happens in spawned tasks)
    std::future::pending::<()>().await;
    Ok(())
}

/// Subscribes to MQTT retained topics for persist restoration.
fn spawn_persist_watcher(bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    tokio::spawn(async move {
        let mut rx = bus.subscribe_mqtt();
        loop {
            let msg = match rx.recv().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !msg.retain {
                continue;
            }
            match msg.topic.as_str() {
                "santuario/persist/pvinv_baseline" => {
                    persist::baseline::on_retained_baseline(msg.payload_str(), &state).await;
                }
                "santuario/persist/yield_yesterday" => {
                    persist::baseline::on_retained_yield_yesterday(msg.payload_str(), &state).await;
                }
                _ => {}
            }
        }
    });
}
