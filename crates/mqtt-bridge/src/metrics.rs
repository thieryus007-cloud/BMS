//! Compteurs atomiques exposés via HTTP /metrics.

use chrono::Utc;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

pub struct BridgeMetrics {
    pub start:               Instant,
    pub local_connected:     AtomicBool,
    pub remote_connected:    AtomicBool,
    pub msgs_local_to_remote: AtomicU64,
    pub msgs_remote_to_local: AtomicU64,
    pub reconnects_local:    AtomicU64,
    pub reconnects_remote:   AtomicU64,
}

impl BridgeMetrics {
    pub fn new() -> Self {
        Self {
            start:                Instant::now(),
            local_connected:      AtomicBool::new(false),
            remote_connected:     AtomicBool::new(false),
            msgs_local_to_remote: AtomicU64::new(0),
            msgs_remote_to_local: AtomicU64::new(0),
            reconnects_local:     AtomicU64::new(0),
            reconnects_remote:    AtomicU64::new(0),
        }
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            uptime_secs:          self.start.elapsed().as_secs(),
            local_connected:      self.local_connected.load(Ordering::Relaxed),
            remote_connected:     self.remote_connected.load(Ordering::Relaxed),
            msgs_local_to_remote: self.msgs_local_to_remote.load(Ordering::Relaxed),
            msgs_remote_to_local: self.msgs_remote_to_local.load(Ordering::Relaxed),
            reconnects_local:     self.reconnects_local.load(Ordering::Relaxed),
            reconnects_remote:    self.reconnects_remote.load(Ordering::Relaxed),
            timestamp:            Utc::now().to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
pub struct MetricsSnapshot {
    pub uptime_secs:          u64,
    pub local_connected:      bool,
    pub remote_connected:     bool,
    pub msgs_local_to_remote: u64,
    pub msgs_remote_to_local: u64,
    pub reconnects_local:     u64,
    pub reconnects_remote:    u64,
    pub timestamp:            String,
}
