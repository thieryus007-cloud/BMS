//! API REST pour l'ATS CHINT NXZB/NXZBN.
//!
//! ## Endpoints
//!
//! | Méthode | URL                          | Description                           |
//! |---------|------------------------------|--------------------------------------|
//! | GET     | /api/v1/ats/status           | Snapshot complet ATS (tous champs)    |
//! | POST    | /api/v1/ats/remote_on        | Activer télécommande                  |
//! | POST    | /api/v1/ats/remote_off       | Désactiver télécommande               |
//! | POST    | /api/v1/ats/force_source1    | Forcer Onduleur (source 1)            |
//! | POST    | /api/v1/ats/force_source2    | Forcer Réseau (source 2)              |
//! | POST    | /api/v1/ats/force_double     | Forcer double déclenché               |
//! | POST    | /api/v1/ats/send_raw         | Envoyer trame Modbus brute (hex)      |
//! | GET     | /api/v1/ats/debug_on         | Activer debug (no-op, loggé)          |
//! | GET     | /api/v1/ats/debug_off        | Désactiver debug (no-op, loggé)       |

use crate::ats::{execute_ats_command, AtsCommand};
use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use tracing::info;

// =============================================================================
// Lecture — snapshot complet
// =============================================================================

/// GET /api/v1/ats/status
///
/// Retourne le dernier snapshot ATS au format JSON complet.
/// Tous les champs de `AtsSnapshot` sont présents, plus un objet `values`
/// avec les représentations texte directement utilisables par le JS du dashboard.
pub async fn get_ats_status(State(state): State<AppState>) -> impl IntoResponse {
    match state.ats_latest().await {
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "error": "Aucune donnée ATS disponible — ATS non configuré ou en attente de données"
            })),
        )
            .into_response(),

        Some(snap) => {
            // Valeurs pré-formatées (texte) pour le dashboard JS
            let values = json!({
                // Tensions source 1 (V, arrondi entier)
                "v1a": format!("{:.0}", snap.v1a),
                "v1b": format!("{:.0}", snap.v1b),
                "v1c": format!("{:.0}", snap.v1c),
                // Tensions source 2
                "v2a": format!("{:.0}", snap.v2a),
                "v2b": format!("{:.0}", snap.v2b),
                "v2c": format!("{:.0}", snap.v2c),
                // Tensions max enregistrées
                "max1": snap.max1_v.to_string(),
                "max2": snap.max2_v.to_string(),
                // Statut phases (label FR)
                "s1a": snap.s1a.label(),
                "s1b": snap.s1b.label(),
                "s1c": snap.s1c.label(),
                "s2a": snap.s2a.label(),
                "s2b": snap.s2b.label(),
                "s2c": snap.s2c.label(),
                // Commutation
                "sw1": if snap.sw1_closed { "Fermé" } else { "Ouvert" },
                "sw2": if snap.sw2_closed { "Fermé" } else { "Ouvert" },
                "middleOFF": if snap.middle_off { "Activé" } else { "Désactivé" },
                "swMode": if snap.sw_mode { "Auto" } else { "Manuel" },
                "swFault": snap.fault.label(),
                "swRemote": if snap.remote { "📡 Activé" } else { "🔒 Désactivé" },
                // Source active (label FR)
                "active_source": snap.active_source.label(),
                // Compteurs & runtime
                "cnt1": snap.cnt1.to_string(),
                "cnt2": snap.cnt2.to_string(),
                "runtime": format!("{} h", snap.runtime_h),
                // Version SW
                "swVer": format!("{:.2}", snap.sw_version),
                // Mode opératoire (MN)
                "operation_mode": snap.operation_mode.map(|m| m.label()).unwrap_or_else(|| "—".to_string()),
                // Seuils (MN)
                "uv1": snap.uv1.map(|v| format!("{} V", v)).unwrap_or_else(|| "—".to_string()),
                "uv2": snap.uv2.map(|v| format!("{} V", v)).unwrap_or_else(|| "—".to_string()),
                "ov1": snap.ov1.map(|v| format!("{} V", v)).unwrap_or_else(|| "—".to_string()),
                "ov2": snap.ov2.map(|v| format!("{} V", v)).unwrap_or_else(|| "—".to_string()),
                // Délais (MN)
                "t1": snap.t1_s.map(|v| format!("{} s", v)).unwrap_or_else(|| "—".to_string()),
                "t2": snap.t2_s.map(|v| format!("{} s", v)).unwrap_or_else(|| "—".to_string()),
                // Fréquences (MN)
                "freq1": snap.freq1_hz.map(|v| format!("{} Hz", v)).unwrap_or_else(|| "—".to_string()),
                "freq2": snap.freq2_hz.map(|v| format!("{} Hz", v)).unwrap_or_else(|| "—".to_string()),
                // Config Modbus
                "modbus_addr": snap.modbus_addr.map(|v| v.to_string()).unwrap_or_else(|| "—".to_string()),
                "modbus_baud": snap.modbus_baud_label().to_string(),
                "modbus_parity": "None (8N1)",
            });

            Json(json!({
                "success": true,
                "address":  snap.address,
                "name":     snap.name,
                "model":    snap.model,
                "timestamp": snap.timestamp,
                // Snapshot complet (tous champs bruts)
                "data": snap,
                // Champs pré-formatés pour le dashboard JS
                "values": values,
            }))
            .into_response()
        }
    }
}

// =============================================================================
// Commandes d'écriture Modbus FC=06
// =============================================================================

/// Exécute une commande ATS via le bus RS485 unifié.
async fn run_command(state: &AppState, cfg_addr: u8, cmd: AtsCommand) -> impl IntoResponse {
    let bus_guard = state.ats_bus.read().await;
    let bus = match bus_guard.as_ref() {
        Some(b) => b.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "success": false,
                    "error": "Bus ATS non disponible — ATS non configuré ou port série non ouvert"
                })),
            )
                .into_response();
        }
    };
    drop(bus_guard);

    match execute_ats_command(&bus, cfg_addr, cmd).await {
        Ok(()) => Json(json!({
            "success": true,
            "message": cmd.label(),
            "command": format!("{:?}", cmd),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "error": e.to_string(),
                "command": format!("{:?}", cmd),
            })),
        )
            .into_response(),
    }
}

/// POST /api/v1/ats/remote_on
pub async fn ats_remote_on(State(state): State<AppState>) -> impl IntoResponse {
    let addr = ats_addr(&state);
    run_command(&state, addr, AtsCommand::RemoteOn).await
}

/// POST /api/v1/ats/remote_off
pub async fn ats_remote_off(State(state): State<AppState>) -> impl IntoResponse {
    let addr = ats_addr(&state);
    run_command(&state, addr, AtsCommand::RemoteOff).await
}

/// POST /api/v1/ats/force_source1
pub async fn ats_force_source1(State(state): State<AppState>) -> impl IntoResponse {
    let addr = ats_addr(&state);
    run_command(&state, addr, AtsCommand::ForceSource1).await
}

/// POST /api/v1/ats/force_source2
pub async fn ats_force_source2(State(state): State<AppState>) -> impl IntoResponse {
    let addr = ats_addr(&state);
    run_command(&state, addr, AtsCommand::ForceSource2).await
}

/// POST /api/v1/ats/force_double
pub async fn ats_force_double(State(state): State<AppState>) -> impl IntoResponse {
    let addr = ats_addr(&state);
    run_command(&state, addr, AtsCommand::ForceDouble).await
}

// =============================================================================
// Console Modbus brute — send_raw
// =============================================================================

#[derive(Deserialize)]
pub struct SendRawBody {
    /// Trame Modbus en hexadécimal, ex: "060300060007E4C9"
    pub frame_hex: String,
    /// Longueur de réponse attendue (optionnel — déduite du FC si absent)
    pub resp_len:  Option<usize>,
}

/// POST /api/v1/ats/send_raw
///
/// Envoie une trame Modbus RTU brute au bus et retourne la réponse hexadécimale.
/// Longueur de réponse déduite du code fonction (FC) si `resp_len` est absent :
/// - FC=01/02 : variable (calculée depuis le count dans la trame)
/// - FC=03/04 : 5 + count*2  (count = octets 4-5 de la trame)
/// - FC=06    : 8 octets
/// - Autres   : 8 octets par défaut
pub async fn ats_send_raw(
    State(state): State<AppState>,
    Json(body): Json<SendRawBody>,
) -> impl IntoResponse {
    // Décoder la trame hex
    let hex = body.frame_hex.replace([' ', ':'], "");
    let frame: Vec<u8> = match (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| ()))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "error": format!("Trame hex invalide : '{}'", body.frame_hex)
                })),
            )
                .into_response();
        }
    };

    if frame.len() < 4 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "error": "Trame trop courte (minimum 4 octets)"
            })),
        )
            .into_response();
    }

    // Déduire la longueur de réponse depuis le FC
    let fc = frame[1];
    let resp_len = body.resp_len.unwrap_or_else(|| match fc {
        0x03 | 0x04 => {
            // FC=03/04 : count = frame[4..5] (nombre de registres) → octets = count*2
            let count = if frame.len() >= 6 {
                (frame[4] as usize) * 256 + frame[5] as usize
            } else { 1 };
            5 + count * 2
        }
        0x06 => 8, // FC=06 : écho de la requête
        0x01 | 0x02 => {
            let count = if frame.len() >= 6 {
                (frame[4] as usize) * 256 + frame[5] as usize
            } else { 8 };
            5 + (count + 7) / 8
        }
        _ => 8,
    });

    // Accéder au bus
    let bus_guard = state.ats_bus.read().await;
    let bus = match bus_guard.as_ref() {
        Some(b) => b.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "success": false,
                    "error": "Bus ATS non disponible"
                })),
            )
                .into_response();
        }
    };
    drop(bus_guard);

    info!(
        frame = %body.frame_hex,
        fc    = format!("0x{:02X}", fc),
        resp_len,
        "ATS send_raw"
    );

    match bus.transact(&frame, resp_len).await {
        Ok(resp) => {
            let resp_hex = resp.iter().map(|b| format!("{:02X}", b)).collect::<String>();
            Json(json!({
                "success":   true,
                "tx_hex":    body.frame_hex,
                "rx_hex":    resp_hex,
                "rx_len":    resp.len(),
                "fc":        format!("0x{:02X}", fc),
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "error":   e.to_string(),
                "tx_hex":  body.frame_hex,
            })),
        )
            .into_response(),
    }
}

// =============================================================================
// Debug — no-op endpoints (loggés)
// =============================================================================

/// GET /api/v1/ats/debug_on
pub async fn ats_debug_on() -> impl IntoResponse {
    info!("ATS debug ON (no-op)");
    Json(json!({ "success": true, "message": "Debug activé (logs niveau DEBUG)" }))
}

/// GET /api/v1/ats/debug_off
pub async fn ats_debug_off() -> impl IntoResponse {
    info!("ATS debug OFF (no-op)");
    Json(json!({ "success": true, "message": "Debug désactivé" }))
}

// =============================================================================
// Helper
// =============================================================================

fn ats_addr(state: &AppState) -> u8 {
    state
        .config
        .ats
        .as_ref()
        .map(|c| c.address)
        .unwrap_or(6)
}
