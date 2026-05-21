//! Module redb writer — pousse les samples des snapshots vers metrics-store::Writer.
//!
//! Ce module remplace l'écriture VictoriaMetrics retirée en Phase 5. Il est
//! appelé depuis chaque `state.on_*_snapshot()` pour pousser les valeurs
//! mesurées dans la TSDB redb.
//!
//! ## Rate limiting
//!
//! Chaque métrique est rate-limitée à 1 écriture toutes les 5 secondes (par
//! couple metric+labels) pour éviter de saturer le writer batché. Sur ~50
//! métriques actives × 1 écriture / 5 s = 10 writes/sec = 36 000 writes/h.
//! Le writer batche par 500 → ~72 commits redb/h. Empreinte I/O négligeable.
//!
//! ## Politique non-bloquante
//!
//! On utilise `try_write()` (non-bloquant). Si le mpsc channel du writer est
//! plein (10k entrées par défaut), le sample est silencieusement drop. Cela
//! évite de bloquer les chemins critiques (poll RS485, callback MQTT) en
//! cas de back-pressure sur l'écriture redb.

use crate::ats::AtsSnapshot;
use crate::et112::Et112Snapshot;
use crate::irradiance::IrradianceSnapshot;
use crate::shelly::ShellyEmSnapshot;
use crate::state::{VenusHeatpump, VenusInverter, VenusMppt, VenusSmartShunt, VenusTemperature};
use crate::tasmota::TasmotaSnapshot;
use daly_bms_core::types::BmsSnapshot;
use metrics_store::{Sample, Writer};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Intervalle minimum entre 2 écritures d'une même série (metric+labels).
/// Garde le rate global gérable et aligne avec ce que faisait l'ancien VmClient.
const MIN_WRITE_INTERVAL: Duration = Duration::from_secs(5);

/// Intervalle min pour les compteurs d'énergie (varient lentement).
const ENERGY_WRITE_INTERVAL: Duration = Duration::from_secs(30);

/// Intervalle min pour température (varient très lentement).
const TEMP_WRITE_INTERVAL: Duration = Duration::from_secs(60);

/// Rate limiter clonable basé sur Arc<Mutex<HashMap>>.
///
/// La clé est `(metric, labels_fingerprint)` — un même metric avec des labels
/// différents est traité indépendamment (ex: bms_voltage{bms_id="0x01"} vs
/// bms_voltage{bms_id="0x02"}).
#[derive(Default, Clone)]
pub struct RateLimiter {
    last_writes: std::sync::Arc<Mutex<HashMap<String, Instant>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Retourne `true` si l'écriture est autorisée (et marque le timestamp).
    pub fn allow(&self, key: &str, min_interval: Duration) -> bool {
        let now = Instant::now();
        // unwrap_or_else pour rester safe si un panic a empoisonné le mutex.
        let mut map = self.last_writes.lock().unwrap_or_else(|p| p.into_inner());
        match map.get(key) {
            Some(&last) if now.duration_since(last) < min_interval => false,
            _ => {
                map.insert(key.to_string(), now);
                true
            }
        }
    }
}

/// Helper interne — push si rate-limit OK.
fn push(writer: &Writer, rl: &RateLimiter, interval: Duration, sample: Sample, key: &str) {
    if rl.allow(key, interval) {
        let _ = writer.try_write(sample);
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// =============================================================================
// BMS
// =============================================================================

pub fn write_bms(writer: &Writer, rl: &RateLimiter, snap: &BmsSnapshot) {
    let ts = now_ms();
    let bms_id = format!("0x{:02x}", snap.address);

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("bms_voltage", ts, snap.dc.voltage as f64)
             .with_label("bms_id", bms_id.clone()),
         &format!("bms_voltage:{}", bms_id));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("bms_current", ts, snap.dc.current as f64)
             .with_label("bms_id", bms_id.clone()),
         &format!("bms_current:{}", bms_id));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("bms_soc", ts, snap.soc as f64)
             .with_label("bms_id", bms_id.clone()),
         &format!("bms_soc:{}", bms_id));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("bms_power", ts, snap.dc.power as f64)
             .with_label("bms_id", bms_id.clone()),
         &format!("bms_power:{}", bms_id));

    push(writer, rl, TEMP_WRITE_INTERVAL,
         Sample::new("bms_temp_max", ts, snap.system.max_cell_temperature as f64)
             .with_label("bms_id", bms_id.clone()),
         &format!("bms_temp_max:{}", bms_id));

    push(writer, rl, TEMP_WRITE_INTERVAL,
         Sample::new("bms_temp_min", ts, snap.system.min_cell_temperature as f64)
             .with_label("bms_id", bms_id.clone()),
         &format!("bms_temp_min:{}", bms_id));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("bms_cell_delta_mv", ts, snap.system.cell_delta_mv() as f64)
             .with_label("bms_id", bms_id.clone()),
         &format!("bms_cell_delta_mv:{}", bms_id));

    // Capacité nominale installée (config statique, change uniquement à la conf).
    push(writer, rl, ENERGY_WRITE_INTERVAL,
         Sample::new("bms_capacity_ah", ts, snap.installed_capacity as f64)
             .with_label("bms_id", bms_id.clone()),
         &format!("bms_capacity_ah:{}", bms_id));

    // État des MOSFETs (proxy via IO permissions — l'API 0x93 n'est pas dans le snapshot).
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("bms_charge_mos", ts, snap.io.allow_to_charge as f64)
             .with_label("bms_id", bms_id.clone()),
         &format!("bms_charge_mos:{}", bms_id));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("bms_discharge_mos", ts, snap.io.allow_to_discharge as f64)
             .with_label("bms_id", bms_id.clone()),
         &format!("bms_discharge_mos:{}", bms_id));

    // Tension par cellule. La clé est "Cell1", "Cell2", … — on extrait l'index numérique.
    for (key, &voltage) in snap.voltages.iter() {
        let cell_idx = key.trim_start_matches("Cell");
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("bms_cell_voltage", ts, voltage as f64)
                 .with_label("bms_id", bms_id.clone())
                 .with_label("cell", cell_idx.to_string()),
             &format!("bms_cell_voltage:{}:{}", bms_id, cell_idx));
    }
}

// =============================================================================
// ET112
// =============================================================================

pub fn write_et112(writer: &Writer, rl: &RateLimiter, snap: &Et112Snapshot) {
    let ts = now_ms();
    let addr = format!("0x{:02x}", snap.address);

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("et112_power_w", ts, snap.power_w as f64)
             .with_label("address", addr.clone()),
         &format!("et112_power_w:{}", addr));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("et112_voltage_v", ts, snap.voltage_v as f64)
             .with_label("address", addr.clone()),
         &format!("et112_voltage_v:{}", addr));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("et112_current_a", ts, snap.current_a as f64)
             .with_label("address", addr.clone()),
         &format!("et112_current_a:{}", addr));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("et112_apparent_power_va", ts, snap.apparent_power_va as f64)
             .with_label("address", addr.clone()),
         &format!("et112_apparent_power_va:{}", addr));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("et112_power_factor", ts, snap.power_factor as f64)
             .with_label("address", addr.clone()),
         &format!("et112_power_factor:{}", addr));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("et112_frequency_hz", ts, snap.frequency_hz as f64)
             .with_label("address", addr.clone()),
         &format!("et112_frequency_hz:{}", addr));

    // Énergie cumulée en Wh (convention VictoriaMetrics historique).
    push(writer, rl, ENERGY_WRITE_INTERVAL,
         Sample::new("et112_energy_import_wh", ts, snap.energy_import_kwh() as f64 * 1000.0)
             .with_label("address", addr.clone()),
         &format!("et112_energy_import_wh:{}", addr));

    push(writer, rl, ENERGY_WRITE_INTERVAL,
         Sample::new("et112_energy_export_wh", ts, snap.energy_export_kwh() as f64 * 1000.0)
             .with_label("address", addr),
         &format!("et112_energy_export_wh:0x{:02x}", snap.address));
}

// =============================================================================
// Irradiance
// =============================================================================

pub fn write_irradiance(writer: &Writer, rl: &RateLimiter, snap: &IrradianceSnapshot) {
    let ts = now_ms();
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("irradiance_wm2", ts, snap.irradiance_wm2 as f64),
         "irradiance_wm2");
}

// =============================================================================
// Venus MPPT (SolarCharger)
// =============================================================================

pub fn write_venus_mppt(writer: &Writer, rl: &RateLimiter, mppt: &VenusMppt) {
    let ts = now_ms();
    let instance = mppt.instance.to_string();

    if let Some(p) = mppt.power_w {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_mppt_power_w", ts, p as f64).with_label("instance", instance.clone()),
             &format!("venus_mppt_power_w:{}", instance));
    }
    if let Some(y) = mppt.yield_today_kwh {
        push(writer, rl, ENERGY_WRITE_INTERVAL,
             Sample::new("venus_mppt_yield_today_kwh", ts, y as f64).with_label("instance", instance.clone()),
             &format!("venus_mppt_yield_today_kwh:{}", instance));
    }
    if let Some(m) = mppt.max_power_today_w {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_mppt_max_power_today_w", ts, m as f64).with_label("instance", instance.clone()),
             &format!("venus_mppt_max_power_today_w:{}", instance));
    }
    if let Some(v) = mppt.pv_voltage_v {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_mppt_pv_voltage_v", ts, v as f64).with_label("instance", instance.clone()),
             &format!("venus_mppt_pv_voltage_v:{}", instance));
    }
    if let Some(i) = mppt.dc_current_a {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_mppt_dc_current_a", ts, i as f64).with_label("instance", instance.clone()),
             &format!("venus_mppt_dc_current_a:{}", instance));
    }
}

// =============================================================================
// Venus SmartShunt
// =============================================================================

pub fn write_venus_smartshunt(writer: &Writer, rl: &RateLimiter, shunt: &VenusSmartShunt) {
    let ts = now_ms();

    if let Some(v) = shunt.soc_percent {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_shunt_soc_percent", ts, v as f64),
             "venus_shunt_soc_percent");
    }
    if let Some(v) = shunt.voltage_v {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_shunt_voltage_v", ts, v as f64),
             "venus_shunt_voltage_v");
    }
    if let Some(v) = shunt.current_a {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_shunt_current_a", ts, v as f64),
             "venus_shunt_current_a");
    }
    if let Some(v) = shunt.power_w {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_shunt_power_w", ts, v as f64),
             "venus_shunt_power_w");
    }
    if let Some(v) = shunt.energy_in_kwh {
        push(writer, rl, ENERGY_WRITE_INTERVAL,
             Sample::new("venus_shunt_energy_in_kwh", ts, v as f64),
             "venus_shunt_energy_in_kwh");
    }
    if let Some(v) = shunt.energy_out_kwh {
        push(writer, rl, ENERGY_WRITE_INTERVAL,
             Sample::new("venus_shunt_energy_out_kwh", ts, v as f64),
             "venus_shunt_energy_out_kwh");
    }
    if let Some(v) = shunt.ah_charged_today {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_shunt_ah_charged_today", ts, v as f64),
             "venus_shunt_ah_charged_today");
    }
    if let Some(v) = shunt.ah_discharged_today {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_shunt_ah_discharged_today", ts, v as f64),
             "venus_shunt_ah_discharged_today");
    }
}

// =============================================================================
// Venus Inverter
// =============================================================================

pub fn write_venus_inverter(writer: &Writer, rl: &RateLimiter, inv: &VenusInverter) {
    let ts = now_ms();
    // Tension/courant DC d'entrée (côté batterie).
    if let Some(v) = inv.voltage_v {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_inverter_voltage_v", ts, v as f64),
             "venus_inverter_voltage_v");
    }
    if let Some(v) = inv.current_a {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_inverter_current_a", ts, v as f64),
             "venus_inverter_current_a");
    }
    // AC d'entrée — gardé en `venus_inverter_power_w` pour compat ascendante du
    // champ DC d'entrée historique (renommage explicite ailleurs).
    if let Some(v) = inv.power_w {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_inverter_power_w", ts, v as f64),
             "venus_inverter_power_w");
    }
    // AC de sortie (cible des dashboards ESS).
    if let Some(v) = inv.ac_output_power_w {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_inverter_ac_output_power_w", ts, v as f64),
             "venus_inverter_ac_output_power_w");
    }
    if let Some(v) = inv.ac_output_voltage_v {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_inverter_ac_output_voltage_v", ts, v as f64),
             "venus_inverter_ac_output_voltage_v");
    }
    if let Some(v) = inv.ac_output_current_a {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_inverter_ac_output_current_a", ts, v as f64),
             "venus_inverter_ac_output_current_a");
    }
    if let Some(v) = inv.ac_out_frequency_hz {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_inverter_ac_freq_hz", ts, v as f64),
             "venus_inverter_ac_freq_hz");
    }
    if let Some(v) = inv.ac_in_ignore {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_inverter_ac_in_ignore", ts, v as i32 as f64),
             "venus_inverter_ac_in_ignore");
    }
}

// =============================================================================
// Venus Temperature
// =============================================================================

pub fn write_venus_temperature(writer: &Writer, rl: &RateLimiter, temp: &VenusTemperature) {
    let ts = now_ms();
    let instance = temp.instance.to_string();

    if let Some(c) = temp.temp_c {
        push(writer, rl, TEMP_WRITE_INTERVAL,
             Sample::new("venus_temp_c", ts, c as f64).with_label("instance", instance.clone()),
             &format!("venus_temp_c:{}", instance));
    }
    if let Some(h) = temp.humidity_percent {
        push(writer, rl, TEMP_WRITE_INTERVAL,
             Sample::new("venus_humidity_percent", ts, h as f64).with_label("instance", instance),
             &format!("venus_humidity_percent:{}", temp.instance));
    }
}

// =============================================================================
// Venus Heatpump
// =============================================================================

pub fn write_venus_heatpump(writer: &Writer, rl: &RateLimiter, hp: &VenusHeatpump) {
    let ts = now_ms();
    let idx = hp.mqtt_index.to_string();

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("venus_heatpump_power_w", ts, hp.ac_power as f64).with_label("mqtt_index", idx.clone()),
         &format!("venus_heatpump_power_w:{}", idx));

    push(writer, rl, ENERGY_WRITE_INTERVAL,
         Sample::new("venus_heatpump_energy_kwh", ts, hp.ac_energy_forward as f64).with_label("mqtt_index", idx.clone()),
         &format!("venus_heatpump_energy_kwh:{}", idx));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("venus_heatpump_state", ts, hp.state as f64).with_label("mqtt_index", idx.clone()),
         &format!("venus_heatpump_state:{}", idx));

    if let Some(t) = hp.temperature {
        push(writer, rl, TEMP_WRITE_INTERVAL,
             Sample::new("venus_heatpump_temp_c", ts, t as f64).with_label("mqtt_index", idx.clone()),
             &format!("venus_heatpump_temp_c:{}", idx));
    }
    if let Some(t) = hp.target_temperature {
        push(writer, rl, TEMP_WRITE_INTERVAL,
             Sample::new("venus_heatpump_target_temp_c", ts, t as f64).with_label("mqtt_index", idx),
             &format!("venus_heatpump_target_temp_c:{}", hp.mqtt_index));
    }
}

// =============================================================================
// ATS CHINT
// =============================================================================

pub fn write_ats(writer: &Writer, rl: &RateLimiter, snap: &AtsSnapshot) {
    let ts = now_ms();

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("ats_active_source", ts, snap.active_source as i32 as f64),
         "ats_active_source");

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("ats_v1a", ts, snap.v1a as f64), "ats_v1a");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("ats_v1b", ts, snap.v1b as f64), "ats_v1b");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("ats_v1c", ts, snap.v1c as f64), "ats_v1c");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("ats_v2a", ts, snap.v2a as f64), "ats_v2a");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("ats_v2b", ts, snap.v2b as f64), "ats_v2b");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("ats_v2c", ts, snap.v2c as f64), "ats_v2c");

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("ats_freq1_hz", ts, snap.freq1_hz.unwrap_or(0) as f64), "ats_freq1_hz");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("ats_freq2_hz", ts, snap.freq2_hz.unwrap_or(0) as f64), "ats_freq2_hz");

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("ats_sw1_closed", ts, snap.sw1_closed as i32 as f64), "ats_sw1_closed");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("ats_sw2_closed", ts, snap.sw2_closed as i32 as f64), "ats_sw2_closed");
}

// =============================================================================
// Tasmota
// =============================================================================

pub fn write_tasmota(writer: &Writer, rl: &RateLimiter, snap: &TasmotaSnapshot) {
    let ts = now_ms();
    let id = snap.id.to_string();

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("tasmota_power_w", ts, snap.power_w as f64).with_label("id", id.clone()),
         &format!("tasmota_power_w:{}", id));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("tasmota_voltage_v", ts, snap.voltage_v as f64).with_label("id", id.clone()),
         &format!("tasmota_voltage_v:{}", id));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("tasmota_current_a", ts, snap.current_a as f64).with_label("id", id.clone()),
         &format!("tasmota_current_a:{}", id));

    push(writer, rl, ENERGY_WRITE_INTERVAL,
         Sample::new("tasmota_energy_today_kwh", ts, snap.energy_today_kwh as f64).with_label("id", id),
         &format!("tasmota_energy_today_kwh:{}", snap.id));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("tasmota_power_on", ts, snap.power_on as i32 as f64)
             .with_label("id", snap.id.to_string()),
         &format!("tasmota_power_on:{}", snap.id));
}

// =============================================================================
// Shelly
// =============================================================================

pub fn write_shelly(writer: &Writer, rl: &RateLimiter, snap: &ShellyEmSnapshot) {
    let ts = now_ms();
    let id = snap.id.to_string();

    // Puissance totale = ch0 + ch1 (déjà agrégée dans le snapshot).
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("shelly_power_w", ts, snap.total_power_w as f64).with_label("id", id.clone()),
         &format!("shelly_power_w:{}", id));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("shelly_voltage_v", ts, snap.channel_0.voltage_v as f64).with_label("id", id.clone()),
         &format!("shelly_voltage_v:{}", id));

    // Détail par canal.
    for (ch_idx, ch) in [(0u8, &snap.channel_0), (1u8, &snap.channel_1)] {
        let ch_label = ch_idx.to_string();
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("shelly_channel_power_w", ts, ch.power_w as f64)
                 .with_label("id", id.clone()).with_label("channel", ch_label.clone()),
             &format!("shelly_channel_power_w:{}:{}", id, ch_idx));
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("shelly_current_a", ts, ch.current_a as f64)
                 .with_label("id", id.clone()).with_label("channel", ch_label.clone()),
             &format!("shelly_current_a:{}:{}", id, ch_idx));
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("shelly_output", ts, ch.output as i32 as f64)
                 .with_label("id", id.clone()).with_label("channel", ch_label.clone()),
             &format!("shelly_output:{}:{}", id, ch_idx));
        push(writer, rl, ENERGY_WRITE_INTERVAL,
             Sample::new("shelly_energy_wh", ts, ch.energy_wh)
                 .with_label("id", id.clone()).with_label("channel", ch_label),
             &format!("shelly_energy_wh:{}:{}", id, ch_idx));
    }
}

// =============================================================================
// Solar total agrégé (calculé séparément, pas un snapshot)
// =============================================================================

pub fn write_solar_total(writer: &Writer, rl: &RateLimiter, solar_total_w: f32) {
    let ts = now_ms();
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("solar_total_w", ts, solar_total_w as f64),
         "solar_total_w");
}

/// Composants de la puissance solaire (publiés par energy-manager via POST).
pub fn write_solar_components(writer: &Writer, rl: &RateLimiter, dc_pv_w: Option<f32>, pvinv_w: Option<f32>) {
    let ts = now_ms();
    if let Some(v) = dc_pv_w {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("dc_pv_power_w", ts, v as f64),
             "dc_pv_power_w");
    }
    if let Some(v) = pvinv_w {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("pvinv_power_w", ts, v as f64),
             "pvinv_power_w");
    }
}

/// Énergie solaire cumulée du jour (kWh + Wh — convention dashboards).
pub fn write_solar_yield(writer: &Writer, rl: &RateLimiter, yield_kwh: f32) {
    let ts = now_ms();
    push(writer, rl, ENERGY_WRITE_INTERVAL,
         Sample::new("solar_yield_kwh", ts, yield_kwh as f64),
         "solar_yield_kwh");
    push(writer, rl, ENERGY_WRITE_INTERVAL,
         Sample::new("solar_total_wh", ts, yield_kwh as f64 * 1000.0),
         "solar_total_wh");
}

// =============================================================================
// Monitor Pi5 (cpu, mem, disk, load, temp, réseau)
// =============================================================================

pub fn write_monitor(writer: &Writer, rl: &RateLimiter, snap: &crate::state::MonitorSnapshot) {
    let ts = now_ms();
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("pi5_cpu_percent", ts, snap.cpu_percent as f64),
         "pi5_cpu_percent");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("pi5_memory_percent", ts, snap.memory_percent as f64),
         "pi5_memory_percent");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("pi5_disk_percent", ts, snap.disk_percent as f64),
         "pi5_disk_percent");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("pi5_load_avg", ts, snap.load_avg[0] as f64)
             .with_label("window", "1m".to_string()),
         "pi5_load_avg:1m");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("pi5_load_avg", ts, snap.load_avg[1] as f64)
             .with_label("window", "5m".to_string()),
         "pi5_load_avg:5m");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("pi5_load_avg", ts, snap.load_avg[2] as f64)
             .with_label("window", "15m".to_string()),
         "pi5_load_avg:15m");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("pi5_mem_used_mb", ts, snap.mem_used_mb as f64),
         "pi5_mem_used_mb");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("pi5_swap_used_mb", ts, snap.swap_used_mb as f64),
         "pi5_swap_used_mb");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("pi5_net_rx_bps", ts, snap.net_rx_bps as f64),
         "pi5_net_rx_bps");
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("pi5_net_tx_bps", ts, snap.net_tx_bps as f64),
         "pi5_net_tx_bps");
    if let Some(t) = snap.cpu_temp_c {
        push(writer, rl, TEMP_WRITE_INTERVAL,
             Sample::new("pi5_cpu_temp_c", ts, t as f64),
             "pi5_cpu_temp_c");
    }
}

// =============================================================================
// Energy-manager + Water heater (publiés via MQTT par energy-manager)
// =============================================================================

/// Métriques `em_*` reçues depuis le topic `santuario/em/metrics` (publié
/// par energy-manager). Toutes optionnelles — on n'écrit que les champs
/// effectivement présents dans le payload.
#[derive(Default, Debug, Clone)]
pub struct EmMetricsPayload {
    pub cpu_percent: Option<f32>,
    pub cpu_temp_c: Option<f32>,
    pub memory_percent: Option<f32>,
    pub mem_used_mb: Option<f32>,
    pub swap_used_mb: Option<f32>,
    pub disk_percent: Option<f32>,
    pub load_avg_1m: Option<f32>,
    pub load_avg_5m: Option<f32>,
    pub load_avg_15m: Option<f32>,
    pub net_rx_bps: Option<f32>,
    pub net_tx_bps: Option<f32>,
}

pub fn write_em_metrics(writer: &Writer, rl: &RateLimiter, m: &EmMetricsPayload) {
    let ts = now_ms();
    if let Some(v) = m.cpu_percent {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("em_cpu_percent", ts, v as f64), "em_cpu_percent");
    }
    if let Some(v) = m.cpu_temp_c {
        push(writer, rl, TEMP_WRITE_INTERVAL,
             Sample::new("em_cpu_temp_c", ts, v as f64), "em_cpu_temp_c");
    }
    if let Some(v) = m.memory_percent {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("em_memory_percent", ts, v as f64), "em_memory_percent");
    }
    if let Some(v) = m.mem_used_mb {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("em_mem_used_mb", ts, v as f64), "em_mem_used_mb");
    }
    if let Some(v) = m.swap_used_mb {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("em_swap_used_mb", ts, v as f64), "em_swap_used_mb");
    }
    if let Some(v) = m.disk_percent {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("em_disk_percent", ts, v as f64), "em_disk_percent");
    }
    if let Some(v) = m.load_avg_1m {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("em_load_avg", ts, v as f64).with_label("window", "1m".to_string()),
             "em_load_avg:1m");
    }
    if let Some(v) = m.load_avg_5m {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("em_load_avg", ts, v as f64).with_label("window", "5m".to_string()),
             "em_load_avg:5m");
    }
    if let Some(v) = m.load_avg_15m {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("em_load_avg", ts, v as f64).with_label("window", "15m".to_string()),
             "em_load_avg:15m");
    }
    if let Some(v) = m.net_rx_bps {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("em_net_rx_bps", ts, v as f64), "em_net_rx_bps");
    }
    if let Some(v) = m.net_tx_bps {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("em_net_tx_bps", ts, v as f64), "em_net_tx_bps");
    }
}

/// Métriques chauffe-eau LG ThinQ (`wh_*`) reçues depuis le topic
/// `santuario/em/water_heater`.
#[derive(Default, Debug, Clone)]
pub struct WhMetricsPayload {
    pub current_temp_c: Option<f32>,
    pub target_temp_c: Option<f32>,
    pub mode: Option<i32>,
}

pub fn write_wh_metrics(writer: &Writer, rl: &RateLimiter, m: &WhMetricsPayload) {
    let ts = now_ms();
    if let Some(v) = m.current_temp_c {
        push(writer, rl, TEMP_WRITE_INTERVAL,
             Sample::new("wh_current_temp_c", ts, v as f64), "wh_current_temp_c");
    }
    if let Some(v) = m.target_temp_c {
        push(writer, rl, TEMP_WRITE_INTERVAL,
             Sample::new("wh_target_temp_c", ts, v as f64), "wh_target_temp_c");
    }
    if let Some(v) = m.mode {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("wh_mode", ts, v as f64), "wh_mode");
    }
}
