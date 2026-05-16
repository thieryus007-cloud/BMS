/// SmartShunt (Victron battery monitor) — reads native Venus OS D-Bus/MQTT topics.
///
/// Primary data source: N/{portalId}/battery/{shunt_instance}/...
///   - Dc/0/Voltage, Dc/0/Current, Dc/0/Power
///   - Soc, TimeToGo, State
///   - History/ChargedEnergy   (kWh cumulative)
///   - History/DischargedEnergy (kWh cumulative)
///
/// Daily baseline capture decisions are delegated to the rule engine (rules/smartshunt.grl).
/// Falls back to system/0/Dc/Battery/* aggregates when direct shunt paths are absent.
mod rules;

use chrono::{Datelike, Utc};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};

use crate::bus::AppBus;
use crate::config::VictronConfig;
use crate::mqtt::topics::publish;
use crate::rules_loader::RulesLoader;
use crate::types::{EnergyState, LiveEvent, MqttIncoming, MqttOutgoing};

pub async fn spawn(vic: Arc<VictronConfig>, bus: AppBus, state: Arc<RwLock<EnergyState>>, loader: Arc<RulesLoader>) {
    tokio::spawn(run(vic, bus, state, loader));
}

async fn run(vic: Arc<VictronConfig>, bus: AppBus, state: Arc<RwLock<EnergyState>>, loader: Arc<RulesLoader>) {
    let pid   = &vic.portal_id;
    let shunt = vic.smartshunt_instance;

    let t_voltage = format!("N/{pid}/battery/{shunt}/Dc/0/Voltage");
    let t_current = format!("N/{pid}/battery/{shunt}/Dc/0/Current");
    let t_power   = format!("N/{pid}/battery/{shunt}/Dc/0/Power");
    let t_soc     = format!("N/{pid}/battery/{shunt}/Soc");
    let t_ttg     = format!("N/{pid}/battery/{shunt}/TimeToGo");
    let t_state   = format!("N/{pid}/battery/{shunt}/State");

    let mut rule_engine = match rules::SmartShuntRuleEngine::with_source(&loader.load("smartshunt")) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to init SmartShunt rule engine: {e}");
            return;
        }
    };

    let mut rx        = bus.subscribe_mqtt();
    let mut reload_rx = bus.subscribe_rule_reload();

    loop {
        tokio::select! {
            result = rx.recv() => {
                let msg = match result {
                    Ok(m) => m,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("smartshunt MQTT subscriber lagged, dropped {n} message(s)");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if let Some(dirty) = handle(
                    &msg, &state, &mut rule_engine,
                    &t_voltage, &t_current, &t_power,
                    &t_soc, &t_ttg, &t_state,
                ).await {
                    if dirty {
                        publish_state(&bus, &state).await;
                    }
                }
            }

            Ok(name) = reload_rx.recv() => {
                if name == "smartshunt" || name == "*" {
                    let src = loader.load("smartshunt");
                    match rules::SmartShuntRuleEngine::with_source(&src) {
                        Ok(e) => { rule_engine = e; tracing::info!("smartshunt rule engine reloaded"); }
                        Err(e) => tracing::warn!("smartshunt reload failed (keeping old engine): {e}"),
                    }
                }
            }
        }
    }
}

async fn handle(
    msg: &MqttIncoming,
    state: &Arc<RwLock<EnergyState>>,
    rule_engine: &mut rules::SmartShuntRuleEngine,
    t_voltage: &str,
    t_current: &str,
    t_power:   &str,
    t_soc:     &str,
    t_ttg:     &str,
    t_state:   &str,
) -> Option<bool> {
    let t = &msg.topic;

    let is_sys_soc     = t.ends_with("Battery/Soc")     && t.contains("/system/0/");
    let is_sys_current = t.ends_with("Battery/Current")  && t.contains("/system/0/");
    let is_sys_state   = t.ends_with("Battery/State")    && t.contains("/system/0/");
    let is_sys_ttg     = t.ends_with("Battery/TimeToGo") && t.contains("/system/0/");
    let is_vebus_v     = t.ends_with("/Dc/0/Voltage")    && t.contains("/vebus/");
    // NOTE: vebus/Dc/0/Power is intentionally NOT used as a fallback for battery_power_w.
    // The VEBus DC power measures only what flows through the Multiplus inverter,
    // which is a different physical quantity from the net battery power measured
    // by the SmartShunt at the battery terminals. It is captured into dc_power_w
    // by the `inverter` module instead.

    let is_charged    = t.ends_with("/History/ChargedEnergy")    && t.contains("/battery/");
    let is_discharged = t.ends_with("/History/DischargedEnergy") && t.contains("/battery/");

    if is_charged || is_discharged {
        info!("SmartShunt energy counter topic: {t}");
    }

    let is_shunt_direct = t == t_voltage || t == t_current || t == t_power
        || t == t_soc || t == t_ttg || t == t_state;
    let is_shunt = is_shunt_direct || is_charged || is_discharged;

    if !is_shunt && !is_sys_soc && !is_sys_current && !is_sys_state
        && !is_sys_ttg && !is_vebus_v {
        return None;
    }

    let mut s = state.write().await;
    let now = Utc::now();

    // Fallbacks (vebus / system aggregates) only apply when no direct SmartShunt
    // topic has been received recently. STALE_AFTER bounds how long we keep
    // trusting cached shunt values before falling back.
    const STALE_AFTER: chrono::Duration = chrono::Duration::seconds(10);
    let shunt_fresh = s.shunt_last_seen_ts
        .map(|ts| now.signed_duration_since(ts) < STALE_AFTER)
        .unwrap_or(false);

    if is_shunt_direct {
        s.shunt_last_seen_ts = Some(now);
    }

    if t == t_voltage {
        if let Some(v) = msg.victron_value::<f64>() { s.battery_voltage_v = Some(v); }
    } else if t == t_current {
        if let Some(v) = msg.victron_value::<f64>() {
            s.battery_current_a = Some(v);
            integrate_ah(&mut s, v, now);
        }
    } else if t == t_power {
        if let Some(v) = msg.victron_value::<f64>() { s.battery_power_w = Some(v); }
    } else if t == t_soc {
        if let Some(v) = msg.victron_value::<f64>() { s.soc_pct = Some(v); }
    } else if t == t_ttg {
        if let Some(v) = msg.victron_value::<i64>() { s.time_to_go_sec = Some(v); }
    } else if t == t_state {
        if let Some(v) = msg.victron_value::<i64>() { s.battery_state = Some(v); }

    } else if is_charged {
        if let Some(kwh) = msg.victron_value::<f64>() {
            let day_key = now.date_naive().num_days_from_ce();

            // Rule engine decides whether to capture baseline
            let decision = rule_engine.baseline_decision(
                s.shunt_charged_day != day_key,
                s.shunt_charged_baseline_kwh.is_none(),
                false,
                false,
            ).unwrap_or_default();

            if decision.capture_charged {
                s.shunt_charged_baseline_kwh = Some(kwh);
                s.shunt_charged_day          = day_key;
            }
            let baseline = s.shunt_charged_baseline_kwh.unwrap_or(kwh);
            s.shunt_charged_today_kwh = (kwh - baseline).max(0.0);
            debug!("SmartShunt ChargedEnergy: raw={kwh:.3} baseline={baseline:.3} today={:.3}",
                   s.shunt_charged_today_kwh);
        }

    } else if is_discharged {
        if let Some(kwh) = msg.victron_value::<f64>() {
            let day_key = now.date_naive().num_days_from_ce();

            let decision = rule_engine.baseline_decision(
                false,
                false,
                s.shunt_discharged_day != day_key,
                s.shunt_discharged_baseline_kwh.is_none(),
            ).unwrap_or_default();

            if decision.capture_discharged {
                s.shunt_discharged_baseline_kwh = Some(kwh);
                s.shunt_discharged_day          = day_key;
            }
            let baseline = s.shunt_discharged_baseline_kwh.unwrap_or(kwh);
            s.shunt_discharged_today_kwh = (kwh - baseline).max(0.0);
            debug!("SmartShunt DischargedEnergy: raw={kwh:.3} baseline={baseline:.3} today={:.3}",
                   s.shunt_discharged_today_kwh);
        }

    } else if is_sys_soc && !shunt_fresh {
        if let Some(v) = msg.victron_value::<f64>() { s.soc_pct = Some(v); }
    } else if is_sys_current && !shunt_fresh {
        if let Some(v) = msg.victron_value::<f64>() {
            integrate_ah(&mut s, v, now);
            s.battery_current_a = Some(v);
        }
    } else if is_sys_state && !shunt_fresh {
        if let Some(v) = msg.victron_value::<i64>() { s.battery_state = Some(v); }
    } else if is_sys_ttg && !shunt_fresh {
        if let Some(v) = msg.victron_value::<i64>() { s.time_to_go_sec = Some(v); }
    } else if is_vebus_v && !shunt_fresh {
        if let Some(v) = msg.victron_value::<f64>() { s.battery_voltage_v = Some(v); }
    } else {
        // Topic matched the filter but a fresher shunt value already holds the field.
        // Suppress the publish_state trigger to avoid republishing unchanged data.
        return Some(false);
    }

    Some(true)
}

fn integrate_ah(s: &mut EnergyState, current_a: f64, now: chrono::DateTime<Utc>) {
    let day_key = now.date_naive().num_days_from_ce();
    if s.ah_last_day != day_key {
        s.ah_charged_today    = 0.0;
        s.ah_discharged_today = 0.0;
        s.ah_last_day         = day_key;
    }
    if let Some(prev_ts) = s.ah_last_ts {
        let delta_ms = (now - prev_ts).num_milliseconds();
        if delta_ms > 0 && delta_ms < 600_000 {
            let delta_h = delta_ms as f64 / 3_600_000.0;
            if current_a > 0.0 {
                s.ah_charged_today    += current_a * delta_h;
            } else if current_a < 0.0 {
                s.ah_discharged_today += (-current_a) * delta_h;
            }
        }
    }
    s.ah_last_ts = Some(now);
}

async fn publish_state(bus: &AppBus, state: &Arc<RwLock<EnergyState>>) {
    let s = state.read().await;
    let payload = json!({
        "Soc":                s.soc_pct,
        "Voltage":            s.battery_voltage_v,
        "Current":            s.battery_current_a,
        "Power":              s.battery_power_w,
        "State":              s.battery_state,
        "TimeToGo":           s.time_to_go_sec,
        "ChargedTodayKwh":    s.shunt_charged_today_kwh,
        "DischargedTodayKwh": s.shunt_discharged_today_kwh,
        "AhChargedToday":     s.ah_charged_today,
        "AhDischargedToday":  s.ah_discharged_today,
    });
    drop(s);
    bus.publish(MqttOutgoing::retained(publish::SYSTEM_VENUS, &payload)).await;
    bus.emit_live(LiveEvent::new("battery", &payload));
}
