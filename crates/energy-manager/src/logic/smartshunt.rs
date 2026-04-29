/// SmartShunt (Victron battery monitor) — reads native Venus OS D-Bus/MQTT topics.
///
/// Primary data source: N/{portalId}/battery/{shunt_instance}/...
///   - Dc/0/Voltage, Dc/0/Current, Dc/0/Power
///   - Soc, TimeToGo, State
///   - History/ChargedEnergy   (kWh cumulative)
///   - History/DischargedEnergy (kWh cumulative)
///
/// Daily charged/discharged kWh are derived from the cumulative counters using
/// a per-day baseline (same approach as pvinverter ET112).
///
/// Falls back to system/0/Dc/Battery/* aggregates for SOC/current/state when
/// the direct SmartShunt instance paths are not available.
use chrono::{Datelike, Utc};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::bus::AppBus;
use crate::config::VictronConfig;
use crate::mqtt::topics::publish;
use crate::types::{EnergyState, LiveEvent, MqttIncoming, MqttOutgoing};

pub async fn spawn(vic: Arc<VictronConfig>, bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    tokio::spawn(run(vic, bus, state));
}

async fn run(vic: Arc<VictronConfig>, bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    let pid   = &vic.portal_id;
    let shunt = vic.smartshunt_instance;

    let t_voltage    = format!("N/{pid}/battery/{shunt}/Dc/0/Voltage");
    let t_current    = format!("N/{pid}/battery/{shunt}/Dc/0/Current");
    let t_power      = format!("N/{pid}/battery/{shunt}/Dc/0/Power");
    let t_soc        = format!("N/{pid}/battery/{shunt}/Soc");
    let t_ttg        = format!("N/{pid}/battery/{shunt}/TimeToGo");
    let t_state      = format!("N/{pid}/battery/{shunt}/State");
    let mut rx = bus.subscribe_mqtt();
    loop {
        let msg = match rx.recv().await {
            Ok(m) => m,
            Err(_) => continue,
        };

        if let Some(dirty) = handle(&msg, &state, &t_voltage, &t_current, &t_power,
                                    &t_soc, &t_ttg, &t_state).await {
            if dirty {
                publish_state(&bus, &state).await;
            }
        }
    }
}

/// Returns Some(true) if state was updated (caller should publish),
/// Some(false) if topic matched but no value changed,
/// None if topic was not ours.
async fn handle(
    msg: &MqttIncoming,
    state: &Arc<RwLock<EnergyState>>,
    t_voltage: &str,
    t_current: &str,
    t_power:   &str,
    t_soc:     &str,
    t_ttg:     &str,
    t_state:   &str,
) -> Option<bool> {
    let t = &msg.topic;

    // --- System aggregate fallbacks (always subscribed) ---
    let is_sys_soc     = t.ends_with("Battery/Soc")     && t.contains("/system/0/");
    let is_sys_current = t.ends_with("Battery/Current")  && t.contains("/system/0/");
    let is_sys_state   = t.ends_with("Battery/State")    && t.contains("/system/0/");
    let is_sys_ttg     = t.ends_with("Battery/TimeToGo") && t.contains("/system/0/");
    let is_vebus_v     = t.ends_with("/Dc/0/Voltage")    && t.contains("/vebus/");
    let is_vebus_pw    = t.ends_with("/Dc/0/Power")      && t.contains("/vebus/");

    // --- Direct SmartShunt topics ---
    // History/ChargedEnergy and DischargedEnergy use wildcard subscription
    // (battery/+/...) so we match by suffix instead of exact topic.
    let is_charged    = t.ends_with("/History/ChargedEnergy")    && t.contains("/battery/");
    let is_discharged = t.ends_with("/History/DischargedEnergy") && t.contains("/battery/");

    if is_charged || is_discharged {
        // Visible in journalctl -u energy-manager -f to confirm the SmartShunt instance.
        info!("SmartShunt energy counter topic: {t}");
    }
    let is_shunt = t == t_voltage || t == t_current || t == t_power
        || t == t_soc || t == t_ttg || t == t_state
        || is_charged || is_discharged;

    if !is_shunt && !is_sys_soc && !is_sys_current && !is_sys_state
        && !is_sys_ttg && !is_vebus_v && !is_vebus_pw {
        return None;
    }

    let mut s = state.write().await;
    let now = Utc::now();

    if t == t_voltage {
        if let Some(v) = msg.victron_value::<f64>() {
            s.battery_voltage_v = Some(v);
        }
    } else if t == t_current {
        if let Some(v) = msg.victron_value::<f64>() {
            s.battery_current_a = Some(v);
            integrate_ah(&mut s, v, now);
        }
    } else if t == t_power {
        if let Some(v) = msg.victron_value::<f64>() {
            s.battery_power_w = Some(v);
        }
    } else if t == t_soc {
        if let Some(v) = msg.victron_value::<f64>() {
            s.soc_pct = Some(v);
        }
    } else if t == t_ttg {
        if let Some(v) = msg.victron_value::<i64>() {
            s.time_to_go_sec = Some(v);
        }
    } else if t == t_state {
        if let Some(v) = msg.victron_value::<i64>() {
            s.battery_state = Some(v);
        }
    } else if is_charged {
        if let Some(kwh) = msg.victron_value::<f64>() {
            let day_key = now.date_naive().num_days_from_ce();
            if s.shunt_charged_day != day_key || s.shunt_charged_baseline_kwh.is_none() {
                // New day or first message: set baseline, reset accumulators
                s.shunt_charged_baseline_kwh    = Some(kwh);
                s.shunt_discharged_baseline_kwh = None; // will be set on next Discharged msg
                s.shunt_charged_day             = day_key;
            }
            let baseline = s.shunt_charged_baseline_kwh.unwrap_or(kwh);
            s.shunt_charged_today_kwh = (kwh - baseline).max(0.0);
            debug!("SmartShunt ChargedEnergy: raw={kwh:.3} baseline={baseline:.3} today={:.3}", s.shunt_charged_today_kwh);
        }
    } else if is_discharged {
        if let Some(kwh) = msg.victron_value::<f64>() {
            let day_key = now.date_naive().num_days_from_ce();
            if s.shunt_discharged_day != day_key || s.shunt_discharged_baseline_kwh.is_none() {
                s.shunt_discharged_baseline_kwh = Some(kwh);
                s.shunt_discharged_day          = day_key;
            }
            let baseline = s.shunt_discharged_baseline_kwh.unwrap_or(kwh);
            s.shunt_discharged_today_kwh = (kwh - baseline).max(0.0);
            debug!("SmartShunt DischargedEnergy: raw={kwh:.3} baseline={baseline:.3} today={:.3}", s.shunt_discharged_today_kwh);
        }
    }
    // System aggregates — fallback when direct shunt not available
    else if is_sys_soc {
        if let Some(v) = msg.victron_value::<f64>() {
            s.soc_pct = Some(v);
        }
    } else if is_sys_current {
        if let Some(v) = msg.victron_value::<f64>() {
            integrate_ah(&mut s, v, now);
            s.battery_current_a = Some(v);
        }
    } else if is_sys_state {
        if let Some(v) = msg.victron_value::<i64>() {
            s.battery_state = Some(v);
        }
    } else if is_sys_ttg {
        if let Some(v) = msg.victron_value::<i64>() {
            s.time_to_go_sec = Some(v);
        }
    } else if is_vebus_v {
        if let Some(v) = msg.victron_value::<f64>() {
            s.battery_voltage_v = Some(v);
        }
    } else if is_vebus_pw {
        if let Some(v) = msg.victron_value::<f64>() {
            s.battery_power_w = Some(v);
        }
    }

    Some(true)
}

/// Current integration into Ah (backup metric alongside kWh from shunt history).
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
    let soc            = s.soc_pct;
    let voltage        = s.battery_voltage_v;
    let current        = s.battery_current_a;
    let power          = s.battery_power_w;
    let batt_state     = s.battery_state;
    let ttg            = s.time_to_go_sec;
    let charged_kwh    = s.shunt_charged_today_kwh;
    let discharged_kwh = s.shunt_discharged_today_kwh;
    let ah_charged     = s.ah_charged_today;
    let ah_discharged  = s.ah_discharged_today;

    let payload = json!({
        "Soc":                    soc,
        "Voltage":                voltage,
        "Current":                current,
        "Power":                  power,
        "State":                  batt_state,
        "TimeToGo":               ttg,
        "ChargedTodayKwh":        charged_kwh,
        "DischargedTodayKwh":     discharged_kwh,
        "AhChargedToday":         ah_charged,
        "AhDischargedToday":      ah_discharged,
    });
    drop(s);

    bus.publish(MqttOutgoing::retained(publish::SYSTEM_VENUS, &payload)).await;
    bus.emit_live(LiveEvent::new("battery", &payload));

}
