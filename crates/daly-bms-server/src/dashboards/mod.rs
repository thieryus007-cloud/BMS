//! Module dashboards — catalogue de panels importés depuis Grafana.
//!
//! Architecture :
//! - `grafana::parse_dashboard()` lit un dashboard Grafana JSON et en extrait
//!   une liste normalisée de panels (`Panel`).
//! - Le catalogue est chargé une seule fois au démarrage via `Catalog::load_default()`
//!   qui inclut le JSON Grafana au build (`include_str!`).
//! - Exposé via `GET /api/v1/dashboards/catalog`.
//! - Exécution des PromQL d'un panel via `GET /api/v1/dashboards/panel/:id/data`.

pub mod grafana;

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Type de panel normalisé (subset de Grafana suffisant pour notre UI).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PanelKind {
    Row,
    Stat,
    TimeSeries,
    BarChart,
    Gauge,
    BarGauge,
    Table,
    Unknown,
}

impl PanelKind {
    pub fn from_grafana(s: &str) -> Self {
        match s {
            "row"        => PanelKind::Row,
            "stat"       => PanelKind::Stat,
            "timeseries" => PanelKind::TimeSeries,
            "barchart"   => PanelKind::BarChart,
            "gauge"      => PanelKind::Gauge,
            "bargauge"   => PanelKind::BarGauge,
            "table"      => PanelKind::Table,
            _            => PanelKind::Unknown,
        }
    }
}

/// Position et taille initiale d'un panel (grille Grafana 24 colonnes).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GridPos {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Une requête PromQL associée à un panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelQuery {
    /// Identifiant interne (A, B, C... — vient de Grafana `refId`).
    pub ref_id: String,
    /// Expression PromQL.
    pub expr:   String,
    /// Format de légende (peut contenir des `{label}` Grafana).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legend: Option<String>,
}

/// Panel normalisé — prêt à être consommé côté UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Panel {
    /// Identifiant stable (id Grafana, ex: "27").
    pub id:       String,
    pub title:    String,
    pub kind:     PanelKind,
    pub grid_pos: GridPos,
    /// Unité Grafana (ex: "watt", "percent", "celsius") — peut être vide.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit:     String,
    /// Liste des requêtes PromQL (vide pour les rows).
    #[serde(default)]
    pub queries:  Vec<PanelQuery>,
    /// Nombre de décimales d'affichage (Grafana `decimals`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,
}

/// Catalogue chargé en mémoire au démarrage.
#[derive(Debug, Clone)]
pub struct Catalog {
    panels: Arc<Vec<Panel>>,
}

impl Catalog {
    /// Construit un catalogue vide (fallback si parsing échoue).
    pub fn empty() -> Self {
        Self { panels: Arc::new(Vec::new()) }
    }

    /// Charge le dashboard Grafana embarqué dans le binaire.
    /// Le fichier est inclus au build via `include_str!`.
    pub fn load_default() -> Self {
        const GRAFANA_JSON: &str = include_str!("../../../../docs/grafana-ess_dashboard.json");
        match grafana::parse_dashboard(GRAFANA_JSON) {
            Ok(panels) => {
                tracing::info!(count = panels.len(), "Catalogue de panels chargé depuis grafana-ess_dashboard.json");
                Self { panels: Arc::new(panels) }
            }
            Err(e) => {
                tracing::error!(error = %e, "Échec parsing grafana-ess_dashboard.json — catalogue vide");
                Self::empty()
            }
        }
    }

    pub fn panels(&self) -> &[Panel] {
        &self.panels
    }

    pub fn find(&self, id: &str) -> Option<&Panel> {
        self.panels.iter().find(|p| p.id == id)
    }
}
