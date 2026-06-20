/// HTTP + WebSocket server for energy-manager.
///
/// Routes:
///   GET  /live                       — WebSocket live event stream
///   GET  /health                     — health check
///   GET  /api/water-heater           — current water heater state (JSON)
///   POST /api/water-heater/mode      — set mode ("HEAT_PUMP" | "VACATION" | "TURBO")
///   GET  /api/rules-status           — aggregated rules status for monitor.html
///   GET  /api/v1/em/rules            — list loaded rules (name, origin, loaded_at)
///   POST /api/v1/em/rules/reload     — hot-reload rules from disk (body: {"name":"*"})
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

use crate::config::WaterHeaterConfig;
use crate::http_clients::lg_thinq::LgThinqClient;
use crate::rules_loader::RulesLoader;
use crate::types::{EnergyState, LiveEvent, WaterHeaterMode};
use chrono::{DateTime, Utc};

#[derive(Clone)]
struct ServerState {
    tx:          broadcast::Sender<LiveEvent>,
    state:       Arc<RwLock<EnergyState>>,
    lg:          Option<Arc<LgThinqClient>>,
    loader:      Arc<RulesLoader>,
    rule_reload: broadcast::Sender<String>,
    wh_cfg:      Arc<RwLock<WaterHeaterConfig>>,
}

pub async fn serve(
    bind: &str,
    live_tx: broadcast::Sender<LiveEvent>,
    state: Arc<RwLock<EnergyState>>,
    lg: Option<Arc<LgThinqClient>>,
    loader: Arc<RulesLoader>,
    rule_reload: broadcast::Sender<String>,
    wh_cfg: Arc<RwLock<WaterHeaterConfig>>,
) {
    let srv = ServerState { tx: live_tx, state, lg, loader, rule_reload, wh_cfg };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/live",                         get(ws_handler))
        .route("/health",                       get(health_handler))
        .route("/api/water-heater",             get(wh_status_handler))
        .route("/api/water-heater/mode",        post(wh_set_mode_handler))
        .route("/api/rules-status",             get(rules_status_handler))
        .route("/api/v1/em/rules",              get(rules_list_handler))
        .route("/api/v1/em/rules/reload",       post(rules_reload_handler))
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
    /// Seuil de température « cible atteinte » (°C) — règle 60°C par défaut.
    temp_max_c:     f64,
    /// Durée de maintien à `temp_max_c` avant de forcer VACATION (secondes).
    temp_max_hold_secs: u64,
    /// Depuis quand la cuve tient `temp_max_c` (None si en-dessous).
    temp_max_since: Option<DateTime<Utc>>,
    /// Vrai quand la cuve tient `temp_max_c` depuis ≥ `temp_max_hold_secs`
    /// → la règle force VACATION.
    temp_max_reached: bool,
}

#[derive(Serialize)]
struct ChargeCurrent {
    current_a:    Option<f64>,
    power_assist: Option<i64>,
    last_ts:      Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct DeyeCard {
    on:              bool,
    /// State-machine state: On / PendingCut / Lockout / Off / PendingRestore
    state:           Option<String>,
    last_change:     Option<DateTime<Utc>>,
    /// Restore held off because the battery is full per the MPPT charge stage (4/5/6).
    /// Sole restore gate now (no grid/SmartShunt input).
    restore_blocked: bool,
    /// Combined islanding predicate (ac_ignore != 1 && ac_connected != 0).
    /// INFORMATIONAL ONLY — no longer part of the DEYE decision (Fréquence + MPPT).
    grid_connected:  bool,
    /// AC-out frequency driving the cut/restore thresholds (Hz)
    freq_hz:         Option<f64>,
    /// Physical grid connection (ActiveIn/Connected): 1=connected, 0=outage
    ac_connected:    Option<i64>,
    /// MPPT charge stage signals a full battery (the MPPT-based cut driver)
    mppt_full:       bool,
    /// MPPT solar-charger State codes (3=Bulk, 4=Absorption, 5=Float, 6=Storage…)
    mppt_273_state:  Option<i64>,
    mppt_289_state:  Option<i64>,
}

#[derive(Serialize)]
struct RulesStatus {
    water_heater:   WaterHeaterCard,
    charge_current: ChargeCurrent,
    deye:           DeyeCard,
    /// SmartShunt SOC used by the water heater rule engine (None = not yet received from MQTT)
    soc_pct:        Option<f64>,
    /// Irradiance W/m² used by the water heater rule engine (None = not yet received)
    irradiance_wm2: Option<f64>,
    /// ac_ignore flag from VEBus (0 = grid connected, 1 = grid ignored/off-grid)
    ac_ignore:      Option<i64>,
}

async fn rules_status_handler(State(srv): State<ServerState>) -> Response {
    let wh_cfg = srv.wh_cfg.read().await.clone();
    let s = srv.state.read().await;
    // « Température cible atteinte » : la cuve tient temp_max_c depuis ≥ hold_secs.
    // Recalculé ici à partir de l'horodatage tenu par le control_task. On exige
    // aussi que la température actuelle soit TOUJOURS ≥ temp_max_c : si elle est
    // déjà redescendue (poller plus rapide que le tick control_task qui efface
    // l'horodatage), la carte ne doit pas afficher « cible atteinte » à tort.
    let temp_max_reached = s.water_heater_temp_max_since
        .map(|since| {
            s.water_heater_temp_c.map(|t| t >= wh_cfg.temp_max_c).unwrap_or(false)
                && (Utc::now() - since).num_seconds().max(0) as u64 >= wh_cfg.temp_max_hold_secs
        })
        .unwrap_or(false);
    let status = RulesStatus {
        water_heater: WaterHeaterCard {
            mode:           s.water_heater_mode.to_lg_str().to_string(),
            current_temp_c: s.water_heater_temp_c,
            target_temp_c:  s.water_heater_target_c,
            last_read_ts:   s.water_heater_last_read,
            last_change_ts: s.water_heater_last_change,
            send_count:     s.water_heater_send_count,
            lg_enabled:     srv.lg.is_some(),
            temp_max_c:     wh_cfg.temp_max_c,
            temp_max_hold_secs: wh_cfg.temp_max_hold_secs,
            temp_max_since: s.water_heater_temp_max_since,
            temp_max_reached,
        },
        charge_current: ChargeCurrent {
            current_a:    s.last_charge_current_a,
            power_assist: s.last_power_assist,
            last_ts:      s.last_charge_ts,
        },
        deye: DeyeCard {
            on:              s.deye_on,
            state:           s.deye_state.clone(),
            last_change:     s.deye_last_change,
            restore_blocked: s.deye_restore_blocked,
            grid_connected:  crate::logic::deye_command::is_grid_connected(s.ac_ignore, s.ac_connected),
            freq_hz:         s.ac_frequency_hz,
            ac_connected:    s.ac_connected,
            mppt_full:       s.deye_mppt_full,
            mppt_273_state:  s.mppt_273.state,
            mppt_289_state:  s.mppt_289.state,
        },
        soc_pct:        s.soc_pct,
        irradiance_wm2: s.irradiance_wm2,
        ac_ignore:      s.ac_ignore,
    };
    Json(status).into_response()
}

// ---------------------------------------------------------------------------
// Rules hot-reload endpoints
// ---------------------------------------------------------------------------

async fn rules_list_handler(State(srv): State<ServerState>) -> Response {
    Json(srv.loader.info()).into_response()
}

#[derive(Deserialize)]
struct ReloadRequest {
    #[serde(default = "default_reload_name")]
    name: String,
}
fn default_reload_name() -> String { "*".to_string() }

async fn rules_reload_handler(
    State(srv): State<ServerState>,
    body: Option<Json<ReloadRequest>>,
) -> Response {
    let name = body.map(|b| b.name.clone()).unwrap_or_else(|| "*".to_string());
    srv.rule_reload.send(name.clone()).ok();
    info!("Rules hot-reload triggered: {name}");
    (StatusCode::OK, format!("reload triggered: {name}")).into_response()
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
