use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

#[cfg(test)]
const GRL_EMBEDDED: &str = include_str!("../../../rules/water_heater.grl");

pub struct WaterHeaterRuleEngine {
    engine: RustRuleEngine,
}

impl WaterHeaterRuleEngine {
    #[cfg(test)]
    pub fn new() -> anyhow::Result<Self> {
        Self::with_source(GRL_EMBEDDED)
    }

    pub fn with_source(grl: &str) -> anyhow::Result<Self> {
        let kb = KnowledgeBase::new("water_heater");
        kb.add_rules_from_grl(grl)
            .context("Failed to load water_heater rules")?;
        Ok(Self {
            engine: RustRuleEngine::new(kb),
        })
    }

    /// Returns "HEAT_PUMP" or "VACATION".
    ///
    /// `soc_low` is pre-computed by the caller: `soc_pct < cfg.soc_min_pct`.
    /// `temp_max_reached` is pre-computed: current temp held `>= temp_max_c`
    /// for at least `temp_max_hold_secs` (cuve à température cible).
    pub fn evaluate(
        &mut self,
        grid_connected: bool,
        soc_low: bool,
        irradiance_low: bool,
        temp_max_reached: bool,
    ) -> anyhow::Result<String> {
        let facts = Facts::new();
        facts.set("WH.want_vacation",   Value::Boolean(false));
        facts.set("WH.grid_connected",  Value::Boolean(grid_connected));
        facts.set("WH.soc_low",         Value::Boolean(soc_low));
        facts.set("WH.irradiance_low",  Value::Boolean(irradiance_low));
        facts.set("WH.temp_max_reached", Value::Boolean(temp_max_reached));
        facts.set("WH.target_mode",     Value::String("VACATION".to_string()));

        self.engine
            .execute(&facts)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn e() -> WaterHeaterRuleEngine { WaterHeaterRuleEngine::new().unwrap() }

    fn eval(grid: bool, soc_low: bool, irr_low: bool, temp_max: bool) -> String {
        e().evaluate(grid, soc_low, irr_low, temp_max).unwrap()
    }

    #[test]
    fn all_ok_gives_heat_pump() {
        assert_eq!(eval(false, false, false, false), "HEAT_PUMP");
    }

    #[test]
    fn grid_connected_forces_vacation() {
        assert_eq!(eval(true, false, false, false), "VACATION");
    }

    #[test]
    fn soc_low_forces_vacation() {
        assert_eq!(eval(false, true, false, false), "VACATION");
    }

    #[test]
    fn irradiance_low_forces_vacation() {
        assert_eq!(eval(false, false, true, false), "VACATION");
    }

    #[test]
    fn temp_max_reached_forces_vacation() {
        assert_eq!(eval(false, false, false, true), "VACATION");
    }

    #[test]
    fn all_bad_forces_vacation() {
        assert_eq!(eval(true, true, true, true), "VACATION");
    }
}
