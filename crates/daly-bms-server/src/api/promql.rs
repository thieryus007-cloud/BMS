//! Endpoints PromQL Prometheus-compat servis exclusivement par le shim
//! `metrics-store` (redb) via `AppState::dispatched_query_*`.
//!
//! Post-Phase 5 cleanup : plus de dispatcher vm/redb — VM est retiré.
//! Le flag `[metrics_store].default_backend` n'existe plus.
//!
//! Routes (GET + POST tous les deux — Grafana httpMethod=POST par défaut) :
//!   /api/v1/query        — instant
//!   /api/v1/query_range  — range
//!   /api/v1/labels       — labels distincts (scan dynamique `series_meta`)

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
// Réponse d'erreur (format Prometheus)
// =============================================================================

#[derive(serde::Serialize)]
pub struct ApiError {
    status: String,
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_type: Option<String>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}

fn backend_unavailable() -> ApiError {
    ApiError {
        status: "error".into(),
        error: "metrics-store backend not enabled".into(),
        error_type: Some("unavailable".into()),
    }
}

fn exec_error(e: anyhow::Error) -> ApiError {
    ApiError {
        status: "error".into(),
        error: e.to_string(),
        error_type: Some("execution".into()),
    }
}

// =============================================================================
// Handlers — GET + POST partagent la même logique via Form/Query
// =============================================================================

pub async fn query_instant(
    State(state): State<AppState>,
    Query(params): Query<InstantQueryParams>,
) -> Response {
    handle_query_instant(state, params).await
}

pub async fn query_instant_post(
    State(state): State<AppState>,
    Form(params): Form<InstantQueryParams>,
) -> Response {
    handle_query_instant(state, params).await
}

async fn handle_query_instant(state: AppState, params: InstantQueryParams) -> Response {
    if !state.is_query_backend_ready() {
        return backend_unavailable().into_response();
    }
    let time_ms = params
        .time
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    match state.dispatched_query_instant(&params.query, time_ms).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => exec_error(e).into_response(),
    }
}

pub async fn query_range(
    State(state): State<AppState>,
    Query(params): Query<RangeQueryParams>,
) -> Response {
    handle_query_range(state, params).await
}

pub async fn query_range_post(
    State(state): State<AppState>,
    Form(params): Form<RangeQueryParams>,
) -> Response {
    handle_query_range(state, params).await
}

async fn handle_query_range(state: AppState, params: RangeQueryParams) -> Response {
    if !state.is_query_backend_ready() {
        return backend_unavailable().into_response();
    }
    if params.start > params.end {
        return ApiError {
            status: "error".into(),
            error: "start must be before end".into(),
            error_type: Some("bad_data".into()),
        }
        .into_response();
    }
    if params.step <= 0 {
        return ApiError {
            status: "error".into(),
            error: "step must be positive".into(),
            error_type: Some("bad_data".into()),
        }
        .into_response();
    }
    match state
        .dispatched_query_range(&params.query, params.start, params.end, params.step)
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(e) => exec_error(e).into_response(),
    }
}

/// `GET /api/v1/labels` — Liste les labels distincts via scan du
/// catalogue `series_meta` (délègue à `api::redb::run_list_labels`).
pub async fn list_metrics(State(state): State<AppState>) -> Response {
    if !state.is_query_backend_ready() {
        return Json(json!({"status": "error", "error": "metrics-store backend not enabled"}))
            .into_response();
    }
    redb_api::run_list_labels(&state).await
}
