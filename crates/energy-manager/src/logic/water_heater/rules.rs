//! Water-heater mode decision — **pure Rust** (was `rules/water_heater.grl`).

/// Returns `"HEAT_PUMP"` or `"VACATION"`.
///
/// HEAT_PUMP requires ALL conditions clear. Any active condition forces VACATION:
/// - `grid_connected`   : grid ON (`ac_ignore == 0`)
/// - `soc_low`          : `soc_pct < soc_min_pct`
/// - `irradiance_low`   : `irradiance < irradiance_min_wm2`
/// - `temp_max_reached` : tank held `>= temp_max_c` for `temp_max_hold_secs` (cuve à température cible)
pub fn evaluate(
    grid_connected: bool,
    soc_low: bool,
    irradiance_low: bool,
    temp_max_reached: bool,
) -> &'static str {
    if grid_connected || soc_low || irradiance_low || temp_max_reached {
        "VACATION"
    } else {
        "HEAT_PUMP"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_ok_gives_heat_pump() {
        assert_eq!(evaluate(false, false, false, false), "HEAT_PUMP");
    }

    #[test]
    fn any_condition_forces_vacation() {
        assert_eq!(evaluate(true, false, false, false), "VACATION");
        assert_eq!(evaluate(false, true, false, false), "VACATION");
        assert_eq!(evaluate(false, false, true, false), "VACATION");
        assert_eq!(evaluate(false, false, false, true), "VACATION");
    }

    #[test]
    fn all_bad_forces_vacation() {
        assert_eq!(evaluate(true, true, true, true), "VACATION");
    }
}
