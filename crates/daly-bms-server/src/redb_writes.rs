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
         Sample::new("bms_power_w", ts, snap.dc.power as f64)
             .with_label("bms_id", bms_id.clone()),
         &format!("bms_power_w:{}", bms_id));

    push(writer, rl, TEMP_WRITE_INTERVAL,
         Sample::new("bms_temp_max", ts, snap.system.max_cell_temperature as f64)
             .with_label("bms_id", bms_id.clone()),
         &format!("bms_temp_max:{}", bms_id));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("bms_cell_delta_mv", ts, snap.system.cell_delta_mv() as f64)
             .with_label("bms_id", bms_id),
         &format!("bms_cell_delta_mv:0x{:02x}", snap.address));
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
             Sample::new("mppt_power_w", ts, p as f64).with_label("instance", instance.clone()),
             &format!("mppt_power_w:{}", instance));
    }
    if let Some(y) = mppt.yield_today_kwh {
        push(writer, rl, ENERGY_WRITE_INTERVAL,
             Sample::new("mppt_yield_today_kwh", ts, y as f64).with_label("instance", instance.clone()),
             &format!("mppt_yield_today_kwh:{}", instance));
    }
    if let Some(v) = mppt.pv_voltage_v {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("mppt_pv_voltage_v", ts, v as f64).with_label("instance", instance.clone()),
             &format!("mppt_pv_voltage_v:{}", instance));
    }
    if let Some(i) = mppt.dc_current_a {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("mppt_dc_current_a", ts, i as f64).with_label("instance", instance.clone()),
             &format!("mppt_dc_current_a:{}", instance));
    }
}

// =============================================================================
// Venus SmartShunt
// =============================================================================

pub fn write_venus_smartshunt(writer: &Writer, rl: &RateLimiter, shunt: &VenusSmartShunt) {
    let ts = now_ms();

    if let Some(v) = shunt.soc_percent {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_shunt_soc", ts, v as f64),
             "venus_shunt_soc");
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
    if let Some(v) = inv.power_w {
        push(writer, rl, MIN_WRITE_INTERVAL,
             Sample::new("venus_inverter_power_w", ts, v as f64),
             "venus_inverter_power_w");
    }
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
         Sample::new("heatpump_power_w", ts, hp.ac_power as f64).with_label("mqtt_index", idx.clone()),
         &format!("heatpump_power_w:{}", idx));

    push(writer, rl, ENERGY_WRITE_INTERVAL,
         Sample::new("heatpump_energy_kwh", ts, hp.ac_energy_forward as f64).with_label("mqtt_index", idx.clone()),
         &format!("heatpump_energy_kwh:{}", idx));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("heatpump_state", ts, hp.state as f64).with_label("mqtt_index", idx.clone()),
         &format!("heatpump_state:{}", idx));

    if let Some(t) = hp.temperature {
        push(writer, rl, TEMP_WRITE_INTERVAL,
             Sample::new("heatpump_temp_c", ts, t as f64).with_label("mqtt_index", idx.clone()),
             &format!("heatpump_temp_c:{}", idx));
    }
    if let Some(t) = hp.target_temperature {
        push(writer, rl, TEMP_WRITE_INTERVAL,
             Sample::new("heatpump_target_temp_c", ts, t as f64).with_label("mqtt_index", idx),
             &format!("heatpump_target_temp_c:{}", hp.mqtt_index));
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

    // Détail par canal pour les analyses fines.
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("shelly_channel_power_w", ts, snap.channel_0.power_w as f64)
             .with_label("id", id.clone()).with_label("channel", "0"),
         &format!("shelly_channel_power_w:{}:0", id));
    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("shelly_channel_power_w", ts, snap.channel_1.power_w as f64)
             .with_label("id", id.clone()).with_label("channel", "1"),
         &format!("shelly_channel_power_w:{}:1", id));

    push(writer, rl, MIN_WRITE_INTERVAL,
         Sample::new("shelly_voltage_v", ts, snap.channel_0.voltage_v as f64).with_label("id", id),
         &format!("shelly_voltage_v:{}", snap.id));
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
