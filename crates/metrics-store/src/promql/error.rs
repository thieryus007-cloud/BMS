//! Erreurs format Prometheus (utilisable directement par les handlers HTTP).

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromQlErrorBody {
    pub status: &'static str,    // toujours "error"
    pub error: String,
    pub error_type: &'static str, // "bad_data" / "execution"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromQlError {
    /// Erreur de syntaxe — texte direct du parser promql-parser.
    ParseError(String),
    /// Construction non supportée par le shim (subquery, set ops, etc.).
    Unsupported(String),
    /// Erreur d'exécution (table absente, type mismatch entre vecteurs, …).
    Execution(String),
}

impl PromQlError {
    pub fn message(&self) -> String {
        match self {
            PromQlError::ParseError(s) => format!("parse error: {s}"),
            PromQlError::Unsupported(s) => format!("unsupported: {s}"),
            PromQlError::Execution(s) => format!("execution error: {s}"),
        }
    }

    pub fn error_type(&self) -> &'static str {
        match self {
            PromQlError::ParseError(_) | PromQlError::Unsupported(_) => "bad_data",
            PromQlError::Execution(_) => "execution",
        }
    }

    pub fn body(&self) -> PromQlErrorBody {
        PromQlErrorBody {
            status: "error",
            error: self.message(),
            error_type: self.error_type(),
        }
    }
}

impl std::fmt::Display for PromQlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for PromQlError {}
