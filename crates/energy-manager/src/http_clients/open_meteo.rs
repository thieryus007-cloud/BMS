use anyhow::Result;
use serde::Deserialize;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info};

use crate::bus::AppBus;
use crate::config::OpenMeteoConfig;
use crate::types::LiveEvent;

// ---------------------------------------------------------------------------
// API response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct OpenMeteoResponse {
    current: Option<CurrentWeather>,
}

#[derive(Debug, Deserialize)]
struct CurrentWeather {
    temperature_2m: Option<f64>,
    relative_humidity_2m: Option<f64>,
    surface_pressure: Option<f64>,
    wind_speed_10m: Option<f64>,
}

// ---------------------------------------------------------------------------
// Published weather snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct WeatherSnapshot {
    pub temperature_c: Option<f64>,
    pub humidity_pct: Option<f64>,
    pub pressure_hpa: Option<f64>,
    pub wind_speed_ms: Option<f64>,
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

pub async fn spawn(
    cfg: OpenMeteoConfig,
    bus: AppBus,
    state: std::sync::Arc<tokio::sync::RwLock<crate::types::EnergyState>>,
) {
    if !cfg.enabled {
        info!("Open-Meteo polling disabled");
        return;
    }
    tokio::spawn(run(cfg, bus, state));
}

async fn run(
    cfg: OpenMeteoConfig,
    bus: AppBus,
    state: std::sync::Arc<tokio::sync::RwLock<crate::types::EnergyState>>,
) {
    info!("Open-Meteo polling started ({:.4}°N, {:.4}°E, interval={}s)",
        cfg.latitude, cfg.longitude, cfg.poll_interval_secs);

    let client = reqwest::Client::new();
    let mut ticker = interval(Duration::from_secs(cfg.poll_interval_secs));

    // Poll immediately then on interval
    loop {
        ticker.tick().await;
        match fetch(&client, cfg.latitude, cfg.longitude).await {
            Ok(snap) => {
                debug!("Open-Meteo: {:?}", snap);
                {
                    let mut s = state.write().await;
                    s.temperature_c = snap.temperature_c;
                    s.humidity_pct  = snap.humidity_pct;
                    s.pressure_hpa  = snap.pressure_hpa;
                    s.wind_speed_ms = snap.wind_speed_ms;
                }
                bus.emit_live(LiveEvent::new("weather", &snap));
            }
            Err(e) => error!("Open-Meteo fetch error: {e}"),
        }
    }
}

async fn fetch(client: &reqwest::Client, lat: f64, lon: f64) -> Result<WeatherSnapshot> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast\
        ?latitude={lat}&longitude={lon}\
        &current=temperature_2m,relative_humidity_2m,surface_pressure,wind_speed_10m\
        &wind_speed_unit=ms"
    );

    let resp: OpenMeteoResponse = client
        .get(&url)
        .timeout(Duration::from_secs(15))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let current = resp.current.unwrap_or(CurrentWeather {
        temperature_2m: None,
        relative_humidity_2m: None,
        surface_pressure: None,
        wind_speed_10m: None,
    });

    Ok(WeatherSnapshot {
        temperature_c: current.temperature_2m,
        humidity_pct:  current.relative_humidity_2m,
        pressure_hpa:  current.surface_pressure,
        wind_speed_ms: current.wind_speed_10m,
    })
}
