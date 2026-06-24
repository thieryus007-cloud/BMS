mod rules;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn};
use crate::bus::AppBus;
use crate::types::EnergyState;

pub async fn spawn(bus: AppBus, state: Arc<RwLock<EnergyState>>, bms_server_url: String) {
    crate::supervise::spawn_critical(http_poll_task(bms_server_url, state.clone(), bus));
}

async fn http_poll_task(bms_server_url: String, state: Arc<RwLock<EnergyState>>, bus: AppBus) {
    let url = format!("{}/api/v1/irradiance/status", bms_server_url.trim_end_matches('/'));
    let client = crate::http_clients::shared_client();
    let mut ticker = interval(Duration::from_secs(30));
    info!("Irradiance HTTP poll started → {url}");

    loop {
        ticker.tick().await;
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

                            if rules::validate(wm2) {
                                bus.emit_live(crate::types::LiveEvent::new(
                                    "irradiance",
                                    serde_json::json!({ "irradiance_wm2": wm2 }),
                                ));
                                // Démoté info!→debug! (2026-06) : ce poll tourne
                                // toutes les 30 s (2880 lignes/jour, même la nuit à
                                // 0 W/m²) → bruit dominant dans journald. La valeur
                                // part déjà dans bus.emit_live + metrics-store : on
                                // ne perd aucune donnée, seulement le doublon texte.
                                debug!("Irradiance HTTP poll: {:.0} W/m²", wm2);
                            } else {
                                warn!("Irradiance HTTP: out of range: {wm2} W/m² (state updated)");
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
