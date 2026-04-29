//! Endpoint historique graphique — utilise Tsink pour le dashboard overview.
//!
//! GET /api/v1/chart/history?minutes=60
//! Retourne { solar:[{t,v}], soc:[{t,v}], load:[{t,v}] }

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use chrono::Utc;
use tsink::promql::PromqlValue;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct HistoryParams {
    pub minutes: Option<u32>,
}

#[derive(Deserialize)]
pub struct EdgeHistoryParams {
    pub measurement: String,
    pub field: String,
    pub address: Option<String>,
    pub minutes: Option<u32>,
}

/// GET /api/v1/chart/history?minutes=X
pub async fn get_chart_history(
    State(state): State<AppState>,
    Query(q): Query<HistoryParams>,
) -> impl IntoResponse {
    let minutes = q.minutes.unwrap_or(60).clamp(1, 720) as i64;

    let tsink = match &state.tsink {
        Some(t) => t,
        None => return Json(json!({"solar": [], "soc": [], "load": [], "ok": false, "reason": "tsink_disabled"})),
    };

    let now_ms  = Utc::now().timestamp_millis();
    let start_ms = now_ms - minutes * 60 * 1000;
    let step_ms: i64 = if minutes <= 60 { 60_000 } else if minutes <= 360 { 300_000 } else { 600_000 };

    let (solar_res, soc_res, load_res) = tokio::join!(
        tsink.query_range("solar_total_w".into(), start_ms, now_ms, step_ms),
        tsink.query_range("avg(bms_soc)".into(), start_ms, now_ms, step_ms),
        tsink.query_range("et112_power_w{address=\"0x08\"}".into(), start_ms, now_ms, step_ms),
    );

    Json(json!({
        "ok": true,
        "solar": extract_series_hhmm(solar_res.ok()),
        "soc":   extract_series_hhmm(soc_res.ok()),
        "load":  extract_series_hhmm(load_res.ok()),
    }))
}

/// Extracts [{t: "HH:MM", v: f64}] from a PromQL range result.
fn extract_series_hhmm(result: Option<PromqlValue>) -> Vec<Value> {
    let series_list = match result {
        Some(PromqlValue::RangeVector(s)) => s,
        _ => return Vec::new(),
    };
    // Take the first series (or aggregate if multiple)
    let Some(first) = series_list.into_iter().next() else { return Vec::new() };
    first.samples.iter().map(|(ts_ms, v)| {
        let dt = chrono::DateTime::from_timestamp_millis(*ts_ms)
            .unwrap_or_default()
            .with_timezone(&chrono::Local);
        let t = dt.format("%H:%M").to_string();
        json!({"t": t, "v": (v * 10.0).round() / 10.0})
    }).collect()
}

/// GET /api/v1/chart/edge-history?measurement=...&field=...&address=...&minutes=360
pub async fn get_edge_history(
    State(state): State<AppState>,
    Query(q): Query<EdgeHistoryParams>,
) -> impl IntoResponse {
    let minutes = q.minutes.unwrap_or(360).clamp(1, 1440) as i64;

    let tsink = match &state.tsink {
        Some(t) => t,
        None => return Json(json!({ "ok": false, "series": [], "reason": "tsink_disabled" })),
    };

    // Map old InfluxDB measurement+field names to Tsink metric names
    let (metric, unit) = match (q.measurement.as_str(), q.field.as_str()) {
        ("bms_status", "current")          => ("bms_current", "A"),
        ("bms_status", "soc")              => ("bms_soc", "%"),
        ("et112_status", "power_w")        => ("et112_power_w", "W"),
        ("et112_status", "current_a")      => ("et112_current_a", "A"),
        ("venus_mppt_total", "power_w")    => ("solar_total_w", "W"),
        ("venus_mppt_total", "current_a")  => ("mppt_power_w", "W"),
        ("venus_smartshunt", "current_a")  => ("venus_shunt_current_a", "A"),
        ("venus_smartshunt", "power_w")    => ("venus_shunt_power_w", "W"),
        ("venus_inverter", "ac_out_power_w") => ("venus_inverter_ac_output_power_w", "W"),
        ("venus_inverter", "dc_power_w")   => ("venus_inverter_power_w", "W"),
        ("solar_power", "mppt_power_w")    => ("mppt_power_w", "W"),
        ("inverter_status", "dc_power_w")  => ("venus_inverter_power_w", "W"),
        ("inverter_status", "ac_out_power_w") => ("venus_inverter_ac_output_power_w", "W"),
        _ => return Json(json!({ "ok": false, "series": [], "reason": "unknown_metric" })),
    };

    // Build label filter if address is given
    let query = if let Some(addr) = q.address.as_deref().filter(|s| !s.is_empty()) {
        // Normalize address: "1" → "0x01"
        let normalized = if addr.starts_with("0x") || addr.starts_with("0X") {
            addr.to_string()
        } else if let Ok(n) = u32::from_str_radix(addr, 16) {
            format!("{:#04x}", n)
        } else if let Ok(n) = addr.parse::<u32>() {
            format!("{:#04x}", n)
        } else {
            addr.to_string()
        };
        format!("{}{{address=\"{}\"}}", metric, normalized)
    } else {
        metric.to_string()
    };

    let now_ms   = Utc::now().timestamp_millis();
    let start_ms = now_ms - minutes * 60 * 1000;
    let step_ms: i64 = if minutes <= 60 { 60_000 } else if minutes <= 360 { 180_000 } else { 600_000 };

    let result = tsink.query_range(query.clone(), start_ms, now_ms, step_ms).await;

    let series: Vec<Value> = match result.ok() {
        Some(PromqlValue::RangeVector(series_list)) => {
            series_list.into_iter()
                .flat_map(|s| s.samples.into_iter().map(|(ts_ms, v)| {
                    let dt = chrono::DateTime::from_timestamp_millis(ts_ms)
                        .unwrap_or_default()
                        .with_timezone(&chrono::Local);
                    let t = dt.format("%H:%M").to_string();
                    json!({"t": t, "v": (v * 100.0).round() / 100.0})
                }))
                .collect()
        }
        _ => Vec::new(),
    };

    Json(json!({
        "ok":      true,
        "series":  series,
        "unit":    unit,
        "metric":  metric,
        "minutes": minutes,
    }))
}
