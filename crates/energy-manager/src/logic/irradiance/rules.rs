//! Irradiance sensor validation — **pure Rust** (was `rules/irradiance.grl`).

/// Valid irradiance range in W/m². Readings outside this band are rejected.
const MIN_WM2: f64 = 0.0;
const MAX_WM2: f64 = 2000.0;

/// True when `raw` is a plausible irradiance reading (`0..=2000` W/m²).
pub fn validate(raw: f64) -> bool {
    (MIN_WM2..=MAX_WM2).contains(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_range_is_valid() {
        assert!(validate(0.0));
        assert!(validate(750.0));
        assert!(validate(2000.0));
    }

    #[test]
    fn out_of_range_is_invalid() {
        assert!(!validate(-1.0));
        assert!(!validate(2001.0));
    }
}
