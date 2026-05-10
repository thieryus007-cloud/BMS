//! Module Shelly EM — compteurs d'énergie 2 canaux WiFi.
//!
//! Reçoit les payloads MQTT natifs Shelly (shellies/.../emeter/...)
//! et les expose via l'API REST, le dashboard.

pub mod types;
pub mod mqtt;

pub use types::ShellyEmSnapshot;
#[allow(unused_imports)]
pub use types::ShellyChannelData;
pub use mqtt::run_shelly_mqtt_loop;
