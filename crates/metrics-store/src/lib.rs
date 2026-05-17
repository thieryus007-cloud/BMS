//! `metrics-store` — backend TSDB pure-Rust basé sur [`redb`].
//!
//! Cette crate remplacera VictoriaMetrics pour stocker les métriques produites
//! par `daly-bms-server` et `energy-manager`. Cf. `docs/plan_migration_vm_redb.md`
//! pour la conception complète :
//!
//! - §4 schéma de tables et encodage des clés
//! - §5 API publique (`MetricsStore`, `Writer`, `Reader`)
//! - §10 plan de bascule en 4 phases
//!
//! État : **squelette** (Phase 0.3). Le writer batché (§5.3), le reader
//! (§5.4) et les agrégats (§5.5) seront ajoutés dans les tickets suivants.

pub mod encoding;
pub mod reader;
pub mod tables;
pub mod writer;

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use redb::{Builder, Database};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub use reader::Reader;
pub use tables::{AggBucket, SeriesMeta, Tier};
pub use writer::{Sample, Writer};

/// Options d'ouverture. La sémantique exacte des champs sera affinée quand le
/// writer batché et la maintenance seront branchés.
#[derive(Debug, Clone)]
pub struct Options {
    /// Taille du cache de pages redb (défaut 64 MiB, cf. §4.4).
    pub cache_bytes: usize,
    /// Profondeur de la queue mpsc entre producteurs et writer_loop.
    pub writer_queue_depth: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cache_bytes: 64 * 1024 * 1024,
            writer_queue_depth: 10_000,
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

#[derive(Clone)]
pub struct MetricsStore {
    db: Arc<Database>,
    writer_tx: mpsc::Sender<Sample>,
}

impl MetricsStore {
    /// Ouvre (ou crée) la base et initialise toutes les tables.
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

        // Queue pour le writer_loop (encore à implémenter, cf. §5.3).
        let (writer_tx, _writer_rx) = mpsc::channel::<Sample>(opts.writer_queue_depth);

        Ok(Self { db, writer_tx })
    }

    pub fn writer(&self) -> Writer {
        Writer { tx: self.writer_tx.clone() }
    }

    pub fn reader(&self) -> Reader {
        Reader { db: self.db.clone() }
    }

    /// Démarre la tâche de maintenance (compaction raw→hourly→daily).
    /// Stub — l'implémentation arrivera avec le module `tiering.rs` (Phase 0.5).
    pub fn spawn_maintenance(&self, _policy: TierPolicy) -> JoinHandle<()> {
        tokio::spawn(async move {
            tracing::warn!("metrics-store: spawn_maintenance pas encore implémenté");
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let _ = std::fs::remove_file(&path);
    }
}
