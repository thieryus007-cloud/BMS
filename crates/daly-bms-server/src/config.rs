//! Chargement et désérialisation de la configuration TOML.
//!
//! La configuration est lue depuis `/etc/daly-bms/config.toml` par défaut,
//! ou depuis le chemin spécifié par `DALY_CONFIG` ou en argument CLI.

use serde::{Deserialize, Serialize};
use std::path::Path;
use anyhow::{Context, Result};

/// Parse une adresse Modbus au format "0x05" ou "5" en u8.
/// Retourne `None` si le format est invalide (overflow u8 ou non-numérique).
fn parse_modbus_address(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u8::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<u8>().ok()
    }
}

// =============================================================================
// Structure principale
// =============================================================================

/// Configuration complète de l'application.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    #[serde(default)]
    pub serial: SerialConfig,

    #[serde(default)]
    pub api: ApiConfig,

    #[serde(default)]
    pub logging: LoggingConfig,

    #[serde(default)]
    pub mqtt: MqttConfig,

    /// Client VictoriaMetrics (stockage time-series externe)
    #[serde(default)]
    pub victoriametrics: VmConfig,

    #[serde(default)]
    pub alerts: AlertsConfig,

    #[serde(default)]
    pub read_only: ReadOnlyConfig,

    /// Configurations individuelles par BMS (optionnel)
    #[serde(default)]
    pub bms: Vec<BmsDeviceConfig>,

    /// Configuration ET112 (compteurs Carlo Gavazzi RS485)
    #[serde(default)]
    pub et112: Et112Config,

    /// Configuration ATS CHINT NXZB/NXZBN (bus RS485 dédié, parité Even)
    pub ats: Option<AtsConfig>,

    /// Capteur d'irradiance PRALRAN RS485 (sur le bus unifié)
    /// Remplace le service Python `irradiance-rs485`.
    pub irradiance: Option<IrradianceConfig>,

    /// Prises connectées Tasmota (MQTT natif Tasmota)
    #[serde(default)]
    pub tasmota: TasmotaConfig,

    /// Compteurs d'énergie Shelly EM (MQTT natif Shelly)
    #[serde(default)]
    pub shelly: ShellyConfig,
}

// =============================================================================
// Configuration par BMS
// =============================================================================

/// Surcharges de configuration pour un BMS individuel.
///
/// ```toml
/// [[bms]]
/// address        = "0x28"     # adresse RS485 (décimal ou hex)
/// name           = "BMS-360Ah"
/// capacity_ah    = 360.0
/// max_charge_a   = 200.0
/// max_discharge_a= 120.0
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct BmsDeviceConfig {
    /// Adresse RS485 (ex : "0x01", "1", "40")
    pub address: String,
    /// Nom affiché dans le dashboard
    pub name: Option<String>,
    /// Capacité nominale installée (Ah)
    pub capacity_ah: Option<f32>,
    /// Courant de charge maximal autorisé (A)
    pub max_charge_a: Option<f32>,
    /// Courant de décharge maximal autorisé (A)
    pub max_discharge_a: Option<f32>,
    /// Index MQTT pour le topic Venus OS (ex: 1 → santuario/bms/1/venus).
    /// Si absent, utilise la position dans le tableau [[bms]] (1-based).
    pub mqtt_index: Option<u8>,
}

impl BmsDeviceConfig {
    /// Parse l'adresse en u8 (supporte "0x28", "40", "1")
    pub fn parsed_address(&self) -> Option<u8> {
        parse_modbus_address(&self.address)
    }
}

impl AppConfig {
    /// Charge la configuration depuis un fichier TOML.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Impossible de lire {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("Erreur de parsing TOML dans {}", path.display()))
    }

    /// Charge depuis le chemin par défaut ou `DALY_CONFIG`.
    ///
    /// Ordre de recherche :
    /// 1. Variable d'environnement `DALY_CONFIG`
    /// 2. `./Config.toml` (répertoire courant — développement Windows)
    /// 3. `/etc/daly-bms/config.toml` (déploiement Linux)
    pub fn load_default() -> Result<Self> {
        if let Ok(path) = std::env::var("DALY_CONFIG") {
            return Self::load(Path::new(&path));
        }
        for candidate in &["Config.toml", "/etc/daly-bms/config.toml"] {
            let p = Path::new(candidate);
            if p.exists() {
                return Self::load(p);
            }
        }
        anyhow::bail!("Config non trouvée — ni Config.toml ni /etc/daly-bms/config.toml");
    }

    /// Retourne la liste des adresses BMS configurées.
    pub fn bms_addresses(&self) -> Vec<u8> {
        self.serial.addresses.iter()
            .filter_map(|s| {
                let s = s.trim();
                if s.starts_with("0x") || s.starts_with("0X") {
                    u8::from_str_radix(&s[2..], 16).ok()
                } else {
                    s.parse::<u8>().ok()
                }
            })
            .collect()
    }
}

// =============================================================================
// Sous-sections
// =============================================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SerialConfig {
    /// Chemin du port série (ex: /dev/ttyUSB0)
    pub port: String,
    /// Vitesse en bauds (Daly = 9600)
    pub baud: u32,
    /// Intervalle de polling global (ms)
    pub poll_interval_ms: u64,
    /// Nombre de cellules par défaut
    pub default_cell_count: u8,
    /// Nombre de sondes NTC par défaut
    pub default_temp_sensors: u8,
    /// Taille du ring buffer (snapshots conservés en mémoire)
    pub ring_buffer_size: usize,
    /// Activer la découverte automatique
    pub auto_discover: bool,
    pub auto_discover_start: u8,
    pub auto_discover_end: u8,
    /// Liste explicite d'adresses BMS (ex: ["0x01", "0x02"])
    #[serde(default)]
    pub addresses: Vec<String>,
}

// =============================================================================
// Configuration ET112
// =============================================================================

/// Configuration globale pour les compteurs Carlo Gavazzi ET112.
///
/// Bus RS485 unifié : port et baud hérités du SharedBus global (ttyUSB0).
///
/// ```toml
/// [et112]
/// poll_interval_ms = 5000
/// ring_buffer_size = 720       # 1 heure à 1 mesure / 5s
///
/// [[et112.devices]]
/// address          = "0x07"
/// name             = "Micro-inverseurs"
/// mqtt_index       = 3         # → topic santuario/pvinverter/3/venus
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Et112Config {
    /// Intervalle de polling en ms
    pub poll_interval_ms: u64,
    /// Taille du ring buffer par appareil
    pub ring_buffer_size: usize,
    /// Liste des compteurs ET112 configurés
    #[serde(default)]
    pub devices: Vec<Et112DeviceConfig>,
}

impl Default for Et112Config {
    fn default() -> Self {
        Self {
            poll_interval_ms: 5000,
            ring_buffer_size: 720,
            devices:          Vec::new(),
        }
    }
}

/// Configuration d'un ET112 individuel.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Et112DeviceConfig {
    /// Adresse Modbus (ex: "0x03", "3")
    pub address: String,
    /// Nom affiché dans le dashboard
    #[serde(default = "default_et112_name")]
    pub name: String,
    /// Index MQTT → topic `santuario/{service_type}/{mqtt_index}/venus`
    pub mqtt_index: Option<u8>,
    /// Type de service D-Bus : "pvinverter" ou "acload" (défaut: "pvinverter")
    #[serde(default = "default_et112_service_type")]
    pub service_type: String,
    /// Puissance nominale max (W) — pour l'affichage gauge
    pub max_power_w: Option<f32>,
    /// Position sur le bus AC Victron : 0=AC Input, 1=AC Output (défaut)
    #[serde(default = "default_et112_position")]
    pub position: u8,
}

fn default_et112_position() -> u8 { 1 }

const DEFAULT_ET112_SERVICE_TYPE: &str = "pvinverter";
fn default_et112_service_type() -> String { DEFAULT_ET112_SERVICE_TYPE.to_string() }

const DEFAULT_ET112_NAME: &str = "ET112";
fn default_et112_name() -> String { DEFAULT_ET112_NAME.to_string() }

// =============================================================================
// Configuration ATS CHINT
// =============================================================================

/// Configuration du commutateur automatique CHINT ATS (NXZB/NXZBN).
///
/// L'ATS utilise le **bus RS485 unifié** (même port que les BMS Daly, ET112 et irradiance).
/// Câbler l'ATS sur le même bus RS485 `/dev/ttyUSB0`, adresse Modbus unique.
///
/// **IMPORTANT parité** : le bus unifié tourne en 9600-8N1 (aucune parité).
/// Configurer la parité de l'ATS à "None" via son bouton Setup ou via
/// le registre Modbus 0x000E (0=None, 1=Odd, 2=Even) depuis l'ancienne connexion.
///
/// ```toml
/// [ats]
/// enabled          = true
/// address          = 6           # Adresse Modbus configurée sur l'ATS (défaut usine = 6)
/// name             = "ATS CHINT NXZB"
/// poll_interval_ms = 5000
/// mqtt_index       = 1           # → santuario/switch/1/venus → com.victronenergy.switch.mqtt_1
/// device_instance  = 60          # Instance D-Bus Venus OS / VRM
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AtsConfig {
    /// Activer le polling ATS.
    #[serde(default = "default_ats_enabled")]
    pub enabled: bool,

    /// Adresse Modbus de l'ATS (défaut 6, défaut usine CHINT).
    #[serde(default = "default_ats_address")]
    pub address: u8,

    /// Nom affiché dans le dashboard et les logs.
    #[serde(default = "default_ats_name")]
    pub name: String,

    /// Intervalle de polling en ms (défaut 5000 = toutes les 5 secondes).
    #[serde(default = "default_ats_interval")]
    pub poll_interval_ms: u64,

    /// Index MQTT → topic `santuario/switch/{mqtt_index}/venus` (défaut 1).
    #[serde(default = "default_ats_mqtt_index")]
    pub mqtt_index: u8,

    /// Instance D-Bus Venus OS (défaut 60).
    #[serde(default = "default_ats_device_instance")]
    pub device_instance: u16,
}

fn default_ats_enabled()         -> bool   { true }
fn default_ats_address()         -> u8     { 6 }

const DEFAULT_ATS_NAME: &str = "ATS CHINT";
fn default_ats_name()            -> String { DEFAULT_ATS_NAME.to_string() }

fn default_ats_interval()        -> u64    { 5000 }
fn default_ats_mqtt_index()      -> u8     { 1 }
fn default_ats_device_instance() -> u16    { 60 }

impl Et112DeviceConfig {
    /// Parse l'adresse en u8 (supporte "0x03", "3").
    pub fn parsed_address(&self) -> u8 {
        parse_modbus_address(&self.address).unwrap_or_else(|| {
            tracing::warn!(addr = %self.address, "ET112 adresse invalide, fallback 0x03");
            3
        })
    }
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self {
            port:               if cfg!(windows) { "COM1".into() } else { "/dev/ttyUSB0".into() },
            baud:               9600,
            poll_interval_ms:   1000,
            default_cell_count: 16,
            default_temp_sensors: 4,
            ring_buffer_size:   3600,
            auto_discover:      false,
            auto_discover_start: 1,
            auto_discover_end:  16,
            addresses:          vec!["0x01".into()],
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiConfig {
    /// Adresse de bind (ex: 0.0.0.0:8000)
    pub bind: String,
    /// Clé API pour les endpoints d'écriture (vide = pas d'auth)
    pub api_key: String,
    /// Activer CORS pour tous les origines
    pub cors_allow_all: bool,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind:          "0.0.0.0:8000".into(),
            api_key:       String::new(),
            cors_allow_all: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
    /// Niveau de log (trace, debug, info, warn, error)
    pub level: String,
    /// Format (pretty | json)
    pub format: String,
    /// Répertoire pour les logs tournants (daily rolling).
    /// Laisser vide pour désactiver l'écriture sur disque.
    #[serde(default = "default_log_dir")]
    pub log_dir: String,
    /// Nombre de jours de logs à conserver.
    #[serde(default = "default_log_keep_days")]
    pub log_keep_days: u64,
}

fn default_log_dir() -> String { "/var/log/daly-bms".into() }
fn default_log_keep_days() -> u64 { 7 }

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level:         "info".into(),
            format:        "pretty".into(),
            log_dir:       default_log_dir(),
            log_keep_days: default_log_keep_days(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct MqttConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub topic_prefix: String,
    pub publish_interval_sec: f64,
    pub username: Option<String>,
    pub password: Option<String>,
    /// "json" | "simple"
    pub format: String,
}

impl MqttConfig {
    #[allow(dead_code)]
    pub fn default_enabled() -> Self {
        Self {
            enabled:              false,
            host:                 "localhost".into(),
            port:                 1883,
            topic_prefix:         "santuario/bms".into(),
            publish_interval_sec: 5.0,
            username:             None,
            password:             None,
            format:               "json".into(),
        }
    }
}

/// Configuration du client VictoriaMetrics.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VmConfig {
    /// Activer l'écriture/lecture VictoriaMetrics
    pub enabled: bool,
    /// URL de base de l'instance VM (ex: http://127.0.0.1:8428)
    pub url: String,
    /// Timeout des requêtes HTTP en secondes
    #[serde(default = "default_vm_timeout")]
    pub timeout_secs: u64,
}

fn default_vm_timeout() -> u64 { 10 }

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            enabled:      false,
            url:          "http://127.0.0.1:8428".to_string(),
            timeout_secs: 10,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AlertsConfig {
    pub db_path: String,
    pub check_interval_sec: f64,
    pub telegram_token: String,
    pub telegram_chat_id: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    pub smtp_password: String,
    pub smtp_from: String,
    pub smtp_to: String,
    #[serde(default)]
    pub thresholds: AlertThresholds,
    /// Durée minimale avant déclenchement (secondes). 0 = immédiat (comportement legacy).
    #[serde(default)]
    pub soc_critical_for_secs: u64,
    #[serde(default)]
    pub soc_low_for_secs: u64,
    #[serde(default)]
    pub temp_high_for_secs: u64,
    #[serde(default)]
    pub current_high_for_secs: u64,
    #[serde(default)]
    pub voltage_for_secs: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AlertThresholds {
    pub cell_ovp_v: f32,
    pub cell_uvp_v: f32,
    pub cell_delta_mv: f32,
    pub soc_low_percent: f32,
    pub soc_critical_percent: f32,
    pub temp_high_c: f32,
    pub current_high_a: f32,
    /// Tension de pack trop haute (V) — règle ESS pack_ovp.
    #[serde(default = "default_pack_ovp_v")]
    pub pack_ovp_v: f32,
    /// Tension de pack trop basse (V) — règle ESS pack_uvp.
    #[serde(default = "default_pack_uvp_v")]
    pub pack_uvp_v: f32,
}

// Seuils d'alerte par défaut — valeurs de production (voir CLAUDE.md §5).
// Ne pas modifier sans mettre à jour la documentation.
const DEFAULT_CELL_OVP_V:           f32 = 3.60;
const DEFAULT_CELL_UVP_V:           f32 = 2.90;
const DEFAULT_CELL_DELTA_MV:        f32 = 100.0;
const DEFAULT_SOC_LOW_PERCENT:      f32 = 20.0;
const DEFAULT_SOC_CRITICAL_PERCENT: f32 = 10.0;
const DEFAULT_TEMP_HIGH_C:          f32 = 45.0;
const DEFAULT_CURRENT_HIGH_A:       f32 = 80.0;
const DEFAULT_PACK_OVP_V:           f32 = 57.0;
const DEFAULT_PACK_UVP_V:           f32 = 44.0;

fn default_pack_ovp_v() -> f32 { DEFAULT_PACK_OVP_V }
fn default_pack_uvp_v() -> f32 { DEFAULT_PACK_UVP_V }

impl Default for AlertThresholds {
    fn default() -> Self {
        Self {
            cell_ovp_v:            DEFAULT_CELL_OVP_V,
            cell_uvp_v:            DEFAULT_CELL_UVP_V,
            cell_delta_mv:         DEFAULT_CELL_DELTA_MV,
            soc_low_percent:       DEFAULT_SOC_LOW_PERCENT,
            soc_critical_percent:  DEFAULT_SOC_CRITICAL_PERCENT,
            temp_high_c:           DEFAULT_TEMP_HIGH_C,
            current_high_a:        DEFAULT_CURRENT_HIGH_A,
            pack_ovp_v:            DEFAULT_PACK_OVP_V,
            pack_uvp_v:            DEFAULT_PACK_UVP_V,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ReadOnlyConfig {
    pub enabled: bool,
}

// =============================================================================
// Configuration capteur irradiance PRALRAN
// =============================================================================

/// Configuration du capteur d'irradiance solaire PRALRAN RS485.
///
/// Le capteur utilise le **bus RS485 unifié** (même port que les BMS Daly).
/// Aucun `port` / `baud` à configurer ici — ils sont hérités de `[serial]`.
///
/// ```toml
/// [irradiance]
/// address          = "0x05"
/// name             = "Irradiance PRALRAN"
/// poll_interval_ms = 5000
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IrradianceConfig {
    /// Adresse Modbus du capteur (ex: "0x05", "5")
    pub address: String,
    /// Nom affiché dans les logs et le dashboard
    #[serde(default = "default_irradiance_name")]
    pub name: String,
    /// Intervalle de polling en ms (défaut 5000)
    #[serde(default = "default_irradiance_interval")]
    pub poll_interval_ms: u64,
}

const DEFAULT_IRRADIANCE_NAME: &str = "Irradiance PRALRAN";
fn default_irradiance_name() -> String { DEFAULT_IRRADIANCE_NAME.to_string() }
fn default_irradiance_interval() -> u64 { 5000 }

impl IrradianceConfig {
    /// Parse l'adresse en u8 (supporte "0x05", "5")
    pub fn parsed_address(&self) -> u8 {
        parse_modbus_address(&self.address).unwrap_or_else(|| {
            tracing::warn!(addr = %self.address, "Irradiance adresse invalide, fallback 0x05");
            5
        })
    }
}

// =============================================================================
// Configuration prises Tasmota
// =============================================================================

/// Configuration globale pour les prises connectées Tasmota.
///
/// Tasmota publie nativement via MQTT :
///   tele/{tasmota_id}/SENSOR  → énergie, puissance, tension, courant
///   stat/{tasmota_id}/POWER   → état relais ON/OFF
///
/// ```toml
/// [tasmota]
/// ring_buffer_size = 720
///
/// [[tasmota.devices]]
/// id           = 1
/// tasmota_id   = "tasmota_01"
/// name         = "Prise Salon"
/// mqtt_index   = 1
/// service_type = "switch"
/// device_instance = 500
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TasmotaConfig {
    /// Taille du ring buffer par appareil
    pub ring_buffer_size: usize,
    /// Liste des prises Tasmota configurées
    #[serde(default)]
    pub devices: Vec<TasmotaDeviceConfig>,
}

impl Default for TasmotaConfig {
    fn default() -> Self {
        Self {
            ring_buffer_size: 720,
            devices:          Vec::new(),
        }
    }
}

/// Configuration d'une prise Tasmota individuelle.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TasmotaDeviceConfig {
    /// Identifiant interne unique (clé du ring buffer, ex: 1)
    pub id: u8,
    /// Nom du device Tasmota dans les topics MQTT (ex: "tasmota_01")
    pub tasmota_id: String,
    /// Nom affiché dans le dashboard
    #[serde(default = "default_tasmota_name")]
    pub name: String,
    /// Index MQTT → forward vers Venus OS : `santuario/{service_type}/{mqtt_index}/venus`
    pub mqtt_index: Option<u8>,
    /// Type de service D-Bus : "switch" (défaut) ou "acload"
    #[serde(default = "default_tasmota_service_type")]
    pub service_type: String,
    /// Instance D-Bus Venus OS (ex: 500)
    pub device_instance: Option<u16>,
}

const DEFAULT_TASMOTA_NAME: &str = "Tasmota";
fn default_tasmota_name()         -> String { DEFAULT_TASMOTA_NAME.to_string() }

const DEFAULT_TASMOTA_SERVICE_TYPE: &str = "switch";
fn default_tasmota_service_type() -> String { DEFAULT_TASMOTA_SERVICE_TYPE.to_string() }

const DEFAULT_SHELLY_NAME: &str = "Shelly";
fn default_shelly_name()          -> String { DEFAULT_SHELLY_NAME.to_string() }

// =============================================================================
// Configuration Shelly (stub — implémentation future)
// =============================================================================

/// Configuration des compteurs d'énergie Shelly EM (MQTT natif Shelly).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ShellyConfig {
    /// Taille du ring buffer par appareil
    pub ring_buffer_size: usize,
    /// Liste des appareils Shelly configurés
    #[serde(default)]
    pub devices: Vec<ShellyDeviceConfig>,
}

impl Default for ShellyConfig {
    fn default() -> Self {
        Self {
            ring_buffer_size: 720,
            devices:          Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_modbus_address_hex_lowercase() {
        assert_eq!(parse_modbus_address("0x05"), Some(5));
        assert_eq!(parse_modbus_address("0xff"), Some(255));
    }

    #[test]
    fn parse_modbus_address_hex_uppercase() {
        assert_eq!(parse_modbus_address("0X05"), Some(5));
        assert_eq!(parse_modbus_address("0XFF"), Some(255));
    }

    #[test]
    fn parse_modbus_address_decimal() {
        assert_eq!(parse_modbus_address("5"), Some(5));
        assert_eq!(parse_modbus_address("255"), Some(255));
    }

    #[test]
    fn parse_modbus_address_whitespace_trimmed() {
        assert_eq!(parse_modbus_address("  0x07 "), Some(7));
        assert_eq!(parse_modbus_address(" 40 "), Some(40));
    }

    #[test]
    fn parse_modbus_address_invalid_returns_none() {
        assert_eq!(parse_modbus_address(""), None);
        assert_eq!(parse_modbus_address("0xZZ"), None);
        assert_eq!(parse_modbus_address("300"), None); // overflow u8
        assert_eq!(parse_modbus_address("not-a-number"), None);
    }

    #[test]
    fn alert_thresholds_defaults_unchanged() {
        let t = AlertThresholds::default();
        assert_eq!(t.cell_ovp_v, 3.60);
        assert_eq!(t.cell_uvp_v, 2.90);
        assert_eq!(t.cell_delta_mv, 100.0);
        assert_eq!(t.soc_low_percent, 30.0);
        assert_eq!(t.soc_critical_percent, 20.0);
        assert_eq!(t.temp_high_c, 45.0);
        assert_eq!(t.current_high_a, 150.0);
        assert_eq!(t.pack_ovp_v, 57.0);
        assert_eq!(t.pack_uvp_v, 44.0);
    }
}

/// Configuration d'un appareil Shelly individuel (stub).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ShellyDeviceConfig {
    /// Identifiant interne unique
    pub id: u8,
    /// Identifiant du device Shelly dans les topics MQTT (ex: "shellyem-A1B2C3")
    pub shelly_id: String,
    /// Nom affiché dans le dashboard
    #[serde(default = "default_shelly_name")]
    pub name: String,
}
