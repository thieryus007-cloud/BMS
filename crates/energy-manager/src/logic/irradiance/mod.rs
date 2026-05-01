/// Receives santuario/irradiance/raw → validates via rule engine → stores in state.
mod rules;

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

use crate::bus::AppBus;
use crate::types::EnergyState;

const TOPIC: &str = "santuario/irradiance/raw";

pub async fn spawn(bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    tokio::spawn(run(bus, state));
}

async fn run(bus: AppBus, state: Arc<RwLock<EnergyState>>) {
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
                debug!("Irradiance out of range: {raw}");
                continue;
            }
            Err(e) => {
                tracing::error!("Irradiance rule engine error: {e}");
                continue;
            }
        }

        debug!("Irradiance: {raw} W/m²");
        state.write().await.irradiance_wm2 = Some(raw);
        bus.emit_live(crate::types::LiveEvent::new(
            "irradiance",
            serde_json::json!({ "irradiance_wm2": raw }),
        ));
    }
}
