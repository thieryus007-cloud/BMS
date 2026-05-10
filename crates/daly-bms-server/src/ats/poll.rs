//! Boucle de polling Modbus RTU pour l'ATS CHINT NXZB/NXZBN.
//!
//! ## Protocole validé sur matériel réel (Qwen reference)
//!
//! L'ATS CHINT ne supporte PAS les lectures multi-registres.
//! Chaque registre est lu individuellement (FC=03, count=1).
//! Délai de 90 ms entre chaque lecture (requis par le matériel).
//!
//! ## Registres (FC=03, count=1)
//!
//! | Adresse | Contenu                          |
//! |---------|----------------------------------|
//! | 0x0006  | Tension source 1 phase A (V)     |
//! | 0x0007  | Tension source 1 phase B (V)     |
//! | 0x0008  | Tension source 1 phase C (V)     |
//! | 0x0009  | Tension source 2 phase A (V)     |
//! | 0x000A  | Tension source 2 phase B (V)     |
//! | 0x000B  | Tension source 2 phase C (V)     |
//! | 0x000C  | Version SW (÷100)                |
//! | 0x000D  | Fréquences (MN: hi=f1, lo=f2)    |
//! | 0x000E  | Parité Modbus                    |
//! | 0x000F  | Tension max enregistrée source 1 |
//! | 0x0012  | Tension max enregistrée source 2 |
//! | 0x0015  | Compteur commutations S1→S2      |
//! | 0x0016  | Compteur commutations S2→S1      |
//! | 0x0017  | Durée de fonctionnement (h)      |
//! | 0x004F  | Statut tensions phases (bits)    |
//! | 0x0050  | Statut commutation (bits)        |
//! | 0x0100  | Adresse Modbus configurée        |
//! | 0x0101  | Baud rate configuré              |
//! | 0x2065  | UV seuil source 1 (MN)           |
//! | 0x2066  | UV seuil source 2 (MN)           |
//! | 0x2067  | OV seuil source 1 (MN)           |
//! | 0x2068  | OV seuil source 2 (MN)           |
//! | 0x2069  | T1 délai transfert (MN)          |
//! | 0x206A  | T2 délai retour (MN)             |
//! | 0x206D  | Mode opératoire (MN)             |
//! | 0x2700  | Registre forçage position (FC=06)|
//! | 0x2800  | Registre télécommande (FC=06)    |

use super::types::{
    ActiveSource, AtsCommand, AtsSnapshot, FaultCode, OperationMode, PhaseStatus,
};
use crate::config::AtsConfig;
use chrono::Local;
use rs485_bus::{modbus_rtu, SharedBus};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Délai entre chaque lecture de registre individuel (ms).
/// Validé sur matériel CHINT NXZB — requis pour éviter les timeouts.
const INTER_REG_DELAY_MS: u64 = 90;

/// Longueur de réponse FC=06 (écho requête).
const FC06_RESPONSE_LEN: usize = 8;

// =============================================================================
// Lecture d'un registre unique (FC=03, count=1)
// =============================================================================

/// Lit un seul registre Modbus et retourne sa valeur, ou None en cas d'erreur.
async fn read_reg(bus: &SharedBus, addr: u8, reg: u16) -> Option<u16> {
    let req = modbus_rtu::build_fc03(addr, reg, 1);
    let resp_len = modbus_rtu::response_len(1);

    tokio::time::sleep(Duration::from_millis(INTER_REG_DELAY_MS)).await;

    match bus.transact(&req, resp_len).await {
        Ok(resp) => match modbus_rtu::parse_read_response(addr, 0x03, &resp) {
            Ok(regs) if !regs.is_empty() => Some(regs[0]),
            _ => None,
        },
        Err(_) => None,
    }
}

// =============================================================================
// Validation réponse FC=06
// =============================================================================

fn validate_fc06_response(addr: u8, response: &[u8]) -> anyhow::Result<()> {
    if response.len() < FC06_RESPONSE_LEN {
        anyhow::bail!(
            "Réponse FC06 trop courte: {} octets (attendu {})",
            response.len(),
            FC06_RESPONSE_LEN
        );
    }
    if response[0] != addr {
        anyhow::bail!(
            "FC06 adresse inattendue: attendu {:#04x}, reçu {:#04x}",
            addr, response[0]
        );
    }
    if response[1] == 0x86 {
        let exc = response.get(2).copied().unwrap_or(0);
        anyhow::bail!("FC06 exception Modbus: code {:#04x}", exc);
    }
    if response[1] != 0x06 {
        anyhow::bail!("FC06 function code inattendu: {:#04x}", response[1]);
    }
    let crc_calc = modbus_rtu::crc16(&response[..6]);
    let crc_recv = (response[6] as u16) | ((response[7] as u16) << 8);
    if crc_recv != crc_calc {
        anyhow::bail!(
            "FC06 CRC invalide: reçu {:#06x}, calculé {:#06x}",
            crc_recv, crc_calc
        );
    }
    Ok(())
}

// =============================================================================
// Polling principal
// =============================================================================

pub async fn run_ats_poll_loop<F, E>(
    bus: Arc<SharedBus>,
    cfg: AtsConfig,
    mut on_snapshot: F,
    mut on_result: E,
)
where
    F: FnMut(AtsSnapshot) + Send + 'static,
    E: FnMut(u8, &str, Result<(), String>) + Send + 'static,
{
    let addr = cfg.address;
    let poll_interval = Duration::from_millis(cfg.poll_interval_ms);

    info!(
        addr = format!("{:#04x}", addr),
        name = %cfg.name,
        "ATS CHINT polling démarré (lecture registre par registre)"
    );

    // Détection du modèle au démarrage
    let model = detect_model(&bus, addr).await;
    info!(addr = format!("{:#04x}", addr), model = %model, "Modèle ATS détecté");

    let mut consecutive_errors: u32 = 0;

    loop {
        match poll_ats(&bus, addr, &cfg.name, &model).await {
            Ok(snap) => {
                debug!(
                    addr   = format!("{:#04x}", addr),
                    source = %snap.active_source.label(),
                    v1a    = snap.v1a,
                    v2a    = snap.v2a,
                    "ATS snapshot OK"
                );
                consecutive_errors = 0;
                on_result(addr, &cfg.name, Ok(()));
                on_snapshot(snap);
            }
            Err(e) => {
                consecutive_errors += 1;
                let msg = format!("{:#}", e);
                if consecutive_errors == 1 || consecutive_errors % 10 == 0 {
                    warn!(
                        addr   = format!("{:#04x}", addr),
                        errors = consecutive_errors,
                        "ATS erreur lecture : {}", msg
                    );
                }
                on_result(addr, &cfg.name, Err(msg));
            }
        }

        tokio::time::sleep(poll_interval).await;
    }
}

// =============================================================================
// Détection du modèle
// =============================================================================

async fn detect_model(bus: &SharedBus, addr: u8) -> String {
    tokio::time::sleep(Duration::from_millis(INTER_REG_DELAY_MS)).await;
    match read_reg(bus, addr, 0x2065).await {
        Some(_) => "MN".to_string(),
        None    => "BN".to_string(),
    }
}

// =============================================================================
// Lecture d'un snapshot complet (un registre à la fois)
// =============================================================================

async fn poll_ats(
    bus:   &SharedBus,
    addr:  u8,
    name:  &str,
    model: &str,
) -> anyhow::Result<AtsSnapshot> {

    // ── Tensions source 1 ────────────────────────────────────────────────────
    let v1a = read_reg(bus, addr, 0x0006).await.ok_or_else(|| anyhow::anyhow!("Timeout 0x0006 (v1a)"))? as f32;
    let v1b = read_reg(bus, addr, 0x0007).await.unwrap_or(0) as f32;
    let v1c = read_reg(bus, addr, 0x0008).await.unwrap_or(0) as f32;

    // ── Tensions source 2 ────────────────────────────────────────────────────
    let v2a = read_reg(bus, addr, 0x0009).await.unwrap_or(0) as f32;
    let v2b = read_reg(bus, addr, 0x000A).await.unwrap_or(0) as f32;
    let v2c = read_reg(bus, addr, 0x000B).await.unwrap_or(0) as f32;

    // ── Version SW ───────────────────────────────────────────────────────────
    let sw_version = read_reg(bus, addr, 0x000C).await.unwrap_or(0) as f32 / 100.0;

    // ── Fréquences (MN seulement) ─────────────────────────────────────────
    let (freq1_hz, freq2_hz) = if model == "MN" {
        match read_reg(bus, addr, 0x000D).await {
            Some(freq) => (Some(((freq >> 8) & 0xFF) as u8), Some((freq & 0xFF) as u8)),
            None       => (None, None),
        }
    } else {
        (None, None)
    };

    // ── Tensions max enregistrées ─────────────────────────────────────────
    let max1_v = read_reg(bus, addr, 0x000F).await.unwrap_or(0);
    let max2_v = read_reg(bus, addr, 0x0012).await.unwrap_or(0);

    // ── Compteurs & runtime ───────────────────────────────────────────────
    let cnt1      = read_reg(bus, addr, 0x0015).await.unwrap_or(0);
    let cnt2      = read_reg(bus, addr, 0x0016).await.unwrap_or(0);
    let runtime_h = read_reg(bus, addr, 0x0017).await.unwrap_or(0);

    // ── Statut tensions phases (0x004F) ───────────────────────────────────
    let pwr = read_reg(bus, addr, 0x004F).await.unwrap_or(0);
    let s1a = PhaseStatus::from_bits(((pwr >> 8)  & 0x03) as u8);
    let s1b = PhaseStatus::from_bits(((pwr >> 10) & 0x03) as u8);
    let s1c = PhaseStatus::from_bits(((pwr >> 12) & 0x03) as u8);
    let s2a = PhaseStatus::from_bits((pwr          & 0x03) as u8);
    let s2b = PhaseStatus::from_bits(((pwr >> 2)  & 0x03) as u8);
    let s2c = PhaseStatus::from_bits(((pwr >> 4)  & 0x03) as u8);

    // ── Statut commutation (0x0050) ───────────────────────────────────────
    // bit 3 = SW1 (Onduleur) : 1=ouvert, 0=fermé
    // bit 4 = SW2 (Réseau)   : 1=ouvert, 0=fermé
    // bit 1 = Mode           : 1=Auto, 0=Manuel
    // bit 8 = Télécommande   : 1=active, 0=inactive
    // bits 5-7 = Code défaut
    let sw = read_reg(bus, addr, 0x0050).await.unwrap_or(0);
    let sw1_raw    = (sw & 0x0008) != 0;
    let sw2_raw    = (sw & 0x0010) != 0;
    let middle_off = !sw1_raw && !sw2_raw;
    let sw1_closed = !middle_off && !sw1_raw;
    let sw2_closed = !middle_off && !sw2_raw;
    let sw_mode    = (sw & 0x0001) != 0;
    let remote     = (sw & 0x0100) != 0;
    let fault      = FaultCode::from_u8(((sw >> 5) & 0x07) as u8);

    let active_source = if sw1_closed {
        ActiveSource::Source1
    } else if sw2_closed {
        ActiveSource::Source2
    } else {
        ActiveSource::Neutral
    };

    // ── Config Modbus (optionnel) ─────────────────────────────────────────
    let modbus_addr     = read_reg(bus, addr, 0x0100).await;
    let modbus_baud_code = read_reg(bus, addr, 0x0101).await;

    // ── T1 et T2 — tous modèles (Qwen reference : toujours lus, BN et MN) ──
    let t1_s = read_reg(bus, addr, 0x2069).await;
    let t2_s = read_reg(bus, addr, 0x206A).await;

    // ── Registres MN uniquement ───────────────────────────────────────────
    let (operation_mode, uv1, uv2, ov1, ov2, t3_s, t4_s) = if model == "MN" {
        (
            read_reg(bus, addr, 0x206D).await.map(OperationMode::from_u16),
            read_reg(bus, addr, 0x2065).await,
            read_reg(bus, addr, 0x2066).await,
            read_reg(bus, addr, 0x2067).await,
            read_reg(bus, addr, 0x2068).await,
            read_reg(bus, addr, 0x206B).await,
            read_reg(bus, addr, 0x206C).await,
        )
    } else {
        (None, None, None, None, None, None, None)
    };

    Ok(AtsSnapshot {
        address: addr,
        name: name.to_string(),
        model: model.to_string(),
        timestamp: Local::now(),
        v1a, v1b, v1c,
        v2a, v2b, v2c,
        max1_v, max2_v,
        s1a, s1b, s1c,
        s2a, s2b, s2c,
        sw_mode,
        sw1_closed,
        sw2_closed,
        middle_off,
        remote,
        fault,
        active_source,
        cnt1, cnt2, runtime_h,
        sw_version,
        freq1_hz, freq2_hz,
        operation_mode,
        uv1, uv2, ov1, ov2,
        t1_s, t2_s, t3_s, t4_s,
        modbus_addr,
        modbus_baud_code,
    })
}

// =============================================================================
// Exécution d'une commande FC=06
// =============================================================================

pub async fn execute_ats_command(
    bus:  &SharedBus,
    addr: u8,
    cmd:  AtsCommand,
) -> anyhow::Result<()> {
    let (reg, value) = cmd.register_value();
    let frame = modbus_rtu::build_fc06(addr, reg, value);

    tokio::time::sleep(Duration::from_millis(INTER_REG_DELAY_MS)).await;

    let response = bus
        .transact(&frame, FC06_RESPONSE_LEN)
        .await
        .map_err(|e| anyhow::anyhow!("ATS FC06 TX erreur: {}", e))?;

    validate_fc06_response(addr, &response)?;

    info!(
        addr  = format!("{:#04x}", addr),
        cmd   = cmd.label(),
        reg   = format!("{:#06x}", reg),
        value = format!("{:#06x}", value),
        "ATS commande exécutée"
    );

    Ok(())
}
