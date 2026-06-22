use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

#[cfg(test)]
const GRL_EMBEDDED: &str = include_str!("../../../rules/deye_command.grl");

/// Stateless DEYE decision engine (`deye_command.grl`). Same model as the other modules:
/// the Rust side pre-computes the boolean flags and this engine maps them to the desired
/// relay state. Holds NO latching state — given the same flags it always returns the same
/// answer, so the relay can never get stuck.
pub struct DeyeRuleEngine {
    engine: RustRuleEngine,
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

    /// Returns the desired relay state (true = ON) from the current state + pre-computed flags.
    ///
    /// - `hard_cut`          : `freq >= freq_hard_hz` → immediate cut
    /// - `cut_debounced`     : the cut condition (high freq OR MPPT full) has held `cut_delay_secs`
    /// - `restore_debounced` : the clear condition (low freq AND not full) has held `reenable_delay_secs`
    pub fn decide(
        &mut self,
        relay_on: bool,
        hard_cut: bool,
        cut_debounced: bool,
        restore_debounced: bool,
    ) -> anyhow::Result<bool> {
        let facts = Facts::new();
        facts.set("DY.relay_on",          Value::Boolean(relay_on));
        facts.set("DY.hard_cut",          Value::Boolean(hard_cut));
        facts.set("DY.cut_debounced",     Value::Boolean(cut_debounced));
        facts.set("DY.restore_debounced", Value::Boolean(restore_debounced));
        // Default: no change. A matching rule overrides it.
        facts.set("DY.desired_on",        Value::Boolean(relay_on));

        self.engine
            .execute(&facts)
            .context("DEYE rule engine evaluation failed")?;

        Ok(facts
            .get("DY.desired_on")
            .and_then(|v| match v {
                Value::Boolean(b) => Some(b),
                _ => None,
            })
            .unwrap_or(relay_on))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e() -> DeyeRuleEngine { DeyeRuleEngine::new().unwrap() }

    #[test]
    fn on_stays_on_when_clear() {
        // ON, no cut flags → stays ON.
        assert!(e().decide(true, false, false, false).unwrap());
    }

    #[test]
    fn on_cuts_on_hard_spike() {
        assert!(!e().decide(true, true, false, false).unwrap());
    }

    #[test]
    fn on_cuts_when_debounced() {
        assert!(!e().decide(true, false, true, false).unwrap());
    }

    #[test]
    fn off_stays_off_until_restore_debounced() {
        // OFF, clear not yet held → stays OFF.
        assert!(!e().decide(false, false, false, false).unwrap());
        // OFF, restore debounce elapsed → turns ON.
        assert!(e().decide(false, false, false, true).unwrap());
    }

    #[test]
    fn off_ignores_cut_flags() {
        // Cut flags are irrelevant while already OFF; only restore_debounced matters.
        assert!(!e().decide(false, true, true, false).unwrap());
    }
}
