//! Endpoints PromQL compatibles Prometheus HTTP API.
//!
//! Proxy vers VictoriaMetrics — la réponse JSON est retournée telle quelle.
//!   GET /api/v1/query        — requête instantanée
//!   GET /api/v1/query_range  — requête sur plage temporelle

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::AppState;

// =============================================================================
// Paramètres de requête
// =============================================================================

#[derive(Deserialize)]
pub struct InstantQueryParams {
    pub query: String,
    /// Timestamp d'évaluation en millisecondes (défaut : maintenant)
    pub time: Option<i64>,
}

#[derive(Deserialize)]
pub struct RangeQueryParams {
    pub query: String,
    /// Début de plage en millisecondes
    pub start: i64,
    /// Fin de plage en millisecondes
    pub end: i64,
    /// Pas en millisecondes
    pub step: i64,
}

// =============================================================================
// Réponse d'erreur
// =============================================================================

#[derive(serde::Serialize)]
pub struct ApiError {
    status: String,
    error:  String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_type: Option<String>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}

fn vm_unavailable() -> ApiError {
    ApiError {
        status:     "error".into(),
        error:      "VictoriaMetrics is not enabled".into(),
        error_type: Some("unavailable".into()),
    }
}

fn vm_error(e: anyhow::Error) -> ApiError {
    ApiError {
        status:     "error".into(),
        error:      e.to_string(),
        error_type: Some("internal".into()),
    }
}

// =============================================================================
// Handlers
// =============================================================================

/// `GET /api/v1/query` — Requête PromQL instantanée.
///
/// Exemple : `/api/v1/query?query=bms_voltage{bms_id="0x01"}&time=1700000000000`
pub async fn query_instant(
    State(state): State<AppState>,
    Query(params): Query<InstantQueryParams>,
) -> Result<Json<Value>, ApiError> {
    let vm = state.vm.as_ref().ok_or_else(vm_unavailable)?;

    let time_ms = params.time.unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

    vm.query_instant_json(&params.query, time_ms)
        .await
        .map(Json)
        .map_err(vm_error)
}

/// `GET /api/v1/query_range` — Requête PromQL sur plage temporelle.
///
/// Exemple : `/api/v1/query_range?query=bms_soc{bms_id="0x01"}&start=1700000000000&end=1700086400000&step=60000`
pub async fn query_range(
    State(state): State<AppState>,
    Query(params): Query<RangeQueryParams>,
) -> Result<Json<Value>, ApiError> {
    let vm = state.vm.as_ref().ok_or_else(vm_unavailable)?;

    if params.start > params.end {
        return Err(ApiError {
            status:     "error".into(),
            error:      "start must be before end".into(),
            error_type: Some("bad_data".into()),
        });
    }
    if params.step <= 0 {
        return Err(ApiError {
            status:     "error".into(),
            error:      "step must be positive".into(),
            error_type: Some("bad_data".into()),
        });
    }

    vm.query_range_json(&params.query, params.start, params.end, params.step)
        .await
        .map(Json)
        .map_err(vm_error)
}

/// `GET /api/v1/labels` — Liste des métriques connues (pour autocomplete frontend).
pub async fn list_metrics(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "status": "success",
        "data": if state.vm.is_some() {
            vec![
                "bms_voltage", "bms_current", "bms_power", "bms_soc",
                "bms_capacity_ah", "bms_cell_delta_mv", "bms_temp_max", "bms_temp_min",
                "bms_charge_mos", "bms_discharge_mos", "bms_cell_voltage",
                "et112_voltage_v", "et112_current_a", "et112_power_w",
                "et112_energy_import_wh", "et112_energy_export_wh",
                "irradiance_wm2",
                "venus_shunt_voltage_v", "venus_shunt_current_a", "venus_shunt_power_w",
                "venus_shunt_soc_percent", "venus_shunt_ah_charged_today",
                "venus_inverter_power_w", "venus_inverter_voltage_v",
            ]
        } else {
            vec![]
        }
    }))
}
