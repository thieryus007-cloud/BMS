use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

// ⚠️ Assurez-vous que ce chemin pointe vers votre fichier .grl
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

    /// Évalue les conditions et retourne le mode cible (ex: "VACATION", "STANDARD")
    pub fn evaluate(&mut self, grid_connected: bool, soc: f64, irradiance_low: bool) -> anyhow::Result<String> {
        let mut facts = Facts::new();
        
        // On injecte les faits nécessaires aux règles
        facts.set("WH.grid_connected", Value::Boolean(grid_connected));
        facts.set("WH.soc", Value::Number(soc));
        facts.set("WH.irradiance_low", Value::Boolean(irradiance_low));
        
        // Valeur par défaut avant exécution
        facts.set("WH.target_mode", Value::String("VACATION".to_string()));

        // Exécution du moteur de règles
        self.engine.execute(&mut facts)
            .context("Water heater rule engine execution failed")?;

        // Récupération du résultat
        Ok(facts
            .get("WH.target_mode")
            .and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "VACATION".to_string()))
    }
}
