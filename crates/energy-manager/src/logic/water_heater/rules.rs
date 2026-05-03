use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

const GRL: &str = include_str!("../../../rules/water_heater.grl");

pub struct WaterHeaterRuleEngine {
    engine: RustRuleEngine,
}

impl WaterHeaterRuleEngine {
    pub fn new() -> anyhow::Result<Self> {
        let kb = KnowledgeBase::new("water_heater");
        kb.add_rules_from_grl(GRL)
            .context("Failed to load water_heater.grl")?;
        Ok(Self {
            engine: RustRuleEngine::new(kb),
        })
    }

    /// Evaluates conditions and returns "HeatPump" or "Vacation".
    /// HEAT_PUMP requires: SOC >= 90%, irradiance >= threshold, grid disconnected (ac_ignore=1).
    pub fn evaluate(
        &mut self,
        grid_connected: bool,
        soc_pct: f64,
        irradiance_low: bool,
    ) -> anyhow::Result<String> {
        let facts = Facts::new();
        facts.set("WH.want_vacation",  Value::Boolean(false));
        facts.set("WH.grid_connected", Value::Boolean(grid_connected));
        facts.set("WH.soc_pct",        Value::Number(soc_pct));
        facts.set("WH.irradiance_low", Value::Boolean(irradiance_low));

        self.engine
            .execute(&facts)
            .context("Water heater rule engine evaluation failed")?;

        let mode = facts
            .get("WH.target_mode")
            .and_then(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| "Vacation".to_string());

        Ok(mode)
    }
}
