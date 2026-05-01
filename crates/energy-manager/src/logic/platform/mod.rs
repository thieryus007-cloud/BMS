/// Publishes platform backup status keepalive every N seconds.
use chrono::Utc;
use serde_json::json;
use tokio::time::{interval, Duration};

use crate::bus::AppBus;
use crate::config::PlatformConfig;
use crate::mqtt::topics::publish;
use crate::types::MqttOutgoing;

pub async fn spawn(cfg: PlatformConfig, bus: AppBus) {
    tokio::spawn(run(cfg, bus));
}

async fn run(cfg: PlatformConfig, bus: AppBus) {
    let mut ticker = interval(Duration::from_secs(cfg.publish_interval_secs));
    loop {
        ticker.tick().await;
        let now = Utc::now().timestamp();
        let payload = json!({
            "Backup": {
                "Status":  0,       // 0=idle, 1=running, 2=ok, 3=error
                "LastRun": now,
            },
            "Restore": {
                "Status":  0,
                "LastRun": now,
            }
        });
        bus.publish(MqttOutgoing::retained(publish::PLATFORM_VENUS, &payload)).await;
    }
}
