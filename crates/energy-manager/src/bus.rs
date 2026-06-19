use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
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
    /// Broadcast of all incoming MQTT messages → all logic tasks subscribe.
    /// Wrapped in Arc so fan-out to N subscribers is N ref-count bumps
    /// instead of N full clones of the topic String + Bytes payload.
    pub mqtt_in:     broadcast::Sender<Arc<MqttIncoming>>,
    /// MPSC → MQTT publisher task
    pub mqtt_out:    mpsc::Sender<MqttOutgoing>,
    /// Broadcast → live WebSocket clients
    pub live:        broadcast::Sender<LiveEvent>,
    /// Broadcast rule reload signals → logic tasks (rule_name or "*" for all)
    pub rule_reload: broadcast::Sender<String>,
    /// Number of MQTT messages dropped because `mqtt_out` was full (publisher
    /// task stalled — e.g. broker down). Telemetry/commands are dropped on
    /// purpose so a wedged MQTT path can never freeze a control loop.
    mqtt_out_dropped: Arc<AtomicU64>,
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

        let bus = Self {
            mqtt_in,
            mqtt_out,
            live,
            rule_reload,
            mqtt_out_dropped: Arc::new(AtomicU64::new(0)),
        };
        let rxs = AppBusReceivers { mqtt_out_rx };
        (bus, rxs)
    }

    pub fn subscribe_mqtt(&self) -> broadcast::Receiver<Arc<MqttIncoming>> {
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

    /// Publish a message to MQTT. **NON-BLOCKING by design**: if `mqtt_out` is
    /// full (the publisher task is stalled — e.g. the rumqttc eventloop is stuck
    /// reconnecting to a downed broker), the message is **dropped** rather than
    /// awaited.
    ///
    /// A blocking `send().await` here would park the calling logic task on a full
    /// channel and freeze the whole control plane: the DEYE state machine stopped
    /// restoring the relay for 14 h because its `select!` loop was parked inside a
    /// telemetry/command publish while the broker was wedged, with no panic and no
    /// systemd restart. Telemetry and relay commands are stale once the broker is
    /// down anyway; the periodic resync re-asserts the relay state on recovery.
    pub async fn publish(&self, msg: MqttOutgoing) {
        match self.mqtt_out.try_send(msg) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(msg)) => {
                let n = self.mqtt_out_dropped.fetch_add(1, Ordering::Relaxed) + 1;
                // Throttle: warn on the first drop, then every 256, to flag the
                // stall without flooding the journal.
                if n == 1 || n.is_multiple_of(256) {
                    tracing::warn!(
                        "MQTT out channel full — dropped publish to {} (total dropped: {n}). \
                         Publisher task likely stalled (broker down?).",
                        msg.topic
                    );
                }
            }
            // Publisher gone → process is shutting down / restarting. Drop silently.
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }

    /// Broadcast a live event to WebSocket clients.
    ///
    /// Skip si aucun client WebSocket n'écoute : évite de retenir un `LiveEvent`
    /// volumineux (contient un `serde_json::Value`) dans le ring buffer broadcast
    /// (capacité 64) en permanence. Sans subscriber, chaque event resterait dans
    /// son slot jusqu'à être écrasé par le prochain — fuite de rétention.
    pub fn emit_live(&self, event: LiveEvent) {
        if self.live.receiver_count() == 0 {
            return;
        }
        let _ = self.live.send(event);
    }
}
