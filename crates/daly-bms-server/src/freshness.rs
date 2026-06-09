//! Registre de fraîcheur des sources de données (audit 2026-06 §18).
//!
//! Toutes les pannes « silencieuses » restantes ont la même signature : une
//! source (RS485, MQTT Venus, Tasmota…) cesse de produire sans erreur visible,
//! et le dashboard affiche des valeurs figées. Chaque funnel `on_*` de
//! l'AppState tamponne ici son passage ; une tâche périodique exporte l'âge
//! de la dernière donnée par source dans metrics-store :
//!
//! ```promql
//! source_last_update_age_seconds{source="bms_0x01"}
//! ```
//!
//! Alerte recommandée (Grafana) : `age > 5 × intervalle de polling`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Default)]
pub struct FreshnessRegistry {
    /// Horodatage monotone (`Instant`, insensible aux sauts NTP) du dernier
    /// snapshot reçu, par source. Sections critiques courtes, jamais
    /// d'`await` sous le verrou.
    map: Mutex<HashMap<String, Instant>>,
}

impl FreshnessRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enregistre un passage de la source `source` (appelé par les `on_*`).
    pub fn touch(&self, source: &str) {
        if let Ok(mut map) = self.map.lock() {
            map.insert(source.to_string(), Instant::now());
        }
    }

    /// Âge (en secondes) de la dernière donnée de chaque source connue.
    pub fn ages_seconds(&self) -> Vec<(String, f64)> {
        match self.map.lock() {
            Ok(map) => map
                .iter()
                .map(|(k, t)| (k.clone(), t.elapsed().as_secs_f64()))
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_then_age_is_small_and_grows() {
        let r = FreshnessRegistry::new();
        r.touch("bms_0x01");
        let ages = r.ages_seconds();
        assert_eq!(ages.len(), 1);
        assert!(ages[0].1 < 1.0, "âge initial anormal : {}", ages[0].1);

        std::thread::sleep(std::time::Duration::from_millis(20));
        let ages2 = r.ages_seconds();
        assert!(ages2[0].1 >= ages[0].1, "l'âge doit croître");

        r.touch("bms_0x01");
        let ages3 = r.ages_seconds();
        assert!(ages3[0].1 < ages2[0].1, "touch doit remettre l'âge à ~0");
    }

    #[test]
    fn multiple_sources_tracked_independently() {
        let r = FreshnessRegistry::new();
        r.touch("et112_0x07");
        r.touch("venus_mppt");
        assert_eq!(r.ages_seconds().len(), 2);
    }
}
