use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};

use crate::bus::AppBus;
use crate::config::LgThinqConfig;
use crate::types::{LiveEvent, WaterHeaterMode};

// ---------------------------------------------------------------------------
// API response types — ThinQ EIC API v2
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LgStateResponse {
    response: LgStateResponseData,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LgStateResponseData {
    water_heater_job_mode: Option<WaterHeaterJobMode>,
    temperature: Option<TemperatureData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WaterHeaterJobMode {
    current_job_mode: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TemperatureData {
    current_temperature: f64,
    target_temperature: f64,
    // unit: String, // non utilisé, supprimé pour éviter warning dead_code
}

// ---------------------------------------------------------------------------
// Public snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct LgSnapshot {
    pub mode: WaterHeaterMode,
    pub current_temp_c: Option<f64>,
    pub target_temp_c: Option<f64>,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct LgThinqClient {
    http: reqwest::Client,
    cfg: LgThinqConfig,
}

impl LgThinqClient {
    pub fn new(cfg: LgThinqConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            cfg,
        }
    }

    fn state_url(&self) -> String {
        format!("{}/devices/{}/state", self.cfg.base_url, self.cfg.device_id)
    }

    fn control_url(&self) -> String {
        format!("{}/devices/{}/control", self.cfg.base_url, self.cfg.device_id)
    }

    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
        let mut h = HeaderMap::new();
        h.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.cfg.bearer_token)).unwrap(),
        );
        if !self.cfg.api_key.is_empty() {
            h.insert("x-api-key",
                HeaderValue::from_str(&self.cfg.api_key).unwrap());
        }
        if !self.cfg.country.is_empty() {
            h.insert("x-country",
                HeaderValue::from_str(&self.cfg.country).unwrap());
        }
        if !self.cfg.client_id.is_empty() {
            h.insert("x-client-id",
                HeaderValue::from_str(&self.cfg.client_id).unwrap());
        }
        let msg_id = format!("{:x}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis());
        h.insert("x-message-id",
            HeaderValue::from_str(&msg_id).unwrap());
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        h
    }

    pub async fn get_state(&self) -> Result<LgSnapshot> {
        let resp = self.http
            .get(self.state_url())
            .headers(self.auth_headers())
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .context("LG ThinQ GET state")?
            .error_for_status()
            .context("LG ThinQ GET state HTTP error")?;

        let body: LgStateResponse = resp.json().await.context("LG ThinQ parse state")?;

        let mode_str = body.response
            .water_heater_job_mode
            .as_ref()
            .map(|m| m.current_job_mode.as_str())
            .unwrap_or_default()
            .to_string();

        let current_temp_c = body.response
            .temperature
            .as_ref()
            .map(|t| t.current_temperature);
        let target_temp_c = body.response
            .temperature
            .as_ref()
            .map(|t| t.target_temperature);

        debug!("LG ThinQ state: mode={mode_str} temp={current_temp_c:?} target={target_temp_c:?}");
        Ok(LgSnapshot {
            mode: WaterHeaterMode::from_lg_str(&mode_str),
            current_temp_c,
            target_temp_c,
        })
    }

    pub async fn set_mode(&self, mode: WaterHeaterMode) -> Result<()> {
        let payload = json!({
            "waterHeaterJobMode": {
                "currentJobMode": mode.to_lg_str()
            }
        });
        let resp = self.http
            .post(self.control_url())
            .headers(self.auth_headers())
            .json(&payload)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .context("LG ThinQ POST control (mode)")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("LG ThinQ control HTTP {status}: {body}"));
        }
        info!("LG ThinQ: mode set to {}", mode.to_lg_str());
        Ok(())
    }

    pub async fn set_target_temp(&self, temp_c: f64) -> Result<()> {
        let payload = json!({
            "temperature": {
                "targetTemperature": temp_c
            }
        });
        self.http
            .post(self.control_url())
            .headers(self.auth_headers())
            .json(&payload)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .context("LG ThinQ POST control (temp)")?
            .error_for_status()
            .context("LG ThinQ POST control temp HTTP error")?;
        info!("LG ThinQ: target temperature set to {temp_c}°C");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Polling task
// ---------------------------------------------------------------------------

pub async fn spawn_poller(
    cfg: LgThinqConfig,
    bus: AppBus,
    state: std::sync::Arc<tokio::sync::RwLock<crate::types::EnergyState>>,
) -> Option<LgThinqClient> {
    if !cfg.enabled {
        info!("LG ThinQ integration disabled");
        return None;
    }

    if cfg.device_id.is_empty() || cfg.bearer_token.is_empty() {
        warn!("LG ThinQ enabled but credentials missing (device_id / bearer_token)");
        return None;
    }

    info!("LG ThinQ poller started (device={}, interval={}s)",
        cfg.device_id, cfg.poll_interval_secs);

    let client = LgThinqClient::new(cfg.clone());

    let cfg2   = cfg.clone();
    let bus2   = bus.clone();
    let state2 = state.clone();
    tokio::spawn(async move {
        let poller = LgThinqClient::new(cfg2);
        let mut ticker = interval(Duration::from_secs(poller.cfg.poll_interval_secs));
        loop {
            ticker.tick().await;
            match poller.get_state().await {
                Ok(snap) => {
                    {
                        let mut s = state2.write().await;
                        s.water_heater_mode     = snap.mode;
                        s.water_heater_temp_c   = snap.current_temp_c;
                        s.water_heater_target_c = snap.target_temp_c;
                    }
                    bus2.emit_live(LiveEvent::new("water_heater", &snap));
                }
                Err(e) => error!("LG ThinQ poll error: {e}"),
            }
        }
    });

    Some(client)
}
