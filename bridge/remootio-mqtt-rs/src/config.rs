use serde::Deserialize;
use std::path::Path;

fn default_mqtt_host() -> String {
    "127.0.0.1".to_string()
}

fn default_mqtt_port() -> u16 {
    1883
}

fn default_ping_interval_secs() -> u64 {
    60
}

#[derive(Debug, Deserialize, Clone)]
pub struct MqttConfig {
    #[serde(default = "default_mqtt_host")]
    pub host: String,
    #[serde(default = "default_mqtt_port")]
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            host: default_mqtt_host(),
            port: default_mqtt_port(),
            user: None,
            password: None,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct DeviceConfig {
    pub name: String,
    pub ip: String,
    pub api_secret_key: String,
    pub api_auth_key: String,
    #[serde(default = "default_ping_interval_secs")]
    pub ping_interval_secs: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub mqtt: MqttConfig,
    #[serde(rename = "devices", default)]
    pub devices: Vec<DeviceConfig>,
}

pub fn load(path: &Path) -> anyhow::Result<Config> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("lecture de {} impossible: {e}", path.display()))?;
    let cfg: Config = toml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("config TOML invalide ({}): {e}", path.display()))?;
    Ok(cfg)
}
