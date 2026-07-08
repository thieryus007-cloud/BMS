//! Conversions **pures** entre l'état Toshiba (JSON MQTT) et le **cluster Thermostat
//! Matter**, et entre une **écriture Matter** et une **commande MQTT**. Aucune I/O →
//! entièrement testable sur host.
//!
//! Références : cluster Thermostat Matter (attributs `LocalTemperature`,
//! `OccupiedHeatingSetpoint`, `OccupiedCoolingSetpoint`, `SystemMode`) ; état Toshiba =
//! `santuario/toshiba/<zone>/state` (firmware ESP32, cf. `docs/toshiba-suzumi-rs-plan.md`
//! §6.6/§6.9). Températures Matter en **centi-degrés Celsius** (`int16`, ex. 21,0 °C → 2100).

use serde::Deserialize;

/// Valeur `int8` renvoyée par l'unité pour une température **invalide** (§6.9).
const TEMP_INVALID: i8 = 127;

/// Sous-ensemble de l'état publié par le firmware, utile au mapping Thermostat.
/// (Le reste — `fan`/`swing`/`preset`/`pwr_level`/`self_clean` — n'a pas d'équivalent
/// standard dans le cluster Thermostat ; il reste porté par MQTT/Grafana.)
#[derive(Debug, Default, Deserialize)]
pub struct ToshibaStatePayload {
    pub power: Option<bool>,
    pub mode: Option<String>,
    pub target_temp: Option<u8>,
    pub current_temp: Option<i8>,
}

/// `SystemMode` du cluster Thermostat Matter (valeurs de la spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MatterSystemMode {
    Off = 0,
    Auto = 1,
    Cool = 3,
    Heat = 4,
    FanOnly = 7,
    Dry = 8,
}

impl MatterSystemMode {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Off),
            1 => Some(Self::Auto),
            3 => Some(Self::Cool),
            4 => Some(Self::Heat),
            7 => Some(Self::FanOnly),
            8 => Some(Self::Dry),
            _ => None,
        }
    }

    /// Nom de mode Toshiba (§6.6) correspondant (hors `Off`, géré par `power`).
    fn toshiba_mode(self) -> &'static str {
        match self {
            Self::Off | Self::Auto => "heat_cool",
            Self::Cool => "cool",
            Self::Heat => "heat",
            Self::FanOnly => "fan_only",
            Self::Dry => "dry",
        }
    }
}

/// Attributs du cluster Thermostat à pousser côté Matter pour une zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThermostatAttrs {
    /// `LocalTemperature` (centi-°C) ; `None` si mesure absente/invalide (attribut null).
    pub local_temperature: Option<i16>,
    /// `OccupiedHeatingSetpoint` (centi-°C).
    pub occupied_heating_setpoint: Option<i16>,
    /// `OccupiedCoolingSetpoint` (centi-°C).
    pub occupied_cooling_setpoint: Option<i16>,
    /// `SystemMode` (`MatterSystemMode as u8`).
    pub system_mode: u8,
}

/// Température mesurée → `LocalTemperature` (centi-°C), `None` si invalide (127) ou absente.
pub fn local_temperature_centi(current_temp_c: Option<i8>) -> Option<i16> {
    match current_temp_c {
        Some(t) if t != TEMP_INVALID => Some(t as i16 * 100),
        _ => None,
    }
}

/// Consigne °C → centi-°C (`int16`). Plage Toshiba 5..30 °C → tient dans `i16`.
pub fn setpoint_centi(target_temp_c: Option<u8>) -> Option<i16> {
    target_temp_c.map(|t| t as i16 * 100)
}

/// `SystemMode` déduit de `power` + `mode`. `power=false` prime (`Off`). Mode inconnu ou
/// absent (mais allumé) → `Auto` (le plus sûr : ne coupe pas, laisse la clim gérer).
pub fn system_mode(power: Option<bool>, mode: Option<&str>) -> MatterSystemMode {
    if power == Some(false) {
        return MatterSystemMode::Off;
    }
    match mode {
        Some("cool") => MatterSystemMode::Cool,
        Some("heat") => MatterSystemMode::Heat,
        Some("dry") => MatterSystemMode::Dry,
        Some("fan_only") => MatterSystemMode::FanOnly,
        Some("heat_cool") => MatterSystemMode::Auto,
        _ => MatterSystemMode::Auto,
    }
}

/// État Toshiba → attributs Thermostat Matter. La clim n'a **qu'une seule** consigne :
/// on la miroite sur les deux setpoints (chauffe/froid) — le contrôleur lit celui qui
/// correspond au `SystemMode`.
pub fn attrs_from_state(p: &ToshibaStatePayload) -> ThermostatAttrs {
    let setpoint = setpoint_centi(p.target_temp);
    ThermostatAttrs {
        local_temperature: local_temperature_centi(p.current_temp),
        occupied_heating_setpoint: setpoint,
        occupied_cooling_setpoint: setpoint,
        system_mode: system_mode(p.power, p.mode.as_deref()).as_u8(),
    }
}

/// Écriture reçue côté Matter (à traduire en commande MQTT Toshiba).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatterWrite {
    /// `SystemMode` écrit par le contrôleur.
    SystemMode(u8),
    /// `OccupiedHeatingSetpoint` ou `OccupiedCoolingSetpoint` (centi-°C) — même consigne
    /// unique côté Toshiba dans les deux cas.
    Setpoint(i16),
}

/// Écriture Matter → **payload JSON** de commande (`santuario/toshiba/<zone>/command`,
/// schéma firmware `mqtt_payload`). `None` si l'écriture n'est pas exploitable.
pub fn command_json(w: MatterWrite) -> Option<String> {
    match w {
        MatterWrite::SystemMode(v) => {
            let m = MatterSystemMode::from_u8(v)?;
            let js = if m == MatterSystemMode::Off {
                r#"{"power":false}"#.to_string()
            } else {
                format!(r#"{{"power":true,"mode":"{}"}}"#, m.toshiba_mode())
            };
            Some(js)
        }
        MatterWrite::Setpoint(centi) => {
            // centi-°C → °C entier (la clim ne prend pas de demi-degrés, §6.11).
            let deg = (centi as f64 / 100.0).round() as i64;
            Some(format!(r#"{{"target_temp":{deg}}}"#))
        }
    }
}

// ---------------------------------------------------------------------------
// Présence FP2 → cluster Occupancy Sensor Matter
// ---------------------------------------------------------------------------
//
// Permet au bridge d'exposer AUSSI la présence (un endpoint Occupancy Sensor par pièce),
// ce qui rend le pont HomeKit `mqtt-homekit-occupancy` (D) redondant : Apple Home étant un
// contrôleur Matter, l'Occupancy apparaît dans l'app Maison **et** dans Homey/Google.
// Cf. `docs/toshiba-bridges.md` §5 (« E supersède D »).

/// Payload de présence publié par le pont FP2 (`bridge/aqara-fp2-mqtt`) sur
/// `santuario/toshiba/presence/<zone>` : `{"present": bool, "ts": epoch}` (`ts` ignoré).
#[derive(Debug, Deserialize)]
pub struct PresencePayload {
    pub present: bool,
}

/// Présence → attribut `Occupancy` du cluster Occupancy Sensor Matter (**bitmap8**,
/// bit0 = occupé) : `1` si présent, `0` sinon.
pub fn occupancy_bitmap(present: bool) -> u8 {
    u8::from(present)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_temp_handles_invalid() {
        assert_eq!(local_temperature_centi(Some(21)), Some(2100));
        assert_eq!(local_temperature_centi(Some(-3)), Some(-300));
        assert_eq!(local_temperature_centi(Some(127)), None); // invalide
        assert_eq!(local_temperature_centi(None), None);
    }

    #[test]
    fn setpoint_conversion() {
        assert_eq!(setpoint_centi(Some(24)), Some(2400));
        assert_eq!(setpoint_centi(Some(8)), Some(800)); // mode 8°
        assert_eq!(setpoint_centi(None), None);
    }

    #[test]
    fn system_mode_from_power_and_mode() {
        assert_eq!(system_mode(Some(false), Some("cool")), MatterSystemMode::Off);
        assert_eq!(system_mode(Some(true), Some("cool")), MatterSystemMode::Cool);
        assert_eq!(system_mode(Some(true), Some("heat")), MatterSystemMode::Heat);
        assert_eq!(system_mode(Some(true), Some("dry")), MatterSystemMode::Dry);
        assert_eq!(system_mode(Some(true), Some("fan_only")), MatterSystemMode::FanOnly);
        assert_eq!(system_mode(Some(true), Some("heat_cool")), MatterSystemMode::Auto);
        // Inconnu / absent mais allumé → Auto.
        assert_eq!(system_mode(Some(true), Some("weird")), MatterSystemMode::Auto);
        assert_eq!(system_mode(None, None), MatterSystemMode::Auto);
    }

    #[test]
    fn attrs_from_full_state() {
        let p = ToshibaStatePayload {
            power: Some(true),
            mode: Some("cool".to_string()),
            target_temp: Some(24),
            current_temp: Some(21),
        };
        let a = attrs_from_state(&p);
        assert_eq!(a.local_temperature, Some(2100));
        assert_eq!(a.occupied_cooling_setpoint, Some(2400));
        assert_eq!(a.occupied_heating_setpoint, Some(2400)); // consigne unique miroitée
        assert_eq!(a.system_mode, MatterSystemMode::Cool.as_u8());
    }

    #[test]
    fn attrs_when_off() {
        let p = ToshibaStatePayload { power: Some(false), ..Default::default() };
        assert_eq!(attrs_from_state(&p).system_mode, MatterSystemMode::Off.as_u8());
    }

    #[test]
    fn command_from_system_mode_writes() {
        assert_eq!(command_json(MatterWrite::SystemMode(0)).unwrap(), r#"{"power":false}"#);
        assert_eq!(
            command_json(MatterWrite::SystemMode(3)).unwrap(),
            r#"{"power":true,"mode":"cool"}"#
        );
        assert_eq!(
            command_json(MatterWrite::SystemMode(4)).unwrap(),
            r#"{"power":true,"mode":"heat"}"#
        );
        assert_eq!(
            command_json(MatterWrite::SystemMode(1)).unwrap(),
            r#"{"power":true,"mode":"heat_cool"}"#
        );
        // Valeur SystemMode non supportée → None.
        assert_eq!(command_json(MatterWrite::SystemMode(9)), None);
    }

    #[test]
    fn command_from_setpoint_rounds_to_integer() {
        assert_eq!(command_json(MatterWrite::Setpoint(2400)).unwrap(), r#"{"target_temp":24}"#);
        // 21,6 °C → 22.
        assert_eq!(command_json(MatterWrite::Setpoint(2160)).unwrap(), r#"{"target_temp":22}"#);
    }

    #[test]
    fn command_payloads_are_valid_json() {
        for w in [
            MatterWrite::SystemMode(0),
            MatterWrite::SystemMode(4),
            MatterWrite::Setpoint(2100),
        ] {
            let js = command_json(w).unwrap();
            let _: serde_json::Value = serde_json::from_str(&js).unwrap();
        }
    }

    #[test]
    fn presence_maps_to_occupancy() {
        assert_eq!(occupancy_bitmap(true), 1);
        assert_eq!(occupancy_bitmap(false), 0);
        let p: PresencePayload = serde_json::from_str(r#"{"present":true,"ts":1}"#).unwrap();
        assert!(p.present);
        let p: PresencePayload = serde_json::from_str(r#"{"present":false}"#).unwrap();
        assert!(!p.present);
    }
}
