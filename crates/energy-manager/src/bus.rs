use tokio::sync::{broadcast, mpsc};
use crate::types::{LiveEvent, MqttIncoming, MqttOutgoing};

// Capacity constants
const MQTT_IN_CAPACITY:  usize = 512;
const MQTT_OUT_CAPACITY: usize = 256;
const LIVE_CAPACITY:     usize = 64;

/// Central message bus passed to all tasks.
/// Clone it freely — each field is Arc-backed.
#[derive(Clone)]
pub struct AppBus {
    /// Broadcast of all incoming MQTT messages → all logic tasks subscribe
    pub mqtt_in:  broadcast::Sender<MqttIncoming>,
    /// MPSC → MQTT publisher task
    pub mqtt_out: mpsc::Sender<MqttOutgoing>,
    /// Broadcast → live WebSocket clients
    pub live:     broadcast::Sender<LiveEvent>,
}

pub struct AppBusReceivers {
    pub mqtt_out_rx: mpsc::Receiver<MqttOutgoing>,
}

impl AppBus {
    pub fn new() -> (Self, AppBusReceivers) {
        let (mqtt_in, _)      = broadcast::channel(MQTT_IN_CAPACITY);
        let (mqtt_out, mqtt_out_rx) = mpsc::channel(MQTT_OUT_CAPACITY);
        let (live, _)               = broadcast::channel(LIVE_CAPACITY);

        let bus = Self { mqtt_in, mqtt_out, live };
        let rxs = AppBusReceivers { mqtt_out_rx };
        (bus, rxs)
    }

    pub fn subscribe_mqtt(&self) -> broadcast::Receiver<MqttIncoming> {
        self.mqtt_in.subscribe()
    }

    #[allow(dead_code)]
    pub fn subscribe_live(&self) -> broadcast::Receiver<LiveEvent> {
        self.live.subscribe()
    }

    /// Publish a message to MQTT (non-blocking, drops if channel full)
    pub async fn publish(&self, msg: MqttOutgoing) {
        let _ = self.mqtt_out.send(msg).await;
    }

    /// Broadcast a live event to WebSocket clients
    pub fn emit_live(&self, event: LiveEvent) {
        let _ = self.live.send(event);
    }
}
