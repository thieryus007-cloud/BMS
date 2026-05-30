//! Validation : walk de l'AST. Rejette tout ce qui n'est pas dans le
//! sous-ensemble PromQL audité §6.5.

use promql_parser::parser::{
    AggregateExpr, BinaryExpr, Call, Expr, MatrixSelector, ParenExpr, SubqueryExpr, UnaryExpr,
    VectorMatchCardinality, VectorSelector,
};

use super::error::PromQlError;

/// Fonctions à fenêtre temporelle (`f(m[range])`).
pub const SUPPORTED_RANGE_FUNCS: &[&str] = &[
    "increase",
    "rate",
    "delta",
    "avg_over_time",
    "sum_over_time",
    "min_over_time",
    "max_over_time",
    "count_over_time",
    "last_over_time",
];

/// Fonctions instantanées (`f(vec)` ou `f(vec, scalar)`).
pub const SUPPORTED_INSTANT_FUNCS: &[&str] =
    &["abs", "clamp_min", "clamp_max", "ceil", "floor", "round"];

/// Opérateurs d'agrégation instant supportés (sans paramètre).
pub const SUPPORTED_AGGREGATORS: &[&str] = &["sum", "max", "min", "avg", "count"];

/// Agrégateurs paramétrés supportés (`op(k, vec)`).
pub const PARAMETERIZED_AGGREGATORS: &[&str] = &["topk", "bottomk"];

/// Opérateurs binaires arithmétiques supportés (vec×scalar et vec×vec
/// aligné — §6.5).
pub const SUPPORTED_BINOPS: &[&str] = &["+", "-", "*", "/"];

/// Opérateurs de comparaison supportés (filtre, ou 0/1 avec `bool`).
pub const SUPPORTED_CMP_OPS: &[&str] = &["==", "!=", ">", "<", ">=", "<="];

pub fn validate(expr: &Expr) -> Result<(), PromQlError> {
    match expr {
        Expr::NumberLiteral(_) => Ok(()),
        Expr::StringLiteral(_) => unsupported("string literal"),
        Expr::VectorSelector(vs) => validate_vector_selector(vs),
        Expr::MatrixSelector(MatrixSelector { vs, .. }) => validate_vector_selector(vs),
        Expr::Paren(ParenExpr { expr }) => validate(expr),
        Expr::Unary(UnaryExpr { expr }) => validate(expr),
        Expr::Subquery(SubqueryExpr { .. }) => unsupported(
            "subquery (e.g. [Xh:Ym]) — réécrire la requête en deux \
             expressions distinctes côté client (cf. plan §6.5)",
        ),
        Expr::Extension(_) => unsupported("extension expression"),
        Expr::Aggregate(a) => validate_aggregate(a),
        Expr::Binary(b) => validate_binary(b),
        Expr::Call(c) => validate_call(c),
    }
}

fn validate_vector_selector(vs: &VectorSelector) -> Result<(), PromQlError> {
    if vs.offset.is_some() {
        return unsupported("offset modifier");
    }
    if vs.at.is_some() {
        return unsupported("@ modifier");
    }
    Ok(())
}

fn validate_aggregate(a: &AggregateExpr) -> Result<(), PromQlError> {
    let op_str = a.op.to_string();
    let is_plain = SUPPORTED_AGGREGATORS.contains(&op_str.as_str());
    let is_param = PARAMETERIZED_AGGREGATORS.contains(&op_str.as_str());
    if !is_plain && !is_param {
        return unsupported(&format!("aggregator: {op_str}"));
    }
    // `topk`/`bottomk` exigent un paramètre `k` ; les autres (`quantile`, …)
    // ne sont pas supportés et les agrégateurs simples n'en acceptent pas.
    if is_param && a.param.is_none() {
        return unsupported(&format!("aggregator {op_str} requires a parameter"));
    }
    if is_plain && a.param.is_some() {
        return unsupported(&format!("parameterized aggregator: {op_str}"));
    }
    if let Some(p) = &a.param {
        validate(p)?;
    }
    // Le groupement `by`/`without` est supporté par l'évaluateur (Phase 2).
    validate(&a.expr)
}

fn validate_binary(b: &BinaryExpr) -> Result<(), PromQlError> {
    let op_str = b.op.to_string();
    let is_arith = SUPPORTED_BINOPS.contains(&op_str.as_str());
    let is_cmp = SUPPORTED_CMP_OPS.contains(&op_str.as_str());
    if !is_arith && !is_cmp {
        return unsupported(&format!("binary operator: {op_str}"));
    }
    if let Some(m) = &b.modifier {
        // `bool` n'est valide que sur une comparaison (filtre → 0/1). Sur un
        // opérateur arithmétique il reste rejeté.
        if m.return_bool && !is_cmp {
            return unsupported("bool modifier");
        }
        // L'évaluateur n'implémente que l'alignement exact tous-labels
        // (`OneToOne` sans `on`/`ignoring`). Un matching non trivial
        // (`on(...)`, `ignoring(...)`, `group_left`, `group_right`) serait
        // silencieusement ignoré → on le rejette.
        if m.matching.is_some() || !matches!(m.card, VectorMatchCardinality::OneToOne) {
            return unsupported(
                "vector matching (on/ignoring/group_left/group_right) — non supporté",
            );
        }
    }
    validate(&b.lhs)?;
    validate(&b.rhs)?;
    Ok(())
}

fn validate_call(c: &Call) -> Result<(), PromQlError> {
    let name = c.func.name;
    let is_range = SUPPORTED_RANGE_FUNCS.contains(&name);
    let is_instant = SUPPORTED_INSTANT_FUNCS.contains(&name);
    if !is_range && !is_instant {
        return unsupported(&format!("function: {name}"));
    }
    for arg in &c.args.args {
        validate(arg)?;
    }
    Ok(())
}

fn unsupported(msg: &str) -> Result<(), PromQlError> {
    Err(PromQlError::Unsupported(msg.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::promql::parse_and_validate;

    fn ok(s: &str) {
        if let Err(e) = parse_and_validate(s) {
            panic!("attendu OK pour {s:?}, erreur: {e}");
        }
    }

    fn ko(s: &str, contains: &str) {
        match parse_and_validate(s) {
            Err(PromQlError::Unsupported(m)) => assert!(
                m.contains(contains),
                "message {m:?} ne contient pas {contains:?}"
            ),
            other => panic!("attendu Unsupported({contains:?}), got {other:?}"),
        }
    }

    #[test]
    fn accepts_simple_selector() {
        ok(r#"bms_v"#);
        ok(r#"bms_v{bms_id="0x01"}"#);
        ok(r#"bms_v{bms_id!="0x02"}"#);
        ok(r#"bms_v{bms_id=~"0x.*"}"#);
    }

    #[test]
    fn accepts_increase_and_division() {
        ok(r#"increase(et112_energy_export_wh{address="0x09"}[24h]) / 1000"#);
    }

    #[test]
    fn accepts_complex_binary() {
        ok(r#"(increase(a[24h]) - increase(b[24h])) / increase(a[24h]) * 100"#);
    }

    #[test]
    fn accepts_aggregations() {
        ok("max(bms_cell_delta_mv)");
        ok("sum(et112_power_w)");
        ok("avg(bms_v)");
    }

    #[test]
    fn rejects_subquery() {
        ko(
            r#"avg_over_time(clamp_min(venus_shunt_current_a,0)[24h:1m])"#,
            "subquery",
        );
    }

    #[test]
    fn rejects_unsupported_function() {
        ko("histogram_quantile(0.95, foo)", "function: histogram_quantile");
        ko("label_replace(foo, \"a\", \"b\", \"c\", \"d\")", "function: label_replace");
    }

    #[test]
    fn accepts_comparison_operators() {
        ok("foo > 5");
        ok("foo >= 5");
        ok("foo < 5");
        ok("foo <= 5");
        ok("foo == 5");
        ok("foo != 5");
        ok("foo > bool 5"); // `bool` se place après l'opérateur en PromQL
        ok("foo > bar"); // vec/vec aligné
        // NB : `bool` sur un opérateur arithmétique (`foo + 5 bool`) est
        // rejeté directement par le parser (ParseError), pas par la
        // validation — la garde `return_bool && !is_cmp` reste défensive.
    }

    #[test]
    fn rejects_set_operator() {
        ko("foo and bar", "binary operator: and");
    }

    #[test]
    fn rejects_offset_modifier() {
        ko("foo offset 5m", "offset");
    }

    #[test]
    fn accepts_topk_bottomk() {
        ok("topk(3, bms_v)");
        ok("bottomk(1, et112_power_w)");
        ok("topk(2, bms_v) by (bms_id)");
    }

    #[test]
    fn rejects_unsupported_parameterized_aggregator() {
        ko("quantile(0.9, bms_v)", "aggregator: quantile");
        ko("count_values(\"v\", bms_v)", "aggregator: count_values");
    }

    #[test]
    fn accepts_aggregation_grouping() {
        ok("sum by (bms_id)(bms_voltage)");
        ok("sum without (x)(m)");
        ok("avg by (address)(et112_power_w)");
        ok("count without (phase)(bms_v)");
    }

    #[test]
    fn rejects_vector_matching() {
        ko("a / on(x) b", "vector matching");
        ko("a * on(x) group_left b", "vector matching");
    }
}
