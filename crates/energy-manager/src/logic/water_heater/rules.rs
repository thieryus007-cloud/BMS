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

    /// Evaluates all conditions and returns "HeatPump" or "Vacation".
    /// Debounce flags are pre-computed in Rust before calling this method.
    pub fn evaluate(
        &mut self,
        grid_connected: bool,
        soc_pct: f64,
        discharge_debounced: bool,
        solar_low_debounced: bool,
        irradiance_low: bool,
    ) -> anyhow::Result<String> {
        let facts = Facts::new();
        // Initialise to false so WH_Decide_HeatPump fires when no condition is met
        facts.set("WH.want_vacation",       Value::Boolean(false));
        facts.set("WH.grid_connected",      Value::Boolean(grid_connected));
        facts.set("WH.soc_pct",             Value::Number(soc_pct));
        facts.set("WH.discharge_debounced", Value::Boolean(discharge_debounced));
        facts.set("WH.solar_low_debounced", Value::Boolean(solar_low_debounced));
        facts.set("WH.irradiance_low",      Value::Boolean(irradiance_low));

        self.engine
            .execute(&facts)
            .context("Water heater rule engine evaluation failed")?;

        let mode = facts
            .get("WH.target_mode")
            .and_then(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            })
            // Fail-safe: prefer Vacation (less consumption) on any engine error
            .unwrap_or_else(|| "Vacation".to_string());

        Ok(mode)
    }
}
