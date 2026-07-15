mod config;
mod device;
mod protocol;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rumqttc::{AsyncClient, Event, Incoming, LastWill, MqttOptions, QoS};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

const BRIDGE_AVAILABILITY_TOPIC: &str = "santuario/remootio/bridge/availability";

fn topic_base(name: &str) -> String {
    format!("santuario/remootio/{name}")
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse un topic `santuario/remootio/<name>/set` ou `santuario/remootio/<name>/secondary/set`.
/// Retourne `(name, is_secondary)`.
fn parse_command_topic(topic: &str) -> Option<(&str, bool)> {
    let parts: Vec<&str> = topic.split('/').collect();
    match parts.as_slice() {
        ["santuario", "remootio", name, "set"] => Some((name, false)),
        ["santuario", "remootio", name, "secondary", "set"] => Some((name, true)),
        _ => None,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config_path = std::env::var("REMOOTIO_MQTT_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config.toml"));
    let cfg = config::load(&config_path)?;

    if cfg.devices.is_empty() {
        anyhow::bail!(
            "aucun appareil configuré ([[devices]] manquant dans {})",
            config_path.display()
        );
    }

    let mut mqtt_options = MqttOptions::new("remootio-mqtt-rs", &cfg.mqtt.host, cfg.mqtt.port);
    mqtt_options.set_keep_alive(std::time::Duration::from_secs(30));
    if let (Some(user), Some(password)) = (&cfg.mqtt.user, &cfg.mqtt.password) {
        mqtt_options.set_credentials(user, password);
    }
    mqtt_options.set_last_will(LastWill::new(
        BRIDGE_AVAILABILITY_TOPIC,
        "offline",
        QoS::AtLeastOnce,
        true,
    ));

    let (mqtt_client, mut event_loop) = AsyncClient::new(mqtt_options, 32);

    let (event_tx, mut event_rx) = mpsc::channel::<(String, device::Event)>(64);
    let mut command_txs: HashMap<String, mpsc::Sender<device::Command>> = HashMap::new();

    for dev in &cfg.devices {
        let secret_key = protocol::decode_hex_key(&dev.api_secret_key)
            .map_err(|e| anyhow::anyhow!("api_secret_key invalide pour '{}': {e}", dev.name))?;
        let auth_key = protocol::decode_hex_key(&dev.api_auth_key)
            .map_err(|e| anyhow::anyhow!("api_auth_key invalide pour '{}': {e}", dev.name))?;

        let (cmd_tx, cmd_rx) = mpsc::channel::<device::Command>(8);
        command_txs.insert(dev.name.clone(), cmd_tx);

        let device_cfg = device::DeviceConfig {
            name: dev.name.clone(),
            ip: dev.ip.clone(),
            secret_key,
            auth_key,
            ping_interval: std::time::Duration::from_secs(dev.ping_interval_secs),
        };
        let event_tx = event_tx.clone();
        tokio::spawn(device::run(device_cfg, cmd_rx, event_tx));
    }
    drop(event_tx);

    // Republie l'état/disponibilité par device en MQTT (retained) dès qu'un événement arrive.
    let publisher = mqtt_client.clone();
    tokio::spawn(async move {
        while let Some((name, event)) = event_rx.recv().await {
            let base = topic_base(&name);
            match event {
                device::Event::Authenticated => {
                    let _ = publisher
                        .publish(
                            format!("{base}/availability"),
                            QoS::AtLeastOnce,
                            true,
                            "online",
                        )
                        .await;
                }
                device::Event::Disconnected => {
                    let _ = publisher
                        .publish(
                            format!("{base}/availability"),
                            QoS::AtLeastOnce,
                            true,
                            "offline",
                        )
                        .await;
                }
                device::Event::State(state) => {
                    let payload =
                        serde_json::json!({ "state": state, "ts": now_epoch() }).to_string();
                    info!("[{name}] état -> {state}");
                    let _ = publisher
                        .publish(format!("{base}/state"), QoS::AtLeastOnce, true, payload)
                        .await;
                }
            }
        }
    });

    info!(
        "pont remootio-mqtt démarré ({} appareil(s))",
        cfg.devices.len()
    );

    loop {
        match event_loop.poll().await {
            Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                // rumqttc ne réabonne PAS automatiquement après une reconnexion (session
                // "clean" par défaut) : sans ce réabonnement explicite à chaque ConnAck
                // (y compris la toute première connexion), une coupure réseau laisse le
                // pont "connecté" mais sourd à toute commande MQTT, silencieusement.
                info!("MQTT (re)connecté, réabonnement...");
                for device_topic in [
                    "santuario/remootio/+/set",
                    "santuario/remootio/+/secondary/set",
                ] {
                    if let Err(e) = mqtt_client.subscribe(device_topic, QoS::AtLeastOnce).await {
                        warn!("échec réabonnement à {device_topic}: {e:#}");
                    }
                }
                if let Err(e) = mqtt_client
                    .publish(BRIDGE_AVAILABILITY_TOPIC, QoS::AtLeastOnce, true, "online")
                    .await
                {
                    warn!("échec publication disponibilité du pont: {e:#}");
                }
            }
            Ok(Event::Incoming(Incoming::Publish(publish))) => {
                debug!(
                    "MQTT publish reçu: topic={} payload={:?}",
                    publish.topic,
                    String::from_utf8_lossy(&publish.payload)
                );
                let Some((name, is_secondary)) = parse_command_topic(&publish.topic) else {
                    continue;
                };
                let Some(cmd_tx) = command_txs.get(name) else {
                    warn!("commande reçue pour un appareil inconnu: '{name}'");
                    continue;
                };
                let action = String::from_utf8_lossy(&publish.payload)
                    .trim()
                    .to_lowercase();
                // Les capabilities booléennes de Homey (onoff/button via MQTT Hub) publient
                // "true"/"on"/"1" en plus (voire à la place) du "trigger" attendu — accepté
                // comme synonyme. Le "false"/"off"/"0" du relâchement est un non-événement
                // volontairement ignoré (silencieux) : le déclencher aussi doublerait
                // l'impulsion physique à chaque pression (validé 2026-07-15 avec Homey).
                const FRONT_MONTANT: [&str; 4] = ["trigger", "true", "on", "1"];
                const FRONT_DESCENDANT: [&str; 3] = ["false", "off", "0"];
                let command = if is_secondary {
                    if FRONT_MONTANT.contains(&action.as_str()) {
                        Some(device::Command::TriggerSecondary)
                    } else if FRONT_DESCENDANT.contains(&action.as_str()) {
                        debug!(
                            "[{name}] front descendant '{action}' ignoré sur la sortie secondaire"
                        );
                        None
                    } else {
                        warn!("[{name}] action secondaire inconnue '{action}' (seul 'trigger' est supporté)");
                        None
                    }
                } else {
                    match action.as_str() {
                        "open" => Some(device::Command::Open),
                        "close" => Some(device::Command::Close),
                        "query" => Some(device::Command::Query),
                        _ if FRONT_MONTANT.contains(&action.as_str()) => {
                            Some(device::Command::Trigger)
                        }
                        _ if FRONT_DESCENDANT.contains(&action.as_str()) => {
                            debug!("[{name}] front descendant '{action}' ignoré");
                            None
                        }
                        other => {
                            warn!("[{name}] action inconnue '{other}'");
                            None
                        }
                    }
                };
                if let Some(command) = command {
                    let _ = cmd_tx.send(command).await;
                }
            }
            Ok(_) => {}
            Err(e) => {
                warn!("boucle MQTT interrompue: {e:#}, nouvelle tentative dans 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}
