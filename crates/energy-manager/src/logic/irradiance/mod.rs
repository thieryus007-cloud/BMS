// crates/energy-manager/src/logic/irradiance/mod.rs
//
mod rules;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn};
use crate::bus::AppBus;
use crate::types::EnergyState;

pub async fn spawn(bus: AppBus, state: Arc<RwLock<EnergyState>>, bms_server_url: String) {
    // ✅ bus.clone() passé à http_poll_task
    tokio::spawn(http_poll_task(bms_server_url, state.clone(), bus));
}

async fn http_poll_task(bms_server_url: String, state: Arc<RwLock<EnergyState>>, bus: AppBus) {
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
                        // ✅ CLÉS SANS ESPACES
                        if let Some(wm2) = json.get("irradiance_wm2").and_then(|v| v.as_f64()) {
                            let connected = json.get("connected").and_then(|v| v.as_bool()).unwrap_or(true);
                            
                            // ✅ LOG explicite pour le debug
                            debug!("Irradiance HTTP: raw={wm2}, connected={connected}");
                            
                            if !connected {
                                warn!("Irradiance HTTP: sensor disconnected (connected=false), keeping last value");
                                continue;
                            }
                            
                            // ✅ TOUJOURS mettre à jour le state avec la valeur brute,
                            // même si le rule engine la considère hors range.
                            // Le water_heater fait sa propre comparaison avec irradiance_min_wm2.
                            state.write().await.irradiance_wm2 = Some(wm2);
                            
                            match rule_engine.validate(wm2) {
                                Ok(true) => {
                                    bus.emit_live(crate::types::LiveEvent::new(
                                        "irradiance",
                                        serde_json::json!({ "irradiance_wm2": wm2 }),
                                    ));
                                    info!("🔍 HTTP Poll Success: {:.0} W/m²", wm2);
                                }
                                Ok(false) => {
                                    // ✅ La valeur est maintenant dans le state, on log juste le rejet
                                    warn!("Irradiance HTTP: out of range: {wm2} W/m² (rule engine rejected, but state updated)");
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
}
// ✅ mqtt_task SUPPRIMÉ comme demandé
