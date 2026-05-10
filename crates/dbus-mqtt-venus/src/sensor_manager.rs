//! Manager des services D-Bus température — orchestre N capteurs.
//!
//! Reçoit les événements `SensorMqttEvent` depuis `mqtt_source` et les route
//! vers le `SensorServiceHandle` correspondant.  Gère la création dynamique
//! des services D-Bus `com.victronenergy.temperature.{n}` et le watchdog.

use crate::config::{SensorRef, VenusConfig};
use crate::mqtt_source::SensorMqttEvent;
use crate::temperature_service::{SensorServiceHandle, create_temperature_service};
use anyhow::Result;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{error, info, warn};

// =============================================================================
// Manager capteurs température
// =============================================================================

/// Gestionnaire des services D-Bus Venus OS pour tous les capteurs de température.
pub struct SensorManager {
    cfg:         VenusConfig,
    sensor_refs: Vec<SensorRef>,
    services:    HashMap<u8, SensorServiceHandle>,
    rx:          mpsc::Receiver<SensorMqttEvent>,
}

impl SensorManager {
    pub fn new(
        cfg:         VenusConfig,
        sensor_refs: Vec<SensorRef>,
        rx:          mpsc::Receiver<SensorMqttEvent>,
    ) -> Self {
        Self {
            cfg,
            sensor_refs,
            services: HashMap::new(),
            rx,
        }
    }

    /// Boucle principale : traite les événements MQTT et le watchdog.
    pub async fn run(mut self) -> Result<()> {
        if !self.cfg.enabled {
            info!("Service capteurs D-Bus désactivé (enabled = false)");
            while self.rx.recv().await.is_some() {}
            return Ok(());
        }

        let watchdog_dur  = Duration::from_secs(self.cfg.watchdog_sec);
        let republish_dur = Duration::from_secs(self.cfg.republish_sec);
        let mut republish_tick = interval(republish_dur);

        info!(
            dbus_bus     = %self.cfg.dbus_bus,
            prefix       = %self.cfg.service_prefix,
            watchdog_sec = self.cfg.watchdog_sec,
            sensors      = self.sensor_refs.len(),
            "SensorManager démarré"
        );

        loop {
            tokio::select! {
                Some(evt) = self.rx.recv() => {
                    if let Err(e) = self.handle_mqtt_event(evt).await {
                        error!("Erreur traitement événement capteur MQTT : {:#}", e);
                    }
                }

                _ = republish_tick.tick() => {
                    self.republish_and_watchdog(watchdog_dur).await;
                }
            }
        }
    }

    /// Traite un événement MQTT : crée le service si besoin, puis met à jour.
    async fn handle_mqtt_event(&mut self, evt: SensorMqttEvent) -> Result<()> {
        let idx = evt.mqtt_index;

        if !self.services.contains_key(&idx) {
            let handle = self.create_service_for_index(idx).await?;
            self.services.insert(idx, handle);
        }

        if let Some(svc) = self.services.get(&idx) {
            svc.update(&evt.payload).await?;
        }

        Ok(())
    }

    /// Crée un service D-Bus température pour un mqtt_index donné.
    async fn create_service_for_index(&self, idx: u8) -> Result<SensorServiceHandle> {
        let service_suffix  = format!("{}_{}", self.cfg.service_prefix, idx);
        let device_instance = self.device_instance_for_index(idx);
        let product_name    = self.product_name_for_index(idx);
        let custom_name     = self.custom_name_for_index(idx);
        let default_type    = self.temperature_type_for_index(idx);

        create_temperature_service(
            &self.cfg.dbus_bus,
            &service_suffix,
            device_instance,
            product_name,
            custom_name,
            default_type,
        )
        .await
    }

    fn device_instance_for_index(&self, idx: u8) -> u32 {
        for (pos, s) in self.sensor_refs.iter().enumerate() {
            let si = s.mqtt_index.unwrap_or((pos + 1) as u8);
            if si == idx {
                return s.device_instance.unwrap_or(si as u32);
            }
        }
        idx as u32
    }

    fn product_name_for_index(&self, idx: u8) -> String {
        for (pos, s) in self.sensor_refs.iter().enumerate() {
            let si = s.mqtt_index.unwrap_or((pos + 1) as u8);
            if si == idx {
                if let Some(name) = &s.name {
                    return name.clone();
                }
            }
        }
        format!("Temperature Sensor {}", idx)
    }

    fn custom_name_for_index(&self, idx: u8) -> String {
        // Le custom_name par défaut est identique au product_name
        self.product_name_for_index(idx)
    }

    fn temperature_type_for_index(&self, idx: u8) -> i32 {
        for (pos, s) in self.sensor_refs.iter().enumerate() {
            let si = s.mqtt_index.unwrap_or((pos + 1) as u8);
            if si == idx {
                return s.temperature_type.unwrap_or(2); // 2=generic par défaut
            }
        }
        2
    }

    /// Republication forcée (keepalive Venus OS) + vérification watchdog.
    async fn republish_and_watchdog(&self, watchdog_dur: Duration) {
        let now = Instant::now();

        for (idx, svc) in &self.services {
            let last_update = {
                let guard = svc.values.lock().unwrap();
                guard.last_update
            };

            if now.duration_since(last_update) > watchdog_dur {
                if let Err(e) = svc.set_disconnected().await {
                    warn!(index = idx, "Erreur watchdog capteur disconnect : {:#}", e);
                }
            } else if let Err(e) = svc.republish().await {
                warn!(index = idx, "Erreur republication keepalive capteur : {:#}", e);
            }
        }
    }
}
