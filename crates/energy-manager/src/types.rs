use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Incoming MQTT message dispatched to all logic tasks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MqttIncoming {
    pub topic: String,
    pub payload: bytes::Bytes,
    pub retain: bool,
}

impl MqttIncoming {
    pub fn payload_str(&self) -> &str {
        std::str::from_utf8(&self.payload).unwrap_or("")
    }

    /// Parse `{"value": <T>}` envelope used by Victron MQTT topics.
    pub fn victron_value<'de, T: Deserialize<'de>>(&self) -> Option<T> {
        #[derive(Deserialize)]
        struct Wrapper<T> { value: T }
        serde_json::from_slice::<Wrapper<T>>(&self.payload)
            .ok()
            .map(|w| w.value)
    }

    pub fn json<'de, T: Deserialize<'de>>(&self) -> Option<T> {
        serde_json::from_slice(&self.payload).ok()
    }
}

// ---------------------------------------------------------------------------
// Outgoing MQTT publish request
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MqttOutgoing {
    pub topic: String,
    pub payload: String,
    pub retain: bool,
    pub qos: MqttQos,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum MqttQos {
    AtMostOnce,
    AtLeastOnce,
}

impl MqttOutgoing {
    pub fn retained(topic: impl Into<String>, payload: impl Serialize) -> Self {
        Self {
            topic: topic.into(),
            payload: serde_json::to_string(&payload).unwrap_or_default(),
            retain: true,
            qos: MqttQos::AtLeastOnce,
        }
    }

    pub fn transient(topic: impl Into<String>, payload: impl Serialize) -> Self {
        Self {
            topic: topic.into(),
            payload: serde_json::to_string(&payload).unwrap_or_default(),
            retain: false,
            qos: MqttQos::AtLeastOnce,
        }
    }

    pub fn raw(topic: impl Into<String>, payload: impl Into<String>, retain: bool) -> Self {
        Self {
            topic: topic.into(),
            payload: payload.into(),
            retain,
            qos: MqttQos::AtLeastOnce,
        }
    }
}

// ---------------------------------------------------------------------------
// Live WebSocket event (broadcast to connected clients)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct LiveEvent {
    pub stream: String,
    pub ts: DateTime<Utc>,
    pub data: serde_json::Value,
}

impl LiveEvent {
    pub fn new(stream: impl Into<String>, data: impl Serialize) -> Self {
        Self {
            stream: stream.into(),
            ts: Utc::now(),
            data: serde_json::to_value(data).unwrap_or(serde_json::Value::Null),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared application state (behind Arc<RwLock<>>)
// ---------------------------------------------------------------------------
// Some fields are written by logic tasks but not yet read by any consumer
// (reserved for future API exposure). Suppress the lint globally on the struct.
#[allow(dead_code)]

#[derive(Debug, Default, Clone)]
pub struct EnergyState {
    // --- Solar / PV ---
    pub mppt_power_273_w: Option<f64>,
    pub mppt_power_289_w: Option<f64>,
    pub pvinverter_power_w: Option<f64>,
    pub solar_total_w: f64,
    pub house_power_w: Option<f64>,

    // --- MPPT detail ---
    pub mppt_273: MpptState,
    pub mppt_289: MpptState,

    // --- Battery ---
    pub soc_pct: Option<f64>,
    pub battery_current_a: Option<f64>,
    pub battery_voltage_v: Option<f64>,
    pub battery_power_w: Option<f64>,
    pub battery_state: Option<String>,
    pub time_to_go_sec: Option<i64>,

    // --- Grid / AC ---
    pub ac_ignore: Option<i64>, // IgnoreAcIn1: 0=grid, 1=off-grid
    pub ac_connected: Option<i64>, // ActiveIn/Connected
    pub ac_frequency_hz: Option<f64>,

    // --- VEBus (inverter) ---
    pub dc_voltage_v: Option<f64>,
    pub dc_current_a: Option<f64>,
    pub dc_power_w: Option<f64>,
    pub ac_out_voltage_v: Option<f64>,
    pub ac_out_current_a: Option<f64>,
    pub ac_out_power_w: Option<f64>,
    pub vebus_state: Option<String>,

    // --- Water heater (LG ThinQ) ---
    pub water_heater_mode: WaterHeaterMode,
    pub water_heater_temp_c: Option<f64>,
    pub water_heater_target_c: Option<f64>,
    pub water_heater_last_change: Option<DateTime<Utc>>,

    // --- DEYE relay (Shelly) ---
    pub deye_on: bool,
    pub deye_last_change: Option<DateTime<Utc>>,
    pub deye_lockout_until: Option<DateTime<Utc>>,

    // --- Irradiance ---
    pub irradiance_wm2: Option<f64>,

    // --- Weather (Open-Meteo) ---
    pub temperature_c: Option<f64>,
    pub humidity_pct: Option<f64>,
    pub pressure_hpa: Option<f64>,
    pub wind_speed_ms: Option<f64>,

    // --- Solar production counters ---
    pub mppt_yield_today_kwh: f64,
    pub pvinv_yield_today_kwh: f64,
    pub pvinv_baseline_kwh: Option<f64>, // ET112 cumulative counter at start of day
    pub pvinv_baseline_day: i32, // day ordinal when baseline was set (reset at midnight)
    pub total_yield_today_kwh: f64,
    pub yield_yesterday_kwh: f64,

    // --- Tasmota water heater relay ---
    pub tasmota_wh_on: bool,
    pub tasmota_wh_power_w: Option<f64>,
    pub tasmota_wh_energy_today_kwh: Option<f64>,

    // --- ATS switch ---
    pub ats_position: i64, // 0=réseau, 1=génératrice
    pub ats_state: i64, // 0=inactif, 1=actif, 2=alerte

    // --- Platform backup status ---
    pub platform_backup_status: i64, // 0=idle, 1=running, 2=ok, 3=error

    // --- Charge current (last published) ---
    pub last_charge_current_a: Option<f64>,
    pub last_power_assist: Option<bool>,

    // --- SmartShunt Ah accumulators (backup: current integration, reset at midnight) ---
    pub ah_charged_today: f64,
    pub ah_discharged_today: f64,
    pub ah_last_ts: Option<DateTime<Utc>>,
    pub ah_last_day: i32,

    // --- SmartShunt kWh from native History/ChargedEnergy & DischargedEnergy ---
    pub shunt_charged_today_kwh: f64,
    pub shunt_discharged_today_kwh: f64,
    pub shunt_charged_baseline_kwh: Option<f64>,
    pub shunt_discharged_baseline_kwh: Option<f64>,
    pub shunt_charged_day: i32,
    pub shunt_discharged_day: i32,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct MpptState {
    pub instance: u32,
    pub power_w: Option<f64>,
    pub pv_voltage_v: Option<f64>,
    pub dc_current_a: Option<f64>,
    pub yield_today_kwh: Option<f64>,
    pub state: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WaterHeaterMode {
    #[default]
    Vacation,
    HeatPump,
    Turbo,
}

impl WaterHeaterMode {
    pub fn to_venus_state(self) -> i64 {
        match self {
            WaterHeaterMode::Vacation => 0,
            WaterHeaterMode::HeatPump => 1,
            WaterHeaterMode::Turbo => 2,
        }
    }

    pub fn from_lg_str(s: &str) -> Self {
        match s {
            "HEAT_PUMP" => WaterHeaterMode::HeatPump,
            "TURBO"     => WaterHeaterMode::Turbo,
            _           => WaterHeaterMode::Vacation,
        }
    }

    /// Convertit l'enum en chaîne LG API pour les commandes
    /// Format de sortie : SCREAMING_SNAKE_CASE (convention LG ThinQ)
    pub fn to_lg_str(self) -> &'static str {
        match self {
            WaterHeaterMode::HeatPump => "HEAT_PUMP",
            WaterHeaterMode::Turbo => "TURBO",
            WaterHeaterMode::Vacation => "VACATION",
        }
    }
}
