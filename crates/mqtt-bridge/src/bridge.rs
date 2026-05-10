//! Logique de bridge bidirectionnel Pi5 (local rumqttd) ↔ NanoPi.
//!
//! Deux demi-bridges indépendants, chacun dans sa propre tâche async :
//!   - `run_nanopi_to_local` : abonné sur NanoPi, republish vers Pi5 local
//!   - `run_local_to_nanopi` : abonné sur Pi5 local, republish vers NanoPi
//!
//! En cas de déconnexion, reconnexion automatique avec backoff.

use crate::{config::BridgeConfig, metrics::BridgeMetrics};
use anyhow::Result;
use rumqttc::{AsyncClient, EventLoop, MqttOptions, QoS};
use std::{sync::Arc, time::Duration};
use tokio::time::sleep;
use tracing::{info, warn};

// =============================================================================
// Topics bridgés depuis NanoPi → Pi5 local (direction "in")
// =============================================================================

fn nanopi_to_local_subscriptions(portal: &str) -> Vec<(String, QoS)> {
    vec![
        // Venus OS données publiées par Venus OS
        (format!("N/{portal}/#"), QoS::AtMostOnce),
        // Données BMS publiées par dbus-mqtt-venus sur NanoPi
        ("santuario/#".to_string(), QoS::AtMostOnce),
        // Shelly bidirectionnel
        ("shellypro2pm-ec62608840a4/#".to_string(), QoS::AtMostOnce),
    ]
}

// =============================================================================
// Topics bridgés depuis Pi5 local → NanoPi (direction "out")
// =============================================================================

fn local_to_nanopi_subscriptions(portal: &str) -> Vec<(String, QoS)> {
    vec![
        // Venus OS commandes (écriture) — QoS 1 pour garantir la livraison
        (format!("W/{portal}/#"), QoS::AtLeastOnce),
        // Venus OS keepalive / requêtes lecture — QoS 1
        (format!("R/{portal}/#"), QoS::AtLeastOnce),
        // Topics santuario envoyés depuis Pi5 vers NanoPi
        ("santuario/heat/#".to_string(),      QoS::AtMostOnce),
        ("santuario/heatpump/#".to_string(),  QoS::AtMostOnce),
        ("santuario/meteo/#".to_string(),     QoS::AtMostOnce),
        ("santuario/switch/#".to_string(),    QoS::AtMostOnce),
        ("santuario/grid/#".to_string(),      QoS::AtMostOnce),
        ("santuario/platform/#".to_string(),  QoS::AtMostOnce),
        ("santuario/inverter/#".to_string(),  QoS::AtMostOnce),
        ("santuario/system/#".to_string(),    QoS::AtMostOnce),
        ("santuario/pvinverter/#".to_string(), QoS::AtMostOnce),
        // Shelly bidirectionnel
        ("shellypro2pm-ec62608840a4/#".to_string(), QoS::AtMostOnce),
    ]
}

// =============================================================================
// Helper : construire MqttOptions
// =============================================================================

fn make_opts(host: &str, port: u16, client_id: &str, keepalive_secs: u16) -> MqttOptions {
    let mut opts = MqttOptions::new(client_id, host, port);
    opts.set_keep_alive(Duration::from_secs(keepalive_secs as u64));
    opts.set_clean_session(false);
    opts.set_max_packet_size(1_048_576, 1_048_576);
    opts
}

// =============================================================================
// Demi-bridge : NanoPi → Pi5 local
// =============================================================================

pub async fn run_nanopi_to_local(cfg: Arc<BridgeConfig>, metrics: Arc<BridgeMetrics>) -> Result<()> {
    let portal = &cfg.portal_id;
    let subs   = nanopi_to_local_subscriptions(portal);
    let reconnect = Duration::from_secs(cfg.reconnect_secs);

    loop {
        info!("[nanopi→local] Connexion {}:{}", cfg.remote_host, cfg.remote_port);

        // Client abonné : se connecte au NanoPi
        let sub_opts = make_opts(&cfg.remote_host, cfg.remote_port,
                                 "dalybms-bridge-sub", cfg.keepalive_secs);
        let (sub_client, mut sub_el) = AsyncClient::new(sub_opts, 64);

        // Client publisher : publie sur Pi5 local
        let pub_opts = make_opts(&cfg.local_host, cfg.local_port,
                                 "dalybms-bridge-pub-n2l", cfg.keepalive_secs);
        let (pub_client, mut pub_el) = AsyncClient::new(pub_opts, 64);

        // Connexion locale (publisher)
        tokio::spawn(async move { while pub_el.poll().await.is_ok() {} });

        // Abonnements sur NanoPi
        for (topic, qos) in &subs {
            if let Err(e) = sub_client.subscribe(topic, *qos).await {
                warn!("[nanopi→local] Subscribe {topic} erreur : {e}");
            }
        }

        metrics.remote_connected.store(true, std::sync::atomic::Ordering::Relaxed);

        // Boucle d'événements NanoPi
        loop {
            match sub_el.poll().await {
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(p))) => {
                    let qos = if p.qos == rumqttc::QoS::AtLeastOnce {
                        QoS::AtLeastOnce
                    } else {
                        QoS::AtMostOnce
                    };
                    if let Err(e) = pub_client
                        .publish(&p.topic, qos, p.retain, p.payload.clone())
                        .await
                    {
                        warn!("[nanopi→local] Republish {} erreur : {e}", p.topic);
                    } else {
                        metrics.msgs_remote_to_local
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("[nanopi→local] Déconnexion NanoPi : {e}");
                    metrics.remote_connected.store(false, std::sync::atomic::Ordering::Relaxed);
                    metrics.reconnects_remote.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    break;
                }
            }
        }

        info!("[nanopi→local] Reconnexion dans {}s…", reconnect.as_secs());
        sleep(reconnect).await;
    }
}

// =============================================================================
// Demi-bridge : Pi5 local → NanoPi
// =============================================================================

pub async fn run_local_to_nanopi(cfg: Arc<BridgeConfig>, metrics: Arc<BridgeMetrics>) -> Result<()> {
    let portal = &cfg.portal_id;
    let subs   = local_to_nanopi_subscriptions(portal);
    let reconnect = Duration::from_secs(cfg.reconnect_secs);

    loop {
        info!("[local→nanopi] Connexion {}:{}", cfg.local_host, cfg.local_port);

        // Client abonné : se connecte au broker local (rumqttd)
        let sub_opts = make_opts(&cfg.local_host, cfg.local_port,
                                 "dalybms-bridge-sub-l2n", cfg.keepalive_secs);
        let (sub_client, mut sub_el) = AsyncClient::new(sub_opts, 64);

        // Client publisher : publie sur NanoPi
        let pub_opts = make_opts(&cfg.remote_host, cfg.remote_port,
                                 "dalybms-bridge-pub", cfg.keepalive_secs);
        let (pub_client, mut pub_el) = AsyncClient::new(pub_opts, 64);

        // Connexion NanoPi (publisher)
        tokio::spawn(async move { while pub_el.poll().await.is_ok() {} });

        // Abonnements sur broker local
        for (topic, qos) in &subs {
            if let Err(e) = sub_client.subscribe(topic, *qos).await {
                warn!("[local→nanopi] Subscribe {topic} erreur : {e}");
            }
        }

        metrics.local_connected.store(true, std::sync::atomic::Ordering::Relaxed);

        // Boucle d'événements local
        loop {
            match sub_el.poll().await {
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(p))) => {
                    let qos = if p.qos == rumqttc::QoS::AtLeastOnce {
                        QoS::AtLeastOnce
                    } else {
                        QoS::AtMostOnce
                    };
                    if let Err(e) = pub_client
                        .publish(&p.topic, qos, p.retain, p.payload.clone())
                        .await
                    {
                        warn!("[local→nanopi] Republish {} erreur : {e}", p.topic);
                    } else {
                        metrics.msgs_local_to_remote
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("[local→nanopi] Déconnexion locale : {e}");
                    metrics.local_connected.store(false, std::sync::atomic::Ordering::Relaxed);
                    metrics.reconnects_local.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    break;
                }
            }
        }

        info!("[local→nanopi] Reconnexion dans {}s…", reconnect.as_secs());
        sleep(reconnect).await;
    }
}
