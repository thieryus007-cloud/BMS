//! Endpoints catalogue de panels & exécution des requêtes panel.
//!
//! - `GET /api/v1/dashboards/catalog`         → liste des panels disponibles
//! - `GET /api/v1/dashboards/panel/:id/data`  → exécute les PromQL du panel
//!
//! Le catalogue est chargé une seule fois au démarrage (cf. `dashboards::Catalog`)
//! et stocké dans `AppState.dashboard_catalog`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::dashboards::{Panel, PanelKind};
use crate::state::AppState;

/// User id par défaut tant qu'il n'y a pas d'auth utilisateur.
const DEFAULT_USER: &str = "default";

// ---------------------------------------------------------------------------
// GET /api/v1/dashboards/catalog
// ---------------------------------------------------------------------------

/// Retourne la liste des panels disponibles.
/// Filtre optionnel `?kind=stat|timeseries|...` pour ne retourner qu'un type.
#[derive(Deserialize)]
pub struct CatalogParams {
    pub kind: Option<String>,
    /// Si `true`, inclut les rows (sections). Défaut: false (les rows ne sont pas
    /// utiles à proposer dans la modale "Ajouter panel").
    pub include_rows: Option<bool>,
}

pub async fn get_catalog(
    State(state): State<AppState>,
    Query(q):     Query<CatalogParams>,
) -> impl IntoResponse {
    let include_rows = q.include_rows.unwrap_or(false);
    let kind_filter = q.kind.as_deref().map(PanelKind::from_grafana);

    let panels: Vec<&Panel> = state.dashboard_catalog.panels().iter()
        .filter(|p| include_rows || p.kind != PanelKind::Row)
        .filter(|p| match &kind_filter {
            Some(k) => &p.kind == k,
            None    => true,
        })
        .collect();

    Json(json!({
        "ok":    true,
        "count": panels.len(),
        "panels": panels,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/v1/dashboards/panel/:id/data
// ---------------------------------------------------------------------------

/// Paramètres de requête pour exécuter un panel.
#[derive(Deserialize)]
pub struct PanelDataParams {
    /// Début de fenêtre en epoch ms. Défaut: now - 1h.
    pub from: Option<i64>,
    /// Fin de fenêtre en epoch ms. Défaut: now.
    pub to: Option<i64>,
    /// Pas de la requête en secondes. Défaut: auto (≈ 200 points sur la fenêtre).
    pub step: Option<i64>,
}

/// Exécute les requêtes PromQL d'un panel et retourne les séries.
///
/// Format de réponse :
/// ```json
/// {
///   "ok": true,
///   "panel": { "id": "...", "title": "...", "kind": "timeseries", "unit": "watt" },
///   "from_ms": ..., "to_ms": ..., "step_ms": ...,
///   "series": [
///     { "ref_id": "A", "legend": "PV", "labels": {...}, "ts_ms": [...], "values": [...] },
///     ...
///   ]
/// }
/// ```
pub async fn get_panel_data(
    State(state): State<AppState>,
    Path(id):     Path<String>,
    Query(q):     Query<PanelDataParams>,
) -> impl IntoResponse {
    let panel = match state.dashboard_catalog.find(&id) {
        Some(p) => p.clone(),
        None => {
            return Json(json!({
                "ok":     false,
                "reason": "panel_not_found",
                "id":     id,
            }));
        }
    };

    let vm = match &state.vm {
        Some(v) => v.clone(),
        None => {
            return Json(json!({"ok": false, "reason": "vm_disabled"}));
        }
    };

    let now_ms = Utc::now().timestamp_millis();
    let to_ms   = q.to.unwrap_or(now_ms);
    let from_ms = q.from.unwrap_or(to_ms - 3_600_000); // défaut 1h
    let span_s  = ((to_ms - from_ms).max(1) / 1000) as f64;
    // Pas auto : ~200 points dans la fenêtre, borné à 5s min / 1d max
    let auto_step_s = (span_s / 200.0).clamp(5.0, 86_400.0) as i64;
    let step_s = q.step.unwrap_or(auto_step_s).max(1);
    let step_ms = step_s * 1000;

    // Rows n'ont pas de queries → réponse vide
    if panel.queries.is_empty() {
        return Json(json!({
            "ok":      true,
            "panel":   panel,
            "from_ms": from_ms,
            "to_ms":   to_ms,
            "step_ms": step_ms,
            "series":  Vec::<Value>::new(),
        }));
    }

    // Exécute toutes les queries en parallèle
    let futures = panel.queries.iter().map(|q| {
        let vm = vm.clone();
        let expr = q.expr.clone();
        let ref_id = q.ref_id.clone();
        let legend = q.legend.clone();
        async move {
            let result = vm.query_range_json(&expr, from_ms, to_ms, step_ms).await;
            (ref_id, legend, expr, result)
        }
    });
    let results: Vec<_> = futures::future::join_all(futures).await;

    let mut series = Vec::new();
    for (ref_id, legend, expr, result) in results {
        match result {
            Ok(json_val) => {
                let extracted = extract_series(&json_val);
                for (labels, ts, values) in extracted {
                    series.push(json!({
                        "ref_id": ref_id,
                        "legend": format_legend(legend.as_deref(), &labels),
                        "expr":   expr,
                        "labels": labels,
                        "ts_ms":  ts,
                        "values": values,
                    }));
                }
            }
            Err(e) => {
                tracing::warn!(panel = %panel.id, ref_id = %ref_id, expr = %expr, error = %e,
                    "panel query failed");
            }
        }
    }

    Json(json!({
        "ok":      true,
        "panel":   panel,
        "from_ms": from_ms,
        "to_ms":   to_ms,
        "step_ms": step_ms,
        "series":  series,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extrait toutes les séries du JSON VM range_query.
/// Retourne `Vec<(labels, ts_ms, values)>` — une entrée par série Prometheus.
fn extract_series(result: &Value) -> Vec<(serde_json::Map<String, Value>, Vec<i64>, Vec<f64>)> {
    let empty = vec![];
    let list = result.pointer("/data/result").and_then(|v| v.as_array()).unwrap_or(&empty);
    let mut out = Vec::with_capacity(list.len());

    for s in list {
        let labels = s.get("metric")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let values = s.get("values").and_then(|v| v.as_array());
        let Some(values) = values else { continue };
        let mut ts = Vec::with_capacity(values.len());
        let mut vs = Vec::with_capacity(values.len());
        for entry in values {
            let ts_secs = entry.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let val: f64 = entry.get(1)
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(f64::NAN);
            ts.push((ts_secs * 1000.0) as i64);
            vs.push(round4(val));
        }
        out.push((labels, ts, vs));
    }
    out
}

/// Remplace `{label}` dans un format de légende Grafana par la valeur du label.
/// Si pas de format, retourne `__name__` ou la concaténation des labels.
fn format_legend(format: Option<&str>, labels: &serde_json::Map<String, Value>) -> String {
    if let Some(fmt) = format {
        let mut result = fmt.to_string();
        for (k, v) in labels {
            let placeholder = format!("{{{{{}}}}}", k); // Grafana utilise {{label}}
            if let Some(val) = v.as_str() {
                result = result.replace(&placeholder, val);
            }
        }
        return result;
    }
    labels.get("__name__").and_then(|v| v.as_str()).unwrap_or("").to_string()
}

#[inline]
fn round4(v: f64) -> f64 {
    if v.is_nan() { return v; }
    (v * 10000.0).round() / 10000.0
}

// ---------------------------------------------------------------------------
// Layouts persistants (SQLite)
// ---------------------------------------------------------------------------

/// GET /api/v1/dashboards/layout
/// Retourne le layout sauvegardé pour l'utilisateur par défaut.
/// Format : `{ "ok": true, "layout": [...] }`. Si rien en base, `layout` est `null`
/// → le client doit utiliser le layout d'origine (positions Grafana).
pub async fn get_layout(State(state): State<AppState>) -> impl IntoResponse {
    let Some(storage) = &state.dashboard_storage else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({
            "ok": false, "reason": "storage_unavailable"
        })));
    };
    match storage.get(DEFAULT_USER) {
        Ok(None) => (StatusCode::OK, Json(json!({"ok": true, "layout": Value::Null}))),
        Ok(Some(json_str)) => {
            // On renvoie le JSON déjà parsé (et on revient sur Null si parsing échoue)
            let parsed: Value = serde_json::from_str(&json_str).unwrap_or(Value::Null);
            (StatusCode::OK, Json(json!({"ok": true, "layout": parsed})))
        }
        Err(e) => {
            tracing::warn!(error = %e, "dashboard layout get failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
                "ok": false, "reason": "db_error", "error": e.to_string()
            })))
        }
    }
}

/// POST /api/v1/dashboards/layout
/// Body attendu : `{ "layout": [{"id":"...","x":0,"y":0,"w":12,"h":8}, ...] }`
/// ou directement le tableau `[...]`.
pub async fn set_layout(
    State(state): State<AppState>,
    Json(body):   Json<Value>,
) -> impl IntoResponse {
    let Some(storage) = &state.dashboard_storage else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({
            "ok": false, "reason": "storage_unavailable"
        })));
    };

    // Accepte {"layout": [...]} ou [...] directement
    let layout = match body.get("layout") {
        Some(v) => v.clone(),
        None    => body.clone(),
    };
    if !layout.is_array() {
        return (StatusCode::BAD_REQUEST, Json(json!({
            "ok": false, "reason": "layout_not_array"
        })));
    }
    let layout_json = layout.to_string();
    // Limite raisonnable pour éviter qu'un client tente d'écrire des MB
    if layout_json.len() > 256_000 {
        return (StatusCode::PAYLOAD_TOO_LARGE, Json(json!({
            "ok": false, "reason": "layout_too_large"
        })));
    }
    match storage.set(DEFAULT_USER, &layout_json) {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))),
        Err(e) => {
            tracing::warn!(error = %e, "dashboard layout set failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
                "ok": false, "reason": "db_error", "error": e.to_string()
            })))
        }
    }
}

/// DELETE /api/v1/dashboards/layout — retour à l'état d'origine côté UI.
pub async fn delete_layout(State(state): State<AppState>) -> impl IntoResponse {
    let Some(storage) = &state.dashboard_storage else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({
            "ok": false, "reason": "storage_unavailable"
        })));
    };
    match storage.delete(DEFAULT_USER) {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))),
        Err(e) => {
            tracing::warn!(error = %e, "dashboard layout delete failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
                "ok": false, "reason": "db_error", "error": e.to_string()
            })))
        }
    }
}
