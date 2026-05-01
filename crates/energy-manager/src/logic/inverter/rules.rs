use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

const GRL: &str = include_str!("../../../rules/inverter.grl");

pub struct InverterRuleEngine {
    engine: RustRuleEngine,
}

impl InverterRuleEngine {
    pub fn new() -> anyhow::Result<Self> {
        let kb = KnowledgeBase::new("inverter");
        kb.add_rules_from_grl(GRL)
            .context("Failed to load inverter.grl")?;
        Ok(Self {
            engine: RustRuleEngine::new(kb),
        })
    }

    /// Returns true when both AC voltage and current are available,
    /// meaning ac_power = v × i can be computed.
    pub fn ac_power_ready(
        &mut self,
        ac_voltage_present: bool,
        ac_current_present: bool,
    ) -> anyhow::Result<bool> {
        let facts = Facts::new();
        facts.set("INV.ac_voltage_present", Value::Boolean(ac_voltage_present));
        facts.set("INV.ac_current_present", Value::Boolean(ac_current_present));
        facts.set("INV.ac_power_ready",     Value::Boolean(false));

        self.engine
            .execute(&facts)
            .context("Inverter rule engine evaluation failed")?;

        Ok(facts
            .get("INV.ac_power_ready")
            .and_then(|v| match v {
                Value::Boolean(b) => Some(b),
                _ => None,
            })
            .unwrap_or(false))
    }
}
