//! Endpoints REST pour les compteurs Shelly Pro 2PM.
//!
//! Routes :
//! ```text
//! GET /api/v1/shelly                           → liste + dernier snapshot
//! GET /api/v1/shelly/:id/status                → dernier snapshot
//! POST /api/v1/shelly/:id/channel/:ch/control  → activer / désactiver un canal
//! ```

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

/// GET /api/v1/shelly
pub async fn list_shelly(State(state): State<AppState>) -> Json<serde_json::Value> {
    let devices = &state.config.shelly.devices;
    let mut result = Vec::new();
    for dev in devices {
        let snap = state.shelly_latest_for(dev.id).await;
        result.push(serde_json::json!({
            "id":       dev.id,
            "name":     dev.name,
            "shelly_id": dev.shelly_id,
            "snapshot": snap,
        }));
    }
    Json(serde_json::json!({ "shelly": result }))
}

/// GET /api/v1/shelly/:id/status
pub async fn get_shelly_status(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let id   = id_str.trim().parse::<u8>().map_err(|_| StatusCode::BAD_REQUEST)?;
    let snap = state.shelly_latest_for(id).await.ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(serde_json::to_value(&snap).unwrap_or_default()))
}

#[derive(Deserialize)]
pub struct ChannelControlPayload {
    pub state: String, // "on" ou "off"
}

/// POST /api/v1/shelly/:id/channel/:ch/control
pub async fn control_shelly_channel(
    State(state): State<AppState>,
    Path((id_str, ch_str)): Path<(String, String)>,
    Json(payload): Json<ChannelControlPayload>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let id: u8 = id_str.trim().parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    let ch: u8 = ch_str.trim().parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    if ch > 1 { return Err(StatusCode::BAD_REQUEST); }

    let device = state
        .config
        .shelly
        .devices
        .iter()
        .find(|d| d.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let on = matches!(payload.state.to_lowercase().as_str(), "on" | "1" | "true");
    let shelly_id = device.shelly_id.clone();
    let rpc_topic = format!("{}/rpc", shelly_id);

    let rpc_payload = serde_json::json!({
        "id": 1,
        "src": "daly-bms",
        "method": "Switch.Set",
        "params": { "id": ch, "on": on }
    });

    let mqtt_cfg = &state.config.mqtt;
    use rumqttc::{AsyncClient, MqttOptions, QoS};
    let mut opts = MqttOptions::new(
        format!("daly-bms-shelly-ctrl-{id}-{ch}"),
        &mqtt_cfg.host,
        mqtt_cfg.port,
    );
    opts.set_keep_alive(std::time::Duration::from_secs(10));
    if let (Some(u), Some(p)) = (&mqtt_cfg.username, &mqtt_cfg.password) {
        opts.set_credentials(u, p);
    }

    let (client, mut eventloop) = AsyncClient::new(opts, 8);
    tokio::spawn(async move {
        loop { if eventloop.poll().await.is_err() { break; } }
    });

    let payload_str = rpc_payload.to_string();
    tokio::spawn(async move {
        let _ = client.publish(&rpc_topic, QoS::AtLeastOnce, false, payload_str.as_bytes()).await;
    });

    tracing::info!("[Shelly] {} ch{} → {}", shelly_id, ch, if on { "ON" } else { "OFF" });

    Ok(Json(serde_json::json!({
        "id": id,
        "channel": ch,
        "state": on,
        "command": format!("Switch.Set id={ch} on={on}"),
    })))
}
