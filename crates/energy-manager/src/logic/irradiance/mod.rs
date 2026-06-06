mod rules;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn};
use crate::bus::AppBus;
use crate::rules_loader::RulesLoader;
use crate::types::EnergyState;

pub async fn spawn(bus: AppBus, state: Arc<RwLock<EnergyState>>, bms_server_url: String, loader: Arc<RulesLoader>) {
    crate::supervise::spawn_critical(http_poll_task(bms_server_url, state.clone(), bus, loader));
}

async fn http_poll_task(bms_server_url: String, state: Arc<RwLock<EnergyState>>, bus: AppBus, loader: Arc<RulesLoader>) {
    let url = format!("{}/api/v1/irradiance/status", bms_server_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let mut ticker = interval(Duration::from_secs(30));
    let mut reload_rx = bus.subscribe_rule_reload();

    let mut rule_engine = match rules::IrradianceRuleEngine::with_source(&loader.load("irradiance")) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Irradiance HTTP: failed to init rule engine: {e}");
            return;
        }
    };
    info!("Irradiance HTTP poll started → {url}");

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                match client.get(&url).timeout(Duration::from_secs(5)).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.json::<serde_json::Value>().await {
                            Ok(json) => {
                                if let Some(wm2) = json.get("irradiance_wm2").and_then(|v| v.as_f64()) {
                                    let connected = json.get("connected").and_then(|v| v.as_bool()).unwrap_or(true);

                                    debug!("Irradiance HTTP: raw={wm2}, connected={connected}");

                                    if !connected {
                                        warn!("Irradiance HTTP: sensor disconnected (connected=false), keeping last value");
                                        continue;
                                    }

                                    state.write().await.irradiance_wm2 = Some(wm2);

                                    match rule_engine.validate(wm2) {
                                        Ok(true) => {
                                            bus.emit_live(crate::types::LiveEvent::new(
                                                "irradiance",
                                                serde_json::json!({ "irradiance_wm2": wm2 }),
                                            ));
                                            info!("Irradiance HTTP poll: {:.0} W/m²", wm2);
                                        }
                                        Ok(false) => {
                                            warn!("Irradiance HTTP: out of range: {wm2} W/m² (state updated)");
                                        }
                                        Err(e) => tracing::error!("Irradiance HTTP: rule engine error: {e}"),
                                    }
                                } else {
                                    warn!("Irradiance HTTP: missing 'irradiance_wm2' field in JSON");
                                }
                            }
                            Err(e) => warn!("Irradiance HTTP: JSON parse error: {e}"),
                        }
                    }
                    Ok(resp) => warn!("Irradiance HTTP: status {}", resp.status()),
                    Err(e) => warn!("Irradiance HTTP: request failed: {e}"),
                }
            }

            Ok(name) = reload_rx.recv() => {
                if name == "irradiance" || name == "*" {
                    let src = loader.load("irradiance");
                    match rules::IrradianceRuleEngine::with_source(&src) {
                        Ok(e) => { rule_engine = e; info!("irradiance rule engine reloaded"); }
                        Err(e) => tracing::warn!("irradiance reload failed (keeping old engine): {e}"),
                    }
                }
            }
        }
    }
}
