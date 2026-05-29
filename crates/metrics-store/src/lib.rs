//! `metrics-store` — backend TSDB pure-Rust basé sur [`redb`].
//!
//! Cette crate remplacera VictoriaMetrics pour stocker les métriques produites
//! par `daly-bms-server` et `energy-manager`. Cf. `docs/plan_migration_vm_redb.md`
//! pour la conception complète :
//!
//! - §4 schéma de tables et encodage des clés
//! - §5 API publique (`MetricsStore`, `Writer`, `Reader`)
//! - §6 transpileur PromQL (à venir)
//! - §9 tiering raw→hourly→daily (à venir)
//! - §10 plan de bascule en 4 phases
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
}
