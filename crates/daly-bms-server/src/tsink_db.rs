//! Wrapper asynchrone pour TsinkDB — stockage time-series embarqué.
//!
//! Remplace le bridge InfluxDB par un stockage local sans dépendance externe.
//! L'API PromQL expose l'historique via `/api/v1/query` et `/api/v1/query_range`.

use std::sync::Arc;
use std::time::Duration;
use tsink::{
    AsyncStorage, AsyncStorageBuilder, DataPoint, Label, Row, TimestampPrecision,
};
use tsink::promql::{Engine, PromqlValue};
use tracing::info;

use crate::config::TsinkConfig;
use daly_bms_core::types::BmsSnapshot;
use crate::et112::Et112Snapshot;
use crate::irradiance::IrradianceSnapshot;
use crate::state::{VenusSmartShunt, VenusInverter};

// =============================================================================
// Erreur
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum TsinkError {
    #[error("Tsink error: {0}")]
    Storage(#[from] tsink::TsinkError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("PromQL query error: {0}")]
    PromQL(String),
    #[error("Task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
}

pub type Result<T> = std::result::Result<T, TsinkError>;

// =============================================================================
// TsinkHandle
// =============================================================================

/// Handle clonable vers le stockage Tsink.
#[derive(Clone)]
pub struct TsinkHandle {
    storage: Arc<AsyncStorage>,
}

impl TsinkHandle {
    /// Crée et initialise le stockage Tsink à partir de la configuration.
    pub async fn new(config: &TsinkConfig) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&config.data_path)?;

        let storage = AsyncStorageBuilder::new()
            .with_data_path(&config.data_path)
            .with_timestamp_precision(TimestampPrecision::Milliseconds)
            .with_retention(Duration::from_secs(config.retention_days * 24 * 3600))
            .with_memory_limit(config.memory_limit_mb * 1024 * 1024)
            .with_cardinality_limit(config.cardinality_limit)
            .build()?;

        info!(
            path = %config.data_path,
            retention_days = config.retention_days,
            "Tsink initialisé"
        );

        Ok(Self {
            storage: Arc::new(storage),
        })
    }

    // -------------------------------------------------------------------------
    // Écriture
    // -------------------------------------------------------------------------

    /// Écrit un batch de lignes dans Tsink (non-bloquant).
    pub async fn write_rows(&self, rows: Vec<Row>) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        self.storage
            .insert_rows(rows)
            .await
            .map_err(TsinkError::Storage)
    }

    // -------------------------------------------------------------------------
    // Requêtes PromQL
    // -------------------------------------------------------------------------

    /// Requête PromQL instantanée — `/api/v1/query`
    pub async fn query_instant(&self, query: String, time_ms: i64) -> Result<PromqlValue> {
        let sync_storage = self.storage.inner();
        tokio::task::spawn_blocking(move || {
            Engine::with_precision(sync_storage, TimestampPrecision::Milliseconds)
                .instant_query(&query, time_ms)
                .map_err(|e| TsinkError::PromQL(e.to_string()))
        })
        .await
        .map_err(TsinkError::Join)?
    }

    /// Requête PromQL sur plage temporelle — `/api/v1/query_range`
    pub async fn query_range(
        &self,
        query: String,
        start_ms: i64,
        end_ms: i64,
        step_ms: i64,
    ) -> Result<PromqlValue> {
        let sync_storage = self.storage.inner();
        tokio::task::spawn_blocking(move || {
            Engine::with_precision(sync_storage, TimestampPrecision::Milliseconds)
                .range_query(&query, start_ms, end_ms, step_ms)
                .map_err(|e| TsinkError::PromQL(e.to_string()))
        })
        .await
        .map_err(TsinkError::Join)?
    }

    // -------------------------------------------------------------------------
    // Stats
    // -------------------------------------------------------------------------

    pub fn memory_used_bytes(&self) -> usize {
        self.storage.memory_used()
    }

    pub fn memory_budget_bytes(&self) -> usize {
        self.storage.memory_budget()
    }

    // -------------------------------------------------------------------------
    // Conversions snapshots → Rows
    // -------------------------------------------------------------------------

    /// Convertit un BmsSnapshot en lignes Tsink.
    pub fn bms_rows(snap: &BmsSnapshot) -> Vec<Row> {
        let ts = snap.timestamp.timestamp_millis();
        let bms_id = format!("{:#04x}", snap.address);

        let mut rows = Vec::with_capacity(10 + snap.voltages.len());

        macro_rules! row {
            ($metric:expr, $value:expr) => {
                Row::with_labels(
                    $metric,
                    vec![Label::new("bms_id", bms_id.as_str())],
                    DataPoint::new(ts, $value as f64),
                )
            };
        }

        rows.push(row!("bms_voltage",      snap.dc.voltage));
        rows.push(row!("bms_current",      snap.dc.current));
        rows.push(row!("bms_power",        snap.dc.power));
        rows.push(row!("bms_soc",          snap.soc));
        rows.push(row!("bms_capacity_ah",  snap.capacity));
        rows.push(row!("bms_cell_delta_mv",snap.system.cell_delta_mv()));
        rows.push(row!("bms_temp_max",     snap.system.max_cell_temperature));
        rows.push(row!("bms_temp_min",     snap.system.min_cell_temperature));
        rows.push(row!("bms_charge_mos",   snap.io.allow_to_charge));
        rows.push(row!("bms_discharge_mos",snap.io.allow_to_discharge));

        for (cell_name, &v) in &snap.voltages {
            rows.push(Row::with_labels(
                "bms_cell_voltage",
                vec![
                    Label::new("bms_id", bms_id.as_str()),
                    Label::new("cell",   cell_name.as_str()),
                ],
                DataPoint::new(ts, v as f64),
            ));
        }

        rows
    }

    /// Convertit un Et112Snapshot en lignes Tsink.
    pub fn et112_rows(snap: &Et112Snapshot) -> Vec<Row> {
        let ts  = snap.timestamp.timestamp_millis();
        let addr = format!("{:#04x}", snap.address);

        macro_rules! row {
            ($metric:expr, $value:expr) => {
                Row::with_labels(
                    $metric,
                    vec![
                        Label::new("address", addr.as_str()),
                        Label::new("name",    snap.name.as_str()),
                    ],
                    DataPoint::new(ts, $value as f64),
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

    /// Convertit un IrradianceSnapshot en lignes Tsink.
    pub fn irradiance_rows(snap: &IrradianceSnapshot) -> Vec<Row> {
        let ts = snap.timestamp.timestamp_millis();
        vec![Row::with_labels(
            "irradiance_wm2",
            vec![Label::new("address", format!("{:#04x}", snap.address).as_str())],
            DataPoint::new(ts, snap.irradiance_wm2 as f64),
        )]
    }

    /// Écrit les données Venus SmartShunt dans Tsink.
    pub fn smartshunt_rows(shunt: &VenusSmartShunt) -> Vec<Row> {
        let ts = shunt.timestamp.timestamp_millis();

        let mut rows = Vec::new();
        macro_rules! push_opt {
            ($metric:expr, $opt:expr) => {
                if let Some(v) = $opt {
                    rows.push(Row::new($metric, DataPoint::new(ts, v as f64)));
                }
            };
        }

        push_opt!("venus_shunt_voltage_v",          shunt.voltage_v);
        push_opt!("venus_shunt_current_a",          shunt.current_a);
        push_opt!("venus_shunt_power_w",            shunt.power_w);
        push_opt!("venus_shunt_soc_percent",        shunt.soc_percent);
        push_opt!("venus_shunt_energy_in_kwh",      shunt.energy_in_kwh);
        push_opt!("venus_shunt_energy_out_kwh",     shunt.energy_out_kwh);
        push_opt!("venus_shunt_ah_charged_today",   shunt.ah_charged_today);
        push_opt!("venus_shunt_ah_discharged_today",shunt.ah_discharged_today);

        rows
    }

    /// Écrit les données Venus Inverter dans Tsink.
    pub fn inverter_rows(inv: &VenusInverter) -> Vec<Row> {
        let ts = inv.timestamp.timestamp_millis();

        let mut rows = Vec::new();
        macro_rules! push_opt {
            ($metric:expr, $opt:expr) => {
                if let Some(v) = $opt {
                    rows.push(Row::new($metric, DataPoint::new(ts, v as f64)));
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
