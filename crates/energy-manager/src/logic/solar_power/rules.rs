use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

const GRL: &str = include_str!("../../../rules/solar_power.grl");

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
    pub fn new() -> anyhow::Result<Self> {
        let kb = KnowledgeBase::new("solar_power");
        kb.add_rules_from_grl(GRL)
            .context("Failed to load solar_power.grl")?;
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
