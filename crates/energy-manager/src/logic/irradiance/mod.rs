/// Receives santuario/irradiance/raw → validates via rule engine → stores in state.
/// Also polls daly-bms-server HTTP API every 30s as primary reliable source.
mod rules;

use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn};

use crate::bus::AppBus;
use crate::types::EnergyState;

const TOPIC: &str = "santuario/irradiance/raw";

pub async fn spawn(bus: AppBus, state: Arc<RwLock<EnergyState>>, bms_server_url: String) {
    tokio::spawn(http_poll_task(bms_server_url, state.clone()));
    tokio::spawn(mqtt_task(bus, state));
}

/// Polls daly-bms-server GET /api/v1/irradiance/status every 30s.
/// This is the primary irradiance source — always has the correct value
/// directly from the PRALRAN RS485 sensor.
async fn http_poll_task(bms_server_url: String, state: Arc<RwLock<EnergyState>>) {
    let url = format!("{}/api/v1/irradiance/status", bms_server_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let mut ticker = interval(Duration::from_secs(30));
    let mut rule_engine = match rules::IrradianceRuleEngine::new() {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Irradiance HTTP: failed to init rule engine: {e}");
            return;
        }
    };

    info!("Irradiance HTTP poll started → {url}");

    loop {
        ticker.tick().await;
        match client.get(&url).timeout(Duration::from_secs(5)).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<serde_json::Value>().await {
                    Ok(json) => {
                        if let Some(wm2) = json.get("irradiance_wm2").and_then(|v| v.as_f64()) {
                            let connected = json.get("connected").and_then(|v| v.as_bool()).unwrap_or(true);
                            if !connected {
                                debug!("Irradiance HTTP: sensor not connected");
                                continue;
                            }
                            match rule_engine.validate(wm2) {
                                Ok(true) => {
                                    let prev = state.read().await.irradiance_wm2;
                                    state.write().await.irradiance_wm2 = Some(wm2);
                                    bus.emit_live(crate::types::LiveEvent::new(
                                        "irradiance",
                                        serde_json::json!({ "irradiance_wm2": wm2 }),
                                    ));
                                    if prev.map(|p| (p - wm2).abs() > 5.0).unwrap_or(true) {
                                        info!("Irradiance HTTP: {:.0} W/m²", wm2);
                                    } else {
                                        debug!("Irradiance HTTP: {:.0} W/m²", wm2);
                                    }
                                }
                                Ok(false) => debug!("Irradiance HTTP: out of range: {wm2}"),
                                Err(e)    => tracing::error!("Irradiance HTTP: rule engine error: {e}"),
                            }
                        }
                    }
                    Err(e) => warn!("Irradiance HTTP: JSON parse error: {e}"),
                }
            }
            Ok(resp) => warn!("Irradiance HTTP: status {}", resp.status()),
            Err(e)   => warn!("Irradiance HTTP: request failed: {e}"),
        }
    }
}

/// Listens to santuario/irradiance/raw MQTT topic as supplementary source.
async fn mqtt_task(bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    let mut rule_engine = match rules::IrradianceRuleEngine::new() {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to init irradiance rule engine: {e}");
            return;
        }
    };

    let mut rx = bus.subscribe_mqtt();
    loop {
        let msg = match rx.recv().await {
            Ok(m)  => m,
            Err(_) => continue,
        };
        if msg.topic != TOPIC {
            continue;
        }

        let raw = msg.payload_str().trim().parse::<f64>().unwrap_or(-1.0);

        match rule_engine.validate(raw) {
            Ok(true)  => {}
            Ok(false) => {
                debug!("Irradiance MQTT out of range: {raw}");
                continue;
            }
            Err(e) => {
                tracing::error!("Irradiance rule engine error: {e}");
                continue;
            }
        }

        debug!("Irradiance MQTT: {raw} W/m²");
        state.write().await.irradiance_wm2 = Some(raw);
        bus.emit_live(crate::types::LiveEvent::new(
            "irradiance",
            serde_json::json!({ "irradiance_wm2": raw }),
        ));
    }
}
