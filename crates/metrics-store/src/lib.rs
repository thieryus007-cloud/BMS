//! `metrics-store` — backend TSDB pure-Rust basé sur [`redb`].
//!
//! Cette crate est la seule TSDB du système : elle stocke les métriques
//! produites par `daly-bms-server` et `energy-manager`.
//! Cf. `docs/architecture-redb.md` pour la conception complète :
//!
//! - schéma de tables et encodage des clés
//! - API publique (`MetricsStore`, `Writer`, `Reader`)
//! - shim PromQL
//! - tiering raw→hourly→daily
//!
//! État Phase 0 :
//! - 0.3 squelette crate ✅
//! - 0.4 writer batché ✅
//! - 0.5a reader query_range + agrégats ✅
//! - 0.5b tiering — à venir
//! - 0.6 PromQL — à venir
//! - 0.7 prom_text — à venir
//! - 0.8 benches — à venir

pub mod agg;
pub mod encoding;
pub mod labels;
pub mod prom_text;
pub mod promql;
pub mod reader;
pub mod tables;
pub mod tiering;
pub mod writer;

use std::path::Path;
use std::sync::Arc;
use std::thread;

use anyhow::Result;
use redb::{Builder, Database};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub use agg::{agg_over_range, AggOp};
pub use reader::Reader;
pub use tables::{AggBucket, SeriesMeta, Tier};
pub use writer::{Sample, Writer, WriterConfig};

/// Options d'ouverture.
#[derive(Debug, Clone)]
pub struct Options {
    /// Taille du cache de pages redb (défaut 64 MiB, cf. §4.4).
    pub cache_bytes: usize,
    /// Profondeur de la queue mpsc entre producteurs et writer.
    pub writer_queue_depth: usize,
    /// Configuration du batching writer.
    pub writer: WriterConfig,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cache_bytes: 64 * 1024 * 1024,
            writer_queue_depth: 10_000,
            writer: WriterConfig::default(),
        }
    }
}

/// Politique de compaction `raw → hourly → daily`. Détails §9.1.
#[derive(Debug, Clone)]
pub struct TierPolicy {
    pub raw_retention_days: u32,
    pub hourly_retention_days: u32,
    pub daily_retention_days: u32,
}

impl Default for TierPolicy {
    fn default() -> Self {
        Self {
            raw_retention_days: 30,
            hourly_retention_days: 365,
            daily_retention_days: 5 * 365,
        }
    }
}

struct Inner {
    db: Arc<Database>,
    writer_tx: parking_lot::Mutex<Option<mpsc::Sender<writer::WriterMsg>>>,
    writer_handle: parking_lot::Mutex<Option<thread::JoinHandle<()>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // 1. Signale l'arrêt au thread writer via une sentinelle in-band.
        //    Contrairement à la simple fermeture du canal, cela arrête le
        //    writer **même si des `Writer` clones survivent** côté appelant
        //    (sinon `blocking_recv` ne retournerait jamais `None` et le
        //    `join()` ci-dessous bloquerait indéfiniment — correctif R2).
        if let Some(tx) = self.writer_tx.lock().take() {
            // `try_send` (non bloquant) : `Drop` peut s'exécuter dans un
            // runtime tokio où `blocking_send` paniquerait. Si la file est
            // pleine, on laisse au writer le temps de drainer puis on
            // réessaie (borné à ~100 ms) avant de relâcher le `Sender`.
            let mut msg = writer::WriterMsg::Shutdown;
            for _ in 0..50 {
                match tx.try_send(msg) {
                    Ok(()) => break,
                    Err(mpsc::error::TrySendError::Closed(_)) => break,
                    Err(mpsc::error::TrySendError::Full(m)) => {
                        msg = m;
                        thread::sleep(std::time::Duration::from_millis(2));
                    }
                }
            }
            drop(tx);
        }
        // 2. Attend la fin du thread writer pour relâcher le lock redb
        //    avant le `Drop` de `Arc<Database>`. Sans ce join, une
        //    réouverture immédiate de la base échouerait avec "Database
        //    already open".
        if let Some(h) = self.writer_handle.lock().take() {
            let _ = h.join();
        }
    }
}

#[derive(Clone)]
pub struct MetricsStore {
    inner: Arc<Inner>,
}

impl MetricsStore {
    /// Ouvre (ou crée) la base, initialise toutes les tables et démarre le
    /// thread writer.
    pub fn open(db_path: &Path, opts: Options) -> Result<Self> {
        let db = Builder::new()
            .set_cache_size(opts.cache_bytes)
            .create(db_path)?;
        let db = Arc::new(db);

        // Crée les tables si elles n'existent pas (cf. §4.4).
        {
            let tx = db.begin_write()?;
            let _ = tx.open_table(tables::TABLE_SERIES_BY_KEY)?;
            let _ = tx.open_table(tables::TABLE_SERIES_META)?;
            let _ = tx.open_table(tables::TABLE_META)?;
            let _ = tx.open_table(tables::TABLE_RAW)?;
            let _ = tx.open_table(tables::TABLE_HOURLY)?;
            let _ = tx.open_table(tables::TABLE_DAILY)?;
            tx.commit()?;
        }

        let (writer_tx, writer_rx) = mpsc::channel::<writer::WriterMsg>(opts.writer_queue_depth);
        let writer_handle = writer::spawn(db.clone(), writer_rx, opts.writer);

        Ok(Self {
            inner: Arc::new(Inner {
                db,
                writer_tx: parking_lot::Mutex::new(Some(writer_tx)),
                writer_handle: parking_lot::Mutex::new(Some(writer_handle)),
            }),
        })
    }

    pub fn writer(&self) -> Writer {
        let tx = self
            .inner
            .writer_tx
            .lock()
            .as_ref()
            .expect("writer channel encore ouvert tant que MetricsStore est vivant")
            .clone();
        Writer { tx }
    }

    pub fn reader(&self) -> Reader {
        Reader { db: self.inner.db.clone() }
    }

    /// Démarre la tâche de maintenance (compaction raw→hourly→daily).
    /// `interval_hours` contrôle la fréquence (défaut recommandé : 6 h ⇒
    /// 4 passes par jour). Cf. plan §9.1.
    pub fn spawn_maintenance(&self, policy: TierPolicy, interval_hours: u64) -> JoinHandle<()> {
        tiering::spawn_maintenance(self.inner.db.clone(), policy, interval_hours)
    }

    /// Lance une seule passe de compaction synchrone (utilisée par
    /// `metrics-cli compact` et les tests).
    pub fn compact_now(&self, policy: &TierPolicy) -> Result<tiering::CompactionStats> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        let cutoff_raw = now - policy.raw_retention_days as i64 * tiering::DAILY_MS;
        let cutoff_hourly = now - policy.hourly_retention_days as i64 * tiering::DAILY_MS;
        let s1 = tiering::compact_raw_to_hourly(&self.inner.db, cutoff_raw)?;
        let s2 = tiering::compact_hourly_to_daily(&self.inner.db, cutoff_hourly)?;
        Ok(tiering::CompactionStats {
            buckets_written: s1.buckets_written + s2.buckets_written,
            points_purged: s1.points_purged + s2.points_purged,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tmp_db_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        p.push(format!("metrics-store-{name}-{pid}-{nanos}.redb"));
        p
    }

    #[test]
    fn open_creates_all_tables() {
        let path = tmp_db_path("open");
        let _ = std::fs::remove_file(&path);

        let store = MetricsStore::open(&path, Options::default()).expect("open");

        let rtx = store.reader().begin_read().expect("read tx");
        rtx.open_table(tables::TABLE_RAW).expect("raw");
        rtx.open_table(tables::TABLE_HOURLY).expect("hourly");
        rtx.open_table(tables::TABLE_DAILY).expect("daily");
        rtx.open_table(tables::TABLE_SERIES_BY_KEY).expect("series_by_key");
        rtx.open_table(tables::TABLE_SERIES_META).expect("series_meta");
        rtx.open_table(tables::TABLE_META).expect("meta");
        drop(rtx);
        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = path;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn round_trip_write_then_read() {
        let path = tmp_db_path("rt");
        let _ = std::fs::remove_file(&path);

        // Bloc imbriqué : garantit que `writer` et `reader` sont droppés
        // AVANT `store` (ordre naturel inverse-déclaration), donc tous les
        // `Sender` sont fermés au moment où `Inner::drop` rejoint le thread.
        {
            let opts = Options {
                writer: WriterConfig {
                    batch_max: 4,
                    flush_ms: 50,
                    poll_idle_ms: 2,
                },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let writer = store.writer();

            // Trois séries : bms_v{bms_id=1}, bms_v{bms_id=2}, bms_soc{bms_id=1}.
            for ts in 1000..1010 {
                writer
                    .write(Sample::new("bms_v", ts, ts as f64 * 0.01).with_label("bms_id", "1"))
                    .await
                    .unwrap();
                writer
                    .write(Sample::new("bms_v", ts, ts as f64 * 0.02).with_label("bms_id", "2"))
                    .await
                    .unwrap();
                writer
                    .write(Sample::new("bms_soc", ts, 50.0).with_label("bms_id", "1"))
                    .await
                    .unwrap();
            }

            // Attend la persistance (≤ 2 flush windows).
            tokio::time::sleep(Duration::from_millis(300)).await;

            let reader = store.reader();
            let series = reader.list_series().unwrap();
            assert_eq!(series.len(), 3, "3 séries distinctes attendues, got: {series:?}");

            let id_bms_v_1 = reader
                .lookup_series_id("bms_v", r#"{"bms_id":"1"}"#)
                .unwrap()
                .expect("bms_v{bms_id=1} doit exister");
            let pts = reader.query_range_raw(id_bms_v_1, 1000, 1009).unwrap();
            assert_eq!(pts.len(), 10);
            assert_eq!(pts.first().unwrap().0, 1000);
            assert_eq!(pts.last().unwrap().0, 1009);
            assert!((pts.first().unwrap().1 - 10.00).abs() < 1e-9);

            // Agrégats
            let avg = agg_over_range(&reader, id_bms_v_1, 1000, 1009, AggOp::Avg)
                .unwrap()
                .unwrap();
            assert!((avg - 10.045).abs() < 1e-6, "avg = {avg}");
            let min = agg_over_range(&reader, id_bms_v_1, 1000, 1009, AggOp::Min)
                .unwrap()
                .unwrap();
            let max = agg_over_range(&reader, id_bms_v_1, 1000, 1009, AggOp::Max)
                .unwrap()
                .unwrap();
            assert!((min - 10.0).abs() < 1e-9);
            assert!((max - 10.09).abs() < 1e-6);

            // Bornes : un range vide doit retourner None pour Avg.
            let nothing = agg_over_range(&reader, id_bms_v_1, 5000, 6000, AggOp::Avg).unwrap();
            assert!(nothing.is_none());
        }
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tiering_raw_to_hourly_to_daily() {
        use crate::tiering::{compact_hourly_to_daily, compact_raw_to_hourly, DAILY_MS};

        let path = tmp_db_path("tier");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 64, flush_ms: 30, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let writer = store.writer();

            // 3 jours de samples : un point toutes les 10 minutes (144 par
            // jour) sur la série `power`. Toutes les valeurs sont = 100.0
            // donc on peut prédire trivialement les agrégats finaux.
            let day_ms = DAILY_MS;
            let step_ms = 10 * 60 * 1000_i64; // 10 minutes
            let t0 = 1_700_000_000_000_i64; // ancre arbitraire alignée jour
            let t0 = t0 - t0 % day_ms;
            for d in 0..3 {
                for i in 0..(day_ms / step_ms) {
                    let ts = t0 + d * day_ms + i * step_ms;
                    writer
                        .write(Sample::new("power", ts, 100.0))
                        .await
                        .unwrap();
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;

            let reader = store.reader();
            let series_id = reader.lookup_series_id("power", "{}").unwrap().unwrap();
            let raw_before = reader
                .query_range_raw(series_id, t0, t0 + 3 * day_ms)
                .unwrap();
            assert_eq!(raw_before.len(), 3 * 144);

            // Phase 1 : compact tous les raws → hourly (cutoff dans le futur).
            let cutoff = t0 + 3 * day_ms + 1;
            let stats = compact_raw_to_hourly(&store.inner.db, cutoff).unwrap();
            assert_eq!(stats.buckets_written, 3 * 24); // 24 buckets/jour × 3 j
            assert_eq!(stats.points_purged, 3 * 144);

            // Les raws doivent être vides après purge
            let raw_after = reader
                .query_range_raw(series_id, t0, t0 + 3 * day_ms)
                .unwrap();
            assert_eq!(raw_after.len(), 0);

            // Les hourlies doivent contenir 72 buckets de 6 points chacun
            // (60 min / 10 min = 6).
            let hourly = reader
                .query_range_buckets(series_id, t0, t0 + 3 * day_ms, Tier::Hourly)
                .unwrap();
            assert_eq!(hourly.len(), 3 * 24);
            for (_, bucket) in &hourly {
                assert_eq!(bucket.cnt, 6);
                assert!((bucket.avg - 100.0).abs() < 1e-9);
                assert_eq!(bucket.min, 100.0);
                assert_eq!(bucket.max, 100.0);
                assert!((bucket.sum - 600.0).abs() < 1e-9);
            }

            // Phase 2 : compact hourlies plus vieux que la fin du jour 2 → daily.
            let cutoff_daily = t0 + 2 * day_ms; // tout jour 0 et jour 1 → daily
            let stats2 = compact_hourly_to_daily(&store.inner.db, cutoff_daily).unwrap();
            assert_eq!(stats2.buckets_written, 2); // 2 jours
            assert_eq!(stats2.points_purged, 2 * 24); // 48 hourlies purgés

            // Reste : seul le jour 2 doit encore avoir 24 hourlies.
            let hourly_left = reader
                .query_range_buckets(series_id, t0, t0 + 3 * day_ms, Tier::Hourly)
                .unwrap();
            assert_eq!(hourly_left.len(), 24);

            // Daily : 2 buckets, 144 points cumulés chacun (24 hourlies × 6).
            let daily = reader
                .query_range_buckets(series_id, t0, t0 + 2 * day_ms, Tier::Daily)
                .unwrap();
            assert_eq!(daily.len(), 2);
            for (_, bucket) in &daily {
                assert_eq!(bucket.cnt, 144);
                assert!((bucket.avg - 100.0).abs() < 1e-9);
                assert_eq!(bucket.min, 100.0);
                assert_eq!(bucket.max, 100.0);
                assert!((bucket.sum - 14_400.0).abs() < 1e-9);
            }

            // Idempotence : refaire la compaction ne doit rien re-écrire.
            let again = compact_raw_to_hourly(&store.inner.db, cutoff).unwrap();
            assert_eq!(again.buckets_written, 0);
            assert_eq!(again.points_purged, 0);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn promql_smoke_selectors_agg_and_increase() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("promql");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 64, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let writer = store.writer();

            // bms_v{bms_id=1} : 10 pts à 1.0..1.9
            // bms_v{bms_id=2} : 10 pts à 2.0..2.9
            // energy_wh        : monotone, 0..100 par pas de 10 → increase = 100
            for i in 0..10_i64 {
                let ts = 100_000 + i * 1_000;
                writer
                    .write(Sample::new("bms_v", ts, 1.0 + i as f64 / 10.0).with_label("bms_id", "1"))
                    .await
                    .unwrap();
                writer
                    .write(Sample::new("bms_v", ts, 2.0 + i as f64 / 10.0).with_label("bms_id", "2"))
                    .await
                    .unwrap();
                writer
                    .write(Sample::new("energy_wh", ts, i as f64 * 10.0))
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);

            // Selector simple → 2 séries
            let expr = parse_and_validate("bms_v").unwrap();
            let v = ev.eval_instant(&expr, 109_000).unwrap();
            assert_eq!(v.len(), 2);

            // Selector matché par label → 1 série, valeur = 1.9
            let expr = parse_and_validate(r#"bms_v{bms_id="1"}"#).unwrap();
            let v = ev.eval_instant(&expr, 109_000).unwrap();
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 1.9).abs() < 1e-9);

            // Agg max(bms_v) → 1 série, valeur = 2.9 (max sur les 2 BMS)
            let expr = parse_and_validate("max(bms_v)").unwrap();
            let v = ev.eval_instant(&expr, 109_000).unwrap();
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 2.9).abs() < 1e-9);

            // Binary scalar / vec : bms_v / 10 → 2 séries, valeurs 0.19 et 0.29
            let expr = parse_and_validate("bms_v / 10").unwrap();
            let mut v = ev.eval_instant(&expr, 109_000).unwrap();
            v.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap());
            assert_eq!(v.len(), 2);
            assert!((v[0].value - 0.19).abs() < 1e-9);
            assert!((v[1].value - 0.29).abs() < 1e-9);

            // increase(energy_wh[10s]) → 90 (de 0 à 90 sur la fenêtre)
            let expr = parse_and_validate("increase(energy_wh[10s])").unwrap();
            let v = ev.eval_instant(&expr, 109_000).unwrap();
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 90.0).abs() < 1e-9, "got {}", v[0].value);

            // Range query : bms_v de 100s à 109s par pas de 3s → 4 points par série
            let expr = parse_and_validate("bms_v").unwrap();
            let series = ev.eval_range(&expr, 100_000, 109_000, 3_000).unwrap();
            assert_eq!(series.len(), 2);
            for s in &series {
                assert_eq!(s.samples.len(), 4, "labels = {:?}", s.labels);
            }
        }
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn promql_offset_modifier() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("promql_offset");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 64, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let writer = store.writer();

            // bms_v{bms_id=1} : 10 pts 1.0..1.9 (ts 100_000..109_000)
            // energy_wh        : monotone 0..90 par pas de 10
            for i in 0..10_i64 {
                let ts = 100_000 + i * 1_000;
                writer
                    .write(Sample::new("bms_v", ts, 1.0 + i as f64 / 10.0).with_label("bms_id", "1"))
                    .await
                    .unwrap();
                writer
                    .write(Sample::new("energy_wh", ts, i as f64 * 10.0))
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);

            // Référence sans offset à t=109_000 → 1.9
            let expr = parse_and_validate(r#"bms_v{bms_id="1"}"#).unwrap();
            let v = ev.eval_instant(&expr, 109_000).unwrap();
            assert!((v[0].value - 1.9).abs() < 1e-9);

            // offset 5s : évalue à t-5s = 104_000 → 1.4
            let expr = parse_and_validate(r#"bms_v{bms_id="1"} offset 5s"#).unwrap();
            let v = ev.eval_instant(&expr, 109_000).unwrap();
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 1.4).abs() < 1e-9, "offset instant got {}", v[0].value);

            // offset négatif (vers le futur) : à t=104_000 offset -5s = 109_000 → 1.9
            let expr = parse_and_validate(r#"bms_v{bms_id="1"} offset -5s"#).unwrap();
            let v = ev.eval_instant(&expr, 104_000).unwrap();
            assert!((v[0].value - 1.9).abs() < 1e-9, "offset neg got {}", v[0].value);

            // increase sans offset sur [10s] à 109_000 → 90
            let expr = parse_and_validate("increase(energy_wh[10s])").unwrap();
            let v = ev.eval_instant(&expr, 109_000).unwrap();
            assert!((v[0].value - 90.0).abs() < 1e-9, "increase got {}", v[0].value);

            // increase avec offset 5s : fenêtre décalée [94_000, 104_000] → 40
            let expr = parse_and_validate("increase(energy_wh[10s] offset 5s)").unwrap();
            let v = ev.eval_instant(&expr, 109_000).unwrap();
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 40.0).abs() < 1e-9, "increase offset got {}", v[0].value);

            // Différence N vs décalé (motif comparaison périodes) : 1.9 - 1.4 = 0.5
            let expr =
                parse_and_validate(r#"bms_v{bms_id="1"} - (bms_v{bms_id="1"} offset 5s)"#).unwrap();
            let v = ev.eval_instant(&expr, 109_000).unwrap();
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 0.5).abs() < 1e-9, "offset diff got {}", v[0].value);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn promql_aggregation_grouping_by_without() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("promql_group");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 64, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let writer = store.writer();

            // 3 séries de `bms_v` :
            //   {bms_id=1}          = 1
            //   {bms_id=2}          = 2
            //   {bms_id=1,phase=a}  = 3
            let ts = 100_000;
            writer
                .write(Sample::new("bms_v", ts, 1.0).with_label("bms_id", "1"))
                .await
                .unwrap();
            writer
                .write(Sample::new("bms_v", ts, 2.0).with_label("bms_id", "2"))
                .await
                .unwrap();
            writer
                .write(
                    Sample::new("bms_v", ts, 3.0)
                        .with_label("bms_id", "1")
                        .with_label("phase", "a"),
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);
            let at = 100_000;

            // Helper : trie les samples par valeur croissante, vérifie l'absence
            // de `__name__` dans les labels de sortie.
            let eval_sorted = |q: &str| {
                let expr = parse_and_validate(q).unwrap();
                let mut v = ev.eval_instant(&expr, at).unwrap();
                for s in &v {
                    assert!(
                        !s.labels.contains_key("__name__"),
                        "{q}: __name__ doit être retiré, labels={:?}",
                        s.labels
                    );
                }
                v.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap());
                v
            };

            // sum by (bms_id) → {bms_id=1}=4, {bms_id=2}=2
            let v = eval_sorted("sum by (bms_id)(bms_v)");
            assert_eq!(v.len(), 2);
            assert!((v[0].value - 2.0).abs() < 1e-9); // bms_id=2
            assert_eq!(v[0].labels.get("bms_id").map(String::as_str), Some("2"));
            assert!((v[1].value - 4.0).abs() < 1e-9); // bms_id=1
            assert_eq!(v[1].labels.get("bms_id").map(String::as_str), Some("1"));
            // by (...) ne garde que bms_id → pas de phase.
            assert!(!v[1].labels.contains_key("phase"));

            // avg by (bms_id) → {bms_id=1}=2, {bms_id=2}=2
            let v = eval_sorted("avg by (bms_id)(bms_v)");
            assert_eq!(v.len(), 2);
            assert!((v[0].value - 2.0).abs() < 1e-9);
            assert!((v[1].value - 2.0).abs() < 1e-9);

            // count by (bms_id) → {bms_id=1}=2, {bms_id=2}=1
            let v = eval_sorted("count by (bms_id)(bms_v)");
            assert_eq!(v.len(), 2);
            assert!((v[0].value - 1.0).abs() < 1e-9);
            assert!((v[1].value - 2.0).abs() < 1e-9);

            // max by (bms_id) → {bms_id=1}=3, {bms_id=2}=2
            let v = eval_sorted("max by (bms_id)(bms_v)");
            assert_eq!(v.len(), 2);
            assert!((v[0].value - 2.0).abs() < 1e-9);
            assert!((v[1].value - 3.0).abs() < 1e-9);

            // sum without (phase) → {bms_id=1}=4, {bms_id=2}=2
            // (les 2 séries bms_id=1 fusionnent une fois `phase` retiré).
            let v = eval_sorted("sum without (phase)(bms_v)");
            assert_eq!(v.len(), 2);
            assert!((v[0].value - 2.0).abs() < 1e-9);
            assert_eq!(v[0].labels.get("bms_id").map(String::as_str), Some("2"));
            assert!((v[1].value - 4.0).abs() < 1e-9);
            assert_eq!(v[1].labels.get("bms_id").map(String::as_str), Some("1"));

            // sum (sans modifier) → 6 (collapse total, labels vides)
            let v = eval_sorted("sum(bms_v)");
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 6.0).abs() < 1e-9);
            assert!(v[0].labels.is_empty());
        }
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn promql_p2_p3_functions() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("promql_p2p3");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 64, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let writer = store.writer();

            // soc : droite décroissante 100,90,…,10 sur 10 pts (pas 1 s) → pente -10/s.
            // status : 0,0,1,1,2 → 2 changements. cnt : 0,5,10,2,8 → 1 reset.
            for i in 0..10_i64 {
                let ts = 100_000 + i * 1_000;
                writer
                    .write(Sample::new("soc", ts, 100.0 - 10.0 * i as f64))
                    .await
                    .unwrap();
            }
            for (i, v) in [0.0, 0.0, 1.0, 1.0, 2.0].iter().enumerate() {
                writer
                    .write(Sample::new("status", 100_000 + i as i64 * 1_000, *v))
                    .await
                    .unwrap();
            }
            for (i, v) in [0.0, 5.0, 10.0, 2.0, 8.0].iter().enumerate() {
                writer
                    .write(Sample::new("cnt", 100_000 + i as i64 * 1_000, *v))
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);
            let at = 109_000;
            let scalar = |q: &str| {
                let expr = parse_and_validate(q).unwrap();
                let v = ev.eval_instant(&expr, at).unwrap();
                assert_eq!(v.len(), 1, "{q}");
                v[0].value
            };

            // deriv ≈ -10/s
            assert!((scalar("deriv(soc[10s])") - (-10.0)).abs() < 1e-6);
            // predict_linear : à t=109s soc=10, pente -10 → +5 s ⇒ 10-50 = -40
            assert!((scalar("predict_linear(soc[10s], 5)") - (-40.0)).abs() < 1e-6);
            // quantile_over_time 0.5 sur [10..100] → médiane 55
            assert!((scalar("quantile_over_time(0.5, soc[10s])") - 55.0).abs() < 1e-9);
            // stddev_over_time > 0
            assert!(scalar("stddev_over_time(soc[10s])") > 0.0);
            // changes(status) = 2
            assert!((scalar("changes(status[10s])") - 2.0).abs() < 1e-9);
            // resets(cnt) = 1
            assert!((scalar("resets(cnt[10s])") - 1.0).abs() < 1e-9);

            // absent : série existante → vecteur vide
            let expr = parse_and_validate("absent(soc)").unwrap();
            assert_eq!(ev.eval_instant(&expr, at).unwrap().len(), 0);
            // absent : série inexistante → 1 sample à 1 avec labels du matcher
            let expr = parse_and_validate(r#"absent(missing_metric{site="A"})"#).unwrap();
            let v = ev.eval_instant(&expr, at).unwrap();
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 1.0).abs() < 1e-9);
            assert_eq!(v[0].labels.get("site").map(String::as_str), Some("A"));

            // absent_over_time : série présente sur la fenêtre → vide
            let expr = parse_and_validate("absent_over_time(soc[10s])").unwrap();
            assert_eq!(ev.eval_instant(&expr, at).unwrap().len(), 0);
            // absent_over_time : série absente → 1 sample à 1 avec labels du matcher
            let expr =
                parse_and_validate(r#"absent_over_time(missing_metric{site="B"}[5m])"#).unwrap();
            let v = ev.eval_instant(&expr, at).unwrap();
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 1.0).abs() < 1e-9);
            assert_eq!(v[0].labels.get("site").map(String::as_str), Some("B"));
            // absent_over_time : fenêtre sans point (trop ancienne) → 1 sample
            let expr = parse_and_validate("absent_over_time(soc[10s])").unwrap();
            assert_eq!(ev.eval_instant(&expr, 10_000_000).unwrap().len(), 1);
            // absent avec argument parenthésé : labels du sélecteur préservés.
            let expr = parse_and_validate(r#"absent((missing_metric{site="C"}))"#).unwrap();
            let v = ev.eval_instant(&expr, at).unwrap();
            assert_eq!(v.len(), 1);
            assert_eq!(v[0].labels.get("site").map(String::as_str), Some("C"));

            // round(v, 0.1) honore to_nearest (105.0 reste 105.0 ; vérifie via soc=100)
            let expr = parse_and_validate("round(soc, 0.1)").unwrap();
            let v = ev.eval_instant(&expr, 100_000).unwrap();
            assert!((v[0].value - 100.0).abs() < 1e-9);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn promql_math_and_label_functions() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("promql_mathlabel");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 64, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let writer = store.writer();

            // bms_v{bms_id="1",phase="a"} = 4.0
            let ts = 100_000;
            writer
                .write(
                    Sample::new("bms_v", ts, 4.0)
                        .with_label("bms_id", "1")
                        .with_label("phase", "a"),
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);
            let at = 100_000;
            let eval = |q: &str| {
                let expr = parse_and_validate(q).unwrap();
                ev.eval_instant(&expr, at).unwrap()
            };

            // sqrt(4) = 2
            let v = eval("sqrt(bms_v)");
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 2.0).abs() < 1e-9);

            // clamp(4, 0, 2) = 2
            let v = eval("clamp(bms_v, 0, 2)");
            assert!((v[0].value - 2.0).abs() < 1e-9);
            // clamp avec min > max → NaN
            let v = eval("clamp(bms_v, 5, 1)");
            assert!(v[0].value.is_nan());

            // sgn(4) = 1
            let v = eval("sgn(bms_v)");
            assert!((v[0].value - 1.0).abs() < 1e-9);

            // label_replace : crée pack="P1" depuis bms_id, valeur inchangée.
            let v = eval(r#"label_replace(bms_v, "pack", "P$1", "bms_id", "(.*)")"#);
            assert_eq!(v.len(), 1);
            assert_eq!(v[0].labels.get("pack").map(String::as_str), Some("P1"));
            assert!((v[0].value - 4.0).abs() < 1e-9);
            // labels d'origine préservés.
            assert_eq!(v[0].labels.get("bms_id").map(String::as_str), Some("1"));

            // label_replace sans match → série inchangée (pas de label ajouté).
            let v = eval(r#"label_replace(bms_v, "pack", "X", "bms_id", "zzz")"#);
            assert!(!v[0].labels.contains_key("pack"));

            // label_join : combo = "1-a"
            let v = eval(r#"label_join(bms_v, "combo", "-", "bms_id", "phase")"#);
            assert_eq!(v[0].labels.get("combo").map(String::as_str), Some("1-a"));
        }
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn promql_irate() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("promql_irate");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 64, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let writer = store.writer();

            // Compteur croissant : 0,10,20,...,90 par pas de 1 s.
            // Les 2 derniers points (80 à 8 s, 90 à 9 s) → irate = 10/s.
            for i in 0..10_i64 {
                let ts = 100_000 + i * 1_000;
                writer
                    .write(Sample::new("energy_wh", ts, i as f64 * 10.0))
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);

            // irate sur la fenêtre complète → pente locale des 2 derniers points.
            let expr = parse_and_validate("irate(energy_wh[10s])").unwrap();
            let v = ev.eval_instant(&expr, 109_000).unwrap();
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 10.0).abs() < 1e-9, "got {}", v[0].value);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn promql_topk_bottomk() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("promql_topk");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 64, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let writer = store.writer();

            // bms_v : {bms_id=1}=1 .. {bms_id=4}=4
            let ts = 100_000;
            for id in 1..=4_i64 {
                writer
                    .write(
                        Sample::new("bms_v", ts, id as f64)
                            .with_label("bms_id", &id.to_string()),
                    )
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);
            let at = 100_000;
            let eval = |q: &str| {
                let expr = parse_and_validate(q).unwrap();
                ev.eval_instant(&expr, at).unwrap()
            };

            // topk(1, bms_v) → 1 sample (le max = 4), labels d'origine conservés.
            let v = eval("topk(1, bms_v)");
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 4.0).abs() < 1e-9);
            assert_eq!(v[0].labels.get("bms_id").map(String::as_str), Some("4"));
            // topk conserve __name__.
            assert_eq!(
                v[0].labels.get("__name__").map(String::as_str),
                Some("bms_v")
            );

            // bottomk(2, bms_v) → 2 samples (1 et 2).
            let mut v = eval("bottomk(2, bms_v)");
            v.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap());
            assert_eq!(v.len(), 2);
            assert!((v[0].value - 1.0).abs() < 1e-9);
            assert!((v[1].value - 2.0).abs() < 1e-9);

            // topk(10, bms_v) → tronque à la taille réelle (4).
            let v = eval("topk(10, bms_v)");
            assert_eq!(v.len(), 4);

            // topk(0, bms_v) → vide.
            let v = eval("topk(0, bms_v)");
            assert_eq!(v.len(), 0);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn promql_comparison_operators() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("promql_cmp");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 64, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let writer = store.writer();

            // bms_v{bms_id=1}=1, {bms_id=2}=2, {bms_id=3}=3
            let ts = 100_000;
            for id in 1..=3_i64 {
                writer
                    .write(
                        Sample::new("bms_v", ts, id as f64)
                            .with_label("bms_id", &id.to_string()),
                    )
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);
            let at = 100_000;
            let eval = |q: &str| {
                let expr = parse_and_validate(q).unwrap();
                ev.eval_instant(&expr, at).unwrap()
            };

            // Filtre : bms_v > 1.5 → 2 samples (2 et 3), valeurs inchangées.
            let mut v = eval("bms_v > 1.5");
            v.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap());
            assert_eq!(v.len(), 2);
            assert!((v[0].value - 2.0).abs() < 1e-9);
            assert!((v[1].value - 3.0).abs() < 1e-9);
            // __name__ retiré.
            assert!(v.iter().all(|s| !s.labels.contains_key("__name__")));

            // bool : bms_v > bool 1.5 → 3 samples, valeurs 0/1.
            let v = eval("bms_v > bool 1.5");
            assert_eq!(v.len(), 3);
            let ones = v.iter().filter(|s| (s.value - 1.0).abs() < 1e-9).count();
            let zeros = v.iter().filter(|s| s.value.abs() < 1e-9).count();
            assert_eq!(ones, 2);
            assert_eq!(zeros, 1);

            // Égalité : bms_v == 2 → 1 sample.
            let v = eval("bms_v == 2");
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 2.0).abs() < 1e-9);

            // vec/vec aligné : bms_v >= bms_v → tous conservés (3), valeur lhs.
            let v = eval("bms_v >= bms_v");
            assert_eq!(v.len(), 3);

            // vec/vec bool : bms_v < bool bms_v → 3 samples tous à 0.
            let v = eval("bms_v < bool bms_v");
            assert_eq!(v.len(), 3);
            assert!(v.iter().all(|s| s.value.abs() < 1e-9));
        }
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn writer_persists_next_series_id_across_reopen() {
        let path = tmp_db_path("reopen");
        let _ = std::fs::remove_file(&path);

        let opts = Options {
            writer: WriterConfig { batch_max: 1, flush_ms: 20, poll_idle_ms: 2 },
            ..Options::default()
        };

        // Phase 1 : crée 2 séries
        {
            let store = MetricsStore::open(&path, opts.clone()).unwrap();
            let w = store.writer();
            w.write(Sample::new("a", 1, 1.0)).await.unwrap();
            w.write(Sample::new("b", 1, 2.0)).await.unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert_eq!(store.reader().list_series().unwrap().len(), 2);
        }

        // Phase 2 : rouvre, ajoute "c" → doit recevoir series_id=3, pas =1
        {
            let store = MetricsStore::open(&path, opts).unwrap();
            let w = store.writer();
            w.write(Sample::new("c", 1, 3.0)).await.unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
            let reader = store.reader();
            assert_eq!(reader.list_series().unwrap().len(), 3);
            let id_c = reader.lookup_series_id("c", "{}").unwrap().unwrap();
            assert!(id_c >= 3, "id_c={id_c} doit être ≥ 3 (pas de réutilisation)");
        }

        let _ = std::fs::remove_file(&path);
    }

    /// R2 : dropper le store alors qu'un `Writer` clone survit ne doit PAS
    /// bloquer indéfiniment (sentinelle `Shutdown` in-band).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drop_store_with_live_writer_clone_does_not_hang() {
        let path = tmp_db_path("r2");
        let _ = std::fs::remove_file(&path);

        let opts = Options {
            writer: WriterConfig { batch_max: 4, flush_ms: 20, poll_idle_ms: 2 },
            ..Options::default()
        };
        let store = MetricsStore::open(&path, opts).expect("open");
        let w = store.writer();
        w.write(Sample::new("x", 1, 1.0)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Drop dans un thread bloquant, borné par timeout : en cas de
        // régression R2 le `join()` resterait bloqué > 5 s.
        let res = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::task::spawn_blocking(move || drop(store)),
        )
        .await;
        assert!(res.is_ok(), "drop(store) a bloqué malgré un Writer clone vivant — régression R2");

        // Le clone survit et reste utilisable sans paniquer (les writes vont
        // au néant après arrêt du writer — best-effort).
        let _ = w.try_write(Sample::new("x", 2, 2.0));
        drop(w);
        let _ = std::fs::remove_file(&path);
    }

    /// R4 : une seconde passe de compaction sur une heure déjà compactée doit
    /// FUSIONNER avec le bucket existant, pas l'écraser.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn compaction_merges_existing_bucket_not_overwrite() {
        use crate::tiering::{compact_raw_to_hourly, HOURLY_MS};

        let path = tmp_db_path("r4");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 1, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let w = store.writer();

            // 3 points dans l'heure 0.
            for ts in [0_i64, 1000, 2000] {
                w.write(Sample::new("p", ts, 10.0)).await.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(150)).await;

            let s1 = compact_raw_to_hourly(&store.inner.db, 3000).unwrap();
            assert_eq!(s1.buckets_written, 1);
            assert_eq!(s1.points_purged, 3);

            // 2 points supplémentaires DANS LA MÊME heure 0, après compaction.
            w.write(Sample::new("p", 2500, 20.0)).await.unwrap();
            w.write(Sample::new("p", 2800, 20.0)).await.unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;

            let s2 = compact_raw_to_hourly(&store.inner.db, 3000).unwrap();
            assert_eq!(s2.buckets_written, 1);
            assert_eq!(s2.points_purged, 2);

            let reader = store.reader();
            let sid = reader.lookup_series_id("p", "{}").unwrap().unwrap();
            let hourly = reader
                .query_range_buckets(sid, 0, HOURLY_MS, Tier::Hourly)
                .unwrap();
            assert_eq!(hourly.len(), 1);
            let b = hourly[0].1;
            // Fusion : 3 + 2 = 5 points, somme 30 + 40 = 70 (pas écrasé à 2/40).
            assert_eq!(b.cnt, 5, "bucket écrasé au lieu d'être fusionné (R4)");
            assert!((b.sum - 70.0).abs() < 1e-9, "sum={}", b.sum);
            assert_eq!(b.min, 10.0);
            assert_eq!(b.max, 20.0);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// M1 : réutiliser un `Evaluator` pour deux requêtes instant différentes
    /// ne doit pas renvoyer un résultat périmé (cache keyé par pointeur vidé
    /// au début de chaque évaluation).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn evaluator_reuse_distinct_instant_queries() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("m1");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 64, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let w = store.writer();
            for i in 0..10_i64 {
                let ts = 100_000 + i * 1_000;
                w.write(Sample::new("bms_v", ts, 1.0 + i as f64 / 10.0).with_label("bms_id", "1"))
                    .await
                    .unwrap();
                w.write(Sample::new("bms_v", ts, 2.0 + i as f64 / 10.0).with_label("bms_id", "2"))
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);

            // 1ʳᵉ requête : tous les bms_v → 2 séries.
            let e1 = parse_and_validate("bms_v").unwrap();
            let v1 = ev.eval_instant(&e1, 109_000).unwrap();
            assert_eq!(v1.len(), 2);

            // 2ᵉ requête sur le MÊME Evaluator, sélecteur plus restrictif → 1 série.
            let e2 = parse_and_validate(r#"bms_v{bms_id="1"}"#).unwrap();
            let v2 = ev.eval_instant(&e2, 109_000).unwrap();
            assert_eq!(v2.len(), 1, "résultat périmé du cache pointeur (régression M1)");
            assert!((v2[0].value - 1.9).abs() < 1e-9);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// Revue Gemini : `increase`/`rate` doivent gérer un reset de compteur
    /// (`last < first`) au lieu de renvoyer une valeur négative.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn increase_rate_handle_counter_reset() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("reset");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 8, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let w = store.writer();
            // Compteur qui se réinitialise : 80 puis 5 (reset matériel).
            w.write(Sample::new("energy_reset", 100_000, 80.0)).await.unwrap();
            w.write(Sample::new("energy_reset", 101_000, 5.0)).await.unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);

            // increase : reset → on retient `last` (5), pas 5-80 = -75.
            let e = parse_and_validate("increase(energy_reset[10s])").unwrap();
            let v = ev.eval_instant(&e, 101_000).unwrap();
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 5.0).abs() < 1e-9, "increase reset = {}", v[0].value);
            assert!(v[0].value >= 0.0, "increase ne doit jamais être négatif");

            // rate : 5 / 10 s = 0.5 (jamais négatif).
            let e = parse_and_validate("rate(energy_reset[10s])").unwrap();
            let v = ev.eval_instant(&e, 101_000).unwrap();
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 0.5).abs() < 1e-9, "rate reset = {}", v[0].value);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// Revue Gemini : un intervalle inversé (`from > to`) ne doit pas paniquer
    /// dans `redb::range` — résultat vide / `None`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn inverted_interval_does_not_panic() {
        let path = tmp_db_path("inverted");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 8, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let w = store.writer();
            for ts in 1000..1005 {
                w.write(Sample::new("g", ts, ts as f64)).await.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let sid = reader.lookup_series_id("g", "{}").unwrap().unwrap();

            // Bornes inversées → vide, pas de panic.
            assert!(reader.query_range_raw(sid, 2000, 1000).unwrap().is_empty());

            let rtx = reader.begin_read().unwrap();
            assert!(reader
                .last_point_in_range_with_tx(&rtx, sid, 2000, 1000)
                .unwrap()
                .is_none());
            assert!(reader
                .first_last_in_range_with_tx(&rtx, sid, 2000, 1000)
                .unwrap()
                .is_none());
            assert!(reader
                .query_range_buckets_with_tx(&rtx, sid, 2000, 1000, Tier::Hourly)
                .unwrap()
                .is_empty());

            // Sanity : intervalle normal renvoie bien les points.
            assert_eq!(reader.query_range_raw(sid, 1000, 1004).unwrap().len(), 5);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// Revue Gemini PR #521 : `increase` sur tier raw doit gérer un reset
    /// *intermédiaire* (au milieu de la fenêtre), pas seulement aux bornes.
    /// Valide de bout en bout le re-routage (chargement de tous les points).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn increase_handles_intermediate_reset_e2e() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("midreset");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 8, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let w = store.writer();
            // Compteur avec reset au milieu : 10, 20, 5, 15.
            for (i, v) in [10.0, 20.0, 5.0, 15.0].into_iter().enumerate() {
                w.write(Sample::new("cnt", 100_000 + i as i64 * 1_000, v)).await.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);
            let e = parse_and_validate("increase(cnt[10s])").unwrap();
            let v = ev.eval_instant(&e, 103_000).unwrap();
            assert_eq!(v.len(), 1);
            // (20-10) + reset(5) + (15-5) = 25, pas l'ancien (15-10)=5.
            assert!((v[0].value - 25.0).abs() < 1e-9, "increase mid-reset = {}", v[0].value);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// Évolution conformité PromQL : opérateurs ensemblistes (`and`/`or`/`unless`)
    /// et matching vectoriel (`on`/`group_left`) — alertes multi-critères ESS.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn promql_set_ops_and_vector_matching() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("promql_setops");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 64, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let w = store.writer();
            let ts = 100_000;
            // SOC / température par BMS.
            w.write(Sample::new("bms_soc", ts, 10.0).with_label("bms_id", "1")).await.unwrap();
            w.write(Sample::new("bms_soc", ts, 50.0).with_label("bms_id", "2")).await.unwrap();
            w.write(Sample::new("bms_temp_c", ts, 50.0).with_label("bms_id", "1")).await.unwrap();
            w.write(Sample::new("bms_temp_c", ts, 30.0).with_label("bms_id", "2")).await.unwrap();
            // Rendement string PV (on()).
            w.write(
                Sample::new("mppt_power_w", ts, 380.0)
                    .with_label("string_id", "A")
                    .with_label("mppt_id", "1"),
            )
            .await
            .unwrap();
            w.write(
                Sample::new("pv_theo_w", ts, 400.0)
                    .with_label("string_id", "A")
                    .with_label("model", "400W"),
            )
            .await
            .unwrap();
            // Puissance par phase + capacité (group_left many-to-one).
            w.write(
                Sample::new("phase_power", ts, 100.0)
                    .with_label("bms_id", "1")
                    .with_label("phase", "L1"),
            )
            .await
            .unwrap();
            w.write(
                Sample::new("phase_power", ts, 200.0)
                    .with_label("bms_id", "1")
                    .with_label("phase", "L2"),
            )
            .await
            .unwrap();
            w.write(
                Sample::new("capacity", ts, 360.0)
                    .with_label("bms_id", "1")
                    .with_label("cls", "big"),
            )
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);
            let at = |e: &str| {
                let expr = parse_and_validate(e).unwrap();
                ev.eval_instant(&expr, ts).unwrap()
            };

            // AND multi-critères : SOC<20 ET T>45 → seul bms_id=1.
            let v = at("bms_soc < 20 and bms_temp_c > 45");
            assert_eq!(v.len(), 1, "and");
            assert_eq!(v[0].labels.get("bms_id").map(String::as_str), Some("1"));
            assert!((v[0].value - 10.0).abs() < 1e-9);

            // OR : SOC>20 (bms 2) ∪ T>45 (bms 1) → 2 séries.
            let v = at("bms_soc > 20 or bms_temp_c > 45");
            assert_eq!(v.len(), 2, "or");

            // UNLESS : bms_soc sauf ceux >20 → seul bms_id=1.
            let v = at("bms_soc unless bms_soc > 20");
            assert_eq!(v.len(), 1, "unless");
            assert_eq!(v[0].labels.get("bms_id").map(String::as_str), Some("1"));

            // on(string_id) : rendement 380/400 = 0.95, labels réduits à string_id.
            let v = at("mppt_power_w / on(string_id) pv_theo_w");
            assert_eq!(v.len(), 1, "on()");
            assert!((v[0].value - 0.95).abs() < 1e-9, "ratio = {}", v[0].value);
            assert_eq!(v[0].labels.get("string_id").map(String::as_str), Some("A"));
            assert!(v[0].labels.get("mppt_id").is_none(), "on() ne garde que string_id");

            // group_left(cls) : many(phase)→one(capacity), copie le label cls.
            let mut v = at("phase_power * on(bms_id) group_left(cls) capacity");
            v.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap());
            assert_eq!(v.len(), 2, "group_left");
            assert!((v[0].value - 36000.0).abs() < 1e-6); // L1: 100*360
            assert!((v[1].value - 72000.0).abs() < 1e-6); // L2: 200*360
            assert_eq!(v[0].labels.get("cls").map(String::as_str), Some("big"));
            assert_eq!(v[0].labels.get("phase").map(String::as_str), Some("L1"));

            // Sans group_left, l'appariement OneToOne avec doublons côté gauche
            // (2 phases, même bms_id) doit échouer (sémantique Prometheus).
            let expr = parse_and_validate("phase_power * on(bms_id) capacity").unwrap();
            let err = ev.eval_instant(&expr, ts).unwrap_err();
            assert!(
                err.to_string().contains("côté gauche"),
                "attendu erreur many-to-one, got: {err}"
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    /// Évolution conformité PromQL : agrégateurs `quantile`, `group`,
    /// `count_values`, `stddev` (SLO santé batterie / état de flotte).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn promql_new_aggregators() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("promql_aggs");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 64, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let w = store.writer();
            let ts = 100_000;
            for (n, val) in [(1, 1.0), (2, 2.0), (3, 3.0), (4, 4.0), (5, 5.0)] {
                w.write(Sample::new("q", ts, val).with_label("n", &n.to_string())).await.unwrap();
            }
            // count_values : valeurs [1,1,2].
            w.write(Sample::new("cv", ts, 1.0).with_label("x", "a")).await.unwrap();
            w.write(Sample::new("cv", ts, 1.0).with_label("x", "b")).await.unwrap();
            w.write(Sample::new("cv", ts, 2.0).with_label("x", "c")).await.unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);
            let at = |e: &str| {
                let expr = parse_and_validate(e).unwrap();
                ev.eval_instant(&expr, ts).unwrap()
            };

            // quantile(0.5) spatial = médiane = 3.
            let v = at("quantile(0.5, q)");
            assert_eq!(v.len(), 1, "quantile");
            assert!((v[0].value - 3.0).abs() < 1e-9, "median = {}", v[0].value);

            // group → présence binaire (1), un seul groupe.
            let v = at("group(q)");
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 1.0).abs() < 1e-9);

            // stddev population de [1..5] = sqrt(2).
            let v = at("stddev(q)");
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 2.0_f64.sqrt()).abs() < 1e-9, "stddev = {}", v[0].value);

            // count_values("v", cv) → {v=1}=2, {v=2}=1.
            let mut v = at(r#"count_values("v", cv)"#);
            v.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap());
            assert_eq!(v.len(), 2, "count_values");
            assert!((v[0].value - 1.0).abs() < 1e-9); // valeur "2" apparaît 1×
            assert_eq!(v[0].labels.get("v").map(String::as_str), Some("2"));
            assert!((v[1].value - 2.0).abs() < 1e-9); // valeur "1" apparaît 2×
            assert_eq!(v[1].labels.get("v").map(String::as_str), Some("1"));
        }
        let _ = std::fs::remove_file(&path);
    }

    /// Évolution conformité PromQL : modificateur `@` (point-in-time fixe) et
    /// sous-requêtes `[range:step]` (lissage de rate, moyennes mobiles).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn promql_at_modifier_and_subquery() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("promql_at_sub");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 64, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let w = store.writer();
            // bms_v{bms_id=1} : 1.0..1.9 aux ts 100_000..109_000 (ms).
            for i in 0..10_i64 {
                let ts = 100_000 + i * 1_000;
                w.write(Sample::new("bms_v", ts, 1.0 + i as f64 / 10.0).with_label("bms_id", "1"))
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);

            // `@ 105` (secondes → 105_000 ms) fixe l'instant, même évalué à t=200_000.
            let e = parse_and_validate(r#"bms_v{bms_id="1"} @ 105"#).unwrap();
            let v = ev.eval_instant(&e, 200_000).unwrap();
            assert_eq!(v.len(), 1, "@ modifier");
            assert!((v[0].value - 1.5).abs() < 1e-9, "@105 → {}", v[0].value);

            // Sous-requête : max_over_time(bms_v[10s:2s]) à t=109_000 → max = 1.9.
            let e = parse_and_validate(r#"max_over_time(bms_v{bms_id="1"}[10s:2s])"#).unwrap();
            let v = ev.eval_instant(&e, 109_000).unwrap();
            assert_eq!(v.len(), 1, "subquery");
            assert!((v[0].value - 1.9).abs() < 1e-9, "max subq = {}", v[0].value);

            // avg_over_time sous-requête : moyenne des sous-échantillons.
            let e = parse_and_validate(r#"avg_over_time(bms_v{bms_id="1"}[8s:2s])"#).unwrap();
            let v = ev.eval_instant(&e, 109_000).unwrap();
            assert_eq!(v.len(), 1);
            // sous-pas à 101,103,105,107,109k → valeurs 1.1,1.3,1.5,1.7,1.9 → moy 1.5.
            assert!((v[0].value - 1.5).abs() < 1e-9, "avg subq = {}", v[0].value);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// P0 (§4.6) : `rate`/`increase` avec un seul point sous la fenêtre doivent
    /// retourner *no data* (et non `0`), pour ne pas masquer les séries
    /// clairsemées dans les bilans énergétiques.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn promql_rate_single_point_no_data() {
        use crate::promql::{parse_and_validate, Evaluator};

        let path = tmp_db_path("promql_1pt");
        let _ = std::fs::remove_file(&path);
        {
            let opts = Options {
                writer: WriterConfig { batch_max: 8, flush_ms: 20, poll_idle_ms: 2 },
                ..Options::default()
            };
            let store = MetricsStore::open(&path, opts).expect("open");
            let w = store.writer();
            // Compteur clairsemé : 2 points espacés de 10 s.
            w.write(Sample::new("sparse", 100_000, 0.0)).await.unwrap();
            w.write(Sample::new("sparse", 110_000, 100.0)).await.unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;

            let reader = store.reader();
            let ev = Evaluator::new(&reader);

            // Fenêtre de 3 s à t=100_000 : un seul point → no data (0 série).
            let e = parse_and_validate("increase(sparse[3s])").unwrap();
            assert_eq!(ev.eval_instant(&e, 100_000).unwrap().len(), 0, "increase 1pt");
            let e = parse_and_validate("rate(sparse[3s])").unwrap();
            assert_eq!(ev.eval_instant(&e, 100_000).unwrap().len(), 0, "rate 1pt");

            // Fenêtre de 15 s à t=110_000 : 2 points → increase = 100 (non régressé).
            let e = parse_and_validate("increase(sparse[15s])").unwrap();
            let v = ev.eval_instant(&e, 110_000).unwrap();
            assert_eq!(v.len(), 1);
            assert!((v[0].value - 100.0).abs() < 1e-9, "increase 2pt = {}", v[0].value);
        }
        let _ = std::fs::remove_file(&path);
    }
}
