//! Chargement de la section [mqtt_bridge] depuis Config.toml.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct BridgeConfig {
    /// Broker local (rumqttd) — ex: "127.0.0.1"
    #[serde(default = "default_local_host")]
    pub local_host: String,

    /// Port broker local
    #[serde(default = "default_local_port")]
    pub local_port: u16,

    /// Broker distant NanoPi — ex: "192.168.1.120"
    #[serde(default = "default_remote_host")]
    pub remote_host: String,

    /// Port broker NanoPi
    #[serde(default = "default_remote_port")]
    pub remote_port: u16,

    /// Préfixe Venus OS portal_id — ex: "c0619ab9929a"
    #[serde(default = "default_portal_id")]
    pub portal_id: String,

    /// Délai de reconnexion en secondes
    #[serde(default = "default_reconnect_secs")]
    pub reconnect_secs: u64,

    /// Keep-alive en secondes
    #[serde(default = "default_keepalive_secs")]
    pub keepalive_secs: u16,
}

fn default_local_host()    -> String { "127.0.0.1".into() }
fn default_local_port()    -> u16    { 1883 }
fn default_remote_host()   -> String { "192.168.1.120".into() }
fn default_remote_port()   -> u16    { 1883 }
fn default_portal_id()     -> String { "c0619ab9929a".into() }
fn default_reconnect_secs() -> u64   { 30 }
fn default_keepalive_secs() -> u16   { 60 }

/// Enveloppe pour parser uniquement [mqtt_bridge] dans Config.toml.
#[derive(Deserialize)]
struct Wrapper {
    mqtt_bridge: BridgeConfig,
}

pub fn load(path: &Path) -> Result<BridgeConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Lecture config : {}", path.display()))?;
    let w: Wrapper = toml::from_str(&raw)
        .with_context(|| "Parse [mqtt_bridge] dans config TOML")?;
    Ok(w.mqtt_bridge)
}
