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
    pub fn victron_value<T: for<'de> Deserialize<'de>>(&self) -> Option<T> {
        #[derive(Deserialize)]
        struct Wrapper<T> { value: T }
        serde_json::from_slice::<Wrapper<T>>(&self.payload)
            .ok()
            .map(|w| w.value)
    }

    pub fn json<T: for<'de> Deserialize<'de>>(&self) -> Option<T> {
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
// Shared application state (behind Arc<RwLock<EnergyState>>)
// ---------------------------------------------------------------------------
// Some fields are written by logic tasks but not yet read by any consumer
// (reserved for future API exposure). Suppress the lint globally on the struct.
#[allow(dead_code)]

#[derive(Debug, Default, Clone)]
pub struct EnergyState {
    // --- Solar / PV ---
    pub mppt_power_273_w: Option<f64>,
    pub mppt_power_289_w: Option<f64>,
    pub dc_pv_power_w: Option<f64>,     // N/.../system/0/Dc/Pv/Power (aggregate MPPT)
    pub pvinverter_power_w: Option<f64>, // N/.../pvinverter/32/Ac/Power
    pub solar_total_w: f64,
    pub house_power_w: Option<f64>,
    // N/.../system/0/Ac/PvOnOutput/L1/Power — AC PV available on inverter output.
    // Used by charge_current to decide PV-excess mode; do not confuse with mppt_power_*_w (DC).
    pub ac_pv_on_output_w: Option<f64>,

    // --- MPPT detail ---
    pub mppt_273: MpptState,
    pub mppt_289: MpptState,

    // --- Battery ---
    pub soc_pct: Option<f64>,
    pub battery_current_a: Option<f64>,
    pub battery_voltage_v: Option<f64>,
    pub battery_power_w: Option<f64>,
    pub battery_state: Option<i64>,
    pub time_to_go_sec: Option<i64>,
    // Timestamp of the last direct SmartShunt topic (battery/{shunt}/Dc/0/*, /Soc, /TimeToGo, /State).
    // Used to gate fallbacks from VEBus / system aggregates: while shunt data is fresh,
    // those secondary sources must not overwrite the authoritative shunt values.
    pub shunt_last_seen_ts: Option<DateTime<Utc>>,

    // --- Grid / AC ---
    pub ac_ignore: Option<i64>,         // IgnoreAcIn1: 0=grid, 1=off-grid
    pub ac_connected: Option<i64>,      // ActiveIn/Connected
    pub ac_frequency_hz: Option<f64>,
    /// Timestamp of the last AC-Out frequency update — freshness guard for the DEYE decision.
    pub ac_frequency_last_ts: Option<DateTime<Utc>>,

    // --- VEBus (inverter) ---
    pub dc_voltage_v: Option<f64>,
    pub dc_current_a: Option<f64>,
    pub dc_power_w: Option<f64>,
    pub ac_out_voltage_v: Option<f64>,
    pub ac_out_current_a: Option<f64>,
    pub ac_out_power_w: Option<f64>,
    pub vebus_state: Option<i64>,

    // --- Water heater (LG ThinQ) ---
    pub water_heater_mode: WaterHeaterMode,
    pub water_heater_temp_c: Option<f64>,
    pub water_heater_target_c: Option<f64>,
    pub water_heater_last_change: Option<DateTime<Utc>>,
    pub water_heater_last_read: Option<DateTime<Utc>>,
    pub water_heater_send_count: u32,
    /// Horodatage depuis lequel la température de la cuve atteint/dépasse le
    /// seuil `temp_max_c`. None tant qu'elle est en-dessous. Sert à forcer le
    /// passage en VACATION après `temp_max_hold_secs` (cible 60°C atteinte).
    pub water_heater_temp_max_since: Option<DateTime<Utc>>,

    // --- DEYE relay (Shelly) ---
    pub deye_on: bool,
    pub deye_last_change: Option<DateTime<Utc>>,
    pub deye_lockout_until: Option<DateTime<Utc>>,
    /// Persisted DEYE state from retained MQTT (set by persist watcher at startup)
    pub deye_persisted_state: Option<String>,
    /// Current state-machine state name (On/PendingCut/Lockout/Off/PendingRestore) — observability.
    pub deye_state: Option<String>,
    /// Whether DEYE restore is currently held off by the structural-excess guard — observability.
    pub deye_restore_blocked: bool,
    /// Whether the MPPT charge stage signals a full battery (the MPPT-based cut driver) — observability.
    pub deye_mppt_full: bool,
    /// AC-Out frequency telemetry is stale (topic silent > input_max_age_secs) — observability.
    /// On stale freq the decision treats it as nominal (restore allowed; DEYE 51.5 Hz auto-trip is the net).
    pub deye_freq_stale: bool,
    /// MPPT State telemetry is stale (topic silent > input_max_age_secs) — observability.
    /// On stale MPPT the decision treats the battery as NOT full (does not strand the relay off).
    pub deye_mppt_stale: bool,

    // --- Irradiance ---
    pub irradiance_wm2: Option<f64>,

    // --- Weather (Open-Meteo) ---
    pub temperature_c: Option<f64>,
    pub humidity_pct: Option<f64>,
    pub pressure_hpa: Option<f64>,
    pub wind_speed_ms: Option<f64>,
    /// Horodatage du dernier fetch Open-Meteo réussi (audit 2026-06 §18) —
    /// permet d'exporter l'âge de la donnée météo et d'alerter si elle
    /// devient périmée (API down → la logique tournerait sur du stale).
    pub weather_last_read: Option<DateTime<Utc>>,

    // --- Solar production counters ---
    pub mppt_yield_today_kwh: f64,
    pub pvinv_yield_today_kwh: f64,
    pub pvinv_baseline_kwh: Option<f64>,   // ET112 cumulative counter at start of day
    pub pvinv_baseline_day: i32,           // day ordinal when baseline was set (reset at midnight)
    pub total_yield_today_kwh: f64,
    pub yield_yesterday_kwh: f64,

    // --- Tasmota water heater relay ---
    pub tasmota_wh_on: bool,
    pub tasmota_wh_power_w: Option<f64>,
    pub tasmota_wh_energy_today_kwh: Option<f64>,

    // --- ATS switch ---
    pub ats_position: i64,  // 0=réseau, 1=génératrice
    pub ats_state: i64,     // 0=inactif, 1=actif, 2=alerte

    // --- Platform backup status ---
    pub platform_backup_status: i64,  // 0=idle, 1=running, 2=ok, 3=error

    // --- Charge current (last published) ---
    pub last_charge_current_a: Option<f64>,
    pub last_power_assist: Option<i64>,
    pub last_charge_ts: Option<DateTime<Utc>>,

    // --- SmartShunt Ah accumulators (backup: current integration, reset at midnight) ---
    pub ah_charged_today: f64,
    pub ah_discharged_today: f64,
    pub ah_last_ts: Option<DateTime<Utc>>,
    pub ah_last_day: i32,

    // --- SmartShunt kWh from native History/ChargedEnergy & DischargedEnergy ---
    pub shunt_charged_today_kwh:         f64,
    pub shunt_discharged_today_kwh:      f64,
    pub shunt_charged_baseline_kwh:      Option<f64>,
    pub shunt_discharged_baseline_kwh:   Option<f64>,
    pub shunt_charged_day:               i32,
    pub shunt_discharged_day:            i32,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct MpptState {
    pub instance: u32,
    pub power_w: Option<f64>,
    pub pv_voltage_v: Option<f64>,
    pub dc_current_a: Option<f64>,
    pub yield_today_kwh: Option<f64>,
    pub max_power_today_w: Option<f64>,
    pub state: Option<i64>,
    /// Timestamp of the last `/State` update — freshness guard for the DEYE decision
    /// (a frozen State must not strand the relay; see deye_command).
    pub state_last_ts: Option<DateTime<Utc>>,
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
            WaterHeaterMode::Vacation  => 0,
            WaterHeaterMode::HeatPump  => 1,
            WaterHeaterMode::Turbo     => 2,
        }
    }

    pub fn from_lg_str(s: &str) -> Self {
        match s {
            "HEAT_PUMP" => WaterHeaterMode::HeatPump,
            "TURBO"     => WaterHeaterMode::Turbo,
            _           => WaterHeaterMode::Vacation,
        }
    }

    pub fn to_lg_str(self) -> &'static str {
        match self {
            WaterHeaterMode::HeatPump => "HEAT_PUMP",
            WaterHeaterMode::Turbo    => "TURBO",
            WaterHeaterMode::Vacation => "VACATION",
        }
    }
}
