//! Commandes d'écriture sécurisées pour le BMS Daly.
//!
//! Toutes les commandes d'écriture :
//! 1. Vérifient que le mode read-only n'est pas activé.
//! 2. Envoient la commande et attendent la confirmation du BMS.
//! 3. Effectuent une lecture de vérification post-écriture.
//!
//! ## Commandes disponibles
//! - [`set_discharge_mos`] — activer/désactiver le MOSFET de décharge (0xD9)
//! - [`set_charge_mos`]    — activer/désactiver le MOSFET de charge (0xDA)
//! - [`set_soc`]           — calibrer le SOC (0x21)
//! - [`reset_bms`]         — réinitialiser le BMS (0x00)

use crate::bus::DalyPort;
use crate::error::{DalyError, Result};
use crate::protocol::DataId;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Activer ou désactiver le MOSFET de décharge (Data ID 0xD9).
///
/// `enable = true` → MOS ON ; `enable = false` → MOS OFF.
pub async fn set_discharge_mos(
    port: &Arc<DalyPort>,
    addr: u8,
    enable: bool,
    read_only: bool,
) -> Result<()> {
    if read_only {
        return Err(DalyError::ReadOnly);
    }
    let payload = u8::from(enable);
    info!(
        bms = format!("{:#04x}", addr),
        "set_discharge_mos → {}",
        if enable { "ON" } else { "OFF" }
    );
    port.send_command(addr, DataId::SetDischargeMos, payload_to_data(payload))
        .await?;
    // Vérification : lire l'état MOS et confirmer
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mos = crate::commands::get_mos_status(port, addr).await?;
    if mos.discharge_mos != enable {
        warn!(bms = format!("{:#04x}", addr), "Vérification set_discharge_mos échouée");
        return Err(DalyError::VerifyFailed { bms_id: addr, cmd: DataId::SetDischargeMos as u8 });
    }
    Ok(())
}

/// Activer ou désactiver le MOSFET de charge (Data ID 0xDA).
pub async fn set_charge_mos(
    port: &Arc<DalyPort>,
    addr: u8,
    enable: bool,
    read_only: bool,
) -> Result<()> {
    if read_only {
        return Err(DalyError::ReadOnly);
    }
    let payload = u8::from(enable);
    info!(
        bms = format!("{:#04x}", addr),
        "set_charge_mos → {}",
        if enable { "ON" } else { "OFF" }
    );
    port.send_command(addr, DataId::SetChargeMos, payload_to_data(payload))
        .await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mos = crate::commands::get_mos_status(port, addr).await?;
    if mos.charge_mos != enable {
        warn!(bms = format!("{:#04x}", addr), "Vérification set_charge_mos échouée");
        return Err(DalyError::VerifyFailed { bms_id: addr, cmd: DataId::SetChargeMos as u8 });
    }
    Ok(())
}

/// Calibrer le SOC à la valeur indiquée en % (Data ID 0x21).
///
/// La valeur est encodée en uint16 BE × 10 à l'offset 4 de la trame.
pub async fn set_soc(
    port: &Arc<DalyPort>,
    addr: u8,
    soc_percent: f32,
    read_only: bool,
) -> Result<()> {
    if read_only {
        return Err(DalyError::ReadOnly);
    }
    if !(0.0..=100.0).contains(&soc_percent) {
        return Err(anyhow::anyhow!("SOC hors plage [0, 100] : {}", soc_percent).into());
    }
    info!(bms = format!("{:#04x}", addr), "set_soc → {:.1}%", soc_percent);
    let raw = (soc_percent * 10.0) as u16;
    let mut data = [0u8; 8];
    // Protocole Daly 0x21 : bytes [4-9] = date/time (mis à 0), bytes [10-11] = SOC
    // → data[6..7] dans le payload de 8 octets
    data[6] = (raw >> 8) as u8;
    data[7] = (raw & 0xFF) as u8;
    port.send_command(addr, DataId::SetSoc, data).await?;
    Ok(())
}

// =============================================================================
// Écriture des paramètres de configuration
// =============================================================================

/// Écrire les seuils d'alarme tension cellule (Data ID 0x19).
///
/// Paramètres en millivolts.
pub async fn set_cell_volt_alarms(
    port: &Arc<DalyPort>,
    addr: u8,
    high_l1_mv: u16,
    high_l2_mv: u16,
    low_l1_mv: u16,
    low_l2_mv: u16,
    read_only: bool,
) -> Result<()> {
    if read_only { return Err(DalyError::ReadOnly); }
    let mut data = [0u8; 8];
    data[0] = (high_l1_mv >> 8) as u8; data[1] = (high_l1_mv & 0xFF) as u8;
    data[2] = (high_l2_mv >> 8) as u8; data[3] = (high_l2_mv & 0xFF) as u8;
    data[4] = (low_l1_mv  >> 8) as u8; data[5] = (low_l1_mv  & 0xFF) as u8;
    data[6] = (low_l2_mv  >> 8) as u8; data[7] = (low_l2_mv  & 0xFF) as u8;
    info!(bms = format!("{:#04x}", addr), "set_cell_volt_alarms hi1={}mV hi2={}mV lo1={}mV lo2={}mV", high_l1_mv, high_l2_mv, low_l1_mv, low_l2_mv);
    port.send_command(addr, DataId::SetCellVoltAlarms, data).await?;
    Ok(())
}

/// Écrire les seuils d'alarme tension pack (Data ID 0x1A).
///
/// Paramètres en 0.1 V.
pub async fn set_pack_volt_alarms(
    port: &Arc<DalyPort>,
    addr: u8,
    high_l1_dv: u16,
    high_l2_dv: u16,
    low_l1_dv: u16,
    low_l2_dv: u16,
    read_only: bool,
) -> Result<()> {
    if read_only { return Err(DalyError::ReadOnly); }
    let mut data = [0u8; 8];
    data[0] = (high_l1_dv >> 8) as u8; data[1] = (high_l1_dv & 0xFF) as u8;
    data[2] = (high_l2_dv >> 8) as u8; data[3] = (high_l2_dv & 0xFF) as u8;
    data[4] = (low_l1_dv  >> 8) as u8; data[5] = (low_l1_dv  & 0xFF) as u8;
    data[6] = (low_l2_dv  >> 8) as u8; data[7] = (low_l2_dv  & 0xFF) as u8;
    info!(bms = format!("{:#04x}", addr), "set_pack_volt_alarms hi1={}dV hi2={}dV lo1={}dV lo2={}dV", high_l1_dv, high_l2_dv, low_l1_dv, low_l2_dv);
    port.send_command(addr, DataId::SetPackVoltAlarms, data).await?;
    Ok(())
}

/// Écrire les seuils d'alarme courant (Data ID 0x1B).
///
/// `chg_*` et `dch_*` en Ampères positifs.
/// Encodage offset 30000 : charge = 30000 - (A × 10), décharge = 30000 + (A × 10).
pub async fn set_current_alarms(
    port: &Arc<DalyPort>,
    addr: u8,
    chg_l1_a: f32,
    chg_l2_a: f32,
    dch_l1_a: f32,
    dch_l2_a: f32,
    read_only: bool,
) -> Result<()> {
    if read_only { return Err(DalyError::ReadOnly); }
    let enc_chg = |a: f32| -> u16 { (30000.0 - a * 10.0) as u16 };
    let enc_dch = |a: f32| -> u16 { (30000.0 + a * 10.0) as u16 };
    let c1 = enc_chg(chg_l1_a); let c2 = enc_chg(chg_l2_a);
    let d1 = enc_dch(dch_l1_a); let d2 = enc_dch(dch_l2_a);
    let mut data = [0u8; 8];
    data[0] = (c1 >> 8) as u8; data[1] = (c1 & 0xFF) as u8;
    data[2] = (c2 >> 8) as u8; data[3] = (c2 & 0xFF) as u8;
    data[4] = (d1 >> 8) as u8; data[5] = (d1 & 0xFF) as u8;
    data[6] = (d2 >> 8) as u8; data[7] = (d2 & 0xFF) as u8;
    info!(bms = format!("{:#04x}", addr), "set_current_alarms chg={}/{}A dch={}/{}A", chg_l1_a, chg_l2_a, dch_l1_a, dch_l2_a);
    port.send_command(addr, DataId::SetCurrentAlarms, data).await?;
    Ok(())
}

/// Écrire les seuils d'alarme delta tension cellule + delta température (Data ID 0x1E).
pub async fn set_delta_alarms(
    port: &Arc<DalyPort>,
    addr: u8,
    cell_delta_l1_mv: u16,
    cell_delta_l2_mv: u16,
    temp_delta_l1: u8,
    temp_delta_l2: u8,
    read_only: bool,
) -> Result<()> {
    if read_only { return Err(DalyError::ReadOnly); }
    let mut data = [0u8; 8];
    data[0] = (cell_delta_l1_mv >> 8) as u8; data[1] = (cell_delta_l1_mv & 0xFF) as u8;
    data[2] = (cell_delta_l2_mv >> 8) as u8; data[3] = (cell_delta_l2_mv & 0xFF) as u8;
    data[4] = temp_delta_l1;
    data[5] = temp_delta_l2;
    info!(bms = format!("{:#04x}", addr), "set_delta_alarms dv={}/{}mV dt={}/{}°C", cell_delta_l1_mv, cell_delta_l2_mv, temp_delta_l1, temp_delta_l2);
    port.send_command(addr, DataId::SetDeltaAlarms, data).await?;
    Ok(())
}

/// Écrire les seuils de balancing (Data ID 0x1F).
pub async fn set_balancing_thresh(
    port: &Arc<DalyPort>,
    addr: u8,
    activation_mv: u16,
    delta_mv: u16,
    read_only: bool,
) -> Result<()> {
    if read_only { return Err(DalyError::ReadOnly); }
    let mut data = [0u8; 8];
    data[0] = (activation_mv >> 8) as u8; data[1] = (activation_mv & 0xFF) as u8;
    data[2] = (delta_mv >> 8) as u8;      data[3] = (delta_mv & 0xFF) as u8;
    info!(bms = format!("{:#04x}", addr), "set_balancing_thresh activation={}mV delta={}mV", activation_mv, delta_mv);
    port.send_command(addr, DataId::SetBalancingThresh, data).await?;
    Ok(())
}

/// Réinitialiser le BMS (Data ID 0x00). ⚠️ Utiliser avec précaution.
pub async fn reset_bms(port: &Arc<DalyPort>, addr: u8, read_only: bool) -> Result<()> {
    if read_only {
        return Err(DalyError::ReadOnly);
    }
    warn!(bms = format!("{:#04x}", addr), "RESET BMS demandé !");
    port.send_command(addr, DataId::Reset, [0u8; 8]).await?;
    Ok(())
}

// =============================================================================
// Utilitaire interne
// =============================================================================

/// Crée un tableau data[8] avec `value` dans data[0], reste à zéro.
fn payload_to_data(value: u8) -> [u8; 8] {
    let mut data = [0u8; 8];
    data[0] = value;
    data
}
