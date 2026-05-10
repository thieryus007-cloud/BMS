use tokio::sync::{broadcast, mpsc};
use crate::types::{LiveEvent, MqttIncoming, MqttOutgoing};

// Capacity constants
const MQTT_IN_CAPACITY:    usize = 512;
const MQTT_OUT_CAPACITY:   usize = 256;
const LIVE_CAPACITY:       usize = 64;
const RULE_RELOAD_CAPACITY: usize = 16;

/// Central message bus passed to all tasks.
/// Clone it freely — each field is Arc-backed.
#[derive(Clone)]
pub struct AppBus {
    /// Broadcast of all incoming MQTT messages → all logic tasks subscribe
    pub mqtt_in:     broadcast::Sender<MqttIncoming>,
    /// MPSC → MQTT publisher task
    pub mqtt_out:    mpsc::Sender<MqttOutgoing>,
    /// Broadcast → live WebSocket clients
    pub live:        broadcast::Sender<LiveEvent>,
    /// Broadcast rule reload signals → logic tasks (rule_name or "*" for all)
    pub rule_reload: broadcast::Sender<String>,
}

pub struct AppBusReceivers {
    pub mqtt_out_rx: mpsc::Receiver<MqttOutgoing>,
}

impl AppBus {
    pub fn new() -> (Self, AppBusReceivers) {
        let (mqtt_in, _)           = broadcast::channel(MQTT_IN_CAPACITY);
        let (mqtt_out, mqtt_out_rx) = mpsc::channel(MQTT_OUT_CAPACITY);
        let (live, _)              = broadcast::channel(LIVE_CAPACITY);
        let (rule_reload, _)       = broadcast::channel(RULE_RELOAD_CAPACITY);

        let bus = Self { mqtt_in, mqtt_out, live, rule_reload };
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

    pub fn subscribe_rule_reload(&self) -> broadcast::Receiver<String> {
        self.rule_reload.subscribe()
    }

    /// Trigger a hot-reload of `rule_name` (or "*" for all rules) in all logic tasks.
    #[allow(dead_code)]
    pub fn trigger_rule_reload(&self, rule_name: &str) {
        let _ = self.rule_reload.send(rule_name.to_string());
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
