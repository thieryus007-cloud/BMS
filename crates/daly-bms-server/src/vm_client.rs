//! Client HTTP pour VictoriaMetrics — remplace Tsink embarqué.
//!
//! Écriture : Prometheus text format → POST /api/v1/import/prometheus
//! Lecture  : API PromQL standard   → GET /api/v1/query[_range]

use std::time::Duration;
use reqwest::Client;
use serde_json::Value;
use tracing::{info, warn};

use crate::config::VmConfig;
use daly_bms_core::types::BmsSnapshot;
use crate::et112::Et112Snapshot;
use crate::irradiance::IrradianceSnapshot;
use crate::state::{VenusSmartShunt, VenusInverter};

// =============================================================================
// VmRow — métrique en format Prometheus text
// =============================================================================

pub struct VmRow {
    metric:       String,
    labels:       Vec<(String, String)>,
    value:        f64,
    timestamp_ms: i64,
}

impl VmRow {
    pub fn new(metric: &str, value: f64, timestamp_ms: i64) -> Self {
        Self {
            metric: metric.to_string(),
            labels: Vec::new(),
            value,
            timestamp_ms,
        }
    }

    pub fn with_labels(metric: &str, labels: Vec<(&str, &str)>, value: f64, timestamp_ms: i64) -> Self {
        Self {
            metric: metric.to_string(),
            labels: labels.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            value,
            timestamp_ms,
        }
    }

    /// Sérialise en Prometheus text format — timestamp en millisecondes (requis par /api/v1/import/prometheus).
    fn to_line(&self) -> String {
        if self.labels.is_empty() {
            format!("{} {} {}", self.metric, self.value, self.timestamp_ms)
        } else {
            let labels_str = self.labels.iter()
                .map(|(k, v)| format!("{}=\"{}\"", k, v))
                .collect::<Vec<_>>()
                .join(",");
            format!("{}{{{}}} {} {}", self.metric, labels_str, self.value, self.timestamp_ms)
        }
    }
}

// =============================================================================
// VmClient
// =============================================================================

/// Client clonable vers VictoriaMetrics (thread-safe via reqwest::Client interne).
#[derive(Clone)]
pub struct VmClient {
    http:     Client,
    base_url: String,
}

impl VmClient {
    pub fn new(config: &VmConfig) -> anyhow::Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()?;
        info!(url = %config.url, "VictoriaMetrics client initialisé");
        Ok(Self {
            http,
            base_url: config.url.trim_end_matches('/').to_string(),
        })
    }

    // -------------------------------------------------------------------------
    // Écriture
    // -------------------------------------------------------------------------

    /// Écrit un batch de métriques dans VictoriaMetrics (non-bloquant).
    pub async fn write_rows(&self, rows: Vec<VmRow>) -> anyhow::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let body = rows.iter().map(|r| r.to_line()).collect::<Vec<_>>().join("\n");
        let resp = self.http
            .post(format!("{}/api/v1/import/prometheus", self.base_url))
            .header("Content-Type", "text/plain")
            .body(body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "VictoriaMetrics write failed");
            return Err(anyhow::anyhow!("VM write error {}: {}", status, text));
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Lecture PromQL (retourne le JSON VM directement — format Prometheus standard)
    // -------------------------------------------------------------------------

    /// Requête PromQL instantanée. `time_ms` en millisecondes → converti en secondes.
    pub async fn query_instant_json(&self, query: &str, time_ms: i64) -> anyhow::Result<Value> {
        let time_secs = (time_ms as f64 / 1000.0).to_string();
        let resp = self.http
            .get(format!("{}/api/v1/query", self.base_url))
            .query(&[("query", query), ("time", &time_secs)])
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        Ok(resp)
    }

    /// Requête PromQL sur plage temporelle. Timestamps en ms → convertis en secondes.
    pub async fn query_range_json(
        &self,
        query:    &str,
        start_ms: i64,
        end_ms:   i64,
        step_ms:  i64,
    ) -> anyhow::Result<Value> {
        let start = (start_ms as f64 / 1000.0).to_string();
        let end   = (end_ms   as f64 / 1000.0).to_string();
        let step  = (step_ms  as f64 / 1000.0).to_string();
        let resp = self.http
            .get(format!("{}/api/v1/query_range", self.base_url))
            .query(&[("query", query), ("start", &start), ("end", &end), ("step", &step)])
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        Ok(resp)
    }

    // -------------------------------------------------------------------------
    // Conversions snapshots → VmRow (mêmes métriques qu'avec Tsink)
    // -------------------------------------------------------------------------

    pub fn bms_rows(snap: &BmsSnapshot) -> Vec<VmRow> {
        let ts     = snap.timestamp.timestamp_millis();
        let bms_id = format!("{:#04x}", snap.address);
        let id     = bms_id.as_str();

        let mut rows = Vec::with_capacity(10 + snap.voltages.len());

        macro_rules! row {
            ($metric:expr, $value:expr) => {
                VmRow::with_labels($metric, vec![("bms_id", id)], $value as f64, ts)
            };
        }

        rows.push(row!("bms_voltage",       snap.dc.voltage));
        rows.push(row!("bms_current",       snap.dc.current));
        rows.push(row!("bms_power",         snap.dc.power));
        rows.push(row!("bms_soc",           snap.soc));
        rows.push(row!("bms_capacity_ah",   snap.capacity));
        rows.push(row!("bms_cell_delta_mv", snap.system.cell_delta_mv()));
        rows.push(row!("bms_temp_max",      snap.system.max_cell_temperature));
        rows.push(row!("bms_temp_min",      snap.system.min_cell_temperature));
        rows.push(row!("bms_charge_mos",    snap.io.allow_to_charge));
        rows.push(row!("bms_discharge_mos", snap.io.allow_to_discharge));

        for (cell_name, &v) in &snap.voltages {
            rows.push(VmRow::with_labels(
                "bms_cell_voltage",
                vec![("bms_id", id), ("cell", cell_name.as_str())],
                v as f64,
                ts,
            ));
        }

        rows
    }

    pub fn et112_rows(snap: &Et112Snapshot) -> Vec<VmRow> {
        let ts   = snap.timestamp.timestamp_millis();
        let addr = format!("{:#04x}", snap.address);

        macro_rules! row {
            ($metric:expr, $value:expr) => {
                VmRow::with_labels(
                    $metric,
                    vec![("address", addr.as_str()), ("name", snap.name.as_str())],
                    $value as f64,
                    ts,
                )
            };
        }

        vec![
            row!("et112_voltage_v",         snap.voltage_v),
            row!("et112_current_a",         snap.current_a),
            row!("et112_power_w",           snap.power_w),
            row!("et112_apparent_power_va", snap.apparent_power_va),
            row!("et112_power_factor",      snap.power_factor),
            row!("et112_frequency_hz",      snap.frequency_hz),
            row!("et112_energy_import_wh",  snap.energy_import_wh),
            row!("et112_energy_export_wh",  snap.energy_export_wh),
        ]
    }

    pub fn irradiance_rows(snap: &IrradianceSnapshot) -> Vec<VmRow> {
        let ts   = snap.timestamp.timestamp_millis();
        let addr = format!("{:#04x}", snap.address);
        vec![VmRow::with_labels(
            "irradiance_wm2",
            vec![("address", addr.as_str())],
            snap.irradiance_wm2 as f64,
            ts,
        )]
    }

    pub fn smartshunt_rows(shunt: &VenusSmartShunt) -> Vec<VmRow> {
        let ts   = shunt.timestamp.timestamp_millis();
        let mut rows = Vec::new();

        macro_rules! push_opt {
            ($metric:expr, $opt:expr) => {
                if let Some(v) = $opt {
                    rows.push(VmRow::new($metric, v as f64, ts));
                }
            };
        }

        push_opt!("venus_shunt_voltage_v",           shunt.voltage_v);
        push_opt!("venus_shunt_current_a",           shunt.current_a);
        push_opt!("venus_shunt_power_w",             shunt.power_w);
        push_opt!("venus_shunt_soc_percent",         shunt.soc_percent);
        push_opt!("venus_shunt_energy_in_kwh",       shunt.energy_in_kwh);
        push_opt!("venus_shunt_energy_out_kwh",      shunt.energy_out_kwh);
        push_opt!("venus_shunt_ah_charged_today",    shunt.ah_charged_today);
        push_opt!("venus_shunt_ah_discharged_today", shunt.ah_discharged_today);

        rows
    }

    pub fn solar_rows(solar_total_w: f32, mppt_power_w: f32, total_yield_kwh: f32) -> Vec<VmRow> {
        let ts = chrono::Utc::now().timestamp_millis();
        vec![
            VmRow::new("solar_total_w",   solar_total_w as f64,   ts),
            VmRow::new("mppt_power_w",    mppt_power_w as f64,    ts),
            VmRow::new("solar_yield_kwh", total_yield_kwh as f64, ts),
        ]
    }

    pub fn inverter_rows(inv: &VenusInverter) -> Vec<VmRow> {
        let ts   = inv.timestamp.timestamp_millis();
        let mut rows = Vec::new();

        macro_rules! push_opt {
            ($metric:expr, $opt:expr) => {
                if let Some(v) = $opt {
                    rows.push(VmRow::new($metric, v as f64, ts));
                }
            };
        }

        push_opt!("venus_inverter_voltage_v",           inv.voltage_v);
        push_opt!("venus_inverter_current_a",           inv.current_a);
        push_opt!("venus_inverter_power_w",             inv.power_w);
        push_opt!("venus_inverter_ac_output_voltage_v", inv.ac_output_voltage_v);
        push_opt!("venus_inverter_ac_output_current_a", inv.ac_output_current_a);
        push_opt!("venus_inverter_ac_output_power_w",   inv.ac_output_power_w);

        rows
    }
}
