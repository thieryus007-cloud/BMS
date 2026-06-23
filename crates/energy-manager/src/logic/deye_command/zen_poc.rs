//! POC — DEYE decision via the GoRules / Zen engine (`zen-engine`), feature `zen-poc`.
//!
//! Same stateless decision as `rules.rs` (the production `rust-rule-engine` path), but
//! expressed as a JDM **decision table** (`rules/deye_command.zen.json`). Built ONLY to
//! compare ergonomics + binary impact — not wired into the running service. A parity test
//! asserts it returns exactly the same answers as `DeyeRuleEngine` over all 16 input combos.
//!
//! Exercised only from the test module (the engine is never wired into the running service),
//! hence the module-wide dead-code allowance.
#![allow(dead_code)]

use std::sync::Arc;

use anyhow::{anyhow, Context};
use serde_json::json;
use zen_engine::model::DecisionContent;
use zen_engine::DecisionEngine;

const ZEN_JSON: &str = include_str!("../../../rules/deye_command.zen.json");

pub struct DeyeZenEngine {
    engine: DecisionEngine,
    content: Arc<DecisionContent>,
}

impl DeyeZenEngine {
    pub fn new() -> anyhow::Result<Self> {
        let content: DecisionContent =
            serde_json::from_str(ZEN_JSON).context("parse deye_command.zen.json")?;
        Ok(Self {
            engine: DecisionEngine::default(),
            content: Arc::new(content),
        })
    }

    /// Returns the desired relay state (true = ON) from the current state + pre-computed flags.
    /// Async because Zen evaluation is async (unlike the synchronous `rust-rule-engine` path).
    pub async fn decide(
        &self,
        relay_on: bool,
        hard_cut: bool,
        cut_debounced: bool,
        restore_debounced: bool,
    ) -> anyhow::Result<bool> {
        let decision = self.engine.create_decision(self.content.clone());
        let input = json!({
            "relay_on": relay_on,
            "hard_cut": hard_cut,
            "cut_debounced": cut_debounced,
            "restore_debounced": restore_debounced,
        });
        let resp = decision
            .evaluate(input.into())
            .await
            .map_err(|e| anyhow!("zen evaluate failed: {e:?}"))?;
        let out = serde_json::to_value(&resp.result).context("serialize zen result")?;
        out.get("desired_on")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| anyhow!("zen result missing boolean `desired_on`: {out}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference decision (source of truth) — same as `rules.rs`.
    fn logic(relay_on: bool, hard_cut: bool, cut_debounced: bool, restore_debounced: bool) -> bool {
        if relay_on { !(hard_cut || cut_debounced) } else { restore_debounced }
    }

    /// POC result: the Zen JDM decision table evaluates the decision **correctly on all 16
    /// combinations** out of the box — no salience/ordering footgun (contrast with
    /// rust-rule-engine, which needed distinct saliences to avoid 7/16 wrong answers).
    #[tokio::test]
    async fn zen_matches_reference_logic_on_all_combos() {
        let zen = DeyeZenEngine::new().expect("zen engine init");

        for bits in 0u8..16 {
            let r = bits & 1 != 0;
            let h = bits & 2 != 0;
            let c = bits & 4 != 0;
            let rd = bits & 8 != 0;

            let got = zen.decide(r, h, c, rd).await.unwrap();
            assert_eq!(
                got,
                logic(r, h, c, rd),
                "combo relay_on={r} hard_cut={h} cut_debounced={c} restore_debounced={rd}"
            );
        }
    }
}
