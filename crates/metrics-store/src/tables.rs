//! Définitions des tables redb. Cf. plan_migration_vm_redb.md §4.2.

use redb::TableDefinition;
use serde::{Deserialize, Serialize};

/// Index inverse : `(metric + 0x00 + labels_json)` → `series_id`.
pub const TABLE_SERIES_BY_KEY: TableDefinition<&[u8], u32> =
    TableDefinition::new("series_by_key");

/// Métadonnées par série (`SeriesMeta` sérialisée en bincode).
pub const TABLE_SERIES_META: TableDefinition<u32, &[u8]> =
    TableDefinition::new("series_meta");

/// Compteur monotone — clé `"next_series_id"`.
pub const TABLE_META: TableDefinition<&str, u64> = TableDefinition::new("meta");

/// Points bruts : clé = `enc_skey(series_id, ts_ms)`.
pub const TABLE_RAW: TableDefinition<&[u8], f64> = TableDefinition::new("metrics_raw");

/// Points compactés horaires : clé = `enc_skey(series_id, bucket_ms)`,
/// valeur = `AggBucket` bincode.
pub const TABLE_HOURLY: TableDefinition<&[u8], &[u8]> = TableDefinition::new("metrics_hourly");

/// Points compactés journaliers.
pub const TABLE_DAILY: TableDefinition<&[u8], &[u8]> = TableDefinition::new("metrics_daily");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesMeta {
    pub metric: String,
    pub labels_json: String,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AggBucket {
    pub avg: f64,
    pub min: f64,
    pub max: f64,
    pub sum: f64,
    /// Première valeur de la fenêtre — nécessaire pour `increase()` après compaction.
    pub first: f64,
    /// Dernière valeur de la fenêtre — idem.
    pub last: f64,
    pub cnt: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Raw,
    Hourly,
    Daily,
}
