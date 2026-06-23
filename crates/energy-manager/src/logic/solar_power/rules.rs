//! Solar PV inverter daily-baseline decision — **pure Rust** (was `rules/solar_power.grl`).

#[derive(Debug, Default)]
pub struct BaselineDecision {
    /// Clear the stored baseline (new calendar day).
    pub reset: bool,
    /// Capture the current kWh counter as the new start-of-day baseline.
    pub capture: bool,
}

/// New calendar day → reset the stored baseline AND capture a fresh one. Within the same day,
/// an absent baseline (restart / first message) → capture without resetting.
pub fn baseline_decision(new_day: bool, baseline_absent: bool) -> BaselineDecision {
    BaselineDecision {
        reset: new_day,
        capture: new_day || baseline_absent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_day_resets_and_captures() {
        let d = baseline_decision(true, false);
        assert!(d.reset);
        assert!(d.capture);
    }

    #[test]
    fn absent_baseline_captures_no_reset() {
        let d = baseline_decision(false, true);
        assert!(!d.reset);
        assert!(d.capture);
    }

    #[test]
    fn neither_flag_no_action() {
        let d = baseline_decision(false, false);
        assert!(!d.reset);
        assert!(!d.capture);
    }

    #[test]
    fn new_day_and_absent_both_trigger() {
        let d = baseline_decision(true, true);
        assert!(d.reset);
        assert!(d.capture);
    }
}
