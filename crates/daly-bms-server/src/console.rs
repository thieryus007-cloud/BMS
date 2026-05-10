//! Diagnostic console — broadcast channel for real-time event streaming.
//!
//! Each significant event (MQTT in/out, RS485 state changes, system messages)
//! is wrapped in a [`ConsoleEvent`] and sent on the [`ConsoleBus`].
//! The `/ws/console` WebSocket handler subscribes and forwards events to the browser.

use chrono::Utc;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::broadcast;

pub const CONSOLE_CAPACITY: usize = 512;

// ---------------------------------------------------------------------------
// Event classification
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    MqttIn,
    MqttOut,
    Rs485,
    State,
    Error,
    System,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventDevice {
    Bms1,
    Bms2,
    Et112,
    Ats,
    Irradiance,   // PRALRAN capteur irradiance RS485
    #[serde(rename = "smartshunt")]
    SmartShunt,
    Tasmota,
    Shelly,
    WaterHeater,  // LG ThinQ / heatpump
    Solar,        // MPPT / pvinverter (energy-manager)
    Inverter,     // Venus OS onduleur
    Venus,        // températures et autres données Venus OS
    System,
}

// ---------------------------------------------------------------------------
// ConsoleEvent
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ConsoleEvent {
    pub ts:      String,
    pub kind:    EventKind,
    pub device:  EventDevice,
    pub label:   String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic:   Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text:    Option<String>,
}

#[allow(dead_code)]
impl ConsoleEvent {
    fn now() -> String {
        Utc::now().format("%H:%M:%S%.3f").to_string()
    }

    pub fn mqtt_in(device: EventDevice, topic: &str, payload: serde_json::Value) -> Self {
        Self {
            ts:      Self::now(),
            kind:    EventKind::MqttIn,
            device,
            label:   format!("MQTT ← {topic}"),
            topic:   Some(topic.to_string()),
            payload: Some(payload),
            text:    None,
        }
    }

    pub fn mqtt_out(device: EventDevice, topic: &str, payload: serde_json::Value) -> Self {
        Self {
            ts:      Self::now(),
            kind:    EventKind::MqttOut,
            device,
            label:   format!("MQTT → {topic}"),
            topic:   Some(topic.to_string()),
            payload: Some(payload),
            text:    None,
        }
    }

    pub fn rs485(device: EventDevice, label: &str, payload: serde_json::Value) -> Self {
        Self {
            ts:      Self::now(),
            kind:    EventKind::Rs485,
            device,
            label:   label.to_string(),
            topic:   None,
            payload: Some(payload),
            text:    None,
        }
    }

    pub fn state(device: EventDevice, label: &str, payload: serde_json::Value) -> Self {
        Self {
            ts:      Self::now(),
            kind:    EventKind::State,
            device,
            label:   label.to_string(),
            topic:   None,
            payload: Some(payload),
            text:    None,
        }
    }

    pub fn error(device: EventDevice, label: &str, msg: &str) -> Self {
        Self {
            ts:      Self::now(),
            kind:    EventKind::Error,
            device,
            label:   label.to_string(),
            topic:   None,
            payload: None,
            text:    Some(msg.to_string()),
        }
    }

    pub fn system(label: &str, msg: &str) -> Self {
        Self {
            ts:      Self::now(),
            kind:    EventKind::System,
            device:  EventDevice::System,
            label:   label.to_string(),
            topic:   None,
            payload: None,
            text:    Some(msg.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// ConsoleBus
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ConsoleBus {
    tx: broadcast::Sender<Arc<ConsoleEvent>>,
}

impl ConsoleBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CONSOLE_CAPACITY);
        Self { tx }
    }

    /// Emit an event. Non-blocking — silently drops if no subscribers.
    pub fn emit(&self, ev: ConsoleEvent) {
        let _ = self.tx.send(Arc::new(ev));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<ConsoleEvent>> {
        self.tx.subscribe()
    }
}
