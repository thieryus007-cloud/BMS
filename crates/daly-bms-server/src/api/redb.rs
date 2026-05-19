//! Endpoints HTTP `/api/v1/redb/*` — interroge le backend `metrics-store`
//! (redb) directement via le shim PromQL. Cf. plan_migration_vm_redb.md
//! Phase 2.
//!
//! Format de réponse : compatible Prometheus HTTP API
//! (`status=success` + `data.{resultType, result}`).
//!
//! Endpoints exposés :
//! - `GET /api/v1/redb/query`            — instant
//! - `GET /api/v1/redb/query_range`      — plage temporelle
//! - `GET /api/v1/redb/series`           — catalogue de séries
//! - `GET /api/v1/redb/labels`           — noms de labels distincts
//! - `GET /api/v1/redb/label/:name/values` — valeurs distinctes d'un label
//! - `GET /api/v1/redb/healthy`          — health-check datasource Grafana

use std::collections::BTreeSet;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Form, Json,
};
use metrics_store::promql::{parse_and_validate, Evaluator, InstantSample, PromQlError, RangeSeries};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use crate::state::AppState;

// =============================================================================
// Helpers : conversion erreur → réponse JSON Prometheus
// =============================================================================

fn err_response(err: PromQlError) -> Response {
    let status = match &err {
        PromQlError::ParseError(_) | PromQlError::Unsupported(_) => StatusCode::BAD_REQUEST,
        PromQlError::Execution(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let body = Json(json!({
        "status": "error",
        "errorType": err.error_type(),
        "error": err.message(),
    }));
    (status, body).into_response()
}

fn not_configured() -> Response {
    let body = Json(json!({
        "status": "error",
        "errorType": "bad_data",
        "error": "metrics-store backend is not enabled (Config.toml [metrics_store])",
    }));
    (StatusCode::SERVICE_UNAVAILABLE, body).into_response()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Format Prometheus : timestamps en secondes (avec décimales ms), valeurs en
/// strings.
fn ts_secs(ms: i64) -> serde_json::Value {
    json!(ms as f64 / 1000.0)
}

fn val_str(v: f64) -> String {
    if v.is_nan() {
        "NaN".to_string()
    } else if v.is_infinite() {
        if v > 0.0 { "+Inf".to_string() } else { "-Inf".to_string() }
    } else {
        v.to_string()
    }
}

// =============================================================================
// /api/v1/redb/query — instant
// =============================================================================

#[derive(Deserialize)]
pub struct InstantParams {
    pub query: String,
    /// Timestamp d'évaluation. Accepte plusieurs formats (cf. `deser_time_ms`) :
    /// - Unix secondes (float, ex: "1700000000.123") — format Prometheus standard
    /// - Unix millisecondes (int, ex: "1700000000123") — notre frontend interne
    /// - RFC3339 (ex: "2026-05-17T19:00:00Z") — fallback Prometheus
    /// Défaut : maintenant si absent.
    #[serde(default, deserialize_with = "deser_time_ms_opt")]
    pub time: Option<i64>,
}

pub async fn query_instant(
    State(state): State<AppState>,
    Query(params): Query<InstantParams>,
) -> Response {
    run_query_instant(&state, &params).await
}

/// Variante POST : Grafana (Prometheus datasource) envoie les paramètres
/// dans le body application/x-www-form-urlencoded quand `httpMethod: POST`.
pub async fn query_instant_post(
    State(state): State<AppState>,
    Form(params): Form<InstantParams>,
) -> Response {
    run_query_instant(&state, &params).await
}

/// Logique pure (sans extracteurs Axum) — appelable depuis le dispatcher
/// `api/promql.rs` quand `default_backend = "redb"`.
pub async fn run_query_instant(state: &AppState, params: &InstantParams) -> Response {
    let Some(store) = state.metrics_store.clone() else {
        return not_configured();
    };
    let expr = match parse_and_validate(&params.query) {
        Ok(e) => e,
        Err(e) => return err_response(e),
    };
    let t = params.time.unwrap_or_else(now_ms);
    let reader = store.reader();
    let ev = Evaluator::new(&reader);
    let result: Vec<InstantSample> = match ev.eval_instant(&expr, t) {
        Ok(v) => v,
        Err(e) => return err_response(e),
    };
    let result_json: Vec<Value> = result
        .into_iter()
        .map(|s| {
            json!({
                "metric": &*s.labels,
                "value": [ts_secs(t), val_str(s.value)],
            })
        })
        .collect();
    Json(json!({
        "status": "success",
        "data": { "resultType": "vector", "result": result_json },
    }))
    .into_response()
}

// =============================================================================
// /api/v1/redb/query_range — plage temporelle
// =============================================================================

#[derive(Deserialize)]
pub struct RangeParams {
    pub query: String,
    /// Début de plage — secondes float OU millisecondes int (cf. `deser_time_ms`).
    #[serde(deserialize_with = "deser_time_ms")]
    pub start: i64,
    /// Fin de plage — secondes float OU millisecondes int.
    #[serde(deserialize_with = "deser_time_ms")]
    pub end: i64,
    /// Pas — secondes float OU millisecondes int OU duration "60s" / "5m" / "1h".
    #[serde(deserialize_with = "deser_step_ms")]
    pub step: i64,
}

// ── Deserializers tolérants aux formats Prometheus (s) et internes (ms) ──────

fn time_str_to_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // RFC3339 fallback
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    // Numérique (int ou float)
    let f: f64 = s.parse().ok()?;
    if f.abs() >= 1e12 {
        Some(f as i64) // déjà en ms
    } else {
        Some((f * 1000.0).round() as i64) // secondes → ms
    }
}

/// Détecte automatiquement secondes vs millisecondes par magnitude.
/// Une valeur >= 1e12 est traitée comme ms, sinon comme secondes (Prometheus).
/// Accepte aussi les strings RFC3339 ("2026-05-17T19:00:00Z").
pub fn deser_time_ms<'de, D>(d: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let v: Value = Value::deserialize(d)?;
    let s = match &v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => return Err(serde::de::Error::custom(format!("time value invalide: {other}"))),
    };
    time_str_to_ms(&s)
        .ok_or_else(|| serde::de::Error::custom(format!("time value non parsable: {s}")))
}

pub fn deser_time_ms_opt<'de, D>(d: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let v: Option<Value> = Option::deserialize(d)?;
    match v {
        None => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(time_str_to_ms(&s)),
        Some(Value::Number(n)) => Ok(time_str_to_ms(&n.to_string())),
        Some(other) => Err(serde::de::Error::custom(format!("time value invalide: {other}"))),
    }
}

/// Step accepte les mêmes formats que time, plus les durations Prometheus
/// (`30s`, `5m`, `1h`, `1d`, `1w`). Retourne le step en millisecondes.
pub fn deser_step_ms<'de, D>(d: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let v: Value = Value::deserialize(d)?;
    let s = match &v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => return Err(serde::de::Error::custom(format!("step value invalide: {other}"))),
    };
    parse_step_to_ms(s.trim())
        .ok_or_else(|| serde::de::Error::custom(format!("step value non parsable: {s}")))
}

fn parse_step_to_ms(s: &str) -> Option<i64> {
    // Duration Prometheus : ms, s, m, h, d, w
    if let Some(rest) = s.strip_suffix("ms") {
        return rest.parse::<i64>().ok();
    }
    let suffixes: &[(&str, i64)] = &[
        ("s", 1_000),
        ("m", 60 * 1_000),
        ("h", 60 * 60 * 1_000),
        ("d", 24 * 60 * 60 * 1_000),
        ("w", 7 * 24 * 60 * 60 * 1_000),
    ];
    for (suf, mult) in suffixes {
        if let Some(rest) = s.strip_suffix(suf) {
            let n: f64 = rest.parse().ok()?;
            return Some((n * (*mult as f64)).round() as i64);
        }
    }
    // Pas de suffixe → numérique (secondes float ou ms int)
    let f: f64 = s.parse().ok()?;
    if f.abs() >= 1e9 {
        Some(f as i64) // déjà en ms (step de plusieurs jours)
    } else {
        Some((f * 1000.0).round() as i64) // secondes → ms
    }
}

pub async fn query_range(
    State(state): State<AppState>,
    Query(params): Query<RangeParams>,
) -> Response {
    run_query_range(&state, &params).await
}

pub async fn query_range_post(
    State(state): State<AppState>,
    Form(params): Form<RangeParams>,
) -> Response {
    run_query_range(&state, &params).await
}

pub async fn run_query_range(state: &AppState, params: &RangeParams) -> Response {
    let Some(store) = state.metrics_store.clone() else {
        return not_configured();
    };
    let expr = match parse_and_validate(&params.query) {
        Ok(e) => e,
        Err(e) => return err_response(e),
    };
    let reader = store.reader();
    let ev = Evaluator::new(&reader);
    let series: Vec<RangeSeries> = match ev.eval_range(&expr, params.start, params.end, params.step) {
        Ok(v) => v,
        Err(e) => return err_response(e),
    };
    let result_json: Vec<Value> = series
        .into_iter()
        .map(|s| {
            let values: Vec<Value> = s
                .samples
                .into_iter()
                .map(|(ts, v)| json!([ts_secs(ts), val_str(v)]))
                .collect();
            json!({ "metric": s.labels, "values": values })
        })
        .collect();
    Json(json!({
        "status": "success",
        "data": { "resultType": "matrix", "result": result_json },
    }))
    .into_response()
}

// =============================================================================
// /api/v1/redb/series — catalogue de séries
// =============================================================================

pub async fn list_series(State(state): State<AppState>) -> Response {
    run_list_series(&state).await
}

pub async fn run_list_series(state: &AppState) -> Response {
    let Some(store) = state.metrics_store.clone() else {
        return not_configured();
    };
    let reader = store.reader();
    let series = match reader.list_series() {
        Ok(v) => v,
        Err(e) => return err_response(PromQlError::Execution(e.to_string())),
    };
    let result: Vec<Value> = series
        .into_iter()
        .map(|(_id, meta)| {
            let mut labels: serde_json::Map<String, Value> =
                serde_json::from_str(&meta.labels_json).unwrap_or_default();
            labels.insert("__name__".into(), Value::String(meta.metric));
            Value::Object(labels)
        })
        .collect();
    Json(json!({ "status": "success", "data": result })).into_response()
}

// =============================================================================
// /api/v1/redb/labels — noms de labels distincts
// =============================================================================

pub async fn list_labels(State(state): State<AppState>) -> Response {
    run_list_labels(&state).await
}

pub async fn run_list_labels(state: &AppState) -> Response {
    let Some(store) = state.metrics_store.clone() else {
        return not_configured();
    };
    let reader = store.reader();
    let series = match reader.list_series() {
        Ok(v) => v,
        Err(e) => return err_response(PromQlError::Execution(e.to_string())),
    };
    let mut labels: BTreeSet<String> = BTreeSet::new();
    labels.insert("__name__".into());
    for (_id, meta) in &series {
        if let Ok(m) = serde_json::from_str::<serde_json::Map<String, Value>>(&meta.labels_json) {
            for k in m.keys() {
                labels.insert(k.clone());
            }
        }
    }
    let data: Vec<&String> = labels.iter().collect();
    Json(json!({ "status": "success", "data": data })).into_response()
}

// =============================================================================
// /api/v1/redb/label/:name/values
// =============================================================================

pub async fn label_values(State(state): State<AppState>, Path(name): Path<String>) -> Response {
    run_label_values(&state, &name).await
}

pub async fn run_label_values(state: &AppState, name: &str) -> Response {
    let Some(store) = state.metrics_store.clone() else {
        return not_configured();
    };
    let reader = store.reader();
    let series = match reader.list_series() {
        Ok(v) => v,
        Err(e) => return err_response(PromQlError::Execution(e.to_string())),
    };
    let mut values: BTreeSet<String> = BTreeSet::new();
    for (_id, meta) in &series {
        if name == "__name__" {
            values.insert(meta.metric.clone());
            continue;
        }
        if let Ok(m) = serde_json::from_str::<serde_json::Map<String, Value>>(&meta.labels_json) {
            if let Some(Value::String(v)) = m.get(name) {
                values.insert(v.clone());
            }
        }
    }
    let data: Vec<&String> = values.iter().collect();
    Json(json!({ "status": "success", "data": data })).into_response()
}

// =============================================================================
// /api/v1/redb/healthy — health-check pour la datasource Prometheus de Grafana
// =============================================================================

pub async fn healthy(State(state): State<AppState>) -> Response {
    if state.metrics_store.is_some() {
        (StatusCode::OK, "metrics-store backend is healthy").into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "metrics-store backend not enabled",
        )
            .into_response()
    }
}
