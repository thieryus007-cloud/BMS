//! État partagé de l'application (AppState).
//!
//! [`AppState`] est clonable et partagé via `Arc` entre toutes les tâches Tokio
//! et les handlers Axum.
use crate::ats::AtsSnapshot;
use crate::bridges::alerts::AlertEngine;
use crate::config::AppConfig;
use crate::console::{ConsoleBus, ConsoleEvent, EventDevice};
use crate::et112::Et112Snapshot;
use crate::irradiance::IrradianceSnapshot;
use crate::shelly::ShellyEmSnapshot;
use crate::tasmota::TasmotaSnapshot;
use daly_bms_core::bus::DalyPort;
use daly_bms_core::types::BmsSnapshot;
use chrono::{DateTime, Datelike, Utc};
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, RwLock};
// =============================================================================
// Buffer de logs en mémoire (pour l'interface web)
// =============================================================================
/// Une entrée de log capturée depuis tracing.
#[derive(Clone, Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}
/// Ring buffer de logs partagé (200 entrées max).
pub type LogBuffer = Arc<Mutex<VecDeque<LogEntry>>>;
/// Capacité du canal broadcast WebSocket.
const WS_BROADCAST_CAPACITY: usize = 128;
// =============================================================================
// Ring buffer par BMS
// =============================================================================
/// Ring buffer de snapshots en mémoire pour un BMS.
pub struct BmsRingBuffer {
    pub buffer: VecDeque<BmsSnapshot>,
    pub capacity: usize,
}
impl BmsRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }
    pub fn push(&mut self, snap: BmsSnapshot) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(snap);
    }
    pub fn latest(&self) -> Option<&BmsSnapshot> {
        self.buffer.back()
    }
}
// =============================================================================
// AppState
// =============================================================================
// =============================================================================
// Ring buffer ET112
// =============================================================================
/// Ring buffer de snapshots ET112 pour un compteur.
pub struct Et112RingBuffer {
    pub buffer: VecDeque<Et112Snapshot>,
    pub capacity: usize,
}
impl Et112RingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }
    pub fn push(&mut self, snap: Et112Snapshot) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(snap);
    }
    pub fn latest(&self) -> Option<&Et112Snapshot> {
        self.buffer.back()
    }
}
// =============================================================================
// Ring buffer Tasmota
// =============================================================================
/// Ring buffer de snapshots Tasmota pour une prise.
pub struct TasmotaRingBuffer {
    pub buffer:   VecDeque<TasmotaSnapshot>,
    pub capacity: usize,
}
impl TasmotaRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer:   VecDeque::with_capacity(capacity),
            capacity,
        }
    }
    pub fn push(&mut self, snap: TasmotaSnapshot) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(snap);
    }
    pub fn latest(&self) -> Option<&TasmotaSnapshot> {
        self.buffer.back()
    }
}
// =============================================================================
// Venus OS — Structures de données D-Bus
// =============================================================================
/// Snapshot MPPT SolarCharger (depuis D-Bus Venus OS via MQTT).
#[derive(Clone, Serialize, Debug)]
pub struct VenusMppt {
    pub instance: u32,
    pub name: String,
    pub power_w: Option<f32>,
    pub yield_today_kwh: Option<f32>,
    pub max_power_today_w: Option<f32>,
    /// État MPPT : "Off", "Fault", "Bulk", "Absorption", "Float", "Storage", etc.
    pub state: Option<String>,
    /// Tension panneau solaire PV (V).
    pub pv_voltage_v: Option<f32>,
    /// Courant DC sortie chargeur (A).
    pub dc_current_a: Option<f32>,
    pub timestamp: DateTime<Utc>,
}
/// Snapshot SmartShunt (depuis D-Bus Venus OS via MQTT).
#[derive(Clone, Serialize, Debug)]
pub struct VenusSmartShunt {
    pub soc_percent: Option<f32>,
    pub voltage_v: Option<f32>,
    pub current_a: Option<f32>,
    pub power_w: Option<f32>,
    pub energy_in_kwh: Option<f32>,
    pub energy_out_kwh: Option<f32>,
    /// État batterie : "Idle", "Charging", "Discharging".
    pub state: Option<String>,
    /// Temps restant en minutes (None = inconnu ou en charge).
    pub time_to_go_min: Option<f32>,
    /// Ah chargés depuis minuit (intégration courant × temps).
    pub ah_charged_today: Option<f32>,
    /// Ah déchargés depuis minuit (intégration courant × temps).
    pub ah_discharged_today: Option<f32>,
    pub timestamp: DateTime<Utc>,
}
/// Snapshot Onduleur/Charger Victron (MultiPlus, cgwacs, etc).
#[derive(Clone, Serialize, Debug)]
pub struct VenusInverter {
    pub voltage_v: Option<f32>,
    pub current_a: Option<f32>,
    pub power_w: Option<f32>,
    pub ac_output_voltage_v: Option<f32>,
    pub ac_output_current_a: Option<f32>,
    pub ac_output_power_w: Option<f32>,
    /// Fréquence AC sortie (Hz).
    pub ac_out_frequency_hz: Option<f32>,
    /// IgnoreAcIn1 : true si l'AC input est ignoré (mode îlotage).
    pub ac_in_ignore: Option<bool>,
    pub state: String, // "off", "on", "inverting", "charger", etc.
    pub mode: String,  // "charger", "inverter", "passthrough", etc.
    pub timestamp: DateTime<Utc>,
}
/// Snapshot Capteur Température (depuis D-Bus Venus OS via MQTT).
#[derive(Clone, Serialize, Debug)]
pub struct VenusTemperature {
    pub instance: u32,
    pub name: String,
    pub temp_c: Option<f32>,
    pub humidity_percent: Option<f32>,
    pub pressure_mbar: Option<f32>,
    pub temp_type: String, // "Outdoor", "Room", "Generic", etc.
    pub connected: bool,
    pub timestamp: DateTime<Utc>,
}
/// Snapshot Pompe à chaleur / Chauffe-eau (depuis MQTT santuario/heatpump/{n}/venus).
#[derive(Clone, Serialize, Debug)]
pub struct VenusHeatpump {
    /// Index MQTT (1 = chauffe-eau LG ThinQ, 8/9 = ET112 PAC).
    pub mqtt_index: u8,
    /// État : 0=Off/Vacances, 1=Pompe chaleur, 2=Turbo.
    pub state: i32,
    /// Température eau courante en °C.
    pub temperature: Option<f32>,
    /// Température eau cible en °C.
    pub target_temperature: Option<f32>,
    /// Puissance consommée en W.
    pub ac_power: f32,
    /// Énergie totale consommée en kWh.
    pub ac_energy_forward: f32,
    /// Position : 0=AC Output, 1=AC Input.
    pub position: i32,
    pub connected: bool,
    pub timestamp: DateTime<Utc>,
}
/// Statut d'un service système.
#[derive(Clone, Serialize, Debug)]
pub struct ServiceStatus {
    pub name: String,
    /// "active", "inactive", "failed", "unknown"
    pub status: String,
    pub active: bool,
}
/// Compteurs de santé par appareil RS485 (BMS, ET112, ATS, PRALRAN).
///
/// Les compteurs sont monotones depuis le démarrage du serveur ; l'UI
/// peut calculer un taux de succès = `successful_polls / (successful_polls
/// + timeout_count + crc_error_count + other_error_count)`.
#[derive(Clone, Serialize, Debug, Default)]
pub struct Rs485DeviceStats {
    pub address: u8,
    pub name: String,
    /// Catégorie d'appareil : "BMS", "ET112", "PRALRAN", "ATS".
    pub kind: String,
    pub successful_polls:  u64,
    pub timeout_count:     u64,
    pub crc_error_count:   u64,
    pub other_error_count: u64,
    pub last_success_ts:   Option<DateTime<Utc>>,
    pub last_error_ts:     Option<DateTime<Utc>>,
    /// Catégorie de la dernière erreur : "timeout", "crc", "other".
    pub last_error_kind:    Option<String>,
    pub last_error_message: Option<String>,
}
impl Rs485DeviceStats {
    pub fn new(address: u8, name: String, kind: &str) -> Self {
        Self {
            address,
            name,
            kind: kind.to_string(),
            ..Default::default()
        }
    }
    pub fn record_success(&mut self) {
        self.successful_polls += 1;
        self.last_success_ts = Some(Utc::now());
    }
    /// Classe une erreur à partir de son message (heuristique).
    pub fn record_error(&mut self, err_msg: &str) {
        let lc = err_msg.to_lowercase();
        let kind = if lc.contains("timeout") || lc.contains("aucune réponse") {
            self.timeout_count += 1;
            "timeout"
        } else if lc.contains("crc") {
            self.crc_error_count += 1;
            "crc"
        } else {
            self.other_error_count += 1;
            "other"
        };
        self.last_error_ts = Some(Utc::now());
        self.last_error_kind = Some(kind.to_string());
        self.last_error_message = Some(err_msg.chars().take(200).collect());
    }
}
/// Informations sur un processus (pour la vue neohtop).
#[derive(Clone, Serialize, Debug, Default)]
pub struct ProcessInfo {
    pub pid:         u32,
    pub name:        String,
    pub cpu_percent: f32,
    pub mem_percent: f32,
    pub mem_rss_mb:  f32,
    pub state:       String,
}
/// Snapshot de monitoring système Pi5.
#[derive(Clone, Serialize, Debug)]
pub struct MonitorSnapshot {
    pub timestamp: DateTime<Utc>,
    /// Services systemd (daly-bms).
    pub services: Vec<ServiceStatus>,
    /// Services réseau vérifiés par sonde TCP (mosquitto, energy-manager, venus).
    pub network_services: Vec<ServiceStatus>,
    /// Port série RS485 présent sur le système.
    pub serial_port_ok: bool,
    /// Charge système [1min, 5min, 15min].
    pub load_avg: [f32; 3],
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub disk_percent: f32,
    pub uptime_secs: u64,
    /// Actions prises automatiquement (ex: "Redémarré mosquitto").
    pub auto_actions: Vec<String>,
    /// Top processus triés par CPU% décroissant.
    #[serde(default)]
    pub processes: Vec<ProcessInfo>,
    /// Mémoire totale en Mo.
    #[serde(default)]
    pub mem_total_mb: u64,
    /// Mémoire utilisée en Mo.
    #[serde(default)]
    pub mem_used_mb: u64,
    /// Swap total en Mo.
    #[serde(default)]
    pub swap_total_mb: u64,
    /// Swap utilisé en Mo.
    #[serde(default)]
    pub swap_used_mb: u64,
    /// Réseau — octets reçus/s (somme interfaces non-lo).
    #[serde(default)]
    pub net_rx_bps: u64,
    /// Réseau — octets envoyés/s (somme interfaces non-lo).
    #[serde(default)]
    pub net_tx_bps: u64,
    /// Température CPU en °C (None si non disponible).
    #[serde(default)]
    pub cpu_temp_c: Option<f32>,
}
/// État global partagé de l'application.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    /// Ring buffers indexés par adresse BMS.
    pub buffers: Arc<RwLock<BTreeMap<u8, BmsRingBuffer>>>,
    /// Canal broadcast pour le WebSocket (tous BMS confondus).
    pub ws_tx: broadcast::Sender<Arc<Vec<BmsSnapshot>>>,
    /// Indicateur polling actif.
    pub polling_active: Arc<std::sync::atomic::AtomicBool>,
    /// Port série partagé.
    /// Partagé avec le poll_loop via le Mutex interne de DalyPort.
    pub port: Arc<RwLock<Option<Arc<DalyPort>>>>,
    /// Buffer de logs pour l'interface web.
    pub log_buffer: LogBuffer,
    /// Ring buffers ET112 indexés par adresse Modbus.
    pub et112_buffers: Arc<RwLock<BTreeMap<u8, Et112RingBuffer>>>,
    /// Dernière mesure du capteur d'irradiance PRALRAN (None si non configuré).
    pub irradiance_value: Arc<RwLock<Option<IrradianceSnapshot>>>,
    /// Ring buffers Tasmota indexés par id de device.
    pub tasmota_buffers: Arc<RwLock<BTreeMap<u8, TasmotaRingBuffer>>>,
    /// Production solaire totale aujourd'hui en kWh (MPPT + delta ET112 micro-onduleurs).
    /// Publiée par energy-manager via POST /api/v1/solar/mppt-yield.
    pub mppt_yield_kwh: Arc<RwLock<f32>>,
    /// Puissance MPPT agrégée en W = N/.../system/0/Dc/Pv/Power (alias de dc_pv_power_w).
    /// Publiée par energy-manager via POST /api/v1/solar/mppt-yield.
    pub mppt_power_w: Arc<RwLock<f32>>,
    /// Puissance MPPT agrégée en W = N/.../system/0/Dc/Pv/Power (source de vérité).
    pub dc_pv_power_w: Arc<RwLock<f32>>,
    /// Puissance ET112 micro-onduleurs en W = N/.../pvinverter/32/Ac/Power.
    pub pvinv_power_w: Arc<RwLock<f32>>,
    /// Puissance solaire totale en W = dc_pv_power_w + pvinv_power_w.
    /// Publiée par energy-manager via POST /api/v1/solar/mppt-yield.
    pub solar_total_w: Arc<RwLock<f32>>,
    /// Puissance consommée par la maison en W (ESS AC output consumption).
    /// Source : N/{portalId}/system/0/Ac/ConsumptionOnOutput/L1/Power via energy-manager.
    pub house_power_w: Arc<RwLock<f32>>,
    /// Données Venus OS — MPPT SolarCharger (indexé par instance).
    pub venus_mppts: Arc<RwLock<BTreeMap<u32, VenusMppt>>>,
    /// Données Venus OS — SmartShunt.
    pub venus_smartshunt: Arc<RwLock<Option<VenusSmartShunt>>>,
    /// Accumulateurs Ah journaliers du SmartShunt (intégration courant, remise à zéro à minuit).
    pub shunt_ah_charged_today:    Arc<RwLock<f32>>,
    pub shunt_ah_discharged_today: Arc<RwLock<f32>>,
    pub shunt_ah_last_ts:          Arc<RwLock<Option<DateTime<Utc>>>>,
    /// Numéro de jour (days_from_ce) au dernier enregistrement — détecte le passage à minuit.
    pub shunt_ah_last_day:         Arc<RwLock<i32>>,
    /// Données Venus OS — Onduleur/Charger (Victron MultiPlus).
    pub venus_inverter: Arc<RwLock<Option<VenusInverter>>>,
    /// Données Venus OS — Capteurs de température (indexés par instance).
    pub venus_temperatures: Arc<RwLock<BTreeMap<u32, VenusTemperature>>>,
    /// Données Venus OS — Pompes à chaleur / chauffe-eau (indexés par mqtt_index).
    pub venus_heatpumps: Arc<RwLock<BTreeMap<u8, VenusHeatpump>>>,
    /// Dernier snapshot du monitoring système Pi5.
    pub monitor_snapshot: Arc<RwLock<Option<MonitorSnapshot>>>,
    /// Dernier snapshot ATS CHINT (None si non configuré ou en attente).
    pub ats_snapshot: Arc<RwLock<Option<AtsSnapshot>>>,
    /// Bus RS485 dédié à l'ATS (parité Even) — pour les commandes d'écriture via API.
    pub ats_bus: Arc<RwLock<Option<Arc<rs485_bus::SharedBus>>>>,
    /// Compteurs de santé par appareil RS485 (indexés par adresse).
    /// Alimenté par les boucles de polling BMS / ET112 / irradiance / ATS.
    pub rs485_stats: Arc<RwLock<BTreeMap<u8, Rs485DeviceStats>>>,
    /// Canal broadcast pour la console de diagnostic (/ws/console).
    pub console_bus: ConsoleBus,
    /// Derniers snapshots Shelly Pro 2PM (indexés par id device).
    pub shelly_latest: Arc<RwLock<BTreeMap<u8, ShellyEmSnapshot>>>,
    /// Client MQTT Shelly (pour les commandes de contrôle Switch.Set).
    pub shelly_client: Arc<tokio::sync::Mutex<Option<rumqttc::AsyncClient>>>,
    /// Moteur d'alertes (None si alerts.db_path vide dans la config).
    pub alert_engine: Option<Arc<AlertEngine>>,
    /// Backend TSDB redb (metrics-store) : SEULE source de vérité pour les
    /// lectures et écritures de séries temporelles. `None` uniquement si
    /// désactivé dans la config (mode dégradé pour debug).
    pub metrics_store: Option<Arc<metrics_store::MetricsStore>>,
    /// Rate limiter pour les écritures redb depuis les `on_*_snapshot()`.
    /// Empêche un sample d'être poussé plus d'1× / 5 s par série.
    pub redb_rl: crate::redb_writes::RateLimiter,
    /// Catalogue de panels (importés depuis docs/grafana-ess_dashboard.json au démarrage).
    pub dashboard_catalog: crate::dashboards::Catalog,
    /// Persistance SQLite des layouts dashboard (None si init a échoué — fallback localStorage côté UI).
    pub dashboard_storage: Option<crate::dashboards::storage::LayoutStorage>,
}
impl AppState {
    /// Indique si le backend de lecture (metrics-store / redb) est prêt.
    /// À utiliser par les routes pour leur pré-flight check.
    pub fn is_query_backend_ready(&self) -> bool {
        self.metrics_store.is_some()
    }
    /// Exécute une requête PromQL sur plage temporelle via le shim redb
    /// (`metrics-store::promql`). Le travail synchrone (open reader + eval
    /// PromQL + B-tree scans) est déporté sur `tokio::task::spawn_blocking`
    /// pour ne pas monopoliser un worker de l'exécuteur async —
    /// particulièrement important quand un caller comme `api/history.rs`
    /// lance 9 requêtes en parallèle via `tokio::join!`.
    ///
    /// Retourne le format JSON Prometheus standard.
    pub async fn dispatched_query_range(
        &self,
        query: &str,
        start_ms: i64,
        end_ms: i64,
        step_ms: i64,
    ) -> anyhow::Result<serde_json::Value> {
        let store = self
            .metrics_store
            .clone()
            .ok_or_else(|| anyhow::anyhow!("metrics-store backend not configured"))?;
        let query = query.to_string();
        let max_points = self.config.metrics_store.query_max_points;
        tokio::task::spawn_blocking(move || {
            redb_query_range_inner(&store, &query, start_ms, end_ms, step_ms, max_points)
        })
        .await
        .map_err(|e| anyhow::anyhow!("redb worker panic: {e}"))?
    }
    /// Variante instant — symétrique de [`dispatched_query_range`].
    #[allow(dead_code)]
    pub async fn dispatched_query_instant(
        &self,
        query: &str,
        time_ms: i64,
    ) -> anyhow::Result<serde_json::Value> {
        let store = self
            .metrics_store
            .clone()
            .ok_or_else(|| anyhow::anyhow!("metrics-store backend not configured"))?;
        let query = query.to_string();
        tokio::task::spawn_blocking(move || {
            redb_query_instant_inner(&store, &query, time_ms)
        })
        .await
        .map_err(|e| anyhow::anyhow!("redb worker panic: {e}"))?
    }
}
// Fonctions sync exécutées sous `spawn_blocking` (pool de threads dédié) :
// la review #1 de gemini-code-assist demande à ne pas bloquer l'executor
// async avec les ops redb memory-mapped + eval PromQL potentiellement
// coûteuses. Cf. PR #456 / commit `0839b84` revue.
fn redb_query_range_inner(
    store: &metrics_store::MetricsStore,
    query: &str,
    start_ms: i64,
    end_ms: i64,
    step_ms: i64,
    max_points: i64,
) -> anyhow::Result<serde_json::Value> {
    use metrics_store::promql::{parse_and_validate, Evaluator};
    let expr = parse_and_validate(query)
        .map_err(|e| anyhow::anyhow!("PromQL: {}", e.message()))?;
    let reader = store.reader();
    let mut ev = Evaluator::new(&reader);
    ev.max_range_points = max_points;
    let series = ev
        .eval_range(&expr, start_ms, end_ms, step_ms)
        .map_err(|e| anyhow::anyhow!("PromQL exec: {}", e.message()))?;
    let result: Vec<serde_json::Value> = series
        .into_iter()
        .map(|s| {
            let values: Vec<serde_json::Value> = s
                .samples
                .into_iter()
                .map(|(ts, v)| serde_json::json!([ts as f64 / 1000.0, fmt_val(v)]))
                .collect();
            serde_json::json!({ "metric": s.labels, "values": values })
        })
        .collect();
    Ok(serde_json::json!({
        "status": "success",
        "data": { "resultType": "matrix", "result": result },
    }))
}
#[allow(dead_code)]
fn redb_query_instant_inner(
    store: &metrics_store::MetricsStore,
    query: &str,
    time_ms: i64,
) -> anyhow::Result<serde_json::Value> {
    use metrics_store::promql::{parse_and_validate, Evaluator};
    let expr = parse_and_validate(query)
        .map_err(|e| anyhow::anyhow!("PromQL: {}", e.message()))?;
    let reader = store.reader();
    let ev = Evaluator::new(&reader);
    let samples = ev
        .eval_instant(&expr, time_ms)
        .map_err(|e| anyhow::anyhow!("PromQL exec: {}", e.message()))?;
    let result: Vec<serde_json::Value> = samples
        .into_iter()
        .map(|s| {
            serde_json::json!({
                "metric": &*s.labels,
                "value": [time_ms as f64 / 1000.0, fmt_val(s.value)],
            })
        })
        .collect();
    Ok(serde_json::json!({
        "status": "success",
        "data": { "resultType": "vector", "result": result },
    }))
}
/// Formatage cohérent avec la convention Prometheus (review gemini #4).
/// `f64::to_string()` natif Rust produit des chaînes du type
/// `53.900001525878906` (artéfact de la conversion f32→f64). On cap à 6
/// décimales et on trim les zéros terminaux — bon compromis entre
/// précision et lisibilité.
fn fmt_val(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "+Inf".into() } else { "-Inf".into() };
    }
    let s = format!("{v:.6}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".into()
    } else {
        trimmed.to_string()
    }
}
impl AppState {
    pub fn new(
        config: AppConfig,
        log_buffer: LogBuffer,
        alert_engine: Option<Arc<AlertEngine>>,
        metrics_store: Option<Arc<metrics_store::MetricsStore>>,
    ) -> Self {
        let (ws_tx, _) = broadcast::channel(WS_BROADCAST_CAPACITY);
        let addresses = config.bms_addresses();
        let ring_size = config.serial.ring_buffer_size;
        let mut buffers = BTreeMap::new();
        for addr in &addresses {
            buffers.insert(*addr, BmsRingBuffer::new(ring_size));
        }
        // Pré-allouer les ring buffers ET112
        let et112_ring_size = config.et112.ring_buffer_size;
        let mut et112_buffers = BTreeMap::new();
        for dev in &config.et112.devices {
            et112_buffers.insert(dev.parsed_address(), Et112RingBuffer::new(et112_ring_size));
        }
        // Pré-allouer les ring buffers Tasmota
        let tasmota_ring_size = config.tasmota.ring_buffer_size;
        let mut tasmota_buffers = BTreeMap::new();
        for dev in &config.tasmota.devices {
            tasmota_buffers.insert(dev.id, TasmotaRingBuffer::new(tasmota_ring_size));
        }
        // Init dashboard layout storage (SQLite séparée du fichier d'alertes)
        let dashboard_storage = {
            let path = crate::dashboards::storage::LayoutStorage::default_path(&config.alerts.db_path);
            match crate::dashboards::storage::LayoutStorage::open(&path) {
                Ok(s)  => { tracing::info!(db = %path.display(), "Dashboard layout storage ouvert"); Some(s) }
                Err(e) => { tracing::error!(error = %e, db = %path.display(), "Dashboard storage init failed — fallback localStorage côté UI"); None }
            }
        };
        Self {
            config: Arc::new(config),
            buffers: Arc::new(RwLock::new(buffers)),
            ws_tx,
            polling_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            port: Arc::new(RwLock::new(None)),
            log_buffer,
            et112_buffers: Arc::new(RwLock::new(et112_buffers)),
            irradiance_value: Arc::new(RwLock::new(None)),
            tasmota_buffers: Arc::new(RwLock::new(tasmota_buffers)),
            mppt_yield_kwh: Arc::new(RwLock::new(0.0)),
            mppt_power_w:   Arc::new(RwLock::new(0.0)),
            dc_pv_power_w:  Arc::new(RwLock::new(0.0)),
            pvinv_power_w:  Arc::new(RwLock::new(0.0)),
            solar_total_w:  Arc::new(RwLock::new(0.0)),
            house_power_w:  Arc::new(RwLock::new(0.0)),
            venus_mppts: Arc::new(RwLock::new(BTreeMap::new())),
            venus_smartshunt: Arc::new(RwLock::new(None)),
            shunt_ah_charged_today:    Arc::new(RwLock::new(0.0)),
            shunt_ah_discharged_today: Arc::new(RwLock::new(0.0)),
            shunt_ah_last_ts:          Arc::new(RwLock::new(None)),
            shunt_ah_last_day:         Arc::new(RwLock::new(0)),
            venus_inverter: Arc::new(RwLock::new(None)),
            venus_temperatures: Arc::new(RwLock::new(BTreeMap::new())),
            venus_heatpumps: Arc::new(RwLock::new(BTreeMap::new())),
            monitor_snapshot: Arc::new(RwLock::new(None)),
            ats_snapshot: Arc::new(RwLock::new(None)),
            ats_bus: Arc::new(RwLock::new(None)),
            rs485_stats: Arc::new(RwLock::new(BTreeMap::new())),
            console_bus: ConsoleBus::new(),
            shelly_latest: Arc::new(RwLock::new(BTreeMap::new())),
            shelly_client: Arc::new(tokio::sync::Mutex::new(None)),
            alert_engine,
            metrics_store,
            redb_rl: crate::redb_writes::RateLimiter::new(),
            dashboard_catalog: crate::dashboards::Catalog::load_default(),
            dashboard_storage,
        }
    }
    /// Incrémente le compteur de polls réussis pour un appareil RS485.
    /// Crée l'entrée si elle n'existe pas.
    pub async fn record_rs485_success(&self, addr: u8, kind: &str, name: &str) {
        let mut stats = self.rs485_stats.write().await;
        stats
            .entry(addr)
            .or_insert_with(|| Rs485DeviceStats::new(addr, name.to_string(), kind))
            .record_success();
    }
    /// Incrémente le compteur d'erreurs pour un appareil RS485 (timeout/CRC/autre).
    /// Crée l'entrée si elle n'existe pas.
    pub async fn record_rs485_error(&self, addr: u8, kind: &str, name: &str, err_msg: &str) {
        let mut stats = self.rs485_stats.write().await;
        stats
            .entry(addr)
            .or_insert_with(|| Rs485DeviceStats::new(addr, name.to_string(), kind))
            .record_error(err_msg);
    }
    /// Retourne la liste actuelle des statistiques RS485 (triée par adresse).
    pub async fn rs485_stats_all(&self) -> Vec<Rs485DeviceStats> {
        let stats = self.rs485_stats.read().await;
        stats.values().cloned().collect()
    }
    /// Enregistre le port série ouvert (mode hardware uniquement).
    pub async fn set_port(&self, port: Arc<DalyPort>) {
        *self.port.write().await = Some(port);
    }
    /// Enregistre un nouveau snapshot dans le ring buffer et broadcast WebSocket.
    pub async fn on_snapshot(&self, snap: BmsSnapshot) {
        // Console diagnostic event — seulement si quelqu'un écoute (évite de retenir
        // un Arc<ConsoleEvent> volumineux dans le ring buffer du broadcast).
        if self.console_bus.receiver_count() > 0 {
            let device = match snap.address {
                1 => EventDevice::Bms1,
                2 => EventDevice::Bms2,
                3 => EventDevice::Bms3,
                _ => EventDevice::Bms2,
            };
            self.console_bus.emit(ConsoleEvent::rs485(device, &format!("BMS-{} snapshot", snap.address), json!({
                "address": snap.address,
                "soc": snap.soc,
                "voltage": snap.dc.voltage,
                "current": snap.dc.current,
                "power": snap.dc.power,
                "temp_max": snap.system.max_cell_temperature,
                "cell_delta_mv": snap.system.cell_delta_mv(),
                "charge_ok": snap.io.allow_to_charge != 0,
                "discharge_ok": snap.io.allow_to_discharge != 0,
            })));
        }
        {
            let mut buffers = self.buffers.write().await;
            buffers
                .entry(snap.address)
                .or_insert_with(|| BmsRingBuffer::new(self.config.serial.ring_buffer_size))
                .push(snap.clone());
        }
        // Broadcast WebSocket : skip si aucun client connecté (évite d'allouer
        // et retenir Arc<Vec<BmsSnapshot>> dans le ring du broadcast).
        if self.ws_tx.receiver_count() > 0 {
            let latest = self.latest_snapshots().await;
            let _ = self.ws_tx.send(Arc::new(latest));
        }
        // Écriture redb (rate-limitée 1/5s par série).
        if let Some(store) = &self.metrics_store {
            crate::redb_writes::write_bms(&store.writer(), &self.redb_rl, &snap);
        }
    }
    /// Retourne le dernier snapshot de chaque BMS.
    pub async fn latest_snapshots(&self) -> Vec<BmsSnapshot> {
        let buffers = self.buffers.read().await;
        buffers.values()
            .filter_map(|b| b.latest().cloned())
            .collect()
    }
    /// Retourne le dernier snapshot d'un BMS spécifique.
    pub async fn latest_for(&self, addr: u8) -> Option<BmsSnapshot> {
        let buffers = self.buffers.read().await;
        buffers.get(&addr)?.latest().cloned()
    }
    /// Retourne les `n` derniers snapshots d'un BMS (pour historique).
    pub async fn history_for(&self, addr: u8, limit: usize) -> Vec<BmsSnapshot> {
        let buffers = self.buffers.read().await;
        if let Some(buf) = buffers.get(&addr) {
            buf.buffer.iter().rev().take(limit).cloned().collect()
        } else {
            vec![]
        }
    }
    /// S'abonne au canal broadcast WebSocket.
    pub fn subscribe_ws(&self) -> broadcast::Receiver<Arc<Vec<BmsSnapshot>>> {
        self.ws_tx.subscribe()
    }
    /// Enregistre un snapshot ET112 dans le ring buffer correspondant.
    pub async fn on_et112_snapshot(&self, snap: Et112Snapshot) {
        if self.console_bus.receiver_count() > 0 {
            self.console_bus.emit(ConsoleEvent::rs485(EventDevice::Et112, &format!("ET112-{:#04x} snapshot", snap.address), json!({
                "address": snap.address,
                "power_w": snap.power_w,
                "voltage_v": snap.voltage_v,
                "current_a": snap.current_a,
                "energy_import_kwh": snap.energy_import_kwh(),
                "energy_export_kwh": snap.energy_export_kwh(),
            })));
        }
        {
            let mut buffers = self.et112_buffers.write().await;
            buffers
                .entry(snap.address)
                .or_insert_with(|| Et112RingBuffer::new(self.config.et112.ring_buffer_size))
                .push(snap.clone());
        }
        if let Some(store) = &self.metrics_store {
            crate::redb_writes::write_et112(&store.writer(), &self.redb_rl, &snap);
        }
    }
    /// Retourne le dernier snapshot ET112 pour une adresse donnée.
    pub async fn et112_latest_for(&self, addr: u8) -> Option<Et112Snapshot> {
        let buffers = self.et112_buffers.read().await;
        buffers.get(&addr)?.latest().cloned()
    }
    /// Retourne les `n` derniers snapshots ET112 (pour historique).
    pub async fn et112_history_for(&self, addr: u8, limit: usize) -> Vec<Et112Snapshot> {
        let buffers = self.et112_buffers.read().await;
        if let Some(buf) = buffers.get(&addr) {
            buf.buffer.iter().rev().take(limit).cloned().collect()
        } else {
            vec![]
        }
    }
    /// Retourne tous les derniers snapshots ET112.
    pub async fn et112_latest_all(&self) -> Vec<Et112Snapshot> {
        let buffers = self.et112_buffers.read().await;
        buffers.values().filter_map(|b| b.latest().cloned()).collect()
    }
    /// Enregistre la dernière mesure du capteur d'irradiance.
    pub async fn on_irradiance_snapshot(&self, snap: IrradianceSnapshot) {
        if self.console_bus.receiver_count() > 0 {
            self.console_bus.emit(ConsoleEvent::rs485(EventDevice::Irradiance, &format!("PRALRAN irradiance — {} W/m²", snap.irradiance_wm2 as i32), json!({
                "address": snap.address,
                "irradiance_wm2": snap.irradiance_wm2,
            })));
        }
        if let Some(store) = &self.metrics_store {
            crate::redb_writes::write_irradiance(&store.writer(), &self.redb_rl, &snap);
        }
        *self.irradiance_value.write().await = Some(snap);
    }
    /// Retourne la dernière mesure d'irradiance (None si jamais reçue).
    pub async fn latest_irradiance(&self) -> Option<IrradianceSnapshot> {
        self.irradiance_value.read().await.clone()
    }
    /// Enregistre un snapshot Tasmota dans le ring buffer correspondant.
    pub async fn on_tasmota_snapshot(&self, snap: TasmotaSnapshot) {
        if self.console_bus.receiver_count() > 0 {
            self.console_bus.emit(ConsoleEvent::state(EventDevice::Tasmota, &format!("Tasmota {} — {}", snap.name, if snap.power_on { "ON" } else { "OFF" }), json!({
                "id": snap.id,
                "name": snap.name,
                "power_on": snap.power_on,
                "power_w": snap.power_w,
                "voltage_v": snap.voltage_v,
                "current_a": snap.current_a,
                "energy_today_kwh": snap.energy_today_kwh,
            })));
        }
        if let Some(store) = &self.metrics_store {
            crate::redb_writes::write_tasmota(&store.writer(), &self.redb_rl, &snap);
        }
        let mut buffers = self.tasmota_buffers.write().await;
        buffers
            .entry(snap.id)
            .or_insert_with(|| TasmotaRingBuffer::new(self.config.tasmota.ring_buffer_size))
            .push(snap);
    }
    /// Retourne le dernier snapshot Tasmota pour un id donné.
    pub async fn tasmota_latest_for(&self, id: u8) -> Option<TasmotaSnapshot> {
        let buffers = self.tasmota_buffers.read().await;
        buffers.get(&id)?.latest().cloned()
    }
    /// Retourne les `n` derniers snapshots Tasmota (pour historique).
    pub async fn tasmota_history_for(&self, id: u8, limit: usize) -> Vec<TasmotaSnapshot> {
        let buffers = self.tasmota_buffers.read().await;
        if let Some(buf) = buffers.get(&id) {
            buf.buffer.iter().rev().take(limit).cloned().collect()
        } else {
            vec![]
        }
    }
    /// Retourne tous les derniers snapshots Tasmota.
    pub async fn tasmota_latest_all(&self) -> Vec<TasmotaSnapshot> {
        let buffers = self.tasmota_buffers.read().await;
        buffers.values().filter_map(|b| b.latest().cloned()).collect()
    }
    // ==========================================================================
    // Méthodes Venus OS
    // ==========================================================================
    /// Enregistre/met à jour un snapshot MPPT unique (format v1 legacy).
    pub async fn on_venus_mppt(&self, mppt: VenusMppt) {
        if let Some(store) = &self.metrics_store {
            crate::redb_writes::write_venus_mppt(&store.writer(), &self.redb_rl, &mppt);
        }
        let mut mppts = self.venus_mppts.write().await;
        mppts.insert(mppt.instance, mppt);
    }
    /// Remplace atomiquement toute la liste MPPT (format v2 — tableau complet).
    ///
    /// Utilisé quand Venus OS publie un snapshot complet de tous les chargeurs.
    /// Les entrées orphelines (MPPT déconnecté) sont ainsi purgées automatiquement.
    pub async fn on_venus_mppts_replace(&self, mppts: Vec<VenusMppt>) {
        if let Some(store) = &self.metrics_store {
            let w = store.writer();
            for m in &mppts {
                crate::redb_writes::write_venus_mppt(&w, &self.redb_rl, m);
            }
        }
        let mut map = self.venus_mppts.write().await;
        map.clear();
        for mppt in mppts {
            map.insert(mppt.instance, mppt);
        }
    }
    /// Retourne tous les MPPT actuels.
    pub async fn venus_mppts_all(&self) -> Vec<VenusMppt> {
        let mppts = self.venus_mppts.read().await;
        mppts.values().cloned().collect()
    }
    /// Retourne la puissance MPPT totale en W.
    pub async fn venus_mppt_total_power(&self) -> f32 {
        let mppts = self.venus_mppts.read().await;
        mppts.values().filter_map(|m| m.power_w).sum()
    }
    /// Retourne le courant DC total MPPT en A (somme de tous les chargeurs).
    pub async fn venus_mppt_total_dc_current(&self) -> f32 {
        let mppts = self.venus_mppts.read().await;
        mppts.values().filter_map(|m| m.dc_current_a).sum()
    }
    /// Enregistre/met à jour le SmartShunt.
    ///
    /// Si energy-manager fournit les Ah (AhChargedToday/AhDischargedToday) dans le payload,
    /// on les utilise directement. Sinon on intègre le courant localement (fallback).
    pub async fn on_venus_smartshunt(&self, mut shunt: VenusSmartShunt) {
        let now     = shunt.timestamp;
        let day_key = now.date_naive().num_days_from_ce();
        // energy-manager calcule déjà les Ah → utiliser ces valeurs directement.
        if shunt.ah_charged_today.is_some() || shunt.ah_discharged_today.is_some() {
            let mut charged    = self.shunt_ah_charged_today.write().await;
            let mut discharged = self.shunt_ah_discharged_today.write().await;
            let mut last_ts    = self.shunt_ah_last_ts.write().await;
            let mut last_day   = self.shunt_ah_last_day.write().await;
            *last_day = day_key;
            *last_ts  = Some(now);
            if let Some(v) = shunt.ah_charged_today    { *charged    = v; }
            if let Some(v) = shunt.ah_discharged_today { *discharged = v; }
            if self.console_bus.receiver_count() > 0 {
                self.console_bus.emit(ConsoleEvent::state(EventDevice::SmartShunt, "SmartShunt update", json!({
                    "soc_pct": shunt.soc_percent,
                    "voltage_v": shunt.voltage_v,
                    "current_a": shunt.current_a,
                    "power_w": shunt.power_w,
                    "state": shunt.state,
                    "ah_charged_today": shunt.ah_charged_today,
                    "ah_discharged_today": shunt.ah_discharged_today,
                })));
            }
            if let Some(store) = &self.metrics_store {
                crate::redb_writes::write_venus_smartshunt(&store.writer(), &self.redb_rl, &shunt);
            }
            *self.venus_smartshunt.write().await = Some(shunt);
            return;
        }
        // Fallback : intégration locale Ah = I × Δt (en heures).
        let mut charged    = self.shunt_ah_charged_today.write().await;
        let mut discharged = self.shunt_ah_discharged_today.write().await;
        let mut last_ts    = self.shunt_ah_last_ts.write().await;
        let mut last_day   = self.shunt_ah_last_day.write().await;
        if *last_day != day_key {
            *charged    = 0.0;
            *discharged = 0.0;
            *last_day   = day_key;
        }
        if let (Some(prev_ts), Some(current_a)) = (*last_ts, shunt.current_a) {
            let delta_ms = (now - prev_ts).num_milliseconds();
            if delta_ms > 0 && delta_ms < 600_000 {
                let delta_h = delta_ms as f32 / 3_600_000.0;
                if current_a > 0.0 {
                    *charged    += current_a * delta_h;
                } else if current_a < 0.0 {
                    *discharged += (-current_a) * delta_h;
                }
            }
        }
        *last_ts = Some(now);
        shunt.ah_charged_today    = Some(*charged);
        shunt.ah_discharged_today = Some(*discharged);
        // Console event (after values are updated)
        if self.console_bus.receiver_count() > 0 {
            self.console_bus.emit(ConsoleEvent::state(EventDevice::SmartShunt, "SmartShunt update", json!({
                "soc_pct": shunt.soc_percent,
                "voltage_v": shunt.voltage_v,
                "current_a": shunt.current_a,
                "power_w": shunt.power_w,
                "state": shunt.state,
                "ah_charged_today": shunt.ah_charged_today,
                "ah_discharged_today": shunt.ah_discharged_today,
            })));
        }
        if let Some(store) = &self.metrics_store {
            crate::redb_writes::write_venus_smartshunt(&store.writer(), &self.redb_rl, &shunt);
        }
        *self.venus_smartshunt.write().await = Some(shunt);
    }
    /// Retourne le SmartShunt actuel.
    pub async fn venus_smartshunt_get(&self) -> Option<VenusSmartShunt> {
        self.venus_smartshunt.read().await.clone()
    }
    /// Enregistre/met à jour un capteur de température.
    pub async fn on_venus_temperature(&self, temp: VenusTemperature) {
        if let Some(store) = &self.metrics_store {
            crate::redb_writes::write_venus_temperature(&store.writer(), &self.redb_rl, &temp);
        }
        let mut temps = self.venus_temperatures.write().await;
        temps.insert(temp.instance, temp);
    }
    /// Retourne tous les capteurs de température actuels.
    pub async fn venus_temperatures_all(&self) -> Vec<VenusTemperature> {
        let temps = self.venus_temperatures.read().await;
        temps.values().cloned().collect()
    }
    /// Enregistre/met à jour les données de l'onduleur Victron (MultiPlus, cgwacs, etc.).
    pub async fn on_venus_inverter(&self, inverter: VenusInverter) {
        if let Some(store) = &self.metrics_store {
            crate::redb_writes::write_venus_inverter(&store.writer(), &self.redb_rl, &inverter);
        }
        *self.venus_inverter.write().await = Some(inverter);
    }
    /// Retourne les données actuelles de l'onduleur Victron.
    pub async fn venus_inverter_get(&self) -> Option<VenusInverter> {
        self.venus_inverter.read().await.clone()
    }
    // ==========================================================================
    // Méthodes Heatpump
    // ==========================================================================
    /// Enregistre/met à jour un snapshot heatpump.
    pub async fn on_venus_heatpump(&self, hp: VenusHeatpump) {
        if let Some(store) = &self.metrics_store {
            crate::redb_writes::write_venus_heatpump(&store.writer(), &self.redb_rl, &hp);
        }
        let mut hps = self.venus_heatpumps.write().await;
        hps.insert(hp.mqtt_index, hp);
    }
    /// Retourne tous les heatpumps actuels.
    pub async fn venus_heatpumps_all(&self) -> Vec<VenusHeatpump> {
        let hps = self.venus_heatpumps.read().await;
        hps.values().cloned().collect()
    }
    /// Retourne un heatpump par index MQTT.
    #[allow(dead_code)]
    pub async fn venus_heatpump_get(&self, idx: u8) -> Option<VenusHeatpump> {
        let hps = self.venus_heatpumps.read().await;
        hps.get(&idx).cloned()
    }
    // ==========================================================================
    // Méthodes Monitor
    // ==========================================================================
    /// Enregistre le dernier snapshot de monitoring système.
    pub async fn on_monitor_snapshot(&self, snap: MonitorSnapshot) {
        if let Some(store) = &self.metrics_store {
            crate::redb_writes::write_monitor(&store.writer(), &self.redb_rl, &snap);
        }
        *self.monitor_snapshot.write().await = Some(snap);
    }
    /// Retourne le dernier snapshot de monitoring système.
    pub async fn monitor_latest(&self) -> Option<MonitorSnapshot> {
        self.monitor_snapshot.read().await.clone()
    }
    // ==========================================================================
    // Méthodes ATS CHINT
    // ==========================================================================
    /// Enregistre le dernier snapshot ATS.
    pub async fn on_ats_snapshot(&self, snap: AtsSnapshot) {
        if self.console_bus.receiver_count() > 0 {
            self.console_bus.emit(ConsoleEvent::rs485(EventDevice::Ats, &format!("ATS CHINT — {}", snap.active_source.label()), json!({
                "source": snap.active_source.label(),
                "v1a": snap.v1a, "v1b": snap.v1b, "v1c": snap.v1c,
                "v2a": snap.v2a, "v2b": snap.v2b, "v2c": snap.v2c,
                "freq1_hz": snap.freq1_hz, "freq2_hz": snap.freq2_hz,
                "sw1_closed": snap.sw1_closed, "sw2_closed": snap.sw2_closed,
                "fault": snap.fault.label(),
                "sw_mode": if snap.sw_mode { "Auto" } else { "Manuel" },
            })));
        }
        if let Some(store) = &self.metrics_store {
            crate::redb_writes::write_ats(&store.writer(), &self.redb_rl, &snap);
        }
        *self.ats_snapshot.write().await = Some(snap);
    }
    /// Retourne le dernier snapshot ATS (None si jamais reçu).
    pub async fn ats_latest(&self) -> Option<AtsSnapshot> {
        self.ats_snapshot.read().await.clone()
    }
    /// Enregistre le bus ATS pour les commandes d'écriture.
    pub async fn set_ats_bus(&self, bus: Arc<rs485_bus::SharedBus>) {
        *self.ats_bus.write().await = Some(bus);
    }
    // ==========================================================================
    // Méthodes Shelly Pro 2PM
    // ==========================================================================
    /// Enregistre le dernier snapshot Shelly et émet un événement console.
    pub async fn on_shelly_snapshot(&self, snap: ShellyEmSnapshot) {
        if self.console_bus.receiver_count() > 0 {
            self.console_bus.emit(ConsoleEvent::state(
                EventDevice::Shelly,
                &format!("Shelly {} — {:.0}W total", snap.name, snap.total_power_w),
                json!({
                    "id": snap.id,
                    "name": snap.name,
                    "shelly_id": snap.shelly_id,
                    "total_power_w": snap.total_power_w,
                    "ch0": {
                        "output": snap.channel_0.output,
                        "power_w": snap.channel_0.power_w,
                        "voltage_v": snap.channel_0.voltage_v,
                        "current_a": snap.channel_0.current_a,
                    },
                    "ch1": {
                        "output": snap.channel_1.output,
                        "power_w": snap.channel_1.power_w,
                        "voltage_v": snap.channel_1.voltage_v,
                        "current_a": snap.channel_1.current_a,
                    },
                }),
            ));
        }
        if let Some(store) = &self.metrics_store {
            crate::redb_writes::write_shelly(&store.writer(), &self.redb_rl, &snap);
        }
        let mut map = self.shelly_latest.write().await;
        map.insert(snap.id, snap);
    }
    /// Retourne le dernier snapshot Shelly pour un id donné.
    pub async fn shelly_latest_for(&self, id: u8) -> Option<ShellyEmSnapshot> {
        self.shelly_latest.read().await.get(&id).cloned()
    }
    /// Retourne tous les derniers snapshots Shelly.
    #[allow(dead_code)]
    pub async fn shelly_latest_all(&self) -> Vec<ShellyEmSnapshot> {
        self.shelly_latest.read().await.values().cloned().collect()
    }
}
#[cfg(test)]
mod fmt_val_tests {
    use super::fmt_val;
    #[test]
    fn formats_typical_values() {
        assert_eq!(fmt_val(53.9), "53.9");
        // Cas où f32→f64 ajoute du bruit décimal (réel observé en prod)
        assert_eq!(fmt_val(53.900001525878906), "53.900002");
        assert_eq!(fmt_val(0.0), "0");
        assert_eq!(fmt_val(-12.34), "-12.34");
        assert_eq!(fmt_val(100.0), "100");
    }
    #[test]
    fn formats_special_values() {
        assert_eq!(fmt_val(f64::NAN), "NaN");
        assert_eq!(fmt_val(f64::INFINITY), "+Inf");
        assert_eq!(fmt_val(f64::NEG_INFINITY), "-Inf");
    }
}
