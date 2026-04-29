/// Automatic management of the LG ThinQ water heater.
/// Switches between HEAT_PUMP and VACATION based on solar, SOC, grid and battery conditions.
/// Implements 5-minute debounce and 15-minute minimum interval between mode changes.
use chrono::{DateTime, Utc};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, sleep, Duration};
use tracing::{debug, info};

use crate::bus::AppBus;
use crate::config::WaterHeaterConfig;
use crate::http_clients::lg_thinq::LgThinqClient;
use crate::mqtt::topics::publish;
use crate::types::{EnergyState, LiveEvent, MqttOutgoing, WaterHeaterMode};

pub async fn spawn(
    cfg: WaterHeaterConfig,
    lg: Option<Arc<LgThinqClient>>,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    let bus2   = bus.clone();
    let state2 = state.clone();
    let cfg2   = cfg.clone();

    // Keepalive: republish last known state every 25s for Venus OS watchdog
    tokio::spawn(keepalive_task(cfg.keepalive_secs, bus2, state2));

    // Control logic
    if let Some(lg_client) = lg {
        tokio::spawn(control_task(cfg2, lg_client, bus, state));
    } else {
        info!("Water heater auto-control disabled (no LG ThinQ client)");
    }
}

// ---------------------------------------------------------------------------
// Keepalive — republish current mode to Venus OS every N seconds
// ---------------------------------------------------------------------------

async fn keepalive_task(
    interval_secs: u64,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    let mut ticker = interval(Duration::from_secs(interval_secs));
    loop {
        ticker.tick().await;
        publish_to_venus(&bus, &state).await;
    }
}

async fn publish_to_venus(bus: &AppBus, state: &Arc<RwLock<EnergyState>>) {
    let s = state.read().await;
    let payload = json!({
        "State":             s.water_heater_mode.to_venus_state(),
        "Temperature":       s.water_heater_temp_c,
        "TargetTemperature": s.water_heater_target_c,
        "Position":          0,
    });
    drop(s);
    bus.publish(MqttOutgoing::retained(publish::HEATPUMP_VENUS, &payload)).await;
    bus.emit_live(LiveEvent::new("water_heater_venus", &payload));
}

// ---------------------------------------------------------------------------
// Control logic — evaluates conditions every 30 seconds
// ---------------------------------------------------------------------------

async fn control_task(
    cfg: WaterHeaterConfig,
    lg: Arc<LgThinqClient>,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    // Condition timestamps: when each condition first appeared
    let mut discharge_since: Option<DateTime<Utc>>      = None;
    let mut low_solar_since:  Option<DateTime<Utc>>      = None;

    let mut ticker = interval(Duration::from_secs(30));

    loop {
        ticker.tick().await;
        let now = Utc::now();

        let (ac_ignore, soc, batt_current, solar_total, irradiance, current_mode, last_change) = {
            let s = state.read().await;
            (
                s.ac_ignore.unwrap_or(0),
                s.soc_pct.unwrap_or(0.0),
                s.battery_current_a.unwrap_or(0.0),
                s.solar_total_w,
                s.irradiance_wm2,
                s.water_heater_mode,
                s.water_heater_last_change,
            )
        };

        // --- Evaluate conditions ---

        // Condition 1: grid connected
        let grid_on = ac_ignore == 0;

        // Condition 2: SOC < 95%
        let soc_low = soc < 95.0;

        // Condition 3: battery discharging for > debounce_secs
        let discharging = batt_current < 0.0;
        if discharging {
            discharge_since.get_or_insert(now);
        } else {
            discharge_since = None;
        }
        let discharge_too_long = discharge_since.map(|t| {
            (now - t).num_seconds() as u64 >= cfg.debounce_secs
        }).unwrap_or(false);

        // Condition 4: solar too low for > debounce_secs
        let solar_low = solar_total <= cfg.solar_min_w;
        if solar_low {
            low_solar_since.get_or_insert(now);
        } else {
            low_solar_since = None;
        }
        let solar_too_low = low_solar_since.map(|t| {
            (now - t).num_seconds() as u64 >= cfg.debounce_secs
        }).unwrap_or(false);

        // Condition 5: irradiance below minimum threshold (immediate, no debounce)
        let irradiance_low = irradiance.map(|w| w < cfg.irradiance_min_wm2).unwrap_or(true);

        let want_vacation = grid_on || soc_low || discharge_too_long || solar_too_low || irradiance_low;
        let target_mode = if want_vacation { WaterHeaterMode::Vacation } else { WaterHeaterMode::HeatPump };

        debug!(
            "Water heater conditions: grid_on={grid_on} soc={soc:.1}% soc_low={soc_low} \
            batt_current={batt_current:.1}A discharge_too_long={discharge_too_long} \
            solar_total={solar_total:.0}W solar_too_low={solar_too_low} \
            irradiance={:?}W/m² irradiance_low={irradiance_low} \
            → want_vacation={want_vacation} current={current_mode:?}",
            irradiance
        );

        // --- Rate limiting ---
        let can_change = last_change.map(|t| {
            (now - t).num_seconds() as u64 >= cfg.mode_change_min_secs
        }).unwrap_or(true);

        if target_mode == current_mode || !can_change {
            if target_mode != current_mode && !can_change {
                let wait = cfg.mode_change_min_secs.saturating_sub(
                    last_change.map(|t| (now - t).num_seconds() as u64).unwrap_or(0)
                );
                debug!("Water heater: mode change blocked — rate limit, {wait}s remaining");
            }
            continue;
        }

        info!("Water heater: changing mode {:?} → {:?} (grid={grid_on}, soc={soc:.1}%, \
            discharge={discharge_too_long}, solar={solar_total:.0}W solar_low={solar_too_low}, irradiance_low={irradiance_low})",
            current_mode, target_mode);

        // --- Apply change ---
        if let Err(e) = lg.set_mode(target_mode).await {
            tracing::error!("LG set_mode error: {e}");
            continue;
        }

        {
            let mut s = state.write().await;
            s.water_heater_mode        = target_mode;
            s.water_heater_last_change = Some(now);
        }

        publish_to_venus(&bus, &state).await;

        // After mode change, set temperature after delay
        let delay_secs = cfg.temp_set_delay_secs;
        let target_temp = match target_mode {
            WaterHeaterMode::HeatPump => cfg.heat_pump_target_c,
            _                        => cfg.vacation_target_c,
        };
        let lg2     = lg.clone();
        let bus2    = bus.clone();
        let state2  = state.clone();
        tokio::spawn(async move {
            sleep(Duration::from_secs(delay_secs)).await;
            if let Err(e) = lg2.set_target_temp(target_temp).await {
                tracing::error!("LG set_target_temp error: {e}");
                return;
            }
            {
                let mut s = state2.write().await;
                s.water_heater_target_c = Some(target_temp);
            }
            publish_to_venus(&bus2, &state2).await;
        });
    }
}
