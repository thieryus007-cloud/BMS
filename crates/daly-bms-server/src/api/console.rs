//! WebSocket console — streams ConsoleEvent to the browser in real-time.
//!
//! GET /ws/console
//!
//! Optional query parameters (comma-separated lists):
//!   devices = bms1,bms2,et112,smartshunt,ats,energy_manager,venus,system
//!   kinds   = mqtt_in,mqtt_out,rs485,state,error,system

use crate::console::{EventDevice, EventKind};
use crate::state::AppState;
use axum::{
    extract::{Query, State, WebSocketUpgrade},
    response::IntoResponse,
};
use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Debug, Deserialize)]
pub struct ConsoleQuery {
    pub devices: Option<String>,
    pub kinds:   Option<String>,
}

/// GET /ws/console
pub async fn ws_console(
    ws:    WebSocketUpgrade,
    Query(q): Query<ConsoleQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_console(socket, state, q))
}

async fn handle_ws_console(socket: WebSocket, state: AppState, q: ConsoleQuery) {
    let device_filter: Option<HashSet<String>> = q.devices.as_deref().map(|s| {
        s.split(',').map(|p| p.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()
    });
    let kind_filter: Option<HashSet<String>> = q.kinds.as_deref().map(|s| {
        s.split(',').map(|p| p.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()
    });

    let mut rx = state.console_bus.subscribe();
    let (mut sender, mut receiver) = socket.split();

    // Welcome message
    let welcome = serde_json::json!({
        "type": "connected",
        "message": "Console diagnostique connectée",
        "filters": {
            "devices": q.devices.as_deref().unwrap_or("all"),
            "kinds":   q.kinds.as_deref().unwrap_or("all"),
        }
    });
    let _ = sender.send(Message::Text(welcome.to_string())).await;

    loop {
        tokio::select! {
            Ok(ev) = rx.recv() => {
                // Apply device filter
                if let Some(ref df) = device_filter {
                    let dev_str = device_to_str(&ev.device);
                    if !df.contains(dev_str) {
                        continue;
                    }
                }
                // Apply kind filter
                if let Some(ref kf) = kind_filter {
                    let kind_str = kind_to_str(&ev.kind);
                    if !kf.contains(kind_str) {
                        continue;
                    }
                }

                if let Ok(json) = serde_json::to_string(&*ev) {
                    if sender.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
            }
            Some(msg) = receiver.next() => {
                match msg {
                    Ok(Message::Close(_)) | Err(_) => break,
                    _ => {}
                }
            }
        }
    }
}

fn device_to_str(d: &EventDevice) -> &'static str {
    match d {
        EventDevice::Bms1        => "bms1",
        EventDevice::Bms2        => "bms2",
        EventDevice::Et112       => "et112",
        EventDevice::Ats         => "ats",
        EventDevice::Irradiance  => "irradiance",
        EventDevice::SmartShunt  => "smartshunt",
        EventDevice::Tasmota     => "tasmota",
        EventDevice::Shelly      => "shelly",
        EventDevice::WaterHeater => "water_heater",
        EventDevice::Solar       => "solar",
        EventDevice::Inverter    => "inverter",
        EventDevice::Venus       => "venus",
        EventDevice::System      => "system",
    }
}

fn kind_to_str(k: &EventKind) -> &'static str {
    match k {
        EventKind::MqttIn  => "mqtt_in",
        EventKind::MqttOut => "mqtt_out",
        EventKind::Rs485   => "rs485",
        EventKind::State   => "state",
        EventKind::Error   => "error",
        EventKind::System  => "system",
    }
}
