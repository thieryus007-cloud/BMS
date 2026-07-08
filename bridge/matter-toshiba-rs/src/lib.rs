//! **Bridge Matter (Pi5) pour les clim Toshiba — Rust pur, sans Node.js.**
//!
//! Expose les climatiseurs Toshiba comme **endpoints Thermostat Matter** (device-type
//! *Bridge/Aggregator*) à partir du dorsal MQTT `santuario/toshiba/<zone>/{state,command}`,
//! pour une passerelle smart-home multi-fabric (Homey/Apple/Google). Voir
//! `docs/toshiba-suzumi-rs-plan.md` **§18.12** (décision + comparaison Matterbridge/Node).
//!
//! ## Séparation I/O ↔ logique (comme `firmware/toshiba-suzumi-rs`)
//!
//! - [`mapping`] : conversions **pures** `état Toshiba (JSON MQTT) ↔ attributs cluster
//!   Thermostat Matter` + `écriture Matter → commande MQTT`. Testé sur host.
//! - [`config`] : `NodeConfig` (zones, broker, paramètres de commissioning Matter) +
//!   validation + dérivation des topics. Testé sur host.
//! - [`bridge`] : **orchestration pure** (cache par zone ; message d'état → attributs ;
//!   écriture Matter → commande). Transport-agnostique, testable sans rs-matter/MQTT.
//!
//! La couche **transport** (rs-matter + rumqttc) vit derrière la feature `matter`
//! (fichiers `matter.rs`/`mqtt.rs`/`main.rs` à ajouter — cf. README). Elle ne fait que
//! **relayer** vers [`bridge::Bridge`] : rien de logique nouveau.

pub mod bridge;
pub mod config;
pub mod mapping;
