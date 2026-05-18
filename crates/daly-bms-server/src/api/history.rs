//! Endpoint historique énergie — GET /api/v1/history/energy?period=day|week|month|year
//!
//! Sources VictoriaMetrics :
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
use serde_json::{json, Value};

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

    if state.vm.is_none() && state.metrics_store.is_none() {
        return Json(json!({"ok": false, "reason": "no_query_backend"}));
    }

    let now_ms = Utc::now().timestamp_millis();

    // step = résolution d'un bucket ; window = fenêtre PromQL pour agréger
    let (start_ms, step_ms, window): (i64, i64, &str) = match period {
        "week"  => (now_ms -   7 * 86_400_000,  86_400_000,    "1d"),
        "month" => (now_ms -  30 * 86_400_000,  86_400_000,    "1d"),
        "year"  => (now_ms - 365 * 86_400_000,  2_592_000_000, "30d"),
        _       => (now_ms -       86_400_000,  3_600_000,     "1h"),  // day
    };

    // ── Requêtes PromQL avec fenêtres d'agrégation ──────────────────────────────
    let q_solar_w        = format!("avg_over_time(solar_total_w[{}])", window);
    let q_import_w       = format!("avg_over_time(et112_power_w{{address=\"0x09\"}}[{}])", window);
    let q_consump_w      = format!("avg_over_time(et112_power_w{{address=\"0x08\"}}[{}])", window);
    let q_solar_energy   = format!("increase(et112_energy_import_wh{{address=\"0x07\"}}[{}])", window);
    let q_import_energy  = format!("increase(et112_energy_import_wh{{address=\"0x09\"}}[{}])", window);
    let q_export_energy  = format!("increase(et112_energy_export_wh{{address=\"0x09\"}}[{}])", window);
    let q_consump_energy = format!("increase(et112_energy_import_wh{{address=\"0x08\"}}[{}])", window);
    let q_charge_ah      = format!("max_over_time(venus_shunt_ah_charged_today[{}])", window);
    let q_discharge_ah   = format!("max_over_time(venus_shunt_ah_discharged_today[{}])", window);

    let (solar_w_r, import_w_r, consumption_w_r,
         solar_energy_r, import_energy_r, export_energy_r, consumption_energy_r,
         charge_ah_r, discharge_ah_r) = tokio::join!(
        state.dispatched_query_range(&q_solar_w,        start_ms, now_ms, step_ms),
        state.dispatched_query_range(&q_import_w,       start_ms, now_ms, step_ms),
        state.dispatched_query_range(&q_consump_w,      start_ms, now_ms, step_ms),
        state.dispatched_query_range(&q_solar_energy,   start_ms, now_ms, step_ms),
        state.dispatched_query_range(&q_import_energy,  start_ms, now_ms, step_ms),
        state.dispatched_query_range(&q_export_energy,  start_ms, now_ms, step_ms),
        state.dispatched_query_range(&q_consump_energy, start_ms, now_ms, step_ms),
        state.dispatched_query_range(&q_charge_ah,      start_ms, now_ms, step_ms),
        state.dispatched_query_range(&q_discharge_ah,   start_ms, now_ms, step_ms),
    );

    // ── Extraction ──────────────────────────────────────────────────────────────
    let (ts_solar,   solar_w)   = extract_ts_values(solar_w_r.ok());
    let (ts_import,  import_w)  = extract_ts_values(import_w_r.ok());
    let (ts_consump, consump_w) = extract_ts_values(consumption_w_r.ok());

    // increase() retourne le delta Wh par fenêtre — conversion directe en kWh/bucket
    let (_, solar_energy_wh)    = extract_ts_values(solar_energy_r.ok());
    let (_, import_energy_wh)   = extract_ts_values(import_energy_r.ok());
    let (_, export_energy_wh)   = extract_ts_values(export_energy_r.ok());
    let (_, consump_energy_wh)  = extract_ts_values(consumption_energy_r.ok());

    let solar_kwh_series:   Vec<f64> = solar_energy_wh.iter().map(|&v| round2(v.max(0.0) / 1000.0)).collect();
    let import_kwh_series:  Vec<f64> = import_energy_wh.iter().map(|&v| round2(v.max(0.0) / 1000.0)).collect();
    let consump_kwh_series: Vec<f64> = consump_energy_wh.iter().map(|&v| round2(v.max(0.0) / 1000.0)).collect();

    let solar_kwh  = round2(solar_energy_wh.iter().map(|&v| v.max(0.0)).sum::<f64>() / 1000.0);
    let import_kwh = round2(import_energy_wh.iter().map(|&v| v.max(0.0)).sum::<f64>() / 1000.0);
    let export_kwh = round2(export_energy_wh.iter().map(|&v| v.max(0.0)).sum::<f64>() / 1000.0);
    let consump_kwh= round2(consump_energy_wh.iter().map(|&v| v.max(0.0)).sum::<f64>() / 1000.0);

    // max_over_time donne le total journalier (compteur SmartShunt remis à 0 à minuit)
    let (ts_charge,    charge_ah_pts)    = extract_ts_values(charge_ah_r.ok());
    let (ts_discharge, discharge_ah_pts) = extract_ts_values(discharge_ah_r.ok());

    let charge_ah_total    = round2(charge_ah_pts.iter().map(|&v| v.max(0.0)).sum::<f64>());
    let discharge_ah_total = round2(discharge_ah_pts.iter().map(|&v| v.max(0.0)).sum::<f64>());

    Json(json!({
        "ok":     true,
        "period": period,

        "ts_solar_ms":      ts_solar,
        "ts_import_ms":     ts_import,
        "ts_consump_ms":    ts_consump,
        "ts_charge_ms":     ts_charge,
        "ts_discharge_ms":  ts_discharge,

        "solar_w":      solar_w,
        "import_w":     import_w,
        "consump_w":    consump_w,

        "solar_kwh_series":   solar_kwh_series,
        "import_kwh_series":  import_kwh_series,
        "consump_kwh_series": consump_kwh_series,

        "charge_ah_series":    charge_ah_pts,
        "discharge_ah_series": discharge_ah_pts,
        "charge_ah_now":    charge_ah_total,
        "discharge_ah_now": discharge_ah_total,

        "totals": {
            "solar_kwh":   solar_kwh,
            "import_kwh":  import_kwh,
            "export_kwh":  export_kwh,
            "consump_kwh": consump_kwh,
            "charge_ah":   charge_ah_total,
            "discharge_ah":discharge_ah_total,
        }
    }))
}

// ---------------------------------------------------------------------------
// Helpers — parsing du JSON VM (timestamps en secondes → ms pour compatibilité frontend)
// ---------------------------------------------------------------------------

/// Extrait timestamps (ms) et valeurs depuis la réponse JSON range de VM.
/// VM retourne des timestamps en secondes (float) — on multiplie par 1000.
fn extract_ts_values(result: Option<Value>) -> (Vec<i64>, Vec<f64>) {
    let result = match result { Some(v) => v, None => return (Vec::new(), Vec::new()) };
    let empty = vec![];
    let series_list = result["data"]["result"].as_array().unwrap_or(&empty);
    let first = match series_list.first() { Some(s) => s, None => return (Vec::new(), Vec::new()) };
    let values = match first["values"].as_array() { Some(v) => v, None => return (Vec::new(), Vec::new()) };

    values.iter().map(|entry| {
        let ts_secs = entry[0].as_f64().unwrap_or(0.0);
        let val: f64 = entry[1].as_str().unwrap_or("0").parse().unwrap_or(0.0);
        ((ts_secs * 1000.0) as i64, round2(val))
    }).unzip()
}


#[inline]
fn round2(v: f64) -> f64 { (v * 100.0).round() / 100.0 }
