use anyhow::Context;
use rust_rule_engine::{Facts, KnowledgeBase, RustRuleEngine, Value};

const GRL: &str = include_str!("../../../rules/deye_command.grl");

pub struct DeyeRuleEngine {
    engine: RustRuleEngine,
}

#[derive(Debug, Default)]
pub struct DeyeDecision {
    /// New state to transition to, or None if no transition should occur
    pub next_state: Option<String>,
    /// Relay should be switched ON (restore DEYE)
    pub relay_on: bool,
    /// Relay should be switched OFF (cut DEYE)
    pub relay_off: bool,
}

impl DeyeRuleEngine {
    pub fn new() -> anyhow::Result<Self> {
        let kb = KnowledgeBase::new("deye_command");
        kb.add_rules_from_grl(GRL)
            .context("Failed to load deye_command.grl")?;
        Ok(Self {
            engine: RustRuleEngine::new(kb),
        })
    }

    /// Evaluates the state machine and returns the decision.
    ///
    /// All threshold comparisons are pre-computed in Rust and passed as bool flags
    /// to avoid fact-to-fact comparisons in GRL.
    pub fn evaluate(
        &mut self,
        state_str: &str,
        freq_hz: f64,
        time_in_state_secs: u64,
        grid_connected: bool,
        cfg_freq_high: f64,
        cfg_freq_low: f64,
        cfg_cut_delay: u64,
        cfg_reenable_delay: u64,
        lockout_expired: bool,
    ) -> anyhow::Result<DeyeDecision> {
        let facts = Facts::new();
        facts.set("DY.state",                  Value::String(state_str.to_string()));
        facts.set("DY.freq_high_exceeded",     Value::Boolean(freq_hz >= cfg_freq_high));
        facts.set("DY.freq_low_reached",       Value::Boolean(freq_hz <= cfg_freq_low));
        facts.set("DY.cut_delay_elapsed",      Value::Boolean(time_in_state_secs >= cfg_cut_delay));
        facts.set("DY.reenable_delay_elapsed", Value::Boolean(time_in_state_secs >= cfg_reenable_delay));
        facts.set("DY.lockout_expired",        Value::Boolean(lockout_expired));
        facts.set("DY.grid_connected",         Value::Boolean(grid_connected));

        self.engine
            .execute(&facts)
            .context("DEYE rule engine evaluation failed")?;

        let next_state = facts.get("DY.next_state").and_then(|v| match v {
            Value::String(s) => Some(s),
            _ => None,
        });
        let relay_on = facts.get("DY.relay_on").and_then(|v| match v {
            Value::Boolean(b) => Some(b),
            _ => None,
        }).unwrap_or(false);
        let relay_off = facts.get("DY.relay_off").and_then(|v| match v {
            Value::Boolean(b) => Some(b),
            _ => None,
        }).unwrap_or(false);

        Ok(DeyeDecision { next_state, relay_on, relay_off })
    }
}
