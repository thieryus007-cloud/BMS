use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

#[cfg(test)]
const GRL_EMBEDDED: &str = include_str!("../../../rules/smartshunt.grl");

pub struct SmartShuntRuleEngine {
    engine: RustRuleEngine,
}

#[derive(Debug, Default)]
pub struct BaselineDecision {
    pub capture_charged: bool,
    pub capture_discharged: bool,
}

impl SmartShuntRuleEngine {
    #[cfg(test)]
    pub fn new() -> anyhow::Result<Self> {
        Self::with_source(GRL_EMBEDDED)
    }

    pub fn with_source(grl: &str) -> anyhow::Result<Self> {
        let kb = KnowledgeBase::new("smartshunt");
        kb.add_rules_from_grl(grl)
            .context("Failed to load smartshunt rules")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn e() -> SmartShuntRuleEngine { SmartShuntRuleEngine::new().unwrap() }

    #[test]
    fn new_day_captures_charged() {
        let d = e().baseline_decision(true, false, false, false).unwrap();
        assert!(d.capture_charged);
        assert!(!d.capture_discharged);
    }

    #[test]
    fn missing_baseline_captures_charged() {
        let d = e().baseline_decision(false, true, false, false).unwrap();
        assert!(d.capture_charged);
        assert!(!d.capture_discharged);
    }

    #[test]
    fn new_day_captures_discharged() {
        let d = e().baseline_decision(false, false, true, false).unwrap();
        assert!(!d.capture_charged);
        assert!(d.capture_discharged);
    }

    #[test]
    fn missing_baseline_captures_discharged() {
        let d = e().baseline_decision(false, false, false, true).unwrap();
        assert!(!d.capture_charged);
        assert!(d.capture_discharged);
    }

    #[test]
    fn all_false_captures_nothing() {
        let d = e().baseline_decision(false, false, false, false).unwrap();
        assert!(!d.capture_charged);
        assert!(!d.capture_discharged);
    }

    #[test]
    fn all_true_captures_both() {
        let d = e().baseline_decision(true, true, true, true).unwrap();
        assert!(d.capture_charged);
        assert!(d.capture_discharged);
    }
}
