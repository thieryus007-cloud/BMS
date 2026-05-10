//! Boucle de polling asynchrone avec reconnexion automatique et backoff exponentiel.
//!
//! La fonction principale [`poll_loop`] interroge cycliquement tous les BMS
//! configurés et appelle le callback `on_snapshot` pour chaque snapshot produit.

use crate::bus::{BmsConfig, DalyBusManager, DalyPort};
use crate::commands;
use crate::error::DalyError;
use crate::types::{
    BmsSnapshot, DcData, HistoryData, InfoData, IoData, SystemData,
};
use chrono::Utc;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

/// Configuration de la boucle de polling.
#[derive(Debug, Clone)]
pub struct PollConfig {
    /// Intervalle entre deux cycles de polling complets (ms).
    pub interval_ms: u64,
    /// Nombre de tentatives par commande avant de marquer un BMS en erreur.
    pub retries: u8,
    /// Délai initial pour le backoff exponentiel en cas d'erreur (ms).
    pub backoff_initial_ms: u64,
    /// Délai maximum de backoff (ms).
    pub backoff_max_ms: u64,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            interval_ms:       1000,
            retries:           3,
            backoff_initial_ms: 2000,
            backoff_max_ms:    30_000,
        }
    }
}

/// Catégorie d'erreur RS485 exposée à l'appelant via le callback `on_error`.
#[derive(Clone, Copy, Debug)]
pub enum PollErrorKind {
    /// Timeout : aucune réponse du BMS dans le délai imparti.
    Timeout,
    /// CRC / trame invalide (checksum, start flag, adresse inattendue…).
    Crc,
    /// Autre erreur non-fatale (ReadOnly, VerifyFailed, etc).
    Other,
    /// Erreur port série (backoff déclenché). L'appelant peut tracer une
    /// dégradation globale du bus.
    Serial,
}

/// Exécute la boucle de polling infinie pour tous les BMS du manager.
///
/// Pour chaque BMS, toutes les commandes de lecture sont émises séquentiellement.
/// Le snapshot résultant est passé au callback `on_snapshot`.
///
/// Le callback `on_error` reçoit `(adresse_bms, catégorie, message)` pour chaque
/// erreur non-fatale et permet de tenir des compteurs de santé RS485.
///
/// En cas d'erreur série (port perdu), la boucle attend `backoff` ms et retente.
pub async fn poll_loop<F, E>(
    manager: Arc<DalyBusManager>,
    config: PollConfig,
    on_snapshot: F,
    on_error: E,
) where
    F: Fn(BmsSnapshot) + Send + Sync + 'static,
    E: Fn(u8, PollErrorKind, String) + Send + Sync + 'static,
{
    let on_snapshot = Arc::new(on_snapshot);
    let on_error = Arc::new(on_error);
    let mut backoff_ms = config.backoff_initial_ms;
    // Cache des versions firmware (lues une seule fois par BMS)
    let mut fw_cache: HashMap<u8, (String, String)> = HashMap::new();

    loop {
        let cycle_start = std::time::Instant::now();

        // Lire les versions firmware pour les BMS non encore mis en cache
        for device in &manager.devices {
            if !fw_cache.contains_key(&device.address) {
                let sw = commands::get_firmware_sw(&manager.port, device.address)
                    .await.unwrap_or_default();
                let hw = commands::get_firmware_hw(&manager.port, device.address)
                    .await.unwrap_or_default();
                if !sw.is_empty() || !hw.is_empty() {
                    info!(
                        bms = format!("{:#04x}", device.address),
                        fw_sw = %sw, fw_hw = %hw,
                        "Firmware version lu"
                    );
                }
                fw_cache.insert(device.address, (sw, hw));
            }
        }

        for device in &manager.devices {
            let (fw_sw, fw_hw) = fw_cache
                .get(&device.address)
                .cloned()
                .unwrap_or_default();
            match poll_device(&manager.port, device, &config, fw_sw, fw_hw).await {
                Ok(snapshot) => {
                    backoff_ms = config.backoff_initial_ms; // reset backoff
                    on_snapshot(snapshot);
                }
                Err(DalyError::Timeout { .. }) => {
                    warn!(
                        bms = format!("{:#04x}", device.address),
                        "Timeout — BMS peut-être hors ligne"
                    );
                    on_error(device.address, PollErrorKind::Timeout, "timeout".to_string());
                }
                Err(DalyError::Serial(e)) => {
                    let msg = e.to_string();
                    error!("Erreur port série : {} — backoff {}ms", e, backoff_ms);
                    on_error(device.address, PollErrorKind::Serial, msg);
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = (backoff_ms * 2).min(config.backoff_max_ms);
                    break; // sortir de la boucle devices et réessayer le cycle
                }
                Err(e) => {
                    let msg = format!("{:?}", e);
                    warn!(
                        bms = format!("{:#04x}", device.address),
                        "Erreur : {}", msg
                    );
                    let kind = match &e {
                        DalyError::Checksum { .. }
                        | DalyError::InvalidFrame { .. }
                        | DalyError::InvalidStartFlag(_)
                        | DalyError::UnexpectedAddress { .. }
                        | DalyError::UnexpectedDataId { .. } => PollErrorKind::Crc,
                        _ => PollErrorKind::Other,
                    };
                    on_error(device.address, kind, msg);
                }
            }
        }

        // Attendre le reste de l'intervalle configuré
        let elapsed = cycle_start.elapsed();
        let interval = Duration::from_millis(config.interval_ms);
        if elapsed < interval {
            tokio::time::sleep(interval - elapsed).await;
        }
    }
}

/// Poll complet d'un seul BMS : toutes les commandes de lecture.
///
/// Retourne un [`BmsSnapshot`] agrégé ou une [`DalyError`].
async fn poll_device(
    port: &Arc<DalyPort>,
    device: &BmsConfig,
    config: &PollConfig,
    firmware_sw: String,
    firmware_hw: String,
) -> crate::error::Result<BmsSnapshot> {
    let addr = device.address;

    // ── 0x90 : Pack status (tension, courant, SOC) ────────────────────────────
    let soc_data = retry(config.retries, || {
        commands::get_pack_status(port, addr)
    }).await?;

    // ── 0x91 : Min/max tensions cellules ──────────────────────────────────────
    let (min_cell_v, min_cell_id, max_cell_v, max_cell_id) = retry(config.retries, || {
        commands::get_cell_voltage_minmax(port, addr)
    }).await?;

    // ── 0x92 : Min/max températures ───────────────────────────────────────────
    let (min_temp, min_temp_id, max_temp, max_temp_id) = retry(config.retries, || {
        commands::get_temperature_minmax(port, addr)
    }).await?;

    // ── 0x93 : État MOS, cycles, capacité ─────────────────────────────────────
    let mos = retry(config.retries, || {
        commands::get_mos_status(port, addr)
    }).await?;

    // ── 0x94 : Status info 1 ──────────────────────────────────────────────────
    let status = retry(config.retries, || {
        commands::get_status_info(port, addr)
    }).await?;

    // ── 0x95 : Tensions individuelles — cell_count issu du 0x94 ──────────────
    let cell_count = if status.cell_count > 0 { status.cell_count } else { device.cell_count };
    let cell_voltages = retry(config.retries, || {
        commands::get_cell_voltages(port, addr, cell_count)
    }).await.unwrap_or_default();

    // ── 0x96 : Températures individuelles — sensor_count issu du 0x94 ────────
    let sensor_count = if status.temp_sensor_count > 0 { status.temp_sensor_count } else { device.temp_sensor_count };
    let _temperatures = retry(config.retries, || {
        commands::get_temperatures(port, addr, sensor_count)
    }).await.unwrap_or_default();

    // ── 0x97 : Flags d'équilibrage ────────────────────────────────────────────
    let balance_flags = retry(config.retries, || {
        commands::get_balance_flags(port, addr, device.cell_count)
    }).await.unwrap_or_default();

    // ── 0x98 : Alarmes ────────────────────────────────────────────────────────
    let (_charge_en, _discharge_en, alarm_bytes) = retry(config.retries, || {
        commands::get_alarm_flags(port, addr)
    }).await.unwrap_or((true, true, [0u8; 7]));

    let alarms = commands::parse_alarm_flags(&alarm_bytes);

    // ── Assemblage du snapshot ────────────────────────────────────────────────
    let dc = DcData {
        voltage:     soc_data.voltage,
        current:     soc_data.current,
        power:       soc_data.voltage * soc_data.current,
        temperature: max_temp,
    };

    let capacity_ah = device.installed_capacity_ah * soc_data.soc / 100.0;
    let consumed_ah = device.installed_capacity_ah - capacity_ah;

    let system = SystemData {
        min_voltage_cell_id: format!("C{}", min_cell_id),
        min_cell_voltage:    min_cell_v,
        max_voltage_cell_id: format!("C{}", max_cell_id),
        max_cell_voltage:    max_cell_v,
        min_temperature_cell_id: format!("C{}", min_temp_id),
        min_cell_temperature:    min_temp,
        max_temperature_cell_id: format!("C{}", max_temp_id),
        max_cell_temperature:    max_temp,
        mos_temperature:     max_temp, // MOS temp from external sensor if available
        nr_of_modules_online: 1,
        nr_of_modules_offline: 0,
        nr_of_cells_per_battery: device.cell_count,
        nr_of_modules_blocking_charge:    u8::from(!mos.charge_mos),
        nr_of_modules_blocking_discharge: u8::from(!mos.discharge_mos),
    };

    let io = IoData {
        allow_to_charge:    u8::from(mos.charge_mos),
        allow_to_discharge: u8::from(mos.discharge_mos),
        allow_to_balance:   1,
        allow_to_heat:      0,
        external_relay:     status.charger_status,
    };

    let history = HistoryData {
        charge_cycles:   status.cycle_count as u32, // 0x94 bytes[5-6] : u16, correct
        minimum_voltage: 0.0, // non disponible en temps réel
        maximum_voltage: 0.0,
        total_ah_drawn:  0.0,
    };

    // TimeToSoc simplifié : interpolation linéaire
    let time_to_soc = compute_time_to_soc(soc_data.soc, soc_data.current, device.installed_capacity_ah);

    let snapshot = BmsSnapshot {
        address:            addr,
        name:               device.name.clone(),
        timestamp:          Utc::now(),
        dc,
        installed_capacity: device.installed_capacity_ah,
        consumed_amphours:  consumed_ah,
        capacity:                 capacity_ah,
        bms_reported_capacity_ah: mos.residual_capacity_mah as f32 / 1000.0,
        soc:                soc_data.soc,
        soh:                100.0, // non disponible directement
        time_to_go:         compute_time_to_go(capacity_ah, soc_data.current),
        balancing:          balance_flags.flags.iter().any(|&f| f) as u8,
        system_switch:      u8::from(mos.charge_mos || mos.discharge_mos),
        alarms,
        info:               InfoData {
            max_charge_current:    device.max_charge_current_a,
            max_discharge_current: device.max_discharge_current_a,
            ..InfoData::default()
        },
        history,
        system,
        voltages:           cell_voltages.to_named_map(),
        balances:           balance_flags.to_named_map(),
        io,
        heating:            0,
        time_to_soc,
        firmware_sw,
        firmware_hw,
    };

    info!(
        bms = format!("{:#04x}", addr),
        soc = format!("{:.1}%", snapshot.soc),
        voltage = format!("{:.2}V", snapshot.dc.voltage),
        current = format!("{:.1}A", snapshot.dc.current),
        "Snapshot OK"
    );

    Ok(snapshot)
}

// =============================================================================
// Utilitaires
// =============================================================================

/// Calcule le temps estimé jusqu'à la décharge complète (secondes).
fn compute_time_to_go(capacity_ah: f32, current_a: f32) -> u32 {
    if current_a >= 0.0 || capacity_ah <= 0.0 {
        return 0;
    }
    let hours = capacity_ah / (-current_a);
    (hours * 3600.0) as u32
}

/// Calcule la map TimeToSoC : SOC% → secondes pour atteindre ce palier.
fn compute_time_to_soc(
    current_soc: f32,
    current_a: f32,
    installed_ah: f32,
) -> BTreeMap<u8, u32> {
    let mut map = BTreeMap::new();
    for soc_target in (0..=100u8).step_by(5) {
        let delta_soc = soc_target as f32 - current_soc;
        let delta_ah  = installed_ah * delta_soc / 100.0;
        let seconds = if current_a.abs() < 0.1 {
            0u32
        } else {
            let hours = (delta_ah / current_a.abs()).abs();
            (hours * 3600.0) as u32
        };
        map.insert(soc_target, seconds);
    }
    map
}

/// Exécute `f` jusqu'à `retries` fois en cas d'erreur non-fatale.
async fn retry<F, Fut, T>(retries: u8, mut f: F) -> crate::error::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = crate::error::Result<T>>,
{
    let mut last_err = DalyError::Other(anyhow::anyhow!("Aucune tentative effectuée"));
    for attempt in 0..=retries {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e @ DalyError::Serial(_)) | Err(e @ DalyError::Io(_)) => {
                // Erreur fatale (port série) — ne pas réessayer
                return Err(e);
            }
            Err(e) => {
                if attempt < retries {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                last_err = e;
            }
        }
    }
    Err(last_err)
}
