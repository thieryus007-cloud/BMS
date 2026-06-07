//! Encodage des clés composites `(series_id u32, ts_ms i64)` en `[u8; 12]`
//! big-endian. Cf. docs/metriques-redb-architecture.md §4.1.

pub const SKEY_LEN: usize = 12;
const TS_SIGN_FLIP: u64 = 0x8000_0000_0000_0000;

pub fn enc_skey(series_id: u32, ts_ms: i64) -> [u8; SKEY_LEN] {
    let mut k = [0u8; SKEY_LEN];
    k[0..4].copy_from_slice(&series_id.to_be_bytes());
    k[4..12].copy_from_slice(&((ts_ms as u64) ^ TS_SIGN_FLIP).to_be_bytes());
    k
}

pub fn dec_skey(k: &[u8]) -> (u32, i64) {
    debug_assert_eq!(k.len(), SKEY_LEN);
    let s = u32::from_be_bytes(k[0..4].try_into().unwrap());
    let raw = u64::from_be_bytes(k[4..12].try_into().unwrap());
    let ts = (raw ^ TS_SIGN_FLIP) as i64;
    (s, ts)
}

/// Construit la clé de l'index inverse `series_by_key` :
/// `metric + 0x00 + labels_json` (ordre lexicographique stable).
pub fn make_lookup_key(metric: &str, labels_json: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(metric.len() + 1 + labels_json.len());
    k.extend_from_slice(metric.as_bytes());
    k.push(0);
    k.extend_from_slice(labels_json.as_bytes());
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_basic() {
        for (s, t) in [(0u32, 0i64), (1, 1), (42, 1_700_000_000_000), (u32::MAX, i64::MAX)] {
            let k = enc_skey(s, t);
            assert_eq!(dec_skey(&k), (s, t));
        }
    }

    #[test]
    fn roundtrip_negative_ts() {
        let k = enc_skey(7, -1);
        assert_eq!(dec_skey(&k), (7, -1));
    }

    #[test]
    fn lexicographic_order_matches_ts_order() {
        let s = 5;
        let k1 = enc_skey(s, 100);
        let k2 = enc_skey(s, 200);
        let k3 = enc_skey(s, 300);
        assert!(k1 < k2 && k2 < k3);
    }

    #[test]
    fn series_id_dominates_ordering() {
        let a = enc_skey(1, i64::MAX);
        let b = enc_skey(2, i64::MIN);
        assert!(a < b);
    }
}
