/// Central meteo pivot:
///  - Aggregates weather (Open-Meteo), irradiance, solar yields
///  - Publishes santuario/heat/1/venus and santuario/meteo/venus (retained)
///  - Persists solar baselines to MQTT retained
///  - Handles midnight reset (cron)
use chrono::{Local, NaiveTime};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, sleep, Duration};
use tracing::info;

use crate::bus::AppBus;
use crate::config::SolarConfig;
use crate::mqtt::topics::publish;
use crate::types::{EnergyState, MqttOutgoing};

const PUBLISH_INTERVAL_SECS: u64 = 60;

pub async fn spawn(
    solar_cfg: SolarConfig,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    let bus2   = bus.clone();
    let state2 = state.clone();
    let cfg2   = solar_cfg.clone();

    tokio::spawn(publish_task(bus2, state2));
    tokio::spawn(midnight_reset_task(cfg2, bus, state));
}

// ---------------------------------------------------------------------------
// Periodic publish of weather + solar data
// ---------------------------------------------------------------------------

async fn publish_task(bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    let mut ticker = interval(Duration::from_secs(PUBLISH_INTERVAL_SECS));
    loop {
        ticker.tick().await;
        publish_all(&bus, &state).await;
    }
}

pub async fn publish_all(bus: &AppBus, state: &Arc<RwLock<EnergyState>>) {
    let s = state.read().await;

    // santuario/heat/1/venus — temperature sensor for Venus OS
    let heat_payload = json!({
        "Temperature":     s.temperature_c,
        "Humidity":        s.humidity_pct,
        "Pressure":        s.pressure_hpa,
        "TemperatureType": 4,  // 4 = Outdoor
    });

    // santuario/meteo/venus — irradiance + solar + wind
    let meteo_payload = json!({
        "Irradiance":    s.irradiance_wm2,
        "TodaysYield":   s.total_yield_today_kwh,
        "YieldYesterday": s.yield_yesterday_kwh,
        "WindSpeed":     s.wind_speed_ms,
        "MpptPower":     s.mppt_273.power_w.unwrap_or(0.0) + s.mppt_289.power_w.unwrap_or(0.0),
        "SolarTotal":    s.solar_total_w,
        "Mppts": [
            {
                "Instance": 273u32,
                "State":    s.mppt_273.state,
                "PvVoltage": s.mppt_273.pv_voltage_v,
                "DcCurrent": s.mppt_273.dc_current_a,
                "Power":     s.mppt_273.power_w,
                "YieldToday": s.mppt_273.yield_today_kwh,
            },
            {
                "Instance": 289u32,
                "State":    s.mppt_289.state,
                "PvVoltage": s.mppt_289.pv_voltage_v,
                "DcCurrent": s.mppt_289.dc_current_a,
                "Power":     s.mppt_289.power_w,
                "YieldToday": s.mppt_289.yield_today_kwh,
            },
        ],
    });
    drop(s);

    bus.publish(MqttOutgoing::retained(publish::HEAT_VENUS, &heat_payload)).await;
    bus.publish(MqttOutgoing::retained(publish::METEO_VENUS, &meteo_payload)).await;
}

// ---------------------------------------------------------------------------
// Midnight reset + baseline persistence via MQTT retained
// ---------------------------------------------------------------------------

async fn midnight_reset_task(
    cfg: SolarConfig,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    loop {
        let wait = secs_until_midnight();
        info!("Midnight reset scheduled in {wait}s");
        sleep(Duration::from_secs(wait)).await;

        do_reset(&cfg, &bus, &state).await;
    }
}

async fn do_reset(_cfg: &SolarConfig, bus: &AppBus, state: &Arc<RwLock<EnergyState>>) {
    info!("Midnight reset triggered");

    let total_today = {
        let s = state.read().await;
        s.total_yield_today_kwh
    };

    // Update yield_yesterday + publish retained
    {
        let mut s = state.write().await;
        s.yield_yesterday_kwh   = total_today;
        s.total_yield_today_kwh = 0.0;
        s.mppt_yield_today_kwh  = 0.0;
        s.pvinv_yield_today_kwh = 0.0;
        s.pvinv_baseline_kwh    = None;   // will be set on next pvinverter message
        s.mppt_273.yield_today_kwh = Some(0.0);
        s.mppt_289.yield_today_kwh = Some(0.0);
    }

    // Publish yield_yesterday retained
    bus.publish(MqttOutgoing::raw(
        publish::YIELD_YESTERDAY,
        format!("{total_today:.3}"),
        true,
    )).await;

    // Clear pvinv_baseline retained (empty payload)
    bus.publish(MqttOutgoing::raw(publish::PVINV_BASELINE, "", true)).await;

    info!("Midnight reset complete — yesterday={total_today:.3} kWh");
}

fn secs_until_midnight() -> u64 {
    let now = Local::now();
    let midnight = NaiveTime::from_hms_opt(0, 0, 5).unwrap(); // 5s after midnight
    let now_time = now.time();
    let diff = midnight.signed_duration_since(now_time);
    let secs = diff.num_seconds();
    if secs <= 0 {
        (86400 + secs) as u64
    } else {
        secs as u64
    }
}
