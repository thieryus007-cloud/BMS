//! Configuration d'un **nœud** (un ESP32 = une unité intérieure) : identité,
//! WiFi, réseau (DHCP/IP statique), broker MQTT, brochage UART, et dérivation
//! des topics MQTT.
//!
//! Logique **pure**, host-testable. Le chargement réel (NVS sur ESP32, variables
//! d'environnement au flash) sera un adaptateur mince ajouté avec la partie
//! firmware ; ce module ne porte que la **structure**, la **validation**, les
//! **valeurs par défaut** et la **dérivation des topics/hostname**.
//!
//! **Identité du nœud** : `node_name` (ex. `"Shorai-31"`) sert de **3 choses à la
//! fois** :
//! 1. **hostname WiFi** (repérable / réservable par DHCP côté box),
//! 2. **segment de topic MQTT** → `santuario/toshiba/Shorai-31/...`,
//! 3. **SSID de l'AP de secours** → `Shorai-31-setup`.
//!
//! Un seul paramètre à changer par unité : `Shorai-31`, `-32`, `-33`, … jusqu'à `-37`.
//!
//! **Schéma de topics** (préfixe LOCAL, non relayé par le bridge NanoPi — règle #11) :
//! ```text
//! santuario/toshiba/<node>/state         (publish, retained : télémétrie JSON)
//! santuario/toshiba/<node>/command       (subscribe : commande JSON)
//! santuario/toshiba/<node>/availability  (publish, LWT : "online"/"offline")
//! ```
//!
//! ⚠️ **Secret (règle projet #12)** : le **mot de passe WiFi n'est PAS** codé en dur
//! ici. `new()` laisse `wifi_pass` **vide** ; la valeur réelle est injectée au
//! **flash** (`option_env!("WIFI_PASS")`) ou provisionnée en **NVS** par le code
//! firmware — jamais commitée.

use std::net::Ipv4Addr;

/// Racine de topics **locale** (non bridgée).
pub const TOPIC_ROOT: &str = "santuario/toshiba";

/// SSID WiFi par défaut (non secret).
pub const DEFAULT_WIFI_SSID: &str = "StarTh";
/// Broker MQTT par défaut (Mosquitto du Pi5).
pub const DEFAULT_MQTT_HOST: &str = "192.168.1.141";
/// Port MQTT par défaut.
pub const DEFAULT_MQTT_PORT: u16 = 1883;
/// GPIO par défaut (convention du composant ESPHome, cf. plan §3.2).
pub const DEFAULT_TX_PIN: u8 = 33;
pub const DEFAULT_RX_PIN: u8 = 32;

/// Mode d'adressage IP du nœud.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IpMode {
    /// Adresse obtenue par DHCP (défaut). Réservation par MAC conseillée côté box.
    #[default]
    Dhcp,
    /// IP fixe côté firmware (utile si pas de réservation DHCP possible).
    Static {
        ip: Ipv4Addr,
        gateway: Ipv4Addr,
        netmask: Ipv4Addr,
        dns: Ipv4Addr,
    },
}

/// Erreurs de validation de configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    EmptyNodeName,
    /// `node_name` contient un caractère interdit (topic/hostname) : `/ + # espace`.
    InvalidNodeName(char),
    EmptyWifiSsid,
    EmptyMqttHost,
    ZeroMqttPort,
    /// TX et RX sur le même GPIO.
    DuplicatePins(u8),
    /// IP statique invalide (adresse ou passerelle non spécifiée, masque nul).
    InvalidStaticIp,
}

/// Configuration d'un nœud. `NodeConfig::new("Shorai-31")` suffit pour une mise
/// en service : WiFi `StarTh` en DHCP + broker Pi5, hostname/topics dérivés.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeConfig {
    /// Identité (ex. `"Shorai-31"`) = hostname + segment de topic + SSID AP secours.
    pub node_name: String,
    pub wifi_ssid: String,
    /// Mot de passe WiFi — **injecté au flash/NVS**, jamais commité (vide par défaut).
    pub wifi_pass: String,
    pub ip: IpMode,
    /// AP de secours si la connexion station échoue (portail de config). Recommandé.
    pub ap_fallback: bool,
    pub mqtt_host: String,
    pub mqtt_port: u16,
    pub mqtt_user: Option<String>,
    pub mqtt_pass: Option<String>,
    pub uart_tx_pin: u8,
    pub uart_rx_pin: u8,
}

impl NodeConfig {
    /// Config de mise en service avec **tous les défauts** : WiFi `StarTh` (mot de
    /// passe à injecter), DHCP, broker Pi5 `192.168.1.141:1883`, GPIO 33/32, AP de
    /// secours activé. Seul `node_name` change d'une unité à l'autre.
    pub fn new(node_name: impl Into<String>) -> Self {
        Self {
            node_name: node_name.into(),
            wifi_ssid: DEFAULT_WIFI_SSID.to_string(),
            wifi_pass: String::new(), // injecté au flash (WIFI_PASS) / NVS — règle #12
            ip: IpMode::Dhcp,
            ap_fallback: true,
            mqtt_host: DEFAULT_MQTT_HOST.to_string(),
            mqtt_port: DEFAULT_MQTT_PORT,
            mqtt_user: None,
            mqtt_pass: None,
            uart_tx_pin: DEFAULT_TX_PIN,
            uart_rx_pin: DEFAULT_RX_PIN,
        }
    }

    /// Fixe une IP statique (au lieu du DHCP par défaut).
    pub fn with_static_ip(
        mut self,
        ip: Ipv4Addr,
        gateway: Ipv4Addr,
        netmask: Ipv4Addr,
        dns: Ipv4Addr,
    ) -> Self {
        self.ip = IpMode::Static {
            ip,
            gateway,
            netmask,
            dns,
        };
        self
    }

    /// Renseigne le mot de passe WiFi (depuis env/NVS au bring-up).
    pub fn with_wifi_pass(mut self, pass: impl Into<String>) -> Self {
        self.wifi_pass = pass.into();
        self
    }

    /// Valide la configuration (à appeler au démarrage — échec = refus de boot).
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.node_name.is_empty() {
            return Err(ConfigError::EmptyNodeName);
        }
        if let Some(c) = self
            .node_name
            .chars()
            .find(|&c| c == '/' || c == '+' || c == '#' || c.is_whitespace())
        {
            return Err(ConfigError::InvalidNodeName(c));
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
        if let IpMode::Static {
            ip,
            gateway,
            netmask,
            ..
        } = self.ip
        {
            if ip.is_unspecified() || gateway.is_unspecified() || netmask == Ipv4Addr::UNSPECIFIED {
                return Err(ConfigError::InvalidStaticIp);
            }
        }
        Ok(())
    }

    /// Hostname WiFi = nom du nœud.
    pub fn hostname(&self) -> &str {
        &self.node_name
    }

    /// SSID de l'AP de secours (`<node_name>-setup`).
    pub fn ap_ssid(&self) -> String {
        format!("{}-setup", self.node_name)
    }

    /// Topics MQTT dérivés du nom du nœud.
    pub fn topics(&self) -> Topics {
        Topics::for_node(&self.node_name)
    }
}

/// Topics MQTT d'un nœud. Voir le schéma en tête de module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topics {
    base: String,
}

impl Topics {
    pub fn for_node(node_name: &str) -> Self {
        Self {
            base: format!("{TOPIC_ROOT}/{node_name}"),
        }
    }

    /// `santuario/toshiba/<node>`
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
    fn commissioning_defaults() {
        // Une seule ligne suffit à mettre en service une unité.
        let c = NodeConfig::new("Shorai-31");
        assert_eq!(c.wifi_ssid, "StarTh");
        assert_eq!(c.mqtt_host, "192.168.1.141");
        assert_eq!(c.mqtt_port, 1883);
        assert_eq!(c.uart_tx_pin, 33);
        assert_eq!(c.uart_rx_pin, 32);
        assert_eq!(c.ip, IpMode::Dhcp);
        assert!(c.ap_fallback);
        assert!(
            c.wifi_pass.is_empty(),
            "le mot de passe n'est PAS codé en dur (règle #12)"
        );
        assert!(c.validate().is_ok());
    }

    #[test]
    fn node_name_drives_hostname_topics_and_ap() {
        let c = NodeConfig::new("Shorai-32");
        assert_eq!(c.hostname(), "Shorai-32");
        assert_eq!(c.ap_ssid(), "Shorai-32-setup");
        let t = c.topics();
        assert_eq!(t.base(), "santuario/toshiba/Shorai-32");
        assert_eq!(t.state(), "santuario/toshiba/Shorai-32/state");
        assert_eq!(t.command(), "santuario/toshiba/Shorai-32/command");
        assert_eq!(t.availability(), "santuario/toshiba/Shorai-32/availability");
    }

    #[test]
    fn static_ip_configurable_and_validated() {
        let c = NodeConfig::new("Shorai-33").with_static_ip(
            Ipv4Addr::new(192, 168, 1, 33),
            Ipv4Addr::new(192, 168, 1, 1),
            Ipv4Addr::new(255, 255, 255, 0),
            Ipv4Addr::new(192, 168, 1, 1),
        );
        assert!(c.validate().is_ok());
        assert!(matches!(c.ip, IpMode::Static { .. }));

        // IP non spécifiée → rejetée.
        let bad = NodeConfig::new("Shorai-33").with_static_ip(
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::new(192, 168, 1, 1),
            Ipv4Addr::new(255, 255, 255, 0),
            Ipv4Addr::new(192, 168, 1, 1),
        );
        assert_eq!(bad.validate(), Err(ConfigError::InvalidStaticIp));
    }

    #[test]
    fn validation_rejects_bad_input() {
        let mut c = NodeConfig::new("Shorai-31");

        c.node_name = String::new();
        assert_eq!(c.validate(), Err(ConfigError::EmptyNodeName));

        c.node_name = "Shorai 31".to_string(); // espace interdit (hostname/topic)
        assert_eq!(c.validate(), Err(ConfigError::InvalidNodeName(' ')));

        c.node_name = "a/b".to_string();
        assert_eq!(c.validate(), Err(ConfigError::InvalidNodeName('/')));

        c.node_name = "Shorai-31".to_string();
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
    fn wildcard_in_node_name_is_rejected() {
        let mut c = NodeConfig::new("a#b");
        assert_eq!(c.validate(), Err(ConfigError::InvalidNodeName('#')));
        c.node_name = "a+b".to_string();
        assert_eq!(c.validate(), Err(ConfigError::InvalidNodeName('+')));
    }
}
