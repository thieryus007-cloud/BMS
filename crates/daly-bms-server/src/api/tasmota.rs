//! Endpoints REST pour les prises connectées Tasmota.
//!
//! Routes :
//! ```text
//! GET /api/v1/tasmota                  → liste des prises configurées + dernier snapshot
//! GET /api/v1/tasmota/:id/status       → dernier snapshot d'une prise
//! GET /api/v1/tasmota/:id/history      → historique (ring buffer)
//! POST /api/v1/tasmota/:id/control     → contrôler l'état du relais (on/off)
//! ```

use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

/// GET /api/v1/tasmota — liste de toutes les prises + dernier snapshot
pub async fn list_tasmota(State(state): State<AppState>) -> Json<serde_json::Value> {
    let devices = &state.config.tasmota.devices;
    let mut result = Vec::new();
    for dev in devices {
        let snap = state.tasmota_latest_for(dev.id).await;
        result.push(serde_json::json!({
            "id":           dev.id,
            "name":         dev.name,
            "tasmota_id":   dev.tasmota_id,
            "mqtt_index":   dev.mqtt_index,
            "service_type": dev.service_type,
            "snapshot":     snap,
        }));
    }
    Json(serde_json::json!({ "tasmota": result }))
}

/// GET /api/v1/tasmota/:id/status — dernier snapshot
pub async fn get_tasmota_status(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let id   = id_str.trim().parse::<u8>().map_err(|_| StatusCode::BAD_REQUEST)?;
    let snap = state.tasmota_latest_for(id).await.ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(serde_json::to_value(&snap).unwrap_or_default()))
}

#[derive(Deserialize)]
pub struct HistoryParams {
    pub limit: Option<usize>,
}

/// GET /api/v1/tasmota/:id/history — historique complet
pub async fn get_tasmota_history(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Query(params): Query<HistoryParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let id    = id_str.trim().parse::<u8>().map_err(|_| StatusCode::BAD_REQUEST)?;
    let limit = params.limit.unwrap_or(360).min(1440);
    let snaps = state.tasmota_history_for(id, limit).await;
    Ok(Json(serde_json::json!({ "id": id, "count": snaps.len(), "history": snaps })))
}

#[derive(Deserialize)]
pub struct ControlPayload {
    pub state: String,  // "on" ou "off"
}

#[derive(Serialize)]
pub struct ControlResponse {
    pub id: u8,
    pub state: bool,
    pub command: String,
}

/// POST /api/v1/tasmota/:id/control — contrôler l'état du relais (on/off)
pub async fn control_tasmota(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<ControlPayload>,
) -> Result<Json<ControlResponse>, StatusCode> {
    let id = id_str.trim().parse::<u8>().map_err(|_| StatusCode::BAD_REQUEST)?;

    // Trouver la configuration du device Tasmota
    let device = state
        .config
        .tasmota
        .devices
        .iter()
        .find(|d| d.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;

    // Parser l'état désiré
    let power_on = matches!(payload.state.to_lowercase().as_str(), "on" | "1" | "true");
    let cmd = if power_on { "ON" } else { "OFF" };

    // Créer un client MQTT temporaire pour envoyer la commande
    let mqtt_cfg = &state.config.mqtt;
    let mqtt_topic = format!("cmnd/{}/POWER", device.tasmota_id);

    use rumqttc::{AsyncClient, MqttOptions, QoS};
    let mut mqtt_opts = MqttOptions::new(
        format!("daly-bms-control-{}", id),
        &mqtt_cfg.host,
        mqtt_cfg.port,
    );
    mqtt_opts.set_keep_alive(std::time::Duration::from_secs(30));

    // Créer le client et la boucle d'événements
    let (client, mut eventloop) = AsyncClient::new(mqtt_opts, 16);

    // Lancer la boucle d'événements en background
    tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    });

    // Publier le message de contrôle (ne pas attendre la confirmation)
    let cmd_str = cmd.to_string();
    let topic_clone = mqtt_topic.clone();
    tokio::spawn(async move {
        let _ = client.publish(&topic_clone, QoS::AtLeastOnce, false, cmd_str.as_bytes()).await;
    });

    tracing::info!(
        "[Tasmota] Commande envoyée {} → {} : {}",
        device.tasmota_id,
        mqtt_topic,
        cmd
    );

    Ok(Json(ControlResponse {
        id,
        state: power_on,
        command: format!("{}:{}", mqtt_topic, cmd),
    }))
}
