//! Writer batché — plan §5.3.
//!
//! Le writer s'exécute sur un **thread dédié** (pas une task tokio) pour
//! deux raisons :
//! - les appels redb (`begin_write` / `commit`) sont bloquants par nature
//!   (fsync), il serait incorrect de monopoliser un worker tokio ;
//! - `metrics-store` ne doit pas imposer de runtime particulier à ses
//!   consommateurs (le binaire `metrics-cli` ne tourne pas sous tokio).
//!
//! La communication producteur → writer passe par `tokio::sync::mpsc` qui
//! reste utilisable depuis du code sync via `blocking_recv()` / `try_recv()`.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use lru::LruCache;
use redb::{Database, ReadableDatabase, ReadableTable};
use smallvec::SmallVec;
use tokio::sync::mpsc;

use crate::encoding::{enc_skey, make_lookup_key};
use crate::labels::{canonical_json, LabelVec};
use crate::tables::{
    SeriesMeta, TABLE_META, TABLE_RAW, TABLE_SERIES_BY_KEY, TABLE_SERIES_META,
};

const META_KEY_NEXT_SERIES_ID: &str = "next_series_id";

/// Taille max du cache LRU.
///
/// 50k est volontairement large pour éviter un churn excessif tout en
/// empêchant une croissance mémoire infinie.
const SERIES_CACHE_CAPACITY: usize = 50_000;

#[derive(Debug, Clone)]
pub struct Sample {
    pub metric: String,
    pub labels: LabelVec,
    pub ts_ms: i64,
    pub value: f64,
}

impl Sample {
    pub fn new(metric: impl Into<String>, ts_ms: i64, value: f64) -> Self {
        Self {
            metric: metric.into(),
            labels: SmallVec::new(),
            ts_ms,
            value,
        }
    }

    pub fn with_label(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.labels.push((k.into(), v.into()));
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WriterConfig {
    pub batch_max: usize,
    pub flush_ms: u64,
    /// Pause entre deux `try_recv` quand la queue est vide pendant la fenêtre
    /// de batching (5 ms ⇒ 50 itérations max sur 250 ms).
    pub poll_idle_ms: u64,
}

impl Default for WriterConfig {
    fn default() -> Self {
        // Cf. §5.3 : 4 fsync/s en régime nominal.
        Self {
            batch_max: 500,
            flush_ms: 250,
            poll_idle_ms: 5,
        }
    }
}

/// Handle clonable côté producteur.
#[derive(Clone)]
pub struct Writer {
    pub(crate) tx: mpsc::Sender<Sample>,
}

impl Writer {
    pub async fn write(&self, sample: Sample) -> Result<(), mpsc::error::SendError<Sample>> {
        self.tx.send(sample).await
    }

    pub fn try_write(&self, sample: Sample) -> Result<(), mpsc::error::TrySendError<Sample>> {
        self.tx.try_send(sample)
    }

    pub fn blocking_write(&self, sample: Sample) -> Result<(), mpsc::error::SendError<Sample>> {
        self.tx.blocking_send(sample)
    }
}

/// Démarre le thread writer. Retourne son `JoinHandle` (le caller peut
/// l'attendre lors d'un shutdown propre ; en pratique le service tourne
/// jusqu'à `SIGTERM` systemd).
pub fn spawn(
    db: Arc<Database>,
    rx: mpsc::Receiver<Sample>,
    cfg: WriterConfig,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("metrics-store-writer".into())
        .spawn(move || {
            if let Err(e) = run(db, rx, cfg) {
                tracing::error!(error = %e, "metrics-store: writer arrêté sur erreur");
            } else {
                tracing::info!("metrics-store: writer terminé (channel fermé)");
            }
        })
        .expect("spawn writer thread")
}

fn run(db: Arc<Database>, mut rx: mpsc::Receiver<Sample>, cfg: WriterConfig) -> Result<()> {
    let mut series_cache: LruCache<(String, String), u32> = LruCache::new(
        NonZeroUsize::new(SERIES_CACHE_CAPACITY).unwrap(),
    );

    let mut next_id = load_next_id(&db).context("load_next_id")?;

    loop {
        let batch = drain(&mut rx, &cfg);

        if batch.is_empty() {
            return Ok(()); // channel fermé
        }

        if let Err(e) = commit_batch(&db, &batch, &mut series_cache, &mut next_id) {
            tracing::error!(error = %e, samples = batch.len(), "commit_batch a échoué");
        }
    }
}

fn drain(rx: &mut mpsc::Receiver<Sample>, cfg: &WriterConfig) -> Vec<Sample> {
    let mut batch = Vec::with_capacity(cfg.batch_max);

    // Premier sample : on bloque jusqu'à réception (ou shutdown).
    match rx.blocking_recv() {
        Some(s) => batch.push(s),
        None => return batch,
    }

    let deadline = Instant::now() + Duration::from_millis(cfg.flush_ms);
    let idle = Duration::from_millis(cfg.poll_idle_ms);

    while batch.len() < cfg.batch_max && Instant::now() < deadline {
        match rx.try_recv() {
            Ok(s) => batch.push(s),
            Err(mpsc::error::TryRecvError::Empty) => thread::sleep(idle),
            Err(mpsc::error::TryRecvError::Disconnected) => break,
        }
    }

    batch
}

fn load_next_id(db: &Database) -> Result<u32> {
    let rtx = db.begin_read()?;
    let t = rtx.open_table(TABLE_META)?;

    Ok(t.get(META_KEY_NEXT_SERIES_ID)?
        .map(|g| g.value() as u32)
        .unwrap_or(1))
}

fn commit_batch(
    db: &Database,
    batch: &[Sample],
    cache: &mut LruCache<(String, String), u32>,
    next_id: &mut u32,
) -> Result<()> {
    let wtx = db.begin_write()?;
    let mut next_id_dirty = false;

    {
        let mut t_raw = wtx.open_table(TABLE_RAW)?;
        let mut t_skey = wtx.open_table(TABLE_SERIES_BY_KEY)?;
        let mut t_smeta = wtx.open_table(TABLE_SERIES_META)?;

        for s in batch {
            let labels_json = canonical_json(&s.labels);
            let cache_key = (s.metric.clone(), labels_json.clone());

            let series_id = if let Some(&id) = cache.get(&cache_key) {
                id
            } else {
                let lookup_key = make_lookup_key(&s.metric, &labels_json);

                // Lookup avant insert : on extrait la valeur du guard
                // immédiatement pour libérer l'emprunt immutable sur `t_skey`
                // avant d'appeler `.insert()` (sinon borrow checker fail).
                let existing = t_skey.get(&lookup_key[..])?.map(|g| g.value());

                let id = if let Some(id) = existing {
                    id
                } else {
                    let id = *next_id;

                    *next_id = next_id.checked_add(1)
                        .context("series_id overflow u32")?;

                    next_id_dirty = true;

                    t_skey.insert(&lookup_key[..], id)?;

                    let meta = SeriesMeta {
                        metric: s.metric.clone(),
                        labels_json: labels_json.clone(),
                        first_seen_ms: s.ts_ms,
                        last_seen_ms: s.ts_ms,
                    };

                    let bytes = bincode::serialize(&meta)?;
                    t_smeta.insert(id, &bytes[..])?;

                    id
                };

                cache.put(cache_key, id);

                id
            };

            let k = enc_skey(series_id, s.ts_ms);
            t_raw.insert(&k[..], s.value)?;
        }

        if next_id_dirty {
            let mut t_meta = wtx.open_table(TABLE_META)?;
            t_meta.insert(META_KEY_NEXT_SERIES_ID, *next_id as u64)?;
        }
    }

    wtx.commit()?;

    Ok(())
}
