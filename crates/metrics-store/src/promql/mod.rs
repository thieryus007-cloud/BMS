//! Transpileur PromQL → plan d'exécution sur le `Reader` redb.
//!
//! Plan : `docs/plan_migration_vm_redb.md` §6.
//!
//! Pipeline :
//! 1. **parse** : délégué à [`promql_parser::parser::parse`].
//! 2. **validate** ([`validate::validate`]) : liste blanche des fonctions et
//!    opérateurs supportés. Depuis l'évolution de conformité (cf.
//!    `docs/Evolution-compliance-PromQL.md`), le sous-ensemble couvre aussi :
//!    les modificateurs temporels `offset` **et** `@`, les opérateurs
//!    ensemblistes `and`/`or`/`unless`, le matching vectoriel
//!    `on`/`ignoring`/`group_left`/`group_right`, les agrégateurs `quantile`,
//!    `group`, `count_values`, `stddev`, `stdvar`, et les **sous-requêtes**
//!    `expr[range:step]`. Seules restent rejetées les fonctions hors liste
//!    blanche (ex. `histogram_quantile`, `vector`).
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
