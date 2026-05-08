/// Restores DEYE relay state at startup from retained MQTT topic.
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::types::EnergyState;

/// Called when retained `santuario/persist/deye_state` arrives.
/// Payload is a plain string: "On" | "Off".
pub async fn on_retained_deye_state(payload: &str, state: &Arc<RwLock<EnergyState>>) {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return;
    }
    let mut s = state.write().await;
    s.deye_persisted_state = Some(trimmed.to_string());
    info!("DEYE persisted state restored: {trimmed}");
}
