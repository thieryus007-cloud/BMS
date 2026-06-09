pub mod lg_thinq;
pub mod open_meteo;

use std::sync::OnceLock;
use std::time::Duration;

/// Client HTTP partagé du crate, avec timeouts globaux (audit 2026-06 §6).
///
/// Tous les appels sortants passent par un client borné : `connect_timeout`
/// 5 s + `timeout` total 15 s. Sans cela, un `reqwest::Client::new()` n'a
/// AUCUN timeout : une connexion TCP gelée suspend la tâche appelante
/// indéfiniment (météo/chauffe-eau figés sans erreur ni redémarrage).
///
/// Les timeouts par requête plus courts (`.timeout(5 s)` sur certains POST)
/// restent prioritaires : le timeout client ne s'applique qu'en plafond.
pub(crate) fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|e| {
                // Improbable (échec init TLS) — même classe d'échec que
                // l'ancien Client::new(), mais tracée au lieu de paniquer ici.
                tracing::warn!("client HTTP partagé : builder en échec ({e}) — fallback sans timeouts");
                reqwest::Client::new()
            })
    })
}
