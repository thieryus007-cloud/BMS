use anyhow::Result;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::bus::AppBus;
use crate::config::MqttConfig;
use crate::types::{MqttOutgoing, MqttQos};

/// Spawn the MQTT subscriber and publisher tasks.
/// Returns the `AsyncClient` so callers can subscribe to topics after connection.
pub async fn spawn(
    cfg: &MqttConfig,
    topics: Vec<String>,
    bus: AppBus,
    mut outgoing_rx: mpsc::Receiver<MqttOutgoing>,
) -> Result<()> {
    let mut opts = MqttOptions::new(&cfg.client_id, &cfg.host, cfg.port);
    opts.set_keep_alive(Duration::from_secs(cfg.keep_alive_secs));
    opts.set_clean_session(false);
    if let (Some(u), Some(p)) = (&cfg.username, &cfg.password) {
        opts.set_credentials(u, p);
    }

    let (client, mut eventloop) = AsyncClient::new(opts, 256);
    let reconnect_delay = Duration::from_secs(cfg.reconnect_delay_secs);

    // --- Publisher task ---
    let pub_client = client.clone();
    tokio::spawn(async move {
        while let Some(msg) = outgoing_rx.recv().await {
            let qos = match msg.qos {
                MqttQos::AtMostOnce  => QoS::AtMostOnce,
                MqttQos::AtLeastOnce => QoS::AtLeastOnce,
            };
            if let Err(e) = pub_client
                .publish(&msg.topic, qos, msg.retain, msg.payload.as_bytes())
                .await
            {
                warn!("MQTT publish error on {}: {e}", msg.topic);
            }
        }
    });

    // --- Event loop (subscriber + dispatcher) ---
    let topics_clone = topics.clone();
    tokio::spawn(async move {
        let mut subscribed = false;
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    info!("MQTT connected — subscribing to {} topics", topics_clone.len());
                    for t in &topics_clone {
                        if let Err(e) = client.subscribe(t, QoS::AtLeastOnce).await {
                            error!("Failed to subscribe {t}: {e}");
                        }
                    }
                    subscribed = true;
                }
                Ok(Event::Incoming(Packet::Publish(p))) if subscribed => {
                    debug!("MQTT rx: {}", p.topic);
                    let msg = crate::types::MqttIncoming {
                        topic: p.topic.to_string(),
                        payload: p.payload.clone(),
                        retain: p.retain,
                    };
                    // Ignore lagged receivers (they'll re-catch up)
                    let _ = bus.mqtt_in.send(msg);
                }
                Ok(_) => {}
                Err(e) => {
                    error!("MQTT connection error: {e} — reconnecting in {}s", reconnect_delay.as_secs());
                    subscribed = false;
                    tokio::time::sleep(reconnect_delay).await;
                }
            }
        }
    });

    Ok(())
}
