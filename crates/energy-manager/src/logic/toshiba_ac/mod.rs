//! Module climatiseurs Toshiba — **télémétrie (lecture) + contrôle par présence (opt-in)**.
//!
//! - **Lecture** : souscrit `santuario/toshiba/<zone>/state` (publié par le firmware ESP32
//!   Rust, cf. `docs/toshiba-suzumi-rs-plan.md`), remplit `EnergyState.toshiba_ac[zone]` et
//!   diffuse un événement live.
//! - **Contrôle** (opt-in `control_enabled`, OFF par défaut) : souscrit la présence FP2
//!   `santuario/toshiba/presence/<zone>` (pont `bridge/aqara-fp2-mqtt`) et, au tick 1 Hz,
//!   émet une commande `santuario/toshiba/<zone>/command` **uniquement sur changement**
//!   d'action (ECO dès absence, OFF après N min, rallumage au retour — cf. plan §18).
//!
//! La logique (parsing + décision présence) est **pure et testée** dans [`rules`] ; cette
//! boucle ne fait que l'alimenter et publier. Modèle : `logic/deye_command` (tick 1 Hz +
//! MQTT dans un `select!`).

pub mod rules;

use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn};

use crate::bus::AppBus;
use crate::config::ToshibaAcConfig;
use crate::mqtt::topics::publish::toshiba_command;
use crate::types::{EnergyState, LiveEvent, MqttOutgoing};

use rules::{
    parse_presence_zone, parse_zone, presence_command_json, snapshot_from_payload,
    PresenceControl, PresencePayload, ToshibaStatePayload,
};

pub async fn spawn(cfg: ToshibaAcConfig, bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    if !cfg.enabled {
        debug!("toshiba_ac désactivé (energy_manager.toshiba_ac.enabled=false)");
        return;
    }
    info!(
        zones = ?cfg.zones,
        control_enabled = cfg.control_enabled,
        eco_after_secs = cfg.eco_after_secs,
        off_after_secs = cfg.off_after_secs,
        stale_after_secs = cfg.stale_after_secs,
        "toshiba_ac activé"
    );
    crate::supervise::spawn_critical(run(cfg, bus, state));
}

async fn run(cfg: ToshibaAcConfig, bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    let mut rx = bus.subscribe_mqtt();
    let mut control = PresenceControl::default();
    // Pilote 1 Hz : ré-évalue la présence et émet les commandes changées. La branche
    // est **désactivée** tant que `control_enabled=false` → aucune commande, aucun réveil.
    let mut ticker = interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            _ = ticker.tick(), if cfg.control_enabled => {
                let now = chrono::Utc::now();
                for (zone, action) in control.evaluate(now, cfg.eco_after_secs, cfg.off_after_secs) {
                    let payload = presence_command_json(action);
                    bus.publish(MqttOutgoing::raw(toshiba_command(&zone), payload, false)).await;
                    info!("toshiba_ac[{zone}] présence → {action:?} : commande {payload}");
                }
            }

            result = rx.recv() => {
                let msg = match result {
                    Ok(m) => m,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("toshiba_ac MQTT subscriber lagged, dropped {n} message(s)");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };

                // 1) Télémétrie d'état (lecture seule) : santuario/toshiba/<zone>/state.
                if let Some(zone) = parse_zone(&msg.topic) {
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
                    continue;
                }

                // 2) Présence FP2 : santuario/toshiba/presence/<zone> (contrôle opt-in).
                //    Ignorée tant que control_enabled=false (rien à piloter).
                if cfg.control_enabled {
                    if let Some(zone) = parse_presence_zone(&msg.topic) {
                        match msg.json::<PresencePayload>() {
                            Some(p) => control.on_presence(zone, p.present, chrono::Utc::now()),
                            None => debug!("toshiba_ac: présence JSON invalide sur {}", msg.topic),
                        }
                    }
                }
            }
        }
    }
}
