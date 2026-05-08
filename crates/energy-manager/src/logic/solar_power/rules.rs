use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

#[cfg(test)]
const GRL_EMBEDDED: &str = include_str!("../../../rules/solar_power.grl");

pub struct SolarRuleEngine {
    engine: RustRuleEngine,
}

#[derive(Debug, Default)]
pub struct BaselineDecision {
    /// Clear the stored baseline (new calendar day)
    pub reset: bool,
    /// Capture current kWh counter as the new start-of-day baseline
    pub capture: bool,
}

impl SolarRuleEngine {
    #[cfg(test)]
    pub fn new() -> anyhow::Result<Self> {
        Self::with_source(GRL_EMBEDDED)
    }

    pub fn with_source(grl: &str) -> anyhow::Result<Self> {
        let kb = KnowledgeBase::new("solar_power");
        kb.add_rules_from_grl(grl)
            .context("Failed to load solar_power rules")?;
        Ok(Self {
            engine: RustRuleEngine::new(kb),
        })
    }

    /// Returns whether the PV inverter ET112 daily baseline should be
    /// reset (new day) and/or captured (first message or post-reset).
    pub fn baseline_decision(
        &mut self,
        new_day: bool,
        baseline_absent: bool,
    ) -> anyhow::Result<BaselineDecision> {
        let facts = Facts::new();
        facts.set("SOLAR.new_day",         Value::Boolean(new_day));
        facts.set("SOLAR.baseline_absent", Value::Boolean(baseline_absent));
        facts.set("SOLAR.reset",           Value::Boolean(false));
        facts.set("SOLAR.capture",         Value::Boolean(false));

        self.engine
            .execute(&facts)
            .context("Solar rule engine evaluation failed")?;

        let reset = facts
            .get("SOLAR.reset")
            .and_then(|v| match v { Value::Boolean(b) => Some(b), _ => None })
            .unwrap_or(false);

        let capture = facts
            .get("SOLAR.capture")
            .and_then(|v| match v { Value::Boolean(b) => Some(b), _ => None })
            .unwrap_or(false);

        Ok(BaselineDecision { reset, capture })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e() -> SolarRuleEngine { SolarRuleEngine::new().unwrap() }

    #[test]
    fn new_day_resets_and_captures() {
        let d = e().baseline_decision(true, false).unwrap();
        assert!(d.reset);
        assert!(d.capture);
    }

    #[test]
    fn absent_baseline_captures_no_reset() {
        let d = e().baseline_decision(false, true).unwrap();
        assert!(!d.reset);
        assert!(d.capture);
    }

    #[test]
    fn neither_flag_no_action() {
        let d = e().baseline_decision(false, false).unwrap();
        assert!(!d.reset);
        assert!(!d.capture);
    }

    #[test]
    fn new_day_and_absent_both_trigger() {
        let d = e().baseline_decision(true, true).unwrap();
        assert!(d.reset);
        assert!(d.capture);
    }
}
