use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

#[cfg(test)]
const GRL_EMBEDDED: &str = include_str!("../../../rules/deye_command.grl");

pub struct DeyeRuleEngine {
    engine: RustRuleEngine,
}

#[derive(Debug, Default)]
pub struct DeyeDecision {
    /// New state to transition to, or None if no transition should occur
    pub next_state: Option<String>,
    /// Relay should be switched ON (restore DEYE)
    pub relay_on: bool,
    /// Relay should be switched OFF (cut DEYE)
    pub relay_off: bool,
}

impl DeyeRuleEngine {
    #[cfg(test)]
    pub fn new() -> anyhow::Result<Self> {
        Self::with_source(GRL_EMBEDDED)
    }

    pub fn with_source(grl: &str) -> anyhow::Result<Self> {
        let kb = KnowledgeBase::new("deye_command");
        kb.add_rules_from_grl(grl)
            .context("Failed to load deye_command rules")?;
        Ok(Self {
            engine: RustRuleEngine::new(kb),
        })
    }

    /// Evaluates the state machine and returns the decision.
    ///
    /// The decision depends ONLY on the AC-Out frequency and the MPPT charge stage.
    /// A single frequency boundary (`cfg_freq_cut`) governs both cut and restore, so
    /// there is no dead band; anti-chatter hysteresis is purely temporal (cut_delay,
    /// lockout, reenable_delay). All threshold comparisons are pre-computed in Rust
    /// and passed as bool flags to avoid fact-to-fact comparisons in GRL.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate(
        &mut self,
        state_str: &str,
        freq_hz: f64,
        time_in_state_secs: u64,
        cfg_freq_cut: f64,
        cfg_freq_hard: f64,
        cfg_cut_delay: u64,
        cfg_reenable_delay: u64,
        lockout_expired: bool,
        restore_blocked: bool,
        mppt_cut: bool,
    ) -> anyhow::Result<DeyeDecision> {
        let facts = Facts::new();
        facts.set("DY.state",                  Value::String(state_str.to_string()));
        facts.set("DY.freq_cut_exceeded",      Value::Boolean(freq_hz >= cfg_freq_cut));
        facts.set("DY.freq_hard_exceeded",     Value::Boolean(freq_hz >= cfg_freq_hard));
        facts.set("DY.cut_delay_elapsed",      Value::Boolean(time_in_state_secs >= cfg_cut_delay));
        facts.set("DY.reenable_delay_elapsed", Value::Boolean(time_in_state_secs >= cfg_reenable_delay));
        facts.set("DY.lockout_expired",        Value::Boolean(lockout_expired));
        facts.set("DY.restore_blocked",        Value::Boolean(restore_blocked));
        facts.set("DY.mppt_cut",               Value::Boolean(mppt_cut));

        self.engine
            .execute(&facts)
            .context("DEYE rule engine evaluation failed")?;

        let next_state = facts.get("DY.next_state").and_then(|v| match v {
            Value::String(s) => Some(s),
            _ => None,
        });
        let relay_on = facts.get("DY.relay_on").and_then(|v| match v {
            Value::Boolean(b) => Some(b),
            _ => None,
        }).unwrap_or(false);
        let relay_off = facts.get("DY.relay_off").and_then(|v| match v {
            Value::Boolean(b) => Some(b),
            _ => None,
        }).unwrap_or(false);

        Ok(DeyeDecision { next_state, relay_on, relay_off })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e() -> DeyeRuleEngine { DeyeRuleEngine::new().unwrap() }

    // Shorthand: evaluate with default config thresholds
    // (cut 51.0 Hz / hard 51.3 Hz / cut_delay 3s / reenable 45s, battery not full).
    fn eval(engine: &mut DeyeRuleEngine, state: &str, freq: f64, time_s: u64, lockout_exp: bool) -> DeyeDecision {
        engine.evaluate(state, freq, time_s, 51.0, 51.3, 3, 45, lockout_exp, false, false).unwrap()
    }

    // Variant exposing the restore-block guard (battery full per MPPT stage).
    fn eval_rb(engine: &mut DeyeRuleEngine, state: &str, freq: f64, time_s: u64, lockout_exp: bool, restore_blocked: bool) -> DeyeDecision {
        engine.evaluate(state, freq, time_s, 51.0, 51.3, 3, 45, lockout_exp, restore_blocked, false).unwrap()
    }

    // Variant exposing the MPPT-based cut signal (battery full per the MPPT stage, debounced).
    fn eval_mppt(engine: &mut DeyeRuleEngine, state: &str, freq: f64, mppt_cut: bool) -> DeyeDecision {
        engine.evaluate(state, freq, 0, 51.0, 51.3, 3, 45, false, false, mppt_cut).unwrap()
    }

    #[test]
    fn on_normal_freq_no_transition() {
        let d = eval(&mut e(), "On", 50.0, 0, false);
        assert!(d.next_state.is_none());
        assert!(!d.relay_on);
        assert!(!d.relay_off);
    }

    #[test]
    fn on_high_freq_transitions_pending_cut() {
        // 51.1 Hz: at/above the cut boundary (51.0) but below the hard cut (51.3) → debounced PendingCut.
        let d = eval(&mut e(), "On", 51.1, 0, false);
        assert_eq!(d.next_state.as_deref(), Some("PendingCut"));
        assert!(!d.relay_off);
    }

    #[test]
    fn on_hard_freq_cuts_immediately() {
        // 51.4 Hz ≥ hard threshold → straight to Lockout + relay_off, no debounce wait.
        let d = eval(&mut e(), "On", 51.4, 0, false);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off);
    }

    #[test]
    fn pending_cut_hard_freq_cuts_immediately() {
        let d = eval(&mut e(), "PendingCut", 51.4, 1, false);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off);
    }

    #[test]
    fn mppt_full_cuts_even_at_low_freq() {
        // Battery full per MPPT → cut the DEYE even though AC frequency is nominal.
        let d = eval_mppt(&mut e(), "On", 50.0, true);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off);
    }

    #[test]
    fn mppt_full_pending_cut_locks() {
        let d = eval_mppt(&mut e(), "PendingCut", 50.0, true);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off);
    }

    #[test]
    fn mppt_not_full_low_freq_stays_on() {
        let d = eval_mppt(&mut e(), "On", 50.0, false);
        assert!(d.next_state.is_none());
        assert!(!d.relay_off);
    }

    #[test]
    fn pending_cut_freq_drops_cancels() {
        let d = eval(&mut e(), "PendingCut", 50.0, 5, false);
        assert_eq!(d.next_state.as_deref(), Some("On"));
    }

    #[test]
    fn pending_cut_delay_elapsed_high_freq_locks() {
        // 51.1 Hz held past cut_delay → Lockout via the debounce path.
        let d = eval(&mut e(), "PendingCut", 51.1, 5, false);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off);
    }

    #[test]
    fn lockout_expires_transitions_off() {
        let d = eval(&mut e(), "Lockout", 50.0, 0, true);
        assert_eq!(d.next_state.as_deref(), Some("Off"));
    }

    #[test]
    fn off_below_boundary_transitions_pending_restore() {
        // No dead band: anything below the cut boundary (51.0) is restore-eligible,
        // including the old dead-band region (50.3–51.0).
        let d = eval(&mut e(), "Off", 50.7, 0, false);
        assert_eq!(d.next_state.as_deref(), Some("PendingRestore"));
    }

    #[test]
    fn off_below_boundary_restore_blocked_stays_off() {
        // Battery full per MPPT → hold DEYE off despite a restorable frequency.
        let d = eval_rb(&mut e(), "Off", 50.7, 0, false, true);
        assert!(d.next_state.is_none());
        assert!(!d.relay_on);
    }

    #[test]
    fn pending_restore_elapsed_restores() {
        let d = eval(&mut e(), "PendingRestore", 50.7, 50, false);
        assert_eq!(d.next_state.as_deref(), Some("On"));
        assert!(d.relay_on);
    }

    #[test]
    fn pending_restore_freq_climbs_cancels() {
        // Frequency climbs back to the cut boundary during the restore wait → back to Off.
        let d = eval(&mut e(), "PendingRestore", 51.0, 50, false);
        assert_eq!(d.next_state.as_deref(), Some("Off"));
        assert!(!d.relay_on);
    }

    #[test]
    fn pending_restore_battery_fills_cancels() {
        // Battery fills up (MPPT reaches a full stage) during the restore wait → back to Off.
        let d = eval_rb(&mut e(), "PendingRestore", 50.0, 50, false, true);
        assert_eq!(d.next_state.as_deref(), Some("Off"));
        assert!(!d.relay_on);
    }

    // =========================================================================
    // No dead band: a DEYE cut at high frequency must restore as soon as the
    // frequency falls back below the single boundary (51.0 Hz), even if it
    // settles in the old 50.3–51.0 dead-band region.
    // =========================================================================
    #[test]
    fn no_dead_band_restores_in_old_dead_zone() {
        let mut engine = e();

        // Cut at high frequency.
        let d = eval(&mut engine, "On", 51.4, 0, false);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));

        // Lockout expires → Off.
        let d = eval(&mut engine, "Lockout", 50.6, 0, true);
        assert_eq!(d.next_state.as_deref(), Some("Off"));

        // Frequency settles at 50.6 Hz (the OLD dead zone) → still restore-eligible now.
        let d = eval(&mut engine, "Off", 50.6, 0, false);
        assert_eq!(d.next_state.as_deref(), Some("PendingRestore"));

        // Sustained below the boundary → restores.
        let d = eval(&mut engine, "PendingRestore", 50.6, 50, false);
        assert_eq!(d.next_state.as_deref(), Some("On"));
        assert!(d.relay_on);
    }

    // =========================================================================
    // Rejeu de l'incident « micro-coupures 51,5 Hz » (audit 2026-06 §17) :
    // les DEYE s'auto-coupent à 51,5 Hz → le Shelly doit couper AVANT
    // (freq_hard = 51,3). Déroule la machine sur le scénario complet.
    // =========================================================================
    #[test]
    fn scenario_montee_51_5_hz_coupe_avant_auto_trip_puis_restaure() {
        let mut engine = e();

        let d = eval(&mut engine, "On", 50.0, 0, false);
        assert!(d.next_state.is_none() && !d.relay_off && !d.relay_on);

        // Montée à 51,1 Hz → débounce PendingCut.
        let d = eval(&mut engine, "On", 51.1, 0, false);
        assert_eq!(d.next_state.as_deref(), Some("PendingCut"));
        assert!(!d.relay_off);

        // 51,5 Hz pendant le débounce → coupure IMMÉDIATE (≥ hard 51,3).
        let d = eval(&mut engine, "PendingCut", 51.5, 1, false);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off);

        // Lockout non expiré → on attend.
        let d = eval(&mut engine, "Lockout", 50.6, 10, false);
        assert!(d.next_state.is_none() && !d.relay_on);

        // Lockout expiré → Off.
        let d = eval(&mut engine, "Lockout", 50.2, 0, true);
        assert_eq!(d.next_state.as_deref(), Some("Off"));

        // Fréquence sous le seuil → fenêtre de restauration.
        let d = eval(&mut engine, "Off", 50.1, 0, false);
        assert_eq!(d.next_state.as_deref(), Some("PendingRestore"));

        // Stabilité pas prouvée (10 s < 45 s) → on attend.
        let d = eval(&mut engine, "PendingRestore", 50.1, 10, false);
        assert!(d.next_state.is_none() && !d.relay_on);

        // 50 s stables → restauration.
        let d = eval(&mut engine, "PendingRestore", 50.0, 50, false);
        assert_eq!(d.next_state.as_deref(), Some("On"));
        assert!(d.relay_on);
    }

    #[test]
    fn scenario_pic_brutal_51_5_hz_depuis_on() {
        let d = eval(&mut e(), "On", 51.5, 0, false);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off);
    }
}
