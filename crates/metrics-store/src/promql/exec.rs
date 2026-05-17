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
//! Pour `increase` sur tier compacté, on utilise `last_bucket.last -
//! first_bucket.first` (correct pour les compteurs monotones — c'est
//! le cas de `et112_energy_*` qui sont nos seuls usages réels).

use std::collections::BTreeMap;
use std::time::Duration;

use promql_parser::label::{MatchOp, Matcher};
use promql_parser::parser::{
    AggregateExpr, BinaryExpr, Call, Expr, MatrixSelector, NumberLiteral, ParenExpr, UnaryExpr,
    VectorSelector,
};

use crate::reader::Reader;
use crate::tables::{AggBucket, Tier};

use super::error::PromQlError;

pub type Labels = BTreeMap<String, String>;

#[derive(Debug, Clone)]
pub struct InstantSample {
    pub labels: Labels,
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
}

impl<'r> Evaluator<'r> {
    pub fn new(reader: &'r Reader) -> Self {
        Self { reader, lookback_ms: 5 * 60_000 }
    }

    pub fn with_lookback(reader: &'r Reader, lookback: Duration) -> Self {
        Self { reader, lookback_ms: lookback.as_millis() as i64 }
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
        let mut by_labels: BTreeMap<Labels, Vec<(i64, f64)>> = BTreeMap::new();
        let mut t = start_ms;
        while t <= end_ms {
            match self.eval_at(expr, t)? {
                Value::Scalar(v) => {
                    by_labels
                        .entry(Labels::new())
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
        Ok(by_labels
            .into_iter()
            .map(|(labels, samples)| RangeSeries { labels, samples })
            .collect())
    }

    /// Évalue `expr` à un instant unique (équivalent `/api/v1/query`).
    pub fn eval_instant(&self, expr: &Expr, t_ms: i64) -> Result<Vec<InstantSample>, PromQlError> {
        match self.eval_at(expr, t_ms)? {
            Value::Scalar(v) => Ok(vec![InstantSample { labels: Labels::new(), value: v }]),
            Value::Vector(s) => Ok(s),
        }
    }

    // ── interne ──────────────────────────────────────────────────────────

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
        let metric = vs
            .name
            .as_deref()
            .ok_or_else(|| PromQlError::Execution("selector sans nom de métrique".into()))?;
        let matched = self.match_series(metric, &vs.matchers.matchers)?;
        let mut out = Vec::with_capacity(matched.len());
        for (sid, labels) in matched {
            let pts = self
                .reader
                .query_range_raw(sid, t - self.lookback_ms, t)
                .map_err(|e| PromQlError::Execution(e.to_string()))?;
            if let Some((_, v)) = pts.last() {
                out.push(InstantSample { labels, value: *v });
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
        Ok(Value::Vector(vec![InstantSample { labels: Labels::new(), value }]))
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
        let metric = vs
            .name
            .as_deref()
            .ok_or_else(|| PromQlError::Execution("matrix selector sans métrique".into()))?;
        let matched = self.match_series(metric, &vs.matchers.matchers)?;
        let mut out = Vec::with_capacity(matched.len());
        for (sid, labels) in matched {
            let value_opt = match tier {
                Tier::Raw => {
                    let pts = self
                        .reader
                        .query_range_raw(sid, win_start, t)
                        .map_err(|e| PromQlError::Execution(e.to_string()))?;
                    apply_range_fn_raw(name, &pts, range_ms)
                }
                Tier::Hourly | Tier::Daily => {
                    let buckets = self
                        .reader
                        .query_range_buckets(sid, win_start, t, tier)
                        .map_err(|e| PromQlError::Execution(e.to_string()))?;
                    apply_range_fn_buckets(name, &buckets, range_ms)
                }
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

    fn match_series(
        &self,
        metric: &str,
        matchers: &[Matcher],
    ) -> Result<Vec<(u32, Labels)>, PromQlError> {
        let all = self
            .reader
            .list_series()
            .map_err(|e| PromQlError::Execution(e.to_string()))?;
        let mut out = Vec::new();
        for (sid, meta) in all {
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
                let mut with_name = labels.clone();
                with_name.insert("__name__".into(), metric.into());
                out.push((sid, with_name));
            }
        }
        Ok(out)
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
        "increase" | "delta" => pts.last().unwrap().1 - pts.first().unwrap().1,
        "rate" => (pts.last().unwrap().1 - pts.first().unwrap().1) / (range_ms as f64 / 1000.0),
        "avg_over_time" => pts.iter().map(|p| p.1).sum::<f64>() / pts.len() as f64,
        "sum_over_time" => pts.iter().map(|p| p.1).sum::<f64>(),
        "min_over_time" => pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min),
        "max_over_time" => pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max),
        "count_over_time" => pts.len() as f64,
        "last_over_time" => pts.last().unwrap().1,
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
        "increase" | "delta" => buckets.last().unwrap().1.last - buckets.first().unwrap().1.first,
        "rate" => {
            (buckets.last().unwrap().1.last - buckets.first().unwrap().1.first)
                / (range_ms as f64 / 1000.0)
        }
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
            let mut l = s.labels;
            l.remove("__name__");
            (l, s.value)
        })
        .collect();
    let mut out = Vec::new();
    for s in lhs {
        let mut key = s.labels;
        key.remove("__name__");
        if let Some(&rval) = rhs_idx.get(&key) {
            out.push(InstantSample { labels: key, value: op(s.value, rval) });
        }
    }
    out
}
