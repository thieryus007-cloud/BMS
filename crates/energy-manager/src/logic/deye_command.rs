/// DEYE relay control via Shelly Pro 2PM (MQTT RPC).
/// State machine: On → WaitingToCut (15s) → Off → WaitingToRestore (45s) → Lockout (120s) → On
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
    PendingCut(DateTime<Utc>),    // high freq first seen at
    Off,
    PendingRestore(DateTime<Utc>), // low freq first seen at
    Lockout(DateTime<Utc>),        // locked out until
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

    let shelly_id   = &vic.shelly_deye_id;
    let channel     = vic.shelly_deye_channel;

    if shelly_id.is_empty() {
        info!("DEYE control disabled — shelly_deye_id not configured");
        return;
    }

    let mut deye_sm = DeyeState::On;
    let mut rx = bus.subscribe_mqtt();

    // Ticker for state machine timeout evaluation
    let mut ticker = interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            Ok(msg) = rx.recv() => {
                let t = &msg.topic;
                if *t == t_freq {
                    if let Some(freq) = msg.victron_value::<f64>() {
                        let connected = {
                            let s = state.read().await;
                            s.ac_ignore.map(|v| v == 0).unwrap_or(true)
                        };
                        // Only act in off-grid mode
                        if connected { continue; }

                        deye_sm = transition_on_freq(deye_sm, freq, &cfg, &bus, shelly_id, channel).await;
                    }
                } else if *t == t_connected {
                    if let Some(v) = msg.victron_value::<i64>() {
                        // Grid reconnected — ensure DEYE is on
                        if v == 1 && deye_sm != DeyeState::On {
                            deye_sm = DeyeState::On;
                            send_shelly(&bus, shelly_id, channel, true).await;
                            info!("Grid connected — DEYE restored");
                        }
                    }
                }
            }
            _ = ticker.tick() => {
                deye_sm = evaluate_timeouts(deye_sm, &cfg, &bus, shelly_id, channel).await;
            }
        }
    }
}

async fn transition_on_freq(
    state: DeyeState,
    freq: f64,
    cfg: &DeyeConfig,
    _bus: &AppBus,
    _shelly_id: &str,
    _channel: u8,
) -> DeyeState {
    let now = Utc::now();
    match state {
        DeyeState::On if freq >= cfg.freq_high_hz => {
            info!("DEYE: freq {freq:.2}Hz ≥ {:.2}Hz — starting cut timer", cfg.freq_high_hz);
            DeyeState::PendingCut(now)
        }
        DeyeState::Off if freq <= cfg.freq_low_hz => {
            info!("DEYE: freq {freq:.2}Hz ≤ {:.2}Hz — starting restore timer", cfg.freq_low_hz);
            DeyeState::PendingRestore(now)
        }
        DeyeState::PendingCut(_) if freq < cfg.freq_high_hz => {
            // Freq dropped back — cancel
            info!("DEYE: freq recovered — cancelling cut timer");
            DeyeState::On
        }
        DeyeState::PendingRestore(_) if freq > cfg.freq_low_hz => {
            // Freq climbed again — cancel
            info!("DEYE: freq climbed — cancelling restore timer");
            DeyeState::Off
        }
        other => other,
    }
}

async fn evaluate_timeouts(
    state: DeyeState,
    cfg: &DeyeConfig,
    bus: &AppBus,
    shelly_id: &str,
    channel: u8,
) -> DeyeState {
    let now = Utc::now();
    match state {
        DeyeState::PendingCut(since) => {
            let elapsed = (now - since).num_seconds() as u64;
            if elapsed >= cfg.cut_delay_secs {
                info!("DEYE: cutting relay after {}s", cfg.cut_delay_secs);
                send_shelly(bus, shelly_id, channel, false).await;
                let lockout_until = now + chrono::Duration::seconds(cfg.lockout_secs as i64);
                DeyeState::Lockout(lockout_until)
            } else {
                state
            }
        }
        DeyeState::Lockout(until) => {
            if now >= until {
                info!("DEYE: lockout expired — entering Off state");
                DeyeState::Off
            } else {
                state
            }
        }
        DeyeState::PendingRestore(since) => {
            let elapsed = (now - since).num_seconds() as u64;
            if elapsed >= cfg.reenable_delay_secs {
                info!("DEYE: restoring relay after {}s", cfg.reenable_delay_secs);
                send_shelly(bus, shelly_id, channel, true).await;
                DeyeState::On
            } else {
                state
            }
        }
        other => other,
    }
}

async fn send_shelly(bus: &AppBus, shelly_id: &str, channel: u8, on: bool) {
    let topic = publish::shelly_rpc(shelly_id);
    let payload = json!({
        "id": 1,
        "src": "energy-manager",
        "method": "Switch.Set",
        "params": {
            "id": channel,
            "on": on
        }
    });
    bus.publish(MqttOutgoing::transient(topic, &payload)).await;
    info!("DEYE Shelly: switch {} = {}", channel, if on { "ON" } else { "OFF" });

}
