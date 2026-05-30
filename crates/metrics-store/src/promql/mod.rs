//! Transpileur PromQL → plan d'exécution sur le `Reader` redb.
//!
//! Plan : `docs/plan_migration_vm_redb.md` §6.
//!
//! Pipeline :
//! 1. **parse** : délégué à [`promql_parser::parser::parse`].
//! 2. **validate** ([`validate::validate`]) : rejette les constructions
//!    non supportées (subqueries, `offset`/`@`, set ops `and/or/unless`,
//!    vector matching `on/ignoring/group_left/right`, fonctions hors liste
//!    blanche). Liste blanche étendue au-delà du golden set §6.5 (cf.
//!    `docs/promql-compat-roadmap.md` : comparaisons, `by/without`,
//!    `topk/bottomk`, `irate`, math, labels, prédiction/stats, `absent`).
//! 3. **execute** ([`exec::Evaluator`]) : évalue l'AST sur une plage
//!    `(start, end, step)` en interrogeant le `Reader`. Sélection
//!    automatique de la table (raw/hourly/daily) selon §6.3.
//!
//! L'erreur retournée par `parse_and_validate` a la forme Prometheus
//! standard (`status=error`, `errorType=bad_data`) pour être renvoyée
//! telle quelle par les handlers HTTP.

pub mod error;
pub mod exec;
pub mod validate;

pub use error::PromQlError;
pub use exec::{Evaluator, InstantSample, RangeSeries};
pub use validate::validate;

use promql_parser::parser::Expr;

/// Parse + valide une expression PromQL. Renvoie l'AST prêt à l'exécution.
pub fn parse_and_validate(src: &str) -> Result<Expr, PromQlError> {
    let expr = promql_parser::parser::parse(src).map_err(PromQlError::ParseError)?;
    validate(&expr)?;
    Ok(expr)
}
