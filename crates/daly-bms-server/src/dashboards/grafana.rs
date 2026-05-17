//! Parser Grafana JSON → liste de `Panel` normalisés.
//!
//! On extrait uniquement les champs nécessaires à l'affichage :
//! id, title, type, gridPos, targets[].expr, fieldConfig.defaults.unit.
//!
//! Les panels imbriqués dans un row collapsed (`panels: [...]`) sont
//! aplatis dans la liste principale.

use super::{GridPos, Panel, PanelKind, PanelQuery};
use serde_json::Value;

/// Parse un JSON Grafana et retourne la liste de panels.
pub fn parse_dashboard(json: &str) -> anyhow::Result<Vec<Panel>> {
    let root: Value = serde_json::from_str(json)?;
    let raw_panels = root.get("panels")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("dashboard JSON: champ 'panels' absent ou non-array"))?;

    let mut out = Vec::with_capacity(raw_panels.len());
    for p in raw_panels {
        collect_panel(p, &mut out);
    }
    Ok(out)
}

/// Ajoute le panel + ses sous-panels (cas row collapsed) à `out`.
fn collect_panel(p: &Value, out: &mut Vec<Panel>) {
    if let Some(panel) = parse_one(p) {
        out.push(panel);
    }
    if let Some(sub) = p.get("panels").and_then(|v| v.as_array()) {
        for child in sub {
            collect_panel(child, out);
        }
    }
}

fn parse_one(p: &Value) -> Option<Panel> {
    let id = p.get("id")?.as_u64().map(|n| n.to_string())?;
    let title = p.get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let raw_type = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let kind = PanelKind::from_grafana(raw_type);

    let grid_pos = p.get("gridPos")
        .map(parse_gridpos)
        .unwrap_or_default();

    // Unit : fieldConfig.defaults.unit
    let unit = p
        .pointer("/fieldConfig/defaults/unit")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let decimals = p
        .pointer("/fieldConfig/defaults/decimals")
        .and_then(|v| v.as_u64())
        .map(|n| n.min(10) as u8);

    let queries = p.get("targets")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_target).collect::<Vec<_>>())
        .unwrap_or_default();

    Some(Panel {
        id,
        title,
        kind,
        grid_pos,
        unit,
        queries,
        decimals,
    })
}

fn parse_gridpos(v: &Value) -> GridPos {
    GridPos {
        x: v.get("x").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
        y: v.get("y").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
        w: v.get("w").and_then(|n| n.as_u64()).unwrap_or(12) as u32,
        h: v.get("h").and_then(|n| n.as_u64()).unwrap_or(8) as u32,
    }
}

fn parse_target(t: &Value) -> Option<PanelQuery> {
    let expr = t.get("expr").and_then(|v| v.as_str())?.to_string();
    if expr.is_empty() {
        return None;
    }
    let ref_id = t.get("refId").and_then(|v| v.as_str()).unwrap_or("A").to_string();
    let legend = t.get("legendFormat")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    Some(PanelQuery { ref_id, expr, legend })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal() {
        let json = r#"{
            "panels": [
                {
                    "id": 1,
                    "title": "Solar",
                    "type": "timeseries",
                    "gridPos": {"x": 0, "y": 0, "w": 12, "h": 8},
                    "fieldConfig": {"defaults": {"unit": "watt"}},
                    "targets": [
                        {"refId": "A", "expr": "solar_total_w", "legendFormat": "PV"}
                    ]
                }
            ]
        }"#;
        let panels = parse_dashboard(json).unwrap();
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].id, "1");
        assert_eq!(panels[0].title, "Solar");
        assert_eq!(panels[0].kind, PanelKind::TimeSeries);
        assert_eq!(panels[0].unit, "watt");
        assert_eq!(panels[0].queries.len(), 1);
        assert_eq!(panels[0].queries[0].expr, "solar_total_w");
    }

    #[test]
    fn parse_real_grafana_ess() {
        const SRC: &str = include_str!("../../../../docs/grafana-ess_dashboard.json");
        let panels = parse_dashboard(SRC).expect("parse échoué");
        assert!(panels.len() >= 20, "trop peu de panels parsés: {}", panels.len());
        // Doit contenir au moins un row, un stat et un timeseries
        assert!(panels.iter().any(|p| p.kind == PanelKind::Row));
        assert!(panels.iter().any(|p| p.kind == PanelKind::Stat));
        assert!(panels.iter().any(|p| p.kind == PanelKind::TimeSeries));
    }

    #[test]
    fn parse_solar_pv_dashboard() {
        const SRC: &str = include_str!("../../../../docs/grafana-solar_pv_dashboard.json");
        let panels = parse_dashboard(SRC).expect("parse échoué");
        // 11 panels attendus (IDs 1001..=1011)
        assert_eq!(panels.len(), 11, "nombre de panels Solar PV inattendu");
        // IDs disjoints du dashboard ess (qui va de 1 à 106)
        for p in &panels {
            let id: u32 = p.id.parse().expect("id non numérique");
            assert!(id >= 1000, "id {} entre en collision avec ess_dashboard", id);
        }
        // Au moins un timeseries, un bargauge, un gauge
        assert!(panels.iter().any(|p| p.kind == PanelKind::TimeSeries));
        assert!(panels.iter().any(|p| p.kind == PanelKind::BarGauge));
        assert!(panels.iter().any(|p| p.kind == PanelKind::Gauge));
        // Chaque panel doit avoir au moins une query non-vide
        for p in &panels {
            assert!(!p.queries.is_empty(), "panel {} sans query", p.id);
            for q in &p.queries {
                assert!(!q.expr.is_empty(), "panel {} query vide", p.id);
            }
        }
    }
}
