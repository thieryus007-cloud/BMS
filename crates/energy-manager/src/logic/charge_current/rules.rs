//! Charge-mode selection — **pure Rust** (was `rules/charge_current.grl`).

/// Charge mode from grid state + PV excess (both pre-computed by the caller):
/// - off-grid (`IgnoreAcIn1 == 1`) → `"offgrid"` (PV excess is irrelevant off-grid)
/// - grid + PV excess              → `"grid_pv_excess"`
/// - grid, no excess               → `"grid_no_excess"`
pub fn evaluate(offgrid: bool, pv_excess: bool) -> &'static str {
    if offgrid {
        "offgrid"
    } else if pv_excess {
        "grid_pv_excess"
    } else {
        "grid_no_excess"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offgrid_ignores_pv_excess() {
        assert_eq!(evaluate(true, false), "offgrid");
        assert_eq!(evaluate(true, true), "offgrid");
    }

    #[test]
    fn grid_with_pv_excess() {
        assert_eq!(evaluate(false, true), "grid_pv_excess");
    }

    #[test]
    fn grid_no_excess() {
        assert_eq!(evaluate(false, false), "grid_no_excess");
    }
}
