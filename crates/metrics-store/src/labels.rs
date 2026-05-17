//! Sérialisation canonique des labels (clé déterministe pour `series_by_key`).
//!
//! La représentation est `{"<label1>":"<val1>","<label2>":"<val2>",…}` avec
//! les labels triés par nom (ordre lexicographique). Deux séries qui
//! diffèrent uniquement par l'ordre d'insertion des labels produisent donc
//! la **même** clé, ce qui garantit qu'on ne crée pas de doublon.

use std::collections::BTreeMap;

use smallvec::SmallVec;

pub type LabelVec = SmallVec<[(String, String); 4]>;

pub fn canonical_json(labels: &[(String, String)]) -> String {
    let map: BTreeMap<&str, &str> = labels
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    serde_json::to_string(&map).expect("BTreeMap<&str,&str> sérialisation infaillible")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_independent() {
        let a = canonical_json(&[("bms_id".into(), "1".into()), ("kind".into(), "soc".into())]);
        let b = canonical_json(&[("kind".into(), "soc".into()), ("bms_id".into(), "1".into())]);
        assert_eq!(a, b);
        assert_eq!(a, r#"{"bms_id":"1","kind":"soc"}"#);
    }

    #[test]
    fn empty_labels() {
        assert_eq!(canonical_json(&[]), "{}");
    }

    #[test]
    fn escapes_quotes_in_values() {
        let s = canonical_json(&[("note".into(), "a\"b".into())]);
        assert_eq!(s, r#"{"note":"a\"b"}"#);
    }
}
