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
    pub fn evaluate(
        &mut self,
        state_str: &str,
        freq_hz: f64,
        time_in_state_secs: u64,
        grid_connected: bool,
        cfg_freq_high: f64,
        cfg_freq_low: f64,
        cfg_cut_delay: u64,
        cfg_reenable_delay: u64,
        lockout_expired: bool,
    ) -> anyhow::Result<DeyeDecision> {
        let facts = Facts::new();
        facts.set("DY.state",                  Value::String(state_str.to_string()));
        facts.set("DY.freq_high_exceeded",     Value::Boolean(freq_hz >= cfg_freq_high));
        facts.set("DY.freq_low_reached",       Value::Boolean(freq_hz <= cfg_freq_low));
        facts.set("DY.cut_delay_elapsed",      Value::Boolean(time_in_state_secs >= cfg_cut_delay));
        facts.set("DY.reenable_delay_elapsed", Value::Boolean(time_in_state_secs >= cfg_reenable_delay));
        facts.set("DY.lockout_expired",        Value::Boolean(lockout_expired));
        facts.set("DY.grid_connected",         Value::Boolean(grid_connected));

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

    // Shorthand: evaluate with default config thresholds (52 Hz / 50.3 Hz / 15s / 45s)
    fn eval(engine: &mut DeyeRuleEngine, state: &str, freq: f64, time_s: u64, grid: bool, lockout_exp: bool) -> DeyeDecision {
        engine.evaluate(state, freq, time_s, grid, 52.0, 50.3, 15, 45, lockout_exp).unwrap()
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
        let d = eval(&mut e(), "On", 52.5, 0, false, false);
        assert_eq!(d.next_state.as_deref(), Some("PendingCut"));
    }

    #[test]
    fn pending_cut_freq_drops_cancels() {
        let d = eval(&mut e(), "PendingCut", 50.0, 5, false, false);
        assert_eq!(d.next_state.as_deref(), Some("On"));
    }

    #[test]
    fn pending_cut_delay_elapsed_high_freq_locks() {
        let d = eval(&mut e(), "PendingCut", 52.5, 20, false, false);
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
    fn pending_restore_elapsed_low_freq_restores() {
        let d = eval(&mut e(), "PendingRestore", 50.0, 50, false, false);
        assert_eq!(d.next_state.as_deref(), Some("On"));
        assert!(d.relay_on);
    }

    #[test]
    fn grid_reconnect_from_off_restores_immediately() {
        let d = eval(&mut e(), "Off", 50.0, 0, true, false);
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
        let d = eval(&mut e(), "PendingCut", 52.5, 5, true, false);
        assert_eq!(d.next_state.as_deref(), Some("On"));
        assert!(d.relay_on);
    }
}
