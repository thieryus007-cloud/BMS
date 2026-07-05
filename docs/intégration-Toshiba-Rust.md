Plan de projet : Transposition du pilote Toshiba SUZUMI en Rust pour ESP32

reference: https://github.com/pedobry/esphome_toshiba_suzumi

1. Objectifs du projet

· Remplacer le composant ESPHome (C++) par un firmware Rust autonome sur ESP32.
· Communiquer avec le climatiseur Toshiba via le connecteur UART CN22.
· Publier l'état du climatiseur sur un broker MQTT.
· Recevoir des commandes MQTT pour piloter le climatiseur.
· Fonctionner sans dépendance à Home Assistant ou tout autre système central.

---

2. Architecture fonctionnelle

```
┌─────────────┐    UART (CN22)    ┌─────────────┐
│ Climatiseur │ ◄───────────────► │   ESP32     │
│   Toshiba   │    (5V ↔ 3.3V)    │  (Rust)     │
└─────────────┘                   └──────┬──────┘
                                         │ WiFi
                                   ┌──────▼──────┐
                                   │ Broker MQTT│
                                   └─────────────┘
```

L'ESP32 agit comme passerelle :

· Il traduit les commandes MQTT en trames UART Toshiba.
· Il traduit les trames UART reçues en messages MQTT (état, température, etc.).

---

3. Prérequis matériels

Élément Référence
ESP32 Module quelconque (DevKit, WROOM, etc.)
Convertisseur de niveau logique 5V ↔ 3.3V (bidirectionnel, ex: TXB0108)
Alimentation Le climatiseur fournit du 5V sur CN22 (broche 3) ou alimentation externe
Câblage 4 fils (TX, RX, GND, 5V) + résistance de pull-up éventuelle

Connexions CN22 -> ESP32 :

CN22 (broche) Couleur Fonction ESP32 GPIO
1 Bleu TX → RX de l'ESP GPIO 33
2 Rose GND GND
3 Noir 5V (alim) Vin (si compatible)
4 Blanc RX ← TX de l'ESP GPIO 32

Attention : Un convertisseur 5V/3.3V doit être intercalé sur les lignes TX/RX.

---

4. Structure du code Rust (cargo)

```
toshiba-rs/
├── .cargo/config.toml         (cible xtensa-esp32-espidf)
├── Cargo.toml
├── build.rs                   (config ESP-IDF)
├── src/
│   ├── main.rs                (point d'entrée, loop principale)
│   ├── protocol.rs            (définition des trames, checksum, parsing)
│   ├── uart.rs                (wrapper UART pour ESP-IDF)
│   ├── mqtt.rs                (gestion WiFi + client MQTT)
│   └── command_queue.rs       (file d'attente des commandes)
└── sdkconfig.defaults         (paramètres ESP-IDF)
```

---

5. Étapes détaillées de développement

Phase 1 : Environnement et squelettage (1h)

1. Installer espup, cargo-generate.
2. Générer un projet avec cargo generate esp-rs/esp-idf-template.
3. Configurer Cargo.toml pour les dépendances :
   ```toml
   [dependencies]
   esp-idf-svc = "0.48"
   esp-idf-hal = "0.43"
   mqtt = { version = "0.12", features = ["esp-idf"] }
   embedded-svc = "0.26"
   ```
4. Configurer les broches WiFi et UART via sdkconfig.defaults (ou via code).

Phase 2 : Transposition du protocole (2h)

1. Traduire les constantes : les tableaux HANDSHAKE, AFTER_HANDSHAKE en &'static [u8].
2. Écrire les fonctions pures :
   · compute_checksum(data: &[u8]) -> u8
   · validate_message(buffer: &[u8]) -> Option<&[u8]>
   · parse_response(data: &[u8]) -> Option<ToshibaStatus>
3. Définir les énumérations :
   ```rust
   #[repr(u8)]
   enum CmdType { Power=48, Mode=49, TargetTemp=50, ... }
   ```
4. Fonction build_command(cmd_type: u8, value: u8) -> Vec<u8>.

Phase 3 : Interface UART (1h)

1. Initialiser l'UART :
   ```rust
   let uart = UartDriver::new(
       Peripherals::take().uart1,
       UartConfig::new(9600, // baudrate (vérifier celui du protocole)
                       UartDataBits::Data8,
                       UartStopBits::Stop1,
                       UartParity::None,
                       Option::<Gpio32>::Some(gpio32), // TX
                       Option::<Gpio33>::Some(gpio33), // RX
   )?;
   ```
2. Implémenter send_bytes(data: &[u8]) et read_bytes(timeout_ms: u32) -> Vec<u8>.

Phase 4 : WiFi et MQTT (2h)

1. Initialiser le WiFi (station) en utilisant esp_idf_svc::wifi::EspWifi.
2. Se connecter au broker MQTT (ex: mqtt::client::EspMqttClient).
3. Souscrire au topic de commandes (ex: climatiseur/commande).
4. Publier les états sur un topic (ex: climatiseur/etat).

Phase 5 : Logique applicative (2h)

1. Gestion de la file d'attente :
   · VecDeque<Vec<u8>> pour stocker les commandes à envoyer.
2. Séquence de handshake :
   · Envoyer les 6 trames HANDSHAKE avec un court délai (50 ms).
   · Attendre 2 secondes.
   · Envoyer les 2 trames AFTER_HANDSHAKE.
3. Boucle principale (loop) :
   · Si la file n'est pas vide, envoyer la commande suivante.
   · Lire les données UART disponibles.
   · Valider et parser les trames reçues.
   · Si une trame valide est trouvée, extraire l'état et le publier en MQTT.
   · Gérer les timeouts (réception de 200 ms max).
   · Ajouter un watchdog logiciel pour éviter les blocages.

Phase 6 : Tests et validation (4h–8h)

1. Tests unitaires (sur PC) : vérifier checksum, parsing, construction de trames.
2. Tests sur cible nue :
   · Vérifier l'envoi du handshake (avec oscilloscope ou analyseur logique).
   · Simuler des réponses du climatiseur avec un PC (via USB-UART) pour valider le parsing.
3. Tests réels avec le climatiseur :
   · Démarrer le handshake, observer les réponses.
   · Vérifier que la température et l'état sont corrects.
   · Envoyer des commandes (marche/arrêt, changement de mode, réglage température).
4. Débogage :
   · Utiliser log (avec esp-idf-svc) pour tracer en UART ou via defmt.
   · Ajuster les délais si nécessaire (le handshake est critique).

---

6. Détails techniques critiques

Checksum

· Algorithme : (256 - (somme des octets de l'index 1 à len-2) % 256) % 256 (en C++ un uint8_t qui overflow).
· En Rust : utiliser wrapping_add pour les sommes.

Temporisation

· Délai entre les trames de handshake : 50 ms (configurable).
· Délai de 2 secondes après les 6 premières trames avant d'envoyer AFTER_HANDSHAKE.
· Timeout de réception : 200 ms.

Gestion des erreurs

· Si le climatiseur ne répond pas, recommencer le handshake après 10 secondes (à implémenter).
· En cas d'échec WiFi / MQTT, tenter une reconnexion automatique.

---

7. Déploiement et maintenance

· Flasher le binaire avec espflash ou cargo run.
· Les paramètres (SSID, mot de passe, broker MQTT) peuvent être stockés en NVS ou via des variables d'environnement au build.
· Mise à jour OTA envisageable en option (via esp-idf-svc).

---

8. Ressources et documentation

· The Rust on ESP Book
· esp-idf-hal documentation
· esp-idf-svc examples
· MQTT crate for ESP-IDF
· Code source original C++ : pedobry/esphome_toshiba_suzumi

---

9. Calendrier prévisionnel (pour un expert C++/Rust)

Phase Durée (heures) Cumul (heures)
Environnement + squelettage 1 1
Protocole (transposition) 2 3
UART 1 4
WiFi + MQTT 2 6
Logique applicative 2 8
Tests et débogage 4–8 12–16

Soit 2 à 3 jours de travail intensif, ou 1 semaine à rythme normal.

---

10. Conseils supplémentaires

· Démarrage progressif : commencez par un MVP qui envoie le handshake et affiche les réponses sur la console UART (sans MQTT). Ajoutez ensuite MQTT.
· Utilisez le logging : esp_idf_svc::sys::link_patches() et esp_idf_svc::log::EspLogger pour voir les traces sur la console série.
· Gardez une version de secours : avant de flasher, assurez-vous de pouvoir récupérer l'ESP32 en mode boot (GPIO0 à GND).

---

Bon courage ! Si vous bloquez sur un point précis, n'hésitez pas à demander.
