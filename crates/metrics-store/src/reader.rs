//! Reader — snapshots MVCC lock-free. Plan §5.4-5.5.

use std::sync::Arc;

use anyhow::Result;
use redb::{Database, ReadTransaction, ReadableDatabase, ReadableTable};

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
    /// qui inspectent plusieurs tables dans le même snapshot ; évaluateur
    /// PromQL qui réutilise une seule rtx pour toute la durée d'`eval_range`).
    pub fn begin_read(&self) -> Result<ReadTransaction> {
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
        query_range_raw_inner(&rtx, series_id, from_ms, to_ms)
    }

    /// Variante qui réutilise une `ReadTransaction` existante — évite
    /// l'overhead `begin_read` répété quand on boucle sur de nombreux steps
    /// (cf. eval_range PromQL, ~200 itérations × N séries).
    pub fn query_range_raw_with_tx(
        &self,
        rtx: &ReadTransaction,
        series_id: u32,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<(i64, f64)>> {
        query_range_raw_inner(rtx, series_id, from_ms, to_ms)
    }

    /// Dernier point `(ts, value)` de `[from_ms, to_ms]` (le plus récent).
    ///
    /// Évite de matérialiser toute la fenêtre quand seul le dernier point
    /// est nécessaire (instant vector selector PromQL). Scan inverse O(1)
    /// au lieu d'allouer un `Vec` de toute la fenêtre de lookback.
    pub fn last_point_in_range_with_tx(
        &self,
        rtx: &ReadTransaction,
        series_id: u32,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Option<(i64, f64)>> {
        last_point_in_range_inner(rtx, series_id, from_ms, to_ms)
    }

    /// Premier et dernier point de `[from_ms, to_ms]`. Utilisé par
    /// `increase` / `rate` / `delta` / `last_over_time` qui n'ont besoin que
    /// des deux bornes — évite de charger toute la fenêtre.
    #[allow(clippy::type_complexity)]
    pub fn first_last_in_range_with_tx(
        &self,
        rtx: &ReadTransaction,
        series_id: u32,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Option<((i64, f64), (i64, f64))>> {
        first_last_in_range_inner(rtx, series_id, from_ms, to_ms)
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
        query_range_buckets_inner(&rtx, series_id, from_ms, to_ms, tier)
    }

    /// Variante avec rtx partagée — cf. `query_range_raw_with_tx`.
    pub fn query_range_buckets_with_tx(
        &self,
        rtx: &ReadTransaction,
        series_id: u32,
        from_ms: i64,
        to_ms: i64,
        tier: Tier,
    ) -> Result<Vec<(i64, AggBucket)>> {
        query_range_buckets_inner(rtx, series_id, from_ms, to_ms, tier)
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
        list_series_inner(&rtx)
    }

    /// Variante avec rtx partagée — cf. `query_range_raw_with_tx`.
    pub fn list_series_with_tx(&self, rtx: &ReadTransaction) -> Result<Vec<(u32, SeriesMeta)>> {
        list_series_inner(rtx)
    }
}

fn query_range_raw_inner(
    rtx: &ReadTransaction,
    series_id: u32,
    from_ms: i64,
    to_ms: i64,
) -> Result<Vec<(i64, f64)>> {
    // Intervalle inversé → résultat vide (évite un panic redb sur bornes
    // de range décroissantes).
    if from_ms > to_ms {
        return Ok(Vec::new());
    }
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

fn last_point_in_range_inner(
    rtx: &ReadTransaction,
    series_id: u32,
    from_ms: i64,
    to_ms: i64,
) -> Result<Option<(i64, f64)>> {
    // Garde anti-panic : bornes inversées → aucun point.
    if from_ms > to_ms {
        return Ok(None);
    }
    let t = rtx.open_table(TABLE_RAW)?;
    let k_lo = enc_skey(series_id, from_ms);
    let k_hi = enc_skey(series_id, to_ms);
    let mut range = t.range::<&[u8]>(&k_lo[..]..=&k_hi[..])?;
    match range.next_back() {
        Some(entry) => {
            let (k, v) = entry?;
            let (_sid, ts) = dec_skey(k.value());
            Ok(Some((ts, v.value())))
        }
        None => Ok(None),
    }
}

#[allow(clippy::type_complexity)]
fn first_last_in_range_inner(
    rtx: &ReadTransaction,
    series_id: u32,
    from_ms: i64,
    to_ms: i64,
) -> Result<Option<((i64, f64), (i64, f64))>> {
    // Garde anti-panic : bornes inversées → aucun point.
    if from_ms > to_ms {
        return Ok(None);
    }
    let t = rtx.open_table(TABLE_RAW)?;
    let k_lo = enc_skey(series_id, from_ms);
    let k_hi = enc_skey(series_id, to_ms);
    let mut range = t.range::<&[u8]>(&k_lo[..]..=&k_hi[..])?;
    let first = match range.next() {
        Some(entry) => {
            let (k, v) = entry?;
            let (_sid, ts) = dec_skey(k.value());
            (ts, v.value())
        }
        None => return Ok(None),
    };
    // `next_back` sur l'itérateur déjà avancé : renvoie le dernier point
    // restant, ou `None` s'il n'y avait qu'un seul élément (alors last=first).
    let last = match range.next_back() {
        Some(entry) => {
            let (k, v) = entry?;
            let (_sid, ts) = dec_skey(k.value());
            (ts, v.value())
        }
        None => first,
    };
    Ok(Some((first, last)))
}

fn query_range_buckets_inner(
    rtx: &ReadTransaction,
    series_id: u32,
    from_ms: i64,
    to_ms: i64,
    tier: Tier,
) -> Result<Vec<(i64, AggBucket)>> {
    if from_ms > to_ms {
        return Ok(Vec::new());
    }
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
        // M2 : un bucket corrompu ne doit pas avorter toute la requête —
        // on le journalise et on le saute (dégradation gracieuse).
        match bincode::deserialize::<AggBucket>(v.value()) {
            Ok(bucket) => out.push((ts, bucket)),
            Err(e) => tracing::warn!(
                series_id = _sid,
                ts,
                error = %e,
                "metrics-store: bucket corrompu ignoré"
            ),
        }
    }
    Ok(out)
}

fn list_series_inner(rtx: &ReadTransaction) -> Result<Vec<(u32, SeriesMeta)>> {
    let t = rtx.open_table(TABLE_SERIES_META)?;
    let mut out = Vec::new();
    for entry in t.iter()? {
        let (k, v) = entry?;
        // M2 : skip-and-log un meta corrompu plutôt que d'avorter le dump
        // complet du catalogue.
        match bincode::deserialize::<SeriesMeta>(v.value()) {
            Ok(meta) => out.push((k.value(), meta)),
            Err(e) => tracing::warn!(
                series_id = k.value(),
                error = %e,
                "metrics-store: SeriesMeta corrompu ignoré"
            ),
        }
    }
    Ok(out)
}
