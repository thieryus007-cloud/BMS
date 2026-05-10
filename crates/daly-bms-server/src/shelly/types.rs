//! Types de données pour les compteurs d'énergie Shelly Pro 2PM (2 canaux).

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

/// Données d'un canal Shelly Pro 2PM.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShellyChannelData {
    /// Relais fermé (ON) ou ouvert (OFF).
    pub output:             bool,
    /// Puissance active instantanée (W).
    pub power_w:            f32,
    /// Tension secteur (V).
    pub voltage_v:          f32,
    /// Courant (A).
    pub current_a:          f32,
    /// Facteur de puissance.
    pub power_factor:       f32,
    /// Énergie totale consommée depuis la remise à zéro (Wh).
    pub energy_wh:          f64,
    /// Énergie retournée au réseau (Wh).
    pub returned_wh:        f64,
}

/// Snapshot complet d'un Shelly Pro 2PM (2 canaux).
#[derive(Debug, Clone, Serialize)]
pub struct ShellyEmSnapshot {
    pub id:           u8,
    pub name:         String,
    pub shelly_id:    String,
    pub timestamp:    DateTime<Local>,
    pub channel_0:    ShellyChannelData,
    pub channel_1:    ShellyChannelData,
    pub rssi:         Option<i32>,
    pub total_power_w: f32,
}
