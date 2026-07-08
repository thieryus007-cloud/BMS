//! Config du bridge : zones exposées, broker MQTT, paramètres de **commissioning
//! Matter**. Pur + testé (aucune I/O).

/// Passcodes de setup Matter **interdits** par la spec (trop faibles / motifs).
const FORBIDDEN_PASSCODES: [u32; 12] = [
    0, 11111111, 22222222, 33333333, 44444444, 55555555, 66666666, 77777777, 88888888,
    99999999, 12345678, 87654321,
];

/// Racine locale des topics Toshiba (non bridgée vers le NanoPi — règle projet #11).
pub const TOPIC_ROOT: &str = "santuario/toshiba";

#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Zones = nœuds Toshiba exposés, un **endpoint Thermostat** chacun (`Shorai-31`, …).
    pub zones: Vec<String>,
    pub mqtt_host: String,
    pub mqtt_port: u16,
    /// Discriminateur de commissioning Matter (12 bits : 0..=4095).
    pub discriminator: u16,
    /// Passcode de setup Matter (à saisir/scanner à l'appairage). **Non commité en prod.**
    pub passcode: u32,
    /// Identifiants Matter (par défaut : plage **test** `0xFFF1` / `0x8000`).
    pub vendor_id: u16,
    pub product_id: u16,
}

impl NodeConfig {
    /// Défauts de mise en service : broker Pi5 local, valeurs de commissioning **de test**
    /// (à remplacer avant toute production certifiée — cf. README).
    pub fn new(zones: Vec<String>) -> Self {
        Self {
            zones,
            mqtt_host: "127.0.0.1".to_string(),
            mqtt_port: 1883,
            discriminator: 3840,
            passcode: 20202021,
            vendor_id: 0xFFF1,
            product_id: 0x8000,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.zones.is_empty() {
            return Err("aucune zone configurée".to_string());
        }
        for z in &self.zones {
            if z.is_empty() || z.chars().any(|c| "/+# ".contains(c)) {
                return Err(format!("zone invalide (segment de topic) : {z:?}"));
            }
        }
        if self.zones.iter().collect::<std::collections::HashSet<_>>().len() != self.zones.len() {
            return Err("zones dupliquées".to_string());
        }
        if self.mqtt_host.trim().is_empty() {
            return Err("mqtt_host vide".to_string());
        }
        if self.discriminator > 4095 {
            return Err(format!("discriminator hors plage 0..=4095 : {}", self.discriminator));
        }
        if !(1..=99_999_998).contains(&self.passcode) || FORBIDDEN_PASSCODES.contains(&self.passcode)
        {
            return Err(format!("passcode Matter invalide (interdit par la spec) : {}", self.passcode));
        }
        Ok(())
    }
}

/// `santuario/toshiba/<zone>/state` (souscrit).
pub fn state_topic(zone: &str) -> String {
    format!("{TOPIC_ROOT}/{zone}/state")
}

/// `santuario/toshiba/<zone>/command` (publié, non-retained).
pub fn command_topic(zone: &str) -> String {
    format!("{TOPIC_ROOT}/{zone}/command")
}

/// Extrait `<zone>` d'un topic `santuario/toshiba/<zone>/state` (souscription wildcard).
pub fn zone_from_state_topic(topic: &str) -> Option<&str> {
    topic
        .strip_prefix("santuario/toshiba/")
        .and_then(|s| s.strip_suffix("/state"))
        .filter(|z| !z.is_empty() && !z.contains('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> NodeConfig {
        NodeConfig::new(vec!["Shorai-31".into(), "Shorai-32".into()])
    }

    #[test]
    fn valid_default_config() {
        assert!(cfg().validate().is_ok());
    }

    #[test]
    fn rejects_bad_zones() {
        assert!(NodeConfig::new(vec![]).validate().is_err());
        assert!(NodeConfig::new(vec!["a/b".into()]).validate().is_err());
        assert!(NodeConfig::new(vec!["x".into(), "x".into()]).validate().is_err()); // doublon
    }

    #[test]
    fn rejects_bad_matter_params() {
        let mut c = cfg();
        c.discriminator = 5000;
        assert!(c.validate().is_err());

        let mut c = cfg();
        c.passcode = 12345678; // interdit
        assert!(c.validate().is_err());

        let mut c = cfg();
        c.passcode = 0; // interdit
        assert!(c.validate().is_err());
    }

    #[test]
    fn topic_helpers() {
        assert_eq!(state_topic("Shorai-31"), "santuario/toshiba/Shorai-31/state");
        assert_eq!(command_topic("Shorai-31"), "santuario/toshiba/Shorai-31/command");
        assert_eq!(
            zone_from_state_topic("santuario/toshiba/Shorai-31/state"),
            Some("Shorai-31")
        );
        assert_eq!(zone_from_state_topic("santuario/toshiba/Shorai-31/command"), None);
        assert_eq!(zone_from_state_topic("santuario/toshiba/presence/Shorai-31"), None);
    }
}
