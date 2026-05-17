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
pub mod reader;
pub mod tables;
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
    writer_tx: parking_lot::Mutex<Option<mpsc::Sender<Sample>>>,
    writer_handle: parking_lot::Mutex<Option<thread::JoinHandle<()>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // 1. Ferme le canal en libérant le `Sender` (les `Writer` clones
        //    encore vivants côté caller maintiennent le canal ouvert ; on
        //    documente que le store doit être dropé en dernier).
        self.writer_tx.lock().take();
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

        let (writer_tx, writer_rx) = mpsc::channel::<Sample>(opts.writer_queue_depth);
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
    /// Stub — l'implémentation arrivera avec le module `tiering.rs` (ticket 0.5b).
    pub fn spawn_maintenance(&self, _policy: TierPolicy) -> JoinHandle<()> {
        tokio::spawn(async move {
            tracing::warn!("metrics-store: spawn_maintenance pas encore implémenté");
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
}
