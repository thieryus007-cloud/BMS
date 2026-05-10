//! Endpoint REST : statut MQTT broker (rumqttd) + bridge NanoPi.

use crate::state::AppState;
use axum::{extract::State, Json};

/// GET /api/v1/mqtt/status
///
/// Retourne le dernier statut connu du broker MQTT local (rumqttd)
/// et du bridge bidirectionnel Pi5 ↔ NanoPi.
/// Mis à jour toutes les 30 s par le monitor agent.
pub async fn get_mqtt_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let s = state.mqtt_status_latest().await;
    Json(serde_json::json!({
        "broker": {
            "running":      s.broker_running,
            "uptime_secs":  s.broker_uptime_secs,
            "messages_rx":  s.broker_msgs_rx,
        },
        "bridge": {
            "local_connected":      s.bridge_local_ok,
            "remote_connected":     s.bridge_remote_ok,
            "msgs_local_to_remote": s.bridge_l2r_total,
            "msgs_remote_to_local": s.bridge_r2l_total,
            "reconnects_local":     s.bridge_reconnects_local,
            "reconnects_remote":    s.bridge_reconnects_remote,
            "uptime_secs":          s.bridge_uptime_secs,
        },
        "timestamp": s.timestamp.map(|t| t.to_rfc3339()),
    }))
}
