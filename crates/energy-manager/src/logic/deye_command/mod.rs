/// DEYE relay control via Shelly Pro 2PM (MQTT RPC).
/// State machine: On → PendingCut (15s) → Lockout (120s) → Off → PendingRestore (45s) → On
/// State transitions are decided by rust-rule-engine (rules/deye_command.grl).
/// State is persisted to santuario/persist/deye_state (retained MQTT) to survive restarts.
mod rules;

use chrono::{DateTime, Utc};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{interval, sleep, Duration};
use tracing::{info, warn};

use crate::bus::AppBus;
use crate::config::{DeyeConfig, VictronConfig};
use crate::mqtt::topics::publish;
use crate::rules_loader::RulesLoader;
use crate::types::{EnergyState, MqttOutgoing};

#[derive(Debug, Clone, Copy, PartialEq)]
enum DeyeState {
    On,
    PendingCut(DateTime<Utc>),
    Off,
    PendingRestore(DateTime<Utc>),
    Lockout(DateTime<Utc>),
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
    loader: Arc<RulesLoader>,
) {
    crate::supervise::spawn_critical(run(vic, cfg, bus, state, loader));
}

async fn run(
    vic: Arc<VictronConfig>,
    cfg: DeyeConfig,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
    loader: Arc<RulesLoader>,
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

    let src = loader.load("deye_command");
    let mut rule_engine = match rules::DeyeRuleEngine::with_source(&src) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to init DEYE rule engine: {e}");
            return;
        }
    };

    // Restore persisted state — wait up to 3s for retained MQTT message.
    // This prevents spurious relay-on when restarting while DEYE is cut.
    let initial_state = {
        sleep(Duration::from_secs(3)).await;
        let s = state.read().await;
        match s.deye_persisted_state.as_deref() {
            Some("Off") | Some("Lockout") | Some("PendingRestore") => {
                info!("DEYE: restoring state=Off from retained MQTT");
                DeyeState::Off
            }
            Some(other) => {
                info!("DEYE: starting with state=On (persisted={other})");
                DeyeState::On
            }
            None => {
                info!("DEYE: no persisted state — starting with On");
                DeyeState::On
            }
        }
    };

    let mut deye_sm   = initial_state;
    let mut last_freq: f64 = 50.0;
    let mut rx        = bus.subscribe_mqtt();
    let mut reload_rx = bus.subscribe_rule_reload();
    let mut ticker    = interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            result = rx.recv() => {
                let msg = match result {
                    Ok(m) => m,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("deye_command MQTT subscriber lagged, dropped {n} message(s)");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let t = &msg.topic;

                if *t == t_freq {
                    if let Some(freq) = msg.victron_value::<f64>() {
                        last_freq = freq;

                        let connected = {
                            state.read().await.ac_ignore.map(|v| v == 0).unwrap_or(true)
                        };
                        if connected { continue; }

                        let now = Utc::now();
                        let new_state = apply_decision(
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
                        if new_state != deye_sm {
                            deye_sm = new_state;
                            persist_deye_state(&bus, &deye_sm).await;
                            update_deye_state(&state, &deye_sm).await;
                        }
                    }

                } else if *t == t_connected {
                    if let Some(v) = msg.victron_value::<i64>() {
                        if v == 1 {
                            let now = Utc::now();
                            let new_state = apply_decision(
                                rule_engine.evaluate(
                                    state_name(&deye_sm),
                                    last_freq,
                                    0,
                                    true,
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
                            if new_state != deye_sm {
                                deye_sm = new_state;
                                persist_deye_state(&bus, &deye_sm).await;
                                update_deye_state(&state, &deye_sm).await;
                            }
                        }
                    }
                }
            }

            _ = ticker.tick() => {
                let now = Utc::now();
                let new_state = apply_decision(
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
                if new_state != deye_sm {
                    deye_sm = new_state;
                    persist_deye_state(&bus, &deye_sm).await;
                    update_deye_state(&state, &deye_sm).await;
                }
            }

            Ok(name) = reload_rx.recv() => {
                if name == "deye_command" || name == "*" {
                    let src = loader.load("deye_command");
                    match rules::DeyeRuleEngine::with_source(&src) {
                        Ok(e) => { rule_engine = e; info!("deye_command rule engine reloaded"); }
                        Err(e) => tracing::warn!("deye_command reload failed (keeping old engine): {e}"),
                    }
                }
            }
        }
    }
}

/// Publishes DEYE state as retained MQTT for persistence across restarts.
/// Only stable states are persisted: "On" and "Off".
async fn persist_deye_state(bus: &AppBus, state: &DeyeState) {
    let persisted = match state {
        DeyeState::On | DeyeState::PendingCut(_) => "On",
        DeyeState::Off | DeyeState::Lockout(_) | DeyeState::PendingRestore(_) => "Off",
    };
    bus.publish(MqttOutgoing::raw(
        publish::DEYE_STATE, persisted, true,
    )).await;
}

/// Updates EnergyState with DEYE relay info for the REST /api/rules-status endpoint.
async fn update_deye_state(state: &Arc<RwLock<EnergyState>>, deye: &DeyeState) {
    let mut s = state.write().await;
    s.deye_on          = matches!(deye, DeyeState::On | DeyeState::PendingCut(_));
    s.deye_last_change = Some(Utc::now());
}

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
