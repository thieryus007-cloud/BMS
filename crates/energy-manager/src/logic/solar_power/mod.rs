/// Aggregates solar production (MPPT + PV inverter ET112) and posts to daly-bms-server API.
/// Daily baseline decisions for the ET112 energy counter are delegated to the rule engine
/// (rules/solar_power.grl).
mod rules;

use chrono::Datelike;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::debug;

use crate::bus::AppBus;
use crate::config::{SolarConfig, VictronConfig};
use crate::mqtt::topics::publish;
use crate::rules_loader::RulesLoader;
use crate::types::{EnergyState, LiveEvent, MqttOutgoing};

pub async fn spawn(
    vic: Arc<VictronConfig>,
    cfg: SolarConfig,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
    loader: Arc<RulesLoader>,
) {
    let bus2   = bus.clone();
    let state2 = state.clone();
    let vic2   = vic.clone();

    tokio::spawn(mqtt_task(vic2, bus2, state2, loader));
    tokio::spawn(writer_task(cfg, bus, state));
}

async fn mqtt_task(
    vic: Arc<VictronConfig>,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
    loader: Arc<RulesLoader>,
) {
    let pid = &vic.portal_id;
    let m1  = vic.mppt1_instance;
    let m2  = vic.mppt2_instance;
    let pv  = vic.pvinverter_instance;

    let t_m1_power  = format!("N/{pid}/solarcharger/{m1}/Yield/Power");
    let t_m2_power  = format!("N/{pid}/solarcharger/{m2}/Yield/Power");
    let t_dc_pv     = format!("N/{pid}/system/0/Dc/Pv/Power");
    let t_pv_power  = format!("N/{pid}/pvinverter/{pv}/Ac/Power");
    let t_pv_energy = format!("N/{pid}/pvinverter/{pv}/Ac/Energy/Forward");
    let t_m1_yield  = format!("N/{pid}/solarcharger/{m1}/History/Daily/0/Yield");
    let t_m2_yield  = format!("N/{pid}/solarcharger/{m2}/History/Daily/0/Yield");
    let t_m1_state  = format!("N/{pid}/solarcharger/{m1}/State");
    let t_m2_state  = format!("N/{pid}/solarcharger/{m2}/State");
    let t_m1_pv_v   = format!("N/{pid}/solarcharger/{m1}/Pv/V");
    let t_m2_pv_v   = format!("N/{pid}/solarcharger/{m2}/Pv/V");
    let t_m1_dc_i   = format!("N/{pid}/solarcharger/{m1}/Dc/0/Current");
    let t_m2_dc_i   = format!("N/{pid}/solarcharger/{m2}/Dc/0/Current");
    let t_consump   = format!("N/{pid}/system/0/Ac/ConsumptionOnOutput/L1/Power");

    let mut rule_engine = match rules::SolarRuleEngine::with_source(&loader.load("solar_power")) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to init solar rule engine: {e}");
            return;
        }
    };

    let mut rx        = bus.subscribe_mqtt();
    let mut reload_rx = bus.subscribe_rule_reload();

    loop {
        tokio::select! {
            Ok(msg) = rx.recv() => {
        let t = &msg.topic;

        let mut publish_baseline: Option<(i32, f64)> = None;

        {
            let mut s = state.write().await;

            if *t == t_m1_power {
                s.mppt_273.power_w = msg.victron_value::<f64>();
                s.mppt_power_273_w = s.mppt_273.power_w;
            } else if *t == t_m2_power {
                s.mppt_289.power_w = msg.victron_value::<f64>();
                s.mppt_power_289_w = s.mppt_289.power_w;
            } else if *t == t_dc_pv {
                // Source de vérité pour la somme MPPT : N/.../system/0/Dc/Pv/Power
                s.dc_pv_power_w = msg.victron_value::<f64>();
            } else if *t == t_pv_power {
                // N/.../pvinverter/32/Ac/Power (ET112 micro-onduleurs, total AC)
                s.pvinverter_power_w = msg.victron_value::<f64>();
            } else if *t == t_pv_energy {
                if let Some(kwh) = msg.victron_value::<f64>() {
                    let today = chrono::Utc::now().date_naive().num_days_from_ce();

                    // Rule engine decides whether to reset and/or capture baseline
                    let decision = rule_engine.baseline_decision(
                        s.pvinv_baseline_day != today,
                        s.pvinv_baseline_kwh.is_none(),
                    ).unwrap_or_default();

                    if decision.reset {
                        s.pvinv_baseline_kwh = None;
                        s.pvinv_baseline_day = today;
                    }
                    if decision.capture {
                        s.pvinv_baseline_kwh = Some(kwh);
                        publish_baseline      = Some((today, kwh));
                    }

                    let baseline = s.pvinv_baseline_kwh.unwrap_or(kwh);
                    s.pvinv_yield_today_kwh = (kwh - baseline).max(0.0);
                }
            } else if *t == t_m1_yield {
                s.mppt_273.yield_today_kwh = msg.victron_value::<f64>();
            } else if *t == t_m2_yield {
                s.mppt_289.yield_today_kwh = msg.victron_value::<f64>();
            } else if *t == t_m1_state {
                s.mppt_273.state = msg.victron_value::<i64>();
            } else if *t == t_m2_state {
                s.mppt_289.state = msg.victron_value::<i64>();
            } else if *t == t_m1_pv_v {
                s.mppt_273.pv_voltage_v = msg.victron_value::<f64>();
            } else if *t == t_m2_pv_v {
                s.mppt_289.pv_voltage_v = msg.victron_value::<f64>();
            } else if *t == t_m1_dc_i {
                s.mppt_273.dc_current_a = msg.victron_value::<f64>();
            } else if *t == t_m2_dc_i {
                s.mppt_289.dc_current_a = msg.victron_value::<f64>();
            } else if *t == t_consump {
                s.house_power_w = msg.victron_value::<f64>();
            } else {
                continue;
            }

            // MPPT sum = system aggregate (N/.../system/0/Dc/Pv/Power)
            let mppt_total  = s.dc_pv_power_w.unwrap_or(0.0);
            let pvinv_total = s.pvinverter_power_w.unwrap_or(0.0);
            s.solar_total_w = mppt_total + pvinv_total;

            s.mppt_yield_today_kwh  = s.mppt_273.yield_today_kwh.unwrap_or(0.0)
                + s.mppt_289.yield_today_kwh.unwrap_or(0.0);
            s.total_yield_today_kwh = s.mppt_yield_today_kwh + s.pvinv_yield_today_kwh;
        }

        if let Some((day, kwh)) = publish_baseline {
            bus.publish(MqttOutgoing::raw(
                publish::PVINV_BASELINE,
                format!("{day}:{kwh:.3}"),
                true,
            )).await;
            debug!("pvinv_baseline published as retained: day={day} kwh={kwh:.3}");
        }
            }   // close Ok(msg) arm

            Ok(name) = reload_rx.recv() => {
                if name == "solar_power" || name == "*" {
                    let src = loader.load("solar_power");
                    match rules::SolarRuleEngine::with_source(&src) {
                        Ok(e) => { rule_engine = e; tracing::info!("solar_power rule engine reloaded"); }
                        Err(e) => tracing::warn!("solar_power reload failed (keeping old engine): {e}"),
                    }
                }
            }
        }   // close select!
    }       // close loop
}

async fn writer_task(
    cfg: SolarConfig,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    let http_client = reqwest::Client::new();
    let api_url     = format!("{}/api/v1/solar/mppt-yield", cfg.bms_server_url);
    let mut ticker  = interval(Duration::from_secs(1));

    loop {
        ticker.tick().await;

        let (solar_total, house_power, dc_pv_w, m273_w, m289_w, pvinv_w, total_yield) = {
            let s = state.read().await;
            (
                s.solar_total_w,
                s.house_power_w.unwrap_or(0.0),
                s.dc_pv_power_w.unwrap_or(0.0),
                s.mppt_273.power_w.unwrap_or(0.0),
                s.mppt_289.power_w.unwrap_or(0.0),
                s.pvinverter_power_w.unwrap_or(0.0),
                s.total_yield_today_kwh,
            )
        };

        let body = json!({
            "solar_total_w":   solar_total,
            "dc_pv_power_w":   dc_pv_w,   // N/.../system/0/Dc/Pv/Power
            "pvinv_power_w":   pvinv_w,   // N/.../pvinverter/32/Ac/Power
            "mppt_power_w":    dc_pv_w,   // alias = dc_pv pour compatibilité API
            "total_yield_kwh": total_yield,
            "house_power_w":   house_power,
        });
        if let Err(e) = http_client
            .post(&api_url)
            .json(&body)
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            debug!("Solar API POST error: {e}");
        }

        bus.emit_live(LiveEvent::new("solar", json!({
            "solar_total_w": solar_total,
            "dc_pv_power_w": dc_pv_w,
            "mppt_power_w":  dc_pv_w,   // alias rétrocompat dashboard
            "mppt_273_w":    m273_w,
            "mppt_289_w":    m289_w,
            "pvinv_power_w": pvinv_w,
            "pvinv_w":       pvinv_w,   // alias rétrocompat dashboard
            "house_power_w": house_power,
        })));
    }
}
