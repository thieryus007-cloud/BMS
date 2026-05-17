//! Agrégats Rust qui remplacent `AVG/MIN/MAX/SUM` SQL. Plan §5.5.

use anyhow::Result;

use crate::reader::Reader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggOp {
    Avg,
    Min,
    Max,
    Sum,
    Count,
    First,
    Last,
}

/// Agrège tous les points raw d'une série sur `[from_ms, to_ms]`. Retourne
/// `None` si la série n'a aucun point dans la plage (`AggOp::Count` retourne
/// alors `Some(0.0)`).
pub fn agg_over_range(
    reader: &Reader,
    series_id: u32,
    from_ms: i64,
    to_ms: i64,
    op: AggOp,
) -> Result<Option<f64>> {
    let pts = reader.query_range_raw(series_id, from_ms, to_ms)?;
    Ok(reduce(&pts, op))
}

pub fn reduce(pts: &[(i64, f64)], op: AggOp) -> Option<f64> {
    if matches!(op, AggOp::Count) {
        return Some(pts.len() as f64);
    }
    if pts.is_empty() {
        return None;
    }
    let v = match op {
        AggOp::Avg => pts.iter().map(|p| p.1).sum::<f64>() / pts.len() as f64,
        AggOp::Min => pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min),
        AggOp::Max => pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max),
        AggOp::Sum => pts.iter().map(|p| p.1).sum(),
        AggOp::First => pts.first().map(|p| p.1).unwrap(),
        AggOp::Last => pts.last().map(|p| p.1).unwrap(),
        AggOp::Count => unreachable!(),
    };
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce_basic() {
        let pts = vec![(0, 1.0), (1, 2.0), (2, 3.0), (3, 4.0)];
        assert_eq!(reduce(&pts, AggOp::Avg), Some(2.5));
        assert_eq!(reduce(&pts, AggOp::Min), Some(1.0));
        assert_eq!(reduce(&pts, AggOp::Max), Some(4.0));
        assert_eq!(reduce(&pts, AggOp::Sum), Some(10.0));
        assert_eq!(reduce(&pts, AggOp::Count), Some(4.0));
        assert_eq!(reduce(&pts, AggOp::First), Some(1.0));
        assert_eq!(reduce(&pts, AggOp::Last), Some(4.0));
    }

    #[test]
    fn reduce_empty() {
        assert_eq!(reduce(&[], AggOp::Avg), None);
        assert_eq!(reduce(&[], AggOp::Count), Some(0.0));
    }
}
