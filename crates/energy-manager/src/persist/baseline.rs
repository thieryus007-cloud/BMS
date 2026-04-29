/// Restores solar baselines and production counters at startup.
/// Primary source: MQTT retained topics (pvinv_baseline, yield_yesterday).
use chrono::Datelike;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::types::EnergyState;

/// Called when MQTT retained baseline arrives (santuario/persist/pvinv_baseline).
/// Format: "{ordinal_day}:{kwh:.3}"  (e.g. "738976:17.123")
/// The day is checked against today to reject stale baselines from previous days.
pub async fn on_retained_baseline(payload: &str, state: &Arc<RwLock<EnergyState>>) {
    if payload.is_empty() {
        return;
    }
    let today = chrono::Utc::now().date_naive().num_days_from_ce();

    let (day, kwh) = if let Some((d_str, kwh_str)) = payload.trim().split_once(':') {
        let Ok(d)   = d_str.parse::<i32>()  else { return };
        let Ok(v)   = kwh_str.parse::<f64>() else { return };
        (d, v)
    } else {
        info!("pvinv_baseline retained: legacy format without day, ignoring to prevent stale baseline");
        return;
    };

    if day != today {
        info!("pvinv_baseline retained: from day {day}, today is {today} — ignoring stale baseline");
        return;
    }

    let mut s = state.write().await;
    if s.pvinv_baseline_kwh.is_none() {
        s.pvinv_baseline_kwh = Some(kwh);
        s.pvinv_baseline_day = today;
        info!("Baseline restored from MQTT retained: pvinv_baseline = {kwh:.3} kWh (day={day})");
    }
}

/// Called when MQTT retained yield_yesterday arrives
pub async fn on_retained_yield_yesterday(payload: &str, state: &Arc<RwLock<EnergyState>>) {
    if payload.is_empty() {
        return;
    }
    if let Ok(v) = payload.trim().parse::<f64>() {
        let mut s = state.write().await;
        s.yield_yesterday_kwh = v;
        info!("Yield yesterday restored from MQTT retained: {v:.3} kWh");
    }
}
