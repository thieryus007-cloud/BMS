//! Commandes de lecture implémentées pour le protocole Daly BMS.
//!
//! Chaque fonction correspond à un Data ID du protocole et retourne
//! une structure typée prête à être assemblée dans un [`BmsSnapshot`].

use crate::bus::DalyPort;
use crate::error::Result;
use crate::protocol::{
    DataId, decode_cell_voltage, decode_current, decode_soc, decode_temperature,
    decode_voltage, read_u16_be,
};
use crate::types::{
    BalanceFlags, BmsSettings, CellTemperatures, CellVoltages, MosStatus, SocData, StatusInfo,
};
use std::sync::Arc;
use tracing::trace;

/// Lit le statut pack : tension totale, courant, SOC (Data ID 0x90).
///
/// Layout du champ Data (8 octets, protocole Daly UART V1.21) :
/// - D0-D1 : Tension totale pack      (uint16 BE, 0.1 V)
/// - D2-D3 : Tension acquisition      (uint16 BE, 0.1 V) — ignoré ici
/// - D4-D5 : Courant                  (uint16 BE, offset 30000, 0.1 A)
/// - D6-D7 : SOC                      (uint16 BE, 0.1 %)
pub async fn get_pack_status(port: &Arc<DalyPort>, addr: u8) -> Result<SocData> {
    let frame = port.send_command(addr, DataId::PackStatus, [0u8; 8]).await?;
    let d = frame.data();
    Ok(SocData {
        voltage: decode_voltage(d, 0),
        current: decode_current(d, 4),
        soc:     decode_soc(d, 6),
    })
}

/// Lit les tensions min/max des cellules avec les numéros de cellule (0x91).
///
/// Retourne (min_voltage, min_cell_id, max_voltage, max_cell_id).
pub async fn get_cell_voltage_minmax(
    port: &Arc<DalyPort>,
    addr: u8,
) -> Result<(f32, u8, f32, u8)> {
    let frame = port
        .send_command(addr, DataId::CellVoltageMinMax, [0u8; 8])
        .await?;
    let d = frame.data();
    let max_v    = decode_cell_voltage(d, 0);
    let max_cell = d[2];
    let min_v    = decode_cell_voltage(d, 3);
    let min_cell = d[5];
    Ok((min_v, min_cell, max_v, max_cell))
}

/// Lit les températures min/max avec les numéros de capteur (0x92).
///
/// Retourne (min_temp, min_sensor, max_temp, max_sensor).
pub async fn get_temperature_minmax(
    port: &Arc<DalyPort>,
    addr: u8,
) -> Result<(f32, u8, f32, u8)> {
    let frame = port
        .send_command(addr, DataId::TemperatureMinMax, [0u8; 8])
        .await?;
    let d = frame.data();
    let max_t      = decode_temperature(d[0]);
    let max_sensor = d[1];
    let min_t      = decode_temperature(d[2]);
    let min_sensor = d[3];
    Ok((min_t, min_sensor, max_t, max_sensor))
}

/// Lit l'état des MOSFET, les cycles et la capacité résiduelle (0x93).
///
/// Layout selon Daly UART V1.21 :
/// - D0 : State (0=repos, 1=charge, 2=décharge)
/// - D1 : Charge MOS state  (0=off, 1=on)
/// - D2 : Discharge MOS status (0=off, 1=on)
/// - D3 : BMS life (0–255 cycles)
/// - D4-D7 : Remain capacity (mAh, uint32 BE)
pub async fn get_mos_status(port: &Arc<DalyPort>, addr: u8) -> Result<MosStatus> {
    let frame = port.send_command(addr, DataId::MosStatus, [0u8; 8]).await?;
    let d = frame.data();
    Ok(MosStatus {
        charge_mos:            d[1] != 0,
        discharge_mos:         d[2] != 0,
        bms_life:              d[3],
        residual_capacity_mah: u32::from_be_bytes([d[4], d[5], d[6], d[7]]),
        charge_cycles:         d[3] as u32,
    })
}

/// Lit les informations de statut 1 : nombre de cellules, capteurs, états (0x94).
pub async fn get_status_info(port: &Arc<DalyPort>, addr: u8) -> Result<StatusInfo> {
    let frame = port.send_command(addr, DataId::StatusInfo1, [0u8; 8]).await?;
    let d = frame.data();
    Ok(StatusInfo {
        cell_count:       d[0],
        temp_sensor_count: d[1],
        charger_status:   d[2],
        load_status:      d[3],
        dio_states:       d[4],
        cycle_count:      read_u16_be(d, 5),
    })
}

/// Lit les tensions individuelles de toutes les cellules (0x95, multi-trames).
///
/// Le BMS répond avec toutes les trames d'un coup après une seule requête.
/// Chaque trame contient 3 tensions (uint16 BE, millivolts) :
///   data[0]   = numéro de trame (1-based)
///   data[1-2] = cellule N
///   data[3-4] = cellule N+1
///   data[5-6] = cellule N+2
pub async fn get_cell_voltages(
    port: &Arc<DalyPort>,
    addr: u8,
    cell_count: u8,
) -> Result<CellVoltages> {
    let frame_count = (cell_count as usize + 2) / 3;
    let frames = port.send_command_multi(addr, DataId::CellVoltages1, frame_count).await?;

    let mut voltages = Vec::with_capacity(cell_count as usize);
    for (i, frame) in frames.iter().enumerate() {
        let d = frame.data();
        // 3 cellules par trame, aux offsets 1-2, 3-4, 5-6 (offset 0 = frame index)
        for j in 0..3 {
            let cell_idx = i * 3 + j;
            if cell_idx >= cell_count as usize {
                break;
            }
            voltages.push(decode_cell_voltage(d, 1 + j * 2));
        }
        trace!(addr = format!("{:#04x}", addr), frame = i + 1, "tensions cellules lues");
    }

    Ok(CellVoltages { voltages })
}

/// Lit les températures individuelles de tous les capteurs (0x96, multi-trames).
///
/// Le BMS répond avec toutes les trames d'un coup après une seule requête.
/// Chaque trame contient 7 températures (encodage = valeur + 40) :
///   data[0]   = numéro de trame (1-based)
///   data[1-7] = températures capteurs
pub async fn get_temperatures(
    port: &Arc<DalyPort>,
    addr: u8,
    sensor_count: u8,
) -> Result<CellTemperatures> {
    let frame_count = (sensor_count as usize + 6) / 7;
    let frames = port.send_command_multi(addr, DataId::Temperatures, frame_count).await?;

    let mut temperatures = Vec::with_capacity(sensor_count as usize);
    for (i, frame) in frames.iter().enumerate() {
        let d = frame.data();
        for j in 0..7 {
            let sensor_idx = i * 7 + j;
            if sensor_idx >= sensor_count as usize {
                break;
            }
            temperatures.push(decode_temperature(d[j + 1]));
        }
    }

    Ok(CellTemperatures { temperatures })
}

/// Lit les flags d'équilibrage cellule par cellule (0x97).
///
/// 48 cellules max, encodées en bits little-endian sur 6 octets.
pub async fn get_balance_flags(
    port: &Arc<DalyPort>,
    addr: u8,
    cell_count: u8,
) -> Result<BalanceFlags> {
    let frame = port
        .send_command(addr, DataId::BalanceStatus, [0u8; 8])
        .await?;
    let d = frame.data();

    let mut flags = Vec::with_capacity(cell_count as usize);
    for i in 0..(cell_count as usize) {
        let byte_idx = i / 8;
        let bit_idx  = i % 8;
        if byte_idx < 6 {
            flags.push((d[byte_idx] >> bit_idx) & 1 != 0);
        } else {
            flags.push(false);
        }
    }

    Ok(BalanceFlags { flags })
}

/// Lit les drapeaux d'alarme/protection (0x98).
///
/// Retourne (charge_mos_en, discharge_mos_en, alarm_flags_7_bytes).
pub async fn get_alarm_flags(
    port: &Arc<DalyPort>,
    addr: u8,
) -> Result<(bool, bool, [u8; 7])> {
    let frame = port
        .send_command(addr, DataId::AlarmFlags, [0u8; 8])
        .await?;
    let d = frame.data();
    let charge_en    = d[0] & 0x02 != 0;
    let discharge_en = d[0] & 0x01 != 0;
    let mut alarm_bytes = [0u8; 7];
    alarm_bytes.copy_from_slice(&d[1..8]);
    Ok((charge_en, discharge_en, alarm_bytes))
}

// =============================================================================
// Commandes de lecture — paramètres/configuration
// =============================================================================

/// Lit la capacité nominale et la tension nominale de cellule (0x50).
///
/// Layout data :
/// - D0-D3 : Capacité nominale (mAh, uint32 BE)
/// - D4-D5 : Réservé
/// - D6-D7 : Tension nominale cellule (mV, uint16 BE)
pub async fn get_rated_capacity(port: &Arc<DalyPort>, addr: u8) -> Result<(u32, u16)> {
    let frame = port.send_command(addr, DataId::RatedCapacity, [0u8; 8]).await?;
    let d = frame.data();
    let capacity_mah = u32::from_be_bytes([d[0], d[1], d[2], d[3]]);
    let nominal_mv   = read_u16_be(d, 6);
    Ok((capacity_mah, nominal_mv))
}

/// Lit les seuils d'alarme tension cellule L1/L2 (0x59).
///
/// Retourne (high_l1_mv, high_l2_mv, low_l1_mv, low_l2_mv).
pub async fn get_cell_volt_alarms(port: &Arc<DalyPort>, addr: u8) -> Result<(u16, u16, u16, u16)> {
    let frame = port.send_command(addr, DataId::CellVoltAlarms, [0u8; 8]).await?;
    let d = frame.data();
    Ok((read_u16_be(d, 0), read_u16_be(d, 2), read_u16_be(d, 4), read_u16_be(d, 6)))
}

/// Lit les seuils d'alarme tension pack L1/L2 (0x5A).
///
/// Retourne (high_l1_dv, high_l2_dv, low_l1_dv, low_l2_dv) en 0.1 V.
pub async fn get_pack_volt_alarms(port: &Arc<DalyPort>, addr: u8) -> Result<(u16, u16, u16, u16)> {
    let frame = port.send_command(addr, DataId::PackVoltAlarms, [0u8; 8]).await?;
    let d = frame.data();
    Ok((read_u16_be(d, 0), read_u16_be(d, 2), read_u16_be(d, 4), read_u16_be(d, 6)))
}

/// Lit les seuils d'alarme courant charge/décharge L1/L2 (0x5B).
///
/// Encodage offset 30000 (même que courant 0x90).
/// Charge : raw < 30000  → A = (30000 - raw) / 10
/// Décharge : raw > 30000 → A = (raw - 30000) / 10
pub async fn get_current_alarms(port: &Arc<DalyPort>, addr: u8) -> Result<(f32, f32, f32, f32)> {
    let frame = port.send_command(addr, DataId::CurrentAlarms, [0u8; 8]).await?;
    let d = frame.data();
    let chg_l1 = (30000i32 - read_u16_be(d, 0) as i32).unsigned_abs() as f32 / 10.0;
    let chg_l2 = (30000i32 - read_u16_be(d, 2) as i32).unsigned_abs() as f32 / 10.0;
    let dch_l1 = (read_u16_be(d, 4) as i32 - 30000i32).unsigned_abs() as f32 / 10.0;
    let dch_l2 = (read_u16_be(d, 6) as i32 - 30000i32).unsigned_abs() as f32 / 10.0;
    Ok((chg_l1, chg_l2, dch_l1, dch_l2))
}

/// Lit les seuils d'alarme delta tension + delta température L1/L2 (0x5E).
///
/// Retourne (cell_delta_mv_l1, cell_delta_mv_l2, temp_delta_l1, temp_delta_l2).
pub async fn get_delta_alarms(port: &Arc<DalyPort>, addr: u8) -> Result<(u16, u16, u8, u8)> {
    let frame = port.send_command(addr, DataId::DeltaAlarms, [0u8; 8]).await?;
    let d = frame.data();
    Ok((read_u16_be(d, 0), read_u16_be(d, 2), d[4], d[5]))
}

/// Lit les seuils de balancing (0x5F).
///
/// Retourne (activation_mv, delta_mv).
pub async fn get_balancing_thresh(port: &Arc<DalyPort>, addr: u8) -> Result<(u16, u16)> {
    let frame = port.send_command(addr, DataId::BalancingThresh, [0u8; 8]).await?;
    let d = frame.data();
    Ok((read_u16_be(d, 0), read_u16_be(d, 2)))
}

/// Lit la version logicielle firmware (0x62, multi-trames, 7 chars/trame).
///
/// Exemple : "20210222-1.01T"
pub async fn get_firmware_sw(port: &Arc<DalyPort>, addr: u8) -> Result<String> {
    let frames = port.send_command_multi(addr, DataId::FirmwareSW, 2).await?;
    let mut s = String::with_capacity(14);
    for frame in &frames {
        let d = frame.data();
        // d[0] = numéro de trame, d[1..8] = 7 chars
        for &b in &d[1..8] {
            if b != 0x00 && b != 0x20 {
                s.push(b as char);
            }
        }
    }
    Ok(s.trim().to_string())
}

/// Lit la version matérielle (0x63, multi-trames, 7 chars/trame).
///
/// Exemple : "DL-BMS-R32-01E"
pub async fn get_firmware_hw(port: &Arc<DalyPort>, addr: u8) -> Result<String> {
    let frames = port.send_command_multi(addr, DataId::FirmwareHW, 2).await?;
    let mut s = String::with_capacity(14);
    for frame in &frames {
        let d = frame.data();
        for &b in &d[1..8] {
            if b != 0x00 && b != 0x20 {
                s.push(b as char);
            }
        }
    }
    Ok(s.trim().to_string())
}

/// Lit tous les paramètres de configuration en une seule opération (0x50, 0x5F, 0x59, 0x5A, 0x5B, 0x5E).
pub async fn get_bms_settings(port: &Arc<DalyPort>, addr: u8) -> Result<BmsSettings> {
    let (rated_mah, nominal_mv)            = get_rated_capacity(port, addr).await?;
    let (bal_act_mv, bal_delta_mv)         = get_balancing_thresh(port, addr).await?;
    let (cv_hi1, cv_hi2, cv_lo1, cv_lo2)  = get_cell_volt_alarms(port, addr).await?;
    let (pv_hi1, pv_hi2, pv_lo1, pv_lo2)  = get_pack_volt_alarms(port, addr).await?;
    let (ci_c1, ci_c2, ci_d1, ci_d2)      = get_current_alarms(port, addr).await?;
    let (dv_l1, dv_l2, dt_l1, dt_l2)      = get_delta_alarms(port, addr).await?;

    Ok(BmsSettings {
        rated_capacity_mah:    rated_mah,
        nominal_cell_mv:       nominal_mv,
        balancing_activation_mv: bal_act_mv,
        balancing_delta_mv:    bal_delta_mv,
        cell_high_v_l1_mv:     cv_hi1,
        cell_high_v_l2_mv:     cv_hi2,
        cell_low_v_l1_mv:      cv_lo1,
        cell_low_v_l2_mv:      cv_lo2,
        pack_high_v_l1_dv:     pv_hi1,
        pack_high_v_l2_dv:     pv_hi2,
        pack_low_v_l1_dv:      pv_lo1,
        pack_low_v_l2_dv:      pv_lo2,
        chg_high_a_l1:         ci_c1,
        chg_high_a_l2:         ci_c2,
        dch_high_a_l1:         ci_d1,
        dch_high_a_l2:         ci_d2,
        cell_delta_v_l1_mv:    dv_l1,
        cell_delta_v_l2_mv:    dv_l2,
        temp_delta_l1:         dt_l1,
        temp_delta_l2:         dt_l2,
    })
}

// =============================================================================
// Parsing des alarmes
// =============================================================================

use crate::types::Alarms;

/// Convertit les 7 octets bruts d'alarme (0x98) en structure [`Alarms`].
///
/// Mapping basé sur la documentation Daly UART V1.21, page 6.
pub fn parse_alarm_flags(bytes: &[u8; 7]) -> Alarms {
    // Mapping basé sur documentation Daly UART V1.21, page 6.
    // Byte 0 : [bit0]=cell_OVP, [bit1]=cell_UVP, [bit2]=pack_OVP, [bit3]=pack_UVP
    // Byte 1 : [bit0]=charge_OTP, [bit1]=charge_UTP, [bit2]=disch_OTP, [bit3]=disch_UTP
    // Byte 2 : [bit0]=charge_OCP, [bit1]=disch_OCP
    // Byte 3 : [bit0]=cell_imbalance
    // Byte 5 : [bit5]=fuse_blown
    Alarms {
        high_voltage:             ((bytes[0] >> 0) | (bytes[0] >> 2)) & 1,
        low_voltage:              ((bytes[0] >> 1) | (bytes[0] >> 3)) & 1,
        low_cell_voltage:         (bytes[0] >> 1) & 1,
        high_charge_temperature:  (bytes[1] >> 0) & 1,
        low_charge_temperature:   (bytes[1] >> 1) & 1,
        high_temperature:         (bytes[1] >> 2) & 1,
        low_temperature:          (bytes[1] >> 3) & 1,
        high_charge_current:      (bytes[2] >> 0) & 1,
        high_discharge_current:   (bytes[2] >> 1) & 1,
        high_current:             ((bytes[2] >> 0) | (bytes[2] >> 1)) & 1,
        cell_imbalance:           (bytes[3] >> 0) & 1,
        fuse_blown:               (bytes[5] >> 5) & 1,
        low_soc:                  0, // calculé par l'AlertEngine logiciel
    }
}
