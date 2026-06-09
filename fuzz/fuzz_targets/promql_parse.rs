//! Fuzz du parseur/validateur PromQL (audit 2026-06 §17).
//!
//! Frontière d'entrée HTTP : `/api/v1/query{,_range}` passe la chaîne
//! utilisateur à `parse_and_validate`. Invariant : jamais de panique —
//! n'importe quelle chaîne doit produire `Ok(ast)` ou une erreur propre.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|query: &str| {
    let _ = metrics_store::promql::parse_and_validate(query);
});
