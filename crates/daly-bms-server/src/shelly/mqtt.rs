//! Subscriber MQTT Shelly Pro 2PM — réception des topics natifs Shelly Gen3.
//!
//! Topics surveillés :
//!   {shelly_id}/status/switch:0   → état + mesures canal 0 (publié sur changement)
//!   {shelly_id}/status/switch:1   → état + mesures canal 1 (publié sur changement)
//!   daly-bms-shelly/rpc           → réponses aux RPC GetStatus demandées par ce client
//!
//! Interrogation active toutes les 30 s via RPC Switch.GetStatus pour obtenir l'état
//! initial et combler les absences de publication (aucun changement d'état).

use crate::config::{MqttConfig, ShellyDeviceConfig};
use super::types::{ShellyChannelData, ShellyEmSnapshot};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{info, warn};
use chrono::Local;

/// Client ID de ce subscriber — utilisé comme topic de réponse RPC.
const RPC_SRC: &str = "daly-bms-shelly";

/// Cache intermédiaire pour assembler les mesures multi-topics.
#[derive(Default)]
struct ShellyCache {
    ch0: ShellyChannelData,
    ch1: ShellyChannelData,
    rssi: Option<i32>,
}

/// Boucle principale d'abonnement MQTT Shelly Pro 2PM.
///
/// Reconnexion automatique après déconnexion (backoff 10 s).
/// Appelle `on_snapshot` pour chaque mise à jour reçue.
pub async fn run_shelly_mqtt_loop<F>(
    devices:         Vec<ShellyDeviceConfig>,
    mqtt_cfg:        MqttConfig,
    client_out:      Arc<Mutex<Option<AsyncClient>>>,
    mut on_snapshot: F,
)
where
    F: FnMut(ShellyEmSnapshot) + Send + 'static,
{
    if devices.is_empty() {
        return;
    }

    info!(count = devices.len(), host = %mqtt_cfg.host, "Démarrage Shelly Pro 2PM MQTT subscriber");

    let mut cache: HashMap<u8, ShellyCache> = HashMap::new();
    for dev in &devices {
        cache.insert(dev.id, ShellyCache::default());
    }

    loop {
        let mut opts = MqttOptions::new(
            format!("{}-{}", RPC_SRC, uuid::Uuid::new_v4()),
            &mqtt_cfg.host,
            mqtt_cfg.port,
        );
        opts.set_keep_alive(Duration::from_secs(30));

        if let (Some(user), Some(pass)) = (&mqtt_cfg.username, &mqtt_cfg.password) {
            opts.set_credentials(user, pass);
        }

        let (client, mut eventloop) = AsyncClient::new(opts, 128);

        {
            let mut guard = client_out.lock().await;
            *guard = Some(client.clone());
        }

        // Abonnement aux topics status de chaque device configuré + réponses RPC
        let rpc_response_topic = format!("{}/rpc", RPC_SRC);
        let _ = client.subscribe(&rpc_response_topic, QoS::AtMostOnce).await;
        for dev in &devices {
            let t0 = format!("{}/status/switch:0", dev.shelly_id);
            let t1 = format!("{}/status/switch:1", dev.shelly_id);
            let ti = format!("{}/info", dev.shelly_id);
            let _ = client.subscribe(&t0, QoS::AtMostOnce).await;
            let _ = client.subscribe(&t1, QoS::AtMostOnce).await;
            let _ = client.subscribe(&ti, QoS::AtMostOnce).await;
        }

        // Tâche de polling périodique (30 s) via RPC Switch.GetStatus.
        // Permet d'obtenir l'état initial et de rafraîchir même sans changement d'état.
        let poll_client  = client.clone();
        let poll_devices = devices.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(30));
            loop {
                ticker.tick().await;
                for (req_id, dev) in poll_devices.iter().enumerate() {
                    for ch in 0u8..2 {
                        let payload = serde_json::json!({
                            "id":     req_id * 2 + ch as usize,
                            "src":    RPC_SRC,
                            "method": "Switch.GetStatus",
                            "params": { "id": ch }
                        });
                        let topic = format!("{}/rpc", dev.shelly_id);
                        let _ = poll_client.publish(
                            &topic,
                            QoS::AtMostOnce,
                            false,
                            payload.to_string().as_bytes(),
                        ).await;
                    }
                }
            }
        });

        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                    info!("Shelly MQTT connecté (broker {}:{})", mqtt_cfg.host, mqtt_cfg.port);
                    // Demande immédiate de l'état courant via RPC
                    for (req_id, dev) in devices.iter().enumerate() {
                        for ch in 0u8..2 {
                            let payload = serde_json::json!({
                                "id":     req_id * 2 + ch as usize + 100,
                                "src":    RPC_SRC,
                                "method": "Switch.GetStatus",
                                "params": { "id": ch }
                            });
                            let topic = format!("{}/rpc", dev.shelly_id);
                            let _ = client.publish(
                                &topic,
                                QoS::AtMostOnce,
                                false,
                                payload.to_string().as_bytes(),
                            ).await;
                        }
                    }
                }

                Ok(Event::Incoming(Incoming::Publish(msg))) => {
                    let topic = msg.topic.as_str();
                    let payload = match std::str::from_utf8(&msg.payload) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };

                    // Réponse RPC : daly-bms-shelly/rpc
                    if topic == rpc_response_topic {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(payload) {
                            // { "id":..., "src": "{shelly_id}", ..., "result": { "id": 0|1, "output": ..., ... } }
                            let result = match json.get("result") { Some(r) => r, None => continue };
                            let src_id = json["src"].as_str().unwrap_or("");
                            let dev_cfg = match devices.iter().find(|d| d.shelly_id == src_id) {
                                Some(d) => d,
                                None    => continue,
                            };
                            let channel = result["id"].as_u64().unwrap_or(0) as u8;
                            let id = dev_cfg.id;
                            let entry = cache.entry(id).or_default();
                            let ch = if channel == 0 { &mut entry.ch0 } else { &mut entry.ch1 };
                            ch.output      = result["output"].as_bool().unwrap_or(false);
                            ch.power_w     = result["apower"].as_f64().unwrap_or(0.0) as f32;
                            ch.voltage_v   = result["voltage"].as_f64().unwrap_or(0.0) as f32;
                            ch.current_a   = result["current"].as_f64().unwrap_or(0.0) as f32;
                            ch.power_factor = result["pf"].as_f64().unwrap_or(0.0) as f32;
                            ch.energy_wh   = result["aenergy"]["total"].as_f64().unwrap_or(0.0);
                            ch.returned_wh = result["ret_aenergy"]["total"].as_f64().unwrap_or(0.0);

                            let total_power_w = entry.ch0.power_w + entry.ch1.power_w;
                            let snap = ShellyEmSnapshot {
                                id,
                                name:         dev_cfg.name.clone(),
                                shelly_id:    dev_cfg.shelly_id.clone(),
                                timestamp:    Local::now(),
                                channel_0:    entry.ch0.clone(),
                                channel_1:    entry.ch1.clone(),
                                rssi:         entry.rssi,
                                total_power_w,
                            };
                            on_snapshot(snap);
                        }
                        continue;
                    }

                    // Trouver le device par shelly_id (préfixe du topic)
                    let dev_cfg = match devices.iter().find(|d| topic.starts_with(&d.shelly_id)) {
                        Some(d) => d,
                        None    => continue,
                    };
                    let id = dev_cfg.id;
                    let entry = cache.entry(id).or_default();

                    // {shelly_id}/status/switch:0 ou /status/switch:1
                    if topic.ends_with("/status/switch:0") || topic.ends_with("/status/switch:1") {
                        let channel: u8 = if topic.ends_with(":0") { 0 } else { 1 };

                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(payload) {
                            let ch = if channel == 0 { &mut entry.ch0 } else { &mut entry.ch1 };
                            ch.output      = json["output"].as_bool().unwrap_or(false);
                            ch.power_w     = json["apower"].as_f64().unwrap_or(0.0) as f32;
                            ch.voltage_v   = json["voltage"].as_f64().unwrap_or(0.0) as f32;
                            ch.current_a   = json["current"].as_f64().unwrap_or(0.0) as f32;
                            ch.power_factor = json["pf"].as_f64().unwrap_or(0.0) as f32;
                            ch.energy_wh   = json["aenergy"]["total"].as_f64().unwrap_or(0.0);
                            ch.returned_wh = json["ret_aenergy"]["total"].as_f64().unwrap_or(0.0);
                        }

                        let total_power_w = entry.ch0.power_w + entry.ch1.power_w;
                        let snap = ShellyEmSnapshot {
                            id,
                            name:         dev_cfg.name.clone(),
                            shelly_id:    dev_cfg.shelly_id.clone(),
                            timestamp:    Local::now(),
                            channel_0:    entry.ch0.clone(),
                            channel_1:    entry.ch1.clone(),
                            rssi:         entry.rssi,
                            total_power_w,
                        };
                        on_snapshot(snap);

                    } else if topic.ends_with("/info") {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(payload) {
                            entry.rssi = json["wifi"]["rssi"].as_i64()
                                .or_else(|| json["wifi_sta"]["rssi"].as_i64())
                                .map(|v| v as i32);
                        }
                    }
                }

                Ok(_) => {}

                Err(e) => {
                    warn!("Shelly MQTT erreur : {:?}", e);
                    break;
                }
            }
        }

        warn!("Shelly MQTT déconnecté — reconnexion dans 10s");
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}
