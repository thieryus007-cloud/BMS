//crates/energy-manager/src/logic/irradiance/rules.rs

use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

const GRL: &str = include_str!("../../../rules/irradiance.grl");

pub struct IrradianceRuleEngine {
    engine: RustRuleEngine,
}

impl IrradianceRuleEngine {
    pub fn new() -> anyhow::Result<Self> {
        let kb = KnowledgeBase::new("irradiance");
        kb.add_rules_from_grl(GRL)
            .context("Failed to load irradiance.grl")?;
        Ok(Self {
            engine: RustRuleEngine::new(kb),
        })
    }

    pub fn validate(&mut self, raw: f64) -> anyhow::Result<bool> {
        let mut facts = Facts::new();
        facts.set("IR.raw", Value::Number(raw));
        facts.set("IR.valid", Value::Boolean(false));

        self.engine
            .execute(&mut facts)
            .context("Irradiance rule engine evaluation failed")?;

        Ok(facts
            .get("IR.valid")
            .and_then(|v| match v {
                Value::Boolean(b) => Some(b),
                _ => None,
            })
            .unwrap_or(false))
    }
}
