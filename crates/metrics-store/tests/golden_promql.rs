//! Golden test §6.5 : tous les `expr` PromQL des deux dashboards Grafana
//! embarqués doivent être acceptés par le shim, à l'exception explicite
//! de la sous-requête du panel 43 (cf. plan §6.5, décision (a)).

use serde_json::Value;

use metrics_store::promql::{parse_and_validate, PromQlError};

fn extract_exprs(json_src: &str) -> Vec<(u64, String)> {
    let v: Value = serde_json::from_str(json_src).expect("dashboard JSON");
    let mut out = Vec::new();
    fn walk(panels: &Value, out: &mut Vec<(u64, String)>) {
        let Some(arr) = panels.as_array() else { return };
        for p in arr {
            let id = p.get("id").and_then(|x| x.as_u64()).unwrap_or(0);
            if let Some(targets) = p.get("targets").and_then(|t| t.as_array()) {
                for t in targets {
                    if let Some(expr) = t.get("expr").and_then(|e| e.as_str()) {
                        let e = expr.trim();
                        if !e.is_empty() {
                            out.push((id, e.to_string()));
                        }
                    }
                }
            }
            if let Some(sub) = p.get("panels") {
                walk(sub, out);
            }
        }
    }
    walk(v.get("panels").unwrap_or(&Value::Null), &mut out);
    out
}

/// IDs de panels dont l'expression PromQL est connue pour NE PAS être
/// supportée par le shim (cf. plan §6.5). Toute autre expression doit
/// passer la validation.
const KNOWN_UNSUPPORTED_PANELS: &[u64] = &[
    // (vide) — le panel 43 (ex-subquery `[24h:1m]`) a été réécrit en
    // `avg_over_time(venus_shunt_current_abs[24h]) * 24/680*100` (décision (a)
    // du plan §6.5), via la métrique dérivée |I|. Plus aucune expression non
    // supportée dans les dashboards embarqués.
];

#[test]
fn ess_dashboard_coverage() {
    let src = include_str!("../../../docs/grafana-ess_dashboard.json");
    let exprs = extract_exprs(src);
    assert!(!exprs.is_empty(), "extraction des expressions a échoué");
    check_coverage(&exprs);
}

#[test]
fn solar_pv_dashboard_coverage() {
    let src = include_str!("../../../docs/grafana-solar_pv_dashboard.json");
    let exprs = extract_exprs(src);
    assert!(!exprs.is_empty(), "extraction des expressions a échoué");
    check_coverage(&exprs);
}

/// Remplace les variables de template Grafana par des valeurs neutres pour
/// que l'expression soit du PromQL valide (Grafana les substitue au runtime).
/// Seul `$bms_id` est réellement utilisé (matcher regex), mais on couvre
/// aussi les variables d'intervalle au cas où une future édition en ajoute.
fn sanitize_grafana_vars(expr: &str) -> String {
    expr.replace("$__rate_interval", "5m")
        .replace("${__rate_interval}", "5m")
        .replace("$__interval", "5m")
        .replace("${__interval}", "5m")
        .replace("$__range", "24h")
        .replace("${bms_id}", ".*")
        .replace("$bms_id", ".*")
}

/// Couverture des **16 dashboards Grafana provisionnés** (`contrib/grafana/
/// dashboards/*.json`) servis par le vrai Grafana via le datasource
/// `daly-metrics`. Toute expression non supportée par le moteur ferait
/// échouer le panneau côté Grafana → ce test garantit qu'aucune ne passe
/// inaperçue (garde CI permanente).
#[test]
fn provisioned_grafana_dashboards_coverage() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../contrib/grafana/dashboards");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .expect("dossier dashboards provisionnés introuvable")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "aucun dashboard provisionné trouvé");

    let mut failures: Vec<(String, u64, String, String)> = Vec::new();
    let mut total = 0usize;
    for path in &files {
        let src = std::fs::read_to_string(path).expect("lecture dashboard");
        let fname = path.file_name().unwrap().to_string_lossy().into_owned();
        for (panel_id, expr) in extract_exprs(&src) {
            total += 1;
            let sanitized = sanitize_grafana_vars(&expr);
            if let Err(e) = parse_and_validate(&sanitized) {
                let msg = match e {
                    PromQlError::ParseError(s) => format!("parse: {s}"),
                    PromQlError::Unsupported(s) => format!("unsupported: {s}"),
                    PromQlError::Execution(s) => format!("exec: {s}"),
                };
                failures.push((fname.clone(), panel_id, expr, msg));
            }
        }
    }

    if !failures.is_empty() {
        eprintln!("\nExpressions PromQL Grafana non supportées par le moteur :");
        for (f, id, e, msg) in &failures {
            eprintln!("  {f} panel {id}:\n    {e}\n    → {msg}");
        }
    }
    assert!(
        failures.is_empty(),
        "{} expression(s) non supportée(s) sur {} (Grafana ne serait pas 100% opérationnel)",
        failures.len(),
        total
    );
}

fn check_coverage(exprs: &[(u64, String)]) {
    let mut unexpected_failures: Vec<(u64, String, String)> = Vec::new();
    let mut unexpected_successes: Vec<(u64, String)> = Vec::new();

    for (panel_id, expr) in exprs {
        let result = parse_and_validate(expr);
        let is_known_unsupported = KNOWN_UNSUPPORTED_PANELS.contains(panel_id);
        match (result, is_known_unsupported) {
            (Ok(_), false) => {}                 // attendu : passe
            (Err(_), true) => {}                 // attendu : échec connu
            (Ok(_), true) => {
                unexpected_successes.push((*panel_id, expr.clone()));
            }
            (Err(e), false) => {
                let msg = match e {
                    PromQlError::ParseError(s) => format!("parse: {s}"),
                    PromQlError::Unsupported(s) => format!("unsupported: {s}"),
                    PromQlError::Execution(s) => format!("exec: {s}"),
                };
                unexpected_failures.push((*panel_id, expr.clone(), msg));
            }
        }
    }

    if !unexpected_failures.is_empty() {
        eprintln!("\nExpressions PromQL qui devraient passer mais échouent :");
        for (id, e, msg) in &unexpected_failures {
            eprintln!("  panel {id}: {e}\n    → {msg}");
        }
    }
    if !unexpected_successes.is_empty() {
        eprintln!("\nExpressions PromQL marquées comme non supportées qui passent :");
        for (id, e) in &unexpected_successes {
            eprintln!("  panel {id}: {e}");
        }
    }
    assert!(
        unexpected_failures.is_empty() && unexpected_successes.is_empty(),
        "couverture incomplète : {} échec(s) inattendu(s), {} succès inattendu(s)",
        unexpected_failures.len(),
        unexpected_successes.len()
    );
}
