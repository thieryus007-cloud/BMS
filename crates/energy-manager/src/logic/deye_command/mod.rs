/// DEYE relay control via Shelly Pro 2PM (MQTT RPC).
/// State machine: On → PendingCut (15s) → Lockout (120s) → Off → PendingRestore (45s) → On
/// State transitions are decided by rust-rule-engine (rules/deye_command.grl).
/// Timestamp tracking and relay I/O remain in Rust.
mod rules;

use chrono::{DateTime, Utc};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::info;

use crate::bus::AppBus;
use crate::config::{DeyeConfig, VictronConfig};
use crate::mqtt::topics::publish;
use crate::types::{EnergyState, MqttOutgoing};

#[derive(Debug, Clone, Copy, PartialEq)]
enum DeyeState {
    On,
    PendingCut(DateTime<Utc>),     // high freq first seen at
    Off,
    PendingRestore(DateTime<Utc>), // low freq first seen at
    Lockout(DateTime<Utc>),        // locked out until this timestamp
}

fn state_name(s: &DeyeState) -> &'static str {
    match s {
        DeyeState::On                => "On",
        DeyeState::PendingCut(_)     => "PendingCut",
        DeyeState::Off               => "Off",
        DeyeState::PendingRestore(_) => "PendingRestore",
        DeyeState::Lockout(_)        => "Lockout",
    }
}

fn time_in_state_secs(s: &DeyeState, now: DateTime<Utc>) -> u64 {
    match s {
        DeyeState::PendingCut(since) | DeyeState::PendingRestore(since) => {
            (now - *since).num_seconds().max(0) as u64
        }
        _ => 0,
    }
}

fn lockout_expired(s: &DeyeState, now: DateTime<Utc>) -> bool {
    if let DeyeState::Lockout(until) = s {
        now >= *until
    } else {
        false
    }
}

pub async fn spawn(
    vic: Arc<VictronConfig>,
    cfg: DeyeConfig,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    tokio::spawn(run(vic, cfg, bus, state));
}

async fn run(
    vic: Arc<VictronConfig>,
    cfg: DeyeConfig,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    let pid = &vic.portal_id;
    let vb  = vic.vebus_instance;

    let t_freq      = format!("N/{pid}/vebus/{vb}/Ac/Out/L1/F");
    let t_connected = format!("N/{pid}/vebus/{vb}/Ac/ActiveIn/Connected");

    let shelly_id = &vic.shelly_deye_id;
    let channel   = vic.shelly_deye_channel;

    if shelly_id.is_empty() {
        info!("DEYE control disabled — shelly_deye_id not configured");
        return;
    }

    let mut rule_engine = match rules::DeyeRuleEngine::new() {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to init DEYE rule engine: {e}");
            return;
        }
    };

    let mut deye_sm   = DeyeState::On;
    let mut last_freq: f64 = 50.0;
    let mut rx = bus.subscribe_mqtt();
    let mut ticker = interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            Ok(msg) = rx.recv() => {
                let t = &msg.topic;

                if *t == t_freq {
                    if let Some(freq) = msg.victron_value::<f64>() {
                        last_freq = freq;

                        let connected = {
                            state.read().await.ac_ignore.map(|v| v == 0).unwrap_or(true)
                        };
                        // Frequency-based logic applies only in off-grid mode
                        if connected { continue; }

                        let now = Utc::now();
                        deye_sm = apply_decision(
                            rule_engine.evaluate(
                                state_name(&deye_sm),
                                last_freq,
                                time_in_state_secs(&deye_sm, now),
                                false,
                                cfg.freq_high_hz,
                                cfg.freq_low_hz,
                                cfg.cut_delay_secs,
                                cfg.reenable_delay_secs,
                                lockout_expired(&deye_sm, now),
                            ),
                            deye_sm,
                            now,
                            &cfg,
                            &bus,
                            shelly_id,
                            channel,
                        ).await;
                    }

                } else if *t == t_connected {
                    if let Some(v) = msg.victron_value::<i64>() {
                        if v == 1 {
                            let now = Utc::now();
                            deye_sm = apply_decision(
                                rule_engine.evaluate(
                                    state_name(&deye_sm),
                                    last_freq,
                                    0,
                                    true, // grid reconnected → rule engine restores On
                                    cfg.freq_high_hz,
                                    cfg.freq_low_hz,
                                    cfg.cut_delay_secs,
                                    cfg.reenable_delay_secs,
                                    false,
                                ),
                                deye_sm,
                                now,
                                &cfg,
                                &bus,
                                shelly_id,
                                channel,
                            ).await;
                        }
                    }
                }
            }

            _ = ticker.tick() => {
                let now = Utc::now();
                deye_sm = apply_decision(
                    rule_engine.evaluate(
                        state_name(&deye_sm),
                        last_freq,
                        time_in_state_secs(&deye_sm, now),
                        false,
                        cfg.freq_high_hz,
                        cfg.freq_low_hz,
                        cfg.cut_delay_secs,
                        cfg.reenable_delay_secs,
                        lockout_expired(&deye_sm, now),
                    ),
                    deye_sm,
                    now,
                    &cfg,
                    &bus,
                    shelly_id,
                    channel,
                ).await;
            }
        }
    }
}

/// Applies a rule engine decision: send relay commands and return updated DeyeState.
async fn apply_decision(
    decision: anyhow::Result<rules::DeyeDecision>,
    current: DeyeState,
    now: DateTime<Utc>,
    cfg: &DeyeConfig,
    bus: &AppBus,
    shelly_id: &str,
    channel: u8,
) -> DeyeState {
    let d = match decision {
        Ok(d)  => d,
        Err(e) => {
            tracing::error!("DEYE rule engine error: {e}");
            return current;
        }
    };

    if d.relay_off {
        send_shelly(bus, shelly_id, channel, false).await;
    }
    if d.relay_on {
        send_shelly(bus, shelly_id, channel, true).await;
    }

    let Some(next_name) = d.next_state else {
        return current;
    };

    match next_name.as_str() {
        "On" => {
            info!("DEYE: → On");
            DeyeState::On
        }
        "Off" => {
            info!("DEYE: → Off");
            DeyeState::Off
        }
        "PendingCut" => {
            info!("DEYE: freq high — starting cut timer");
            DeyeState::PendingCut(now)
        }
        "PendingRestore" => {
            info!("DEYE: freq low — starting restore timer");
            DeyeState::PendingRestore(now)
        }
        "Lockout" => {
            let until = now + chrono::Duration::seconds(cfg.lockout_secs as i64);
            info!("DEYE: relay cut — lockout until {until}");
            DeyeState::Lockout(until)
        }
        other => {
            tracing::warn!("DEYE rule engine returned unknown state: {other}");
            current
        }
    }
}

async fn send_shelly(bus: &AppBus, shelly_id: &str, channel: u8, on: bool) {
    let topic   = publish::shelly_rpc(shelly_id);
    let payload = json!({
        "id":     1,
        "src":    "energy-manager",
        "method": "Switch.Set",
        "params": { "id": channel, "on": on }
    });
    bus.publish(MqttOutgoing::transient(topic, &payload)).await;
    info!("DEYE Shelly: switch {} = {}", channel, if on { "ON" } else { "OFF" });
}
