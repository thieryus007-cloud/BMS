/// Handles Tasmota MQTT topics for the water heater relay (tongou_3BC764).
/// Parses stat/{id}/POWER and tele/{id}/SENSOR.
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

use crate::bus::AppBus;
use crate::config::VictronConfig;
use crate::types::{EnergyState, LiveEvent, MqttOutgoing};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct TasmotaSensor {
    #[serde(rename = "ENERGY")]
    energy: Option<TasmotaEnergy>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TasmotaEnergy {
    power: Option<f64>,
    voltage: Option<f64>,
    current: Option<f64>,
    today: Option<f64>,
    total: Option<f64>,
}

pub async fn spawn(
    cfg: Arc<VictronConfig>,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    tokio::spawn(run(cfg, bus, state));
}

async fn run(
    cfg: Arc<VictronConfig>,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    let id = &cfg.tasmota_waterheater_id;
    if id.is_empty() {
        return;
    }
    let stat_topic  = format!("stat/{id}/POWER");
    let tele_topic  = format!("tele/{id}/SENSOR");

    let mut rx = bus.subscribe_mqtt();
    loop {
        let msg = match rx.recv().await {
            Ok(m) => m,
            Err(_) => continue,
        };
        let t = &msg.topic;
        if t == &stat_topic {
            let on = msg.payload_str().trim().eq_ignore_ascii_case("ON");
            debug!("Tasmota WH relay: {}", if on { "ON" } else { "OFF" });
            state.write().await.tasmota_wh_on = on;
            bus.emit_live(LiveEvent::new("tasmota_wh", serde_json::json!({ "on": on })));
        } else if t == &tele_topic {
            if let Some(sensor) = msg.json::<TasmotaSensor>() {
                if let Some(e) = sensor.energy {
                    let mut s = state.write().await;
                    s.tasmota_wh_power_w         = e.power;
                    s.tasmota_wh_energy_today_kwh = e.today;
                    bus.emit_live(LiveEvent::new("tasmota_wh_energy", serde_json::json!({
                        "power_w": e.power,
                        "voltage_v": e.voltage,
                        "current_a": e.current,
                        "today_kwh": e.today,
                        "total_kwh": e.total,
                    })));
                }
            }
        }
    }
}

/// Send ON/OFF/TOGGLE command to the Tasmota relay.
#[allow(dead_code)]
pub async fn send_command(
    bus: &AppBus,
    tasmota_id: &str,
    cmd: &str,  // "ON", "OFF", "TOGGLE"
) {
    let topic = crate::mqtt::topics::publish::tasmota_cmd(tasmota_id);
    bus.publish(MqttOutgoing::raw(topic, cmd, false)).await;
}
