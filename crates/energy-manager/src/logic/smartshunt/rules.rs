use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

const GRL: &str = include_str!("../../../rules/smartshunt.grl");

pub struct SmartShuntRuleEngine {
    engine: RustRuleEngine,
}

#[derive(Debug, Default)]
pub struct BaselineDecision {
    pub capture_charged: bool,
    pub capture_discharged: bool,
}

impl SmartShuntRuleEngine {
    pub fn new() -> anyhow::Result<Self> {
        let kb = KnowledgeBase::new("smartshunt");
        kb.add_rules_from_grl(GRL)
            .context("Failed to load smartshunt.grl")?;
        Ok(Self {
            engine: RustRuleEngine::new(kb),
        })
    }

    /// Decides whether to capture the charged and/or discharged energy baseline.
    pub fn baseline_decision(
        &mut self,
        charged_new_day: bool,
        charged_baseline_absent: bool,
        discharged_new_day: bool,
        discharged_baseline_absent: bool,
    ) -> anyhow::Result<BaselineDecision> {
        let facts = Facts::new();
        facts.set("SHUNT.charged_new_day",            Value::Boolean(charged_new_day));
        facts.set("SHUNT.charged_baseline_absent",    Value::Boolean(charged_baseline_absent));
        facts.set("SHUNT.discharged_new_day",         Value::Boolean(discharged_new_day));
        facts.set("SHUNT.discharged_baseline_absent", Value::Boolean(discharged_baseline_absent));
        facts.set("SHUNT.capture_charged",            Value::Boolean(false));
        facts.set("SHUNT.capture_discharged",         Value::Boolean(false));

        self.engine
            .execute(&facts)
            .context("SmartShunt rule engine evaluation failed")?;

        let capture_charged = facts
            .get("SHUNT.capture_charged")
            .and_then(|v| match v { Value::Boolean(b) => Some(b), _ => None })
            .unwrap_or(false);

        let capture_discharged = facts
            .get("SHUNT.capture_discharged")
            .and_then(|v| match v { Value::Boolean(b) => Some(b), _ => None })
            .unwrap_or(false);

        Ok(BaselineDecision { capture_charged, capture_discharged })
    }
}
