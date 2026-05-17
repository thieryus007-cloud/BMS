//! Reader — snapshots MVCC lock-free. Plan §5.4-5.5.

use std::sync::Arc;

use anyhow::Result;
use redb::{Database, ReadableDatabase, ReadableTable};

use crate::encoding::{dec_skey, enc_skey, make_lookup_key};
use crate::tables::{
    AggBucket, SeriesMeta, Tier, TABLE_DAILY, TABLE_HOURLY, TABLE_RAW, TABLE_SERIES_BY_KEY,
    TABLE_SERIES_META,
};

#[derive(Clone)]
pub struct Reader {
    pub(crate) db: Arc<Database>,
}

impl Reader {
    /// Construit un Reader à partir d'une base redb existante — utilisé par
    /// les benches et `metrics-cli`. Le chemin normal reste
    /// `MetricsStore::reader()`.
    pub fn from_db(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Expose une transaction de lecture pour les usages avancés (ex: tests
    /// qui inspectent plusieurs tables dans le même snapshot).
    pub fn begin_read(&self) -> Result<redb::ReadTransaction> {
        Ok(self.db.begin_read()?)
    }

    /// Range scan sur `metrics_raw`. Renvoie les points (ts_ms, value) triés
    /// par timestamp croissant.
    pub fn query_range_raw(
        &self,
        series_id: u32,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<(i64, f64)>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(TABLE_RAW)?;
        let k_lo = enc_skey(series_id, from_ms);
        let k_hi = enc_skey(series_id, to_ms);
        let mut out = Vec::new();
        for entry in t.range::<&[u8]>(&k_lo[..]..=&k_hi[..])? {
            let (k, v) = entry?;
            let (_sid, ts) = dec_skey(k.value());
            out.push((ts, v.value()));
        }
        Ok(out)
    }

    /// Range scan sur les tables compactées (hourly/daily). Renvoie les
    /// `AggBucket` désérialisés.
    pub fn query_range_buckets(
        &self,
        series_id: u32,
        from_ms: i64,
        to_ms: i64,
        tier: Tier,
    ) -> Result<Vec<(i64, AggBucket)>> {
        let rtx = self.db.begin_read()?;
        let table = match tier {
            Tier::Raw => anyhow::bail!("query_range_buckets: Tier::Raw non supporté"),
            Tier::Hourly => TABLE_HOURLY,
            Tier::Daily => TABLE_DAILY,
        };
        let t = rtx.open_table(table)?;
        let k_lo = enc_skey(series_id, from_ms);
        let k_hi = enc_skey(series_id, to_ms);
        let mut out = Vec::new();
        for entry in t.range::<&[u8]>(&k_lo[..]..=&k_hi[..])? {
            let (k, v) = entry?;
            let (_sid, ts) = dec_skey(k.value());
            let bucket: AggBucket = bincode::deserialize(v.value())?;
            out.push((ts, bucket));
        }
        Ok(out)
    }

    /// Lookup `series_id` à partir de `(metric, labels_json canonique)`.
    pub fn lookup_series_id(&self, metric: &str, labels_json: &str) -> Result<Option<u32>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(TABLE_SERIES_BY_KEY)?;
        let key = make_lookup_key(metric, labels_json);
        Ok(t.get(&key[..])?.map(|g| g.value()))
    }

    /// Dump complet du catalogue de séries (utilisé par `/api/v1/series` et
    /// par `metrics-cli list-series`).
    pub fn list_series(&self) -> Result<Vec<(u32, SeriesMeta)>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(TABLE_SERIES_META)?;
        let mut out = Vec::new();
        for entry in t.iter()? {
            let (k, v) = entry?;
            let meta: SeriesMeta = bincode::deserialize(v.value())?;
            out.push((k.value(), meta));
        }
        Ok(out)
    }
}
