/// Manages VEBus charge current based on grid state and PV excess.
/// Publishes W/.../MaxChargeCurrent and W/.../PowerAssistEnabled.
/// Mode selection is pure Rust (`rules::evaluate`).
mod rules;

use chrono::Utc;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn};

use crate::bus::AppBus;
use crate::config::{ChargeCurrent as ChargeCfg, VictronConfig};
use crate::mqtt::topics::publish;
use crate::types::{EnergyState, MqttOutgoing};

pub async fn spawn(
    vic: Arc<VictronConfig>,
    cfg: ChargeCfg,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    crate::supervise::spawn_critical(run(vic, cfg, bus, state));
}

async fn run(
    vic: Arc<VictronConfig>,
    cfg: ChargeCfg,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    let vb  = vic.vebus_instance;
    let pid = vic.portal_id.clone();

    let topic_ignore   = format!("N/{pid}/vebus/{vb}/Ac/State/IgnoreAcIn1");
    let topic_pv_power = format!("N/{pid}/system/0/Ac/PvOnOutput/L1/Power");
    let topic_consump  = format!("N/{pid}/system/0/Ac/ConsumptionOnOutput/L1/Power");

    let mut rx = bus.subscribe_mqtt();

    loop {
        let msg = match rx.recv().await {
            Ok(m) => m,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!("charge_current MQTT subscriber lagged, dropped {n} message(s)");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };
        let t = &msg.topic;
        if t != &topic_ignore && t != &topic_pv_power && t != &topic_consump {
            continue;
        }

        {
            let mut s = state.write().await;
            if t == &topic_ignore {
                if let Some(v) = msg.victron_value::<i64>() {
                    s.ac_ignore = Some(v);
                }
            } else if t == &topic_pv_power {
                if let Some(v) = msg.victron_value::<f64>() {
                    // AC PV on inverter output — NOT the DC MPPT-273 measurement.
                    // (Previously this overwrote mppt_power_273_w with an AC value.)
                    s.ac_pv_on_output_w = Some(v);
                }
            } else if t == &topic_consump {
                if let Some(v) = msg.victron_value::<f64>() {
                    s.house_power_w = Some(v);
                }
            }
        }

        compute_and_publish(&bus, &state, &cfg, &pid, vb).await;
    }
}

async fn compute_and_publish(
    bus: &AppBus,
    state: &Arc<RwLock<EnergyState>>,
    cfg: &ChargeCfg,
    portal_id: &str,
    vebus: u32,
) {
    let s = state.read().await;

    let offgrid   = s.ac_ignore.map(|v| v == 1).unwrap_or(false);
    let pv_w      = s.ac_pv_on_output_w.unwrap_or(0.0);
    let cons_w    = s.house_power_w.unwrap_or(0.0);
    let pv_excess = (pv_w - cons_w) > cfg.pv_excess_threshold_w;

    let mode = rules::evaluate(offgrid, pv_excess);

    let (charge_a, power_assist, feed_in) = match mode {
        "offgrid"        => (cfg.offgrid_max_a,    1i64, None),
        "grid_pv_excess" => (cfg.grid_pv_excess_a, 0i64, Some(0i64)),
        _                => (cfg.grid_no_excess_a, 0i64, Some(0i64)),
    };

    let changed = s.last_charge_current_a != Some(charge_a)
        || s.last_power_assist != Some(power_assist);
    drop(s);

    if !changed {
        return;
    }

    info!("Charge current: {charge_a}A, mode={mode}");

    {
        let mut s = state.write().await;
        s.last_charge_current_a = Some(charge_a);
        s.last_power_assist     = Some(power_assist);
        s.last_charge_ts        = Some(Utc::now());
    }

    bus.publish(MqttOutgoing::transient(
        publish::vebus_max_charge_current(portal_id, vebus),
        json!({ "value": charge_a }),
    )).await;

    bus.publish(MqttOutgoing::transient(
        publish::vebus_power_assist(portal_id, vebus),
        json!({ "value": power_assist }),
    )).await;

    if let Some(fi) = feed_in {
        bus.publish(MqttOutgoing::transient(
            publish::cgwacs_max_feed_in(portal_id),
            json!({ "value": fi }),
        )).await;
    }
}
