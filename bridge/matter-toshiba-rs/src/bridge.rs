//! **Orchestration pure** du bridge — transport-agnostique (aucune dépendance
//! rs-matter/MQTT) → testable de bout en bout sur host.
//!
//! La couche transport (feature `matter`) se contente d'appeler :
//! - [`Bridge::on_state_json`] à chaque message MQTT `…/<zone>/state` → pousse les
//!   [`ThermostatAttrs`] retournés dans le cluster Thermostat de l'endpoint de la zone ;
//! - [`Bridge::on_matter_write`] à chaque écriture d'attribut Matter → publie le JSON
//!   retourné sur `config::command_topic(zone)`.

use std::collections::HashMap;

use crate::mapping::{attrs_from_state, command_json, MatterWrite, ThermostatAttrs, ToshibaStatePayload};

/// Cache par zone du dernier état projeté en attributs Matter.
#[derive(Default)]
pub struct Bridge {
    zones: HashMap<String, ThermostatAttrs>,
}

impl Bridge {
    pub fn new() -> Self {
        Self::default()
    }

    /// Traite un message d'état MQTT (`santuario/toshiba/<zone>/state`). Renvoie les
    /// attributs Thermostat à pousser côté Matter, ou `None` si le JSON est invalide
    /// (message ignoré, cache inchangé).
    pub fn on_state_json(&mut self, zone: &str, json: &[u8]) -> Option<ThermostatAttrs> {
        let payload: ToshibaStatePayload = serde_json::from_slice(json).ok()?;
        let attrs = attrs_from_state(&payload);
        self.zones.insert(zone.to_string(), attrs.clone());
        Some(attrs)
    }

    /// Dernier état connu d'une zone (pour initialiser l'endpoint Matter au démarrage).
    pub fn attrs(&self, zone: &str) -> Option<&ThermostatAttrs> {
        self.zones.get(zone)
    }

    /// Traduit une écriture d'attribut Matter en **payload de commande** MQTT. Le topic
    /// complet est construit par le transport via `config::command_topic(zone)`.
    pub fn on_matter_write(&self, write: MatterWrite) -> Option<String> {
        command_json(write)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::MatterSystemMode;

    #[test]
    fn state_message_updates_cache_and_maps() {
        let mut b = Bridge::new();
        let attrs = b
            .on_state_json(
                "Shorai-31",
                br#"{"power":true,"mode":"heat","target_temp":22,"current_temp":19}"#,
            )
            .unwrap();
        assert_eq!(attrs.system_mode, MatterSystemMode::Heat.as_u8());
        assert_eq!(attrs.local_temperature, Some(1900));
        assert_eq!(attrs.occupied_heating_setpoint, Some(2200));
        // Mis en cache.
        assert_eq!(b.attrs("Shorai-31"), Some(&attrs));
        assert!(b.attrs("Shorai-99").is_none());
    }

    #[test]
    fn invalid_json_is_ignored() {
        let mut b = Bridge::new();
        assert!(b.on_state_json("Shorai-31", b"pas du json").is_none());
        assert!(b.attrs("Shorai-31").is_none());
    }

    #[test]
    fn matter_write_produces_command() {
        let b = Bridge::new();
        assert_eq!(
            b.on_matter_write(MatterWrite::SystemMode(MatterSystemMode::Off.as_u8()))
                .unwrap(),
            r#"{"power":false}"#
        );
        assert_eq!(
            b.on_matter_write(MatterWrite::Setpoint(2300)).unwrap(),
            r#"{"target_temp":23}"#
        );
    }
}
