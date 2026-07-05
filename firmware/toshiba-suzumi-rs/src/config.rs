//! Configuration d'un **nœud** (un ESP32 = une unité intérieure = une « zone »)
//! et dérivation des topics MQTT.
//!
//! Logique **pure**, host-testable. Le chargement réel (NVS sur ESP32, ou fichier)
//! sera un adaptateur mince ajouté avec la partie firmware ; ce module ne porte que
//! la **structure**, la **validation** et la **dérivation des topics**.
//!
//! **Schéma de topics retenu** (voie Rust — c'est le firmware qui les définit, donc
//! plus besoin de « les observer au 1er boot » comme avec ESPHome) :
//!
//! ```text
//! santuario/toshiba/<zone>/state         (publish, retained : télémétrie JSON)
//! santuario/toshiba/<zone>/command       (subscribe : commande JSON)
//! santuario/toshiba/<zone>/availability  (publish, LWT : "online"/"offline")
//! ```
//!
//! ⚠️ Le préfixe `santuario/toshiba/...` est **local** (non relayé par le bridge
//! NanoPi) → pas de risque de boucle (règle projet #11). Le module Pi5
//! `energy-manager logic/toshiba_ac` consommera ce schéma.

/// Racine de topics **locale** (non bridgée).
pub const TOPIC_ROOT: &str = "santuario/toshiba";

/// GPIO par défaut (convention du composant ESPHome, cf. plan §3.2).
pub const DEFAULT_TX_PIN: u8 = 33;
pub const DEFAULT_RX_PIN: u8 = 32;
/// Port MQTT par défaut (Mosquitto Pi5).
pub const DEFAULT_MQTT_PORT: u16 = 1883;

/// Erreurs de validation de configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    EmptyZone,
    /// La zone contient un caractère interdit dans un segment de topic (`/`, `+`, `#`).
    InvalidZone(char),
    EmptyWifiSsid,
    EmptyMqttHost,
    ZeroMqttPort,
    /// TX et RX sur le même GPIO.
    DuplicatePins(u8),
}

/// Configuration d'un nœud (une zone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeConfig {
    /// Segment de topic identifiant l'unité (ex. `"salon"`, `"chambre"`, `"bureau"`).
    pub zone: String,
    pub wifi_ssid: String,
    pub wifi_pass: String,
    pub mqtt_host: String,
    pub mqtt_port: u16,
    pub mqtt_user: Option<String>,
    pub mqtt_pass: Option<String>,
    pub uart_tx_pin: u8,
    pub uart_rx_pin: u8,
}

impl NodeConfig {
    /// Construit une config avec les valeurs par défaut (port 1883, GPIO 33/32,
    /// broker anonyme). Les champs obligatoires sont demandés explicitement.
    pub fn new(
        zone: impl Into<String>,
        wifi_ssid: impl Into<String>,
        wifi_pass: impl Into<String>,
        mqtt_host: impl Into<String>,
    ) -> Self {
        Self {
            zone: zone.into(),
            wifi_ssid: wifi_ssid.into(),
            wifi_pass: wifi_pass.into(),
            mqtt_host: mqtt_host.into(),
            mqtt_port: DEFAULT_MQTT_PORT,
            mqtt_user: None,
            mqtt_pass: None,
            uart_tx_pin: DEFAULT_TX_PIN,
            uart_rx_pin: DEFAULT_RX_PIN,
        }
    }

    /// Valide la configuration (à appeler au démarrage — échec = refus de boot).
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.zone.is_empty() {
            return Err(ConfigError::EmptyZone);
        }
        if let Some(c) = self
            .zone
            .chars()
            .find(|&c| c == '/' || c == '+' || c == '#')
        {
            return Err(ConfigError::InvalidZone(c));
        }
        if self.wifi_ssid.is_empty() {
            return Err(ConfigError::EmptyWifiSsid);
        }
        if self.mqtt_host.is_empty() {
            return Err(ConfigError::EmptyMqttHost);
        }
        if self.mqtt_port == 0 {
            return Err(ConfigError::ZeroMqttPort);
        }
        if self.uart_tx_pin == self.uart_rx_pin {
            return Err(ConfigError::DuplicatePins(self.uart_tx_pin));
        }
        Ok(())
    }

    /// Topics MQTT dérivés de la zone.
    pub fn topics(&self) -> Topics {
        Topics::for_zone(&self.zone)
    }
}

/// Topics MQTT d'une zone. Voir le schéma en tête de module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topics {
    base: String,
}

impl Topics {
    pub fn for_zone(zone: &str) -> Self {
        Self {
            base: format!("{TOPIC_ROOT}/{zone}"),
        }
    }

    /// `santuario/toshiba/<zone>`
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Télémétrie (publish, retained).
    pub fn state(&self) -> String {
        format!("{}/state", self.base)
    }

    /// Commandes (subscribe).
    pub fn command(&self) -> String {
        format!("{}/command", self.base)
    }

    /// Disponibilité (publish + LWT).
    pub fn availability(&self) -> String {
        format!("{}/availability", self.base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_applied() {
        let c = NodeConfig::new("salon", "StarTh", "secret", "192.168.1.141");
        assert_eq!(c.mqtt_port, DEFAULT_MQTT_PORT);
        assert_eq!(c.uart_tx_pin, 33);
        assert_eq!(c.uart_rx_pin, 32);
        assert!(c.mqtt_user.is_none());
        assert!(c.validate().is_ok());
    }

    #[test]
    fn topics_are_derived_from_zone() {
        let t = NodeConfig::new("chambre", "s", "p", "h").topics();
        assert_eq!(t.base(), "santuario/toshiba/chambre");
        assert_eq!(t.state(), "santuario/toshiba/chambre/state");
        assert_eq!(t.command(), "santuario/toshiba/chambre/command");
        assert_eq!(t.availability(), "santuario/toshiba/chambre/availability");
    }

    #[test]
    fn validation_rejects_bad_input() {
        let mut c = NodeConfig::new("salon", "StarTh", "p", "h");

        c.zone = String::new();
        assert_eq!(c.validate(), Err(ConfigError::EmptyZone));

        c.zone = "sa/lon".to_string();
        assert_eq!(c.validate(), Err(ConfigError::InvalidZone('/')));

        c.zone = "salon".to_string();
        c.wifi_ssid = String::new();
        assert_eq!(c.validate(), Err(ConfigError::EmptyWifiSsid));

        c.wifi_ssid = "StarTh".to_string();
        c.mqtt_port = 0;
        assert_eq!(c.validate(), Err(ConfigError::ZeroMqttPort));

        c.mqtt_port = 1883;
        c.uart_rx_pin = c.uart_tx_pin;
        assert_eq!(c.validate(), Err(ConfigError::DuplicatePins(33)));
    }

    #[test]
    fn wildcard_in_zone_is_rejected() {
        let mut c = NodeConfig::new("a#b", "s", "p", "h");
        assert_eq!(c.validate(), Err(ConfigError::InvalidZone('#')));
        c.zone = "a+b".to_string();
        assert_eq!(c.validate(), Err(ConfigError::InvalidZone('+')));
    }
}
