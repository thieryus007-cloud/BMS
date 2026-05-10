use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

#[cfg(test)]
const GRL_EMBEDDED: &str = include_str!("../../../rules/charge_current.grl");

pub struct ChargeRuleEngine {
    engine: RustRuleEngine,
}

impl ChargeRuleEngine {
    #[cfg(test)]
    pub fn new() -> anyhow::Result<Self> {
        Self::with_source(GRL_EMBEDDED)
    }

    pub fn with_source(grl: &str) -> anyhow::Result<Self> {
        let kb = KnowledgeBase::new("charge_current");
        kb.add_rules_from_grl(grl)
            .context("Failed to load charge_current rules")?;
        Ok(Self {
            engine: RustRuleEngine::new(kb),
        })
    }

    /// Returns "offgrid", "grid_pv_excess", or "grid_no_excess".
    pub fn evaluate(&mut self, offgrid: bool, pv_excess: bool) -> anyhow::Result<String> {
        let facts = Facts::new();
        facts.set("CC.offgrid",   Value::Boolean(offgrid));
        facts.set("CC.pv_excess", Value::Boolean(pv_excess));

        self.engine
            .execute(&facts)
            .context("Charge current rule engine evaluation failed")?;

        let mode = facts
            .get("CC.mode")
            .and_then(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| "grid_no_excess".to_string());

        Ok(mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e() -> ChargeRuleEngine { ChargeRuleEngine::new().unwrap() }

    #[test]
    fn offgrid_ignores_pv_excess() {
        assert_eq!(e().evaluate(true, false).unwrap(), "offgrid");
    }

    #[test]
    fn offgrid_with_pv_excess_still_offgrid() {
        assert_eq!(e().evaluate(true, true).unwrap(), "offgrid");
    }

    #[test]
    fn grid_with_pv_excess() {
        assert_eq!(e().evaluate(false, true).unwrap(), "grid_pv_excess");
    }

    #[test]
    fn grid_no_excess() {
        assert_eq!(e().evaluate(false, false).unwrap(), "grid_no_excess");
    }
}
