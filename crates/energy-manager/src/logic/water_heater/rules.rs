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

    /// ⚠️ VERSION TEST : Force "HeatPump" pour valider le pipeline Rust → LG
    /// Remplace temporairement la logique GRL pour isoler le problème.
    pub fn evaluate(
        &mut self,
        _grid_connected: bool,
        _soc_pct: f64,
        _irradiance_low: bool,
    ) -> anyhow::Result<String> {
        Ok("HeatPump".to_string())
    }
}
