use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

#[cfg(test)]
const GRL_EMBEDDED: &str = include_str!("../../../rules/irradiance.grl");

pub struct IrradianceRuleEngine {
    engine: RustRuleEngine,
}

impl IrradianceRuleEngine {
    #[cfg(test)]
    pub fn new() -> anyhow::Result<Self> {
        Self::with_source(GRL_EMBEDDED)
    }

    pub fn with_source(grl: &str) -> anyhow::Result<Self> {
        let kb = KnowledgeBase::new("irradiance");
        kb.add_rules_from_grl(grl)
            .context("Failed to load irradiance rules")?;
        Ok(Self {
            engine: RustRuleEngine::new(kb),
        })
    }

    pub fn validate(&mut self, raw: f64) -> anyhow::Result<bool> {
        let mut facts = Facts::new();
        facts.set("IR.raw",   Value::Number(raw));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn e() -> IrradianceRuleEngine { IrradianceRuleEngine::new().unwrap() }

    #[test]
    fn zero_is_valid() {
        assert!(e().validate(0.0).unwrap());
    }

    #[test]
    fn max_is_valid() {
        assert!(e().validate(2000.0).unwrap());
    }

    #[test]
    fn midrange_is_valid() {
        assert!(e().validate(750.0).unwrap());
    }

    #[test]
    fn negative_is_invalid() {
        assert!(!e().validate(-1.0).unwrap());
    }

    #[test]
    fn above_max_is_invalid() {
        assert!(!e().validate(2001.0).unwrap());
    }
}
