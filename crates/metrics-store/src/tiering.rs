//! Tiering — compaction `raw → hourly → daily`. Plan §9.1.
//!
//! Trois opérations :
//! - [`compact_raw_to_hourly`] : groupe les points raw < cutoff par bucket
//!   horaire, écrit dans `TABLE_HOURLY`, supprime les raws compactés.
//! - [`compact_hourly_to_daily`] : agrège les `AggBucket` horaires < cutoff
//!   par bucket journalier dans `TABLE_DAILY`, supprime les hourlies
//!   compactés.
//! - [`spawn_maintenance`] : tâche tokio qui exécute périodiquement les deux
//!   compactions selon la `TierPolicy`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use redb::{Database, ReadableDatabase, ReadableTable};
use tokio::task::JoinHandle;

use crate::encoding::{dec_skey, enc_skey, SKEY_LEN};
use crate::tables::{AggBucket, TABLE_DAILY, TABLE_HOURLY, TABLE_RAW};
use crate::TierPolicy;

pub const HOURLY_MS: i64 = 3_600_000;
pub const DAILY_MS: i64 = 86_400_000;

/// Builder mutable d'un bucket d'agrégation.
///
/// Accepte à la fois des points raw (`accumulate`) et des buckets
/// pré-agrégés (`merge_bucket`), ce qui permet de chaîner les compactions
/// `raw → hourly → daily` sans relire les raws après leur purge.
#[derive(Debug, Default)]
pub struct AggBucketBuilder {
    sum: f64,
    min: f64,
    max: f64,
    cnt: u32,
    first_ts: i64,
    first_val: f64,
    last_ts: i64,
    last_val: f64,
}

impl AggBucketBuilder {
    pub fn accumulate(&mut self, val: f64, ts: i64) {
        if self.cnt == 0 {
            self.min = val;
            self.max = val;
            self.first_ts = ts;
            self.first_val = val;
            self.last_ts = ts;
            self.last_val = val;
        } else {
            if val < self.min {
                self.min = val;
            }
            if val > self.max {
                self.max = val;
            }
            if ts < self.first_ts {
                self.first_ts = ts;
                self.first_val = val;
            }
            if ts > self.last_ts {
                self.last_ts = ts;
                self.last_val = val;
            }
        }
        self.sum += val;
        self.cnt += 1;
    }

    /// Absorbe un `AggBucket` déjà calculé positionné à `bucket_ts`.
    pub fn merge_bucket(&mut self, b: &AggBucket, bucket_ts: i64) {
        if self.cnt == 0 {
            self.min = b.min;
            self.max = b.max;
            self.first_ts = bucket_ts;
            self.first_val = b.first;
            self.last_ts = bucket_ts;
            self.last_val = b.last;
        } else {
            if b.min < self.min {
                self.min = b.min;
            }
            if b.max > self.max {
                self.max = b.max;
            }
            if bucket_ts < self.first_ts {
                self.first_ts = bucket_ts;
                self.first_val = b.first;
            }
            if bucket_ts > self.last_ts {
                self.last_ts = bucket_ts;
                self.last_val = b.last;
            }
        }
        self.sum += b.sum;
        self.cnt += b.cnt;
    }

    pub fn finalize(self) -> AggBucket {
        let cnt_f = self.cnt as f64;
        AggBucket {
            avg: if self.cnt > 0 { self.sum / cnt_f } else { 0.0 },
            min: self.min,
            max: self.max,
            sum: self.sum,
            first: self.first_val,
            last: self.last_val,
            cnt: self.cnt,
        }
    }
}

fn bucket_floor(ts_ms: i64, bucket_ms: i64) -> i64 {
    // Division Euclidienne pour que les ts négatifs (théoriques) tombent
    // sur le bord inférieur.
    ts_ms.div_euclid(bucket_ms) * bucket_ms
}

/// Statistiques retournées par les fonctions de compaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompactionStats {
    pub buckets_written: usize,
    pub points_purged: usize,
}

/// `raw → hourly` : agrège tous les points raw `ts < cutoff_ms`.
pub fn compact_raw_to_hourly(db: &Database, cutoff_ms: i64) -> Result<CompactionStats> {
    // 1. Phase lecture — agrège en mémoire. On mémorise le ts maximum
    //    réellement lu (`max_ts_read`) pour borner la purge (correctif R3).
    let mut buckets: HashMap<(u32, i64), AggBucketBuilder> = HashMap::new();
    let mut max_ts_read = i64::MIN;
    {
        let rtx = db.begin_read()?;
        let t_raw = rtx.open_table(TABLE_RAW)?;
        for entry in t_raw.iter()? {
            let (k, v) = entry?;
            let (sid, ts) = dec_skey(k.value());
            if ts >= cutoff_ms {
                continue;
            }
            if ts > max_ts_read {
                max_ts_read = ts;
            }
            let bucket = bucket_floor(ts, HOURLY_MS);
            buckets
                .entry((sid, bucket))
                .or_default()
                .accumulate(v.value(), ts);
        }
    }

    let buckets_written = buckets.len();
    if buckets_written == 0 {
        return Ok(CompactionStats::default());
    }

    // 2. Phase écriture des buckets — transaction unique. R4 : si un bucket
    //    existe déjà à cette clé (compaction partielle antérieure), on le
    //    fusionne au lieu de l'écraser (sinon perte des données déjà agrégées).
    {
        let wtx = db.begin_write()?;
        {
            let mut t_hour = wtx.open_table(TABLE_HOURLY)?;
            for ((sid, b), mut builder) in buckets.into_iter() {
                let key = enc_skey(sid, b);
                if let Some(existing) = t_hour.get(&key[..])? {
                    let prev: AggBucket = bincode::deserialize(existing.value())?;
                    drop(existing);
                    builder.merge_bucket(&prev, b);
                }
                let agg = builder.finalize();
                let bytes = bincode::serialize(&agg)?;
                t_hour.insert(&key[..], &bytes[..])?;
            }
        }
        wtx.commit()?;
    }

    // 3. Phase purge — transaction(s) séparée(s), bornée à `max_ts_read`
    //    (R3 : un point inséré tardivement avec un ts dans
    //    `]max_ts_read, cutoff[` survit pour la prochaine passe au lieu
    //    d'être purgé sans avoir été agrégé). Purge par lots (P3).
    let points_purged = purge_raw_before(db, max_ts_read.saturating_add(1))?;

    Ok(CompactionStats { buckets_written, points_purged })
}

/// `hourly → daily` : agrège tous les buckets horaires `ts < cutoff_ms`.
pub fn compact_hourly_to_daily(db: &Database, cutoff_ms: i64) -> Result<CompactionStats> {
    let mut buckets: HashMap<(u32, i64), AggBucketBuilder> = HashMap::new();
    let mut max_ts_read = i64::MIN;
    {
        let rtx = db.begin_read()?;
        let t_hour = rtx.open_table(TABLE_HOURLY)?;
        for entry in t_hour.iter()? {
            let (k, v) = entry?;
            let (sid, ts) = dec_skey(k.value());
            if ts >= cutoff_ms {
                continue;
            }
            if ts > max_ts_read {
                max_ts_read = ts;
            }
            let bucket = bucket_floor(ts, DAILY_MS);
            let hourly: AggBucket = bincode::deserialize(v.value())?;
            buckets
                .entry((sid, bucket))
                .or_default()
                .merge_bucket(&hourly, ts);
        }
    }

    let buckets_written = buckets.len();
    if buckets_written == 0 {
        return Ok(CompactionStats::default());
    }

    // R4 : fusionne avec le bucket journalier existant le cas échéant.
    {
        let wtx = db.begin_write()?;
        {
            let mut t_day = wtx.open_table(TABLE_DAILY)?;
            for ((sid, b), mut builder) in buckets.into_iter() {
                let key = enc_skey(sid, b);
                if let Some(existing) = t_day.get(&key[..])? {
                    let prev: AggBucket = bincode::deserialize(existing.value())?;
                    drop(existing);
                    builder.merge_bucket(&prev, b);
                }
                let agg = builder.finalize();
                let bytes = bincode::serialize(&agg)?;
                t_day.insert(&key[..], &bytes[..])?;
            }
        }
        wtx.commit()?;
    }

    // R3 + P3 : purge bornée au max lu, par lots.
    let points_purged = purge_hourly_before(db, max_ts_read.saturating_add(1))?;

    Ok(CompactionStats { buckets_written, points_purged })
}

/// Nombre max de clés supprimées par transaction de purge.
///
/// P3 : redb est mono-writer ; une purge en une seule transaction sur des
/// millions de points gèlerait l'ingestion pendant toute sa durée. On
/// découpe donc la suppression en lots bornés, en relâchant le write-lock
/// entre chaque commit pour laisser le thread writer intercaler ses batchs.
const PURGE_CHUNK: usize = 10_000;

/// Collecte (transaction lecture) les clés `< cutoff_ms`, puis les supprime
/// par lots bornés (transactions écriture courtes). Évite à la fois de tenir
/// un write-lock long (P3) et un `extract_if` global.
fn collect_keys_before(
    rtx: &redb::ReadTransaction,
    table: redb::TableDefinition<&[u8], f64>,
    cutoff_ms: i64,
) -> Result<Vec<[u8; SKEY_LEN]>> {
    let t = rtx.open_table(table)?;
    let mut keys = Vec::new();
    for entry in t.iter()? {
        let (k, _v) = entry?;
        let kb = k.value();
        let (_, ts) = dec_skey(kb);
        if ts < cutoff_ms {
            let mut arr = [0u8; SKEY_LEN];
            arr.copy_from_slice(kb);
            keys.push(arr);
        }
    }
    Ok(keys)
}

fn purge_raw_before(db: &Database, cutoff_ms: i64) -> Result<usize> {
    let keys = {
        let rtx = db.begin_read()?;
        collect_keys_before(&rtx, TABLE_RAW, cutoff_ms)?
    };
    let total = keys.len();
    for chunk in keys.chunks(PURGE_CHUNK) {
        let wtx = db.begin_write()?;
        {
            let mut t_raw = wtx.open_table(TABLE_RAW)?;
            for k in chunk {
                t_raw.remove(&k[..])?;
            }
        }
        wtx.commit()?;
    }
    Ok(total)
}

fn purge_hourly_before(db: &Database, cutoff_ms: i64) -> Result<usize> {
    // Collecte des clés à supprimer (lecture). La valeur de TABLE_HOURLY est
    // `&[u8]` ; on n'inspecte que la clé, donc on itère directement ici.
    let keys: Vec<[u8; SKEY_LEN]> = {
        let rtx = db.begin_read()?;
        let t = rtx.open_table(TABLE_HOURLY)?;
        let mut keys = Vec::new();
        for entry in t.iter()? {
            let (k, _v) = entry?;
            let kb = k.value();
            let (_, ts) = dec_skey(kb);
            if ts < cutoff_ms {
                let mut arr = [0u8; SKEY_LEN];
                arr.copy_from_slice(kb);
                keys.push(arr);
            }
        }
        keys
    };
    let total = keys.len();
    for chunk in keys.chunks(PURGE_CHUNK) {
        let wtx = db.begin_write()?;
        {
            let mut t_hour = wtx.open_table(TABLE_HOURLY)?;
            for k in chunk {
                t_hour.remove(&k[..])?;
            }
        }
        wtx.commit()?;
    }
    Ok(total)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Boucle de maintenance — exécute les deux compactions toutes les
/// `interval_hours` heures.
pub fn spawn_maintenance(
    db: Arc<Database>,
    policy: TierPolicy,
    interval_hours: u64,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let interval = Duration::from_secs(interval_hours.max(1) * 3600);
        // M4 : première passe peu après le démarrage (60 s, le temps que le
        // service finisse de booter) au lieu d'attendre un intervalle
        // complet — sinon un service qui redémarre plus souvent que
        // `interval_hours` ne compacterait jamais et la table raw croîtrait
        // sans borne.
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            let now = now_ms();
            let cutoff_raw = now - policy.raw_retention_days as i64 * DAILY_MS;
            let cutoff_hourly = now - policy.hourly_retention_days as i64 * DAILY_MS;

            let db_raw = db.clone();
            let raw_res = tokio::task::spawn_blocking(move || {
                compact_raw_to_hourly(&db_raw, cutoff_raw)
            })
            .await;
            match raw_res {
                Ok(Ok(s)) => tracing::info!(
                    buckets = s.buckets_written,
                    purged = s.points_purged,
                    "compact raw→hourly OK"
                ),
                Ok(Err(e)) => tracing::error!(error = %e, "compact raw→hourly failed"),
                Err(e) => tracing::error!(error = %e, "spawn_blocking raw→hourly panicked"),
            }

            let db_day = db.clone();
            let day_res = tokio::task::spawn_blocking(move || {
                compact_hourly_to_daily(&db_day, cutoff_hourly)
            })
            .await;
            match day_res {
                Ok(Ok(s)) => tracing::info!(
                    buckets = s.buckets_written,
                    purged = s.points_purged,
                    "compact hourly→daily OK"
                ),
                Ok(Err(e)) => tracing::error!(error = %e, "compact hourly→daily failed"),
                Err(e) => tracing::error!(error = %e, "spawn_blocking hourly→daily panicked"),
            }

            tokio::time::sleep(interval).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_accumulate_basic() {
        let mut b = AggBucketBuilder::default();
        b.accumulate(10.0, 1000);
        b.accumulate(20.0, 1001);
        b.accumulate(15.0, 999);
        let a = b.finalize();
        assert_eq!(a.cnt, 3);
        assert_eq!(a.sum, 45.0);
        assert_eq!(a.avg, 15.0);
        assert_eq!(a.min, 10.0);
        assert_eq!(a.max, 20.0);
        assert_eq!(a.first, 15.0); // ts=999 le plus tôt
        assert_eq!(a.last, 20.0); // ts=1001 le plus tard
    }

    #[test]
    fn builder_merge_buckets() {
        let h1 = AggBucket {
            avg: 10.0,
            min: 5.0,
            max: 15.0,
            sum: 100.0,
            first: 5.0,
            last: 12.0,
            cnt: 10,
        };
        let h2 = AggBucket {
            avg: 20.0,
            min: 8.0,
            max: 30.0,
            sum: 200.0,
            first: 25.0,
            last: 30.0,
            cnt: 10,
        };
        let mut b = AggBucketBuilder::default();
        b.merge_bucket(&h1, 0);
        b.merge_bucket(&h2, HOURLY_MS);
        let a = b.finalize();
        assert_eq!(a.cnt, 20);
        assert_eq!(a.sum, 300.0);
        assert_eq!(a.avg, 15.0);
        assert_eq!(a.min, 5.0);
        assert_eq!(a.max, 30.0);
        assert_eq!(a.first, 5.0); // h1 (bucket_ts=0)
        assert_eq!(a.last, 30.0); // h2 (bucket_ts=HOURLY_MS)
    }

    #[test]
    fn bucket_floor_aligns_to_hour() {
        assert_eq!(bucket_floor(0, HOURLY_MS), 0);
        assert_eq!(bucket_floor(HOURLY_MS - 1, HOURLY_MS), 0);
        assert_eq!(bucket_floor(HOURLY_MS, HOURLY_MS), HOURLY_MS);
        assert_eq!(bucket_floor(HOURLY_MS + 1, HOURLY_MS), HOURLY_MS);
        assert_eq!(bucket_floor(3 * HOURLY_MS + 42, HOURLY_MS), 3 * HOURLY_MS);
    }
}
