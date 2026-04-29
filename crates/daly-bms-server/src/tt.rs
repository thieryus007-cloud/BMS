//! Module d'intégration de Tsink (base de données temps-réel embarquée)
//! Remplace InfluxDB.

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use tsink::{
    AsyncStorage, AsyncStorageBuilder, Row, Label, DataPoint,
    TimestampPrecision, Error as TsinkError,
};

/// Configuration pour Tsink
#[derive(Debug, Clone)]
pub struct TsinkConfig {
    pub data_path: PathBuf,
    pub retention_days: u32,
    pub memory_limit_mb: usize,
    pub worker_threads: usize,
    pub batch_size: usize,
    pub flush_interval_secs: u64,
    pub compaction_interval_secs: u64,
}

impl Default for TsinkConfig {
    fn default() -> Self {
        Self {
            data_path: PathBuf::from("./tsink_data"),
            retention_days: 30,
            memory_limit_mb: 512,
            worker_threads: 2,
            batch_size: 500,
            flush_interval_secs: 10,
            compaction_interval_secs: 300,
        }
    }
}

/// Gestionnaire thread-safe de Tsink
#[derive(Clone)]
pub struct TsinkHandle {
    storage: Arc<AsyncStorage>,
}

impl TsinkHandle {
    /// Crée une nouvelle instance de Tsink avec la configuration donnée.
    pub async fn new(config: TsinkConfig) -> Result<Self, TsinkError> {
        info!(
            "Initialisation Tsink: path={:?}, retention={} days, workers={}",
            config.data_path, config.retention_days, config.worker_threads
        );

        let storage = AsyncStorageBuilder::new()
            .with_data_path(config.data_path)
            .with_timestamp_precision(TimestampPrecision::Milliseconds)
            .with_retention(Duration::from_secs(config.retention_days as u64 * 24 * 3600))
            .with_memory_limit(config.memory_limit_mb * 1024 * 1024)
            .with_worker_threads(config.worker_threads)
            .with_batch_size(config.batch_size)
            .with_flush_interval(Duration::from_secs(config.flush_interval_secs))
            .with_compaction_interval(Duration::from_secs(config.compaction_interval_secs))
            .build()
            .await?;

        Ok(Self {
            storage: Arc::new(storage),
        })
    }

    /// Insère plusieurs lignes en une seule opération batch.
    pub async fn insert_rows(&self, rows: Vec<Row>) -> Result<(), TsinkError> {
        if rows.is_empty() {
            return Ok(());
        }
        self.storage.insert_rows(rows).await?;
        Ok(())
    }

    /// Exécute une requête PromQL (à implémenter selon vos besoins)
    pub async fn query(&self, _query: &str, _start: i64, _end: i64) -> Result<Vec<DataPoint>, TsinkError> {
        // Placeholder
        Ok(vec![])
    }
}
