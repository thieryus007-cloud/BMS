//! POST /api/v1/import/prometheus
//!
//! Accepte le format texte Prometheus exposition (Prometheus remote write simplifié)
//! et écrit chaque échantillon directement dans metrics-store (redb).
//!
//! Ce endpoint remplace l'endpoint VictoriaMetrics `/api/v1/import/prometheus`
//! utilisé par energy-manager (monitoring.rs, lg_thinq.rs, rule_metrics.rs,
//! water_heater/mod.rs). Après la suppression de VictoriaMetrics (Phase 5),
//! ces écriture tombaient silencieusement dans le vide (port 8428 fermé).
//!
//! ## Format accepté
//!
//! ```text
//! # commentaire ignoré
//! metric_name value [timestamp_ms]
//! metric_name{label1="val1",label2="val2"} value [timestamp_ms]
//! ```
//!
//! Le timestamp est en millisecondes depuis l'époque Unix (comme VictoriaMetrics).
//! S'il est absent, on utilise l'horodatage courant.
//! Les valeurs non-finies (NaN, Inf) sont ignorées silencieusement.
//!
//! ## Différence avec le rate-limiter redb
//!
//! Cet endpoint N'utilise PAS le RateLimiter — l'émetteur (energy-manager) gère
//! lui-même sa cadence (60s pour le monitoring système, à la demande pour les règles).
//! Appliquer un rate-limiter ici écraserait silencieusement des données légitimes.

use axum::{extract::State, http::StatusCode, response::IntoResponse};
use metrics_store::Sample;
use tracing::{debug, warn};

use crate::state::AppState;

/// POST /api/v1/import/prometheus
pub async fn import_prometheus(
    State(state): State<AppState>,
    body: String,
) -> impl IntoResponse {
    let Some(store) = &state.metrics_store else {
        return (StatusCode::SERVICE_UNAVAILABLE, "metrics-store not available");
    };
    let writer = store.writer();
    let mut written = 0usize;
    let mut skipped = 0usize;

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_line(line) {
            Some(sample) => {
                let _ = writer.try_write(sample);
                written += 1;
            }
            None => {
                warn!("prometheus_import: ligne ignorée (format invalide) : {}", line);
                skipped += 1;
            }
        }
    }

    debug!("prometheus_import: {} sample(s) écrits, {} ignorés", written, skipped);
    (StatusCode::NO_CONTENT, "")
}

/// Parse une ligne du format Prometheus text.
///
/// Retourne `None` si la ligne est mal formée ou la valeur non finie (NaN/Inf).
fn parse_line(line: &str) -> Option<Sample> {
    // Découpe entre la spec métrique (nom + labels) et "valeur [timestamp]"
    let (spec, rest) = if let Some(brace_end) = line.find('}') {
        (&line[..=brace_end], line[brace_end + 1..].trim())
    } else {
        // Pas de labels : le premier espace délimite le nom
        let idx = line.find(' ')?;
        (&line[..idx], line[idx..].trim())
    };

    // Valeur et timestamp optionnel
    let mut parts = rest.split_whitespace();
    let value: f64 = parts.next()?.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    let ts_ms: i64 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

    // Nom de la métrique et labels
    let (name, labels) = if let Some(brace_start) = spec.find('{') {
        let name = spec[..brace_start].trim();
        let inner = spec[brace_start + 1..].trim_end_matches('}');
        (name, parse_labels(inner))
    } else {
        (spec.trim(), vec![])
    };

    if name.is_empty() {
        return None;
    }

    let mut sample = Sample::new(name, ts_ms, value);
    for (k, v) in labels {
        sample = sample.with_label(k, v);
    }
    Some(sample)
}

/// Parse `key="val1",key2="val2"` en vecteur de paires.
fn parse_labels(s: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let eq = match rest.find('=') {
            Some(i) => i,
            None => break,
        };
        let key = rest[..eq].trim().to_string();
        rest = rest[eq + 1..].trim();
        if !rest.starts_with('"') {
            break;
        }
        rest = &rest[1..]; // saute le guillemet ouvrant
        let mut val = String::new();
        let mut end_idx = rest.len();
        let mut chars = rest.char_indices();
        loop {
            match chars.next() {
                Some((i, '"')) => {
                    end_idx = i;
                    break;
                }
                Some((_, '\\')) => {
                    if let Some((_, c)) = chars.next() {
                        val.push(c);
                    }
                }
                Some((_, c)) => val.push(c),
                None => break,
            }
        }
        rest = rest[end_idx..].trim().trim_start_matches(',').trim();
        result.push((key, val));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::parse_line;

    #[test]
    fn test_simple_metric() {
        let s = parse_line("em_cpu_percent 4.2 1700000000000").unwrap();
        assert_eq!(s.metric, "em_cpu_percent");
        assert_eq!(s.value, 4.2);
        assert_eq!(s.ts_ms, 1700000000000);
    }

    #[test]
    fn test_metric_with_labels() {
        let s = parse_line(r#"em_load_avg{window="1m"} 0.45 1700000000000"#).unwrap();
        assert_eq!(s.metric, "em_load_avg");
        assert_eq!(s.value, 0.45);
        assert!(s.labels.iter().any(|(k, v)| k == "window" && v == "1m"));
    }

    #[test]
    fn test_multi_labels() {
        let s = parse_line(r#"em_process_cpu_percent{process="daly-bms-server"} 1.5 1700000000000"#).unwrap();
        assert_eq!(s.metric, "em_process_cpu_percent");
        assert!(s.labels.iter().any(|(k, v)| k == "process" && v == "daly-bms-server"));
    }

    #[test]
    fn test_no_timestamp() {
        let s = parse_line("pi5_cpu_percent 15.3").unwrap();
        assert_eq!(s.metric, "pi5_cpu_percent");
        assert_eq!(s.value, 15.3);
        assert!(s.ts_ms > 0);
    }

    #[test]
    fn test_nan_rejected() {
        assert!(parse_line("bad_metric NaN 1700000000000").is_none());
    }

    #[test]
    fn test_comment_ignored() {
        // parse_line n'est pas appelé pour les commentaires (filtre dans import_prometheus)
        assert!(parse_line("# HELP em_cpu_percent CPU usage").is_none());
    }
}
