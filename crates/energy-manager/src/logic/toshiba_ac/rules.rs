//! Logique **pure** du module toshiba_ac — testable sans runtime.
//!
//! - **Parsing** de la télémétrie (lecture seule) : `parse_zone`, `snapshot_from_payload`.
//! - **Contrôle par présence** (opt-in `control_enabled`) : `decide_presence` +
//!   `presence_command_json`. Source de présence = capteurs **Aqara FP2** (un par pièce)
//!   publiant en MQTT. Politique : **ECO dès absence, OFF après N minutes d'absence**.
//!   Décision **pure et stateless** (modèle `logic/deye_command/rules.rs`).

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

// ---------------------------------------------------------------------------
// Contrôle par présence (Aqara FP2) — décision pure, stateless
// ---------------------------------------------------------------------------

// `#[allow(dead_code)]` : décision prête + testée, mais pas encore appelée — la
// boucle qui consomme la présence FP2 sera câblée quand les capteurs seront posés
// et leur topic MQTT connu (control_enabled). Cf. plan §10.4.

/// Action de climatisation déduite de la présence dans la pièce.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceAction {
    /// Pièce occupée → confort (restaurer marche/mode normal).
    Occupied,
    /// Absence courte → ECO (économie sans couper).
    Eco,
    /// Absence prolongée → OFF (arrêt).
    Off,
}

/// Décide l'action à partir de la présence. **Pure et stateless** :
/// - `present` = présence détectée à l'instant,
/// - `secs_since_presence` = temps écoulé depuis la dernière détection (0 si présent),
/// - `eco_after` ≤ `off_after` (garanti par la validation de config).
///
/// Politique : présent → `Occupied` ; absent ≥ `off_after` → `Off` ; absent ≥
/// `eco_after` → `Eco` ; sinon (délai de grâce avant ECO) → `Occupied`.
#[allow(dead_code)]
pub fn decide_presence(
    present: bool,
    secs_since_presence: u64,
    eco_after: u64,
    off_after: u64,
) -> PresenceAction {
    if present {
        PresenceAction::Occupied
    } else if secs_since_presence >= off_after {
        PresenceAction::Off
    } else if secs_since_presence >= eco_after {
        PresenceAction::Eco
    } else {
        PresenceAction::Occupied
    }
}

/// Traduit une [`PresenceAction`] en **payload JSON de commande** (schéma
/// `santuario/toshiba/<zone>/command`, cf. firmware `mqtt_payload`). L'appelant
/// n'émet **que sur changement** d'action (anti-rebattement).
#[allow(dead_code)]
pub fn presence_command_json(action: PresenceAction) -> &'static str {
    match action {
        // Retour de présence : rallumer + preset normal (annule ECO/OFF).
        PresenceAction::Occupied => r#"{"power":true,"preset":"standard"}"#,
        PresenceAction::Eco => r#"{"preset":"eco"}"#,
        PresenceAction::Off => r#"{"power":false}"#,
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

    // ---- Contrôle par présence (eco_after=0, off_after=300 par défaut) ----

    #[test]
    fn presence_present_is_occupied() {
        assert_eq!(decide_presence(true, 0, 0, 300), PresenceAction::Occupied);
        // Présent prime même si le compteur d'absence serait élevé.
        assert_eq!(decide_presence(true, 9999, 0, 300), PresenceAction::Occupied);
    }

    #[test]
    fn presence_eco_immediately_then_off_after_5min() {
        // ECO dès l'absence (eco_after=0).
        assert_eq!(decide_presence(false, 0, 0, 300), PresenceAction::Eco);
        assert_eq!(decide_presence(false, 299, 0, 300), PresenceAction::Eco);
        // OFF au seuil des 5 min.
        assert_eq!(decide_presence(false, 300, 0, 300), PresenceAction::Off);
        assert_eq!(decide_presence(false, 3600, 0, 300), PresenceAction::Off);
    }

    #[test]
    fn presence_grace_before_eco() {
        // Avec un délai de grâce eco_after=60 : reste Occupied avant 60 s.
        assert_eq!(decide_presence(false, 30, 60, 300), PresenceAction::Occupied);
        assert_eq!(decide_presence(false, 60, 60, 300), PresenceAction::Eco);
    }

    #[test]
    fn presence_command_payloads_are_valid_json() {
        for a in [PresenceAction::Occupied, PresenceAction::Eco, PresenceAction::Off] {
            let js = presence_command_json(a);
            // Doit être un JSON de commande parsable.
            let _: serde_json::Value = serde_json::from_str(js).unwrap();
        }
        assert_eq!(presence_command_json(PresenceAction::Eco), r#"{"preset":"eco"}"#);
        assert_eq!(presence_command_json(PresenceAction::Off), r#"{"power":false}"#);
    }
}
