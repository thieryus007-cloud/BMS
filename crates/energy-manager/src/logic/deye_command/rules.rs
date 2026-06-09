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
    /// All threshold comparisons are pre-computed in Rust and passed as bool flags
    /// to avoid fact-to-fact comparisons in GRL.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate(
        &mut self,
        state_str: &str,
        freq_hz: f64,
        time_in_state_secs: u64,
        grid_connected: bool,
        cfg_freq_high: f64,
        cfg_freq_hard: f64,
        cfg_freq_low: f64,
        cfg_cut_delay: u64,
        cfg_reenable_delay: u64,
        lockout_expired: bool,
        restore_blocked: bool,
        mppt_cut: bool,
    ) -> anyhow::Result<DeyeDecision> {
        let facts = Facts::new();
        facts.set("DY.state",                  Value::String(state_str.to_string()));
        facts.set("DY.freq_high_exceeded",     Value::Boolean(freq_hz >= cfg_freq_high));
        facts.set("DY.freq_hard_exceeded",     Value::Boolean(freq_hz >= cfg_freq_hard));
        facts.set("DY.freq_low_reached",       Value::Boolean(freq_hz <= cfg_freq_low));
        facts.set("DY.cut_delay_elapsed",      Value::Boolean(time_in_state_secs >= cfg_cut_delay));
        facts.set("DY.reenable_delay_elapsed", Value::Boolean(time_in_state_secs >= cfg_reenable_delay));
        facts.set("DY.lockout_expired",        Value::Boolean(lockout_expired));
        facts.set("DY.grid_connected",         Value::Boolean(grid_connected));
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
    // (high 51.0 Hz / hard 51.3 Hz / low 50.3 Hz / cut 3s / reenable 45s, restore unblocked).
    fn eval(engine: &mut DeyeRuleEngine, state: &str, freq: f64, time_s: u64, grid: bool, lockout_exp: bool) -> DeyeDecision {
        engine.evaluate(state, freq, time_s, grid, 51.0, 51.3, 50.3, 3, 45, lockout_exp, false, false).unwrap()
    }

    // Variant exposing the restore-block guard.
    fn eval_rb(engine: &mut DeyeRuleEngine, state: &str, freq: f64, time_s: u64, grid: bool, lockout_exp: bool, restore_blocked: bool) -> DeyeDecision {
        engine.evaluate(state, freq, time_s, grid, 51.0, 51.3, 50.3, 3, 45, lockout_exp, restore_blocked, false).unwrap()
    }

    // Variant exposing the MPPT-based cut signal (battery topping/full per the MPPT stage).
    fn eval_mppt(engine: &mut DeyeRuleEngine, state: &str, freq: f64, grid: bool, mppt_cut: bool) -> DeyeDecision {
        engine.evaluate(state, freq, 0, grid, 51.0, 51.3, 50.3, 3, 45, false, false, mppt_cut).unwrap()
    }

    #[test]
    fn on_normal_freq_no_transition() {
        let d = eval(&mut e(), "On", 50.0, 0, false, false);
        assert!(d.next_state.is_none());
        assert!(!d.relay_on);
        assert!(!d.relay_off);
    }

    #[test]
    fn on_high_freq_transitions_pending_cut() {
        // 51.1 Hz: above the soft cut (51.0) but below the hard cut (51.3) → debounced PendingCut.
        let d = eval(&mut e(), "On", 51.1, 0, false, false);
        assert_eq!(d.next_state.as_deref(), Some("PendingCut"));
        assert!(!d.relay_off);
    }

    #[test]
    fn on_hard_freq_cuts_immediately() {
        // 51.4 Hz ≥ hard threshold → straight to Lockout + relay_off, no debounce wait.
        let d = eval(&mut e(), "On", 51.4, 0, false, false);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off);
    }

    #[test]
    fn pending_cut_hard_freq_cuts_immediately() {
        // Hard threshold reached while waiting in PendingCut → cut now, even before cut_delay.
        let d = eval(&mut e(), "PendingCut", 51.4, 1, false, false);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off);
    }

    #[test]
    fn mppt_full_cuts_even_at_low_freq() {
        // Battery topping/full per MPPT → cut the DEYE even though AC frequency is nominal.
        let d = eval_mppt(&mut e(), "On", 50.0, false, true);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off);
    }

    #[test]
    fn mppt_full_pending_cut_locks() {
        let d = eval_mppt(&mut e(), "PendingCut", 50.0, false, true);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off);
    }

    #[test]
    fn mppt_full_grid_connected_no_cut() {
        // Grid-tied: the MPPT-based cut is suppressed (frequency-shift only happens off-grid).
        let d = eval_mppt(&mut e(), "On", 50.0, true, true);
        assert!(d.next_state.is_none());
        assert!(!d.relay_off);
    }

    #[test]
    fn mppt_not_full_low_freq_stays_on() {
        let d = eval_mppt(&mut e(), "On", 50.0, false, false);
        assert!(d.next_state.is_none());
        assert!(!d.relay_off);
    }

    #[test]
    fn pending_cut_freq_drops_cancels() {
        let d = eval(&mut e(), "PendingCut", 50.0, 5, false, false);
        assert_eq!(d.next_state.as_deref(), Some("On"));
    }

    #[test]
    fn pending_cut_delay_elapsed_high_freq_locks() {
        // 51.1 Hz (soft, not hard) held past cut_delay → Lockout via the debounce path.
        let d = eval(&mut e(), "PendingCut", 51.1, 5, false, false);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off);
    }

    #[test]
    fn lockout_expires_transitions_off() {
        let d = eval(&mut e(), "Lockout", 50.0, 0, false, true);
        assert_eq!(d.next_state.as_deref(), Some("Off"));
    }

    #[test]
    fn off_low_freq_transitions_pending_restore() {
        let d = eval(&mut e(), "Off", 50.1, 0, false, false);
        assert_eq!(d.next_state.as_deref(), Some("PendingRestore"));
    }

    #[test]
    fn off_low_freq_restore_blocked_stays_off() {
        // Structural PV excess still present (battery full + sun) → hold DEYE off despite low freq.
        let d = eval_rb(&mut e(), "Off", 50.1, 0, false, false, true);
        assert!(d.next_state.is_none());
        assert!(!d.relay_on);
    }

    #[test]
    fn pending_restore_elapsed_low_freq_restores() {
        let d = eval(&mut e(), "PendingRestore", 50.0, 50, false, false);
        assert_eq!(d.next_state.as_deref(), Some("On"));
        assert!(d.relay_on);
    }

    #[test]
    fn on_hard_freq_grid_connected_no_cut() {
        // Grid-connected: a high/hard frequency transient must NOT cut the DEYE
        // (the ticker passes the real grid state, suppressing the cut rules).
        let d = eval(&mut e(), "On", 51.4, 30, true, false);
        assert!(d.next_state.is_none());
        assert!(!d.relay_off);
        assert!(!d.relay_on);
    }

    #[test]
    fn grid_reconnect_from_off_restores_immediately() {
        let d = eval(&mut e(), "Off", 50.0, 0, true, false);
        assert_eq!(d.next_state.as_deref(), Some("On"));
        assert!(d.relay_on);
    }

    #[test]
    fn grid_reconnect_overrides_restore_block() {
        // Even with a structural excess flagged, grid reconnect restores immediately.
        let d = eval_rb(&mut e(), "Off", 50.0, 0, true, false, true);
        assert_eq!(d.next_state.as_deref(), Some("On"));
        assert!(d.relay_on);
    }

    #[test]
    fn grid_reconnect_from_lockout_restores_immediately() {
        let d = eval(&mut e(), "Lockout", 50.0, 0, true, false);
        assert_eq!(d.next_state.as_deref(), Some("On"));
        assert!(d.relay_on);
    }

    #[test]
    fn grid_reconnect_from_pending_cut_restores_immediately() {
        // Hard freq + grid reconnect + cut_delay elapsed: grid override must win cleanly
        // (no conflicting relay_off from the cut path).
        let d = eval(&mut e(), "PendingCut", 51.4, 5, true, false);
        assert_eq!(d.next_state.as_deref(), Some("On"));
        assert!(d.relay_on);
        assert!(!d.relay_off);
    }

    // =========================================================================
    // Rejeu de l'incident « micro-coupures 51,5 Hz » (audit 2026-06 §17).
    // Cf. CLAUDE.md §8 : les DEYE s'auto-coupent à 51,5 Hz → le Shelly doit
    // les couper AVANT (freq_hard = 51,3). Cette séquence déroule la machine
    // d'états sur le scénario complet observé en prod : montée de fréquence,
    // coupure, verrouillage anti-rebond, retour stable, restauration.
    // =========================================================================
    #[test]
    fn scenario_montee_51_5_hz_coupe_avant_auto_trip_puis_restaure() {
        let mut engine = e();

        // 1. Fréquence nominale : aucun mouvement.
        let d = eval(&mut engine, "On", 50.0, 0, false, false);
        assert!(d.next_state.is_none() && !d.relay_off && !d.relay_on);

        // 2. La fréquence monte (excédent PV off-grid) : 51,1 Hz → débounce.
        let d = eval(&mut engine, "On", 51.1, 0, false, false);
        assert_eq!(d.next_state.as_deref(), Some("PendingCut"));
        assert!(!d.relay_off, "coupure prématurée pendant le débounce");

        // 3. POINT CRITIQUE : 51,5 Hz atteint pendant le débounce (seuil
        //    d'auto-trip DEYE). La coupure doit être IMMÉDIATE (51,5 ≥ hard
        //    51,3), sans attendre cut_delay — sinon le DEYE s'auto-coupe et
        //    provoque la micro-coupure AC Out.
        let d = eval(&mut engine, "PendingCut", 51.5, 1, false, false);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off, "le relais doit couper avant l'auto-trip DEYE");

        // 4. Anti-rebattement : fréquence retombée mais lockout non expiré →
        //    on ne bouge pas (pas de cycle on/off rapide).
        let d = eval(&mut engine, "Lockout", 50.6, 10, false, false);
        assert!(d.next_state.is_none() && !d.relay_on);

        // 5. Lockout expiré → passage Off (toujours relais coupé).
        let d = eval(&mut engine, "Lockout", 50.2, 0, false, true);
        assert_eq!(d.next_state.as_deref(), Some("Off"));
        assert!(!d.relay_on);

        // 6. Fréquence basse stable (≤ 50,3) → fenêtre de restauration.
        let d = eval(&mut engine, "Off", 50.1, 0, false, false);
        assert_eq!(d.next_state.as_deref(), Some("PendingRestore"));

        // 7. Stabilité pas encore prouvée (10 s < reenable 45 s) → on attend.
        let d = eval(&mut engine, "PendingRestore", 50.1, 10, false, false);
        assert!(d.next_state.is_none() && !d.relay_on, "restauration trop précoce");

        // 8. 50 s stables → restauration effective.
        let d = eval(&mut engine, "PendingRestore", 50.0, 50, false, false);
        assert_eq!(d.next_state.as_deref(), Some("On"));
        assert!(d.relay_on);
    }

    #[test]
    fn scenario_pic_brutal_51_5_hz_depuis_on() {
        // Variante : montée si rapide que 51,5 Hz est vu directement depuis
        // On (pas de passage PendingCut observable au tick précédent).
        let d = eval(&mut e(), "On", 51.5, 0, false, false);
        assert_eq!(d.next_state.as_deref(), Some("Lockout"));
        assert!(d.relay_off, "coupure immédiate requise à 51,5 Hz");
    }
}
