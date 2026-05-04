/// Automatic management of the LG ThinQ water heater.
mod rules;
use chrono::{DateTime, Utc};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, sleep, Duration};
use tracing::{error, info};
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

async fn control_task(
    cfg: WaterHeaterConfig,
    lg: Arc<LgThinqClient>,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    // ✅ Rule engine recreated each cycle (no more no-loop issue)
    let mut last_change: Option<DateTime<Utc>> = None;
    let mut last_sent_mode: Option<WaterHeaterMode> = None; // ← NOUVEAU
    let mut ticker = interval(Duration::from_secs(30));

    info!("Water heater control task started");

    loop {
        ticker.tick().await;
        let now = Utc::now();

        // ✅ Recreate rule engine each cycle
        let mut rule_engine = match rules::WaterHeaterRuleEngine::new() {
            Ok(e) => e,
            Err(e) => {
                error!("Failed to init water heater rule engine: {e}");
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        // Read inputs
        let (ac_ignore, soc, irradiance) = {
            let s = state.read().await;
            (
                s.ac_ignore.unwrap_or(0),
                s.soc_pct.unwrap_or(0.0),
                s.irradiance_wm2,
            )
        };

        let irradiance_low = irradiance
            .map(|w| w < cfg.irradiance_min_wm2)
            .unwrap_or(true);
        let grid_connected = ac_ignore == 0;

        // Evaluate rules
        let target_mode_str = match rule_engine.evaluate(grid_connected, soc, irradiance_low) {
            Ok(m) => m,
            Err(e) => {
                error!("Rule engine error: {e} — fallback VACATION");
                "VACATION".to_string()  // ✅ Uppercase!
            }
        };

        let target_mode = match target_mode_str.as_str() {
            "HEAT_PUMP" => WaterHeaterMode::HeatPump,
            _ => WaterHeaterMode::Vacation,
        };

        // Rate limiting
        let can_change = last_change
            .map(|t| (now - t).num_seconds() as u64 >= cfg.mode_change_min_secs)
            .unwrap_or(true);

        // ✅ DECISION BASED ON last_sent_mode (NOT current_mode!)
        let should_send = match last_sent_mode {
            Some(last) => last != target_mode,
            None => true, // first run
        };

        // Safety refresh every 10 minutes
        let force_refresh = last_change
            .map(|t| (now - t).num_minutes() >= 10)
            .unwrap_or(true);

        if (!should_send && !force_refresh) || !can_change {
            continue;
        }

        // ✅ THIS LINE MUST APPEAR
        info!(
            "Water heater: SEND {:?} (soc={:.1}%, grid={}, irradiance_low={})",
            target_mode, soc, grid_connected, irradiance_low
        );

        // Send command to LG
        if let Err(e) = lg.set_mode(target_mode).await {
            error!("LG set_mode error: {e}");
            continue;
        }

        // ✅ Update last_sent_mode AFTER successful send
        last_sent_mode = Some(target_mode);
        last_change = Some(now);

        {
            let mut s = state.write().await;
            s.water_heater_mode = target_mode;
            s.water_heater_last_change = Some(now);
        }

        publish_to_venus(&bus, &state).await;

        // Set temperature (delayed)
        let delay_secs = cfg.temp_set_delay_secs;
        let target_temp = match target_mode {
            WaterHeaterMode::HeatPump => cfg.heat_pump_target_c,
            _ => cfg.vacation_target_c,
        };

        let lg2 = lg.clone();
        let bus2 = bus.clone();
        let state2 = state.clone();

        tokio::spawn(async move {
            sleep(Duration::from_secs(delay_secs)).await;
            if let Err(e) = lg2.set_target_temp(target_temp).await {
                error!("LG set_target_temp error: {e}");
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
