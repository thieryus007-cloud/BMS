/// ATS CHINT switch — publishes santuario/switch/1/venus on command or keepalive.
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

use crate::bus::AppBus;
use crate::mqtt::topics::publish;
use crate::types::{EnergyState, MqttOutgoing};

const KEEPALIVE_SECS: u64 = 60;

pub async fn spawn(bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    tokio::spawn(run(bus, state));
}

async fn run(bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    let mut ticker = interval(Duration::from_secs(KEEPALIVE_SECS));
    loop {
        ticker.tick().await;
        publish_state(&bus, &state).await;
    }
}

#[allow(dead_code)]
pub async fn set_position(
    bus: &AppBus,
    state: &Arc<RwLock<EnergyState>>,
    position: i64,  // 0=réseau, 1=génératrice
) {
    {
        let mut s = state.write().await;
        s.ats_position = position;
        s.ats_state    = 1;
    }
    publish_state(bus, state).await;
}

async fn publish_state(bus: &AppBus, state: &Arc<RwLock<EnergyState>>) {
    let s = state.read().await;
    let position = s.ats_position;
    let ats_state = s.ats_state;
    let payload = json!({
        "Position": position,
        "State":    ats_state,
    });
    drop(s);
    bus.publish(MqttOutgoing::retained(publish::SWITCH_VENUS, &payload)).await;

}
