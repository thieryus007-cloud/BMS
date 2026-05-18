//! Endpoint de santé `/health` pour supervision externe.

use axum::{extract::State, Json};
use serde_json::json;
use std::sync::atomic::Ordering;

use crate::state::AppState;

/// `GET /health` — Statut global du serveur.
pub async fn health_check(State(state): State<AppState>) -> Json<serde_json::Value> {
    let polling = state.polling_active.load(Ordering::Relaxed);
    let ws_clients = state.ws_tx.receiver_count();
    let backend_ready = state.is_query_backend_ready();

    let overall = if backend_ready && polling {
        "healthy"
    } else if !polling {
        "degraded"
    } else {
        "ok"
    };

    Json(json!({
        "status":      overall,
        "backend":     if backend_ready { "redb" } else { "none" },
        "bms_polling": if polling { "active" } else { "inactive" },
        "ws_clients":  ws_clients,
    }))
}
