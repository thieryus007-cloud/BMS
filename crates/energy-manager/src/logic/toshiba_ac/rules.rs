//! Logique **pure** du module toshiba_ac (parsing) — testable sans runtime.
//!
//! Phase actuelle = **lecture seule** : pas de décision de contrôle. Le futur
//! pilotage (effacement/solaire, opt-in `control_enabled`) viendra ici sous forme
//! de fonctions pures + tests, sur le modèle de `logic/deye_command/rules.rs`.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::types::ToshibaAcSnapshot;

/// Extrait la `zone` d'un topic `santuario/toshiba/<zone>/state`.
/// Renvoie `None` pour tout autre topic (le module ne traite que celui-ci).
pub fn parse_zone(topic: &str) -> Option<&str> {
    let mut it = topic.split('/');
    match (it.next(), it.next(), it.next(), it.next(), it.next()) {
        (Some("santuario"), Some("toshiba"), Some(zone), Some("state"), None)
            if !zone.is_empty() =>
        {
            Some(zone)
        }
        _ => None,
    }
}

/// Payload JSON d'état publié par le firmware ESP32 (`mqtt_payload::state_to_json`).
/// Tous les champs sont optionnels (l'unité les renseigne au fil de l'eau).
#[derive(Debug, Deserialize)]
pub struct ToshibaStatePayload {
    pub power: Option<bool>,
    pub mode: Option<String>,
    pub target_temp: Option<u8>,
    pub current_temp: Option<i8>,
    pub outdoor_temp: Option<i8>,
    pub fan: Option<String>,
    pub swing: Option<String>,
    pub preset: Option<String>,
    pub pwr_level: Option<u8>,
    #[serde(default)]
    pub self_clean: bool,
}

/// Convertit un payload reçu en instantané stocké dans `EnergyState`.
pub fn snapshot_from_payload(p: ToshibaStatePayload, now: DateTime<Utc>) -> ToshibaAcSnapshot {
    ToshibaAcSnapshot {
        power: p.power,
        mode: p.mode,
        target_temp_c: p.target_temp,
        current_temp_c: p.current_temp,
        outdoor_temp_c: p.outdoor_temp,
        fan: p.fan,
        swing: p.swing,
        preset: p.preset,
        pwr_level_pct: p.pwr_level,
        self_clean: p.self_clean,
        last_update: Some(now),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_zone_accepts_state_topic() {
        assert_eq!(parse_zone("santuario/toshiba/salon/state"), Some("salon"));
        assert_eq!(parse_zone("santuario/toshiba/chambre/state"), Some("chambre"));
    }

    #[test]
    fn parse_zone_rejects_other_topics() {
        assert_eq!(parse_zone("santuario/toshiba/salon/command"), None);
        assert_eq!(parse_zone("santuario/toshiba/salon/state/extra"), None);
        assert_eq!(parse_zone("santuario/toshiba//state"), None); // zone vide
        assert_eq!(parse_zone("stat/tongou/POWER"), None);
        assert_eq!(parse_zone("santuario/toshiba/salon"), None);
    }

    #[test]
    fn payload_maps_to_snapshot() {
        let json = r#"{"power":true,"mode":"cool","target_temp":24,"current_temp":21,
            "outdoor_temp":8,"fan":"auto","swing":"vertical","preset":"standard",
            "pwr_level":100,"self_clean":false}"#;
        let p: ToshibaStatePayload = serde_json::from_str(json).unwrap();
        let now = Utc::now();
        let s = snapshot_from_payload(p, now);
        assert_eq!(s.power, Some(true));
        assert_eq!(s.mode.as_deref(), Some("cool"));
        assert_eq!(s.target_temp_c, Some(24));
        assert_eq!(s.current_temp_c, Some(21));
        assert_eq!(s.outdoor_temp_c, Some(8));
        assert_eq!(s.pwr_level_pct, Some(100));
        assert!(!s.self_clean);
        assert_eq!(s.last_update, Some(now));
    }

    #[test]
    fn partial_payload_is_accepted() {
        // L'unité peut n'avoir renseigné que quelques champs.
        let p: ToshibaStatePayload = serde_json::from_str(r#"{"mode":"off"}"#).unwrap();
        let s = snapshot_from_payload(p, Utc::now());
        assert_eq!(s.mode.as_deref(), Some("off"));
        assert_eq!(s.target_temp_c, None);
        assert!(!s.self_clean);
    }
}
