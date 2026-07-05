//! Firmware Rust natif pour clim Toshiba (protocole SUZUMI, connecteur CN22).
//!
//! Ce crate est **détaché** du workspace Pi5. À ce stade, il ne contient que la
//! **couche protocole pure** ([`protocol`]) — sans I/O, sans ESP-IDF — entièrement
//! testable sur host. La partie firmware ESP32 (UART/WiFi/MQTT) sera ajoutée quand
//! le matériel arrivera.
//!
//! Référence de spécification : `docs/toshiba-suzumi-rs-plan.md` §6/§8, vérifiée
//! contre `pedobry/esphome_toshiba_suzumi` et recoupée avec `o0Zz/climate-uart`.

pub mod protocol;
