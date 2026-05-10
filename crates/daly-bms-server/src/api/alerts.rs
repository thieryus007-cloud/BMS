//! Endpoints REST pour le journal d'alertes (lecture depuis SQLite via AlertEngine).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use tokio::task::spawn_blocking;

use crate::state::AppState;
use crate::bridges::alerts::{AlertEventRow, AlertStats};

// =============================================================================
// Types de réponse
// =============================================================================

#[derive(Serialize)]
pub struct AlertListResponse {
    pub total:  i64,
    pub events: Vec<AlertEventRow>,
}

// =============================================================================
// Paramètres de requête
// =============================================================================

#[derive(Deserialize)]
pub struct AlertListQuery {
    /// Nombre max d'événements (défaut 100, max 500).
    limit:    Option<usize>,
    /// Décalage pour la pagination.
    offset:   Option<usize>,
    /// Filtrer par sévérité : "critical" | "warning".
    severity: Option<String>,
    /// Si `true`, ne retourner que les événements "triggered" (non effacés).
    active:   Option<bool>,
}

// =============================================================================
// Handlers
// =============================================================================

/// Liste les événements d'alerte avec pagination et filtres optionnels.
///
/// `GET /api/v1/alerts/list?limit=50&offset=0&severity=critical&active=true`
pub async fn list_alerts(
    State(state): State<AppState>,
    Query(q): Query<AlertListQuery>,
) -> Result<Json<AlertListResponse>, StatusCode> {
    let engine = state.alert_engine.as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .clone();

    let limit      = q.limit.unwrap_or(100).min(500);
    let offset     = q.offset.unwrap_or(0);
    let only_active = q.active.unwrap_or(false);
    let severity   = q.severity;

    let result = spawn_blocking(move || {
        engine.query_events(limit, offset, only_active, severity)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let (total, events) = result;
    Ok(Json(AlertListResponse { total, events }))
}

/// Retourne les compteurs agrégés.
///
/// `GET /api/v1/alerts/stats`
pub async fn get_stats(
    State(state): State<AppState>,
) -> Result<Json<AlertStats>, StatusCode> {
    let engine = state.alert_engine.as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .clone();

    let stats = spawn_blocking(move || engine.count_stats())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(stats))
}

/// Acquitte une alerte par son ID.
///
/// `POST /api/v1/alerts/:id/acknowledge`
pub async fn acknowledge_alert(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    let engine = state.alert_engine.as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .clone();

    let found = spawn_blocking(move || engine.acknowledge(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if found { Ok(StatusCode::OK) } else { Err(StatusCode::NOT_FOUND) }
}
