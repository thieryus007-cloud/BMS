/// Sends a periodic keepalive to the Victron GX MQTT broker.
///
/// Venus OS stops publishing N/ topics after ~60s if no client publishes
/// an R/ keepalive.  We publish `R/{portal_id}/keepalive` every 30s so the
/// GX keeps streaming live data.
use tokio::time::{interval, Duration};
use tracing::debug;

use crate::bus::AppBus;
use crate::types::MqttOutgoing;

pub async fn spawn(portal_id: String, bus: AppBus) {
    tokio::spawn(run(portal_id, bus));
}

async fn run(portal_id: String, bus: AppBus) {
    let topic = format!("R/{portal_id}/keepalive");
    let mut ticker = interval(Duration::from_secs(30));
    loop {
        ticker.tick().await;
        debug!("Sending Victron keepalive → {topic}");
        bus.publish(MqttOutgoing::raw(&topic, "", false)).await;
    }
}
