//! Fuzz du parseur de trames Daly (audit 2026-06 §17).
//!
//! Frontière d'entrée RS485 : tout octet du bus passe par
//! `ResponseFrame::parse`. Invariants vérifiés : jamais de panique, et si
//! une trame est acceptée, ses accesseurs sont cohérents avec les octets.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(frame) = daly_bms_core::protocol::ResponseFrame::parse(data) {
        // Accesseurs : ne doivent jamais paniquer et refléter les octets bruts.
        assert_eq!(frame.address(), data[1]);
        assert_eq!(frame.data_id(), data[2]);
        let _ = frame.data();
        // validate_for sur des attentes arbitraires : Result propre, pas de panique.
        let _ = frame.validate_for(data[1], daly_bms_core::protocol::DataId::PackStatus);
    }
});
