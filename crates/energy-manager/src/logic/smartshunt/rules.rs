//! SmartShunt daily-baseline capture decision — **pure Rust** (was `rules/smartshunt.grl`).

#[derive(Debug, Default)]
pub struct BaselineDecision {
    pub capture_charged: bool,
    pub capture_discharged: bool,
}

/// Capture the start-of-day cumulative energy baseline on a new calendar day OR when none is
/// stored yet (first message after start/reset). Charged and discharged are decided independently.
pub fn baseline_decision(
    charged_new_day: bool,
    charged_baseline_absent: bool,
    discharged_new_day: bool,
    discharged_baseline_absent: bool,
) -> BaselineDecision {
    BaselineDecision {
        capture_charged: charged_new_day || charged_baseline_absent,
        capture_discharged: discharged_new_day || discharged_baseline_absent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_day_or_missing_captures_charged() {
        assert!(baseline_decision(true, false, false, false).capture_charged);
        assert!(baseline_decision(false, true, false, false).capture_charged);
    }

    #[test]
    fn new_day_or_missing_captures_discharged() {
        assert!(baseline_decision(false, false, true, false).capture_discharged);
        assert!(baseline_decision(false, false, false, true).capture_discharged);
    }

    #[test]
    fn all_false_captures_nothing() {
        let d = baseline_decision(false, false, false, false);
        assert!(!d.capture_charged);
        assert!(!d.capture_discharged);
    }

    #[test]
    fn all_true_captures_both() {
        let d = baseline_decision(true, true, true, true);
        assert!(d.capture_charged);
        assert!(d.capture_discharged);
    }
}
