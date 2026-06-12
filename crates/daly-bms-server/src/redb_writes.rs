//! Module redb writer — pousse les samples des snapshots vers metrics-store::Writer.
//!
//! Il est appelé depuis chaque `state.on_*_snapshot()` pour pousser les
//! valeurs mesurées dans la TSDB redb (seule source de vérité des séries).
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
/// Garde le rate global gérable (1 écriture / 5 s max par série).
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
    ///
    /// `get_mut` + mise à jour en place : la `String` de la clé n'est allouée
    /// qu'à la PREMIÈRE rencontre d'une série, plus jamais ensuite (review
    /// gemini phase D — zéro allocation sur le chemin chaud).
    pub fn allow(&self, key: &str, min_interval: Duration) -> bool {
        let now = Instant::now();
        // unwrap_or_else pour rester safe si un panic a empoisonné le mutex.
        let mut map = self.last_writes.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(last) = map.get_mut(key) {
            if now.duration_since(*last) < min_interval {
                return false;
            }
            *last = now;
            true
        } else {
            map.insert(key.to_string(), now);
            true
        }
    }
}

/// Helper interne — push si rate-limit OK.
///
/// Deux garde-fous anti-churn (investigation RSS,
/// docs/diagnostic-depannage.md §17 + review gemini) :
/// - `make` est une closure : le `Sample` (String metric + Strings labels)
///   n'est construit QUE si le rate-limiter autorise l'écriture (~20 % des
///   appels — snapshots à ~1 Hz, écriture 1×/5 s par série) ;
/// - `key` est un `fmt::Arguments` rendu dans un buffer thread-local
///   réutilisé : zéro allocation heap pour la clé, même sur les ~80 %
///   d'appels rejetés (l'ancien `&format!(…)` allouait une String par appel).
fn push(
    writer: &Writer,
    rl: &RateLimiter,
    interval: Duration,
    key: std::fmt::Arguments<'_>,
    make: impl FnOnce() -> Sample,
) {
    use std::fmt::Write as _;
    thread_local! {
        static KEY_BUF: std::cell::RefCell<String> =
            std::cell::RefCell::new(String::with_capacity(64));
    }
    KEY_BUF.with(|buf| {
        let mut buf = buf.borrow_mut();
        buf.clear();
        // write_fmt sur String est infaillible.
        let _ = buf.write_fmt(key);
        if rl.allow(&buf, interval) {
            let _ = writer.try_write(make());
        }
    });
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

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_voltage:{}", bms_id),
         || Sample::new("bms_voltage", ts, snap.dc.voltage as f64)
            .with_label("bms_id", bms_id.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_current:{}", bms_id),
         || Sample::new("bms_current", ts, snap.dc.current as f64)
            .with_label("bms_id", bms_id.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_soc:{}", bms_id),
         || Sample::new("bms_soc", ts, snap.soc as f64)
            .with_label("bms_id", bms_id.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_power:{}", bms_id),
         || Sample::new("bms_power", ts, snap.dc.power as f64)
            .with_label("bms_id", bms_id.clone()));

    push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("bms_temp_max:{}", bms_id),
         || Sample::new("bms_temp_max", ts, snap.system.max_cell_temperature as f64)
            .with_label("bms_id", bms_id.clone()));

    push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("bms_temp_min:{}", bms_id),
         || Sample::new("bms_temp_min", ts, snap.system.min_cell_temperature as f64)
            .with_label("bms_id", bms_id.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_cell_delta_mv:{}", bms_id),
         || Sample::new("bms_cell_delta_mv", ts, snap.system.cell_delta_mv() as f64)
            .with_label("bms_id", bms_id.clone()));

    // Capacité nominale installée (config statique, change uniquement à la conf).
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("bms_capacity_ah:{}", bms_id),
         || Sample::new("bms_capacity_ah", ts, snap.installed_capacity as f64)
            .with_label("bms_id", bms_id.clone()));

    // État des MOSFETs (proxy via IO permissions — l'API 0x93 n'est pas dans le snapshot).
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_charge_mos:{}", bms_id),
         || Sample::new("bms_charge_mos", ts, snap.io.allow_to_charge as f64)
            .with_label("bms_id", bms_id.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_discharge_mos:{}", bms_id),
         || Sample::new("bms_discharge_mos", ts, snap.io.allow_to_discharge as f64)
            .with_label("bms_id", bms_id.clone()));

    // Tension par cellule. La clé est "Cell1", "Cell2", … — on extrait l'index numérique.
    for (key, &voltage) in snap.voltages.iter() {
        let cell_idx = key.trim_start_matches("Cell");
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_cell_voltage:{}:{}", bms_id, cell_idx),
             || Sample::new("bms_cell_voltage", ts, voltage as f64)
                .with_label("bms_id", bms_id.clone())
                .with_label("cell", cell_idx.to_string()));
    }

    // État d'équilibrage par cellule (Balances Cell1..N → 0/1).
    for (key, &active) in snap.balances.iter() {
        let cell_idx = key.trim_start_matches("Cell");
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_cell_balancing:{}:{}", bms_id, cell_idx),
             || Sample::new("bms_cell_balancing", ts, active as f64)
                .with_label("bms_id", bms_id.clone())
                .with_label("cell", cell_idx.to_string()));
    }

    // ── État de santé & temps restant ────────────────────────────────────────
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_soh:{}", bms_id),
         || Sample::new("bms_soh", ts, snap.soh as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_time_to_go_secs:{}", bms_id),
         || Sample::new("bms_time_to_go_secs", ts, snap.time_to_go as f64)
            .with_label("bms_id", bms_id.clone()));

    // ── États bool/flag ──────────────────────────────────────────────────────
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_balancing_active:{}", bms_id),
         || Sample::new("bms_balancing_active", ts, snap.balancing as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_system_switch:{}", bms_id),
         || Sample::new("bms_system_switch", ts, snap.system_switch as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_heating_active:{}", bms_id),
         || Sample::new("bms_heating_active", ts, snap.heating as f64)
            .with_label("bms_id", bms_id.clone()));

    // ── Tensions / température DC ───────────────────────────────────────────
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_min_cell_voltage:{}", bms_id),
         || Sample::new("bms_min_cell_voltage", ts, snap.system.min_cell_voltage as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_max_cell_voltage:{}", bms_id),
         || Sample::new("bms_max_cell_voltage", ts, snap.system.max_cell_voltage as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("bms_mos_temp_c:{}", bms_id),
         || Sample::new("bms_mos_temp_c", ts, snap.system.mos_temperature as f64)
            .with_label("bms_id", bms_id.clone()));

    // ── Capacités (Ah) ───────────────────────────────────────────────────────
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("bms_consumed_ah:{}", bms_id),
         || Sample::new("bms_consumed_ah", ts, snap.consumed_amphours as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("bms_capacity_remaining_ah:{}", bms_id),
         || Sample::new("bms_capacity_remaining_ah", ts, snap.capacity as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("bms_reported_capacity_ah:{}", bms_id),
         || Sample::new("bms_reported_capacity_ah", ts, snap.bms_reported_capacity_ah as f64)
            .with_label("bms_id", bms_id.clone()));

    // ── Modules ──────────────────────────────────────────────────────────────
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_modules_online:{}", bms_id),
         || Sample::new("bms_modules_online", ts, snap.system.nr_of_modules_online as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_modules_offline:{}", bms_id),
         || Sample::new("bms_modules_offline", ts, snap.system.nr_of_modules_offline as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_modules_blocking_charge:{}", bms_id),
         || Sample::new("bms_modules_blocking_charge", ts, snap.system.nr_of_modules_blocking_charge as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_modules_blocking_discharge:{}", bms_id),
         || Sample::new("bms_modules_blocking_discharge", ts, snap.system.nr_of_modules_blocking_discharge as f64)
            .with_label("bms_id", bms_id.clone()));

    // ── Alarmes (13 flags) ──────────────────────────────────────────────────
    let alarms: [(&str, u8); 13] = [
        ("bms_alarm_low_voltage",            snap.alarms.low_voltage),
        ("bms_alarm_high_voltage",           snap.alarms.high_voltage),
        ("bms_alarm_low_soc",                snap.alarms.low_soc),
        ("bms_alarm_high_charge_current",    snap.alarms.high_charge_current),
        ("bms_alarm_high_discharge_current", snap.alarms.high_discharge_current),
        ("bms_alarm_high_current",           snap.alarms.high_current),
        ("bms_alarm_cell_imbalance",         snap.alarms.cell_imbalance),
        ("bms_alarm_high_charge_temp",       snap.alarms.high_charge_temperature),
        ("bms_alarm_low_charge_temp",        snap.alarms.low_charge_temperature),
        ("bms_alarm_low_cell_voltage",       snap.alarms.low_cell_voltage),
        ("bms_alarm_low_temp",               snap.alarms.low_temperature),
        ("bms_alarm_high_temp",              snap.alarms.high_temperature),
        ("bms_alarm_fuse_blown",             snap.alarms.fuse_blown),
    ];
    for (name, v) in alarms {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("{}:{}", name, bms_id),
             || Sample::new(name, ts, v as f64).with_label("bms_id", bms_id.clone()));
    }

    // ── Historique de vie ────────────────────────────────────────────────────
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("bms_charge_cycles:{}", bms_id),
         || Sample::new("bms_charge_cycles", ts, snap.history.charge_cycles as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("bms_min_voltage_hist:{}", bms_id),
         || Sample::new("bms_min_voltage_hist", ts, snap.history.minimum_voltage as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("bms_max_voltage_hist:{}", bms_id),
         || Sample::new("bms_max_voltage_hist", ts, snap.history.maximum_voltage as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("bms_total_ah_drawn:{}", bms_id),
         || Sample::new("bms_total_ah_drawn", ts, snap.history.total_ah_drawn as f64)
            .with_label("bms_id", bms_id.clone()));

    // ── Limites de charge ───────────────────────────────────────────────────
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_charge_request:{}", bms_id),
         || Sample::new("bms_charge_request", ts, snap.info.charge_request as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_max_charge_voltage:{}", bms_id),
         || Sample::new("bms_max_charge_voltage", ts, snap.info.max_charge_voltage as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_max_charge_current:{}", bms_id),
         || Sample::new("bms_max_charge_current", ts, snap.info.max_charge_current as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_max_discharge_current:{}", bms_id),
         || Sample::new("bms_max_discharge_current", ts, snap.info.max_discharge_current as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_max_charge_cell_voltage:{}", bms_id),
         || Sample::new("bms_max_charge_cell_voltage", ts, snap.info.max_charge_cell_voltage as f64)
            .with_label("bms_id", bms_id.clone()));

    // ── IO permissions (autorisations BMS) ──────────────────────────────────
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_balance_mos:{}", bms_id),
         || Sample::new("bms_balance_mos", ts, snap.io.allow_to_balance as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_heat_mos:{}", bms_id),
         || Sample::new("bms_heat_mos", ts, snap.io.allow_to_heat as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_external_relay:{}", bms_id),
         || Sample::new("bms_external_relay", ts, snap.io.external_relay as f64)
            .with_label("bms_id", bms_id.clone()));

    // ── Chauffage (consigne / consommation) ─────────────────────────────────
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_heating_current:{}", bms_id),
         || Sample::new("bms_heating_current", ts, snap.info.heating_current as f64)
            .with_label("bms_id", bms_id.clone()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("bms_heating_power:{:02x}", snap.address),
         || Sample::new("bms_heating_power", ts, snap.info.heating_power as f64)
            .with_label("bms_id", bms_id));
}

// =============================================================================
// ET112
// =============================================================================

pub fn write_et112(writer: &Writer, rl: &RateLimiter, snap: &Et112Snapshot) {
    let ts = now_ms();
    let addr = format!("0x{:02x}", snap.address);

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("et112_power_w:{}", addr),
         || Sample::new("et112_power_w", ts, snap.power_w as f64)
            .with_label("address", addr.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("et112_voltage_v:{}", addr),
         || Sample::new("et112_voltage_v", ts, snap.voltage_v as f64)
            .with_label("address", addr.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("et112_current_a:{}", addr),
         || Sample::new("et112_current_a", ts, snap.current_a as f64)
            .with_label("address", addr.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("et112_apparent_power_va:{}", addr),
         || Sample::new("et112_apparent_power_va", ts, snap.apparent_power_va as f64)
            .with_label("address", addr.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("et112_power_factor:{}", addr),
         || Sample::new("et112_power_factor", ts, snap.power_factor as f64)
            .with_label("address", addr.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("et112_frequency_hz:{}", addr),
         || Sample::new("et112_frequency_hz", ts, snap.frequency_hz as f64)
            .with_label("address", addr.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("et112_reactive_power_var:{}", addr),
         || Sample::new("et112_reactive_power_var", ts, snap.reactive_power_var as f64)
            .with_label("address", addr.clone()));

    // Énergie cumulée en Wh (compteur monotone).
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("et112_energy_import_wh:{}", addr),
         || Sample::new("et112_energy_import_wh", ts, snap.energy_import_kwh() as f64 * 1000.0)
            .with_label("address", addr.clone()));

    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("et112_energy_export_wh:0x{:02x}", snap.address),
         || Sample::new("et112_energy_export_wh", ts, snap.energy_export_kwh() as f64 * 1000.0)
            .with_label("address", addr));
}

// =============================================================================
// Irradiance
// =============================================================================

pub fn write_irradiance(writer: &Writer, rl: &RateLimiter, snap: &IrradianceSnapshot) {
    let ts = now_ms();
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("irradiance_wm2"),
         || Sample::new("irradiance_wm2", ts, snap.irradiance_wm2 as f64));
}

// =============================================================================
// Venus MPPT (SolarCharger)
// =============================================================================

pub fn write_venus_mppt(writer: &Writer, rl: &RateLimiter, mppt: &VenusMppt) {
    let ts = now_ms();
    let instance = mppt.instance.to_string();

    if let Some(p) = mppt.power_w {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_mppt_power_w:{}", instance),
             || Sample::new("venus_mppt_power_w", ts, p as f64).with_label("instance", instance.clone()));
    }
    if let Some(y) = mppt.yield_today_kwh {
        push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("venus_mppt_yield_today_kwh:{}", instance),
             || Sample::new("venus_mppt_yield_today_kwh", ts, y as f64).with_label("instance", instance.clone()));
    }
    if let Some(m) = mppt.max_power_today_w {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_mppt_max_power_today_w:{}", instance),
             || Sample::new("venus_mppt_max_power_today_w", ts, m as f64).with_label("instance", instance.clone()));
    }
    if let Some(v) = mppt.pv_voltage_v {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_mppt_pv_voltage_v:{}", instance),
             || Sample::new("venus_mppt_pv_voltage_v", ts, v as f64).with_label("instance", instance.clone()));
    }
    if let Some(i) = mppt.dc_current_a {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_mppt_dc_current_a:{}", instance),
             || Sample::new("venus_mppt_dc_current_a", ts, i as f64).with_label("instance", instance.clone()));
    }
    // État du chargeur encodé en numérique : 0=Off, 1=Low power, 2=Fault, 3=Bulk,
    // 4=Absorption, 5=Float, 6=Storage, 7=Equalize, 8=Passthru, 9=Inverting,
    // 10=Power assist, 11=Power supply. Le `None` (état inconnu) ne produit rien.
    if let Some(state_code) = mppt.state.as_ref().and_then(|s| mppt_state_to_code(s)) {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_mppt_state:{}", instance),
             || Sample::new("venus_mppt_state", ts, state_code as f64)
                .with_label("instance", instance.clone()));
    }
}

/// Inverse du switch dans `handle_meteo_topic` : nom Victron → code numérique.
fn mppt_state_to_code(label: &str) -> Option<i32> {
    let code = match label {
        "Off"          => 0,
        "Low power"    => 1,
        "Fault"        => 2,
        "Bulk"         => 3,
        "Absorption"   => 4,
        "Float"        => 5,
        "Storage"      => 6,
        "Equalize"     => 7,
        "Passthru"     => 8,
        "Inverting"    => 9,
        "Power assist" => 10,
        "Power supply" => 11,
        s if s.starts_with("State ") => s.trim_start_matches("State ").parse().ok()?,
        _ => return None,
    };
    Some(code)
}

// =============================================================================
// Venus SmartShunt
// =============================================================================

pub fn write_venus_smartshunt(writer: &Writer, rl: &RateLimiter, shunt: &VenusSmartShunt) {
    let ts = now_ms();

    if let Some(v) = shunt.soc_percent {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_shunt_soc_percent"),
             || Sample::new("venus_shunt_soc_percent", ts, v as f64));
    }
    if let Some(v) = shunt.voltage_v {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_shunt_voltage_v"),
             || Sample::new("venus_shunt_voltage_v", ts, v as f64));
    }
    if let Some(v) = shunt.current_a {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_shunt_current_a"),
             || Sample::new("venus_shunt_current_a", ts, v as f64));
        // Valeur absolue dérivée |I| — alimente le panel "Taux de Cyclage 24h".
        // L'ancienne subquery `abs(avg(clamp_min(I,0))) + abs(avg(clamp_max(I,0)))`
        // se simplifie exactement en `avg(|I|)` (cf. plan §6.5, décision (a)),
        // donc le panel utilise désormais `avg_over_time(venus_shunt_current_abs[24h])`.
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_shunt_current_abs"),
             || Sample::new("venus_shunt_current_abs", ts, (v as f64).abs()));
    }
    if let Some(v) = shunt.power_w {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_shunt_power_w"),
             || Sample::new("venus_shunt_power_w", ts, v as f64));
    }
    if let Some(v) = shunt.energy_in_kwh {
        push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("venus_shunt_energy_in_kwh"),
             || Sample::new("venus_shunt_energy_in_kwh", ts, v as f64));
    }
    if let Some(v) = shunt.energy_out_kwh {
        push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("venus_shunt_energy_out_kwh"),
             || Sample::new("venus_shunt_energy_out_kwh", ts, v as f64));
    }
    if let Some(v) = shunt.ah_charged_today {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_shunt_ah_charged_today"),
             || Sample::new("venus_shunt_ah_charged_today", ts, v as f64));
    }
    if let Some(v) = shunt.ah_discharged_today {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_shunt_ah_discharged_today"),
             || Sample::new("venus_shunt_ah_discharged_today", ts, v as f64));
    }
    // Temps restant en minutes (None si en charge ou inconnu).
    if let Some(v) = shunt.time_to_go_min {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_shunt_time_to_go_min"),
             || Sample::new("venus_shunt_time_to_go_min", ts, v as f64));
    }
    // État batterie : 0=Idle, 1=Charging, 2=Discharging, 3=Unknown.
    if let Some(code) = shunt.state.as_ref().map(|s| shunt_state_to_code(s)) {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_shunt_state"),
             || Sample::new("venus_shunt_state", ts, code as f64));
    }
}

fn shunt_state_to_code(label: &str) -> i32 {
    match label {
        "Idle"        => 0,
        "Charging"    => 1,
        "Discharging" => 2,
        _             => 3,
    }
}

// =============================================================================
// Venus Inverter
// =============================================================================

pub fn write_venus_inverter(writer: &Writer, rl: &RateLimiter, inv: &VenusInverter) {
    let ts = now_ms();
    // Tension/courant DC d'entrée (côté batterie).
    if let Some(v) = inv.voltage_v {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_inverter_voltage_v"),
             || Sample::new("venus_inverter_voltage_v", ts, v as f64));
    }
    if let Some(v) = inv.current_a {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_inverter_current_a"),
             || Sample::new("venus_inverter_current_a", ts, v as f64));
    }
    // AC d'entrée — gardé en `venus_inverter_power_w` pour compat ascendante du
    // champ DC d'entrée historique (renommage explicite ailleurs).
    if let Some(v) = inv.power_w {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_inverter_power_w"),
             || Sample::new("venus_inverter_power_w", ts, v as f64));
    }
    // AC de sortie (cible des dashboards ESS).
    if let Some(v) = inv.ac_output_power_w {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_inverter_ac_output_power_w"),
             || Sample::new("venus_inverter_ac_output_power_w", ts, v as f64));
    }
    if let Some(v) = inv.ac_output_voltage_v {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_inverter_ac_output_voltage_v"),
             || Sample::new("venus_inverter_ac_output_voltage_v", ts, v as f64));
    }
    if let Some(v) = inv.ac_output_current_a {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_inverter_ac_output_current_a"),
             || Sample::new("venus_inverter_ac_output_current_a", ts, v as f64));
    }
    if let Some(v) = inv.ac_out_frequency_hz {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_inverter_ac_freq_hz"),
             || Sample::new("venus_inverter_ac_freq_hz", ts, v as f64));
    }
    if let Some(v) = inv.ac_in_ignore {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_inverter_ac_in_ignore"),
             || Sample::new("venus_inverter_ac_in_ignore", ts, v as i32 as f64));
    }
    // États encodés en numérique (string → code).
    let st_code = inverter_state_to_code(&inv.state);
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_inverter_state"),
         || Sample::new("venus_inverter_state", ts, st_code as f64));
    let mode_code = inverter_mode_to_code(&inv.mode);
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_inverter_mode"),
         || Sample::new("venus_inverter_mode", ts, mode_code as f64));
}

/// État inverter Victron : 0=off, 1=on, 2=inverting, 3=charger, 4=passthrough, 5=other.
fn inverter_state_to_code(label: &str) -> i32 {
    match label.to_lowercase().as_str() {
        "off"         => 0,
        "on"          => 1,
        "inverting"   => 2,
        "charger"     => 3,
        "passthrough" => 4,
        _             => 5,
    }
}

/// Mode inverter Victron : 0=charger, 1=inverter, 2=passthrough, 3=other.
fn inverter_mode_to_code(label: &str) -> i32 {
    match label.to_lowercase().as_str() {
        "charger"     => 0,
        "inverter"    => 1,
        "passthrough" => 2,
        _             => 3,
    }
}

// =============================================================================
// Venus Temperature
// =============================================================================

pub fn write_venus_temperature(writer: &Writer, rl: &RateLimiter, temp: &VenusTemperature) {
    let ts = now_ms();
    let instance = temp.instance.to_string();

    if let Some(c) = temp.temp_c {
        push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("venus_temp_c:{}", instance),
             || Sample::new("venus_temp_c", ts, c as f64).with_label("instance", instance.clone()));
    }
    if let Some(h) = temp.humidity_percent {
        push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("venus_humidity_percent:{}", temp.instance),
             || Sample::new("venus_humidity_percent", ts, h as f64).with_label("instance", instance.clone()));
    }
    if let Some(p) = temp.pressure_mbar {
        push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("venus_pressure_mbar:{}", temp.instance),
             || Sample::new("venus_pressure_mbar", ts, p as f64).with_label("instance", instance.clone()));
    }
    // Bool de connexion : 0 = déconnecté, 1 = en ligne.
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_connected:temp:{}", temp.instance),
         || Sample::new("venus_connected", ts, temp.connected as i32 as f64)
            .with_label("instance", instance)
            .with_label("device_type", "temperature".to_string()));
}

// =============================================================================
// Venus Heatpump
// =============================================================================

pub fn write_venus_heatpump(writer: &Writer, rl: &RateLimiter, hp: &VenusHeatpump) {
    let ts = now_ms();
    let idx = hp.mqtt_index.to_string();

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_heatpump_power_w:{}", idx),
         || Sample::new("venus_heatpump_power_w", ts, hp.ac_power as f64).with_label("mqtt_index", idx.clone()));

    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("venus_heatpump_energy_kwh:{}", idx),
         || Sample::new("venus_heatpump_energy_kwh", ts, hp.ac_energy_forward as f64).with_label("mqtt_index", idx.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_heatpump_state:{}", idx),
         || Sample::new("venus_heatpump_state", ts, hp.state as f64).with_label("mqtt_index", idx.clone()));

    if let Some(t) = hp.temperature {
        push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("venus_heatpump_temp_c:{}", idx),
             || Sample::new("venus_heatpump_temp_c", ts, t as f64).with_label("mqtt_index", idx.clone()));
    }
    if let Some(t) = hp.target_temperature {
        push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("venus_heatpump_target_temp_c:{}", hp.mqtt_index),
             || Sample::new("venus_heatpump_target_temp_c", ts, t as f64).with_label("mqtt_index", idx.clone()));
    }
    // Position : 0=AC Output, 1=AC Input.
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_heatpump_position:{}", hp.mqtt_index),
         || Sample::new("venus_heatpump_position", ts, hp.position as f64)
            .with_label("mqtt_index", idx.clone()));
    // Connected (bool).
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("venus_heatpump_connected:{}", hp.mqtt_index),
         || Sample::new("venus_heatpump_connected", ts, hp.connected as i32 as f64)
            .with_label("mqtt_index", idx));
}

// =============================================================================
// ATS CHINT
// =============================================================================

pub fn write_ats(writer: &Writer, rl: &RateLimiter, snap: &AtsSnapshot) {
    let ts = now_ms();

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_active_source"),
         || Sample::new("ats_active_source", ts, snap.active_source as i32 as f64));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_v1a"),
         || Sample::new("ats_v1a", ts, snap.v1a as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_v1b"),
         || Sample::new("ats_v1b", ts, snap.v1b as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_v1c"),
         || Sample::new("ats_v1c", ts, snap.v1c as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_v2a"),
         || Sample::new("ats_v2a", ts, snap.v2a as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_v2b"),
         || Sample::new("ats_v2b", ts, snap.v2b as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_v2c"),
         || Sample::new("ats_v2c", ts, snap.v2c as f64));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_freq1_hz"),
         || Sample::new("ats_freq1_hz", ts, snap.freq1_hz.unwrap_or(0) as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_freq2_hz"),
         || Sample::new("ats_freq2_hz", ts, snap.freq2_hz.unwrap_or(0) as f64));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_sw1_closed"),
         || Sample::new("ats_sw1_closed", ts, snap.sw1_closed as i32 as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_sw2_closed"),
         || Sample::new("ats_sw2_closed", ts, snap.sw2_closed as i32 as f64));

    // Alias scalaires de la source active (utilisés par certains dashboards
    // Grafana qui ne discriminent pas la phase). On expose la valeur de la
    // source actuellement sélectionnée par l'ATS — pas une moyenne arbitraire.
    // Source1=0 (Onduleur) → v1*/freq1, Source2=1 (Réseau) → v2*/freq2.
    // Neutral (2) ou inconnu : on retombe par défaut sur la source 2 (Réseau).
    let (av_a, av_b, av_c, af_hz) = match snap.active_source as i32 {
        0 => (snap.v1a, snap.v1b, snap.v1c, snap.freq1_hz.unwrap_or(0)),
        _ => (snap.v2a, snap.v2b, snap.v2c, snap.freq2_hz.unwrap_or(0)),
    };
    let ats_voltage_v = ((av_a as f64) + (av_b as f64) + (av_c as f64)) / 3.0;
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_voltage_v"),
         || Sample::new("ats_voltage_v", ts, ats_voltage_v));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_freq_hz"),
         || Sample::new("ats_freq_hz", ts, af_hz as f64));

    // ── Code de défaut (0 = aucun, 1..7 = défauts variés) ───────────────────
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_fault"),
         || Sample::new("ats_fault", ts, snap.fault as i32 as f64));

    // ── Modes / flags ATS ───────────────────────────────────────────────────
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_sw_mode"),
         || Sample::new("ats_sw_mode", ts, snap.sw_mode as i32 as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_remote"),
         || Sample::new("ats_remote", ts, snap.remote as i32 as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("ats_middle_off"),
         || Sample::new("ats_middle_off", ts, snap.middle_off as i32 as f64));

    // ── Statut par phase (0=Normal, 1=UnderVoltage, 2=OverVoltage, 3=Error) ─
    let phases: [(&str, crate::ats::PhaseStatus); 6] = [
        ("ats_phase_s1a", snap.s1a),
        ("ats_phase_s1b", snap.s1b),
        ("ats_phase_s1c", snap.s1c),
        ("ats_phase_s2a", snap.s2a),
        ("ats_phase_s2b", snap.s2b),
        ("ats_phase_s2c", snap.s2c),
    ];
    for (name, status) in phases {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("{name}"),
             || Sample::new(name, ts, status as i32 as f64));
    }

    // ── Compteurs de commutation et runtime ─────────────────────────────────
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("ats_cnt1"),
         || Sample::new("ats_cnt1", ts, snap.cnt1 as f64));
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("ats_cnt2"),
         || Sample::new("ats_cnt2", ts, snap.cnt2 as f64));
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("ats_runtime_h"),
         || Sample::new("ats_runtime_h", ts, snap.runtime_h as f64));

    // ── Tensions max historiques ────────────────────────────────────────────
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("ats_max1_v"),
         || Sample::new("ats_max1_v", ts, snap.max1_v as f64));
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("ats_max2_v"),
         || Sample::new("ats_max2_v", ts, snap.max2_v as f64));
}

// =============================================================================
// Tasmota
// =============================================================================

pub fn write_tasmota(writer: &Writer, rl: &RateLimiter, snap: &TasmotaSnapshot) {
    let ts = now_ms();
    let id = snap.id.to_string();

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("tasmota_power_w:{}", id),
         || Sample::new("tasmota_power_w", ts, snap.power_w as f64).with_label("id", id.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("tasmota_voltage_v:{}", id),
         || Sample::new("tasmota_voltage_v", ts, snap.voltage_v as f64).with_label("id", id.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("tasmota_current_a:{}", id),
         || Sample::new("tasmota_current_a", ts, snap.current_a as f64).with_label("id", id.clone()));

    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("tasmota_energy_today_kwh:{}", snap.id),
         || Sample::new("tasmota_energy_today_kwh", ts, snap.energy_today_kwh as f64).with_label("id", id));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("tasmota_power_on:{}", snap.id),
         || Sample::new("tasmota_power_on", ts, snap.power_on as i32 as f64)
            .with_label("id", snap.id.to_string()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("tasmota_apparent_power_va:{}", snap.id),
         || Sample::new("tasmota_apparent_power_va", ts, snap.apparent_power_va as f64)
            .with_label("id", snap.id.to_string()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("tasmota_power_factor:{}", snap.id),
         || Sample::new("tasmota_power_factor", ts, snap.power_factor as f64)
            .with_label("id", snap.id.to_string()));

    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("tasmota_energy_yesterday_kwh:{}", snap.id),
         || Sample::new("tasmota_energy_yesterday_kwh", ts, snap.energy_yesterday_kwh as f64)
            .with_label("id", snap.id.to_string()));

    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("tasmota_energy_total_kwh:{}", snap.id),
         || Sample::new("tasmota_energy_total_kwh", ts, snap.energy_total_kwh as f64)
            .with_label("id", snap.id.to_string()));

    if let Some(rssi) = snap.rssi {
        push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("tasmota_rssi:{}", snap.id),
             || Sample::new("tasmota_rssi", ts, rssi as f64)
                .with_label("id", snap.id.to_string()));
    }
}

// =============================================================================
// Shelly
// =============================================================================

pub fn write_shelly(writer: &Writer, rl: &RateLimiter, snap: &ShellyEmSnapshot) {
    let ts = now_ms();
    let id = snap.id.to_string();

    // Puissance totale = ch0 + ch1 (déjà agrégée dans le snapshot).
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("shelly_power_w:{}", id),
         || Sample::new("shelly_power_w", ts, snap.total_power_w as f64).with_label("id", id.clone()));

    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("shelly_voltage_v:{}", id),
         || Sample::new("shelly_voltage_v", ts, snap.channel_0.voltage_v as f64).with_label("id", id.clone()));

    // Détail par canal.
    for (ch_idx, ch) in [(0u8, &snap.channel_0), (1u8, &snap.channel_1)] {
        let ch_label = ch_idx.to_string();
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("shelly_channel_power_w:{}:{}", id, ch_idx),
             || Sample::new("shelly_channel_power_w", ts, ch.power_w as f64)
                .with_label("id", id.clone()).with_label("channel", ch_label.clone()));
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("shelly_current_a:{}:{}", id, ch_idx),
             || Sample::new("shelly_current_a", ts, ch.current_a as f64)
                .with_label("id", id.clone()).with_label("channel", ch_label.clone()));
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("shelly_output:{}:{}", id, ch_idx),
             || Sample::new("shelly_output", ts, ch.output as i32 as f64)
                .with_label("id", id.clone()).with_label("channel", ch_label.clone()));
        push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("shelly_energy_wh:{}:{}", id, ch_idx),
             || Sample::new("shelly_energy_wh", ts, ch.energy_wh)
                .with_label("id", id.clone()).with_label("channel", ch_label.clone()));
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("shelly_power_factor:{}:{}", id, ch_idx),
             || Sample::new("shelly_power_factor", ts, ch.power_factor as f64)
                .with_label("id", id.clone()).with_label("channel", ch_label.clone()));
        push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("shelly_returned_wh:{}:{}", id, ch_idx),
             || Sample::new("shelly_returned_wh", ts, ch.returned_wh)
                .with_label("id", id.clone()).with_label("channel", ch_label));
    }
    if let Some(rssi) = snap.rssi {
        push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("shelly_rssi:{}", snap.id),
             || Sample::new("shelly_rssi", ts, rssi as f64).with_label("id", id));
    }
}

// =============================================================================
// Solar total agrégé (calculé séparément, pas un snapshot)
// =============================================================================

pub fn write_solar_total(writer: &Writer, rl: &RateLimiter, solar_total_w: f32) {
    let ts = now_ms();
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("solar_total_w"),
         || Sample::new("solar_total_w", ts, solar_total_w as f64));
    // Alias attendu par les dashboards Grafana drilldown.
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("total_solar_power"),
         || Sample::new("total_solar_power", ts, solar_total_w as f64));
}

/// Composants de la puissance solaire (publiés par energy-manager via POST).
pub fn write_solar_components(writer: &Writer, rl: &RateLimiter, dc_pv_w: Option<f32>, pvinv_w: Option<f32>) {
    let ts = now_ms();
    if let Some(v) = dc_pv_w {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("dc_pv_power_w"),
             || Sample::new("dc_pv_power_w", ts, v as f64));
    }
    if let Some(v) = pvinv_w {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pvinv_power_w"),
             || Sample::new("pvinv_power_w", ts, v as f64));
    }
}

/// Énergie solaire cumulée du jour (kWh + Wh — convention dashboards).
pub fn write_solar_yield(writer: &Writer, rl: &RateLimiter, yield_kwh: f32) {
    let ts = now_ms();
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("solar_yield_kwh"),
         || Sample::new("solar_yield_kwh", ts, yield_kwh as f64));
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("solar_total_wh"),
         || Sample::new("solar_total_wh", ts, yield_kwh as f64 * 1000.0));
}

// =============================================================================
// Monitor Pi5 (cpu, mem, disk, load, temp, réseau)
// =============================================================================

pub fn write_monitor(writer: &Writer, rl: &RateLimiter, snap: &crate::state::MonitorSnapshot) {
    let ts = now_ms();
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_cpu_percent"),
         || Sample::new("pi5_cpu_percent", ts, snap.cpu_percent as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_memory_percent"),
         || Sample::new("pi5_memory_percent", ts, snap.memory_percent as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_disk_percent"),
         || Sample::new("pi5_disk_percent", ts, snap.disk_percent as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_load_avg:1m"),
         || Sample::new("pi5_load_avg", ts, snap.load_avg[0] as f64)
            .with_label("window", "1m".to_string()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_load_avg:5m"),
         || Sample::new("pi5_load_avg", ts, snap.load_avg[1] as f64)
            .with_label("window", "5m".to_string()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_load_avg:15m"),
         || Sample::new("pi5_load_avg", ts, snap.load_avg[2] as f64)
            .with_label("window", "15m".to_string()));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_mem_used_mb"),
         || Sample::new("pi5_mem_used_mb", ts, snap.mem_used_mb as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_swap_used_mb"),
         || Sample::new("pi5_swap_used_mb", ts, snap.swap_used_mb as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_net_rx_bps"),
         || Sample::new("pi5_net_rx_bps", ts, snap.net_rx_bps as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_net_tx_bps"),
         || Sample::new("pi5_net_tx_bps", ts, snap.net_tx_bps as f64));
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_uptime_secs"),
         || Sample::new("pi5_uptime_secs", ts, snap.uptime_secs as f64));
    if let Some(t) = snap.cpu_temp_c {
        push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("pi5_cpu_temp_c"),
             || Sample::new("pi5_cpu_temp_c", ts, t as f64));
    }

    // ── Totaux mémoire / swap (manquaient — utiles pour pourcentage calculé) ─
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("pi5_mem_total_mb"),
         || Sample::new("pi5_mem_total_mb", ts, snap.mem_total_mb as f64));
    push(writer, rl, ENERGY_WRITE_INTERVAL, format_args!("pi5_swap_total_mb"),
         || Sample::new("pi5_swap_total_mb", ts, snap.swap_total_mb as f64));

    // ── État port série RS485 ───────────────────────────────────────────────
    push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_serial_port_ok"),
         || Sample::new("pi5_serial_port_ok", ts, snap.serial_port_ok as i32 as f64));

    // ── État des services systemd (active=1, inactive=0) ────────────────────
    for svc in &snap.services {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_service_active:{}", svc.name),
             || Sample::new("pi5_service_active", ts, svc.active as i32 as f64)
                .with_label("name", svc.name.clone()));
    }
    for svc in &snap.network_services {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_network_service_active:{}", svc.name),
             || Sample::new("pi5_network_service_active", ts, svc.active as i32 as f64)
                .with_label("name", svc.name.clone()));
    }

    // ── Top processus par CPU% (cardinalité contrôlée par les filtres dans
    //    monitor.rs — typiquement < 15 noms agrégés). ──────────────────────────
    for proc in &snap.processes {
        if proc.cpu_percent < 0.1 && proc.mem_rss_mb < 5.0 {
            continue; // ignore bruit
        }
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_process_cpu_percent:{}", proc.name),
             || Sample::new("pi5_process_cpu_percent", ts, proc.cpu_percent as f64)
                .with_label("process", proc.name.clone()));
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("pi5_process_mem_mb:{}", proc.name),
             || Sample::new("pi5_process_mem_mb", ts, proc.mem_rss_mb as f64)
                .with_label("process", proc.name.clone()));
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
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("em_cpu_percent"),
             || Sample::new("em_cpu_percent", ts, v as f64));
    }
    if let Some(v) = m.cpu_temp_c {
        push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("em_cpu_temp_c"),
             || Sample::new("em_cpu_temp_c", ts, v as f64));
    }
    if let Some(v) = m.memory_percent {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("em_memory_percent"),
             || Sample::new("em_memory_percent", ts, v as f64));
    }
    if let Some(v) = m.mem_used_mb {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("em_mem_used_mb"),
             || Sample::new("em_mem_used_mb", ts, v as f64));
    }
    if let Some(v) = m.swap_used_mb {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("em_swap_used_mb"),
             || Sample::new("em_swap_used_mb", ts, v as f64));
    }
    if let Some(v) = m.disk_percent {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("em_disk_percent"),
             || Sample::new("em_disk_percent", ts, v as f64));
    }
    if let Some(v) = m.load_avg_1m {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("em_load_avg:1m"),
             || Sample::new("em_load_avg", ts, v as f64).with_label("window", "1m".to_string()));
    }
    if let Some(v) = m.load_avg_5m {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("em_load_avg:5m"),
             || Sample::new("em_load_avg", ts, v as f64).with_label("window", "5m".to_string()));
    }
    if let Some(v) = m.load_avg_15m {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("em_load_avg:15m"),
             || Sample::new("em_load_avg", ts, v as f64).with_label("window", "15m".to_string()));
    }
    if let Some(v) = m.net_rx_bps {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("em_net_rx_bps"),
             || Sample::new("em_net_rx_bps", ts, v as f64));
    }
    if let Some(v) = m.net_tx_bps {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("em_net_tx_bps"),
             || Sample::new("em_net_tx_bps", ts, v as f64));
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
        push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("wh_current_temp_c"),
             || Sample::new("wh_current_temp_c", ts, v as f64));
    }
    if let Some(v) = m.target_temp_c {
        push(writer, rl, TEMP_WRITE_INTERVAL, format_args!("wh_target_temp_c"),
             || Sample::new("wh_target_temp_c", ts, v as f64));
    }
    if let Some(v) = m.mode {
        push(writer, rl, MIN_WRITE_INTERVAL, format_args!("wh_mode"),
             || Sample::new("wh_mode", ts, v as f64));
    }
}

// =============================================================================
// Auto-télémétrie mémoire du process (investigation RSS §17 — axe B)
// =============================================================================

/// RSS kernel + statistiques allocateur jemalloc, collectées par l'agent
/// monitor toutes les 30 s. Tous les champs sont optionnels : sur une cible
/// sans jemalloc (msvc) ou si une lecture échoue, on n'écrit que le reste.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessMemoryStats {
    /// VmRSS du process (octets) — lu dans /proc/self/status.
    pub rss_bytes:          Option<u64>,
    /// Octets vivants alloués par le code Rust (jemalloc stats.allocated).
    /// C'est LA métrique qui discrimine : si elle croît linéairement, la
    /// fuite est applicative ; si elle reste plate alors que resident/RSS
    /// montent, c'est de la rétention allocateur.
    pub jemalloc_allocated: Option<u64>,
    /// Pages actives (stats.active).
    pub jemalloc_active:    Option<u64>,
    /// Mémoire résidente vue par jemalloc (stats.resident).
    pub jemalloc_resident:  Option<u64>,
    /// Mémoire mappée (stats.mapped).
    pub jemalloc_mapped:    Option<u64>,
    /// Mémoire rendue au kernel mais retenue en VM (stats.retained).
    pub jemalloc_retained:  Option<u64>,
}

pub fn write_process_memory(writer: &Writer, rl: &RateLimiter, m: &ProcessMemoryStats) {
    let ts = now_ms();
    let metrics: [(&str, Option<u64>); 6] = [
        ("process_rss_bytes",                m.rss_bytes),
        ("process_jemalloc_allocated_bytes", m.jemalloc_allocated),
        ("process_jemalloc_active_bytes",    m.jemalloc_active),
        ("process_jemalloc_resident_bytes",  m.jemalloc_resident),
        ("process_jemalloc_mapped_bytes",    m.jemalloc_mapped),
        ("process_jemalloc_retained_bytes",  m.jemalloc_retained),
    ];
    for (name, v) in metrics {
        if let Some(v) = v {
            push(writer, rl, MIN_WRITE_INTERVAL, format_args!("{name}"),
                 || Sample::new(name, ts, v as f64));
        }
    }
}
