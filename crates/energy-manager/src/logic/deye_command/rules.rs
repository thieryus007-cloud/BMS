//! Stateless DEYE relay decision — **pure Rust** (was `rules/deye_command.grl`).
//!
//! The Rust side (`mod.rs`) carries the relay state + the two debounce timers and pre-computes
//! the boolean flags below; this function just MAPS them to the desired relay state. It holds
//! NO state of its own: given the same flags it always returns the same answer, so the relay
//! can never get stuck.

/// Returns the desired relay state (`true` = ON) from the current state + pre-computed flags.
///
/// - `hard_cut`          : `freq >= freq_hard_hz` → immediate cut
/// - `cut_debounced`     : the cut condition (high freq OR MPPT full) has held `cut_delay_secs`
/// - `restore_debounced` : the clear condition (low freq AND not full) has held `reenable_delay_secs`
///
/// While ON, the relay cuts on a hard spike OR a debounced cut. While OFF, it restores once the
/// clear condition has been debounced. Otherwise the relay state is unchanged.
pub fn decide(relay_on: bool, hard_cut: bool, cut_debounced: bool, restore_debounced: bool) -> bool {
    if relay_on {
        !(hard_cut || cut_debounced)
    } else {
        restore_debounced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exhaustive guard over all 16 flag combinations against a hand-written truth table.
    ///
    /// History: this decision used to run on `rust-rule-engine`, whose `no-loop` modifier
    /// mis-fired with several rules present (7/16 combos wrong). Pure Rust removes the engine
    /// (and the footgun) entirely; this table locks the behaviour in. Index bits:
    /// bit0=relay_on, bit1=hard_cut, bit2=cut_debounced, bit3=restore_debounced.
    #[test]
    fn matches_truth_table_on_all_combos() {
        const EXPECTED: [bool; 16] = [
            false, true, false, false, false, false, false, false,
            true, true, true, false, true, false, true, false,
        ];
        for bits in 0u8..16 {
            let r = bits & 1 != 0;
            let h = bits & 2 != 0;
            let c = bits & 4 != 0;
            let rd = bits & 8 != 0;
            assert_eq!(
                decide(r, h, c, rd),
                EXPECTED[bits as usize],
                "combo relay_on={r} hard_cut={h} cut_debounced={c} restore_debounced={rd}"
            );
        }
    }

    #[test]
    fn on_stays_on_when_clear() {
        assert!(decide(true, false, false, false));
    }

    #[test]
    fn on_cuts_on_hard_spike() {
        assert!(!decide(true, true, false, false));
    }

    #[test]
    fn on_cuts_when_debounced() {
        assert!(!decide(true, false, true, false));
    }

    #[test]
    fn off_stays_off_until_restore_debounced() {
        assert!(!decide(false, false, false, false));
        assert!(decide(false, false, false, true));
    }

    #[test]
    fn off_ignores_cut_flags() {
        // Cut flags are irrelevant while already OFF; only restore_debounced matters.
        assert!(!decide(false, true, true, false));
    }
}
