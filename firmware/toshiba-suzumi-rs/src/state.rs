//! Agrégat d'état de l'unité (image courante), alimenté par les [`Field`] parsés.
//!
//! Pur, sans I/O — testable sur host. Voir `docs/toshiba-suzumi-rs-plan.md` §6.9.

use crate::protocol::{
    Fan, Field, IduStatus, Mode, OduStatus, PwrLevel, SelfCleanState, SpecialMode, State, Swing,
};

/// Image courante de l'unité intérieure. Chaque champ est `Option` tant qu'il
/// n'a pas été reçu au moins une fois.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ToshibaState {
    pub power: Option<State>,
    pub mode: Option<Mode>,
    /// Consigne en °C, déjà « dé-offsetée » (mode 8° géré à la réception).
    pub target_temp: Option<u8>,
    pub room_temp: Option<i8>,
    pub outdoor_temp: Option<i8>,
    pub fan: Option<Fan>,
    pub swing: Option<Swing>,
    pub pwr_level: Option<PwrLevel>,
    pub special_mode: Option<SpecialMode>,
    pub self_clean: bool,
    pub odu: Option<OduStatus>,
    pub idu: Option<IduStatus>,
}

impl ToshibaState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Le mode 8° (garde hors-gel) est-il actif ? Sert au décalage `±16` de la
    /// consigne lors du parsing (§6.11).
    pub fn eight_deg_active(&self) -> bool {
        self.special_mode == Some(SpecialMode::EightDeg)
    }

    /// L'unité est-elle en marche ? (`None` si l'état n'est pas encore connu.)
    pub fn is_on(&self) -> bool {
        self.power == Some(State::On)
    }

    /// Applique un champ parsé à l'état. Les trames `Ack`/`Unknown` sont ignorées.
    pub fn apply(&mut self, field: Field) {
        match field {
            Field::Power(s) => self.power = Some(s),
            Field::Mode(m) => self.mode = Some(m),
            Field::TargetTemp(t) => self.target_temp = Some(t),
            Field::RoomTemp(t) => self.room_temp = t,
            Field::OutdoorTemp(t) => self.outdoor_temp = t,
            Field::Fan(f) => self.fan = Some(f),
            Field::Swing(s) => self.swing = Some(s),
            Field::PwrLevel(p) => self.pwr_level = Some(p),
            Field::SpecialMode(sp) => self.special_mode = Some(sp),
            Field::SelfClean(sc) => self.self_clean = sc == SelfCleanState::Running,
            Field::Odu(o) => self.odu = Some(o),
            Field::Idu(i) => self.idu = Some(i),
            Field::Ack | Field::Unknown { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_updates_fields() {
        let mut s = ToshibaState::new();
        assert!(!s.is_on());
        s.apply(Field::Power(State::On));
        s.apply(Field::Mode(Mode::Heat));
        s.apply(Field::RoomTemp(Some(21)));
        s.apply(Field::SpecialMode(SpecialMode::EightDeg));
        assert!(s.is_on());
        assert_eq!(s.mode, Some(Mode::Heat));
        assert_eq!(s.room_temp, Some(21));
        assert!(s.eight_deg_active());
    }

    #[test]
    fn invalid_temp_clears_to_none() {
        let mut s = ToshibaState::new();
        s.apply(Field::RoomTemp(Some(20)));
        assert_eq!(s.room_temp, Some(20));
        s.apply(Field::RoomTemp(None)); // 127 sur le fil
        assert_eq!(s.room_temp, None);
    }

    #[test]
    fn ack_and_unknown_are_ignored() {
        let mut s = ToshibaState::new();
        s.apply(Field::Ack);
        s.apply(Field::Unknown {
            cmd: 0x99,
            value: 1,
        });
        assert_eq!(s, ToshibaState::default());
    }

    #[test]
    fn self_clean_flag() {
        let mut s = ToshibaState::new();
        s.apply(Field::SelfClean(SelfCleanState::Running));
        assert!(s.self_clean);
        s.apply(Field::SelfClean(SelfCleanState::Off));
        assert!(!s.self_clean);
    }
}
