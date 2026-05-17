//! Endpoints PromQL Prometheus-compat avec **dispatch dynamique** entre
//! VictoriaMetrics (proxy HTTP historique) et `metrics-store` (shim PromQL
//! local redb).
//!
//! Le choix de backend est piloté par `[metrics_store].default_backend` :
//! - `"vm"` (défaut, comportement historique) — proxy vers VM
//! - `"redb"` — utilise `metrics_store::promql::Evaluator`
//!
//! Les routes explicites `/api/v1/redb/*` (cf. `api::redb`) restent toujours
//! redb-backed indépendamment de ce flag (utile pour debug / parité).
//!
//! Routes exposées :
//!   GET /api/v1/query        — requête instantanée
//!   GET /api/v1/query_range  — requête sur plage temporelle
//!   GET /api/v1/labels       — liste métriques (autocomplete frontend)

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Form, Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::api::redb::{self as redb_api, deser_step_ms, deser_time_ms, deser_time_ms_opt};
use crate::state::AppState;

// =============================================================================
// Paramètres de requête (alignés avec api::redb pour la cohérence)
// Acceptent secondes float (Prometheus standard, Grafana) ET ms int (interne).
// =============================================================================

#[derive(Deserialize)]
pub struct InstantQueryParams {
    pub query: String,
    /// Timestamp en secondes float (Prom std) OU millisecondes int (interne).
    #[serde(default, deserialize_with = "deser_time_ms_opt")]
    pub time: Option<i64>,
}

#[derive(Deserialize)]
pub struct RangeQueryParams {
    pub query: String,
    #[serde(deserialize_with = "deser_time_ms")]
    pub start: i64,
    #[serde(deserialize_with = "deser_time_ms")]
    pub end: i64,
    #[serde(deserialize_with = "deser_step_ms")]
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

fn use_redb(state: &AppState) -> bool {
    state.config.metrics_store.default_backend.eq_ignore_ascii_case("redb")
}

// =============================================================================
// Handlers
// =============================================================================

/// `GET /api/v1/query` — Requête PromQL instantanée.
pub async fn query_instant(
    State(state): State<AppState>,
    Query(params): Query<InstantQueryParams>,
) -> Response {
    handle_query_instant(state, params).await
}

/// `POST /api/v1/query` — Variante POST utilisée par Grafana
/// (Prometheus datasource avec `httpMethod: POST`). Les paramètres
/// arrivent dans le body `application/x-www-form-urlencoded`.
pub async fn query_instant_post(
    State(state): State<AppState>,
    Form(params): Form<InstantQueryParams>,
) -> Response {
    handle_query_instant(state, params).await
}

async fn handle_query_instant(state: AppState, params: InstantQueryParams) -> Response {
    if use_redb(&state) {
        let p = redb_api::InstantParams { query: params.query, time: params.time };
        return redb_api::run_query_instant(&state, &p).await;
    }
    let vm = match state.vm.as_ref() {
        Some(v) => v,
        None => return vm_unavailable().into_response(),
    };
    let time_ms = params.time.unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    match vm.query_instant_json(&params.query, time_ms).await {
        Ok(json) => Json(json).into_response(),
        Err(e) => vm_error(e).into_response(),
    }
}

/// `GET /api/v1/query_range` — Requête PromQL sur plage temporelle.
pub async fn query_range(
    State(state): State<AppState>,
    Query(params): Query<RangeQueryParams>,
) -> Response {
    handle_query_range(state, params).await
}

/// `POST /api/v1/query_range` — variante POST (Grafana POST mode).
pub async fn query_range_post(
    State(state): State<AppState>,
    Form(params): Form<RangeQueryParams>,
) -> Response {
    handle_query_range(state, params).await
}

async fn handle_query_range(state: AppState, params: RangeQueryParams) -> Response {
    if use_redb(&state) {
        let p = redb_api::RangeParams {
            query: params.query,
            start: params.start,
            end:   params.end,
            step:  params.step,
        };
        return redb_api::run_query_range(&state, &p).await;
    }
    let vm = match state.vm.as_ref() {
        Some(v) => v,
        None => return vm_unavailable().into_response(),
    };
    if params.start > params.end {
        return ApiError {
            status:     "error".into(),
            error:      "start must be before end".into(),
            error_type: Some("bad_data".into()),
        }
        .into_response();
    }
    if params.step <= 0 {
        return ApiError {
            status:     "error".into(),
            error:      "step must be positive".into(),
            error_type: Some("bad_data".into()),
        }
        .into_response();
    }
    match vm
        .query_range_json(&params.query, params.start, params.end, params.step)
        .await
    {
        Ok(json) => Json(json).into_response(),
        Err(e) => vm_error(e).into_response(),
    }
}

/// `GET /api/v1/labels` — Liste des métriques connues.
///
/// Backend redb : scan dynamique de `series_meta`.
/// Backend VM   : liste statique conservée (compat frontend).
pub async fn list_metrics(State(state): State<AppState>) -> Response {
    if use_redb(&state) {
        return redb_api::run_list_labels(&state).await;
    }
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
    .into_response()
}
