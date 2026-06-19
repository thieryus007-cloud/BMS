use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};

use std::sync::Arc;
use tokio::sync::RwLock;

use crate::bus::AppBus;
use crate::config::{LgThinqConfig, WaterHeaterConfig};
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

#[derive(Clone)]
pub struct LgThinqClient {
    http: reqwest::Client,
    cfg: LgThinqConfig,
    /// Headers d'authentification pré-validés au démarrage (audit 2026-06 §4).
    /// Un caractère invalide dans la config provoquait un `unwrap()` → panique
    /// à chaque poll (crash-loop avec panic=abort). Désormais : erreur claire
    /// à la construction, nommant le champ fautif.
    base_headers: reqwest::header::HeaderMap,
}

/// Construit les headers fixes en validant chaque champ de config.
fn build_base_headers(cfg: &LgThinqConfig) -> Result<reqwest::header::HeaderMap> {
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
    let mut h = HeaderMap::new();
    h.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", cfg.bearer_token))
            .context("lg_thinq.bearer_token : caractère invalide pour un header HTTP")?,
    );
    if !cfg.api_key.is_empty() {
        h.insert("x-api-key",
            HeaderValue::from_str(&cfg.api_key)
                .context("lg_thinq.api_key : caractère invalide pour un header HTTP")?);
    }
    if !cfg.country.is_empty() {
        h.insert("x-country",
            HeaderValue::from_str(&cfg.country)
                .context("lg_thinq.country : caractère invalide pour un header HTTP")?);
    }
    if !cfg.client_id.is_empty() {
        h.insert("x-client-id",
            HeaderValue::from_str(&cfg.client_id)
                .context("lg_thinq.client_id : caractère invalide pour un header HTTP")?);
    }
    h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(h)
}

impl LgThinqClient {
    pub fn new(cfg: LgThinqConfig) -> Result<Self> {
        let base_headers = build_base_headers(&cfg)?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .context("LG ThinQ : construction du client HTTP")?;
        Ok(Self { http, cfg, base_headers })
    }

    fn state_url(&self) -> String {
        format!("{}/devices/{}/state", self.cfg.base_url, self.cfg.device_id)
    }

    fn control_url(&self) -> String {
        format!("{}/devices/{}/control", self.cfg.base_url, self.cfg.device_id)
    }

    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::HeaderValue;
        let mut h = self.base_headers.clone();
        // x-message-id : hex de millis epoch — toujours ASCII valide, mais on
        // évite tout unwrap : header simplement omis dans le cas impossible.
        let msg_id = format!("{:x}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis());
        if let Ok(v) = HeaderValue::from_str(&msg_id) {
            h.insert("x-message-id", v);
        }
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
    state: Arc<RwLock<crate::types::EnergyState>>,
    wh_cfg: Arc<RwLock<WaterHeaterConfig>>,
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

    // Validation des headers au démarrage (audit 2026-06 §4) : config invalide
    // → intégration désactivée avec erreur explicite, au lieu d'un crash-loop
    // au premier poll.
    let client = match LgThinqClient::new(cfg.clone()) {
        Ok(c) => c,
        Err(e) => {
            error!("LG ThinQ désactivé — config invalide : {e:#}");
            return None;
        }
    };

    let poller  = client.clone();
    let cfg2    = cfg.clone();
    let bus2    = bus.clone();
    let state2  = state.clone();
    let wh_cfg2 = wh_cfg.clone();
    crate::supervise::spawn_critical(async move {
        let vm_url = cfg2.vm_url.clone();
        let mut ticker = interval(Duration::from_secs(poller.cfg.poll_interval_secs));
        loop {
            ticker.tick().await;
            let now = chrono::Utc::now();
            match poller.get_state().await {
                Ok(snap) => {
                    // Seuil « cuve à température cible » lu hors verrou `state`
                    // (config rechargeable à chaud, partagée avec le control_task).
                    let temp_max_c = wh_cfg2.read().await.temp_max_c;
                    {
                        let mut s = state2.write().await;
                        s.water_heater_mode      = snap.mode;
                        s.water_heater_temp_c    = snap.current_temp_c;
                        s.water_heater_target_c  = snap.target_temp_c;
                        s.water_heater_last_read = Some(now);
                        // Ancre le chrono dès que le poller voit la cuve ≥ seuil,
                        // sans attendre le tick de 5 min du control_task.
                        crate::logic::water_heater::anchor_temp_max(&mut s, now, temp_max_c);
                    }
                    bus2.emit_live(LiveEvent::new("water_heater", &snap));

                    // Write 3 LG ThinQ metrics to daly-bms-server (metrics-store redb)
                    let ts_ms = now.timestamp_millis();
                    let mut lines = Vec::new();
                    lines.push(format!("wh_mode{{}} {} {}", snap.mode.to_venus_state(), ts_ms));
                    if let Some(t) = snap.current_temp_c {
                        lines.push(format!("wh_current_temp_c{{}} {} {}", t, ts_ms));
                    }
                    if let Some(t) = snap.target_temp_c {
                        lines.push(format!("wh_target_temp_c{{}} {} {}", t, ts_ms));
                    }
                    let body = lines.join("\n");
                    let url  = format!("{}/api/v1/import/prometheus", vm_url);
                    if let Err(e) = super::shared_client().post(&url).body(body).send().await {
                        warn!("LG ThinQ VM write error: {e}");
                    }

                    // Publication MQTT vers `santuario/em/water_heater` (consommée par daly-bms-server).
                    let payload = serde_json::json!({
                        "mode":           snap.mode.to_venus_state(),
                        "current_temp_c": snap.current_temp_c,
                        "target_temp_c":  snap.target_temp_c,
                    });
                    bus2.publish(crate::types::MqttOutgoing::transient(
                        "santuario/em/water_heater", payload,
                    )).await;
                }
                Err(e) => error!("LG ThinQ poll error: {e}"),
            }
        }
    });

    Some(client)
}
