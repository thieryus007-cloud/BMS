//! Validation : walk de l'AST. Rejette tout ce qui n'est pas dans le
//! sous-ensemble PromQL supporté par l'évaluateur (`exec.rs`).
//!
//! Couverture (cf. `docs/Evolution-compliance-PromQL.md`) : sélecteurs avec
//! `offset` **et** `@` (modificateurs temporels), arithmétique, comparaisons,
//! agrégations (`sum/min/max/avg/count/group/stddev/stdvar` + paramétrées
//! `topk/bottomk/quantile/count_values`), opérateurs ensemblistes
//! (`and/or/unless`), matching vectoriel (`on/ignoring/group_left/group_right`),
//! fonctions à fenêtre / instantanées / labels, et **sous-requêtes**
//! (`expr[range:step]`).

use promql_parser::parser::{
    AggregateExpr, BinaryExpr, Call, Expr, MatrixSelector, ParenExpr, SubqueryExpr, UnaryExpr,
    VectorSelector,
};

use super::error::PromQlError;

/// Fonctions à fenêtre temporelle (`f(m[range])`).
pub const SUPPORTED_RANGE_FUNCS: &[&str] = &[
    "increase",
    "rate",
    "irate",
    "delta",
    "deriv",
    "predict_linear",
    "changes",
    "resets",
    "avg_over_time",
    "sum_over_time",
    "min_over_time",
    "max_over_time",
    "count_over_time",
    "last_over_time",
    "stddev_over_time",
    "stdvar_over_time",
    "quantile_over_time",
    "absent_over_time",
];

/// Fonctions instantanées (`f(vec)` ou `f(vec, scalar…)`).
pub const SUPPORTED_INSTANT_FUNCS: &[&str] = &[
    "abs", "clamp_min", "clamp_max", "clamp", "ceil", "floor", "round", "sqrt", "exp", "ln",
    "log2", "log10", "sgn", "absent",
];

/// Fonctions de manipulation de labels (`f(vec, "str", …)`) — arguments string
/// autorisés uniquement pour ces fonctions.
pub const SUPPORTED_LABEL_FUNCS: &[&str] = &["label_replace", "label_join"];

/// Opérateurs d'agrégation instant supportés (sans paramètre).
pub const SUPPORTED_AGGREGATORS: &[&str] =
    &["sum", "max", "min", "avg", "count", "group", "stddev", "stdvar"];

/// Agrégateurs paramétrés par un **scalaire** (`op(k, vec)` / `op(φ, vec)`).
pub const SCALAR_PARAM_AGGREGATORS: &[&str] = &["topk", "bottomk", "quantile"];

/// Agrégateurs paramétrés par une **chaîne** (`count_values("label", vec)`).
pub const STRING_PARAM_AGGREGATORS: &[&str] = &["count_values"];

/// Opérateurs binaires arithmétiques supportés (vec×scalar et vec×vec avec
/// matching `on`/`ignoring` et cardinalité `group_left`/`group_right`).
pub const SUPPORTED_BINOPS: &[&str] = &["+", "-", "*", "/", "%", "^"];

/// Opérateurs de comparaison supportés (filtre, ou 0/1 avec `bool`).
pub const SUPPORTED_CMP_OPS: &[&str] = &["==", "!=", ">", "<", ">=", "<="];

/// Opérateurs ensemblistes (logiques) supportés.
pub const SUPPORTED_SET_OPS: &[&str] = &["and", "or", "unless"];

pub fn validate(expr: &Expr) -> Result<(), PromQlError> {
    match expr {
        Expr::NumberLiteral(_) => Ok(()),
        Expr::StringLiteral(_) => unsupported("string literal"),
        Expr::VectorSelector(vs) => validate_vector_selector(vs),
        Expr::MatrixSelector(MatrixSelector { vs, .. }) => validate_vector_selector(vs),
        Expr::Paren(ParenExpr { expr }) => validate(expr),
        Expr::Unary(UnaryExpr { expr }) => validate(expr),
        // Sous-requête : on valide l'expression interne. La résolution
        // (range:step) est gérée par l'évaluateur (`exec.rs::eval_subquery`).
        Expr::Subquery(SubqueryExpr { expr, .. }) => validate(expr),
        Expr::Extension(_) => unsupported("extension expression"),
        Expr::Aggregate(a) => validate_aggregate(a),
        Expr::Binary(b) => validate_binary(b),
        Expr::Call(c) => validate_call(c),
    }
}

fn validate_vector_selector(_vs: &VectorSelector) -> Result<(), PromQlError> {
    // `offset` et `@` (modificateurs temporels) sont tous deux supportés par
    // l'évaluateur (cf. exec.rs::offset_ms / resolve_at). Rien à rejeter ici.
    Ok(())
}

fn validate_aggregate(a: &AggregateExpr) -> Result<(), PromQlError> {
    let op_str = a.op.to_string();
    let is_plain = SUPPORTED_AGGREGATORS.contains(&op_str.as_str());
    let is_scalar_param = SCALAR_PARAM_AGGREGATORS.contains(&op_str.as_str());
    let is_string_param = STRING_PARAM_AGGREGATORS.contains(&op_str.as_str());
    if !is_plain && !is_scalar_param && !is_string_param {
        return unsupported(&format!("aggregator: {op_str}"));
    }
    if is_plain && a.param.is_some() {
        return unsupported(&format!("parameterized aggregator: {op_str}"));
    }
    if (is_scalar_param || is_string_param) && a.param.is_none() {
        return unsupported(&format!("aggregator {op_str} requires a parameter"));
    }
    if let Some(p) = &a.param {
        if is_string_param {
            // `count_values("label", vec)` : le paramètre doit être une chaîne.
            if !matches!(p.as_ref(), Expr::StringLiteral(_)) {
                return unsupported(&format!(
                    "{op_str}: le paramètre doit être un littéral chaîne"
                ));
            }
        } else {
            // topk/bottomk/quantile : paramètre scalaire (pas une chaîne).
            if matches!(p.as_ref(), Expr::StringLiteral(_)) {
                return unsupported(&format!("{op_str}: le paramètre doit être scalaire"));
            }
            validate(p)?;
        }
    }
    // Le groupement `by`/`without` est supporté par l'évaluateur.
    validate(&a.expr)
}

fn validate_binary(b: &BinaryExpr) -> Result<(), PromQlError> {
    let op_str = b.op.to_string();
    let is_arith = SUPPORTED_BINOPS.contains(&op_str.as_str());
    let is_cmp = SUPPORTED_CMP_OPS.contains(&op_str.as_str());
    let is_set = SUPPORTED_SET_OPS.contains(&op_str.as_str());
    if !is_arith && !is_cmp && !is_set {
        return unsupported(&format!("binary operator: {op_str}"));
    }
    if let Some(m) = &b.modifier {
        // `bool` n'est valide que sur une comparaison (filtre → 0/1).
        if m.return_bool && !is_cmp {
            return unsupported("bool modifier (réservé aux comparaisons)");
        }
    }
    // Matching vectoriel (`on`/`ignoring`/`group_left`/`group_right`) est
    // désormais supporté par l'évaluateur (`exec.rs::vector_binary_op`).
    validate(&b.lhs)?;
    validate(&b.rhs)?;
    Ok(())
}

fn validate_call(c: &Call) -> Result<(), PromQlError> {
    let name = c.func.name;
    let is_range = SUPPORTED_RANGE_FUNCS.contains(&name);
    let is_instant = SUPPORTED_INSTANT_FUNCS.contains(&name);
    let is_label = SUPPORTED_LABEL_FUNCS.contains(&name);
    if !is_range && !is_instant && !is_label {
        return unsupported(&format!("function: {name}"));
    }
    if is_label {
        // `label_replace`/`label_join` : 1er arg = vecteur instantané (validé),
        // arguments suivants = string literals (autorisés uniquement ici).
        let first = c
            .args
            .args
            .first()
            .ok_or_else(|| PromQlError::Unsupported(format!("{name}: argument manquant")))?;
        validate(first)?;
        for arg in &c.args.args[1..] {
            if !matches!(arg.as_ref(), Expr::StringLiteral(_)) {
                return unsupported(&format!(
                    "{name}: les arguments après le vecteur doivent être des chaînes"
                ));
            }
        }
        return Ok(());
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
        ok("group(bms_status)");
        ok("stddev(bms_v)");
        ok("stdvar(bms_v)");
    }

    #[test]
    fn accepts_math_functions() {
        ok("sqrt(bms_v)");
        ok("exp(bms_v)");
        ok("ln(bms_v)");
        ok("log2(bms_v)");
        ok("log10(bms_v)");
        ok("sgn(bms_v)");
        ok("clamp(bms_v, 0, 100)");
    }

    #[test]
    fn accepts_p2_p3_functions() {
        // P2
        ok("deriv(bms_v[10m])");
        ok("predict_linear(bms_soc[1h], 3600)");
        ok("quantile_over_time(0.95, bms_v[1d])");
        ok("stddev_over_time(bms_v[1h])");
        ok("stdvar_over_time(bms_v[1h])");
        // P3
        ok("changes(bms_status[1h])");
        ok("resets(bms_uptime[1d])");
        ok(r#"absent(bms_v{bms_id="1"})"#);
        ok(r#"absent_over_time(bms_v{bms_id="1"}[5m])"#);
        ok("round(bms_v, 0.1)");
    }

    #[test]
    fn accepts_label_functions() {
        ok(r#"label_replace(bms_v, "pack", "$1", "bms_id", "(.*)")"#);
        ok(r#"label_join(bms_v, "combo", "-", "bms_id", "phase")"#);
    }

    #[test]
    fn rejects_bare_string_literal() {
        // Une chaîne nue (hors argument de fonction de labels) reste rejetée.
        ko(r#""foo""#, "string literal");
    }

    #[test]
    fn accepts_subquery() {
        // Sous-requêtes désormais supportées (cf. exec.rs::eval_subquery).
        ok(r#"avg_over_time(clamp_min(venus_shunt_current_a,0)[24h:1m])"#);
        ok("avg_over_time(rate(bms_energy_wh[5m])[1h:5m])");
        ok("max_over_time(rate(cnt[5m])[1h:1m])");
    }

    #[test]
    fn rejects_unsupported_function() {
        ko("histogram_quantile(0.95, foo)", "function: histogram_quantile");
        ko("vector(1)", "function: vector");
        ko("idelta(foo[5m])", "function: idelta");
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
    }

    #[test]
    fn accepts_set_operators() {
        ok("bms_soc < 20 and bms_temp_c > 45");
        ok("foo or bar");
        ok("bms_status unless mppt_status");
        ok("bms_status unless on(bms_id) mppt_status");
        ok("foo and on(instance) bar");
    }

    #[test]
    fn accepts_offset_modifier() {
        ok("foo offset 5m");
        ok("foo offset -5m");
        ok("rate(foo[5m] offset 1h)");
        ok("max_over_time(solar_yield_kwh[1d] offset 365d)");
        ok("avg_over_time(bms_v[1h]) - avg_over_time(bms_v[1h] offset 24h)");
        ok(r#"bms_soc{bms_id="1"} - bms_soc{bms_id="1"} offset 24h"#);
    }

    #[test]
    fn accepts_at_modifier() {
        ok("foo @ 1620000000");
        ok("foo @ start()");
        ok("foo @ end()");
        ok("rate(foo[5m] @ 1620000000)");
    }

    #[test]
    fn accepts_topk_bottomk() {
        ok("topk(3, bms_v)");
        ok("bottomk(1, et112_power_w)");
        ok("topk(2, bms_v) by (bms_id)");
    }

    #[test]
    fn accepts_quantile_and_count_values() {
        ok("quantile(0.95, bms_cell_voltage_v)");
        ok("quantile(0.9, bms_v) by (bms_id)");
        ok(r#"count_values("version", build_info)"#);
        ok(r#"count_values("le", bms_v) by (bms_id)"#);
    }

    #[test]
    fn rejects_misparameterized_aggregators() {
        // count_values exige une chaîne, quantile un scalaire. Ces erreurs de
        // type de paramètre sont détectées par le parser lui-même (ParseError)
        // avant même la validation — on vérifie simplement le rejet.
        assert!(parse_and_validate(r#"count_values(5, bms_v)"#).is_err());
        assert!(parse_and_validate(r#"quantile("x", bms_v)"#).is_err());
    }

    #[test]
    fn accepts_aggregation_grouping() {
        ok("sum by (bms_id)(bms_voltage)");
        ok("sum without (x)(m)");
        ok("avg by (address)(et112_power_w)");
        ok("count without (phase)(bms_v)");
    }

    #[test]
    fn accepts_vector_matching() {
        ok("mppt_power_w / on(string_id) pv_panel_theoretical_w");
        ok("a * on(x) group_left b");
        ok("a * on(x) group_left(capacity_ah) b");
        ok("a / ignoring(mppt_id) b");
        ok("bms_power_w * on(bms_id) group_left(capacity_ah) bms_capacity_ah");
    }
}
