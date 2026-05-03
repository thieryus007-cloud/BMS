/// Automatic management of the LG ThinQ water heater.
/// Switches between HEAT_PUMP and VACATION based on SOC, irradiance and grid status.
/// HEAT_PUMP requires: SOC >= 90%, irradiance >= irradiance_min_wm2, grid disconnected (ac_ignore=1).
/// Decision logic is handled by rust-rule-engine (rules/water_heater.grl).
/// Rate limiting remains in Rust (mode_change_min_secs).
mod rules;

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

    tokio::spawn(keepalive_task(cfg.keepalive_secs, bus2, state2));

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
// Control logic — evaluates conditions every 30 seconds via rule engine
// ---------------------------------------------------------------------------

async fn control_task(
    cfg: WaterHeaterConfig,
    lg: Arc<LgThinqClient>,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    let mut rule_engine = match rules::WaterHeaterRuleEngine::new() {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to init water heater rule engine: {e}");
            return;
        }
    };

    let mut last_change: Option<DateTime<Utc>> = None;
    let mut ticker = interval(Duration::from_secs(30));

    loop {
        ticker.tick().await;
        let now = Utc::now();

        let (ac_ignore, soc, irradiance, current_mode, stored_last_change) = {
            let s = state.read().await;
            (
                s.ac_ignore.unwrap_or(0),
                s.soc_pct.unwrap_or(0.0),
                s.irradiance_wm2,
                s.water_heater_mode,
                s.water_heater_last_change,
            )
        };
        // Sync last_change from persisted state on first tick
        if last_change.is_none() {
            last_change = stored_last_change;
        }

        let irradiance_low = irradiance.map(|w| w < cfg.irradiance_min_wm2).unwrap_or(true);
        let grid_connected = ac_ignore == 0;

        let target_mode_str = match rule_engine.evaluate(grid_connected, soc, irradiance_low) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("Water heater rule engine error: {e} — defaulting to Vacation");
                "Vacation".to_string()
            }
        };
        
        tracing::info!("👉 CHAÎNE BRUTE REÇUE : [{}]", target_mode_str);
        
        let target_mode = match target_mode_str.as_str() {
            "HEAT_PUMP" => WaterHeaterMode::HeatPump,
            _          => WaterHeaterMode::Vacation,
        };

        debug!(
            "Water heater rule engine: grid={grid_connected} soc={soc:.1}% \
            irradiance_low={irradiance_low} → {target_mode_str}"
        );

        // Rate limiting
        let can_change = last_change
            .map(|t| (now - t).num_seconds() as u64 >= cfg.mode_change_min_secs)
            .unwrap_or(true);

        if target_mode == current_mode || !can_change {
            if target_mode != current_mode && !can_change {
                let wait = cfg.mode_change_min_secs.saturating_sub(
                    last_change.map(|t| (now - t).num_seconds() as u64).unwrap_or(0),
                );
                debug!("Water heater: mode change blocked — rate limit, {wait}s remaining");
            }
            continue;
        }

        info!(
            "Water heater: changing mode {:?} → {:?} (grid={grid_connected}, soc={soc:.1}%, irradiance_low={irradiance_low})",
            current_mode, target_mode
        );

        if let Err(e) = lg.set_mode(target_mode).await {
            tracing::error!("LG set_mode error: {e}");
            continue;
        }

        last_change = Some(now);
        {
            let mut s = state.write().await;
            s.water_heater_mode        = target_mode;
            s.water_heater_last_change = Some(now);
        }

        publish_to_venus(&bus, &state).await;

        // Set target temperature after a short delay
        let delay_secs = cfg.temp_set_delay_secs;
        let target_temp = match target_mode {
            WaterHeaterMode::HeatPump => cfg.heat_pump_target_c,
            _                         => cfg.vacation_target_c,
        };
        let lg2    = lg.clone();
        let bus2   = bus.clone();
        let state2 = state.clone();
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
