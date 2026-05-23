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
pub mod storage;

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
    /// Charge tous les dashboards Grafana embarqués dans le binaire.
    /// Les fichiers sont inclus au build via `include_str!` puis concaténés.
    /// Les IDs Grafana d'origine se chevauchent entre fichiers (chaque dashboard
    /// commence à id=1), donc on namespace chaque ID avec son préfixe source
    /// (ex: `bms.1`, `infra.2`) pour garantir l'unicité dans le catalogue.
    pub fn load_default() -> Self {
        const SOURCES: &[(&str, &str)] = &[
            // Dashboards hérités (IDs déjà disjoints, non préfixés pour compat
            // avec SOLAR_PV_IDS dans templates/history.html).
            ("",         include_str!("../../../../docs/grafana-ess_dashboard.json")),
            ("",         include_str!("../../../../docs/grafana-solar_pv_dashboard.json")),
            // 15 dashboards Grafana (contrib/grafana/dashboards/) — IDs préfixés
            // pour éviter la collision avec ess (1..106) et entre eux.
            ("bms",      include_str!("../../../../contrib/grafana/dashboards/01-bms.json")),
            ("et112",    include_str!("../../../../contrib/grafana/dashboards/02-et112.json")),
            ("mppt",     include_str!("../../../../contrib/grafana/dashboards/03-mppt.json")),
            ("shunt",    include_str!("../../../../contrib/grafana/dashboards/04-smartshunt.json")),
            ("inv",      include_str!("../../../../contrib/grafana/dashboards/05-onduleur.json")),
            ("temp",     include_str!("../../../../contrib/grafana/dashboards/06-temperatures-venus.json")),
            ("heat",     include_str!("../../../../contrib/grafana/dashboards/07-heatpumps.json")),
            ("solar",    include_str!("../../../../contrib/grafana/dashboards/08-solaire.json")),
            ("irr",      include_str!("../../../../contrib/grafana/dashboards/09-irradiance.json")),
            ("ats",      include_str!("../../../../contrib/grafana/dashboards/10-ats.json")),
            ("tasmota",  include_str!("../../../../contrib/grafana/dashboards/11-tasmota.json")),
            ("shelly",   include_str!("../../../../contrib/grafana/dashboards/12-shelly.json")),
            ("wh",       include_str!("../../../../contrib/grafana/dashboards/13-chauffe-eau.json")),
            ("pi5",      include_str!("../../../../contrib/grafana/dashboards/14-pi5.json")),
            ("em",       include_str!("../../../../contrib/grafana/dashboards/15-energy-manager.json")),
        ];

        // Chaque dashboard source a sa propre grille (24 colonnes, y commençant à 0).
        // Pour éviter la superposition dans /dashboard/history, on décale les y de
        // chaque dashboard pour qu'il vienne APRÈS le précédent. On laisse 1 row
        // d'espace entre deux dashboards.
        let mut all: Vec<Panel> = Vec::new();
        let mut y_offset: u32 = 0;
        for (prefix, src) in SOURCES {
            match grafana::parse_dashboard(src) {
                Ok(mut panels) => {
                    if !prefix.is_empty() {
                        for p in panels.iter_mut() {
                            p.id = format!("{}.{}", prefix, p.id);
                        }
                    }
                    // Décale chaque panel et calcule la nouvelle borne max y.
                    let mut max_y_in_source: u32 = 0;
                    for p in panels.iter_mut() {
                        p.grid_pos.y = p.grid_pos.y.saturating_add(y_offset);
                        let bottom = p.grid_pos.y.saturating_add(p.grid_pos.h);
                        if bottom > max_y_in_source {
                            max_y_in_source = bottom;
                        }
                    }
                    tracing::info!(count = panels.len(), source = prefix, y_offset,
                                   "Catalogue : panels chargés");
                    all.append(&mut panels);
                    // +1 row d'espace entre sources.
                    y_offset = max_y_in_source.saturating_add(1);
                }
                Err(e) => {
                    tracing::error!(error = %e, source = prefix,
                                    "Catalogue : parsing échoué — source ignorée");
                }
            }
        }

        // Détection de collision d'ID (les dashboards sources doivent rester
        // disjoints). On garde la première occurrence et on logue les doublons.
        let mut seen = std::collections::HashSet::new();
        all.retain(|p| {
            if seen.insert(p.id.clone()) {
                true
            } else {
                tracing::warn!(id = %p.id, title = %p.title,
                               "Catalogue : ID dupliqué entre sources — panel ignoré");
                false
            }
        });

        tracing::info!(total = all.len(), "Catalogue de panels prêt");
        Self { panels: Arc::new(all) }
    }

    pub fn panels(&self) -> &[Panel] {
        &self.panels
    }

    pub fn find(&self, id: &str) -> Option<&Panel> {
        self.panels.iter().find(|p| p.id == id)
    }
}

#[cfg(test)]
mod load_tests {
    use super::*;

    #[test]
    fn load_default_includes_all_sources() {
        let cat = Catalog::load_default();
        let panels = cat.panels();

        // ess (~45) + solar_pv (11) + 15 dashboards (bms 23 + et112 12 + mppt 10
        // + shunt 15 + inv 14 + temp 7 + heat 9 + solar 10 + irr 4 + ats 22
        // + tasmota 12 + shelly 13 + wh 6 + pi5 16 + em 13) >> 200
        assert!(panels.len() >= 200, "trop peu de panels: {}", panels.len());

        // IDs uniques après préfixage
        let mut seen = std::collections::HashSet::new();
        for p in panels {
            assert!(seen.insert(&p.id), "ID dupliqué: {}", p.id);
        }

        // Au moins un panel de chaque nouveau préfixe (15 dashboards)
        for prefix in &[
            "bms.", "et112.", "mppt.", "shunt.", "inv.", "temp.", "heat.",
            "solar.", "irr.", "ats.", "tasmota.", "shelly.", "wh.", "pi5.", "em.",
        ] {
            assert!(
                panels.iter().any(|p| p.id.starts_with(prefix)),
                "aucun panel avec préfixe '{}'", prefix
            );
        }
    }
}
