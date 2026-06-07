//! Évaluateur PromQL — couvre le golden set §6.5 + extensions roadmap
//! promql-compat (cf. `docs/metriques-promql-reference.md`).
//!
//! ## Couverture
//! - `VectorSelector` (instant) avec matchers `= != =~ !~`
//! - Arithmétique : `+ - * /` (vec×scalar, vec×vec aligné hors `__name__`)
//! - Comparaisons : `== != > < >= <=` (filtre, ou 0/1 avec `bool`) — Phase 3a
//! - Agrégations instant : `sum max min avg count` avec `by`/`without`
//!   (Phase 2) ; `topk`/`bottomk` (Phase 3b, labels d'origine conservés)
//! - Fonctions à fenêtre : `increase rate irate delta deriv predict_linear
//!   changes resets avg_over_time sum_over_time min_over_time max_over_time
//!   count_over_time last_over_time stddev_over_time stdvar_over_time
//!   quantile_over_time` (`irate` Phase 3c ; deriv/predict_linear/stats Phase 5)
//! - Fonctions instant : `abs clamp clamp_min clamp_max ceil floor round
//!   sqrt exp ln log2 log10 sgn` (Phase 4 ; `round(v, to_nearest)` honoré)
//! - Labels : `label_replace label_join` (Phase 4)
//! - Absence : `absent` (vecteur instant) / `absent_over_time` (range) — Phase 5
//!
//! ## Évolution conformité (cf. `docs/metriques-promql-reference.md`)
//! - Modificateurs temporels : `offset` **et** `@ <ts>`/`@ start()`/`@ end()`
//!   (`resolve_eval_time`).
//! - Opérateurs ensemblistes : `and`/`or`/`unless` (`eval_set_op`), avec
//!   `on`/`ignoring` éventuel.
//! - Matching vectoriel : `on`/`ignoring` + `group_left`/`group_right`
//!   (`vector_binary_op` + `result_metric`) pour arithmétique et comparaisons.
//! - Agrégateurs : `quantile` (percentile spatial), `group`, `count_values`,
//!   `stddev`, `stdvar`.
//! - Sous-requêtes : `expr[range:step]` (`eval_subquery_call`) — matérialise une
//!   série synthétique par sous-échantillonnage puis applique la fonction fenêtre.
//! - Arithmétique étendue : `%` (modulo) et `^` (puissance).
//! - P0 : `rate`/`increase` retournent *no data* (et non `0`) sur un seul point.
//!
//! ## Sémantique des labels
//! Agrégations (sauf topk/bottomk) et comparaisons **droppent `__name__`** ;
//! topk/bottomk et les fonctions de fenêtre **conservent** les labels.
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
//! ## Optimisations mémoire (cf. docs/diagnostic-depannage.md §12)
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

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use promql_parser::label::MatchOp;
use promql_parser::parser::{
    AggregateExpr, AtModifier, BinModifier, BinaryExpr, Call, Expr, LabelModifier, MatrixSelector,
    NumberLiteral, Offset, ParenExpr, StringLiteral, SubqueryExpr, UnaryExpr,
    VectorMatchCardinality, VectorSelector,
};
use redb::ReadTransaction;
use regex::Regex;

use crate::reader::Reader;
use crate::tables::{AggBucket, SeriesMeta, Tier};

use super::error::PromQlError;

/// Décalage temporel (ms) induit par un modificateur `offset` PromQL.
///
/// Sémantique PromQL : `expr offset d` évalue le sélecteur à `t - d`.
/// - `Offset::Pos(d)` (`offset 5m`) → décalage `+d` vers le passé.
/// - `Offset::Neg(d)` (`offset -5m`) → décalage `-d` (vers le futur).
/// - aucun offset → 0.
///
/// L'appelant calcule donc l'instant effectif `t_eff = t - offset_ms(&vs.offset)`.
///
/// Robustesse : conversion `u128 → i64` saturée (`try_into`) et négation saturée
/// pour éviter toute panique (debug) ou inversion de signe sur des durées
/// d'offset extrêmes (revue Gemini).
fn offset_ms(off: &Option<Offset>) -> i64 {
    match off {
        Some(Offset::Pos(d)) => d.as_millis().try_into().unwrap_or(i64::MAX),
        Some(Offset::Neg(d)) => d
            .as_millis()
            .try_into()
            .map(|ms: i64| ms.saturating_neg())
            .unwrap_or(i64::MIN),
        None => 0,
    }
}

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

/// Résolution par défaut d'une sous-requête sans `step` explicite (1 min).
const DEFAULT_SUBQUERY_STEP_MS: i64 = 60_000;
/// Borne supérieure du nombre de sous-pas d'une sous-requête (anti-explosion
/// mémoire/CPU). Au-delà, l'évaluation est rejetée avec un message explicite.
const MAX_SUBQUERY_STEPS: i64 = 11_000;

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
    #[allow(clippy::type_complexity)]
    match_cache: RefCell<HashMap<usize, Arc<Vec<(u32, Arc<Labels>)>>>>,
    /// Bornes `start`/`end` de la requête courante — nécessaires pour résoudre
    /// les modificateurs `@ start()` / `@ end()`. Renseignées au début de
    /// `eval_range` / `eval_instant`.
    query_start_ms: Cell<i64>,
    query_end_ms: Cell<i64>,
}

impl<'r> Evaluator<'r> {
    pub fn new(reader: &'r Reader) -> Self {
        Self {
            reader,
            lookback_ms: 5 * 60_000,
            read_txn: OnceCell::new(),
            series_catalog: OnceCell::new(),
            match_cache: RefCell::new(HashMap::new()),
            query_start_ms: Cell::new(0),
            query_end_ms: Cell::new(0),
        }
    }

    pub fn with_lookback(reader: &'r Reader, lookback: Duration) -> Self {
        Self {
            reader,
            lookback_ms: lookback.as_millis() as i64,
            read_txn: OnceCell::new(),
            series_catalog: OnceCell::new(),
            match_cache: RefCell::new(HashMap::new()),
            query_start_ms: Cell::new(0),
            query_end_ms: Cell::new(0),
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
        // Bornes pour `@ start()` / `@ end()`.
        self.query_start_ms.set(start_ms);
        self.query_end_ms.set(end_ms);
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
        // Pour une requête instant, `start` == `end` == t (cf. `@ start()/end()`).
        self.query_start_ms.set(t_ms);
        self.query_end_ms.set(t_ms);
        match self.eval_at(expr, t_ms)? {
            Value::Scalar(v) => Ok(vec![InstantSample { labels: Arc::new(Labels::new()), value: v }]),
            Value::Vector(s) => Ok(s),
        }
    }

    /// Résout l'instant d'évaluation effectif d'un sélecteur en tenant compte
    /// des modificateurs temporels `@` (fixe l'instant) **puis** `offset`
    /// (décale relativement). `@` est prioritaire : `m @ T offset O` évalue à
    /// `T - O`. Sans `@`, on part de l'instant courant `t`.
    fn resolve_eval_time(&self, at: &Option<AtModifier>, offset: &Option<Offset>, t: i64) -> i64 {
        let base = match at {
            Some(AtModifier::Start) => self.query_start_ms.get(),
            Some(AtModifier::End) => self.query_end_ms.get(),
            Some(AtModifier::At(time)) => {
                // SystemTime → ms epoch (peut être avant epoch → 0).
                match time.duration_since(UNIX_EPOCH) {
                    Ok(d) => d.as_millis().try_into().unwrap_or(i64::MAX),
                    Err(_) => 0,
                }
            }
            None => t,
        };
        base.saturating_sub(offset_ms(offset))
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
        // Modificateurs temporels `@` (fixe) puis `offset` (décale).
        let t_eff = self.resolve_eval_time(&vs.at, &vs.offset, t);
        let matched = self.match_series(vs)?;
        let rtx = self.txn()?;
        let mut out = Vec::with_capacity(matched.len());
        for (sid, labels) in matched.iter() {
            // P2 : seul le dernier point de la fenêtre de lookback est
            // nécessaire — scan inverse au lieu de matérialiser tout le Vec.
            let last = self
                .reader
                .last_point_in_range_with_tx(rtx, *sid, t_eff.saturating_sub(self.lookback_ms), t_eff)
                .map_err(|e| PromQlError::Execution(e.to_string()))?;
            if let Some((_, v)) = last {
                out.push(InstantSample { labels: labels.clone(), value: v });
            }
        }
        Ok(Value::Vector(out))
    }

    fn eval_binary(&self, b: &BinaryExpr, t: i64) -> Result<Value, PromQlError> {
        let op = b.op.to_string();
        // Opérateurs ensemblistes (`and`/`or`/`unless`) : logique dédiée
        // (filtrage par présence de labels, sans arithmétique). Les deux
        // opérandes doivent être des vecteurs instantanés.
        if matches!(op.as_str(), "and" | "or" | "unless") {
            let lhs = self.eval_at(&b.lhs, t)?;
            let rhs = self.eval_at(&b.rhs, t)?;
            return eval_set_op(&op, lhs, rhs, b.modifier.as_ref());
        }
        let lhs = self.eval_at(&b.lhs, t)?;
        let rhs = self.eval_at(&b.rhs, t)?;
        // Comparaisons (`== != > < >= <=`) : logique dédiée (filtre vs `bool`).
        if let Some(cmp) = cmp_fn(&op) {
            let return_bool = b.modifier.as_ref().map(|m| m.return_bool).unwrap_or(false);
            return self.eval_comparison(lhs, rhs, cmp, return_bool, b.modifier.as_ref());
        }
        let scalar_fn = arith_fn(&op)?;
        match (lhs, rhs) {
            (Value::Scalar(a), Value::Scalar(c)) => Ok(Value::Scalar(scalar_fn(a, c))),
            // NB : pour les opérations scalaire×vecteur on conserve les labels
            // d'origine (dont `__name__`) — comportement historique préservé
            // pour ne pas modifier l'affichage des séries existantes.
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
            (Value::Vector(lhs), Value::Vector(rhs)) => Ok(Value::Vector(vector_binary_op(
                lhs,
                rhs,
                b.modifier.as_ref(),
                true, // arithmétique → drop __name__
                &|l, r| Some(scalar_fn(l, r)),
            )?)),
        }
    }

    /// Évalue une comparaison (`== != > < >= <=`).
    ///
    /// - **sans `bool`** : filtre — seuls les samples dont la condition est
    ///   vraie sont conservés, **valeur inchangée**.
    /// - **avec `bool`** : tous les samples sont conservés, valeur = `1.0`/`0.0`.
    ///
    /// `__name__` est toujours retiré (sémantique de comparaison, §3a). Toute
    /// comparaison impliquant un `NaN` est fausse.
    fn eval_comparison(
        &self,
        lhs: Value,
        rhs: Value,
        cmp: fn(f64, f64) -> bool,
        return_bool: bool,
        modifier: Option<&BinModifier>,
    ) -> Result<Value, PromQlError> {
        // Applique la comparaison à un sample vecteur contre une valeur
        // scalaire. `lhs_is_vec` indique l'ordre des opérandes.
        let map_sample = |s: InstantSample, other: f64, lhs_is_vec: bool| -> Option<InstantSample> {
            let (a, b) = if lhs_is_vec { (s.value, other) } else { (other, s.value) };
            let cond = cmp_truth(cmp, a, b);
            if return_bool {
                Some(InstantSample {
                    labels: drop_name(&s.labels),
                    value: if cond { 1.0 } else { 0.0 },
                })
            } else if cond {
                Some(InstantSample { labels: drop_name(&s.labels), value: s.value })
            } else {
                None
            }
        };
        match (lhs, rhs) {
            (Value::Scalar(a), Value::Scalar(c)) => {
                // Le parser PromQL exige `bool` pour scalar/scalar — garde
                // défensive si jamais on arrivait ici sans.
                if !return_bool {
                    return Err(PromQlError::Execution(
                        "comparaison scalar/scalar requiert le modifier bool".into(),
                    ));
                }
                Ok(Value::Scalar(if cmp_truth(cmp, a, c) { 1.0 } else { 0.0 }))
            }
            (Value::Vector(v), Value::Scalar(c)) => Ok(Value::Vector(
                v.into_iter().filter_map(|s| map_sample(s, c, true)).collect(),
            )),
            (Value::Scalar(a), Value::Vector(v)) => Ok(Value::Vector(
                v.into_iter().filter_map(|s| map_sample(s, a, false)).collect(),
            )),
            (Value::Vector(lhs), Value::Vector(rhs)) => {
                // vec×vec : matching vectoriel (`on`/`ignoring`/`group_*`). En
                // mode filtre, on émet la valeur du lhs si la condition est vraie ;
                // en mode `bool`, on émet 0/1 pour chaque paire alignée.
                let combine = move |l: f64, r: f64| -> Option<f64> {
                    let cond = cmp_truth(cmp, l, r);
                    if return_bool {
                        Some(if cond { 1.0 } else { 0.0 })
                    } else if cond {
                        Some(l)
                    } else {
                        None
                    }
                };
                Ok(Value::Vector(vector_binary_op(
                    lhs, rhs, modifier, true, &combine,
                )?))
            }
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

        // Noms de labels du modifier (promql_parser::label::Labels { labels:
        // Vec<String> }) — à ne pas confondre avec exec::Labels (BTreeMap).
        let grp_names: Vec<String> = match &a.modifier {
            Some(m) => m.labels().labels.clone(),
            None => Vec::new(),
        };

        // Clé de groupe : sous-ensemble des labels d'un sample selon le
        // modifier. `__name__` est toujours retiré (sémantique d'agrégation).
        let group_key = |labels: &Labels| -> Labels {
            let mut g = Labels::new();
            match &a.modifier {
                // Sans modifier : 1 seul groupe, labels vides (collapse total).
                None => {}
                // by (...) : ne garder que les labels listés.
                Some(LabelModifier::Include(_)) => {
                    for k in &grp_names {
                        if let Some(v) = labels.get(k) {
                            g.insert(k.clone(), v.clone());
                        }
                    }
                }
                // without (...) : garder tous les labels SAUF ceux listés et
                // `__name__`.
                Some(LabelModifier::Exclude(_)) => {
                    for (k, v) in labels.iter() {
                        if k == "__name__" || grp_names.contains(k) {
                            continue;
                        }
                        g.insert(k.clone(), v.clone());
                    }
                }
            }
            g
        };

        // topk/bottomk : sélection des k samples extrêmes par groupe. Les
        // labels d'origine (y compris `__name__`) sont CONSERVÉS — on ne
        // collapse pas, on ne fait que filtrer/trier.
        if op == "topk" || op == "bottomk" {
            let k_val = match &a.param {
                Some(p) => match self.eval_at(p, t)? {
                    Value::Scalar(v) => v,
                    Value::Vector(_) => {
                        return Err(PromQlError::Execution(
                            "le paramètre de topk/bottomk doit être scalaire".into(),
                        ))
                    }
                },
                None => {
                    return Err(PromQlError::Execution(
                        "topk/bottomk requiert un paramètre k".into(),
                    ))
                }
            };
            // k négatif ou NaN → aucun sample.
            let k = if k_val.is_nan() || k_val < 0.0 { 0 } else { k_val as usize };
            let descending = op == "topk";
            // Partitionne par clé de groupe (clé seulement pour le découpage —
            // les samples de sortie gardent leurs labels d'origine).
            let mut groups: BTreeMap<Labels, Vec<InstantSample>> = BTreeMap::new();
            for s in inner {
                groups.entry(group_key(&s.labels)).or_default().push(s);
            }
            let mut out = Vec::new();
            for (_, mut samples) in groups {
                samples.sort_by(|x, y| {
                    let ord = x
                        .value
                        .partial_cmp(&y.value)
                        .unwrap_or(std::cmp::Ordering::Equal);
                    if descending {
                        ord.reverse()
                    } else {
                        ord
                    }
                });
                samples.truncate(k);
                out.extend(samples);
            }
            return Ok(Value::Vector(out));
        }

        // quantile(φ, vec) : φ-quantile spatial sur l'ensemble des séries du
        // groupe (interpolation linéaire). `__name__` retiré (agrégation).
        if op == "quantile" {
            let phi = match &a.param {
                Some(p) => match self.eval_at(p, t)? {
                    Value::Scalar(v) => v,
                    Value::Vector(_) => {
                        return Err(PromQlError::Execution(
                            "le paramètre de quantile doit être scalaire".into(),
                        ))
                    }
                },
                None => {
                    return Err(PromQlError::Execution("quantile requiert un paramètre φ".into()))
                }
            };
            let mut groups: BTreeMap<Labels, Vec<f64>> = BTreeMap::new();
            for s in &inner {
                groups.entry(group_key(&s.labels)).or_default().push(s.value);
            }
            let out = groups
                .into_iter()
                .map(|(labels, vals)| InstantSample {
                    labels: Arc::new(labels),
                    value: quantile_value(phi, vals),
                })
                .collect();
            return Ok(Value::Vector(out));
        }

        // count_values("label", vec) : compte les séries par valeur distincte,
        // en ajoutant un label `label=<valeur>` au groupe.
        if op == "count_values" {
            let label_name = match a.param.as_deref() {
                Some(Expr::StringLiteral(StringLiteral { val })) => val.clone(),
                _ => {
                    return Err(PromQlError::Execution(
                        "count_values requiert un paramètre chaîne (nom de label)".into(),
                    ))
                }
            };
            let mut groups: BTreeMap<Labels, u64> = BTreeMap::new();
            for s in &inner {
                let mut key = group_key(&s.labels);
                key.insert(label_name.clone(), fmt_value_label(s.value));
                *groups.entry(key).or_insert(0) += 1;
            }
            let out = groups
                .into_iter()
                .map(|(labels, cnt)| InstantSample { labels: Arc::new(labels), value: cnt as f64 })
                .collect();
            return Ok(Value::Vector(out));
        }

        // Accumulateur de Welford : `mean`/`m2` permettent de calculer la
        // variance de population de façon numériquement stable (évite
        // l'annulation catastrophique de `Σx²/n − (Σx/n)²` sur des compteurs
        // d'énergie cumulés à grande magnitude et faible variance).
        struct Acc {
            sum: f64,
            mean: f64,
            m2: f64,
            min: f64,
            max: f64,
            cnt: u64,
        }
        // BTreeMap comme clé de groupe ⇒ ordre de sortie déterministe.
        let mut groups: BTreeMap<Labels, Acc> = BTreeMap::new();
        for s in &inner {
            let e = groups.entry(group_key(&s.labels)).or_insert(Acc {
                sum: 0.0,
                mean: 0.0,
                m2: 0.0,
                min: f64::INFINITY,
                max: f64::NEG_INFINITY,
                cnt: 0,
            });
            e.sum += s.value;
            e.cnt += 1;
            // Mise à jour incrémentale de Welford.
            let delta = s.value - e.mean;
            e.mean += delta / e.cnt as f64;
            e.m2 += delta * (s.value - e.mean);
            e.min = e.min.min(s.value);
            e.max = e.max.max(s.value);
        }
        let mut out = Vec::with_capacity(groups.len());
        for (labels, acc) in groups {
            let n = acc.cnt as f64;
            let value = match op.as_str() {
                "sum" => acc.sum,
                "min" => acc.min,
                "max" => acc.max,
                "avg" => acc.sum / n,
                "count" => n,
                // `group` : présence binaire (valeur 1 par groupe).
                "group" => 1.0,
                // Variance / écart-type de population (Welford : `m2 / n`).
                "stdvar" => acc.m2 / n,
                "stddev" => (acc.m2 / n).sqrt(),
                other => return Err(PromQlError::Unsupported(format!("aggregator {other}"))),
            };
            out.push(InstantSample { labels: Arc::new(labels), value });
        }
        Ok(Value::Vector(out))
    }

    fn eval_call(&self, c: &Call, t: i64) -> Result<Value, PromQlError> {
        let name = c.func.name;
        // Fonctions de labels (`label_replace`/`label_join`) : arguments string
        // → routage dédié (ne passent pas par l'évaluation scalaire des args).
        if name == "label_replace" || name == "label_join" {
            let inner = match self.eval_at(c.args.args[0].as_ref(), t)? {
                Value::Vector(v) => v,
                Value::Scalar(_) => {
                    return Err(PromQlError::Execution(format!(
                        "{name}: le premier argument doit être un vecteur instantané"
                    )))
                }
            };
            return if name == "label_replace" {
                label_replace(inner, c)
            } else {
                label_join(inner, c)
            };
        }
        // `absent(v)` (vecteur instantané) et `absent_over_time(v[range])`
        // (vecteur de range) : sémantique spéciale (1 sample si la série est
        // absente, sinon vide) — nécessitent les matchers du sélecteur.
        if name == "absent" || name == "absent_over_time" {
            return self.eval_absent(c, t);
        }
        // Fonctions à fenêtre : le MatrixSelector peut être le 1er argument
        // (`rate(m[5m])`, `predict_linear(m[1h], 3600)`) ou le 2e
        // (`quantile_over_time(0.95, m[1d])`). On le localise n'importe où.
        if let Some((vs, range)) = c.args.args.iter().find_map(|a| match a.as_ref() {
            Expr::MatrixSelector(MatrixSelector { vs, range }) => Some((vs, range)),
            _ => None,
        }) {
            return self.eval_range_call(name, vs, range, c, t);
        }
        // Sous-requête : `f(<expr>[range:step])` — l'argument fenêtre est une
        // `Subquery` (et non un `MatrixSelector` sur un sélecteur simple).
        if let Some(sq) = c.args.args.iter().find_map(|a| match a.as_ref() {
            Expr::Subquery(sq) => Some(sq),
            _ => None,
        }) {
            return self.eval_subquery_call(name, sq, c, t);
        }
        // Fonctions instantanées : 1er arg = vecteur, args suivants = scalaires.
        let inner = match self.eval_at(c.args.args[0].as_ref(), t)? {
            Value::Vector(v) => v,
            Value::Scalar(s) => return Ok(Value::Scalar(self.apply_instant_scalar(name, s, c, t)?)),
        };
        self.apply_instant_fn(name, inner, c, t)
    }

    /// `absent(v)` / `absent_over_time(v[range])` : renvoie un vecteur vide si
    /// la série est présente, sinon un unique sample à `1` étiqueté des matchers
    /// d'égalité du sélecteur (alerting « série absente »).
    ///
    /// Le parser garantit le type de l'argument : `absent` reçoit toujours un
    /// vecteur instantané, `absent_over_time` toujours un `MatrixSelector`.
    fn eval_absent(&self, c: &Call, t: i64) -> Result<Value, PromQlError> {
        // Déroule les parenthèses éventuelles (`absent_over_time((m[5m]))`) pour
        // que le filtrage du MatrixSelector et l'extraction des labels du
        // sélecteur fonctionnent (revue Gemini PR #528).
        let mut arg = c.args.args[0].as_ref();
        while let Expr::Paren(ParenExpr { expr }) = arg {
            arg = expr.as_ref();
        }
        let present = match arg {
            // absent_over_time : présent si ≥ 1 série matchée a un point dans la
            // fenêtre.
            Expr::MatrixSelector(MatrixSelector { vs, range }) => {
                // Modificateurs `@`/`offset` : fenêtre fixée/décalée comme pour
                // les autres fonctions à fenêtre.
                let t_eff = self.resolve_eval_time(&vs.at, &vs.offset, t);
                let range_ms: i64 = range.as_millis().try_into().unwrap_or(i64::MAX);
                let win_start = t_eff.saturating_sub(range_ms);
                let matched = self.match_series(vs)?;
                let rtx = self.txn()?;
                let mut any = false;
                for (sid, _) in matched.iter() {
                    if self
                        .reader
                        .last_point_in_range_with_tx(rtx, *sid, win_start, t_eff)
                        .map_err(|e| PromQlError::Execution(e.to_string()))?
                        .is_some()
                    {
                        any = true;
                        break;
                    }
                }
                any
            }
            // absent : présent si le vecteur instantané a des éléments.
            _ => match self.eval_at(arg, t)? {
                Value::Vector(v) => !v.is_empty(),
                Value::Scalar(_) => true,
            },
        };
        if present {
            return Ok(Value::Vector(vec![]));
        }
        // Série absente → 1 sample à 1, étiqueté des matchers d'égalité.
        let vs_opt = match arg {
            Expr::VectorSelector(vs) => Some(vs),
            Expr::MatrixSelector(MatrixSelector { vs, .. }) => Some(vs),
            _ => None,
        };
        let mut labels = Labels::new();
        if let Some(vs) = vs_opt {
            for m in &vs.matchers.matchers {
                if matches!(m.op, MatchOp::Equal) && m.name != "__name__" {
                    labels.insert(m.name.clone(), m.value.clone());
                }
            }
        }
        Ok(Value::Vector(vec![InstantSample { labels: Arc::new(labels), value: 1.0 }]))
    }

    fn eval_range_call(
        &self,
        name: &str,
        vs: &VectorSelector,
        range: &Duration,
        c: &Call,
        t: i64,
    ) -> Result<Value, PromQlError> {
        let range_ms: i64 = range.as_millis().try_into().unwrap_or(i64::MAX);
        // Modificateurs temporels `@`/`offset` : la fenêtre `[win_start, t_eff]`
        // est fixée puis décalée.
        let t_eff = self.resolve_eval_time(&vs.at, &vs.offset, t);
        let win_start = t_eff.saturating_sub(range_ms);
        let tier = tier_for_range(range_ms);
        let matched = self.match_series(vs)?;
        // Paramètre scalaire éventuel = 1er argument qui n'est pas le
        // MatrixSelector (`predict_linear(m, T)` → T ; `quantile_over_time(φ, m)`
        // → φ). Évalué une seule fois.
        let param: Option<f64> = {
            let mut p = None;
            for a in &c.args.args {
                if !matches!(a.as_ref(), Expr::MatrixSelector(_)) {
                    match self.eval_at(a, t)? {
                        Value::Scalar(v) => p = Some(v),
                        Value::Vector(_) => {
                            return Err(PromQlError::Execution(format!(
                                "{name}: paramètre scalaire attendu"
                            )))
                        }
                    }
                    break;
                }
            }
            p
        };
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
                // `irate` (tier raw) : taux instantané sur les 2 derniers points.
                Tier::Raw if name == "irate" => {
                    let lt = self
                        .reader
                        .last_two_in_range_with_tx(rtx, *sid, win_start, t_eff)
                        .map_err(|e| PromQlError::Execution(e.to_string()))?;
                    apply_irate_last_two(lt)
                }
                Tier::Raw if endpoints_only => {
                    let fl = self
                        .reader
                        .first_last_in_range_with_tx(rtx, *sid, win_start, t_eff)
                        .map_err(|e| PromQlError::Execution(e.to_string()))?;
                    apply_range_fn_endpoints(name, fl, range_ms)
                }
                Tier::Raw => {
                    let pts = self
                        .reader
                        .query_range_raw_with_tx(rtx, *sid, win_start, t_eff)
                        .map_err(|e| PromQlError::Execution(e.to_string()))?;
                    apply_range_fn_raw(name, &pts, range_ms, t_eff, param)
                }
                Tier::Hourly | Tier::Daily => {
                    let buckets = self
                        .reader
                        .query_range_buckets_with_tx(rtx, *sid, win_start, t_eff, tier)
                        .map_err(|e| PromQlError::Execution(e.to_string()))?;
                    apply_range_fn_buckets(name, &buckets, range_ms, t_eff, param)
                }
            };
            if let Some(v) = value_opt {
                out.push(InstantSample { labels: labels.clone(), value: v });
            }
        }
        Ok(Value::Vector(out))
    }

    /// Évalue une fonction à fenêtre dont l'argument est une **sous-requête**
    /// `inner[range:step]`. On matérialise une série synthétique en évaluant
    /// `inner` à chaque sous-pas `step` sur `[t_eff-range, t_eff]`, puis on
    /// applique la fonction de fenêtre (`apply_range_fn_raw`) sur les points
    /// ainsi obtenus, par combinaison de labels.
    fn eval_subquery_call(
        &self,
        name: &str,
        sq: &SubqueryExpr,
        c: &Call,
        t: i64,
    ) -> Result<Value, PromQlError> {
        let range_ms: i64 = sq.range.as_millis().try_into().unwrap_or(i64::MAX);
        // Résolution interne : `step` explicite, sinon défaut 1 min (comme
        // l'intervalle d'évaluation global de Prometheus).
        let step_ms: i64 = sq
            .step
            .map(|d| d.as_millis().try_into().unwrap_or(i64::MAX))
            .filter(|s| *s > 0)
            .unwrap_or(DEFAULT_SUBQUERY_STEP_MS);
        // Modificateurs `@`/`offset` portés par la sous-requête elle-même.
        let t_eff = self.resolve_eval_time(&sq.at, &sq.offset, t);
        let win_start = t_eff.saturating_sub(range_ms);
        // Garde anti-explosion : borne le nombre de sous-pas.
        let n_steps = range_ms / step_ms + 1;
        if n_steps > MAX_SUBQUERY_STEPS {
            return Err(PromQlError::Execution(format!(
                "sous-requête trop fine : {n_steps} sous-pas (max {MAX_SUBQUERY_STEPS}) — \
                 élargir la résolution [range:step]"
            )));
        }
        // Paramètre scalaire éventuel (predict_linear T, quantile_over_time φ) =
        // 1er argument qui n'est ni la sous-requête ni un matrix selector.
        let param: Option<f64> = {
            let mut p = None;
            for a in &c.args.args {
                if !matches!(a.as_ref(), Expr::Subquery(_) | Expr::MatrixSelector(_)) {
                    match self.eval_at(a, t)? {
                        Value::Scalar(v) => p = Some(v),
                        Value::Vector(_) => {
                            return Err(PromQlError::Execution(format!(
                                "{name}: paramètre scalaire attendu"
                            )))
                        }
                    }
                    break;
                }
            }
            p
        };
        // Matérialise les points par combinaison de labels.
        let scalar_key: Arc<Labels> = Arc::new(Labels::new());
        let mut series: BTreeMap<Arc<Labels>, Vec<(i64, f64)>> = BTreeMap::new();
        let mut st = win_start;
        while st <= t_eff {
            match self.eval_at(&sq.expr, st)? {
                Value::Scalar(v) => {
                    series.entry(scalar_key.clone()).or_default().push((st, v));
                }
                Value::Vector(samples) => {
                    for s in samples {
                        series.entry(s.labels).or_default().push((st, s.value));
                    }
                }
            }
            // Garde anti-boucle infinie : si `st` est sur le point de saturer à
            // `i64::MAX` (cas `@` résolvant un timestamp extrême), `saturating_add`
            // figerait `st` à `t_eff` et `st <= t_eff` resterait vrai à jamais.
            if st > t_eff.saturating_sub(step_ms) {
                break;
            }
            st = st.saturating_add(step_ms);
        }
        // Applique la fonction de fenêtre sur chaque série synthétique. Les
        // points sont déjà triés par timestamp croissant (st croissant).
        let mut out = Vec::with_capacity(series.len());
        for (labels, pts) in series {
            let value_opt = if name == "irate" {
                let lt = if pts.len() >= 2 {
                    Some((pts[pts.len() - 2], pts[pts.len() - 1]))
                } else {
                    None
                };
                apply_irate_last_two(lt)
            } else {
                apply_range_fn_raw(name, &pts, range_ms, t_eff, param)
            };
            if let Some(v) = value_opt {
                out.push(InstantSample { labels, value: v });
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
            "round" => {
                // 2e argument optionnel `to_nearest` (défaut 1) — était
                // auparavant ignoré (résultat faux silencieux).
                let to_nearest = scalar_arg_opt(c, 1, t, self)?.unwrap_or(1.0);
                inner
                    .into_iter()
                    .map(|s| InstantSample { labels: s.labels, value: round_to(s.value, to_nearest) })
                    .collect()
            }
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
            "clamp" => {
                let min = scalar_arg(c, 1, t, self)?;
                let max = scalar_arg(c, 2, t, self)?;
                inner
                    .into_iter()
                    .map(|s| InstantSample { labels: s.labels, value: clamp_val(s.value, min, max) })
                    .collect()
            }
            "sqrt" | "exp" | "ln" | "log2" | "log10" | "sgn" => inner
                .into_iter()
                .map(|s| InstantSample { labels: s.labels, value: unary_math(name, s.value) })
                .collect(),
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
            "round" => round_to(s, scalar_arg_opt(c, 1, t, self)?.unwrap_or(1.0)),
            "clamp_min" => s.max(scalar_arg(c, 1, t, self)?),
            "clamp_max" => s.min(scalar_arg(c, 1, t, self)?),
            "clamp" => clamp_val(s, scalar_arg(c, 1, t, self)?, scalar_arg(c, 2, t, self)?),
            "sqrt" | "exp" | "ln" | "log2" | "log10" | "sgn" => unary_math(name, s),
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
    #[allow(clippy::type_complexity)]
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

/// Variante optionnelle de `scalar_arg` : `Ok(None)` si l'argument est absent.
fn scalar_arg_opt(
    c: &Call,
    idx: usize,
    t: i64,
    ev: &Evaluator,
) -> Result<Option<f64>, PromQlError> {
    if c.args.args.get(idx).is_none() {
        return Ok(None);
    }
    scalar_arg(c, idx, t, ev).map(Some)
}

/// Formate une valeur flottante en valeur de label pour `count_values`
/// (représentation compacte, analogue à `strconv.FormatFloat(f, 'f', -1, 64)`
/// côté Prometheus). Les valeurs spéciales sont rendues comme Prometheus.
fn fmt_value_label(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.is_infinite() {
        if v > 0.0 {
            "+Inf".into()
        } else {
            "-Inf".into()
        }
    } else {
        format!("{v}")
    }
}

/// Arrondi PromQL au plus proche multiple de `to_nearest` (`round(v, tn)`),
/// demi-vers-le-haut comme Prometheus (`floor(v/tn + 0.5) * tn`).
fn round_to(v: f64, to_nearest: f64) -> f64 {
    let tn = if to_nearest == 0.0 { 1.0 } else { to_nearest };
    (v / tn + 0.5).floor() * tn
}

/// Régression linéaire par moindres carrés sur `pts`, avec l'origine de l'axe x
/// fixée à `intercept_time_ms` (en ms). Renvoie `(pente_par_seconde, ordonnée
/// à intercept_time)`. Utilisé par `deriv` et `predict_linear`.
fn linear_regression(pts: &[(i64, f64)], intercept_time_ms: i64) -> (f64, f64) {
    // Garde défensive : une régression linéaire exige ≥ 2 points (sinon la
    // pente est indéfinie). Les appelants garantissent déjà ce minimum, mais on
    // évite tout panic (`pts[0]`, `pts.len() - 1`) et on renvoie `NaN` (état
    // indéfini, convention Prometheus) plutôt que 0 qui masquerait l'erreur
    // (revue Gemini PR #528). NB : le cas ≥ 2 points de même timestamp est
    // traité plus bas (pente 0, ordonnée = moyenne).
    if pts.len() < 2 {
        return (f64::NAN, f64::NAN);
    }
    let n = pts.len() as f64;
    let (mut sum_x, mut sum_y, mut sum_xy, mut sum_x2) = (0.0, 0.0, 0.0, 0.0);
    for (ts, v) in pts {
        let x = (*ts - intercept_time_ms) as f64 / 1000.0;
        sum_x += x;
        sum_y += v;
        sum_xy += x * v;
        sum_x2 += x * x;
    }
    let cov_xy = sum_xy - sum_x * sum_y / n;
    let var_x = sum_x2 - sum_x * sum_x / n;
    // Garde anti division par zéro : si tous les points partagent le même
    // timestamp, la variance en x est nulle. On compare directement le premier
    // et le dernier timestamp (pts trié par ts croissant) — exact et O(1),
    // contrairement à `var_x == 0.0` qui est instable en flottant (revue Gemini
    // PR #528). Pente nulle, ordonnée = moyenne de Y (sémantique Prometheus).
    if pts[0].0 == pts[pts.len() - 1].0 {
        return (0.0, sum_y / n);
    }
    let slope = cov_xy / var_x;
    let intercept = sum_y / n - slope * sum_x / n;
    (slope, intercept)
}

/// Variance de population (`Σ(x-µ)²/n`) sur une série de valeurs.
fn variance_pop(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n
}

/// φ-quantile avec interpolation linéaire (sémantique Prometheus). `φ<0` →
/// `-Inf`, `φ>1` → `+Inf`. `values` doit être non vide.
fn quantile_value(phi: f64, mut values: Vec<f64>) -> f64 {
    if phi.is_nan() {
        return f64::NAN;
    }
    if phi < 0.0 {
        return f64::NEG_INFINITY;
    }
    if phi > 1.0 {
        return f64::INFINITY;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    let rank = phi * (n - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let weight = rank - lower as f64;
    values[lower] * (1.0 - weight) + values[upper] * weight
}

/// Nombre de changements de valeur entre points consécutifs (`changes`). Deux
/// `NaN` consécutifs ne comptent pas comme un changement (sémantique Prometheus).
fn changes_count(values: &[f64]) -> f64 {
    values
        .windows(2)
        .filter(|w| w[0] != w[1] && !(w[0].is_nan() && w[1].is_nan()))
        .count() as f64
}

/// Nombre de resets de compteur (`resets`) : transitions où la valeur décroît.
fn resets_count(values: &[f64]) -> f64 {
    values.windows(2).filter(|w| w[1] < w[0]).count() as f64
}

fn apply_range_fn_raw(
    name: &str,
    pts: &[(i64, f64)],
    range_ms: i64,
    eval_t: i64,
    param: Option<f64>,
) -> Option<f64> {
    if pts.is_empty() {
        return match name {
            "count_over_time" => Some(0.0),
            "changes" | "resets" => Some(0.0),
            _ => None,
        };
    }
    // Fonctions nécessitant ≥ 2 points sous la fenêtre. Pour `rate`/`increase`
    // c'est la sémantique Prometheus stricte : un seul point ⇒ *no data* (et
    // non `0`, qui masquait silencieusement les séries clairsemées — cf. P0 de
    // `docs/metriques-promql-reference.md` §4.6/§7). Idem `deriv`/`predict_linear`
    // dont la régression est indéfinie sur un point.
    if matches!(name, "deriv" | "predict_linear" | "rate" | "increase") && pts.len() < 2 {
        return None;
    }
    let vals: Vec<f64> = pts.iter().map(|p| p.1).collect();
    Some(match name {
        "increase" => raw_counter_increase(pts),
        "delta" => pts.last().unwrap().1 - pts.first().unwrap().1,
        "rate" => raw_counter_increase(pts) / (range_ms as f64 / 1000.0),
        "deriv" => linear_regression(pts, pts[0].0).0,
        "predict_linear" => {
            let (slope, intercept) = linear_regression(pts, eval_t);
            slope * param? + intercept
        }
        "changes" => changes_count(&vals),
        "resets" => resets_count(&vals),
        "avg_over_time" => vals.iter().sum::<f64>() / vals.len() as f64,
        "sum_over_time" => vals.iter().sum::<f64>(),
        "min_over_time" => vals.iter().copied().fold(f64::INFINITY, f64::min),
        "max_over_time" => vals.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        "count_over_time" => vals.len() as f64,
        "last_over_time" => pts.last().unwrap().1,
        "stdvar_over_time" => variance_pop(&vals),
        "stddev_over_time" => variance_pop(&vals).sqrt(),
        "quantile_over_time" => quantile_value(param?, vals),
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

/// `irate` (tier raw) : taux instantané sur les **deux derniers points** de la
/// fenêtre : `counter_increase(prev, last) / Δt_secondes`. Gère le reset de
/// compteur via `counter_increase`. Renvoie `None` s'il y a moins de 2 points
/// ou si `Δt == 0`.
fn apply_irate_last_two(lt: Option<((i64, f64), (i64, f64))>) -> Option<f64> {
    let (prev, last) = lt?;
    let dt = (last.0 - prev.0) as f64 / 1000.0;
    if dt <= 0.0 {
        return None;
    }
    Some(counter_increase(prev.1, last.1) / dt)
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
    eval_t: i64,
    param: Option<f64>,
) -> Option<f64> {
    if buckets.is_empty() {
        return match name {
            "count_over_time" => Some(0.0),
            "changes" | "resets" => Some(0.0),
            _ => None,
        };
    }
    // Fonctions statistiques / régression sur tier compacté : les points raw
    // sont purgés. Approximation documentée — on opère sur les valeurs
    // représentatives des buckets (`avg` pour régression/stats ; séquence
    // `first,last` pour changes/resets). Précis sur tier raw, approché ici.
    match name {
        "deriv" | "predict_linear" => {
            if buckets.len() < 2 {
                return None;
            }
            let pts: Vec<(i64, f64)> = buckets.iter().map(|(ts, b)| (*ts, b.avg)).collect();
            return Some(if name == "deriv" {
                linear_regression(&pts, pts[0].0).0
            } else {
                let (slope, intercept) = linear_regression(&pts, eval_t);
                slope * param? + intercept
            });
        }
        "stdvar_over_time" | "stddev_over_time" | "quantile_over_time" => {
            let vals: Vec<f64> = buckets.iter().map(|(_, b)| b.avg).collect();
            return Some(match name {
                "stdvar_over_time" => variance_pop(&vals),
                "stddev_over_time" => variance_pop(&vals).sqrt(),
                _ => quantile_value(param?, vals),
            });
        }
        "changes" | "resets" => {
            // Séquence first→last de chaque bucket (capture le net intra-bucket
            // et les transitions inter-buckets ; les oscillations internes sont
            // invisibles).
            let mut seq = Vec::with_capacity(buckets.len() * 2);
            for (_, b) in buckets {
                seq.push(b.first);
                seq.push(b.last);
            }
            return Some(if name == "changes" {
                changes_count(&seq)
            } else {
                resets_count(&seq)
            });
        }
        _ => {}
    }
    // `irate` sur tier compacté : mal défini (points raw purgés). Choix retenu
    // (cf. roadmap §3c) : approximer avec les `last` des deux derniers buckets,
    // sur leur écart de timestamps de bucket. Requiert ≥ 2 buckets.
    if name == "irate" {
        if buckets.len() < 2 {
            return None;
        }
        let prev = &buckets[buckets.len() - 2];
        let last = &buckets[buckets.len() - 1];
        let dt = (last.0 - prev.0) as f64 / 1000.0;
        if dt <= 0.0 {
            return None;
        }
        return Some(counter_increase(prev.1.last, last.1.last) / dt);
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

/// Fonction scalaire associée à un opérateur arithmétique PromQL.
fn arith_fn(op: &str) -> Result<fn(f64, f64) -> f64, PromQlError> {
    Ok(match op {
        "+" => |a, b| a + b,
        "-" => |a, b| a - b,
        "*" => |a, b| a * b,
        "/" => |a, b| a / b,
        // `%` : modulo flottant (sémantique Go `math.Mod`, fournie par `f64::rem`).
        "%" => |a, b| a % b,
        // `^` : puissance.
        "^" => |a: f64, b: f64| a.powf(b),
        other => return Err(PromQlError::Unsupported(format!("binary {other}"))),
    })
}

/// **Signature de matching** d'un sample : sous-ensemble de labels servant à
/// apparier deux vecteurs selon le modificateur `on`/`ignoring`.
/// - `on(l…)` (`Include`) : ne garder que les labels listés.
/// - `ignoring(l…)` (`Exclude`) : garder tous les labels sauf ceux listés (et
///   `__name__`).
/// - aucun modificateur : tous les labels sauf `__name__`.
fn matching_sig(labels: &Labels, matching: Option<&LabelModifier>) -> Labels {
    let mut key = Labels::new();
    match matching {
        Some(LabelModifier::Include(ls)) => {
            for l in &ls.labels {
                if let Some(v) = labels.get(l) {
                    key.insert(l.clone(), v.clone());
                }
            }
        }
        Some(LabelModifier::Exclude(ls)) => {
            for (k, v) in labels.iter() {
                if k == "__name__" || ls.labels.contains(k) {
                    continue;
                }
                key.insert(k.clone(), v.clone());
            }
        }
        None => {
            for (k, v) in labels.iter() {
                if k == "__name__" {
                    continue;
                }
                key.insert(k.clone(), v.clone());
            }
        }
    }
    key
}

/// Construit les labels du résultat d'une opération vec×vec (sémantique
/// Prometheus `resultMetric`).
///
/// - `OneToOne` : on part des labels du côté `many` (= lhs), on droppe
///   `__name__` (si `drop_name`), puis selon le matching : `on` ⇒ ne garder que
///   les labels d'appariement, `ignoring` ⇒ retirer les labels listés.
/// - `group_left`/`group_right` : on garde **tous** les labels du côté `many`
///   (sauf `__name__`), puis on recopie les labels `include` depuis le côté
///   `one`.
fn result_metric(
    many: &Labels,
    one: &Labels,
    card: &VectorMatchCardinality,
    matching: Option<&LabelModifier>,
    drop_name: bool,
) -> Labels {
    let mut lb = many.clone();
    if drop_name {
        lb.remove("__name__");
    }
    match card {
        VectorMatchCardinality::OneToOne | VectorMatchCardinality::ManyToMany => {
            match matching {
                Some(LabelModifier::Include(ls)) => {
                    lb.retain(|k, _| ls.labels.contains(k));
                }
                Some(LabelModifier::Exclude(ls)) => {
                    for l in &ls.labels {
                        lb.remove(l);
                    }
                }
                None => {}
            }
        }
        VectorMatchCardinality::ManyToOne(include) | VectorMatchCardinality::OneToMany(include) => {
            for l in &include.labels {
                match one.get(l) {
                    Some(v) => {
                        lb.insert(l.clone(), v.clone());
                    }
                    None => {
                        lb.remove(l);
                    }
                }
            }
        }
    }
    lb
}

/// Opération binaire vec×vec avec matching vectoriel et cardinalité.
///
/// `combine(l, r)` calcule la valeur de sortie (orientation préservée :
/// `l` = lhs, `r` = rhs) ; `None` filtre la paire (mode comparaison filtre).
/// `drop_name` indique si `__name__` doit être retiré du résultat.
fn vector_binary_op(
    lhs: Vec<InstantSample>,
    rhs: Vec<InstantSample>,
    modifier: Option<&BinModifier>,
    drop_name: bool,
    combine: &dyn Fn(f64, f64) -> Option<f64>,
) -> Result<Vec<InstantSample>, PromQlError> {
    let matching = modifier.and_then(|m| m.matching.as_ref());
    let default_card = VectorMatchCardinality::OneToOne;
    let card = modifier.map(|m| &m.card).unwrap_or(&default_card);

    // `group_right` : le côté « many » est le rhs → on indexe le lhs (« one »).
    if matches!(card, VectorMatchCardinality::OneToMany(_)) {
        let mut one_idx: HashMap<Labels, (f64, Arc<Labels>)> = HashMap::with_capacity(lhs.len());
        for s in lhs {
            let key = matching_sig(&s.labels, matching);
            if one_idx.insert(key, (s.value, s.labels)).is_some() {
                return Err(PromQlError::Execution(
                    "many-to-many matching non autorisé : plusieurs séries du côté \
                     'one' (group_right) partagent les mêmes labels d'appariement"
                        .into(),
                ));
            }
        }
        let mut out = Vec::with_capacity(rhs.len());
        for s in rhs {
            let key = matching_sig(&s.labels, matching);
            if let Some((lval, llabels)) = one_idx.get(&key) {
                if let Some(v) = combine(*lval, s.value) {
                    let labels = result_metric(&s.labels, llabels, card, matching, drop_name);
                    out.push(InstantSample { labels: Arc::new(labels), value: v });
                }
            }
        }
        return Ok(out);
    }

    // `OneToOne` / `group_left` : le côté « one » est le rhs → on l'indexe.
    let mut one_idx: HashMap<Labels, (f64, Arc<Labels>)> = HashMap::with_capacity(rhs.len());
    for s in rhs {
        let key = matching_sig(&s.labels, matching);
        if one_idx.insert(key, (s.value, s.labels)).is_some() {
            return Err(PromQlError::Execution(
                "matching vectoriel : plusieurs séries du côté droit partagent les \
                 mêmes labels d'appariement (préciser on()/ignoring() ou group_left)"
                    .into(),
            ));
        }
    }
    // En `OneToOne`, l'appariement doit aussi être unique côté gauche : si deux
    // séries du lhs partagent la même signature, c'est une relation many-to-one
    // qui doit être explicitée par `group_left` (sinon erreur Prometheus). En
    // `group_left` (`ManyToOne`), les doublons côté gauche sont au contraire
    // attendus, donc cette garde ne s'applique pas.
    let check_lhs_unique = matches!(card, VectorMatchCardinality::OneToOne);
    let mut seen_lhs: HashSet<Labels> = HashSet::new();
    let mut out = Vec::with_capacity(lhs.len());
    for s in lhs {
        let key = matching_sig(&s.labels, matching);
        if let Some((rval, rlabels)) = one_idx.get(&key) {
            if check_lhs_unique && !seen_lhs.insert(key.clone()) {
                return Err(PromQlError::Execution(
                    "matching vectoriel : plusieurs séries du côté gauche partagent les \
                     mêmes labels d'appariement (préciser group_left/group_right)"
                        .into(),
                ));
            }
            if let Some(v) = combine(s.value, *rval) {
                let labels = result_metric(&s.labels, rlabels, card, matching, drop_name);
                out.push(InstantSample { labels: Arc::new(labels), value: v });
            }
        }
    }
    Ok(out)
}

/// Opérateurs ensemblistes `and` / `or` / `unless` (vec×vec uniquement).
/// L'appariement utilise `matching_sig` (avec `on`/`ignoring` éventuel). Les
/// labels (dont `__name__`) sont **conservés** (pas de drop, pas d'arithmétique).
fn eval_set_op(
    op: &str,
    lhs: Value,
    rhs: Value,
    modifier: Option<&BinModifier>,
) -> Result<Value, PromQlError> {
    let (lhs, rhs) = match (lhs, rhs) {
        (Value::Vector(l), Value::Vector(r)) => (l, r),
        _ => {
            return Err(PromQlError::Execution(format!(
                "l'opérateur ensembliste '{op}' requiert deux vecteurs instantanés"
            )))
        }
    };
    let matching = modifier.and_then(|m| m.matching.as_ref());
    let out = match op {
        "and" => {
            let rhs_keys: HashSet<Labels> =
                rhs.iter().map(|s| matching_sig(&s.labels, matching)).collect();
            lhs.into_iter()
                .filter(|s| rhs_keys.contains(&matching_sig(&s.labels, matching)))
                .collect()
        }
        "unless" => {
            let rhs_keys: HashSet<Labels> =
                rhs.iter().map(|s| matching_sig(&s.labels, matching)).collect();
            lhs.into_iter()
                .filter(|s| !rhs_keys.contains(&matching_sig(&s.labels, matching)))
                .collect()
        }
        "or" => {
            let lhs_keys: HashSet<Labels> =
                lhs.iter().map(|s| matching_sig(&s.labels, matching)).collect();
            let mut out = lhs;
            for s in rhs {
                if !lhs_keys.contains(&matching_sig(&s.labels, matching)) {
                    out.push(s);
                }
            }
            out
        }
        other => return Err(PromQlError::Unsupported(format!("set operator {other}"))),
    };
    Ok(Value::Vector(out))
}

/// Fonctions mathématiques unaires (`sqrt exp ln log2 log10 sgn`). `NaN` se
/// propage naturellement (sauf `sgn` qui le renvoie explicitement).
fn unary_math(name: &str, v: f64) -> f64 {
    match name {
        "sqrt" => v.sqrt(),
        "exp" => v.exp(),
        "ln" => v.ln(),
        "log2" => v.log2(),
        "log10" => v.log10(),
        // `sgn` : 1 si >0, -1 si <0, 0 si ==0, NaN si NaN (sémantique Prometheus,
        // différente de `f64::signum` qui renvoie ±1 pour 0).
        "sgn" => {
            if v.is_nan() {
                f64::NAN
            } else if v > 0.0 {
                1.0
            } else if v < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        _ => f64::NAN,
    }
}

/// `clamp(v, min, max)` : borne `v` dans `[min, max]`. Renvoie `NaN` si
/// `min > max` ou si l'un des arguments est `NaN` (sémantique Prometheus).
fn clamp_val(v: f64, min: f64, max: f64) -> f64 {
    // `f64::max`/`f64::min` ignorent `NaN` (contrairement à `math.Max`/`math.Min`
    // de Go utilisés par Prometheus, qui propagent `NaN`). On propage donc
    // explicitement : sinon `clamp(NaN, min, max)` renverrait `min`.
    if v.is_nan() || min.is_nan() || max.is_nan() || min > max {
        f64::NAN
    } else {
        v.max(min).min(max)
    }
}

/// Extrait l'argument string littéral d'indice `idx` d'un appel de fonction.
/// La validation (`validate_call`) garantit déjà que ces arguments sont des
/// `StringLiteral` pour `label_replace`/`label_join`.
fn string_arg(c: &Call, idx: usize) -> Result<String, PromQlError> {
    match c.args.args.get(idx).map(|a| a.as_ref()) {
        Some(Expr::StringLiteral(StringLiteral { val })) => Ok(val.clone()),
        _ => Err(PromQlError::Execution(format!(
            "argument string #{idx} attendu"
        ))),
    }
}

/// `label_replace(v, dst, replacement, src, regex)` : si `regex` (ancrée sur la
/// chaîne entière) matche la valeur du label `src`, le label `dst` reçoit
/// `replacement` (avec expansion des groupes `$1`/`${name}`). Si la valeur
/// résultante est vide, `dst` est retiré. Sinon le sample est inchangé.
fn label_replace(inner: Vec<InstantSample>, c: &Call) -> Result<Value, PromQlError> {
    let dst = string_arg(c, 1)?;
    let repl = string_arg(c, 2)?;
    let src = string_arg(c, 3)?;
    let regex_src = string_arg(c, 4)?;
    // Prometheus ancre le regex sur toute la valeur du label (full match).
    let re = Regex::new(&format!("^(?:{regex_src})$"))
        .map_err(|e| PromQlError::Execution(format!("label_replace: regex invalide: {e}")))?;
    let mut out = Vec::with_capacity(inner.len());
    for s in inner {
        let src_val = s.labels.get(&src).map(String::as_str).unwrap_or("");
        if let Some(caps) = re.captures(src_val) {
            let mut expanded = String::new();
            caps.expand(&repl, &mut expanded);
            // Évite de cloner le BTreeMap + allouer un Arc quand le label est
            // déjà correct (ou déjà absent pour un remplacement vide).
            let needs_update = if expanded.is_empty() {
                s.labels.contains_key(&dst)
            } else {
                s.labels.get(&dst).map(String::as_str) != Some(&expanded)
            };
            if needs_update {
                let mut labels = (*s.labels).clone();
                if expanded.is_empty() {
                    labels.remove(&dst);
                } else {
                    labels.insert(dst.clone(), expanded);
                }
                out.push(InstantSample { labels: Arc::new(labels), value: s.value });
            } else {
                out.push(s);
            }
        } else {
            out.push(s);
        }
    }
    Ok(Value::Vector(out))
}

/// `label_join(v, dst, separator, src_1, …, src_n)` : concatène les valeurs des
/// labels sources avec `separator` dans le label `dst`. Valeur vide → `dst`
/// retiré.
fn label_join(inner: Vec<InstantSample>, c: &Call) -> Result<Value, PromQlError> {
    let dst = string_arg(c, 1)?;
    let sep = string_arg(c, 2)?;
    let srcs: Vec<String> = (3..c.args.args.len())
        .map(|i| string_arg(c, i))
        .collect::<Result<_, _>>()?;
    let mut out = Vec::with_capacity(inner.len());
    for s in inner {
        let joined = srcs
            .iter()
            .map(|l| s.labels.get(l).map(String::as_str).unwrap_or(""))
            .collect::<Vec<_>>()
            .join(&sep);
        // Même optimisation que label_replace : pas de clone si rien ne change.
        let needs_update = if joined.is_empty() {
            s.labels.contains_key(&dst)
        } else {
            s.labels.get(&dst).map(String::as_str) != Some(&joined)
        };
        if needs_update {
            let mut labels = (*s.labels).clone();
            if joined.is_empty() {
                labels.remove(&dst);
            } else {
                labels.insert(dst.clone(), joined);
            }
            out.push(InstantSample { labels: Arc::new(labels), value: s.value });
        } else {
            out.push(s);
        }
    }
    Ok(Value::Vector(out))
}

/// Renvoie la fonction de comparaison associée à un opérateur, ou `None` si
/// l'opérateur n'est pas une comparaison.
fn cmp_fn(op: &str) -> Option<fn(f64, f64) -> bool> {
    Some(match op {
        "==" => |a, b| a == b,
        "!=" => |a, b| a != b,
        ">" => |a, b| a > b,
        "<" => |a, b| a < b,
        ">=" => |a, b| a >= b,
        "<=" => |a, b| a <= b,
        _ => return None,
    })
}

/// Applique une comparaison en traitant tout `NaN` comme « condition fausse »
/// (y compris pour `!=`, où IEEE renverrait `true`). Conforme à la sémantique
/// PromQL où une comparaison impliquant `NaN` est toujours fausse.
fn cmp_truth(cmp: fn(f64, f64) -> bool, a: f64, b: f64) -> bool {
    if a.is_nan() || b.is_nan() {
        false
    } else {
        cmp(a, b)
    }
}

/// Retire `__name__` des labels d'un sample sans cloner inutilement quand il
/// est déjà absent.
fn drop_name(labels: &Arc<Labels>) -> Arc<Labels> {
    if labels.contains_key("__name__") {
        let mut l = (**labels).clone();
        l.remove("__name__");
        Arc::new(l)
    } else {
        labels.clone()
    }
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

    #[test]
    fn irate_last_two_local_slope() {
        // 2 points à 2 s d'écart : (10 → 30) / 2 s = 10/s.
        let lt = Some(((0_i64, 10.0), (2_000_i64, 30.0)));
        assert_eq!(apply_irate_last_two(lt), Some(10.0));
    }

    #[test]
    fn irate_last_two_handles_reset() {
        // Reset : last < prev → counter_increase renvoie last (5) ; /1 s = 5.
        let lt = Some(((0_i64, 20.0), (1_000_i64, 5.0)));
        assert_eq!(apply_irate_last_two(lt), Some(5.0));
    }

    #[test]
    fn irate_last_two_edge_cases() {
        // Moins de 2 points → None.
        assert_eq!(apply_irate_last_two(None), None);
        // Δt == 0 → None (évite division par zéro).
        let lt = Some(((1_000_i64, 1.0), (1_000_i64, 2.0)));
        assert_eq!(apply_irate_last_two(lt), None);
    }

    #[test]
    fn clamp_val_borne_et_propage_nan() {
        assert_eq!(clamp_val(5.0, 0.0, 10.0), 5.0);
        assert_eq!(clamp_val(-3.0, 0.0, 10.0), 0.0);
        assert_eq!(clamp_val(42.0, 0.0, 10.0), 10.0);
        // min > max → NaN.
        assert!(clamp_val(5.0, 10.0, 1.0).is_nan());
        // v ou bornes NaN → NaN (et NON `min`, cf. revue Gemini PR #528).
        assert!(clamp_val(f64::NAN, 0.0, 10.0).is_nan());
        assert!(clamp_val(5.0, f64::NAN, 10.0).is_nan());
        assert!(clamp_val(5.0, 0.0, f64::NAN).is_nan());
    }

    #[test]
    fn unary_math_sgn_semantics() {
        assert_eq!(unary_math("sgn", 3.0), 1.0);
        assert_eq!(unary_math("sgn", -3.0), -1.0);
        assert_eq!(unary_math("sgn", 0.0), 0.0);
        assert_eq!(unary_math("sgn", -0.0), 0.0);
        assert!(unary_math("sgn", f64::NAN).is_nan());
        assert_eq!(unary_math("sqrt", 9.0), 3.0);
    }

    #[test]
    fn round_to_honors_to_nearest() {
        assert!((round_to(1.27, 0.1) - 1.3).abs() < 1e-9);
        assert!((round_to(1.24, 0.1) - 1.2).abs() < 1e-9);
        assert_eq!(round_to(2.5, 1.0), 3.0); // demi-vers-le-haut
        assert_eq!(round_to(7.0, 5.0), 5.0);
        assert_eq!(round_to(8.0, 5.0), 10.0);
        assert_eq!(round_to(42.0, 0.0), 42.0); // to_nearest=0 → défaut 1
    }

    #[test]
    fn linear_regression_pente_et_ordonnee() {
        // y = 10 + 2*(t en s) à partir de t=0, pas de 1 s.
        let pts: Vec<(i64, f64)> = (0..5).map(|i| (i * 1000, 10.0 + 2.0 * i as f64)).collect();
        let (slope, intercept) = linear_regression(&pts, 0);
        assert!((slope - 2.0).abs() < 1e-9);
        assert!((intercept - 10.0).abs() < 1e-9);
        // deriv = pente quelle que soit l'origine.
        let (slope2, _) = linear_regression(&pts, pts[0].0);
        assert!((slope2 - 2.0).abs() < 1e-9);
    }

    #[test]
    fn linear_regression_var_x_zero_guard() {
        // Tous les points au même timestamp → var_x == 0 : pente 0, ordonnée
        // = moyenne de Y (pas de NaN/Inf).
        let pts = vec![(1_000_i64, 10.0), (1_000_i64, 20.0), (1_000_i64, 30.0)];
        let (slope, intercept) = linear_regression(&pts, 0);
        assert_eq!(slope, 0.0);
        assert!((intercept - 20.0).abs() < 1e-9);
        // < 2 points → (NaN, NaN) (régression indéfinie, pas de panic).
        let (s, i) = linear_regression(&[], 0);
        assert!(s.is_nan() && i.is_nan());
        let (s1, i1) = linear_regression(&[(1_000, 42.0)], 0);
        assert!(s1.is_nan() && i1.is_nan());
    }

    #[test]
    fn quantile_value_interpolation() {
        let v = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(quantile_value(0.0, v.clone()), 1.0);
        assert_eq!(quantile_value(1.0, v.clone()), 4.0);
        assert_eq!(quantile_value(0.5, v.clone()), 2.5);
        assert!(quantile_value(-0.1, v.clone()) == f64::NEG_INFINITY);
        assert!(quantile_value(1.1, v) == f64::INFINITY);
    }

    #[test]
    fn variance_changes_resets() {
        assert!((variance_pop(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]) - 4.0).abs() < 1e-9);
        assert_eq!(changes_count(&[1.0, 1.0, 2.0, 2.0, 3.0]), 2.0);
        assert_eq!(changes_count(&[f64::NAN, f64::NAN]), 0.0); // 2 NaN consécutifs
        assert_eq!(resets_count(&[0.0, 5.0, 10.0, 2.0, 8.0]), 1.0); // un seul reset (10→2)
    }
}
