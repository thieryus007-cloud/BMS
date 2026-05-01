use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

const GRL: &str = include_str!("../../../rules/charge_current.grl");

pub struct ChargeRuleEngine {
    engine: RustRuleEngine,
}

impl ChargeRuleEngine {
    pub fn new() -> anyhow::Result<Self> {
        let kb = KnowledgeBase::new("charge_current");
        kb.add_rules_from_grl(GRL)
            .context("Failed to load charge_current.grl")?;
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
