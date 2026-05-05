/// Manages VEBus charge current based on grid state and PV excess.
/// Publishes W/.../MaxChargeCurrent and W/.../PowerAssistEnabled.
/// Mode selection is handled by rust-rule-engine (rules/charge_current.grl).
mod rules;

use chrono::Utc;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

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
    tokio::spawn(run(vic, cfg, bus, state));
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

    let mut rule_engine = match rules::ChargeRuleEngine::new() {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to init charge current rule engine: {e}");
            return;
        }
    };

    let mut rx = bus.subscribe_mqtt();
    loop {
        let msg = match rx.recv().await {
            Ok(m)  => m,
            Err(_) => continue,
        };

        let t = &msg.topic;
        if t != &topic_ignore && t != &topic_pv_power && t != &topic_consump {
            continue;
        }

        // Update state from MQTT
        {
            let mut s = state.write().await;
            if t == &topic_ignore {
                if let Some(v) = msg.victron_value::<i64>() {
                    s.ac_ignore = Some(v);
                }
            } else if t == &topic_pv_power {
                if let Some(v) = msg.victron_value::<f64>() {
                    s.mppt_power_273_w = Some(v);
                }
            } else if t == &topic_consump {
                if let Some(v) = msg.victron_value::<f64>() {
                    s.house_power_w = Some(v);
                }
            }
        }

        compute_and_publish(&mut rule_engine, &bus, &state, &cfg, &pid, vb).await;
    }
}

async fn compute_and_publish(
    rule_engine: &mut rules::ChargeRuleEngine,
    bus: &AppBus,
    state: &Arc<RwLock<EnergyState>>,
    cfg: &ChargeCfg,
    portal_id: &str,
    vebus: u32,
) {
    let s = state.read().await;

    let offgrid   = s.ac_ignore.map(|v| v == 1).unwrap_or(false);
    let pv_w      = s.mppt_power_273_w.unwrap_or(0.0);
    let cons_w    = s.house_power_w.unwrap_or(0.0);
    let pv_excess = (pv_w - cons_w) > cfg.pv_excess_threshold_w;

    // Rule engine decides the charging mode
    let mode = match rule_engine.evaluate(offgrid, pv_excess) {
        Ok(m)  => m,
        Err(e) => {
            tracing::error!("Charge current rule engine error: {e}");
            "grid_no_excess".to_string()
        }
    };

    let (charge_a, power_assist, feed_in) = match mode.as_str() {
        "offgrid"        => (cfg.offgrid_max_a,      1i64, None),
        "grid_pv_excess" => (cfg.grid_pv_excess_a,   0i64, Some(0i64)),
        _                => (cfg.grid_no_excess_a,   0i64, Some(0i64)),
    };

    // Only publish if changed
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
