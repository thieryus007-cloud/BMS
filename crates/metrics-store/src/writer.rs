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

/// Cache d'identité de série → `series_id`, keyé par fingerprint `u64`.
/// La valeur conserve `(metric, labels, series_id)` pour la vérification
/// anti-collision (cf. `commit_batch`).
type SeriesCache = LruCache<u64, (String, LabelVec, u32)>;

/// Fingerprint d'une série calculé sans allocation à partir de références.
/// Sensible à l'ordre des labels (deux ordres ⇒ deux entrées de cache qui
/// résolvent vers le même `series_id` via la clé canonique `series_by_key`).
fn series_fingerprint(metric: &str, labels: &LabelVec) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    metric.hash(&mut h);
    labels.hash(&mut h);
    h.finish()
}

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

/// Message interne du canal writer.
///
/// `Shutdown` est une sentinelle in-band qui permet d'arrêter proprement le
/// thread writer **même si des `Writer` clones survivent** côté appelant
/// (sinon le canal mpsc resterait ouvert et `blocking_recv` ne retournerait
/// jamais `None` → le `join()` dans `Inner::drop` bloquerait pour toujours).
/// Cf. correctif R2.
///
/// `Sample` n'est volontairement pas boxé : ce serait une allocation par
/// écriture sur le chemin chaud d'ingestion. La variante `Shutdown` (rare)
/// justifie l'écart de taille.
#[allow(clippy::large_enum_variant)]
pub(crate) enum WriterMsg {
    Sample(Sample),
    Shutdown,
}

/// Reconstruit l'erreur publique `SendError<Sample>` à partir de l'erreur
/// interne, en préservant la signature historique de l'API.
fn map_send_err(e: mpsc::error::SendError<WriterMsg>) -> mpsc::error::SendError<Sample> {
    match e.0 {
        WriterMsg::Sample(s) => mpsc::error::SendError(s),
        // Inatteignable : `Writer` n'envoie jamais `Shutdown`.
        WriterMsg::Shutdown => unreachable!("Writer n'émet jamais Shutdown"),
    }
}

fn map_try_send_err(
    e: mpsc::error::TrySendError<WriterMsg>,
) -> mpsc::error::TrySendError<Sample> {
    use mpsc::error::TrySendError;
    match e {
        TrySendError::Full(WriterMsg::Sample(s)) => TrySendError::Full(s),
        TrySendError::Closed(WriterMsg::Sample(s)) => TrySendError::Closed(s),
        // Inatteignable : `Writer` n'envoie jamais `Shutdown`.
        _ => unreachable!("Writer n'émet jamais Shutdown"),
    }
}

/// Handle clonable côté producteur.
#[derive(Clone)]
pub struct Writer {
    pub(crate) tx: mpsc::Sender<WriterMsg>,
}

impl Writer {
    pub async fn write(&self, sample: Sample) -> Result<(), mpsc::error::SendError<Sample>> {
        self.tx.send(WriterMsg::Sample(sample)).await.map_err(map_send_err)
    }

    pub fn try_write(&self, sample: Sample) -> Result<(), mpsc::error::TrySendError<Sample>> {
        self.tx.try_send(WriterMsg::Sample(sample)).map_err(map_try_send_err)
    }

    pub fn blocking_write(&self, sample: Sample) -> Result<(), mpsc::error::SendError<Sample>> {
        self.tx.blocking_send(WriterMsg::Sample(sample)).map_err(map_send_err)
    }
}

/// Démarre le thread writer. Retourne son `JoinHandle` (le caller peut
/// l'attendre lors d'un shutdown propre ; en pratique le service tourne
/// jusqu'à `SIGTERM` systemd).
pub(crate) fn spawn(
    db: Arc<Database>,
    rx: mpsc::Receiver<WriterMsg>,
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

fn run(db: Arc<Database>, mut rx: mpsc::Receiver<WriterMsg>, cfg: WriterConfig) -> Result<()> {
    // Cache keyé par fingerprint u64 calculé depuis des références (zéro
    // allocation/clone sur cache-hit, le cas nominal). La valeur stocke
    // l'identité `(metric, labels)` pour pouvoir détecter une collision de
    // hash (astronomiquement rare) et retomber sur le lookup canonique.
    let mut series_cache: SeriesCache = LruCache::new(
        NonZeroUsize::new(SERIES_CACHE_CAPACITY).unwrap(),
    );

    let mut next_id = load_next_id(&db).context("load_next_id")?;

    loop {
        let (batch, stop) = drain(&mut rx, &cfg);

        if !batch.is_empty() {
            if let Err(e) = commit_batch(&db, &batch, &mut series_cache, &mut next_id) {
                tracing::error!(error = %e, samples = batch.len(), "commit_batch a échoué");
            }
        }

        if stop {
            return Ok(()); // channel fermé ou Shutdown reçu
        }
    }
}

/// Draine un batch. Retourne `(batch, stop)` où `stop` indique que le canal
/// est fermé ou qu'un `Shutdown` a été reçu — le batch collecté (les samples
/// déjà en file) est tout de même commité avant l'arrêt, sans perte.
fn drain(rx: &mut mpsc::Receiver<WriterMsg>, cfg: &WriterConfig) -> (Vec<Sample>, bool) {
    let mut batch = Vec::with_capacity(cfg.batch_max);

    // Premier message : on bloque jusqu'à réception (sommeil efficace, pas
    // de busy-poll quand la file est vide).
    match rx.blocking_recv() {
        Some(WriterMsg::Sample(s)) => batch.push(s),
        Some(WriterMsg::Shutdown) => return (batch, true),
        None => return (batch, true), // canal fermé
    }

    let deadline = Instant::now() + Duration::from_millis(cfg.flush_ms);
    let idle = Duration::from_millis(cfg.poll_idle_ms);

    while batch.len() < cfg.batch_max && Instant::now() < deadline {
        match rx.try_recv() {
            Ok(WriterMsg::Sample(s)) => batch.push(s),
            Ok(WriterMsg::Shutdown) => return (batch, true),
            Err(mpsc::error::TryRecvError::Empty) => {
                // M3 : ne jamais dormir au-delà de la deadline de flush.
                let remaining = deadline.saturating_duration_since(Instant::now());
                thread::sleep(idle.min(remaining));
            }
            Err(mpsc::error::TryRecvError::Disconnected) => return (batch, true),
        }
    }

    (batch, false)
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
    cache: &mut SeriesCache,
    next_id: &mut u32,
) -> Result<()> {
    let wtx = db.begin_write()?;
    // R1 : on mémorise l'état d'allocation d'ID au début du batch pour
    // pouvoir restaurer l'état mémoire à l'identique si le commit échoue.
    let next_id_start = *next_id;
    let mut next_id_dirty = false;

    {
        let mut t_raw = wtx.open_table(TABLE_RAW)?;
        let mut t_skey = wtx.open_table(TABLE_SERIES_BY_KEY)?;
        let mut t_smeta = wtx.open_table(TABLE_SERIES_META)?;

        for s in batch {
            // P1 : fingerprint u64 calculé depuis des références — aucune
            // allocation ni clone sur cache-hit (le cas nominal >99%). Le
            // `canonical_json` (coûteux) et les clones ne se produisent que
            // sur cache-miss. La vérification d'identité ci-dessous garantit
            // qu'une collision de hash (extrêmement rare) ne renvoie jamais
            // un mauvais `series_id` : on retombe alors sur le lookup
            // canonique. Deux ordres de labels différents produisent deux
            // entrées de cache mais résolvent vers le **même** series_id via
            // la clé canonique de `series_by_key`.
            let fp = series_fingerprint(&s.metric, &s.labels);
            let cached_id = match cache.get(&fp) {
                Some((m, l, id)) if m == &s.metric && l == &s.labels => Some(*id),
                _ => None, // miss ou collision de hash
            };

            let series_id = if let Some(id) = cached_id {
                id
            } else {
                let labels_json = canonical_json(&s.labels);
                let lookup_key = make_lookup_key(&s.metric, &labels_json);

                // Lookup avant insert : on extrait la valeur du guard
                // immédiatement pour libérer l'emprunt immutable sur `t_skey`
                // avant d'appeler `.insert()` (sinon borrow checker fail).
                // NB : au sein de la même write-tx, ce `get` voit les inserts
                // précédents du batch (déduplication intra-batch correcte).
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
                        labels_json,
                        first_seen_ms: s.ts_ms,
                        last_seen_ms: s.ts_ms,
                    };

                    let bytes = bincode::serialize(&meta)?;
                    t_smeta.insert(id, &bytes[..])?;

                    id
                };

                // Clone uniquement sur miss : stocke l'identité pour la
                // vérification anti-collision des hits futurs.
                cache.put(fp, (s.metric.clone(), s.labels.clone(), id));

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

    // R1 : si le commit échoue, la transaction (inserts series_by_key /
    // series_meta inclus) est annulée. Le cache contiendrait alors des
    // series_id jamais persistés → futurs points raw orphelins. On purge
    // donc le cache et on restaure `next_id` pour rester cohérent avec la
    // base. Le cache n'est qu'une optimisation : il sera repeuplé depuis la
    // base au batch suivant.
    if let Err(e) = wtx.commit() {
        *next_id = next_id_start;
        cache.clear();
        return Err(e.into());
    }

    Ok(())
}
