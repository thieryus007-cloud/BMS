//! Bridge MQTT — publication périodique vers Mosquitto.
//!
//! ## Topics publiés
//!
//! ```text
//! {prefix}/{bms_id}/soc          → "56.4"
//! {prefix}/{bms_id}/voltage      → "52.53"
//! {prefix}/{bms_id}/current      → "-1.60"
//! {prefix}/{bms_id}/power        → "-84.0"
//! {prefix}/{bms_id}/status       → JSON complet
//! {prefix}/{bms_id}/cells        → JSON tensions
//! {prefix}/{bms_id}/alarms       → JSON alarmes
//! {prefix}/{bms_id}/venus        → JSON format dbus-mqtt-battery (si activé)
//! ```

use crate::ats::AtsSnapshot;
use crate::config::MqttConfig;
use crate::console::{ConsoleEvent, EventDevice};
use crate::et112::Et112Snapshot;
use crate::state::{AppState, VenusHeatpump, VenusMppt, VenusSmartShunt, VenusTemperature};
use crate::tasmota::TasmotaSnapshot;
use chrono::Utc;
use daly_bms_core::types::BmsSnapshot;
use rumqttc::{AsyncClient, Event, MqttOptions, Outgoing, Packet, QoS};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::{interval, timeout};
use tracing::{debug, error, info, trace, warn};

// =============================================================================
// Paramètres MQTT optimisés pour réduire la rétention mémoire
// =============================================================================

/// QoS pour publications périodiques (données temps-réel).
/// AtMostOnce = 0 élimine la file inflight et réduit drastiquement la pression
/// mémoire côté publisher (pas d'attente de PUBACK, pas de retransmission).
const PUB_QOS: QoS = QoS::AtMostOnce;

/// QoS pour les souscriptions. AtMostOnce suffisant pour les données Venus OS
/// qui sont publiées périodiquement toutes les secondes.
const SUB_QOS: QoS = QoS::AtMostOnce;

/// Capacité interne des channels rumqttc. Réduite de 128 → 64 pour limiter
/// la quantité maximale de messages retenus en mémoire en cas de ralentissement.
const EVENTLOOP_CAP: usize = 64;

/// Démarre la tâche de publication MQTT en arrière-plan.
///
/// `addr_map` : table adresse RS485 → identifiant de topic (ex: 0x28 → "1").
/// Permet d'aligner les topics sur la configuration `dbus-mqttbattery` du NanoPi
/// (santuario/bms/1/venus, santuario/bms/2/venus, …).
/// Si l'adresse n'est pas dans la map, on publie avec l'adresse décimale brute.
pub async fn run_mqtt_bridge(state: AppState, cfg: MqttConfig, addr_map: HashMap<u8, String>) {
    if !cfg.enabled {
        info!("MQTT bridge désactivé (enabled = false)");
        return;
    }

    info!(
        host = %cfg.host,
        port = cfg.port,
        authenticated = cfg.username.is_some() && cfg.password.is_some(),
        "Démarrage MQTT bridge"
    );

    let mut opts = MqttOptions::new(
        format!("daly-bms-{}", uuid::Uuid::new_v4()),
        &cfg.host,
        cfg.port,
    );
    opts.set_keep_alive(Duration::from_secs(30));

    if let (Some(user), Some(pass)) = (&cfg.username, &cfg.password) {
        debug!(username = %user, "MQTT credentials configurés");
        opts.set_credentials(user, pass);
    }

    let (client, mut eventloop) = AsyncClient::new(opts, EVENTLOOP_CAP);

    // Spawner la boucle d'événements MQTT (requis pour rumqttc async).
    // Timeout ajouté pour éviter les blocages infinis qui empêcheraient
    // la libération des buffers réseau internes.
    tokio::spawn(async move {
        loop {
            match timeout(Duration::from_secs(60), eventloop.poll()).await {
                Ok(Ok(Event::Incoming(Packet::ConnAck(_)))) => {
                    trace!("MQTT publisher connecté");
                }
                Ok(Ok(Event::Incoming(Packet::PubAck(_)))) => {
                    trace!("MQTT publisher PUBACK reçu");
                }
                Ok(Ok(Event::Outgoing(Outgoing::Publish(_)))) => {
                    trace!("MQTT publisher PUBLISH sortant");
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    warn!("MQTT eventloop erreur : {:?}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                Err(_) => {
                    warn!("MQTT publisher eventloop timeout (60s) — possible blocage réseau");
                }
            }
        }
    });

    let mut ticker = interval(Duration::from_secs_f64(cfg.publish_interval_sec.max(1.0)));

    loop {
        ticker.tick().await;

        // ── BMS snapshots ─────────────────────────────────────────────────────
        let snapshots = state.latest_snapshots().await;
        for snap in &snapshots {
            let topic_id = addr_map
                .get(&snap.address)
                .cloned()
                .unwrap_or_else(|| snap.address.to_string());
            let topic = format!("{}/bms/{}/venus", cfg.topic_prefix.trim_end_matches('/').rsplit_once('/').map(|(p,_)| p).unwrap_or("santuario"), topic_id);
            let device = match snap.address {
                1 => EventDevice::Bms1,
                3 => EventDevice::Bms3,
                _ => EventDevice::Bms2,
            };
            if state.console_bus.receiver_count() > 0 {
                state.console_bus.emit(ConsoleEvent::mqtt_out(device, &topic, json!({
                    "Soc": snap.soc,
                    "Voltage": snap.dc.voltage,
                    "Current": snap.dc.current,
                })));
            }
            if let Err(e) = publish_snapshot(&client, &cfg, snap, &topic_id).await {
                error!("MQTT publish BMS erreur : {:?}", e);
            }
        }

        // ── ET112 snapshots → topic {service_type}/{mqtt_index}/venus ──────
        let et112_snaps = state.et112_latest_all().await;
        for snap in &et112_snaps {
            // Résoudre le mqtt_index, position et service_type depuis la config
            let dev_cfg = state.config.et112.devices
                .iter()
                .find(|d| d.parsed_address() == snap.address);
            let idx          = dev_cfg.and_then(|d| d.mqtt_index).unwrap_or(snap.address);
            let position     = dev_cfg.map(|d| d.position).unwrap_or(1);
            let service_type = dev_cfg.map(|d| d.service_type.as_str()).unwrap_or("pvinverter");
            if let Err(e) = publish_et112_snapshot(&client, &cfg, snap, idx, position, service_type).await {
                error!("MQTT publish ET112 erreur : {:?}", e);
            }
        }

        // ── Irradiance PRALRAN → santuario/irradiance/raw ────────────────────
        // Même topic que l'ancien irradiance_reader.py → Node-RED inchangé.
        if let Some(snap) = state.latest_irradiance().await {
            if let Err(e) = publish_irradiance(&client, &cfg, snap.irradiance_wm2).await {
                error!("MQTT publish irradiance erreur : {:?}", e);
            }
        }

        // ── ATS CHINT → santuario/switch/{idx}/venus ──────────────────────
        if let Some(ats_snap) = state.ats_latest().await {
            if let Some(ats_cfg) = state.config.ats.as_ref() {
                if ats_cfg.enabled {
                    if let Err(e) = publish_ats_snapshot(&client, &cfg, &ats_snap, ats_cfg.mqtt_index).await {
                        error!("MQTT publish ATS erreur : {:?}", e);
                    }
                }
            }
        }

        // ── Tasmota → forward Venus OS switch/acload si mqtt_index défini ──
        let tasmota_snaps = state.tasmota_latest_all().await;
        for snap in &tasmota_snaps {
            let dev_cfg = state.config.tasmota.devices
                .iter()
                .find(|d| d.id == snap.id);
            if let Some(dev) = dev_cfg {
                if let Some(idx) = dev.mqtt_index {
                    let svc = dev.service_type.as_str();
                    if let Err(e) = publish_tasmota_snapshot(&client, &cfg, snap, idx, svc).await {
                        error!("MQTT publish Tasmota erreur : {:?}", e);
                    }
                }
            }
        }
    }
}

/// Publie la valeur d'irradiance sur `santuario/irradiance/raw` (retain=true).
///
/// Même format que l'ancien `irradiance_reader.py` — entier W/m² en string.
async fn publish_irradiance(
    client: &AsyncClient,
    cfg: &MqttConfig,
    irradiance_wm2: f32,
) -> anyhow::Result<()> {
    let base = cfg.topic_prefix
        .rsplit_once('/')
        .map(|(prefix, _)| prefix)
        .unwrap_or("santuario");
    let topic = format!("{}/irradiance/raw", base);
    let payload = format!("{:.0}", irradiance_wm2);
    client
        .publish(&topic, PUB_QOS, true, payload)
        .await?;
    Ok(())
}

/// Publie un snapshot ET112 sur le topic `santuario/{service_type}/{idx}/venus`.
///
/// service_type = "pvinverter" → topic pvinverter/{idx}/venus  (PvinverterPayload)
/// service_type = "grid"       → topic grid/{idx}/venus        (GridPayload → com.victronenergy.grid)
/// service_type = "acload"     → topic grid/{idx}/venus        (GridPayload → com.victronenergy.acload)
/// service_type = "heatpump"   → topic heatpump/{idx}/venus    (HeatpumpPayload)
async fn publish_et112_snapshot(
    client: &AsyncClient,
    cfg: &MqttConfig,
    snap: &Et112Snapshot,
    mqtt_index: u8,
    position: u8,
    service_type: &str,
) -> anyhow::Result<()> {
    let base = cfg.topic_prefix
        .rsplit_once('/')
        .map(|(prefix, _)| prefix)
        .unwrap_or("santuario");

    let topic_prefix = match service_type {
        "grid" | "acload" => "grid",   // → com.victronenergy.grid / acload (selon service_type côté NanoPi)
        "heatpump"        => "heatpump",
        _                 => "pvinverter",
    };
    let topic = format!("{}/{}/{}/venus", base, topic_prefix, mqtt_index);

    let payload = if service_type == "heatpump" {
        // HeatpumpPayload — l'ET112 mesure la consommation AC de la PAC
        json!({
            "Ac": {
                "Power":  snap.power_w,
                "Energy": { "Forward": snap.energy_import_kwh() }
            },
            "Position":    position,   // 1=AC Output
            "State":       0,          // 0=Off/unknown (l'ET112 ne connaît pas l'état)
            "ProductName": snap.name,
            "CustomName":  snap.name,
        })
    } else {
        // PvinverterPayload / GridPayload — format complet L1
        json!({
            "Ac": {
                "L1": {
                    "Voltage": snap.voltage_v,
                    "Current": snap.current_a,
                    "Power":   snap.power_w,
                    "Energy": {
                        "Forward": snap.energy_import_kwh(),
                        "Reverse": snap.energy_export_kwh()
                    }
                },
                "Power":  snap.power_w,
                "Energy": {
                    "Forward": snap.energy_import_kwh(),
                    "Reverse": snap.energy_export_kwh()
                }
            },
            "StatusCode":           7,   // Running
            "ErrorCode":            0,   // No Error
            "Position":             position,
            "IsGenericEnergyMeter": 1,
            "ProductName":          snap.name,
            "CustomName":           snap.name,
        })
    };

    client
        .publish(&topic, PUB_QOS, true, serde_json::to_vec(&payload)?)
        .await?;

    Ok(())
}

/// Publie l'état de l'ATS sur `santuario/switch/{idx}/venus` (retain=true).
///
/// Format compatible SwitchPayload de dbus-mqtt-venus :
/// ```json
/// { "State": 1, "Position": 0, "ProductName": "ATS CHINT", "CustomName": "..." }
/// ```
///
/// Mapping :
/// - Position : 0=AC1/Réseau, 1=AC2/Onduleur
/// - State    : 0=inactive, 1=active, 2=alerted (défaut)
async fn publish_ats_snapshot(
    client:     &AsyncClient,
    cfg:        &MqttConfig,
    snap:       &AtsSnapshot,
    mqtt_index: u8,
) -> anyhow::Result<()> {
    let base = cfg.topic_prefix
        .rsplit_once('/')
        .map(|(prefix, _)| prefix)
        .unwrap_or("santuario");

    let topic = format!("{}/switch/{}/venus", base, mqtt_index);

    let position = snap.active_source.venus_position();
    let state_val = snap.active_source.venus_state(&snap.fault);

    let payload = json!({
        "Position":    position,
        "State":       state_val,
        "ProductName": snap.name,
        "CustomName":  snap.name,
    });

    client
        .publish(&topic, PUB_QOS, true, serde_json::to_vec(&payload)?)
        .await?;

    Ok(())
}

/// Publie un snapshot Tasmota vers Venus OS.
///
/// service_type = "switch" → topic `santuario/switch/{idx}/venus`  (SwitchPayload)
/// service_type = "acload" → topic `santuario/grid/{idx}/venus`    (GridPayload)
async fn publish_tasmota_snapshot(
    client: &AsyncClient,
    cfg: &MqttConfig,
    snap: &TasmotaSnapshot,
    mqtt_index: u8,
    service_type: &str,
) -> anyhow::Result<()> {
    let base = cfg.topic_prefix
        .rsplit_once('/')
        .map(|(prefix, _)| prefix)
        .unwrap_or("santuario");

    let (topic_prefix, payload) = if service_type == "acload" {
        let topic = format!("{}/grid/{}/venus", base, mqtt_index);
        let p = json!({
            "Ac/L1/Power":   snap.power_w,
            "Ac/L1/Voltage": snap.voltage_v,
            "Ac/L1/Current": snap.current_a,
            "ProductName":   snap.name,
            "CustomName":    snap.name,
        });
        (topic, p)
    } else {
        // switch (défaut)
        let topic = format!("{}/switch/{}/venus", base, mqtt_index);
        let p = json!({
            "State":       if snap.power_on { 1 } else { 0 },
            "Position":    1,
            "ProductName": snap.name,
            "CustomName":  snap.name,
        });
        (topic, p)
    };

    client
        .publish(&topic_prefix, PUB_QOS, true, serde_json::to_vec(&payload)?)
        .await?;

    Ok(())
}

/// Publie un snapshot complet sur tous les topics d'un BMS.
///
/// `topic_id` : identifiant résolu (ex: "1" pour 0x28, "2" pour 0x29).
async fn publish_snapshot(
    client: &AsyncClient,
    cfg: &MqttConfig,
    snap: &BmsSnapshot,
    topic_id: &str,
) -> anyhow::Result<()> {
    let prefix = format!("{}/{}", cfg.topic_prefix, topic_id);

    // Scalaires
    publish_str(client, &format!("{}/soc",     prefix), &format!("{:.1}", snap.soc)).await;
    publish_str(client, &format!("{}/voltage", prefix), &format!("{:.2}", snap.dc.voltage)).await;
    publish_str(client, &format!("{}/current", prefix), &format!("{:.1}", snap.dc.current)).await;
    publish_str(client, &format!("{}/power",   prefix), &format!("{:.1}", snap.dc.power)).await;

    // JSON status complet
    let status_json = serde_json::to_string(snap)?;
    client
        .publish(format!("{}/status", prefix), PUB_QOS, true, status_json)
        .await?;

    // JSON cellules
    let cells_json = serde_json::to_string(&snap.voltages)?;
    client
        .publish(format!("{}/cells", prefix), PUB_QOS, false, cells_json)
        .await?;

    // JSON alarmes
    let alarms_json = serde_json::to_string(&snap.alarms)?;
    client
        .publish(format!("{}/alarms", prefix), PUB_QOS, false, alarms_json)
        .await?;

    // Format Venus OS (dbus-mqtt-battery)
    let venus_payload = build_venus_payload(snap);
    let venus_json = serde_json::to_string(&venus_payload)?;
    client
        .publish(format!("{}/venus", prefix), PUB_QOS, true, venus_json)
        .await?;

    Ok(())
}

async fn publish_str(client: &AsyncClient, topic: &str, value: &str) {
    if let Err(e) = client
        .publish(topic, PUB_QOS, false, value.to_string())
        .await
    {
        warn!("MQTT publish failed on {topic}: {e}");
    }
}

/// Extrait le numéro entier d'un identifiant de cellule ("C3" → 3, "Cell3" → 3).
fn cell_id_to_int(id: &str) -> u32 {
    id.trim_start_matches("Cell")
      .trim_start_matches('C')
      .parse()
      .unwrap_or(0)
}

/// Construit le payload au format dbus-mqtt-battery (Venus OS).
///
/// Compatible avec https://github.com/mr-manuel/venus-os_dbus-mqtt-battery
///
/// IMPORTANT : seuls les champs reconnus par dbus-mqtt-battery sont inclus.
/// Les champs inconnus (Voltages/sum, Balances, TimeToSoC, Soh, Heating) provoquent
/// une exception Python dans le callback MQTT → first_data_received reste False → timeout.
fn build_venus_payload(snap: &BmsSnapshot) -> serde_json::Value {
    json!({
        "Dc": {
            "Power":       snap.dc.power,
            "Voltage":     snap.dc.voltage,
            "Current":     snap.dc.current,
            "Temperature": snap.dc.temperature,
        },
        "InstalledCapacity":  snap.installed_capacity,
        "ConsumedAmphours":   snap.consumed_amphours,
        "Capacity":           snap.bms_reported_capacity_ah,
        "Soc":                snap.soc,
        "TimeToGo":           snap.time_to_go,
        "Balancing":          snap.balancing,
        "SystemSwitch":       snap.system_switch,
        "Alarms": {
            "LowVoltage":             snap.alarms.low_voltage,
            "HighVoltage":            snap.alarms.high_voltage,
            "LowSoc":                 snap.alarms.low_soc,
            "HighChargeCurrent":      snap.alarms.high_charge_current,
            "HighDischargeCurrent":   snap.alarms.high_discharge_current,
            "HighCurrent":            snap.alarms.high_current,
            "CellImbalance":          snap.alarms.cell_imbalance,
            "HighChargeTemperature":  snap.alarms.high_charge_temperature,
            "LowChargeTemperature":   snap.alarms.low_charge_temperature,
            "LowCellVoltage":         snap.alarms.low_cell_voltage,
            "LowTemperature":         snap.alarms.low_temperature,
            "HighTemperature":        snap.alarms.high_temperature,
            "FuseBlown":              snap.alarms.fuse_blown,
        },
        "System": {
            // Entiers 1-based requis par dbus-mqtt-battery
            "MinVoltageCellId":               cell_id_to_int(&snap.system.min_voltage_cell_id),
            "MinCellVoltage":                 snap.system.min_cell_voltage,
            "MaxVoltageCellId":               cell_id_to_int(&snap.system.max_voltage_cell_id),
            "MaxCellVoltage":                 snap.system.max_cell_voltage,
            "MinTemperatureCellId":           cell_id_to_int(&snap.system.min_temperature_cell_id),
            "MinCellTemperature":             snap.system.min_cell_temperature,
            "MaxTemperatureCellId":           cell_id_to_int(&snap.system.max_temperature_cell_id),
            "MaxCellTemperature":             snap.system.max_cell_temperature,
            "NrOfCellsPerBattery":            snap.system.nr_of_cells_per_battery,
            "NrOfModulesOnline":              snap.system.nr_of_modules_online,
            "NrOfModulesOffline":             snap.system.nr_of_modules_offline,
            "NrOfModulesBlockingCharge":      snap.system.nr_of_modules_blocking_charge,
            "NrOfModulesBlockingDischarge":   snap.system.nr_of_modules_blocking_discharge,
        },
        // AllowToCharge / AllowToDischarge volontairement figés à 1 :
        // on ne veut pas que Venus OS (systemcalc) transmette ces signaux aux MPPT.
        "Io": {
            "AllowToCharge":    1,
            "AllowToDischarge": 1,
            "AllowToBalance":   snap.io.allow_to_balance,
            "ExternalRelay":    snap.io.external_relay,
        },
    })
}

// =============================================================================
// MQTT Subscriber — Réception des données Venus OS
// =============================================================================

/// Démarre un abonnement MQTT pour recevoir les données Venus OS.
///
/// Cette tâche écoute les topics :
/// - `santuario/meteo/venus` → MPPT SolarCharger (puissance, production kWh)
/// - `santuario/heat/*/venus` → Capteurs de température
/// - `santuario/heatpump/*/venus` → PAC/Chauffe-eau (optionnel)
/// - `santuario/system/venus` → SmartShunt (SOC, tension, courant)
pub async fn start_venus_mqtt_subscriber(state: AppState, cfg: MqttConfig) {
    if !cfg.enabled {
        return;
    }

    let mut opts = MqttOptions::new(
        format!("daly-bms-venus-sub-{}", uuid::Uuid::new_v4()),
        &cfg.host,
        cfg.port,
    );
    opts.set_keep_alive(Duration::from_secs(30));

    if let (Some(user), Some(pass)) = (&cfg.username, &cfg.password) {
        opts.set_credentials(user, pass);
    }

    let (client, mut eventloop) = AsyncClient::new(opts, EVENTLOOP_CAP);

    // S'abonner aux topics Venus OS — QoS 0 pour éviter la rétention inflight
    let topics = vec![
        ("santuario/meteo/venus", SUB_QOS),
        ("santuario/inverter/venus", SUB_QOS),
        ("santuario/heat/+/venus", SUB_QOS),
        ("santuario/heatpump/+/venus", SUB_QOS),
        ("santuario/system/venus", SUB_QOS),
        ("santuario/em/metrics", SUB_QOS),
        ("santuario/em/water_heater", SUB_QOS),
    ];

    for (topic, qos) in &topics {
        if let Err(e) = client.subscribe(*topic, *qos).await {
            warn!("MQTT subscribe erreur pour {}: {:?}", topic, e);
        } else {
            debug!("MQTT abonné à {}", topic);
        }
    }

    // Boucle de réception avec timeout pour éviter les blocages infinis
    loop {
        match timeout(Duration::from_secs(60), eventloop.poll()).await {
            Ok(Ok(Event::Incoming(Packet::ConnAck(_)))) => {
                info!("MQTT Venus connecté/reconnecté — réabonnement aux topics");
                if state.console_bus.receiver_count() > 0 {
                    state.console_bus.emit(ConsoleEvent::system("MQTT Venus", "Connecté/reconnecté — réabonnement aux topics"));
                }
                for (topic, qos) in &topics {
                    if let Err(e) = client.subscribe(*topic, *qos).await {
                        warn!("MQTT re-subscribe erreur pour {}: {:?}", topic, e);
                    }
                }
            }
            Ok(Ok(Event::Incoming(Packet::Publish(p)))) => {
                let topic = &p.topic;
                let payload_str = std::str::from_utf8(&p.payload).unwrap_or("");
                debug!("MQTT reçu {} = {}", topic, payload_str);

                // Parser directement depuis les bytes sans allocation UTF-8 intermédiaire
                if let Ok(json) = serde_json::from_slice::<Value>(&p.payload) {
                    // Console MQTT IN — device inferred from topic.
                    // Skip si pas d'abonné : évite un clone() complet du JSON
                    // (peut être lourd) à chaque message MQTT entrant.
                    if state.console_bus.receiver_count() > 0 {
                        let ev_device = if topic.contains("/meteo/") {
                            EventDevice::Solar
                        } else if topic.contains("/inverter/") {
                            EventDevice::Inverter
                        } else if topic.contains("/system/") {
                            EventDevice::SmartShunt
                        } else if topic.contains("/heatpump/") {
                            EventDevice::WaterHeater
                        } else {
                            EventDevice::Venus
                        };
                        state.console_bus.emit(ConsoleEvent::mqtt_in(ev_device, topic, json.clone()));
                    }

                    if topic == "santuario/meteo/venus" {
                        handle_meteo_topic(&state, &json).await;
                    } else if topic.starts_with("santuario/heat/") && topic.ends_with("/venus") {
                        handle_temperature_topic(&state, &json).await;
                    } else if topic.starts_with("santuario/heatpump/") && topic.ends_with("/venus") {
                        handle_heatpump_topic(&state, topic, &json).await;
                    } else if topic == "santuario/system/venus" {
                        handle_system_topic(&state, &json).await;
                    } else if topic == "santuario/inverter/venus" {
                        handle_inverter_topic(&state, &json).await;
                    } else if topic == "santuario/em/metrics" {
                        handle_em_metrics_topic(&state, &json);
                    } else if topic == "santuario/em/water_heater" {
                        handle_wh_metrics_topic(&state, &json);
                    }
                }
            }
            Ok(Ok(Event::Outgoing(_))) => {
                // Ignorer les ACK d'envoi
            }
            Ok(Ok(_)) => {
                // Autres variants Event::Incoming (PingResp, SubAck, etc.) — non actionnables
            }
            Ok(Err(e)) => {
                warn!("MQTT Venus eventloop erreur (reconnexion dans 5s) : {:?}", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Err(_) => {
                warn!("MQTT Venus eventloop timeout (60s) — possible blocage réseau");
            }
        }
    }
}

/// Traite le topic `santuario/meteo/venus`
///
/// Payload format v1 (legacy) : { "Irradiance": 750, "TodaysYield": 12.5, "MpptPower": 2500 }
/// Payload format v2 (étendu) : { ..., "Mppts": [
///   { "Instance": 273, "State": "Float", "PvVoltage": 72.5, "DcCurrent": 12.3, "Power": 1250, "YieldToday": 8.5 },
///   { "Instance": 289, ... }
/// ] }
async fn handle_meteo_topic(state: &AppState, json: &Value) {
    let irradiance = json.get("Irradiance").and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(0.0);
    let yield_kwh  = json.get("TodaysYield").and_then(|v| v.as_f64()).map(|v| v as f32);
    let mppt_power = json.get("MpptPower").and_then(|v| v.as_f64()).map(|v| v as f32);
    if state.console_bus.receiver_count() > 0 {
        state.console_bus.emit(ConsoleEvent::state(EventDevice::Solar, "Solaire/MPPT update", json!({
            "irradiance_wm2": irradiance,
            "yield_today_kwh": yield_kwh,
            "mppt_power_w": mppt_power,
        })));
    }

    // Format v2 : tableau Mppts avec données individuelles par chargeur.
    // On remplace toute la map en une seule opération pour purger les entrées
    // orphelines (ex : MPPT qui n'est plus connecté disparaît du message).
    if let Some(arr) = json.get("Mppts").and_then(|v| v.as_array()) {
        let mut new_mppts = Vec::with_capacity(arr.len());
        for item in arr {
            let instance = item.get("Instance").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let name     = format!("MPPT-{}", instance);
            // State comes as integer from energy-manager; fall back to string for legacy sources.
            let state_str = item.get("State").and_then(|v| {
                if let Some(i) = v.as_i64() {
                    Some(match i {
                        0  => "Off".to_string(),
                        1  => "Low power".to_string(),
                        2  => "Fault".to_string(),
                        3  => "Bulk".to_string(),
                        4  => "Absorption".to_string(),
                        5  => "Float".to_string(),
                        6  => "Storage".to_string(),
                        7  => "Equalize".to_string(),
                        8  => "Passthru".to_string(),
                        9  => "Inverting".to_string(),
                        10 => "Power assist".to_string(),
                        11 => "Power supply".to_string(),
                        _  => format!("State {}", i),
                    })
                } else {
                    v.as_str().map(|s| s.to_string())
                }
            });
            let pv_v     = item.get("PvVoltage").and_then(|v| v.as_f64()).map(|v| v as f32);
            let dc_i     = item.get("DcCurrent").and_then(|v| v.as_f64()).map(|v| v as f32);
            let power    = item.get("Power").and_then(|v| v.as_f64()).map(|v| v as f32);
            let yield_t  = item.get("YieldToday").and_then(|v| v.as_f64()).map(|v| v as f32);
            let max_pw   = item.get("MaxPowerToday").and_then(|v| v.as_f64()).map(|v| v as f32);
            new_mppts.push(VenusMppt {
                instance,
                name,
                power_w: power,
                yield_today_kwh: yield_t,
                max_power_today_w: max_pw,
                state: state_str,
                pv_voltage_v: pv_v,
                dc_current_a: dc_i,
                timestamp: Utc::now(),
            });
        }
        // Sync mppt_yield_kwh with the sum from all chargers.
        // Distinguish "no data" (no MPPT reported yield_today_kwh) from "zero yield"
        // (fields present but sum is 0 — e.g., immediately after midnight reset).
        // The previous `> 0.0` guard kept yesterday's cached value across midnight.
        let has_yield_data = new_mppts.iter().any(|m| m.yield_today_kwh.is_some());
        let total_yield: f32 = new_mppts.iter().filter_map(|m| m.yield_today_kwh).sum();
        state.on_venus_mppts_replace(new_mppts).await;
        if has_yield_data {
            *state.mppt_yield_kwh.write().await = total_yield;
        }
        return; // format v2 traité, pas de fallback nécessaire
    }

    // Format v1 (legacy) : un seul MPPT agrégé
    if let Some(yield_kwh) = yield_kwh {
        let mppt = VenusMppt {
            instance: 0,
            name: "MPPT SolarCharger".to_string(),
            power_w: mppt_power.or(if irradiance > 0.0 { Some(irradiance) } else { None }),
            yield_today_kwh: Some(yield_kwh),
            max_power_today_w: None,
            state: None,
            pv_voltage_v: None,
            dc_current_a: None,
            timestamp: Utc::now(),
        };
        state.on_venus_mppt(mppt).await;
    }
}

/// Traite les topics `santuario/heat/*/venus`
/// Payload : { "Temperature": 15.3, "TemperatureType": 4, "Humidity": 65 }
async fn handle_temperature_topic(state: &AppState, json: &Value) {
    if let Some(temp_c) = json.get("Temperature").and_then(|v| v.as_f64()).map(|v| v as f32) {
        let instance = json
            .get("DeviceInstance")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;

        let humidity = json.get("Humidity").and_then(|v| v.as_f64()).map(|v| v as f32);
        let pressure = json.get("Pressure").and_then(|v| v.as_f64()).map(|v| v as f32);

        let temp_type = json
            .get("TemperatureType")
            .and_then(|v| v.as_u64())
            .and_then(|t| match t {
                0 => Some("Battery"),
                1 => Some("Fridge"),
                2 => Some("Generic"),
                3 => Some("Room"),
                4 => Some("Outdoor"),
                5 => Some("WaterHeater"),
                6 => Some("Freezer"),
                _ => None,
            })
            .unwrap_or("Generic")
            .to_string();

        let temp = VenusTemperature {
            instance,
            name: format!("Temperature {}", instance),
            temp_c: Some(temp_c),
            humidity_percent: humidity,
            pressure_mbar: pressure,
            temp_type,
            connected: true,
            timestamp: Utc::now(),
        };

        state.on_venus_temperature(temp).await;
    }
}

/// Traite le topic `santuario/system/venus`
///
/// Payload energy-manager : { "Soc":75.2, "Voltage":48.3, "Current":5.5, "Power":266.0,
///   "State":2, "TimeToGo":3600, "ChargedTodayKwh":5.2, "DischargedTodayKwh":2.1,
///   "AhChargedToday":108.0, "AhDischargedToday":43.0 }
async fn handle_system_topic(state: &AppState, json: &Value) {
    if let Some(soc) = json.get("Soc").and_then(|v| v.as_f64()).map(|v| v as f32) {
        let voltage    = json.get("Voltage").and_then(|v| v.as_f64()).map(|v| v as f32);
        let current    = json.get("Current").and_then(|v| v.as_f64()).map(|v| v as f32);
        let power      = json.get("Power").and_then(|v| v.as_f64()).map(|v| v as f32);
        // ChargedTodayKwh / DischargedTodayKwh — publiés par energy-manager en kWh
        let energy_in  = json.get("ChargedTodayKwh").and_then(|v| v.as_f64()).map(|v| v as f32);
        let energy_out = json.get("DischargedTodayKwh").and_then(|v| v.as_f64()).map(|v| v as f32);

        // State : entier Victron → libellé
        let state_str = json.get("State").and_then(|v| v.as_u64()).map(|s| match s {
            0 => "Idle".to_string(),
            1 => "Charging".to_string(),
            2 => "Discharging".to_string(),
            _ => format!("State {}", s),
        });

        // TimeToGo : secondes → minutes (None si absent ou ≥ 864000 s = 10 j → "∞")
        let time_to_go_min = json.get("TimeToGo")
            .and_then(|v| v.as_f64())
            .map(|secs| secs as f32 / 60.0)
            .filter(|&m| m < 14400.0); // > 10 jours → ignorer (= en charge)

        // Ah values computed by energy-manager; fall back to local integration when absent.
        let ah_charged    = json.get("AhChargedToday").and_then(|v| v.as_f64()).map(|v| v as f32);
        let ah_discharged = json.get("AhDischargedToday").and_then(|v| v.as_f64()).map(|v| v as f32);

        let shunt = VenusSmartShunt {
            soc_percent: Some(soc),
            voltage_v: voltage,
            current_a: current,
            power_w: power,
            energy_in_kwh: energy_in,   // déjà en kWh
            energy_out_kwh: energy_out, // déjà en kWh
            state: state_str,
            time_to_go_min,
            ah_charged_today:    ah_charged,
            ah_discharged_today: ah_discharged,
            timestamp: Utc::now(),
        };

        state.on_venus_smartshunt(shunt).await;
    }
}

/// Traite les topics `santuario/heatpump/*/venus`
///
/// Payload (depuis Node-RED setwaterheater.json) :
/// ```json
/// { "State": 1, "Temperature": 52.5, "TargetTemperature": 55.0, "Position": 0,
///   "Ac": { "Power": 0.0, "Energy": { "Forward": 0.0 } } }
/// ```
/// State : 0=Off/Vacances, 1=Pompe chaleur, 2=Turbo
async fn handle_heatpump_topic(state: &AppState, topic: &str, json: &Value) {
    // Extraire l'index depuis le topic : santuario/heatpump/{idx}/venus
    let idx: u8 = topic
        .split('/')
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if idx == 0 { return; }

    // Read previous snapshot so missing fields in this MQTT message keep their last
    // known value instead of fabricating a 0. The upstream device is the source of
    // truth — when it omits a field, we must not invent data.
    let prev = state.venus_heatpump_get(idx).await;

    let hp_state    = json.get("State").and_then(|v| v.as_i64()).map(|v| v as i32)
        .unwrap_or_else(|| prev.as_ref().map(|p| p.state).unwrap_or(0));
    let temperature = json.get("Temperature").and_then(|v| v.as_f64()).map(|v| v as f32)
        .or_else(|| prev.as_ref().and_then(|p| p.temperature));
    let target_temp = json.get("TargetTemperature").and_then(|v| v.as_f64()).map(|v| v as f32)
        .or_else(|| prev.as_ref().and_then(|p| p.target_temperature));
    let position    = json.get("Position").and_then(|v| v.as_i64()).map(|v| v as i32)
        .unwrap_or_else(|| prev.as_ref().map(|p| p.position).unwrap_or(0));
    let ac_power    = json.get("Ac").and_then(|a| a.get("Power")).and_then(|v| v.as_f64()).map(|v| v as f32)
        .unwrap_or_else(|| prev.as_ref().map(|p| p.ac_power).unwrap_or(0.0));
    let ac_energy   = json
        .get("Ac").and_then(|a| a.get("Energy"))
        .and_then(|e| e.get("Forward")).and_then(|v| v.as_f64()).map(|v| v as f32)
        .unwrap_or_else(|| prev.as_ref().map(|p| p.ac_energy_forward).unwrap_or(0.0));

    let hp = VenusHeatpump {
        mqtt_index:          idx,
        state:               hp_state,
        temperature,
        target_temperature:  target_temp,
        ac_power,
        ac_energy_forward:   ac_energy,
        position,
        connected:           true,
        timestamp:           Utc::now(),
    };

    debug!(index = idx, state = hp_state, "Heatpump MQTT reçu");
    if state.console_bus.receiver_count() > 0 {
        let mode_lbl = match hp_state { 0 => "Vacances", 1 => "HEAT_PUMP", 2 => "TURBO", _ => "Inconnu" };
        let temp_str = temperature.map(|t| format!(" | {:.1}°C", t)).unwrap_or_default();
        let tgt_str  = target_temp.map(|t| format!(" → {:.1}°C", t)).unwrap_or_default();
        state.console_bus.emit(ConsoleEvent::state(EventDevice::WaterHeater, &format!("Chauffe-eau idx={idx} — {mode_lbl}{temp_str}{tgt_str}"), json!({
            "idx": idx, "state": hp_state, "mode": mode_lbl,
            "temperature_c": temperature, "target_c": target_temp,
            "power_w": ac_power, "energy_kwh": ac_energy,
        })));
    }
    state.on_venus_heatpump(hp).await;
}

/// Traite le topic `santuario/inverter/venus`
///
/// Payload v1 : { "Voltage": 48.32, "Current": 5.5, "Power": 250.0, "AcVoltage": 230.0, "AcCurrent": 1.09, "AcPower": 250.0, "State": "on", "Mode": "inverter" }
/// Payload v2 : { ..., "AcFrequency": 50.03, "IgnoreAcIn": 0, "VebusState": 244 }
///   VebusState : entier VEBus → libellé État (240=External, 241=Active, 243=Bulk, 244=Absorption, 245=Float,
///                 246=Storage, 249=Passthru, 250=Inverting, 252=Power assist, 254=Charge, 255=Inverter Only, etc.)
async fn handle_inverter_topic(state: &AppState, json: &Value) {
    let voltage    = json.get("Voltage").and_then(|v| v.as_f64()).map(|v| v as f32);
    let current    = json.get("Current").and_then(|v| v.as_f64()).map(|v| v as f32);
    let power      = json.get("Power").and_then(|v| v.as_f64()).map(|v| v as f32);
    let ac_voltage = json.get("AcVoltage").and_then(|v| v.as_f64()).map(|v| v as f32);
    let ac_current = json.get("AcCurrent").and_then(|v| v.as_f64()).map(|v| v as f32);
    let ac_power   = json.get("AcPower").and_then(|v| v.as_f64()).map(|v| v as f32);

    // Fréquence AC sortie (Hz)
    let ac_freq = json.get("AcFrequency").and_then(|v| v.as_f64()).map(|v| v as f32);

    // IgnoreAcIn1 : 0=normal, 1=ignoré (mode îlotage forcé)
    let ac_in_ignore = json.get("IgnoreAcIn").and_then(|v| v.as_u64()).map(|v| v != 0);

    // VebusState → libellé lisible (VEBus numeric state)
    let state_str = if let Some(vs) = json.get("VebusState").and_then(|v| v.as_u64()) {
        match vs {
            0   => "Off".to_string(),
            1   => "Low Power".to_string(),
            2   => "Fault".to_string(),
            3   => "Bulk".to_string(),
            4   => "Absorption".to_string(),
            5   => "Float".to_string(),
            6   => "Storage".to_string(),
            7   => "Equalize".to_string(),
            8   => "Passthru".to_string(),
            9   => "Inverting".to_string(),
            10  => "Power assist".to_string(),
            11  => "Power supply".to_string(),
            244 => "Absorption".to_string(),
            245 => "Float".to_string(),
            246 => "Storage".to_string(),
            249 => "Passthru".to_string(),
            250 => "Inverting".to_string(),
            252 => "Bulk".to_string(),
            _   => json.get("State").and_then(|v| v.as_str()).unwrap_or("—").to_string(),
        }
    } else {
        json.get("State").and_then(|v| v.as_str()).unwrap_or("unknown").to_string()
    };

    let mode_str = json.get("Mode").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();

    let inverter = crate::state::VenusInverter {
        voltage_v: voltage,
        current_a: current,
        power_w: power,
        ac_output_voltage_v: ac_voltage,
        ac_output_current_a: ac_current,
        ac_output_power_w: ac_power,
        ac_out_frequency_hz: ac_freq,
        ac_in_ignore,
        state: state_str,
        mode: mode_str,
        timestamp: Utc::now(),
    };

    state.on_venus_inverter(inverter).await;
}

/// Traite le topic `santuario/em/metrics` — métriques système energy-manager.
///
/// Payload JSON attendu (tous champs optionnels) :
/// ```json
/// { "cpu_percent": 4.2, "cpu_temp_c": 51.0, "memory_percent": 35.0,
///   "mem_used_mb": 280, "swap_used_mb": 0, "disk_percent": 27.0,
///   "load_avg_1m": 0.4, "load_avg_5m": 0.6, "load_avg_15m": 0.5,
///   "net_rx_bps": 1234, "net_tx_bps": 567 }
/// ```
fn handle_em_metrics_topic(state: &AppState, json: &Value) {
    let Some(store) = &state.metrics_store else { return };
    let f = |k: &str| json.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
    let payload = crate::redb_writes::EmMetricsPayload {
        cpu_percent:    f("cpu_percent"),
        cpu_temp_c:     f("cpu_temp_c"),
        memory_percent: f("memory_percent"),
        mem_used_mb:    f("mem_used_mb"),
        swap_used_mb:   f("swap_used_mb"),
        disk_percent:   f("disk_percent"),
        load_avg_1m:    f("load_avg_1m"),
        load_avg_5m:    f("load_avg_5m"),
        load_avg_15m:   f("load_avg_15m"),
        net_rx_bps:     f("net_rx_bps"),
        net_tx_bps:     f("net_tx_bps"),
    };
    crate::redb_writes::write_em_metrics(&store.writer(), &state.redb_rl, &payload);
}

/// Traite le topic `santuario/em/water_heater` — état chauffe-eau LG ThinQ.
///
/// Payload : `{ "current_temp_c": 48.0, "target_temp_c": 50.0, "mode": 1 }`
fn handle_wh_metrics_topic(state: &AppState, json: &Value) {
    let Some(store) = &state.metrics_store else { return };
    let f = |k: &str| json.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
    let i = |k: &str| json.get(k).and_then(|v| v.as_i64()).map(|v| v as i32);
    let payload = crate::redb_writes::WhMetricsPayload {
        current_temp_c: f("current_temp_c"),
        target_temp_c:  f("target_temp_c"),
        mode:           i("mode"),
    };
    crate::redb_writes::write_wh_metrics(&store.writer(), &state.redb_rl, &payload);
}
