/// HTTP + WebSocket server for energy-manager.
///
/// Routes:
///   GET  /live               — WebSocket live event stream
///   GET  /health             — health check
///   GET  /api/water-heater   — current water heater state (JSON)
///   POST /api/water-heater/mode   — set mode ("HEAT_PUMP" | "VACATION" | "TURBO")
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Json, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use crate::http_clients::lg_thinq::LgThinqClient;
use crate::types::{EnergyState, LiveEvent, WaterHeaterMode};
use chrono::{DateTime, Utc};

#[derive(Clone)]
struct ServerState {
    tx:       broadcast::Sender<LiveEvent>,
    state:    Arc<RwLock<EnergyState>>,
    lg:       Option<Arc<LgThinqClient>>,
}

pub async fn serve(
    bind: &str,
    live_tx: broadcast::Sender<LiveEvent>,
    state: Arc<RwLock<EnergyState>>,
    lg: Option<Arc<LgThinqClient>>,
) {
    let srv = ServerState { tx: live_tx, state, lg };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/live",                     get(ws_handler))
        .route("/health",                   get(health_handler))
        .route("/api/water-heater",         get(wh_status_handler))
        .route("/api/water-heater/mode",    post(wh_set_mode_handler))
        .route("/api/rules-status",         get(rules_status_handler))
        .with_state(srv)
        .layer(cors);

    let addr: SocketAddr = bind.parse().unwrap_or_else(|_| "0.0.0.0:8081".parse().unwrap());
    info!("Energy-manager HTTP server listening on {addr}");

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Cannot bind {addr}: {e}");
            return;
        }
    };
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!("HTTP server error: {e}");
    }
}

// ---------------------------------------------------------------------------
// Water heater REST handlers
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct WaterHeaterStatus {
    mode:             String,
    current_temp_c:   Option<f64>,
    target_temp_c:    Option<f64>,
    lg_enabled:       bool,
}

async fn wh_status_handler(State(srv): State<ServerState>) -> Response {
    let s = srv.state.read().await;
    let status = WaterHeaterStatus {
        mode:           s.water_heater_mode.to_lg_str().to_string(),
        current_temp_c: s.water_heater_temp_c,
        target_temp_c:  s.water_heater_target_c,
        lg_enabled:     srv.lg.is_some(),
    };
    Json(status).into_response()
}

#[derive(Deserialize)]
struct SetModeRequest {
    mode: String,
}

async fn wh_set_mode_handler(
    State(srv): State<ServerState>,
    Json(body): Json<SetModeRequest>,
) -> Response {
    let Some(lg) = srv.lg else {
        return (StatusCode::SERVICE_UNAVAILABLE, "LG ThinQ not configured").into_response();
    };
    let mode = WaterHeaterMode::from_lg_str(&body.mode);
    if let Err(e) = lg.set_mode(mode).await {
        return (StatusCode::BAD_GATEWAY, format!("LG error: {e}")).into_response();
    }
    {
        let mut s = srv.state.write().await;
        s.water_heater_mode = mode;
    }
    (StatusCode::OK, "ok").into_response()
}

// ---------------------------------------------------------------------------
// Rules status — aggregated data for monitor.html cards
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct WaterHeaterCard {
    mode:           String,
    current_temp_c: Option<f64>,
    target_temp_c:  Option<f64>,
    last_read_ts:   Option<DateTime<Utc>>,
    last_change_ts: Option<DateTime<Utc>>,
    send_count:     u32,
    lg_enabled:     bool,
}

#[derive(Serialize)]
struct ChargeCurrent {
    current_a:    Option<f64>,
    power_assist: Option<i64>,
    last_ts:      Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct DeyeCard {
    on:            bool,
    last_change:   Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct RulesStatus {
    water_heater:   WaterHeaterCard,
    charge_current: ChargeCurrent,
    deye:           DeyeCard,
}

async fn rules_status_handler(State(srv): State<ServerState>) -> Response {
    let s = srv.state.read().await;
    let status = RulesStatus {
        water_heater: WaterHeaterCard {
            mode:           s.water_heater_mode.to_lg_str().to_string(),
            current_temp_c: s.water_heater_temp_c,
            target_temp_c:  s.water_heater_target_c,
            last_read_ts:   s.water_heater_last_read,
            last_change_ts: s.water_heater_last_change,
            send_count:     s.water_heater_send_count,
            lg_enabled:     srv.lg.is_some(),
        },
        charge_current: ChargeCurrent {
            current_a:    s.last_charge_current_a,
            power_assist: s.last_power_assist,
            last_ts:      s.last_charge_ts,
        },
        deye: DeyeCard {
            on:          s.deye_on,
            last_change: s.deye_last_change,
        },
    };
    Json(status).into_response()
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

async fn health_handler() -> &'static str {
    "energy-manager ok"
}

// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<ServerState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state.tx))
}

async fn handle_socket(mut socket: WebSocket, tx: broadcast::Sender<LiveEvent>) {
    let mut rx = tx.subscribe();
    loop {
        match rx.recv().await {
            Ok(event) => {
                let json = match serde_json::to_string(&event) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                if socket.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("WebSocket client lagged {n} events");
            }
            Err(_) => break,
        }
    }
}
