//! Module climatiseurs Toshiba — **phase lecture seule**.
//!
//! Souscrit `santuario/toshiba/<zone>/state` (publié par le firmware ESP32 Rust,
//! cf. `docs/toshiba-suzumi-rs-plan.md`), remplit `EnergyState.toshiba_ac[zone]`
//! et diffuse un événement live. **Aucune commande émise** à ce stade.
//!
//! Pattern calqué sur `logic/tasmota` (consommateur MQTT lecture seule). La logique
//! de parsing pure est isolée dans [`rules`] (testée sur host). Le futur pilotage
//! (opt-in `control_enabled`) s'ajoutera ici sur le modèle de `logic/deye_command`.

pub mod rules;

use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};

use crate::bus::AppBus;
use crate::config::ToshibaAcConfig;
use crate::types::{EnergyState, LiveEvent};

use rules::{parse_zone, snapshot_from_payload, ToshibaStatePayload};

pub async fn spawn(cfg: ToshibaAcConfig, bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    if !cfg.enabled {
        debug!("toshiba_ac désactivé (energy_manager.toshiba_ac.enabled=false)");
        return;
    }
    info!(
        zones = ?cfg.zones,
        control_enabled = cfg.control_enabled,
        stale_after_secs = cfg.stale_after_secs,
        "toshiba_ac activé (télémétrie lecture seule)"
    );
    crate::supervise::spawn_critical(run(bus, state));
}

async fn run(bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    let mut rx = bus.subscribe_mqtt();
    loop {
        let msg = match rx.recv().await {
            Ok(m) => m,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!("toshiba_ac MQTT subscriber lagged, dropped {n} message(s)");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };

        let Some(zone) = parse_zone(&msg.topic) else {
            continue;
        };
        let Some(payload) = msg.json::<ToshibaStatePayload>() else {
            debug!("toshiba_ac: JSON d'état invalide sur {}", msg.topic);
            continue;
        };

        let snap = snapshot_from_payload(payload, chrono::Utc::now());
        state
            .write()
            .await
            .toshiba_ac
            .insert(zone.to_string(), snap.clone());
        bus.emit_live(LiveEvent::new(format!("toshiba_{zone}"), &snap));
        debug!("toshiba_ac[{zone}] mis à jour (mode={:?})", snap.mode);
    }
}
