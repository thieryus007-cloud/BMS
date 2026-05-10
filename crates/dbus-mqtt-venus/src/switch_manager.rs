//! Manager des services D-Bus switch — orchestre N switches/ATS.
//!
//! Reçoit les `SwitchMqttEvent` depuis `mqtt_source` et les route vers
//! le `SwitchServiceHandle` correspondant.
//!
//! Topic MQTT entrant : `santuario/switch/{n}/venus`
//! Service D-Bus      : `com.victronenergy.switch.{prefix}_{n}`
//!
//! ## Contrôle bidirectionnel (switches Tasmota)
//!
//! Si un switch a un `command_topic` configuré dans `[[switches]]`,
//! le manager expose également les chemins `/SwitchableOutput/0/...` sur D-Bus.
//!
//! Lorsque la console Venus OS bascule le switch (écriture D-Bus) :
//!
//! ```text
//! Console Venus → D-Bus set_value(/SwitchableOutput/0/State, 1)
//!   → BusItemLeaf.set_value() → cmd_tx → SwitchManager
//!     → MQTT publish(command_topic, "ON")
//!       → Tasmota switch ON
//!         → stat/{id}/POWER = "ON" → Pi5 → santuario/switch/{n}/venus
//!           → SwitchManager.handle_event() → D-Bus update
//! ```

use crate::config::{MqttRef, SwitchRef, VenusConfig};
use crate::mqtt_source::SwitchMqttEvent;
use crate::switch_service::{SwitchServiceHandle, create_switch_service};
use anyhow::Result;
use rumqttc::{AsyncClient, MqttOptions, QoS};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{error, info, warn};

// =============================================================================
// Manager
// =============================================================================

pub struct SwitchManager {
    cfg:         VenusConfig,
    switch_refs: Vec<SwitchRef>,
    services:    HashMap<u8, SwitchServiceHandle>,
    rx:          mpsc::Receiver<SwitchMqttEvent>,
    mqtt_cfg:    MqttRef,
    /// Client MQTT partagé pour la publication des commandes vers Tasmota.
    /// Initialisé dans `run()` si au moins un switch a un `command_topic`.
    cmd_client:  Option<AsyncClient>,
}

impl SwitchManager {
    pub fn new(
        cfg:         VenusConfig,
        switch_refs: Vec<SwitchRef>,
        rx:          mpsc::Receiver<SwitchMqttEvent>,
        mqtt_cfg:    MqttRef,
    ) -> Self {
        Self {
            cfg,
            switch_refs,
            services: HashMap::new(),
            rx,
            mqtt_cfg,
            cmd_client: None,
        }
    }

    pub async fn run(mut self) -> Result<()> {
        if !self.cfg.enabled {
            info!("Service switch D-Bus désactivé (enabled = false)");
            while self.rx.recv().await.is_some() {}
            return Ok(());
        }

        // Initialiser le client MQTT de commande si des switches sont contrôlables
        let has_controllable = self.switch_refs.iter().any(|s| s.command_topic.is_some());
        if has_controllable {
            self.cmd_client = Some(build_cmd_mqtt_client(&self.mqtt_cfg).await);
        }

        let watchdog_dur  = Duration::from_secs(self.cfg.watchdog_sec);
        let republish_dur = Duration::from_secs(self.cfg.republish_sec);
        let mut tick      = interval(republish_dur);

        let controllable_count = self.switch_refs.iter()
            .filter(|s| s.command_topic.is_some()).count();

        info!(
            dbus_bus          = %self.cfg.dbus_bus,
            switches          = self.switch_refs.len(),
            controllable      = controllable_count,
            watchdog_sec      = self.cfg.watchdog_sec,
            "SwitchManager démarré"
        );

        loop {
            tokio::select! {
                Some(evt) = self.rx.recv() => {
                    if let Err(e) = self.handle_event(evt).await {
                        error!("Erreur événement switch MQTT : {:#}", e);
                    }
                }
                _ = tick.tick() => {
                    self.republish_and_watchdog(watchdog_dur).await;
                }
            }
        }
    }

    async fn handle_event(&mut self, evt: SwitchMqttEvent) -> Result<()> {
        let idx = evt.mqtt_index;
        if !self.is_configured(idx) {
            warn!(index = idx, "Message switch reçu pour index non configuré — ignoré");
            return Ok(());
        }
        if !self.services.contains_key(&idx) {
            let handle = self.create_service(idx).await?;
            self.services.insert(idx, handle);
        }
        if let Some(svc) = self.services.get(&idx) {
            svc.update(&evt.payload).await?;
        }
        Ok(())
    }

    fn is_configured(&self, idx: u8) -> bool {
        self.switch_refs.iter().enumerate().any(|(pos, s)| {
            s.mqtt_index.unwrap_or((pos + 1) as u8) == idx
        })
    }

    async fn create_service(&mut self, idx: u8) -> Result<SwitchServiceHandle> {
        let suffix          = format!("{}_{}", self.cfg.service_prefix, idx);
        let device_instance = self.device_instance_for(idx);
        let product_name    = self.product_name_for(idx);
        let custom_name     = self.custom_name_for(idx);
        let group           = self.group_for(idx);
        let command_topic   = self.command_topic_for(idx);
        let controllable    = command_topic.is_some();

        let mut handle = create_switch_service(
            &self.cfg.dbus_bus,
            &suffix,
            device_instance,
            product_name,
            custom_name,
            group,
            controllable,
        ).await?;

        // Si controllable : prendre le récepteur de commandes et lancer
        // la tâche de publication MQTT vers Tasmota.
        if let (Some(cmd_rx), Some(topic)) = (handle.cmd_rx.take(), command_topic) {
            let client = match &self.cmd_client {
                Some(c) => c.clone(),
                None => {
                    // Ne devrait pas arriver (cmd_client initialisé si controllable)
                    warn!(index = idx, "cmd_client absent alors que command_topic défini — reconnexion");
                    build_cmd_mqtt_client(&self.mqtt_cfg).await
                }
            };
            info!(index = idx, topic = %topic, "Switch contrôlable — tâche commande MQTT lancée");
            tokio::spawn(async move {
                run_command_forwarder(cmd_rx, client, topic).await;
            });
        }

        Ok(handle)
    }

    // ── Accesseurs config ──────────────────────────────────────────────────

    fn device_instance_for(&self, idx: u8) -> u32 {
        for (pos, s) in self.switch_refs.iter().enumerate() {
            let si = s.mqtt_index.unwrap_or((pos + 1) as u8);
            if si == idx { return s.device_instance.unwrap_or(si as u32); }
        }
        idx as u32
    }

    fn product_name_for(&self, idx: u8) -> String {
        for (pos, s) in self.switch_refs.iter().enumerate() {
            let si = s.mqtt_index.unwrap_or((pos + 1) as u8);
            if si == idx { if let Some(n) = &s.name { return n.clone(); } }
        }
        format!("Switch {}", idx)
    }

    fn custom_name_for(&self, idx: u8) -> String {
        for (pos, s) in self.switch_refs.iter().enumerate() {
            let si = s.mqtt_index.unwrap_or((pos + 1) as u8);
            if si == idx {
                if let Some(cn) = &s.custom_name { return cn.clone(); }
                if let Some(n)  = &s.name        { return n.clone(); }
            }
        }
        format!("Switch {}", idx)
    }

    fn group_for(&self, idx: u8) -> String {
        for (pos, s) in self.switch_refs.iter().enumerate() {
            let si = s.mqtt_index.unwrap_or((pos + 1) as u8);
            if si == idx {
                if let Some(g) = &s.group { return g.clone(); }
            }
        }
        String::new()
    }

    fn command_topic_for(&self, idx: u8) -> Option<String> {
        for (pos, s) in self.switch_refs.iter().enumerate() {
            let si = s.mqtt_index.unwrap_or((pos + 1) as u8);
            if si == idx { return s.command_topic.clone(); }
        }
        None
    }

    // ── Watchdog & republication ──────────────────────────────────────────

    async fn republish_and_watchdog(&self, watchdog_dur: Duration) {
        let now = Instant::now();
        for (idx, svc) in &self.services {
            let last = { svc.values.lock().unwrap().last_update };
            if now.duration_since(last) > watchdog_dur {
                if let Err(e) = svc.set_disconnected().await {
                    warn!(index = idx, "Erreur watchdog switch : {:#}", e);
                }
            } else if let Err(e) = svc.republish().await {
                warn!(index = idx, "Erreur republication switch : {:#}", e);
            }
        }
    }
}

// =============================================================================
// Client MQTT pour les commandes
// =============================================================================

/// Crée un client MQTT pour la publication de commandes vers les switches.
/// Lance la boucle d'événements rumqttc en arrière-plan.
async fn build_cmd_mqtt_client(mqtt_cfg: &MqttRef) -> AsyncClient {
    let client_id = format!("dbus-venus-switch-cmd-{}", uuid_short());
    let mut opts  = MqttOptions::new(client_id, &mqtt_cfg.host, mqtt_cfg.port);
    opts.set_keep_alive(Duration::from_secs(30));
    if let (Some(u), Some(p)) = (&mqtt_cfg.username, &mqtt_cfg.password) {
        opts.set_credentials(u, p);
    }

    let (client, mut eventloop) = AsyncClient::new(opts, 32);

    // Boucle rumqttc — requise pour que les publish soient effectivement envoyés
    tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(_) => {}
                Err(e) => {
                    warn!("Switch cmd MQTT eventloop : {:#}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    });

    info!("Client MQTT commandes switch initialisé (broker {}:{})",
        mqtt_cfg.host, mqtt_cfg.port);

    client
}

// =============================================================================
// Tâche de transfert commande D-Bus → MQTT
// =============================================================================

/// Tâche longue durée : reçoit les commandes ON/OFF depuis D-Bus
/// et les publie sur le topic MQTT Tasmota.
///
/// - `cmd_rx` : valeurs 0 (Off) ou 1 (On) envoyées par `BusItemLeaf.set_value()`
/// - `client`  : client MQTT partagé (rumqttc AsyncClient)
/// - `topic`   : topic de commande Tasmota, ex. `cmnd/tongou_3BC764/Power`
async fn run_command_forwarder(
    mut cmd_rx: mpsc::Receiver<i32>,
    client:     AsyncClient,
    topic:      String,
) {
    info!(topic = %topic, "Tâche commande switch démarrée");

    while let Some(state) = cmd_rx.recv().await {
        let payload = if state != 0 { "ON" } else { "OFF" };
        match client.publish(&topic, QoS::AtLeastOnce, false, payload).await {
            Ok(_)  => info!(topic = %topic, payload, "Commande switch → Tasmota envoyée"),
            Err(e) => warn!(topic = %topic, "Échec commande switch MQTT : {:#}", e),
        }
    }

    info!(topic = %topic, "Tâche commande switch terminée (canal fermé)");
}

// =============================================================================
// Utilitaire
// =============================================================================

fn uuid_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{:08x}", t)
}
