use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

// ⚠️ Adaptez le chemin si votre fichier .grl est ailleurs
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

    /// Évalue les conditions et retourne le mode cible (ex: "VACATION", "STANDARD", "ECO")
    pub fn evaluate(&mut self, grid_connected: bool, soc: f64, irradiance_low: bool) -> anyhow::Result<String> {
        let mut facts = Facts::new();
        facts.set("WH.grid_connected", Value::Boolean(grid_connected));
        facts.set("WH.soc", Value::Number(soc));
        facts.set("WH.irradiance_low", Value::Boolean(irradiance_low));
        facts.set("WH.target_mode", Value::String("UNKNOWN".into()));

        self.engine
            .execute(&mut facts)
            .context("Water heater rule engine evaluation failed")?;

        Ok(facts
            .get("WH.target_mode")
            .and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "VACATION".to_string()))
    }
}
