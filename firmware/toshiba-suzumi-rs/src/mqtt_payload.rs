//! Couche MQTT **applicative** : conversion état ↔ JSON et commande JSON ↔ typée.
//!
//! **Transport-agnostique** : ce module ne connaît **ni broker ni topics** (le
//! schéma de topics `santuario/toshiba/<zone>/...` sera figé au branchement réel,
//! cf. `docs/integration-toshiba-shorai-esphome.md` §5.2). Il ne fait que la
//! (dé)sérialisation → **entièrement testable sur host**.
//!
//! - [`state_to_json`] : télémétrie ([`ToshibaState`] → JSON stable).
//! - [`parse_command`] : JSON de commande → `Vec<`[`Command`]`>` à appliquer via
//!   [`crate::machine::Client::apply_command`].

use serde::{Deserialize, Serialize};

use crate::protocol::{Fan, Mode, PwrLevel, SpecialMode, State, Swing};
use crate::state::ToshibaState;

// ============================================================================
// Erreurs
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadError {
    /// JSON invalide.
    Json(String),
    /// Valeur d'énumération inconnue pour un champ.
    UnknownValue { field: &'static str, value: String },
}

fn unknown(field: &'static str, value: &str) -> PayloadError {
    PayloadError::UnknownValue {
        field,
        value: value.to_string(),
    }
}

// ============================================================================
// Correspondances chaîne ↔ enum (noms stables, snake_case)
// ============================================================================

pub fn mode_to_str(m: Mode) -> &'static str {
    match m {
        Mode::HeatCool => "heat_cool",
        Mode::Cool => "cool",
        Mode::Heat => "heat",
        Mode::Dry => "dry",
        Mode::FanOnly => "fan_only",
    }
}

fn str_to_mode(s: &str) -> Option<Mode> {
    Some(match () {
        _ if s.eq_ignore_ascii_case("heat_cool") => Mode::HeatCool,
        _ if s.eq_ignore_ascii_case("cool") => Mode::Cool,
        _ if s.eq_ignore_ascii_case("heat") => Mode::Heat,
        _ if s.eq_ignore_ascii_case("dry") => Mode::Dry,
        _ if s.eq_ignore_ascii_case("fan_only") => Mode::FanOnly,
        _ => return None,
    })
}

pub fn fan_to_str(f: Fan) -> &'static str {
    match f {
        Fan::Quiet => "quiet",
        Fan::Low => "low",
        Fan::Level2 => "low_medium",
        Fan::Medium => "medium",
        Fan::Level4 => "medium_high",
        Fan::High => "high",
        Fan::Auto => "auto",
    }
}

fn str_to_fan(s: &str) -> Option<Fan> {
    Some(match () {
        _ if s.eq_ignore_ascii_case("quiet") => Fan::Quiet,
        _ if s.eq_ignore_ascii_case("low") => Fan::Low,
        _ if s.eq_ignore_ascii_case("low_medium") => Fan::Level2,
        _ if s.eq_ignore_ascii_case("medium") => Fan::Medium,
        _ if s.eq_ignore_ascii_case("medium_high") => Fan::Level4,
        _ if s.eq_ignore_ascii_case("high") => Fan::High,
        _ if s.eq_ignore_ascii_case("auto") => Fan::Auto,
        _ => return None,
    })
}

pub fn swing_to_str(s: Swing) -> &'static str {
    match s {
        Swing::Off => "off",
        Swing::Vertical => "vertical",
        Swing::Horizontal => "horizontal",
        Swing::Both => "both",
        Swing::VFix1 => "pos1",
        Swing::VFix2 => "pos2",
        Swing::VFix3 => "pos3",
        Swing::VFix4 => "pos4",
        Swing::VFix5 => "pos5",
    }
}

fn str_to_swing(s: &str) -> Option<Swing> {
    Some(match () {
        _ if s.eq_ignore_ascii_case("off") => Swing::Off,
        _ if s.eq_ignore_ascii_case("vertical") => Swing::Vertical,
        _ if s.eq_ignore_ascii_case("horizontal") => Swing::Horizontal,
        _ if s.eq_ignore_ascii_case("both") => Swing::Both,
        _ if s.eq_ignore_ascii_case("pos1") => Swing::VFix1,
        _ if s.eq_ignore_ascii_case("pos2") => Swing::VFix2,
        _ if s.eq_ignore_ascii_case("pos3") => Swing::VFix3,
        _ if s.eq_ignore_ascii_case("pos4") => Swing::VFix4,
        _ if s.eq_ignore_ascii_case("pos5") => Swing::VFix5,
        _ => return None,
    })
}

pub fn preset_to_str(p: SpecialMode) -> &'static str {
    match p {
        SpecialMode::Standard => "standard",
        SpecialMode::HiPower => "hi_power",
        SpecialMode::Silent1 => "silent_1",
        SpecialMode::Eco => "eco",
        SpecialMode::EightDeg => "eight_deg",
        SpecialMode::Sleep => "sleep",
        SpecialMode::Floor => "floor",
        SpecialMode::Comfort => "comfort",
        SpecialMode::Silent2 => "silent_2",
        SpecialMode::Fireplace1 => "fireplace_1",
        SpecialMode::Fireplace2 => "fireplace_2",
    }
}

fn str_to_preset(s: &str) -> Option<SpecialMode> {
    Some(match () {
        _ if s.eq_ignore_ascii_case("standard") => SpecialMode::Standard,
        _ if s.eq_ignore_ascii_case("hi_power") => SpecialMode::HiPower,
        _ if s.eq_ignore_ascii_case("silent_1") => SpecialMode::Silent1,
        _ if s.eq_ignore_ascii_case("eco") => SpecialMode::Eco,
        _ if s.eq_ignore_ascii_case("eight_deg") => SpecialMode::EightDeg,
        _ if s.eq_ignore_ascii_case("sleep") => SpecialMode::Sleep,
        _ if s.eq_ignore_ascii_case("floor") => SpecialMode::Floor,
        _ if s.eq_ignore_ascii_case("comfort") => SpecialMode::Comfort,
        _ if s.eq_ignore_ascii_case("silent_2") => SpecialMode::Silent2,
        _ if s.eq_ignore_ascii_case("fireplace_1") => SpecialMode::Fireplace1,
        _ if s.eq_ignore_ascii_case("fireplace_2") => SpecialMode::Fireplace2,
        _ => return None,
    })
}

// ============================================================================
// Télémétrie : ToshibaState → JSON
// ============================================================================

#[derive(Serialize)]
struct StatePayload<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    power: Option<bool>,
    /// Mode « effectif » : `"off"` si l'unité est éteinte, sinon le mode CVC.
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_temp: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_temp: Option<i8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outdoor_temp: Option<i8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fan: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    swing: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preset: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pwr_level: Option<u8>,
    self_clean: bool,
}

/// Sérialise l'état courant en JSON (télémétrie). Les champs encore inconnus
/// (`None`) sont omis.
pub fn state_to_json(s: &ToshibaState) -> String {
    let mode = if s.power == Some(State::Off) {
        Some("off")
    } else {
        s.mode.map(mode_to_str)
    };
    let payload = StatePayload {
        power: s.power.map(|p| p == State::On),
        mode,
        target_temp: s.target_temp,
        current_temp: s.room_temp,
        outdoor_temp: s.outdoor_temp,
        fan: s.fan.map(fan_to_str),
        swing: s.swing.map(swing_to_str),
        preset: s.special_mode.map(preset_to_str),
        pwr_level: s.pwr_level.map(|p| p as u8),
        self_clean: s.self_clean,
    };
    // Sérialisation d'une struct simple : ne peut pas échouer en pratique.
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
}

// ============================================================================
// Commandes : JSON → Vec<Command>
// ============================================================================

/// Commande typée, appliquée via [`crate::machine::Client::apply_command`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    SetPower(bool),
    SetMode(Mode),
    SetTargetTemp(u8),
    SetFan(Fan),
    SetSwing(Swing),
    SetPreset(SpecialMode),
    SetPowerLevel(PwrLevel),
}

#[derive(Deserialize, Default)]
struct CommandJson {
    power: Option<bool>,
    mode: Option<String>,
    target_temp: Option<u8>,
    fan: Option<String>,
    swing: Option<String>,
    preset: Option<String>,
    power_level: Option<u8>,
}

/// Parse un JSON de commande en une liste ordonnée de [`Command`].
///
/// `mode: "off"` est traduit en `SetPower(false)`. Un champ absent est ignoré ;
/// une valeur d'énum inconnue renvoie [`PayloadError::UnknownValue`].
pub fn parse_command(json: &str) -> Result<Vec<Command>, PayloadError> {
    let c: CommandJson =
        serde_json::from_str(json).map_err(|e| PayloadError::Json(e.to_string()))?;
    let mut out = Vec::new();
    if let Some(p) = c.power {
        out.push(Command::SetPower(p));
    }
    if let Some(m) = c.mode {
        if m.eq_ignore_ascii_case("off") {
            out.push(Command::SetPower(false));
        } else {
            out.push(Command::SetMode(
                str_to_mode(&m).ok_or_else(|| unknown("mode", &m))?,
            ));
        }
    }
    if let Some(t) = c.target_temp {
        out.push(Command::SetTargetTemp(t));
    }
    if let Some(f) = c.fan {
        out.push(Command::SetFan(
            str_to_fan(&f).ok_or_else(|| unknown("fan", &f))?,
        ));
    }
    if let Some(sw) = c.swing {
        out.push(Command::SetSwing(
            str_to_swing(&sw).ok_or_else(|| unknown("swing", &sw))?,
        ));
    }
    if let Some(pr) = c.preset {
        out.push(Command::SetPreset(
            str_to_preset(&pr).ok_or_else(|| unknown("preset", &pr))?,
        ));
    }
    if let Some(pl) = c.power_level {
        out.push(Command::SetPowerLevel(
            PwrLevel::from_u8(pl).ok_or_else(|| unknown("power_level", &pl.to_string()))?,
        ));
    }
    Ok(out)
}

// ============================================================================
// Tests (host)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn state_serializes_expected_fields() {
        let mut s = ToshibaState::new();
        s.power = Some(State::On);
        s.mode = Some(Mode::Cool);
        s.target_temp = Some(24);
        s.room_temp = Some(21);
        s.fan = Some(Fan::Auto);
        let v: Value = serde_json::from_str(&state_to_json(&s)).unwrap();
        assert_eq!(v["power"], true);
        assert_eq!(v["mode"], "cool");
        assert_eq!(v["target_temp"], 24);
        assert_eq!(v["current_temp"], 21);
        assert_eq!(v["fan"], "auto");
        assert_eq!(v["self_clean"], false);
        // Champs inconnus omis.
        assert!(v.get("outdoor_temp").is_none());
        assert!(v.get("preset").is_none());
    }

    #[test]
    fn mode_is_off_when_powered_off() {
        let mut s = ToshibaState::new();
        s.power = Some(State::Off);
        s.mode = Some(Mode::Heat); // ignoré au profit de "off"
        let v: Value = serde_json::from_str(&state_to_json(&s)).unwrap();
        assert_eq!(v["mode"], "off");
        assert_eq!(v["power"], false);
    }

    #[test]
    fn parses_a_full_command() {
        let cmds = parse_command(
            r#"{"power":true,"mode":"heat","target_temp":22,"fan":"auto","swing":"off","preset":"eco","power_level":75}"#,
        )
        .unwrap();
        assert_eq!(
            cmds,
            vec![
                Command::SetPower(true),
                Command::SetMode(Mode::Heat),
                Command::SetTargetTemp(22),
                Command::SetFan(Fan::Auto),
                Command::SetSwing(Swing::Off),
                Command::SetPreset(SpecialMode::Eco),
                Command::SetPowerLevel(PwrLevel::Pct75),
            ]
        );
    }

    #[test]
    fn mode_off_becomes_set_power_false() {
        let cmds = parse_command(r#"{"mode":"off"}"#).unwrap();
        assert_eq!(cmds, vec![Command::SetPower(false)]);
    }

    #[test]
    fn empty_object_yields_no_commands() {
        assert_eq!(parse_command("{}").unwrap(), vec![]);
    }

    #[test]
    fn unknown_enum_value_errors() {
        let err = parse_command(r#"{"mode":"turbo"}"#).unwrap_err();
        assert_eq!(
            err,
            PayloadError::UnknownValue {
                field: "mode",
                value: "turbo".to_string()
            }
        );
    }

    #[test]
    fn invalid_json_errors() {
        assert!(matches!(
            parse_command("not json"),
            Err(PayloadError::Json(_))
        ));
    }

    #[test]
    fn string_maps_roundtrip() {
        assert_eq!(str_to_mode(mode_to_str(Mode::Dry)), Some(Mode::Dry));
        assert_eq!(str_to_fan(fan_to_str(Fan::Level2)), Some(Fan::Level2));
        assert_eq!(str_to_swing(swing_to_str(Swing::VFix5)), Some(Swing::VFix5));
        assert_eq!(
            str_to_preset(preset_to_str(SpecialMode::Fireplace2)),
            Some(SpecialMode::Fireplace2)
        );
    }
}
