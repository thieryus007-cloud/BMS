//! Parser du format Prometheus exposition (text-based).
//!
//! Spec : <https://github.com/prometheus/docs/blob/main/content/docs/instrumenting/exposition_formats.md>
//!
//! Une ligne valide :
//! ```text
//! metric_name{label1="value1",label2="value2"} 3.14 1700000000000
//! ```
//! - Le timestamp (millisecondes Unix) est optionnel ; absent → `default_ts_ms`.
//! - Les commentaires `# …`, lignes blanches, et `# HELP` / `# TYPE` sont ignorés.
//! - Échappements dans les valeurs de label : `\"`, `\\`, `\n`.
//!
//! Le parser est tolérant ligne par ligne : une ligne malformée n'avorte
//! pas le batch — elle est comptée comme erreur et la ligne suivante est
//! parsée. Retourne `(samples, errors)`.

use smallvec::SmallVec;

use crate::writer::Sample;

#[derive(Debug, Clone)]
pub struct ParseStats {
    pub lines_total: usize,
    pub lines_comment: usize,
    pub lines_blank: usize,
    pub lines_sample: usize,
    pub lines_error: usize,
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub line_no: usize,
    pub line: String,
    pub message: String,
}

/// Parse un body Prometheus exposition. `default_ts_ms` est utilisé pour
/// les lignes sans timestamp explicite.
pub fn parse(body: &str, default_ts_ms: i64) -> (Vec<Sample>, Vec<ParseError>, ParseStats) {
    let mut samples = Vec::new();
    let mut errors = Vec::new();
    let mut stats = ParseStats {
        lines_total: 0,
        lines_comment: 0,
        lines_blank: 0,
        lines_sample: 0,
        lines_error: 0,
    };

    for (idx, raw_line) in body.lines().enumerate() {
        stats.lines_total += 1;
        let line = raw_line.trim();
        if line.is_empty() {
            stats.lines_blank += 1;
            continue;
        }
        if line.starts_with('#') {
            stats.lines_comment += 1;
            continue;
        }
        match parse_line(line, default_ts_ms) {
            Ok(s) => {
                samples.push(s);
                stats.lines_sample += 1;
            }
            Err(msg) => {
                errors.push(ParseError {
                    line_no: idx + 1,
                    line: raw_line.to_string(),
                    message: msg,
                });
                stats.lines_error += 1;
            }
        }
    }

    (samples, errors, stats)
}

fn parse_line(line: &str, default_ts_ms: i64) -> Result<Sample, String> {
    // Lit metric_name (jusqu'à `{`, ` ` ou EOL).
    let (metric, rest) = split_metric_name(line)?;
    let rest = rest.trim_start();

    let (labels, rest) = if let Some(after_brace) = rest.strip_prefix('{') {
        parse_labels(after_brace)?
    } else {
        (SmallVec::new(), rest)
    };
    let rest = rest.trim_start();

    let mut it = rest.split_ascii_whitespace();
    let value_str = it
        .next()
        .ok_or_else(|| "valeur manquante après le nom de la métrique".to_string())?;
    let value: f64 = value_str
        .parse()
        .map_err(|e| format!("valeur invalide '{value_str}': {e}"))?;

    let ts_ms = match it.next() {
        Some(ts_str) => ts_str
            .parse::<i64>()
            .map_err(|e| format!("timestamp invalide '{ts_str}': {e}"))?,
        None => default_ts_ms,
    };

    if it.next().is_some() {
        return Err("tokens supplémentaires après le timestamp".to_string());
    }

    Ok(Sample { metric: metric.to_string(), labels, ts_ms, value })
}

fn split_metric_name(line: &str) -> Result<(&str, &str), String> {
    let bytes = line.as_bytes();
    if bytes.is_empty() || !is_metric_name_start(bytes[0]) {
        return Err(format!(
            "caractère initial invalide pour un nom de métrique: {:?}",
            bytes.first()
        ));
    }
    let end = bytes
        .iter()
        .position(|&b| !is_metric_name_cont(b))
        .unwrap_or(bytes.len());
    Ok((&line[..end], &line[end..]))
}

fn is_metric_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b':'
}
fn is_metric_name_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b':'
}
fn is_label_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}
fn is_label_name_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn parse_labels(input: &str) -> Result<(SmallVec<[(String, String); 4]>, &str), String> {
    let mut labels = SmallVec::new();
    let mut s = input.trim_start();
    if let Some(rest) = s.strip_prefix('}') {
        return Ok((labels, rest));
    }
    loop {
        s = s.trim_start();
        // nom du label
        let (name, after_name) = take_label_name(s)?;
        let after_name = after_name.trim_start();
        let after_eq = after_name
            .strip_prefix('=')
            .ok_or_else(|| format!("attendu '=' après '{name}'"))?;
        let after_eq = after_eq.trim_start();
        // valeur quotée
        let (value, after_val) = take_quoted(after_eq)?;
        labels.push((name.to_string(), value));
        let after_val = after_val.trim_start();
        if let Some(rest) = after_val.strip_prefix(',') {
            s = rest;
            // Tolère `{a="x",}` à la Go : si juste après on a `}`, on sort.
            let trimmed = s.trim_start();
            if let Some(rest2) = trimmed.strip_prefix('}') {
                return Ok((labels, rest2));
            }
            continue;
        }
        if let Some(rest) = after_val.strip_prefix('}') {
            return Ok((labels, rest));
        }
        return Err(format!("attendu ',' ou '}}' après valeur de label, vu: {after_val:?}"));
    }
}

fn take_label_name(s: &str) -> Result<(&str, &str), String> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !is_label_name_start(bytes[0]) {
        return Err(format!("caractère initial invalide pour un label: {:?}", bytes.first()));
    }
    let end = bytes
        .iter()
        .position(|&b| !is_label_name_cont(b))
        .unwrap_or(bytes.len());
    Ok((&s[..end], &s[end..]))
}

/// Lit une chaîne entre guillemets, gère `\"`, `\\`, `\n`.
fn take_quoted(s: &str) -> Result<(String, &str), String> {
    let rest = s
        .strip_prefix('"')
        .ok_or_else(|| format!("attendu '\"' au début d'une valeur de label, vu: {s:?}"))?;
    let mut out = String::new();
    let mut chars = rest.char_indices();
    while let Some((i, ch)) = chars.next() {
        match ch {
            '"' => {
                // Position absolue de la fin = i (offset dans `rest`) + 1 pour consommer le ".
                let consumed = &rest[..i];
                let after = &rest[i + 1..];
                let _ = consumed;
                return Ok((out, after));
            }
            '\\' => {
                let (_j, next) = chars.next().ok_or_else(|| "échappement tronqué".to_string())?;
                match next {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    'n' => out.push('\n'),
                    other => {
                        return Err(format!("séquence d'échappement inconnue: \\{other}"));
                    }
                }
            }
            other => out.push(other),
        }
    }
    Err("chaîne non terminée par '\"'".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(line: &str) -> Sample {
        let (s, errs, _) = parse(line, 12345);
        assert!(errs.is_empty(), "erreurs inattendues: {errs:?}");
        assert_eq!(s.len(), 1);
        s.into_iter().next().unwrap()
    }

    #[test]
    fn simple_metric_no_labels() {
        let s = one("foo_total 42");
        assert_eq!(s.metric, "foo_total");
        assert_eq!(s.value, 42.0);
        assert_eq!(s.ts_ms, 12345);
        assert!(s.labels.is_empty());
    }

    #[test]
    fn with_labels_and_timestamp() {
        let s = one(r#"bms_v{bms_id="0x01",pack="1"} 12.34 1700000000000"#);
        assert_eq!(s.metric, "bms_v");
        assert_eq!(s.value, 12.34);
        assert_eq!(s.ts_ms, 1_700_000_000_000);
        assert_eq!(s.labels.len(), 2);
        assert_eq!(s.labels[0], ("bms_id".to_string(), "0x01".to_string()));
        assert_eq!(s.labels[1], ("pack".to_string(), "1".to_string()));
    }

    #[test]
    fn empty_label_set() {
        let s = one("foo{} 1");
        assert!(s.labels.is_empty());
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let body = "# HELP foo The number of foos\n\
                    # TYPE foo counter\n\
                    \n\
                    foo 1 1000\n\
                    foo 2 2000\n";
        let (samples, errs, stats) = parse(body, 0);
        assert!(errs.is_empty());
        assert_eq!(samples.len(), 2);
        assert_eq!(stats.lines_comment, 2);
        assert_eq!(stats.lines_blank, 1);
        assert_eq!(stats.lines_sample, 2);
    }

    #[test]
    fn escape_sequences_in_label_values() {
        let s = one(r#"x{a="line1\nline2",b="quote: \"hi\"",c="back: \\"} 0"#);
        assert_eq!(s.labels[0].1, "line1\nline2");
        assert_eq!(s.labels[1].1, "quote: \"hi\"");
        assert_eq!(s.labels[2].1, "back: \\");
    }

    #[test]
    fn trailing_comma_in_labels_tolerated() {
        let s = one(r#"x{a="1",} 0"#);
        assert_eq!(s.labels.len(), 1);
    }

    #[test]
    fn negative_and_scientific_values() {
        let s = one("x -3.14e-2");
        assert!((s.value - -0.0314).abs() < 1e-12);

        let s = one("x +inf");
        assert!(s.value.is_infinite() && s.value > 0.0);

        let s = one("x nan");
        assert!(s.value.is_nan());
    }

    #[test]
    fn invalid_lines_collected_in_errors() {
        let body = "good_one 1\n\
                    not a metric\n\
                    also{bad=missing_quotes} 1\n\
                    good_two 2\n";
        let (samples, errors, _) = parse(body, 0);
        assert_eq!(samples.len(), 2);
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].line_no, 2);
        assert_eq!(errors[1].line_no, 3);
    }

    #[test]
    fn missing_value_is_error() {
        let body = "foo\n";
        let (samples, errors, _) = parse(body, 0);
        assert!(samples.is_empty());
        assert_eq!(errors.len(), 1);
    }
}
