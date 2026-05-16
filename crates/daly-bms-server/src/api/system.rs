//! Endpoints système : status global, config, découverte.

use crate::state::AppState;
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

/// GET /api/v1/system/status
///
/// Retourne l'état global : BMS connectés, polling actif, clients WS.
pub async fn get_status(State(state): State<AppState>) -> Json<Value> {
    let buffers = state.buffers.read().await;
    let bms_list: Vec<Value> = buffers
        .iter()
        .map(|(addr, buf)| {
            let online = buf.latest().is_some();
            let last_ts = buf.latest().map(|s| s.timestamp.to_rfc3339());
            json!({
                "address": format!("{:#04x}", addr),
                "online": online,
                "last_update": last_ts,
                "snapshots_count": buf.buffer.len(),
            })
        })
        .collect();

    Json(json!({
        "polling_active": state.polling_active.load(Ordering::Relaxed),
        "bms_count": bms_list.len(),
        "bms": bms_list,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// GET /api/v1/config
///
/// Retourne la configuration active (sans secrets).
pub async fn get_config(State(state): State<AppState>) -> Json<Value> {
    let cfg = &state.config;
    let addresses = cfg.bms_addresses()
        .iter()
        .map(|a| format!("{:#04x}", a))
        .collect::<Vec<_>>();

    Json(json!({
        "serial": {
            "port": cfg.serial.port,
            "baud": cfg.serial.baud,
            "poll_interval_ms": cfg.serial.poll_interval_ms,
        },
        "api": {
            "bind": cfg.api.bind,
            "auth_enabled": !cfg.api.api_key.is_empty(),
        },
        "addresses": addresses,
        "mqtt_enabled": cfg.mqtt.enabled,
        "vm_enabled": cfg.victoriametrics.enabled,
        "read_only": cfg.read_only.enabled,
    }))
}

/// GET /api/v1/system/logs?limit=N
///
/// Retourne les dernières entrées de logs capturées en mémoire (max 200).
#[derive(Deserialize)]
pub struct LogsQuery {
    pub limit: Option<usize>,
}

pub async fn get_logs(
    State(state): State<AppState>,
    Query(params): Query<LogsQuery>,
) -> Json<Value> {
    let limit = params.limit.unwrap_or(100).min(200);
    let buf = state.log_buffer.lock().unwrap();
    let logs: Vec<_> = buf.iter().rev().take(limit).collect();
    Json(json!({ "logs": logs, "total": buf.len() }))
}

/// GET /api/v1/irradiance/status
///
/// Retourne la dernière mesure du capteur d'irradiance PRALRAN.
pub async fn get_irradiance_status(State(state): State<AppState>) -> impl IntoResponse {
    let kwh    = *state.mppt_yield_kwh.read().await;
    let dc_pv  = *state.dc_pv_power_w.read().await;
    let pvinv  = *state.pvinv_power_w.read().await;
    let solw   = *state.solar_total_w.read().await;
    let housew = *state.house_power_w.read().await;
    match state.latest_irradiance().await {
        Some(snap) => (
            StatusCode::OK,
            Json(json!({
                "connected": true,
                "address": format!("{:#04x}", snap.address),
                "name": snap.name,
                "irradiance_wm2": snap.irradiance_wm2,
                "timestamp": snap.timestamp.to_rfc3339(),
                "total_yield_kwh": kwh,
                "dc_pv_power_w":   dc_pv,
                "pvinv_power_w":   pvinv,
                "mppt_power_w":    dc_pv,
                "solar_total_w":   solw,
                "house_power_w":   housew,
            })),
        ),
        None => (
            StatusCode::OK,
            Json(json!({
                "connected": false,
                "irradiance_wm2": 0.0,
                "total_yield_kwh": kwh,
                "dc_pv_power_w":   dc_pv,
                "pvinv_power_w":   pvinv,
                "mppt_power_w":    dc_pv,
                "solar_total_w":   solw,
                "house_power_w":   housew,
            })),
        ),
    }
}

/// Corps de la requête POST /api/v1/solar/mppt-yield
#[derive(Deserialize, Serialize)]
pub struct MpptYieldBody {
    /// Production solaire totale aujourd'hui en kWh (MPPT + ET112 delta).
    pub total_yield_kwh: Option<f32>,
    /// Rétrocompat ancien nom de champ.
    pub mppt_yield_kwh:  Option<f32>,
    /// Puissance MPPT agrégée en W = N/.../system/0/Dc/Pv/Power (alias de dc_pv_power_w).
    pub mppt_power_w:    Option<f32>,
    /// Puissance MPPT agrégée en W = N/.../system/0/Dc/Pv/Power (source de vérité).
    pub dc_pv_power_w:   Option<f32>,
    /// Puissance ET112 micro-onduleurs en W = N/.../pvinverter/32/Ac/Power.
    pub pvinv_power_w:   Option<f32>,
    /// Puissance solaire totale en W = dc_pv_power_w + pvinv_power_w.
    pub solar_total_w:   Option<f32>,
    /// Puissance maison en W = N/c0619ab9929a/system/0/Ac/ConsumptionOnOutput/L1/Power.
    pub house_power_w:   Option<f32>,
}

/// POST /api/v1/solar/mppt-yield
///
/// Mise à jour partielle : seuls les champs présents dans le body sont écrits.
/// energy-manager envoie solar_total_w + dc_pv_power_w + pvinv_power_w + mppt_power_w.
pub async fn set_mppt_yield(
    State(state): State<AppState>,
    Json(body): Json<MpptYieldBody>,
) -> impl IntoResponse {
    if let Some(kwh) = body.total_yield_kwh.or(body.mppt_yield_kwh) {
        *state.mppt_yield_kwh.write().await = kwh;
    }
    // dc_pv_power_w est prioritaire ; mppt_power_w est l'alias rétrocompat
    if let Some(v) = body.dc_pv_power_w.or(body.mppt_power_w) {
        *state.dc_pv_power_w.write().await = v;
        *state.mppt_power_w.write().await  = v;
    }
    if let Some(v) = body.pvinv_power_w {
        *state.pvinv_power_w.write().await = v;
    }
    if let Some(tw) = body.solar_total_w {
        *state.solar_total_w.write().await = tw;
    }
    if let Some(hw) = body.house_power_w {
        *state.house_power_w.write().await = hw;
    }

    // Write solar metrics to VictoriaMetrics — throttlé à 1 écriture / 5s
    if let Some(vm) = &state.vm {
        if state.vm_solar_rate_ok() {
            let kwh_val    = *state.mppt_yield_kwh.read().await;
            let dc_pv_val  = *state.dc_pv_power_w.read().await;
            let pvinv_val  = *state.pvinv_power_w.read().await;
            let total_w    = *state.solar_total_w.read().await;
            let rows = crate::vm_client::VmClient::solar_rows(total_w, dc_pv_val, pvinv_val, kwh_val);
            if let Err(e) = vm.write_rows(rows).await {
                tracing::warn!("VictoriaMetrics solar write failed: {e}");
            }
        }
    }

    let kwh    = *state.mppt_yield_kwh.read().await;
    let dc_pv  = *state.dc_pv_power_w.read().await;
    let pvinv  = *state.pvinv_power_w.read().await;
    let tw     = *state.solar_total_w.read().await;
    let housew = *state.house_power_w.read().await;
    (StatusCode::OK, Json(json!({
        "ok": true,
        "total_yield_kwh": kwh,
        "dc_pv_power_w":   dc_pv,
        "pvinv_power_w":   pvinv,
        "mppt_power_w":    dc_pv,
        "solar_total_w":   tw,
        "house_power_w":   housew,
    })))
}

/// GET /api/v1/discover
///
/// Lance une découverte live sur le bus RS485.
/// ⚠️ Bloquant pendant la durée du scan (peut prendre plusieurs secondes).
pub async fn discover(State(_state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": "Découverte non encore implémentée (Phase 2)",
        })),
    )
}

// =============================================================================
// Endpoints Venus OS — Données D-Bus via MQTT
// =============================================================================

/// GET /api/v1/venus/mppt
///
/// Retourne tous les MPPT SolarCharger actuels (depuis D-Bus Venus OS).
pub async fn get_venus_mppt(State(state): State<AppState>) -> impl IntoResponse {
    let mppts = state.venus_mppts_all().await;
    (StatusCode::OK, Json(json!({
        "count": mppts.len(),
        "mppts": mppts,
        "total_power_w": state.venus_mppt_total_power().await,
        "total_dc_current_a": state.venus_mppt_total_dc_current().await,
    })))
}

/// GET /api/v1/venus/smartshunt
///
/// Retourne le SmartShunt actuel (depuis D-Bus Venus OS).
pub async fn get_venus_smartshunt(State(state): State<AppState>) -> impl IntoResponse {
    match state.venus_smartshunt_get().await {
        Some(shunt) => (
            StatusCode::OK,
            Json(json!({
                "connected": true,
                "shunt": shunt,
            })),
        ),
        None => (
            StatusCode::OK,
            Json(json!({
                "connected": false,
                "shunt": Value::Null,
                "message": "SmartShunt non disponible ou non configuré",
            })),
        ),
    }
}

/// GET /api/v1/venus/inverter
///
/// Retourne les données de l'onduleur Victron (MultiPlus, cgwacs, etc.).
pub async fn get_venus_inverter(State(state): State<AppState>) -> impl IntoResponse {
    match state.venus_inverter_get().await {
        Some(inv) => (
            StatusCode::OK,
            Json(json!({
                "connected": true,
                "inverter": inv,
            })),
        ),
        None => (
            StatusCode::OK,
            Json(json!({
                "connected": false,
                "inverter": Value::Null,
                "message": "Onduleur Victron non disponible",
            })),
        ),
    }
}

/// GET /api/v1/venus/temperatures
///
/// Retourne tous les capteurs de température actuels (depuis D-Bus Venus OS).
pub async fn get_venus_temperatures(State(state): State<AppState>) -> impl IntoResponse {
    let temps = state.venus_temperatures_all().await;
    (StatusCode::OK, Json(json!({
        "count": temps.len(),
        "temperatures": temps,
    })))
}

/// GET /api/v1/venus/heatpumps
///
/// Retourne toutes les pompes à chaleur / chauffe-eau actuels.
pub async fn get_venus_heatpumps(State(state): State<AppState>) -> impl IntoResponse {
    let hps = state.venus_heatpumps_all().await;
    (StatusCode::OK, Json(json!({
        "count": hps.len(),
        "heatpumps": hps,
    })))
}

/// GET /api/v1/monitor/status
///
/// Retourne le dernier snapshot de monitoring système Pi5.
pub async fn get_monitor_status(State(state): State<AppState>) -> impl IntoResponse {
    match state.monitor_latest().await {
        Some(snap) => (StatusCode::OK, Json(json!({ "available": true, "monitor": snap }))),
        None       => (StatusCode::OK, Json(json!({ "available": false, "monitor": null }))),
    }
}

/// GET /api/v1/monitor/rs485-health
///
/// Retourne les compteurs de santé par appareil RS485 (BMS, ET112, ATS, PRALRAN).
/// Contient : polls réussis, timeouts, erreurs CRC, autres erreurs, horodatages
/// du dernier succès et de la dernière erreur.
pub async fn get_rs485_health(State(state): State<AppState>) -> impl IntoResponse {
    let stats = state.rs485_stats_all().await;
    let total_success: u64 = stats.iter().map(|s| s.successful_polls).sum();
    let total_timeouts: u64 = stats.iter().map(|s| s.timeout_count).sum();
    let total_crc: u64 = stats.iter().map(|s| s.crc_error_count).sum();
    let total_other: u64 = stats.iter().map(|s| s.other_error_count).sum();
    (
        StatusCode::OK,
        Json(json!({
            "count": stats.len(),
            "devices": stats,
            "totals": {
                "successful_polls":  total_success,
                "timeout_count":     total_timeouts,
                "crc_error_count":   total_crc,
                "other_error_count": total_other,
            },
        })),
    )
}

/// GET /api/v1/system/totals
///
/// Retourne les totaux agrégés du système :
/// - Puissance production (MPPT + PV Inverter)
/// - Puissance consommation (maison)
/// - Charge/décharge batteries
/// - SOC moyen batteries
#[derive(Serialize)]
pub struct SystemTotals {
    pub production_w: f32,           // MPPT + ET112 pvinverter
    pub consumption_w: f32,          // Maison
    pub batteries_power_w: f32,      // Charge (+) / Décharge (-)
    pub avg_soc_percent: f32,        // SOC moyen des batteries
    pub avg_voltage_v: f32,          // Tension moyenne batteries
    pub total_current_a: f32,        // Courant total batteries
    pub smartshunt_soc_percent: Option<f32>, // SmartShunt SOC si disponible
}

pub async fn get_system_totals(State(state): State<AppState>) -> impl IntoResponse {
    // Production : source de vérité = solar_total_w (dc_pv_power_w + pvinv_power_w)
    let production_w = *state.solar_total_w.read().await;

    // Consommation : maison
    let consumption_w = *state.house_power_w.read().await;

    // Batteries : SOC moyen et tension/courant total
    let bms_snapshots = state.latest_snapshots().await;
    let avg_soc_percent = if !bms_snapshots.is_empty() {
        bms_snapshots
            .iter()
            .map(|s| s.soc)
            .sum::<f32>() / bms_snapshots.len() as f32
    } else {
        0.0
    };

    let avg_voltage_v = if !bms_snapshots.is_empty() {
        bms_snapshots
            .iter()
            .map(|s| s.dc.voltage)
            .sum::<f32>() / bms_snapshots.len() as f32
    } else {
        0.0
    };

    let total_current_a: f32 = bms_snapshots
        .iter()
        .map(|s| s.dc.current)
        .sum();

    let batteries_power_w: f32 = bms_snapshots
        .iter()
        .map(|s| s.dc.power)
        .sum();

    let smartshunt_soc = state
        .venus_smartshunt_get()
        .await
        .and_then(|s| s.soc_percent);

    (
        StatusCode::OK,
        Json(SystemTotals {
            production_w,
            consumption_w,
            batteries_power_w,
            avg_soc_percent,
            avg_voltage_v,
            total_current_a,
            smartshunt_soc_percent: smartshunt_soc,
        }),
    )
}


// =============================================================================
// Log archive endpoints
// =============================================================================

#[derive(Deserialize)]
pub struct LogFileQuery {
    /// Nom du fichier (ex: "daly-bms.log.2026-04-30")
    pub name: Option<String>,
    /// Nombre de lignes à retourner depuis la fin.
    pub lines: Option<usize>,
}

/// GET /api/v1/monitor/logs — liste les fichiers de log disponibles.
pub async fn get_monitor_logs(State(state): State<AppState>) -> Json<Value> {
    let log_dir = &state.config.logging.log_dir;
    if log_dir.is_empty() {
        return Json(json!({ "ok": false, "reason": "file_logging_disabled", "files": [] }));
    }
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return Json(json!({ "ok": false, "reason": "directory_not_found", "files": [] }));
    };
    let mut files: Vec<serde_json::Value> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.contains("daly-bms") { return None; }
            let meta = e.metadata().ok()?;
            let size_kb = meta.len() / 1024;
            let modified = meta.modified().ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            Some(json!({ "name": name, "size_kb": size_kb, "modified": modified }))
        })
        .collect();
    // Sort by name descending (most recent first)
    files.sort_by(|a, b| {
        b["name"].as_str().unwrap_or("").cmp(a["name"].as_str().unwrap_or(""))
    });
    Json(json!({ "ok": true, "dir": log_dir, "files": files }))
}

/// GET /api/v1/monitor/logs/content?name=...&lines=500
pub async fn get_monitor_log_content(
    State(state): State<AppState>,
    Query(q): Query<LogFileQuery>,
) -> impl IntoResponse {
    let log_dir = &state.config.logging.log_dir;
    let Some(filename) = q.name.as_deref() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "reason": "missing name" }))).into_response();
    };
    // Sanitize: no path traversal
    if filename.contains('/') || filename.contains("..") {
        return (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "reason": "invalid filename" }))).into_response();
    }
    let path = std::path::Path::new(log_dir).join(filename);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return (StatusCode::NOT_FOUND, Json(json!({ "ok": false, "reason": "file not found" }))).into_response();
    };
    let limit = q.lines.unwrap_or(500).min(5000);
    let lines: Vec<&str> = content.lines().rev().take(limit).collect::<Vec<_>>();
    let lines: Vec<&str> = lines.into_iter().rev().collect();
    Json(json!({ "ok": true, "name": filename, "lines": lines })).into_response()
}
