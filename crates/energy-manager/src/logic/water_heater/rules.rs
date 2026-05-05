// crates/energy-manager/src/logic/water_heater/rules.rs
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

    /// Évalue les conditions et retourne le mode cible ("VACATION" ou "HEAT_PUMP")
    pub fn evaluate(&mut self, grid_connected: bool, soc: f64, irradiance_low: bool) -> anyhow::Result<String> {
        let mut facts = Facts::new();
        
        // ⚠️ Noms EXACTS comme dans votre fichier .grl
        facts.set("WH.want_vacation", Value::Boolean(false));
        facts.set("WH.grid_connected", Value::Boolean(grid_connected));
        facts.set("WH.soc_pct", Value::Number(soc)); // ✅ "soc_pct", pas "soc"
        facts.set("WH.irradiance_low", Value::Boolean(irradiance_low));
        facts.set("WH.target_mode", Value::String("VACATION".to_string()));

        self.engine
            .execute(&mut facts) // ✅ &mut obligatoire
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
