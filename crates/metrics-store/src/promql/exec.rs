//! Évaluateur PromQL minimal — couvre le golden set §6.5.
//!
//! ## Couverture
//! - `VectorSelector` (instant) avec matchers `= != =~ !~`
//! - Arithmétique : `+ - * /` entre vecteurs et scalaires (alignement
//!   par labels hors `__name__`)
//! - Agrégations instant : `sum max min avg count`
//! - Fonctions à fenêtre : `increase rate delta avg_over_time
//!   sum_over_time min_over_time max_over_time count_over_time
//!   last_over_time`
//! - Fonctions instant : `abs clamp_min clamp_max ceil floor round`
//!
//! ## Sélection automatique de tier (§6.3)
//! - matrix range ≤ 7 j → `TABLE_RAW`
//! - 7 j < range ≤ 90 j → `TABLE_HOURLY`
//! - > 90 j → `TABLE_DAILY`
//!
//! Pour `increase`/`rate`, on somme les incréments par paire adjacente :
//! sur points raw via `windows(2)` (`raw_counter_increase`), sur buckets
//! compactés au sein de chaque bucket et entre buckets adjacents
//! (`buckets_counter_increase`). Un reset (`cur < prev`) est traité comme
//! une remise à zéro. La somme télescope vers `last - first` pour un
//! compteur monotone (cas normal), et gère les resets intermédiaires.
//! Limite résiduelle sur tier compacté : un reset interne à un bucket
//! horaire/journalier reste invisible (points raw déjà purgés).
//!
//! ## Optimisations mémoire (cf. docs/memory-leak-investigation.md §12)
//! L'Evaluator est scopé per-request et porte trois caches partagés sur
//! toute la durée d'un `eval_range` :
//! 1. `read_txn` : une seule `ReadTransaction` redb, évite N×
//!    `begin_read` + `open_table` (1 par série × step).
//! 2. `series_catalog` : `Arc<Vec<SeriesMeta>>` chargé 1×, partagé entre
//!    tous les VectorSelectors.
//! 3. `match_cache` keyé par **adresse mémoire** du `VectorSelector` —
//!    l'AST est immutable pendant `eval_range` donc les pointeurs sont
//!    stables. Évite le calcul de fingerprint + allocations de clé à
//!    chaque step.
//!
//! `InstantSample.labels` est un `Arc<Labels>` pour propager les labels
//! d'un step à l'autre sans cloner le `BTreeMap` (les Labels sont
//! co-possédés par le cache et tous les samples qui en dérivent).

use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use promql_parser::label::MatchOp;
use promql_parser::parser::{
    AggregateExpr, BinaryExpr, Call, Expr, MatrixSelector, NumberLiteral, ParenExpr, UnaryExpr,
    VectorSelector,
};
use redb::ReadTransaction;

use crate::reader::Reader;
use crate::tables::{AggBucket, SeriesMeta, Tier};

use super::error::PromQlError;

pub type Labels = BTreeMap<String, String>;

#[derive(Debug, Clone)]
pub struct InstantSample {
    /// Labels partagés via `Arc` — propagation sans clone du BTreeMap.
    /// Pour produire un `Labels` owned (sortie de l'évaluateur), faire
    /// `Arc::try_unwrap(arc).unwrap_or_else(|a| (*a).clone())`.
    pub labels: Arc<Labels>,
    pub value: f64,
}

#[derive(Debug, Clone)]
pub struct RangeSeries {
    pub labels: Labels,
    pub samples: Vec<(i64, f64)>,
}

#[derive(Debug, Clone)]
enum Value {
    Scalar(f64),
    Vector(Vec<InstantSample>),
}

const HOURLY_THRESHOLD_MS: i64 = 7 * 86_400_000;
const DAILY_THRESHOLD_MS: i64 = 90 * 86_400_000;

fn tier_for_range(range_ms: i64) -> Tier {
    if range_ms <= HOURLY_THRESHOLD_MS {
        Tier::Raw
    } else if range_ms <= DAILY_THRESHOLD_MS {
        Tier::Hourly
    } else {
        Tier::Daily
    }
}

pub struct Evaluator<'r> {
    reader: &'r Reader,
    /// Lookback pour les instant vector selectors (analogue à Prometheus,
    /// défaut 5 min).
    pub lookback_ms: i64,
    /// Transaction de lecture redb partagée sur toute la durée de l'évaluation.
    /// Lazy-initialisée au premier accès via `txn()`.
    read_txn: OnceCell<ReadTransaction>,
    /// Catalogue `TABLE_SERIES_META` chargé 1× puis partagé via `Arc`.
    series_catalog: OnceCell<Arc<Vec<(u32, SeriesMeta)>>>,
    /// Cache `match_series` keyé par adresse mémoire du `VectorSelector`.
    /// L'AST PromQL est immutable pendant l'évaluation donc le pointeur
    /// est stable. Évite tout calcul de fingerprint après le 1er step.
    match_cache: RefCell<HashMap<usize, Arc<Vec<(u32, Arc<Labels>)>>>>,
}

impl<'r> Evaluator<'r> {
    pub fn new(reader: &'r Reader) -> Self {
        Self {
            reader,
            lookback_ms: 5 * 60_000,
            read_txn: OnceCell::new(),
            series_catalog: OnceCell::new(),
            match_cache: RefCell::new(HashMap::new()),
        }
    }

    pub fn with_lookback(reader: &'r Reader, lookback: Duration) -> Self {
        Self {
            reader,
            lookback_ms: lookback.as_millis() as i64,
            read_txn: OnceCell::new(),
            series_catalog: OnceCell::new(),
            match_cache: RefCell::new(HashMap::new()),
        }
    }

    /// Évalue `expr` aux instants `start, start+step, …, end`. Renvoie une
    /// série par combinaison de labels.
    pub fn eval_range(
        &self,
        expr: &Expr,
        start_ms: i64,
        end_ms: i64,
        step_ms: i64,
    ) -> Result<Vec<RangeSeries>, PromQlError> {
        if step_ms <= 0 {
            return Err(PromQlError::Execution("step must be > 0".into()));
        }
        if end_ms < start_ms {
            return Err(PromQlError::Execution("end < start".into()));
        }
        // M1 : le cache est keyé par adresse mémoire de `VectorSelector`.
        // On le vide avant toute évaluation pour qu'une réutilisation de
        // l'Evaluator sur une autre expression (dont un nœud aurait été
        // réalloué à la même adresse) ne renvoie jamais un résultat périmé.
        self.match_cache.borrow_mut().clear();
        let mut by_labels: BTreeMap<Arc<Labels>, Vec<(i64, f64)>> = BTreeMap::new();
        // Sentinelle pour les valeurs scalaires (labels vides). Partagée
        // sur tous les steps scalaires pour éviter N allocations.
        let scalar_key: Arc<Labels> = Arc::new(Labels::new());
        let mut t = start_ms;
        while t <= end_ms {
            match self.eval_at(expr, t)? {
                Value::Scalar(v) => {
                    by_labels
                        .entry(scalar_key.clone())
                        .or_default()
                        .push((t, v));
                }
                Value::Vector(samples) => {
                    for s in samples {
                        by_labels.entry(s.labels).or_default().push((t, s.value));
                    }
                }
            }
            t = t.saturating_add(step_ms);
        }
        // Libère les Arc<Labels> co-détenus par le cache et la sentinelle
        // locale — sinon `Arc::try_unwrap` ci-dessous échoue toujours et
        // retombe sur le clone fallback. L'Evaluator est scoped per-request
        // (un seul eval_range par instance) donc vider le cache ici n'a
        // pas d'effet de bord. Cf. review Gemini PR #482.
        drop(scalar_key);
        self.match_cache.borrow_mut().clear();
        Ok(by_labels
            .into_iter()
            .map(|(labels_arc, samples)| {
                let labels = Arc::try_unwrap(labels_arc).unwrap_or_else(|arc| (*arc).clone());
                RangeSeries { labels, samples }
            })
            .collect())
    }

    /// Évalue `expr` à un instant unique (équivalent `/api/v1/query`).
    pub fn eval_instant(&self, expr: &Expr, t_ms: i64) -> Result<Vec<InstantSample>, PromQlError> {
        // M1 : cf. note dans `eval_range` — invalide le cache keyé par
        // pointeur avant toute évaluation pour éviter un faux positif
        // d'adresse mémoire réutilisée.
        self.match_cache.borrow_mut().clear();
        match self.eval_at(expr, t_ms)? {
            Value::Scalar(v) => Ok(vec![InstantSample { labels: Arc::new(Labels::new()), value: v }]),
            Value::Vector(s) => Ok(s),
        }
    }

    // ── interne ──────────────────────────────────────────────────────────

    /// Lazy-init de la `ReadTransaction` partagée. Renvoie une référence
    /// vivante tant que l'Evaluator existe.
    fn txn(&self) -> Result<&ReadTransaction, PromQlError> {
        if let Some(rtx) = self.read_txn.get() {
            return Ok(rtx);
        }
        let rtx = self
            .reader
            .begin_read()
            .map_err(|e| PromQlError::Execution(e.to_string()))?;
        // set() ne peut échouer qu'en cas de double init, qu'on a écarté
        // par le get() ci-dessus en mono-thread.
        let _ = self.read_txn.set(rtx);
        Ok(self.read_txn.get().expect("read_txn juste initialisée"))
    }

    fn eval_at(&self, expr: &Expr, t: i64) -> Result<Value, PromQlError> {
        match expr {
            Expr::NumberLiteral(NumberLiteral { val, .. }) => Ok(Value::Scalar(*val)),
            Expr::Paren(ParenExpr { expr }) => self.eval_at(expr, t),
            Expr::Unary(UnaryExpr { expr }) => match self.eval_at(expr, t)? {
                Value::Scalar(v) => Ok(Value::Scalar(-v)),
                Value::Vector(v) => Ok(Value::Vector(
                    v.into_iter()
                        .map(|s| InstantSample { labels: s.labels, value: -s.value })
                        .collect(),
                )),
            },
            Expr::VectorSelector(vs) => self.eval_vector_selector(vs, t),
            Expr::MatrixSelector(_) => Err(PromQlError::Execution(
                "matrix selector ne peut apparaître qu'à l'intérieur d'une fonction".into(),
            )),
            Expr::Binary(b) => self.eval_binary(b, t),
            Expr::Aggregate(a) => self.eval_aggregate(a, t),
            Expr::Call(c) => self.eval_call(c, t),
            Expr::Subquery(_) | Expr::Extension(_) | Expr::StringLiteral(_) => Err(
                PromQlError::Execution("expr non supportée par l'évaluateur".into()),
            ),
        }
    }

    fn eval_vector_selector(&self, vs: &VectorSelector, t: i64) -> Result<Value, PromQlError> {
        let matched = self.match_series(vs)?;
        let rtx = self.txn()?;
        let mut out = Vec::with_capacity(matched.len());
        for (sid, labels) in matched.iter() {
            // P2 : seul le dernier point de la fenêtre de lookback est
            // nécessaire — scan inverse au lieu de matérialiser tout le Vec.
            let last = self
                .reader
                .last_point_in_range_with_tx(rtx, *sid, t - self.lookback_ms, t)
                .map_err(|e| PromQlError::Execution(e.to_string()))?;
            if let Some((_, v)) = last {
                out.push(InstantSample { labels: labels.clone(), value: v });
            }
        }
        Ok(Value::Vector(out))
    }

    fn eval_binary(&self, b: &BinaryExpr, t: i64) -> Result<Value, PromQlError> {
        let lhs = self.eval_at(&b.lhs, t)?;
        let rhs = self.eval_at(&b.rhs, t)?;
        let op = b.op.to_string();
        let scalar_fn: fn(f64, f64) -> f64 = match op.as_str() {
            "+" => |a, b| a + b,
            "-" => |a, b| a - b,
            "*" => |a, b| a * b,
            "/" => |a, b| a / b,
            other => return Err(PromQlError::Unsupported(format!("binary {other}"))),
        };
        match (lhs, rhs) {
            (Value::Scalar(a), Value::Scalar(c)) => Ok(Value::Scalar(scalar_fn(a, c))),
            (Value::Scalar(a), Value::Vector(v)) => Ok(Value::Vector(
                v.into_iter()
                    .map(|s| InstantSample { labels: s.labels, value: scalar_fn(a, s.value) })
                    .collect(),
            )),
            (Value::Vector(v), Value::Scalar(c)) => Ok(Value::Vector(
                v.into_iter()
                    .map(|s| InstantSample { labels: s.labels, value: scalar_fn(s.value, c) })
                    .collect(),
            )),
            (Value::Vector(lhs), Value::Vector(rhs)) => Ok(Value::Vector(align_and_op(
                lhs, rhs, scalar_fn,
            ))),
        }
    }

    fn eval_aggregate(&self, a: &AggregateExpr, t: i64) -> Result<Value, PromQlError> {
        let inner = match self.eval_at(&a.expr, t)? {
            Value::Vector(v) => v,
            Value::Scalar(s) => return Ok(Value::Scalar(s)),
        };
        if inner.is_empty() {
            return Ok(Value::Vector(vec![]));
        }
        let op = a.op.to_string();
        let value = match op.as_str() {
            "sum" => inner.iter().map(|s| s.value).sum::<f64>(),
            "max" => inner
                .iter()
                .map(|s| s.value)
                .fold(f64::NEG_INFINITY, f64::max),
            "min" => inner.iter().map(|s| s.value).fold(f64::INFINITY, f64::min),
            "avg" => inner.iter().map(|s| s.value).sum::<f64>() / inner.len() as f64,
            "count" => inner.len() as f64,
            other => return Err(PromQlError::Unsupported(format!("aggregator {other}"))),
        };
        Ok(Value::Vector(vec![InstantSample {
            labels: Arc::new(Labels::new()),
            value,
        }]))
    }

    fn eval_call(&self, c: &Call, t: i64) -> Result<Value, PromQlError> {
        let name = c.func.name;
        // Fonctions à fenêtre : 1er arg = MatrixSelector.
        if let Some(first) = c.args.first() {
            if let Expr::MatrixSelector(MatrixSelector { vs, range }) = first.as_ref() {
                return self.eval_range_call(name, vs, range, t);
            }
        }
        // Fonctions instantanées : 1er arg = vecteur, args suivants = scalaires.
        let inner = match self.eval_at(c.args.args[0].as_ref(), t)? {
            Value::Vector(v) => v,
            Value::Scalar(s) => return Ok(Value::Scalar(self.apply_instant_scalar(name, s, c, t)?)),
        };
        self.apply_instant_fn(name, inner, c, t)
    }

    fn eval_range_call(
        &self,
        name: &str,
        vs: &VectorSelector,
        range: &Duration,
        t: i64,
    ) -> Result<Value, PromQlError> {
        let range_ms = range.as_millis() as i64;
        let win_start = t - range_ms;
        let tier = tier_for_range(range_ms);
        let matched = self.match_series(vs)?;
        let rtx = self.txn()?;
        let mut out = Vec::with_capacity(matched.len());
        // P2 : `delta` (différence de jauge) et `last_over_time` n'ont besoin
        // que des bornes → on évite de charger toute la fenêtre. `increase` et
        // `rate` chargent tous les points pour sommer les incréments par paire
        // et gérer correctement les resets de compteur intermédiaires (revue
        // Gemini PR #521) — leur somme télescope vers `last - first` pour un
        // compteur monotone, donc aucun changement sur les données normales.
        let endpoints_only = matches!(name, "delta" | "last_over_time");
        for (sid, labels) in matched.iter() {
            let value_opt = match tier {
                Tier::Raw if endpoints_only => {
                    let fl = self
                        .reader
                        .first_last_in_range_with_tx(rtx, *sid, win_start, t)
                        .map_err(|e| PromQlError::Execution(e.to_string()))?;
                    apply_range_fn_endpoints(name, fl, range_ms)
                }
                Tier::Raw => {
                    let pts = self
                        .reader
                        .query_range_raw_with_tx(rtx, *sid, win_start, t)
                        .map_err(|e| PromQlError::Execution(e.to_string()))?;
                    apply_range_fn_raw(name, &pts, range_ms)
                }
                Tier::Hourly | Tier::Daily => {
                    let buckets = self
                        .reader
                        .query_range_buckets_with_tx(rtx, *sid, win_start, t, tier)
                        .map_err(|e| PromQlError::Execution(e.to_string()))?;
                    apply_range_fn_buckets(name, &buckets, range_ms)
                }
            };
            if let Some(v) = value_opt {
                out.push(InstantSample { labels: labels.clone(), value: v });
            }
        }
        Ok(Value::Vector(out))
    }

    fn apply_instant_fn(
        &self,
        name: &str,
        inner: Vec<InstantSample>,
        c: &Call,
        t: i64,
    ) -> Result<Value, PromQlError> {
        let out: Vec<InstantSample> = match name {
            "abs" => inner
                .into_iter()
                .map(|s| InstantSample { labels: s.labels, value: s.value.abs() })
                .collect(),
            "ceil" => inner
                .into_iter()
                .map(|s| InstantSample { labels: s.labels, value: s.value.ceil() })
                .collect(),
            "floor" => inner
                .into_iter()
                .map(|s| InstantSample { labels: s.labels, value: s.value.floor() })
                .collect(),
            "round" => inner
                .into_iter()
                .map(|s| InstantSample { labels: s.labels, value: s.value.round() })
                .collect(),
            "clamp_min" => {
                let k = scalar_arg(c, 1, t, self)?;
                inner
                    .into_iter()
                    .map(|s| InstantSample { labels: s.labels, value: s.value.max(k) })
                    .collect()
            }
            "clamp_max" => {
                let k = scalar_arg(c, 1, t, self)?;
                inner
                    .into_iter()
                    .map(|s| InstantSample { labels: s.labels, value: s.value.min(k) })
                    .collect()
            }
            other => {
                return Err(PromQlError::Unsupported(format!("instant fn {other}")))
            }
        };
        Ok(Value::Vector(out))
    }

    fn apply_instant_scalar(
        &self,
        name: &str,
        s: f64,
        c: &Call,
        t: i64,
    ) -> Result<f64, PromQlError> {
        Ok(match name {
            "abs" => s.abs(),
            "ceil" => s.ceil(),
            "floor" => s.floor(),
            "round" => s.round(),
            "clamp_min" => s.max(scalar_arg(c, 1, t, self)?),
            "clamp_max" => s.min(scalar_arg(c, 1, t, self)?),
            other => return Err(PromQlError::Unsupported(format!("instant fn {other}"))),
        })
    }

    /// Charge le catalogue `TABLE_SERIES_META` au plus une fois par Evaluator.
    /// Le résultat est partagé via `Arc` entre tous les appels de
    /// `match_series` dans la même `eval_range` (~200 steps × N VectorSelector).
    fn series_catalog(&self) -> Result<&Arc<Vec<(u32, SeriesMeta)>>, PromQlError> {
        if let Some(catalog) = self.series_catalog.get() {
            return Ok(catalog);
        }
        let all = self
            .reader
            .list_series_with_tx(self.txn()?)
            .map_err(|e| PromQlError::Execution(e.to_string()))?;
        let _ = self.series_catalog.set(Arc::new(all));
        Ok(self.series_catalog.get().expect("series_catalog juste initialisé"))
    }

    /// Récupère (en cache) les séries qui matchent un `VectorSelector`.
    ///
    /// La clé du cache est l'**adresse mémoire** du `VectorSelector` —
    /// l'AST PromQL est immutable et possédé par l'appelant pendant toute
    /// la durée de l'eval_range, donc le pointeur est stable et unique
    /// par nœud syntaxique. Cela évite tout coût de fingerprint après le
    /// premier step (~200 steps × 1 lookup ptr trivial).
    fn match_series(
        &self,
        vs: &VectorSelector,
    ) -> Result<Arc<Vec<(u32, Arc<Labels>)>>, PromQlError> {
        let key = vs as *const VectorSelector as usize;
        if let Some(cached) = self.match_cache.borrow().get(&key) {
            return Ok(cached.clone());
        }
        let metric = vs
            .name
            .as_deref()
            .ok_or_else(|| PromQlError::Execution("selector sans nom de métrique".into()))?;
        let matchers = &vs.matchers.matchers;
        let catalog = self.series_catalog()?.clone();
        let mut out = Vec::new();
        for (sid, meta) in catalog.iter() {
            if meta.metric != metric {
                continue;
            }
            let labels: Labels = serde_json::from_str(&meta.labels_json)
                .map_err(|e| PromQlError::Execution(format!("labels_json: {e}")))?;
            if matchers.iter().all(|m| {
                let v = labels.get(&m.name).map(String::as_str).unwrap_or("");
                m.is_match(v)
                    || (matches!(m.op, MatchOp::NotEqual | MatchOp::NotRe(_)) && v.is_empty())
            }) {
                // Inclut __name__ pour la compatibilité Grafana.
                let mut with_name = labels;
                with_name.insert("__name__".into(), metric.into());
                out.push((*sid, Arc::new(with_name)));
            }
        }
        let arc = Arc::new(out);
        self.match_cache.borrow_mut().insert(key, arc.clone());
        Ok(arc)
    }
}

fn scalar_arg(c: &Call, idx: usize, t: i64, ev: &Evaluator) -> Result<f64, PromQlError> {
    let arg = c
        .args
        .args
        .get(idx)
        .ok_or_else(|| PromQlError::Execution(format!("argument scalaire #{idx} manquant")))?;
    match ev.eval_at(arg, t)? {
        Value::Scalar(v) => Ok(v),
        Value::Vector(_) => Err(PromQlError::Execution(format!(
            "argument #{idx} doit être scalaire"
        ))),
    }
}

fn apply_range_fn_raw(name: &str, pts: &[(i64, f64)], range_ms: i64) -> Option<f64> {
    if pts.is_empty() {
        return match name {
            "count_over_time" => Some(0.0),
            _ => None,
        };
    }
    Some(match name {
        "increase" => raw_counter_increase(pts),
        "delta" => pts.last().unwrap().1 - pts.first().unwrap().1,
        "rate" => raw_counter_increase(pts) / (range_ms as f64 / 1000.0),
        "avg_over_time" => pts.iter().map(|p| p.1).sum::<f64>() / pts.len() as f64,
        "sum_over_time" => pts.iter().map(|p| p.1).sum::<f64>(),
        "min_over_time" => pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min),
        "max_over_time" => pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max),
        "count_over_time" => pts.len() as f64,
        "last_over_time" => pts.last().unwrap().1,
        _ => return None,
    })
}

/// Augmentation d'un compteur entre deux valeurs adjacentes `prev` → `cur`,
/// avec gestion d'un reset (remise à zéro) : si `cur < prev`, on suppose un
/// reset et on retient `cur` (équivalent à un reset à 0 puis remontée).
fn counter_increase(prev: f64, cur: f64) -> f64 {
    if cur < prev {
        cur
    } else {
        cur - prev
    }
}

/// Augmentation totale d'un compteur sur une série de points raw, en sommant
/// les incréments **par paire adjacente** (`windows(2)`). Détecte donc les
/// resets intermédiaires au sein de la fenêtre, pas seulement aux bornes
/// (revue Gemini PR #521). Pour un compteur monotone, la somme télescope
/// vers `last - first`.
fn raw_counter_increase(pts: &[(i64, f64)]) -> f64 {
    pts.windows(2).map(|w| counter_increase(w[0].1, w[1].1)).sum()
}

/// Augmentation totale d'un compteur sur des buckets compactés, en gérant les
/// resets à la fois **au sein** de chaque bucket (`first` → `last`) et **entre**
/// buckets adjacents (`prev.last` → `cur.first`). Télescope vers
/// `last_bucket.last - first_bucket.first` pour un compteur monotone.
fn buckets_counter_increase(buckets: &[(i64, AggBucket)]) -> f64 {
    let mut total = 0.0;
    let mut prev_last: Option<f64> = None;
    for (_, b) in buckets {
        if let Some(pl) = prev_last {
            total += counter_increase(pl, b.first);
        }
        total += counter_increase(b.first, b.last);
        prev_last = Some(b.last);
    }
    total
}

/// Fonctions de fenêtre qui n'ont besoin que des bornes : `delta` (différence
/// de jauge, peut être négative — pas de gestion de reset) et `last_over_time`.
/// `increase`/`rate` passent désormais par le chemin complet pour gérer les
/// resets intermédiaires (cf. `apply_range_fn_raw`).
fn apply_range_fn_endpoints(
    name: &str,
    fl: Option<((i64, f64), (i64, f64))>,
    _range_ms: i64,
) -> Option<f64> {
    let ((_, first_v), (_, last_v)) = fl?;
    Some(match name {
        "delta" => last_v - first_v,
        "last_over_time" => last_v,
        _ => return None,
    })
}

fn apply_range_fn_buckets(
    name: &str,
    buckets: &[(i64, AggBucket)],
    range_ms: i64,
) -> Option<f64> {
    if buckets.is_empty() {
        return match name {
            "count_over_time" => Some(0.0),
            _ => None,
        };
    }
    let total_cnt: u32 = buckets.iter().map(|(_, b)| b.cnt).sum();
    if total_cnt == 0 {
        return None;
    }
    let total_sum: f64 = buckets.iter().map(|(_, b)| b.sum).sum();
    Some(match name {
        "increase" => buckets_counter_increase(buckets),
        "delta" => buckets.last().unwrap().1.last - buckets.first().unwrap().1.first,
        "rate" => buckets_counter_increase(buckets) / (range_ms as f64 / 1000.0),
        "avg_over_time" => total_sum / total_cnt as f64,
        "sum_over_time" => total_sum,
        "min_over_time" => buckets.iter().map(|(_, b)| b.min).fold(f64::INFINITY, f64::min),
        "max_over_time" => buckets
            .iter()
            .map(|(_, b)| b.max)
            .fold(f64::NEG_INFINITY, f64::max),
        "count_over_time" => total_cnt as f64,
        "last_over_time" => buckets.last().unwrap().1.last,
        _ => return None,
    })
}

/// Aligne deux vecteurs par labels (hors `__name__`), applique l'opération
/// scalaire sur les paires alignées. Élimine `__name__` du résultat
/// (semantique PromQL standard).
fn align_and_op(
    lhs: Vec<InstantSample>,
    rhs: Vec<InstantSample>,
    op: fn(f64, f64) -> f64,
) -> Vec<InstantSample> {
    let rhs_idx: BTreeMap<Labels, f64> = rhs
        .into_iter()
        .map(|s| {
            let mut l = (*s.labels).clone();
            l.remove("__name__");
            (l, s.value)
        })
        .collect();
    // P4 : pré-dimensionne le résultat (au plus min(|lhs|, |rhs|) paires
    // alignées) pour éviter les réallocations successives du Vec.
    let mut out = Vec::with_capacity(lhs.len().min(rhs_idx.len()));
    for s in lhs {
        let mut key = (*s.labels).clone();
        key.remove("__name__");
        if let Some(&rval) = rhs_idx.get(&key) {
            out.push(InstantSample { labels: Arc::new(key), value: op(s.value, rval) });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(vals: &[f64]) -> Vec<(i64, f64)> {
        vals.iter().enumerate().map(|(i, v)| (i as i64 * 1000, *v)).collect()
    }

    #[test]
    fn raw_counter_increase_monotonic_telescopes() {
        // Compteur monotone → somme des incréments = last - first.
        assert_eq!(raw_counter_increase(&pts(&[10.0, 20.0, 30.0])), 20.0);
        assert_eq!(raw_counter_increase(&pts(&[0.0, 90.0])), 90.0);
    }

    #[test]
    fn raw_counter_increase_handles_intermediate_reset() {
        // Scénario de la revue Gemini : [10, 20, 5, 15] → (20-10)+5+(15-5)=25.
        assert_eq!(raw_counter_increase(&pts(&[10.0, 20.0, 5.0, 15.0])), 25.0);
    }

    #[test]
    fn raw_counter_increase_edge_cases() {
        assert_eq!(raw_counter_increase(&pts(&[])), 0.0);
        assert_eq!(raw_counter_increase(&pts(&[42.0])), 0.0);
    }

    fn bucket(first: f64, last: f64) -> AggBucket {
        AggBucket { avg: 0.0, min: 0.0, max: 0.0, sum: 0.0, first, last, cnt: 1 }
    }

    #[test]
    fn buckets_counter_increase_monotonic_telescopes() {
        let bs = vec![(0, bucket(0.0, 10.0)), (1, bucket(10.0, 25.0))];
        // within b1 (10) + between (0) + within b2 (15) = 25 = last.last - first.first.
        assert_eq!(buckets_counter_increase(&bs), 25.0);
    }

    #[test]
    fn buckets_counter_increase_handles_reset_between_and_within() {
        // Reset entre buckets : b1 monte à 20, b2 repart de 0 → 5.
        let bs = vec![(0, bucket(0.0, 20.0)), (1, bucket(0.0, 5.0))];
        // within b1 (20) + between counter(20→0)=0 + within b2 (5) = 25.
        assert_eq!(buckets_counter_increase(&bs), 25.0);

        // Reset au sein d'un bucket (first > last).
        let bs = vec![(0, bucket(20.0, 5.0))];
        assert_eq!(buckets_counter_increase(&bs), 5.0);
    }
}
