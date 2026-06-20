use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

// ---------------------------------------------------------------------------
// Top-level config — read from the same Config.toml as daly-bms-server
// (section [energy_manager]) or from ENERGY_CONFIG env var.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct EnergyConfig {
    pub energy_manager: EnergyManagerConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EnergyManagerConfig {
    #[serde(default)]
    pub mqtt: MqttConfig,
    #[serde(default)]
    pub api: ApiConfig,
    pub victron: VictronConfig,
    #[serde(default)]
    pub open_meteo: OpenMeteoConfig,
    #[serde(default)]
    pub lg_thinq: LgThinqConfig,
    #[serde(default)]
    pub charge_current: ChargeCurrent,
    #[serde(default)]
    pub deye: DeyeConfig,
    #[serde(default)]
    pub water_heater: WaterHeaterConfig,
    #[serde(default)]
    pub solar: SolarConfig,
    #[serde(default)]
    pub platform: PlatformConfig,
    #[serde(default)]
    pub rules: RulesConfig,
}

// ---------------------------------------------------------------------------
// Rules hot-reload
// ---------------------------------------------------------------------------

/// Optional directory containing .grl rule files to load at startup instead of
/// the embedded versions. Hot-reload is triggered via POST /api/v1/em/rules/reload.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RulesConfig {
    /// Path to directory containing .grl files (e.g. "/etc/daly-bms/rules").
    /// If absent or files not found, embedded rules are used as fallback.
    pub dir: Option<String>,
}

// ---------------------------------------------------------------------------
// MQTT
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct MqttConfig {
    #[serde(default = "default_mqtt_host")]
    pub host: String,
    #[serde(default = "default_mqtt_port")]
    pub port: u16,
    #[serde(default = "default_client_id")]
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(default = "default_keep_alive_secs")]
    pub keep_alive_secs: u64,
    #[serde(default = "default_reconnect_delay_secs")]
    pub reconnect_delay_secs: u64,
}

fn default_mqtt_host() -> String { "192.168.1.141".into() }
fn default_mqtt_port() -> u16 { 1883 }
fn default_client_id() -> String { format!("energy-manager-{}", uuid::Uuid::new_v4()) }
fn default_keep_alive_secs() -> u64 { 60 }
fn default_reconnect_delay_secs() -> u64 { 5 }

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            host: default_mqtt_host(),
            port: default_mqtt_port(),
            client_id: default_client_id(),
            username: None,
            password: None,
            keep_alive_secs: default_keep_alive_secs(),
            reconnect_delay_secs: default_reconnect_delay_secs(),
        }
    }
}

// ---------------------------------------------------------------------------
// API / WebSocket server
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ApiConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
}

fn default_bind() -> String { "0.0.0.0:8081".into() }

impl Default for ApiConfig {
    fn default() -> Self {
        Self { bind: default_bind() }
    }
}

// ---------------------------------------------------------------------------
// Victron / Venus OS identifiers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct VictronConfig {
    /// Victron GX portal ID (e.g. "c0619ab9929a")
    pub portal_id: String,
    /// VEBus device instance (e.g. 275)
    #[serde(default = "default_vebus_instance")]
    pub vebus_instance: u32,
    /// MPPT 1 device instance (e.g. 273)
    #[serde(default = "default_mppt1_instance")]
    pub mppt1_instance: u32,
    /// MPPT 2 device instance (e.g. 289)
    #[serde(default = "default_mppt2_instance")]
    pub mppt2_instance: u32,
    /// PVInverter device instance (e.g. 32)
    #[serde(default = "default_pvinv_instance")]
    pub pvinverter_instance: u32,
    /// Shelly device ID for DEYE relay (e.g. "shellypro2pm-ec62608840a4")
    #[serde(default)]
    pub shelly_deye_id: String,
    /// Shelly switch channel for DEYE (0-indexed) — legacy single-channel fallback.
    #[serde(default)]
    pub shelly_deye_channel: u8,
    /// Shelly switch channels for the DEYE relays (0-indexed, one per DEYE).
    /// When non-empty this takes precedence over `shelly_deye_channel`.
    /// Example: `[0, 1]` to drive both channels of a Shelly Pro 2PM.
    #[serde(default)]
    pub shelly_deye_channels: Vec<u8>,
    /// Tasmota device ID for water heater relay (e.g. "tongou_3BC764")
    #[serde(default)]
    pub tasmota_waterheater_id: String,
    /// SmartShunt device instance on Venus OS (VRM ID 274)
    #[serde(default = "default_smartshunt_instance")]
    pub smartshunt_instance: u32,
}

fn default_vebus_instance() -> u32 { 275 }
fn default_mppt1_instance() -> u32 { 273 }
fn default_mppt2_instance() -> u32 { 289 }
fn default_pvinv_instance() -> u32 { 32 }
fn default_smartshunt_instance() -> u32 { 274 }

// ---------------------------------------------------------------------------
// Open-Meteo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct OpenMeteoConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_latitude")]
    pub latitude: f64,
    #[serde(default = "default_longitude")]
    pub longitude: f64,
    #[serde(default = "default_meteo_interval_secs")]
    pub poll_interval_secs: u64,
}

fn default_latitude() -> f64 { 43.9025 }
fn default_longitude() -> f64 { 7.8364 }
fn default_meteo_interval_secs() -> u64 { 300 }

impl Default for OpenMeteoConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            latitude: default_latitude(),
            longitude: default_longitude(),
            poll_interval_secs: default_meteo_interval_secs(),
        }
    }
}

// ---------------------------------------------------------------------------
// LG ThinQ
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct LgThinqConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Base URL for the API (e.g. "https://api-eic.lgthinq.com")
    #[serde(default = "default_lg_base_url")]
    pub base_url: String,
    /// Device ID (read from env LG_DEVICE_ID or Config.toml)
    #[serde(default)]
    pub device_id: String,
    /// Bearer token (read from env LG_BEARER_TOKEN or Config.toml)
    #[serde(default)]
    pub bearer_token: String,
    /// API key (read from env LG_API_KEY or Config.toml)
    #[serde(default)]
    pub api_key: String,
    /// x-country header (e.g. "FR")
    #[serde(default = "default_lg_country")]
    pub country: String,
    /// x-client-id header
    #[serde(default = "default_lg_client_id")]
    pub client_id: String,
    #[serde(default = "default_lg_poll_secs")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_lg_vm_url")]
    pub vm_url: String,
}

fn default_lg_base_url()  -> String { "https://api-eic.lgthinq.com".into() }
fn default_lg_country()   -> String { "FR".into() }
fn default_lg_client_id() -> String { "energy-manager".into() }
fn default_lg_poll_secs() -> u64 { 600 }
fn default_lg_vm_url()    -> String { "http://127.0.0.1:8080".into() }

impl Default for LgThinqConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: default_lg_base_url(),
            device_id: String::new(),
            bearer_token: String::new(),
            api_key: String::new(),
            country: default_lg_country(),
            client_id: default_lg_client_id(),
            poll_interval_secs: default_lg_poll_secs(),
            vm_url: default_lg_vm_url(),
        }
    }
}

// ---------------------------------------------------------------------------
// Charge current logic
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ChargeCurrent {
    /// Max charge current when off-grid (A)
    #[serde(default = "default_offgrid_charge_a")]
    pub offgrid_max_a: f64,
    /// Charge current when grid is connected and PV excess (A)
    #[serde(default = "default_pgrid_pv_a")]
    pub grid_pv_excess_a: f64,
    /// Charge current when grid is connected and no PV excess (A)
    #[serde(default)]
    pub grid_no_excess_a: f64,
    /// Minimum PV excess to trigger grid_pv_excess_a (W)
    #[serde(default = "default_pv_excess_threshold_w")]
    pub pv_excess_threshold_w: f64,
}

fn default_offgrid_charge_a() -> f64 { 70.0 }
fn default_pgrid_pv_a() -> f64 { 4.0 }
fn default_pv_excess_threshold_w() -> f64 { 50.0 }

impl Default for ChargeCurrent {
    fn default() -> Self {
        Self {
            offgrid_max_a: default_offgrid_charge_a(),
            grid_pv_excess_a: default_pgrid_pv_a(),
            grid_no_excess_a: 0.0,
            pv_excess_threshold_w: default_pv_excess_threshold_w(),
        }
    }
}

// ---------------------------------------------------------------------------
// DEYE command logic
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct DeyeConfig {
    /// Single frequency boundary (Hz): freq ≥ this → cut side (debounced by `cut_delay_secs`,
    /// or immediate at `freq_hard_hz`); freq < this → restore side. No dead band. Set below
    /// the DEYE's own 51.5 Hz self-trip so the Shelly relay pre-empts it.
    #[serde(default = "default_freq_high")]
    pub freq_high_hz: f64,
    /// Hard frequency threshold for an immediate cut, bypassing the debounce (Hz).
    /// Safety net if the frequency ramps quickly toward the 51.5 Hz self-trip.
    #[serde(default = "default_freq_hard")]
    pub freq_hard_hz: f64,
    /// Debounce before a soft cut: frequency must stay at/above the boundary this long (seconds).
    #[serde(default = "default_cut_delay_secs")]
    pub cut_delay_secs: u64,
    /// Sustained below-boundary time required before restoring the DEYE (seconds).
    #[serde(default = "default_reenable_delay_secs")]
    pub reenable_delay_secs: u64,
    /// Anti-oscillation lockout after a cut — mandatory off-time before the DEYE may restore
    /// (seconds). Main anti-thrash mechanism now that the frequency hysteresis is purely temporal.
    #[serde(default = "default_lockout_secs")]
    pub lockout_secs: u64,
    /// Period at which the desired relay state is re-asserted on every channel,
    /// so the physical relay reconverges after a missed command or Shelly reboot (seconds).
    #[serde(default = "default_relay_resync_secs")]
    pub relay_resync_secs: u64,
    /// Cut the DEYE based on the MPPT charge stage (battery topping/full), on top of the
    /// AC-frequency machine. Lets the battery finish charging on the DC-coupled MPPT alone,
    /// pre-empting the AC-out frequency rise. Disable to fall back to frequency-only.
    #[serde(default)]
    pub mppt_cut_enabled: bool,
    /// MPPT solar-charger State codes meaning "battery topping off / full" → cut & hold DEYE.
    /// Default [4,5,6] = Absorption, Float, Storage (3=Bulk means the battery still charges,
    /// so a charger in Bulk allows restore).
    #[serde(default = "default_mppt_full_states")]
    pub mppt_full_states: Vec<i64>,
    /// Debounce: the MPPT-full condition must hold this long before cutting (seconds).
    #[serde(default = "default_mppt_cut_delay_secs")]
    pub mppt_cut_delay_secs: u64,
    /// Freshness window for the DEYE decision inputs (frequency + MPPT State), in seconds.
    /// Beyond this a telemetry value is considered stale (topic silent): a stale MPPT State
    /// no longer blocks restore, and a stale frequency is treated as nominal (restore allowed,
    /// the DEYE 51.5 Hz auto-trip being the hardware net). Must stay well above the Venus
    /// keepalive period (30 s) to avoid false positives. Default 90 s (3× keepalive).
    #[serde(default = "default_input_max_age_secs")]
    pub input_max_age_secs: u64,
}

fn default_freq_high() -> f64 { 51.0 }
fn default_freq_hard() -> f64 { 51.3 }
fn default_cut_delay_secs() -> u64 { 3 }
fn default_reenable_delay_secs() -> u64 { 45 }
fn default_lockout_secs() -> u64 { 120 }
fn default_relay_resync_secs() -> u64 { 60 }
fn default_mppt_full_states() -> Vec<i64> { vec![4, 5, 6] }
fn default_mppt_cut_delay_secs() -> u64 { 10 }
fn default_input_max_age_secs() -> u64 { 90 }

impl Default for DeyeConfig {
    fn default() -> Self {
        Self {
            freq_high_hz: default_freq_high(),
            freq_hard_hz: default_freq_hard(),
            cut_delay_secs: default_cut_delay_secs(),
            reenable_delay_secs: default_reenable_delay_secs(),
            lockout_secs: default_lockout_secs(),
            relay_resync_secs: default_relay_resync_secs(),
            mppt_cut_enabled: false,
            mppt_full_states: default_mppt_full_states(),
            mppt_cut_delay_secs: default_mppt_cut_delay_secs(),
            input_max_age_secs: default_input_max_age_secs(),
        }
    }
}

// ---------------------------------------------------------------------------
// Water heater management
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct WaterHeaterConfig {
    /// Kept for TOML compatibility — no longer used in rule evaluation
    #[serde(default = "default_solar_min_w")]
    pub solar_min_w: f64,
    /// Kept for TOML compatibility — no longer used in rule evaluation
    #[serde(default = "default_debounce_secs")]
    pub debounce_secs: u64,
    /// Minimum time between two mode changes (seconds)
    #[serde(default = "default_mode_change_min_secs")]
    pub mode_change_min_secs: u64,
    /// Target temperature in HEAT_PUMP mode (°C)
    #[serde(default = "default_hp_target_c")]
    pub heat_pump_target_c: f64,
    /// Target temperature in VACATION mode (°C)
    #[serde(default = "default_vacation_target_c")]
    pub vacation_target_c: f64,
    /// Delay after mode change before setting temperature (seconds)
    #[serde(default = "default_temp_set_delay_secs")]
    pub temp_set_delay_secs: u64,
    /// Keepalive interval for Venus OS watchdog (seconds)
    #[serde(default = "default_keepalive_secs")]
    pub keepalive_secs: u64,
    /// Minimum irradiance to allow HEAT_PUMP mode (W/m²)
    #[serde(default = "default_irradiance_min_wm2")]
    pub irradiance_min_wm2: f64,
    /// Minimum battery SOC to allow HEAT_PUMP mode (%)
    #[serde(default = "default_soc_min_pct")]
    pub soc_min_pct: f64,
    /// Température à partir de laquelle la cuve est considérée « cible atteinte » (°C).
    /// Si la température actuelle reste ≥ ce seuil pendant `temp_max_hold_secs`,
    /// la règle force le passage en VACATION (inutile de maintenir la pompe à chaleur).
    #[serde(default = "default_temp_max_c")]
    pub temp_max_c: f64,
    /// Durée de maintien à `temp_max_c` avant de forcer VACATION (secondes). Défaut : 600 (10 min).
    #[serde(default = "default_temp_max_hold_secs")]
    pub temp_max_hold_secs: u64,
    /// URL d'écriture des métriques chauffe-eau (daly-bms-server → redb,
    /// endpoint POST /api/v1/import/prometheus). Défaut : http://127.0.0.1:8080
    #[serde(default = "default_wh_vm_url")]
    pub vm_url: String,
}

fn default_solar_min_w() -> f64 { 1000.0 }
fn default_debounce_secs() -> u64 { 300 }
fn default_mode_change_min_secs() -> u64 { 900 }
fn default_hp_target_c() -> f64 { 60.0 }
fn default_vacation_target_c() -> f64 { 45.0 }
fn default_temp_set_delay_secs() -> u64 { 15 }
fn default_keepalive_secs() -> u64 { 25 }
fn default_irradiance_min_wm2() -> f64 { 300.0 }
fn default_soc_min_pct() -> f64 { 90.0 }
fn default_temp_max_c() -> f64 { 60.0 }
fn default_temp_max_hold_secs() -> u64 { 600 }
fn default_wh_vm_url() -> String { "http://127.0.0.1:8080".into() }

impl Default for WaterHeaterConfig {
    fn default() -> Self {
        Self {
            solar_min_w: default_solar_min_w(),
            debounce_secs: default_debounce_secs(),
            mode_change_min_secs: default_mode_change_min_secs(),
            heat_pump_target_c: default_hp_target_c(),
            vacation_target_c: default_vacation_target_c(),
            temp_set_delay_secs: default_temp_set_delay_secs(),
            keepalive_secs: default_keepalive_secs(),
            irradiance_min_wm2: default_irradiance_min_wm2(),
            soc_min_pct: default_soc_min_pct(),
            temp_max_c: default_temp_max_c(),
            temp_max_hold_secs: default_temp_max_hold_secs(),
            vm_url: default_wh_vm_url(),
        }
    }
}

// ---------------------------------------------------------------------------
// Solar production
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct SolarConfig {
    /// URL of daly-bms-server for solar data POST
    #[serde(default = "default_bms_server_url")]
    pub bms_server_url: String,
}

fn default_bms_server_url() -> String { "http://192.168.1.141:8080".into() }

impl Default for SolarConfig {
    fn default() -> Self {
        Self {
            bms_server_url: default_bms_server_url(),
        }
    }
}

// ---------------------------------------------------------------------------
// Platform status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PlatformConfig {
    #[serde(default = "default_platform_interval_secs")]
    pub publish_interval_secs: u64,
}

fn default_platform_interval_secs() -> u64 { 60 }

impl Default for PlatformConfig {
    fn default() -> Self {
        Self { publish_interval_secs: default_platform_interval_secs() }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_true() -> bool { true }

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

pub fn load() -> Result<EnergyManagerConfig> {
    // Load .env first (secrets: LG tokens, etc.)
    dotenvy::dotenv().ok();

    let path = std::env::var("ENERGY_CONFIG")
        .unwrap_or_else(|_| find_config_path());

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("Cannot read config file: {path}"))?;

    // Détection des typos dans NOTRE section (audit 2026-06 §12) : une clé
    // inconnue sous [energy_manager.*] était silencieusement ignorée → la
    // valeur par défaut s'appliquait sans bruit. Le reste du fichier
    // (sections daly-bms-server) est légitimement inconnu ici — pas de
    // `deny_unknown_fields`, qui casserait le Config.toml partagé.
    let de = toml::Deserializer::new(&raw);
    let mut unknown: Vec<String> = Vec::new();
    let mut cfg: EnergyConfig = serde_ignored::deserialize(de, |p| unknown.push(p.to_string()))
        .with_context(|| format!("Invalid TOML in {path}"))?;
    for key in unknown.iter().filter(|p| p.starts_with("energy_manager")) {
        tracing::warn!(cle = %key, fichier = %path, "Config : clé inconnue ignorée (typo ?)");
    }

    // Override sensitive fields from environment
    if let Ok(v) = std::env::var("LG_DEVICE_ID") {
        cfg.energy_manager.lg_thinq.device_id = v;
    }
    if let Ok(v) = std::env::var("LG_BEARER_TOKEN") {
        cfg.energy_manager.lg_thinq.bearer_token = v;
    }
    if let Ok(v) = std::env::var("LG_API_KEY") {
        cfg.energy_manager.lg_thinq.api_key = v;
    }
    cfg.energy_manager.validate()?;
    Ok(cfg.energy_manager)
}

impl EnergyManagerConfig {
    /// Validation des bornes (audit 2026-06 §12) — volontairement minimale :
    /// uniquement les invariants dont la violation produit un comportement
    /// pathologique (boucle de polling chaude, service inopérant).
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.mqtt.host.trim().is_empty(),
            "config invalide : `energy_manager.mqtt.host` est vide"
        );
        anyhow::ensure!(
            self.mqtt.port > 0,
            "config invalide : `energy_manager.mqtt.port` doit être > 0"
        );
        anyhow::ensure!(
            !self.api.bind.trim().is_empty(),
            "config invalide : `energy_manager.api.bind` est vide"
        );
        if self.open_meteo.enabled {
            anyhow::ensure!(
                self.open_meteo.poll_interval_secs > 0,
                "config invalide : `energy_manager.open_meteo.poll_interval_secs` doit être > 0"
            );
        }
        if self.lg_thinq.enabled {
            anyhow::ensure!(
                self.lg_thinq.poll_interval_secs > 0,
                "config invalide : `energy_manager.lg_thinq.poll_interval_secs` doit être > 0"
            );
        }
        Ok(())
    }
}

fn find_config_path() -> String {
    let candidates = [
        "./Config.toml",
        "/etc/daly-bms/config.toml",
    ];
    for p in &candidates {
        if Path::new(p).exists() {
            return p.to_string();
        }
    }
    candidates[0].to_string()
}
