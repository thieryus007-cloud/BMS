//! Endpoint historique énergie — GET /api/v1/history/energy?period=day|week|month|year
//!
//! Sources Tsink :
//!   et112_power_w{address}          → puissance W par compteur
//!   et112_energy_import_wh{address} → énergie cumulée (import)
//!   et112_energy_export_wh{address} → énergie cumulée (export)
//!   solar_total_w                   → puissance solaire totale
//!   venus_shunt_ah_charged_today    → Ah chargés aujourd'hui
//!   venus_shunt_ah_discharged_today → Ah déchargés aujourd'hui
//!
//! Adresses ET112 production :
//!   0x07 = Micro-Onduleurs (PV inverter)
//!   0x08 = Maison (consommation)
//!   0x09 = Réseau (grid import/export)

use axum::{extract::{Query, State}, response::IntoResponse, Json};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use tsink::promql::PromqlValue;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct EnergyHistoryParams {
    /// Période : day | week | month | year
    pub period: Option<String>,
}

/// GET /api/v1/history/energy?period=day
pub async fn get_energy_history(
    State(state): State<AppState>,
    Query(q): Query<EnergyHistoryParams>,
) -> impl IntoResponse {
    let period = q.period.as_deref().unwrap_or("day");

    let tsink = match &state.tsink {
        Some(t) => t,
        None => return Json(json!({"ok": false, "reason": "tsink_disabled"})),
    };

    let now_ms = Utc::now().timestamp_millis();
    let (start_ms, step_ms): (i64, i64) = match period {
        "week"  => (now_ms - 7 * 86_400_000,   3_600_000),   // 7 jours, pas 1h
        "month" => (now_ms - 30 * 86_400_000,  21_600_000),  // 30 jours, pas 6h
        "year"  => (now_ms - 365 * 86_400_000, 86_400_000),  // 1 an, pas 1 jour
        _       => (now_ms - 86_400_000,        900_000),    // 24h, pas 15min
    };

    // ── Requêtes PromQL en parallèle ────────────────────────────────────────────
    let (solar_w_r, import_w_r, consumption_w_r,
         solar_energy_r, import_energy_r, export_energy_r, consumption_energy_r,
         charge_ah_r, discharge_ah_r) = tokio::join!(
        // W series
        tsink.query_range("solar_total_w".into(),                               start_ms, now_ms, step_ms),
        tsink.query_range("et112_power_w{address=\"0x09\"}".into(),             start_ms, now_ms, step_ms),
        tsink.query_range("et112_power_w{address=\"0x08\"}".into(),             start_ms, now_ms, step_ms),
        // kWh cumul (compteurs ET112)
        tsink.query_range("et112_energy_import_wh{address=\"0x07\"}".into(),    start_ms, now_ms, step_ms),
        tsink.query_range("et112_energy_import_wh{address=\"0x09\"}".into(),    start_ms, now_ms, step_ms),
        tsink.query_range("et112_energy_export_wh{address=\"0x09\"}".into(),    start_ms, now_ms, step_ms),
        tsink.query_range("et112_energy_import_wh{address=\"0x08\"}".into(),    start_ms, now_ms, step_ms),
        // Ah
        tsink.query_range("venus_shunt_ah_charged_today".into(),                start_ms, now_ms, step_ms),
        tsink.query_range("venus_shunt_ah_discharged_today".into(),             start_ms, now_ms, step_ms),
    );

    // ── Extraction des séries ────────────────────────────────────────────────────
    let (ts_solar, solar_w)       = extract_ts_values(solar_w_r.ok());
    let (ts_import, import_w)     = extract_ts_values(import_w_r.ok());
    let (ts_consump, consump_w)   = extract_ts_values(consumption_w_r.ok());

    let solar_energy_pts   = extract_cumul(solar_energy_r.ok());
    let import_energy_pts  = extract_cumul(import_energy_r.ok());
    let export_energy_pts  = extract_cumul(export_energy_r.ok());
    let consump_energy_pts = extract_cumul(consumption_energy_r.ok());

    let (ts_charge,    charge_ah_pts)    = extract_ts_values(charge_ah_r.ok());
    let (ts_discharge, discharge_ah_pts) = extract_ts_values(discharge_ah_r.ok());

    // Totaux kWh = dernier - premier du compteur cumulatif (en kWh)
    let solar_kwh   = delta_wh_to_kwh(&solar_energy_pts);
    let import_kwh  = delta_wh_to_kwh(&import_energy_pts);
    let export_kwh  = delta_wh_to_kwh(&export_energy_pts);
    let consump_kwh = delta_wh_to_kwh(&consump_energy_pts);

    // kWh cumulatif depuis début de période (delta par rapport au premier point)
    let solar_kwh_series   = cumul_delta_kwh(&solar_energy_pts);
    let import_kwh_series  = cumul_delta_kwh(&import_energy_pts);
    let consump_kwh_series = cumul_delta_kwh(&consump_energy_pts);

    // Ah : dériver des kWh (approximation V_nom = 51.2V batterie 48V)
    let v_nom = 51.2_f64;
    let solar_ah   = solar_kwh   * 1000.0 / v_nom;
    let import_ah  = import_kwh  * 1000.0 / v_nom;
    let consump_ah = consump_kwh * 1000.0 / v_nom;

    // Ah SmartShunt (valeurs courantes)
    let charge_ah_now    = charge_ah_pts.last().copied().unwrap_or(0.0);
    let discharge_ah_now = discharge_ah_pts.last().copied().unwrap_or(0.0);

    Json(json!({
        "ok":     true,
        "period": period,

        // Timestamps ms pour chaque série
        "ts_solar_ms":       ts_solar,
        "ts_import_ms":      ts_import,
        "ts_consump_ms":     ts_consump,
        "ts_charge_ms":      ts_charge,
        "ts_discharge_ms":   ts_discharge,

        // 1. Puissance W instantanée
        "solar_w":       solar_w,
        "import_w":      import_w,
        "consump_w":     consump_w,

        // 2. Énergie kWh cumulatif depuis début de période
        "solar_kwh_series":   solar_kwh_series,
        "import_kwh_series":  import_kwh_series,
        "consump_kwh_series": consump_kwh_series,

        // 3. Ah dérivés de kWh (approx)
        "solar_ah":   round2(solar_ah),
        "import_ah":  round2(import_ah),
        "consump_ah": round2(consump_ah),

        // 4. Charge / Décharge Ah SmartShunt (courant du jour)
        "charge_ah_series":    charge_ah_pts,
        "discharge_ah_series": discharge_ah_pts,
        "charge_ah_now":    round2(charge_ah_now),
        "discharge_ah_now": round2(discharge_ah_now),

        // Totaux
        "totals": {
            "solar_kwh":   round2(solar_kwh),
            "import_kwh":  round2(import_kwh),
            "export_kwh":  round2(export_kwh),
            "consump_kwh": round2(consump_kwh),
            "charge_ah":   round2(charge_ah_now),
            "discharge_ah":round2(discharge_ah_now),
        }
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extrait timestamps (ms) et valeurs d'un résultat PromQL range.
fn extract_ts_values(result: Option<PromqlValue>) -> (Vec<i64>, Vec<f64>) {
    let series_list = match result {
        Some(PromqlValue::RangeVector(s)) => s,
        _ => return (Vec::new(), Vec::new()),
    };
    let first = match series_list.into_iter().next() {
        Some(s) => s,
        None    => return (Vec::new(), Vec::new()),
    };
    let (ts, vals): (Vec<i64>, Vec<f64>) = first.samples.into_iter()
        .map(|(t, v)| (t, round2(v)))
        .unzip();
    (ts, vals)
}

/// Extrait les valeurs brutes d'un compteur cumulatif (en Wh).
fn extract_cumul(result: Option<PromqlValue>) -> Vec<f64> {
    let series_list = match result {
        Some(PromqlValue::RangeVector(s)) => s,
        _ => return Vec::new(),
    };
    match series_list.into_iter().next() {
        Some(s) => s.samples.into_iter().map(|(_, v)| v).collect(),
        None    => Vec::new(),
    }
}

/// Delta total (dernier - premier) en kWh depuis des valeurs Wh.
fn delta_wh_to_kwh(pts: &[f64]) -> f64 {
    if pts.len() < 2 { return 0.0; }
    let first = pts.first().copied().unwrap_or(0.0);
    let last  = pts.last().copied().unwrap_or(0.0);
    if last >= first { (last - first) / 1000.0 } else { 0.0 }
}

/// Série de kWh cumulatifs depuis le début de période (delta par rapport au premier point).
fn cumul_delta_kwh(pts: &[f64]) -> Vec<f64> {
    if pts.is_empty() { return Vec::new(); }
    let base = pts[0];
    pts.iter()
        .map(|&v| round2(if v >= base { (v - base) / 1000.0 } else { 0.0 }))
        .collect()
}

#[inline]
fn round2(v: f64) -> f64 { (v * 100.0).round() / 100.0 }
