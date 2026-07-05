# Plan de projet : Transposition du pilote Toshiba SUZUMI en Rust pour ESP32

> **Référence** : [pedobry/esphome_toshiba_suzumi](https://github.com/pedobry/esphome_toshiba_suzumi)  
> **Objectif** : Remplacer le composant ESPHome (C++) par un firmware Rust autonome sur ESP32, sans dépendance à Home Assistant.

---

## 1. Objectifs du projet

- **Remplacer** le composant ESPHome C++ par un firmware Rust autonome sur ESP32.
- **Communiquer** avec le climatiseur Toshiba via le connecteur UART CN22 (protocole propriétaire SUZUMI).
- **Publier** l'état du climatiseur sur un broker MQTT (JSON structuré).
- **Recevoir** des commandes MQTT pour piloter le climatiseur (mode, température, puissance, etc.).
- **Fonctionner** sans dépendance à Home Assistant ni à tout autre système central — le broker MQTT est le seul point de contact.
- **Exposer** les capteurs de diagnostic optionnels (ODU/IDU) si supportés par l'unité.

---

## 2. Architecture fonctionnelle

```
┌─────────────┐     UART (CN22)     ┌─────────────┐
│ Climatiseur │ ◄──────────────────►│   ESP32     │
│   Toshiba   │   9600 baud, EVEN     │  (Rust)     │
│   (5V TTL)  │   5V ↔ 3.3V level   │             │
└─────────────┘                     └──────┬──────┘
                                           │ WiFi / Ethernet
                                     ┌─────▼─────┐
                                     │ Broker    │
                                     │  MQTT     │
                                     └───────────┘
```

L'ESP32 agit comme **passerelle protocolaire** :
- Il traduit les commandes MQTT en trames UART Toshiba (protocole SUZUMI).
- Il traduit les trames UART reçues en messages MQTT (état, température, diagnostics).
- Il gère la séquence de handshake initiale et la reconnexion automatique en cas de perte de communication.

---

## 3. Prérequis matériels

### 3.1 Composants nécessaires

| Élément | Référence / Conseil |
|---------|---------------------|
| **ESP32** | DevKitC, WROOM-32, ou tout module avec 2 UART disponibles (UART0 pour debug, UART1/2 pour CN22) |
| **Convertisseur de niveau** | Bidirectionnel 5V ↔ 3.3V (ex: TXB0108, ou simple diviseur résistif + diode Zener en réception) |
| **Alimentation** | Le climatiseur fournit du 5V sur CN22 broche 3. **Vérifier** que l'ESP32 supporte 5V sur VIN, sinon utiliser un régulateur 3.3V (AMS1117-3.3) |
| **Câblage** | 4 fils minimum (TX, RX, GND, 5V) + résistance de pull-up sur RX si niveau 5V |

### 3.2 Connexions CN22 → ESP32

| CN22 (broche) | Couleur | Fonction | ESP32 GPIO | Remarque |
|:-------------:|:-------:|:---------|:----------:|:---------|
| 1 | Bleu | TX climatiseur → RX ESP32 | GPIO 33 | **Via level-shifter 5V→3.3V** |
| 2 | Rose | GND | GND | Commun |
| 3 | Noir | 5V (alim) | VIN (ou 5V) | Vérifier la capacité de sortie du CN22 (typ. 100-200mA) |
| 4 | Blanc | RX climatiseur ← TX ESP32 | GPIO 32 | **Via level-shifter 3.3V→5V** |

> **⚠️ Attention critique** : Les lignes TX/RX du CN22 sont en **5V TTL**. L'ESP32 est **3.3V**. Un level-shifter bidirectionnel est **obligatoire** sous peine de destruction du GPIO. La parité UART est **EVEN** (et non NONE).

---

## 4. Structure du projet Rust

```
toshiba-suzumi-rs/
├── .cargo/
│   └── config.toml              # Cible xtensa-esp32-espidf
├── Cargo.toml
├── build.rs                     # Configuration ESP-IDF (sdkconfig)
├── sdkconfig.defaults           # Paramètres ESP-IDF (heap, WiFi, UART buffers)
├── partitions.csv               # Table de partition personnalisée (OTA + NVS)
├── src/
│   ├── main.rs                  # Point d'entrée, init, task principale
│   ├── protocol.rs              # Définition des trames, checksum, parsing SUZUMI
│   ├── uart.rs                  # Wrapper UART ESP-IDF (async ou blocking)
│   ├── mqtt.rs                  # Client MQTT (WiFi + connexion broker)
│   ├── wifi.rs                  # Gestion WiFi station (connexion, reconnexion)
│   ├── command_queue.rs         # File d'attente des commandes (MPMC channel)
│   ├── state_machine.rs         # Machine à états (Handshake → Online → Error → Retry)
│   ├── sensors.rs               # Parsing des capteurs ODU/IDU optionnels
│   └── config.rs                # Configuration NVS (SSID, MQTT, topics)
└── tests/
    ├── test_protocol.rs         # Tests unitaires checksum / parsing (exécutable sur host)
    └── test_frame_builder.rs    # Tests construction de trames
```

---

## 5. Dépendances Rust (Cargo.toml)

```toml
[package]
name = "toshiba-suzumi-rs"
version = "0.1.0"
edition = "2021"
resolver = "2"

[dependencies]
# HAL & système ESP-IDF
esp-idf-svc = { version = "0.49", features = ["alloc", "native"] }
esp-idf-hal = "0.44"
esp-idf-sys = "0.35"

# Async runtime (optionnel mais recommandé pour MQTT + UART concurrents)
embassy-sync = "0.6"
embassy-futures = "0.1"

# MQTT client (rust-mqtt ou esp-mqtt)
# Option A : esp-idf-svc intègre déjà un client MQTT via esp-mqtt-dispatcher
# Option B : crate mqtt externe si besoin de QoS 2 avancé

# Serialization JSON pour payloads MQTT
serde = { version = "1.0", default-features = false, features = ["derive"] }
serde-json-core = "0.6"          # Version no-std compatible heap limité

# Logging & diagnostic
log = "0.4"
esp-idf-svc::log = "0.1"         # EspLogger intégré

# Utilitaires no-std
heapless = "0.8"                 # Vec/String statiques (pas d'allocateur requis)
nb = "1.1"                       # Non-blocking traits

[dev-dependencies]
# Tests host-side (pas de cross-compilation)
serde_json = "1.0"

[[bin]]
name = "toshiba-suzumi-rs"
harness = false                  # Pas de libtest embarqué (sauvegarde RAM/Flash)

[profile.release]
opt-level = 3
lto = true
```

---

## 6. Spécification du protocole SUZUMI

### 6.1 Paramètres UART

| Paramètre | Valeur | Commentaire |
|-----------|--------|-------------|
| Baudrate | **9600** | Confirmé sur toutes les unités SUZUMI/SHORAI/SEIYA |
| Data bits | 8 | |
| Parité | **EVEN** | ⚠️ Erreur fréquente : ce n'est PAS `None` |
| Stop bits | 1 | |
| Flow control | Aucun | |

### 6.2 Format général d'une trame

Une trame SUZUMI suit le schéma suivant (à adapter selon reverse-engineering du C++ source) :

```
[0]     Octet de début / synchro (ex: 0xF0 ou 0xF1)
[1]     Longueur du payload (n) ou type de message
[2..n]  Payload (commande ou état)
[n+1]   Checksum (8 bits)
```

### 6.3 Checksum

Algorithme : `(256 - (somme des octets de l'index 1 à len-2) % 256) % 256`

En Rust (wrapping arithmétique) :

```rust
pub fn compute_checksum(data: &[u8]) -> u8 {
    let sum: u8 = data[1..data.len()-1]
        .iter()
        .fold(0u8, |acc, &b| acc.wrapping_add(b));
    (256u16 - sum as u16) as u8
}
```

### 6.4 Séquence de handshake (boot / reconnexion)

Le climatiseur **ne répond pas** aux commandes tant que le handshake n'est pas établi. La séquence est critique.

```
Étape 1 : Envoi des 6 trames d'initialisation (INIT_FRAMES)
    ├── Délai inter-trame : 50 ms
    └── Contenu : constantes définies dans le firmware d'origine

Étape 2 : Attente obligatoire
    └── Délai : 2000 ms (2 secondes exactes)

Étape 3 : Envoi des 2 trames AFTER_HANDSHAKE
    ├── Délai inter-trame : 50 ms
    └── À ce stade, le climatiseur commence à émettre des trames d'état

Étape 4 : Réception des trames d'état périodiques
    └── Le climatiseur pousse spontanément son état toutes les ~1-2 secondes
```

> **Note** : Si le climatiseur est éteint puis rallumé, ou si le câble est débranché, le handshake doit être **re-joué** intégralement.

### 6.5 Types de commandes (CmdType)

Basé sur l'analyse du composant ESPHome :

```rust
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CmdType {
    Power       = 0x30,  // 48 decimal — ON/OFF
    Mode        = 0x31,  // 49 decimal — Cool, Heat, Dry, Fan, Auto
    TargetTemp  = 0x32,  // 50 decimal — 17-30°C (5-30°C en mode "8 degrees")
    FanSpeed    = 0x33,  // Ventilation
    PowerLevel  = 0x34,  // Niveau de puissance (Hi Power, ECO, etc.)
    Preset      = 0x35,  // Standard, Sleep, Silent, Fireplace...
    SwingVertical   = 0x36,
    SwingHorizontal = 0x37,  // Optionnel selon modèle
    DisableWifiLed  = 0x38,  // Éteindre la LED WiFi de l'unité interne
    // ... autres commandes identifiées par scan
}
```

### 6.6 Modes de fonctionnement

| Valeur | Mode | Description |
|--------|------|-------------|
| 0x00 | OFF | Unité arrêtée (Power=OFF) |
| 0x01 | COOL | Refroidissement |
| 0x02 | HEAT | Chauffage |
| 0x03 | DRY | Déshumidification |
| 0x04 | FAN | Ventilation seule |
| 0x05 | AUTO | Automatique |

### 6.7 Préréglages (Presets)

- `Standard`
- `Hi POWER`
- `ECO`
- `Fireplace 1`
- `Fireplace 2`
- `8 degrees` → Plage température forcée à 5-30°C
- `Silent#1`
- `Silent#2`
- `Sleep`
- `Floor`
- `Comfort`

### 6.8 Capteurs de diagnostic ODU / IDU (optionnels)

Certaines unités envoient des trames étendues contenant :

| Capteur | Unité | Description | Valeur invalide |
|---------|-------|-------------|-----------------|
| `outdoor_temp` | °C | Température extérieure | 127 (à filtrer) |
| `cdu_load` | % | Charge compresseur (fréquence) | — |
| `cdu_iac` | A | Courant compresseur / EEV | — |
| `cdu_td_temp` | °C | Température tube de refoulement | — |
| `cdu_ts_temp` | °C | Température tube d'aspiration | — |
| `cdu_te_temp` | °C | Température évaporateur ODU | — |
| `fcu_tc_temp` | °C | Température échangeur IDU | — |
| `fcu_tcj_temp` | °C | Température jonction échangeur IDU | — |
| `fcu_fan_rpm` | RPM | Vitesse ventilateur IDU | — |

> Ces valeurs arrivent **spontanément** depuis l'unité (pas de polling requis). Il suffit de parser les trames étendues.

---

## 7. Étapes détaillées de développement

### Phase 1 : Environnement et squelettage (1h)

1. Installer `espup` et `cargo-generate`.
2. Générer le projet : `cargo generate esp-rs/esp-idf-template`.
3. Configurer `.cargo/config.toml` pour la cible `xtensa-esp32-espidf`.
4. Ajouter les dépendances dans `Cargo.toml` (voir §5).
5. Configurer `sdkconfig.defaults` :
   ```
   CONFIG_ESP_MAIN_TASK_STACK_SIZE=8192
   CONFIG_ESP_SYSTEM_EVENT_TASK_STACK_SIZE=2048
   CONFIG_FREERTOS_UNICORE=n
   CONFIG_UART_ISR_IN_IRAM=y
   CONFIG_MBEDTLS_DYNAMIC_BUFFER=1
   ```
6. Vérifier la compilation : `cargo build --release`.

### Phase 2 : Transposition du protocole (3h)

1. **Extraire les constantes** du C++ source : tableaux `HANDSHAKE` (6 trames), `AFTER_HANDSHAKE` (2 trames), et les magic bytes.
2. **Implémenter `compute_checksum`** (wrapping_add, voir §6.3).
3. **Implémenter `validate_message`** : vérifier longueur, checksum, et structure minimale.
4. **Définir les structures** :
   ```rust
   pub struct ToshibaStatus {
       pub power: bool,
       pub mode: OperationMode,
       pub target_temp: f32,      // ou u8 si demi-degrés non supportés
       pub current_temp: f32,     // Température ambiante mesurée
       pub fan_speed: FanSpeed,
       pub preset: Option<Preset>,
       pub swing_vertical: Swing,
       pub swing_horizontal: Option<Swing>,
       pub outdoor_temp: Option<i8>, // 127 = invalide
       // ... diagnostics ODU/IDU
   }
   ```
5. **Implémenter `build_command`** : construire une trame de commande à partir de `CmdType` + valeur.
6. **Implémenter `parse_response`** : mapper les octets reçus vers `ToshibaStatus`.
7. **Écrire les tests unitaires host-side** (`tests/test_protocol.rs`) pour valider checksum et parsing.

### Phase 3 : Interface UART (1h30)

1. Initialiser UART2 (ou UART1) avec les bons paramètres :
   ```rust
   let config = uart::config::Config::new()
       .baudrate(Hertz(9600))
       .data_bits(uart::config::DataBits::DataBits8)
       .parity(uart::config::Parity::ParityEven)  // ⚠️ EVEN
       .stop_bits(uart::config::StopBits::STOP1);
   ```
2. Configurer les pins GPIO32 (TX) et GPIO33 (RX).
3. Implémenter `send_bytes(data: &[u8])` avec flush après envoi.
4. Implémenter `read_bytes(timeout_ms: u32) -> heapless::Vec<u8, 64>` (buffer circulaire).
5. **Gérer le framing** : détection du start-byte, lecture de la longueur, attente du checksum.

### Phase 4 : WiFi et MQTT (2h)

1. **WiFi** : Utiliser `esp_idf_svc::wifi::EspWifi` en mode Station.
   - Lecture des credentials depuis NVS (flash) ou variables d'environnement au build.
   - Reconnexion automatique avec backoff exponentiel.
2. **MQTT** : Utiliser le client intégré à `esp_idf_svc::mqtt::client::EspMqttClient`.
   - Connexion au broker avec LWT (Last Will Testament) pour signaler la déconnexion.
   - **Topic d'état** (publish) : `toshiba/<device_id>/state` — JSON structuré.
   - **Topic de commande** (subscribe) : `toshiba/<device_id>/command` — JSON ou payload simple.
   - **Topic de disponibilité** : `toshiba/<device_id>/availability` — `online` / `offline`.
3. **Format JSON d'état** (exemple) :
   ```json
   {
     "power": true,
     "mode": "heat",
     "target_temp": 22.0,
     "current_temp": 21.5,
     "fan_speed": "auto",
     "preset": "standard",
     "swing_vertical": "auto",
     "outdoor_temp": 8,
     "cdu_load": 45,
     "timestamp": 1720000000
   }
   ```
4. **Format JSON de commande** (exemple) :
   ```json
   {
     "cmd": "set",
     "power": true,
     "mode": "cool",
     "target_temp": 24,
     "preset": "eco"
   }
   ```

### Phase 5 : Machine à états et logique applicative (3h)

```rust
pub enum State {
    Boot,           // Initialisation hardware
    HandshakeInit,  // Envoi des 6 trames INIT
    HandshakeWait,  // Attente 2s
    HandshakeAfter, // Envoi des 2 trames AFTER
    Online,         // Communication normale
    Error,          // Timeout ou checksum invalide
    RetryDelay,     // Attente avant re-handshake (10s)
}
```

1. **Gestion de la file d'attente** : `heapless::spsc::Queue` ou `embassy_sync::channel` pour stocker les commandes MQTT à envoyer.
2. **Boucle principale** (task FreeRTOS) :
   - Si `Online` et file non vide → envoyer commande UART.
   - Lire les données UART disponibles (timeout 200 ms).
   - Valider et parser les trames reçues.
   - Si trame valide → mettre à jour `ToshibaStatus` et publier MQTT si changement détecté (delta).
   - Si timeout persistant (3x) → transition vers `Error` puis `RetryDelay`.
3. **Watchdog logiciel** : Si aucune trame reçue pendant 60s, forcer un re-handshake.
4. **Déduplication** : Ne publier MQTT que si l'état a changé (évite le spam).

### Phase 6 : Configuration et NVS (1h)

1. Stocker en NVS (flash) via `esp_idf_svc::nvs::EspNvs` :
   - `wifi_ssid`, `wifi_pass`
   - `mqtt_broker`, `mqtt_port`, `mqtt_user`, `mqtt_pass`
   - `device_id` (pour les topics MQTT)
   - `uart_tx_pin`, `uart_rx_pin` (optionnel)
2. **Premier boot** : Si NVS vide, démarrer en mode AP (WiFi Access Point) avec un serveur HTTP minimal pour configurer via un formulaire web.
   - Alternative : utiliser le port série (UART0) pour injecter la config en CLI.

### Phase 7 : Tests et validation (4h–8h)

1. **Tests unitaires (host)** :
   - `test_checksum` : valider sur des trames capturées réelles.
   - `test_parse_status` : vérifier le décodage d'une trame d'état brute.
   - `test_build_command` : vérifier que `build_command(CmdType::TargetTemp, 22)` produit la bonne séquence.
2. **Tests sur cible nue (ESP32)** :
   - Vérifier l'envoi du handshake avec un analyseur logique (Saleae, PulseView).
   - Simuler le climatiseur avec un second ESP32 ou un PC (USB-UART) envoyant des trames de réponse pré-enregistrées.
3. **Tests réels avec le climatiseur** :
   - Démarrer le handshake, observer les réponses dans les logs (UART0).
   - Vérifier que la température ambiante et l'état ON/OFF sont corrects.
   - Envoyer des commandes MQTT et vérifier la réaction de l'unité (marche/arrêt, changement de mode, réglage température).
   - Tester les presets avancés (`8 degrees`, `Fireplace`, etc.) si l'unité les supporte.
4. **Tests de robustesse** :
   - Débrancher/rebrancher le câble CN22 → vérifier la reconnexion auto.
   - Couper le WiFi → vérifier la reconnexion MQTT et le LWT.
   - Envoyer une commande invalide → vérifier que le firmware ne panique pas.

---

## 8. Détails techniques critiques

### 8.1 Checksum

```rust
/// Calcule le checksum SUZUMI sur une trame complète (incluant le checksum à l'index final)
/// data[0] = start byte, data[1..n-1] = payload + checksum, data.len() = n+1
pub fn compute_checksum(data: &[u8]) -> u8 {
    let sum = data[1..data.len().saturating_sub(1)]
        .iter()
        .fold(0u8, |acc, &b| acc.wrapping_add(b));
    (256u16 - sum as u16) as u8
}

pub fn validate_frame(data: &[u8]) -> Result<(), ProtocolError> {
    if data.len() < 3 { return Err(ProtocolError::TooShort); }
    let expected = compute_checksum(data);
    if data[data.len() - 1] != expected {
        return Err(ProtocolError::ChecksumMismatch { expected, got: data[data.len()-1] });
    }
    Ok(())
}
```

### 8.2 Temporisation

| Étape | Délai | Tolérance |
|-------|-------|-----------|
| Inter-trame handshake | 50 ms | ±10 ms |
| Attente post-INIT | 2000 ms | **Stricte** (pas de réponse avant) |
| Timeout réception UART | 200 ms | Si dépassé, considérer la trame comme incomplète |
| Re-handshake après échec | 10 s | Backoff exponentiel max 60 s |
| Période heartbeat MQTT | 60 s | Publish `availability: online` + état complet |

### 8.3 Gestion des erreurs et reprise

| Erreur | Comportement |
|--------|--------------|
| Checksum invalide | Ignorer la trame, incrémenter un compteur. Au 3e échec consécutif → re-handshake. |
| Timeout UART (pas de réponse) | Attendre 10s, puis rejouer le handshake complet. |
| WiFi déconnecté | Tentative de reconnexion toutes les 5s. MQTT reste en pause. |
| MQTT déconnecté | Tentative de reconnexion toutes les 10s. Bufferiser les commandes reçues (max 10). |
| Mémoire insuffisante | Log d'erreur, redémarrage watchdog (panic handler personnalisé). |
| Commande invalide reçue (MQTT) | Répondre sur `.../command/response` avec JSON d'erreur, ne pas crasher. |

---

## 9. Déploiement et maintenance

### 9.1 Flashage initial

```bash
# Avec espflash (recommandé)
espflash flash --monitor target/xtensa-esp32-espidf/release/toshiba-suzumi-rs

# Ou cargo intégré
cargo run --release
```

### 9.2 Mise à jour OTA (optionnel)

- Prévoir une partition OTA dans `partitions.csv`.
- Utiliser `esp-idf-svc::ota::EspOta` pour télécharger et flasher un nouveau firmware depuis une URL HTTPS.
- **Sécurité** : vérifier la signature ECDSA du binaire avant flash.

### 9.3 Configuration runtime

- **NVS** : stockage persistant des credentials WiFi/MQTT.
- **Mode AP de secours** : si WiFi inconnu au boot, créer un AP `Toshiba-Setup-<chipid>` avec un captive portal HTTP.
- **UART0 CLI** : commandes `wifi-set`, `mqtt-set`, `reboot`, `status` accessibles via le port série de debug.

---

## 10. Ressources et documentation

| Ressource | Lien |
|-----------|------|
| The Rust on ESP Book | https://docs.esp-rs.org/book/ |
| esp-idf-hal documentation | https://docs.esp-rs.org/esp-idf-hal/ |
| esp-idf-svc examples | https://github.com/esp-rs/esp-idf-svc/tree/master/examples |
| Code source C++ original | https://github.com/pedobry/esphome_toshiba_suzumi |
| ESP-IDF UART API | https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/peripherals/uart.html |
| Projet connexe (Shorai) | https://github.com/toremick/shorai-esp32 |
| Projet connexe (TConnect) | https://github.com/Vpowgh/TConnect |
| Discord communautaire | https://discord.gg/wYYFawvqfr |

---

## 11. Calendrier prévisionnel

| Phase | Durée (heures) | Cumul (heures) |
|-------|:--------------:|:--------------:|
| Environnement + squelettage | 1 | 1 |
| Protocole (transposition + tests unitaires) | 3 | 4 |
| UART (init, read, write, framing) | 1.5 | 5.5 |
| WiFi + MQTT (connexion, topics, JSON) | 2 | 7.5 |
| Logique applicative (state machine, queue, watchdog) | 3 | 10.5 |
| Configuration NVS + mode AP secours | 1 | 11.5 |
| Tests sur cible + débogage réel | 4–8 | 15.5–19.5 |
| **Total** | | **~2–3 jours intensifs** ou **1 semaine à rythme normal** |

---

## 12. Conseils supplémentaires et pièges à éviter

1. **Démarrage progressif** : Commencez par un MVP qui envoie le handshake et affiche les réponses brutes en hexadécimal sur la console UART0 (sans MQTT). Validez le handshake avant d'ajouter la pile réseau.
2. **Logging** : Utilisez `esp_idf_svc::sys::link_patches()` et `esp_idf_svc::log::EspLogger` pour avoir les traces sur UART0. Activez `CONFIG_LOG_DEFAULT_LEVEL_DEBUG` pendant le développement.
3. **Parité UART** : C'est le piège le plus courant. Le C++ source utilise `parity: EVEN`. Si vous configurez `None`, le climatiseur ignorera silencieusement toutes les trames.
4. **Level-shifter** : Ne pas négliger l'étage de conversion de niveau. Un simple diviseur résistif côté RX peut suffire, mais un TXB0108 est plus sûr et bidirectionnel.
5. **Version de secours** : Avant le premier flash sur le hardware définitif, testez avec un ESP32 de développement et assurez-vous de pouvoir entrer en mode boot (GPIO0 → GND) en cas de brick.
6. **Scan de capteurs inconnus** : Le C++ original propose une fonction `scan()` pour découvrir les capteurs sur des modèles non répertoriés. Prévoir une commande MQTT `toshiba/<id>/scan` qui active un mode debug et logue toutes les trames inconnues.
7. **Mémoire** : L'ESP32 a 520 Ko de SRAM. Utilisez `heapless::Vec` et `heapless::String` pour éviter les allocations dynamiques dans le hot path (UART/MQTT).
8. **Task priorities** : Donnez une priorité FreeRTOS plus élevée à la task UART que à la task MQTT pour ne pas manquer de trames.

---

## 13. Checklist de validation avant mise en production

- [ ] Handshake réussi sur 10 démarrages consécutifs sans intervention.
- [ ] Commandes ON/OFF, mode, température testées et réactives (< 2s).
- [ ] Reconnexion WiFi testée (coupure 30s, reprise automatique).
- [ ] Reconnexion MQTT testée (broker redémarré, LWT correct).
- [ ] Déconnexion/reconnexion CN22 testée (re-handshake auto).
- [ ] Capteurs ODU/IDU parsés correctement (si présents sur l'unité).
- [ ] Mémoire stable : pas de fuite détectée sur 24h de fonctionnement (`esp_get_free_heap_size()`).
- [ ] Watchdog hardware alimenté (`CONFIG_ESP_TASK_WDT_EN=y`).
- [ ] Mode AP de secours fonctionnel (si WiFi invalide).
- [ ] Binaire signé pour OTA (si fonctionnalité activée).

---

*Bon courage ! Si vous bloquez sur un point précis (handshake, checksum, parsing d'une trame spécifique), n'hésitez pas à demander avec un extrait de log hexadécimal.*
