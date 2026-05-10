# Guide Exhaustif — Migration MQTT vers RMQTT (DalyBMS Pi5)

> **Capitalisé sur la migration rumqttd (mai 2026) — lire l'historique des erreurs avant de commencer.**
> Version rmqtt documentée : **0.20.0** (v2025.04)

---

## Table des matières

1. [Pourquoi RMQTT et pas rumqttd](#1-pourquoi-rmqtt-et-pas-rumqttd)
2. [Leçons apprises — erreurs de la migration rumqttd](#2-leçons-apprises)
3. [Architecture cible](#3-architecture-cible)
4. [Flux de données MQTT complet](#4-flux-de-données-mqtt-complet)
5. [Règles bridge anti-boucle (CRITIQUE)](#5-règles-bridge-anti-boucle-critique)
6. [Installation RMQTT sur Pi5 (aarch64)](#6-installation-rmqtt-sur-pi5-aarch64)
7. [Configuration rmqtt.toml complète](#7-configuration-rmqtttoml-complète)
8. [Configuration bridge ingress (NanoPi → Pi5)](#8-configuration-bridge-ingress-nanopi--pi5)
9. [Configuration bridge egress (Pi5 → NanoPi)](#9-configuration-bridge-egress-pi5--nanopi)
10. [Service systemd mqtt-broker](#10-service-systemd-mqtt-broker)
11. [Suppression du bridge custom (mqtt-bridge)](#11-suppression-du-bridge-custom-mqtt-bridge)
12. [Mise à jour Config.toml Pi5](#12-mise-à-jour-configtoml-pi5)
13. [Déploiement pas à pas](#13-déploiement-pas-à-pas)
14. [Vérification et validation](#14-vérification-et-validation)
15. [Procédure de rollback](#15-procédure-de-rollback)
16. [Checklist finale](#16-checklist-finale)

---

## 1. Pourquoi RMQTT et pas rumqttd

### Problèmes rencontrés avec rumqttd

| Problème | Impact |
|----------|--------|
| Pas de bridge natif | Nécessite un binaire custom `mqtt-bridge` avec logique manuelle anti-boucle |
| `LinkRx::recv()` synchrone | Oblige à `spawn_blocking` → complexité inutile |
| Feature flag `websocket` vs `websockets` | Erreur de compilation silencieuse difficile à diagnostiquer |
| Pas de protection anti-boucle intégrée | Boucle infinie de messages si une même subscription est dans les deux sens |
| Configuration TOML non documentée | `id = 0` requis au niveau racine non mentionné dans les docs |
| Pas d'API REST de management | Pas de moyen de vérifier l'état du broker sans logs |

### Avantages de RMQTT

| Fonctionnalité | rumqttd | RMQTT |
|----------------|---------|-------|
| Bridge MQTT natif (ingress + egress) | Non | **Oui** (plugins) |
| Anti-boucle bridge | Non | **Oui** (SHA-256 fingerprint TTL) |
| WebSocket natif | Via feature flag | **Oui** |
| MQTT v3.1.1 + v5.0 | v3.1.1 seulement | **Oui** |
| QoS 0, 1, 2 | Oui | **Oui** |
| API REST HTTP | Non | **Oui** (`rmqtt-http-api`) |
| Retained messages | Oui | **Oui** (`rmqtt-retainer`) |
| `cargo install` | Non | **Oui** (`cargo install rmqttd`) |
| Binary release aarch64 | Non | **Oui** (GitHub releases) |

---

## 2. Leçons apprises

### 2.1 La boucle infinie — l'erreur fatale

**Symptôme** : Devices qui flashent dans VRM (apparaissent/disparaissent toutes les 5 secondes), flood de messages dans l'explorateur MQTT.

**Cause** : Un topic présent dans les deux directions du bridge.

```
Pi5 publie santuario/bms/1/venus sur broker local
  → bridge local→nanopi le transfère sur NanoPi
    → bridge nanopi→local le ramène sur Pi5 (car santuario/# était souscrit des deux côtés)
      → Pi5 le republish → NanoPi → Pi5 → ... (boucle infinie)
```

**Règle absolue** : Chaque topic ne peut être que dans UNE SEULE direction.

### 2.2 Inventory complet des topics et leur direction

```
DIRECTION : Pi5 LOCAL → NanoPi
  santuario/bms/#          (BMS → D-Bus battery.mqtt_1/2)
  santuario/pvinverter/#   (ET112 → D-Bus pvinverter.mqtt_7)
  santuario/heatpump/#     (ET112 → D-Bus heatpump.mqtt_8/9)
  santuario/heat/#         (temp → D-Bus temperature.mqtt_1)
  santuario/switch/#       (ATS+Tongou → D-Bus switch.mqtt_1-6)
  santuario/grid/#         (grid → D-Bus grid.mqtt_n)
  santuario/meteo/#        (PRALRAN → D-Bus meteo)
  santuario/platform/#     (Pi5 platform)
  santuario/inverter/#     (onduleur EasySolar)
  santuario/system/#       (état système)
  W/{portal}/#             (commandes Venus OS → écriture D-Bus)
  R/{portal}/#             (keepalive / requêtes Venus OS)
  cmnd/#                   (commandes ON/OFF vers Tongou sur NanoPi)
  shellypro2pm-ec62608840a4/rpc  (commandes RPC vers Shelly sur NanoPi)

DIRECTION : NanoPi → Pi5 LOCAL
  N/{portal}/#             (données télémétriques Venus OS Cerbo GX)
  tele/#                   (Tongou SENSOR — mesures énergie)
  stat/#                   (Tongou POWER — état relais ON/OFF)
  shellypro2pm-ec62608840a4/#   (Shelly Pro 2PM — status + events)
  daly-bms-shelly/rpc      (réponses RPC Shelly)
```

### 2.3 Erreurs spécifiques à ne pas répéter

| Erreur | Contexte | Solution |
|--------|----------|----------|
| Feature `websockets` au lieu de `websocket` | Cargo.toml rumqttd | Toujours vérifier les noms exacts de features sur crates.io |
| `santuario/#` dans nanopi→local | bridge.rs | Ce namespace appartient à Pi5. NanoPi ne publie JAMAIS en `santuario/*` |
| `santuario/bms/#` absent de local→nanopi | bridge.rs | BMS invisibles sur VRM pendant 1h avant détection |
| `cmnd/#` absent de local→nanopi | bridge.rs | Commandes Tongou depuis Pi5 web ignorées silencieusement |
| `StartLimitIntervalSec` dans `[Service]` | systemd unit | Ce champ appartient à `[Unit]` |
| `AmbientCapabilities` + `ProtectSystem=strict` | systemd unit | Trop restrictif pour un broker — causait des échecs silencieux au démarrage |
| Port :1883 occupé par Docker Mosquitto | Déploiement | Toujours arrêter et supprimer le conteneur Docker avant de lancer le nouveau broker |

---

## 3. Architecture cible

```
Pi5 (192.168.1.141)
  rmqtt (systemd, :1883 TCP, :9001 WS, :8083 HTTP API)
    ├── TCP  :1883  ← tous les clients MQTT locaux
    ├── WS   :9001  ← explorateur dashboard JS
    ├── HTTP :8083  ← API REST + métriques
    ├── Bridge EGRESS (plugin natif) : Pi5→NanoPi
    │     santuario/#, W/{portal}/#, R/{portal}/#, cmnd/#,
    │     shellypro2pm.../rpc
    └── Bridge INGRESS (plugin natif) : NanoPi→Pi5
          N/{portal}/#, tele/#, stat/#,
          shellypro2pm-ec62608840a4/#, daly-bms-shelly/rpc

  daly-bms-server (systemd, :8080)
    ├── RS485 → publie santuario/* sur localhost:1883
    └── subscribe tele/+/SENSOR, stat/+/POWER, shellypro2pm.../status/*

  energy-manager (systemd, :8081)
    └── subscribe/publish santuario/* sur localhost:1883

NanoPi (192.168.1.120)
  Mosquitto (existant) :1883
    └── dbus-mqtt-venus subscribe santuario/* → D-Bus Victron
```

**Avantage clé** : Le crate `mqtt-bridge` custom est supprimé. Le bridge est géré nativement par RMQTT via ses plugins `rmqtt-bridge-egress-mqtt` et `rmqtt-bridge-ingress-mqtt`. La protection anti-boucle est intégrée (SHA-256 fingerprint).

---

## 4. Flux de données MQTT complet

### 4.1 BMS → VRM (le plus critique)

```
BMS Daly (RS485 /dev/ttyUSB0 addr 0x01/0x02)
  → daly-bms-server lit les données
  → publie santuario/bms/1/venus sur localhost:1883 (rmqtt)
  → rmqtt bridge EGRESS forward vers NanoPi:1883 (Mosquitto)
  → dbus-mqtt-venus subscribe santuario/bms/+/venus
  → écrit sur D-Bus com.victronenergy.battery.mqtt_1 (inst.151)
  → Venus OS → VRM affiche "Battery Monitor [151]"
```

### 4.2 ET112 Micro-Onduleurs → VRM

```
ET112 addr 0x07 (RS485)
  → daly-bms-server publie santuario/pvinverter/7/venus
  → bridge EGRESS → NanoPi
  → dbus-mqtt-venus → D-Bus pvinverter.mqtt_7 (inst.32)
  → VRM affiche "ET112-Micro-Onduleurs"
```

### 4.3 Tongou → Pi5 (mesures)

```
Tongou (sur NanoPi broker 192.168.1.120)
  → publie tele/tongou_3BC764/SENSOR
  → rmqtt bridge INGRESS ramène sur Pi5 localhost:1883
  → daly-bms-server subscribe tele/+/SENSOR → TasmotaSnapshot
  → affiche dans dashboard Pi5
```

### 4.4 Commande ON/OFF Tongou depuis Pi5 web

```
Utilisateur clique ON dans dashboard Pi5
  → API POST /api/v1/tasmota/1/control
  → daly-bms-server publie cmnd/tongou_3BC764/POWER sur localhost:1883
  → rmqtt bridge EGRESS forward vers NanoPi:1883
  → Tongou (abonné sur NanoPi) reçoit la commande → switch ON
```

### 4.5 Shelly Pro 2PM → Pi5

```
Shelly (sur NanoPi broker)
  → publie shellypro2pm-ec62608840a4/status/switch:0
  → bridge INGRESS ramène sur Pi5
  → daly-bms-server → ShellyEmSnapshot → dashboard

Pi5 envoie RPC GetStatus :
  → publie shellypro2pm-ec62608840a4/rpc sur Pi5 local
  → bridge EGRESS → NanoPi → Shelly
  → Shelly répond sur daly-bms-shelly/rpc (sur NanoPi)
  → bridge INGRESS → Pi5 → daly-bms-server reçoit la réponse
```

---

## 5. Règles bridge anti-boucle (CRITIQUE)

### Principe fondamental

RMQTT implémente la protection anti-boucle par **SHA-256 fingerprint** : chaque message forwarded est hashé (topic + payload + QoS + retain). Si le même hash est reçu dans la fenêtre TTL (défaut 60s), le message est dropé. Cela évite les boucles même si la configuration est imparfaite.

Malgré cette protection, la règle de conception reste : **un topic ne doit être que dans UNE direction**.

### Tableau de référence rapide

```
TOPIC                              EGRESS (Pi5→NanoPi)   INGRESS (NanoPi→Pi5)
─────────────────────────────────────────────────────────────────────────────
santuario/#                              ✓                      ✗
W/{portal}/#                             ✓                      ✗
R/{portal}/#                             ✓                      ✗
cmnd/#                                   ✓                      ✗
shellypro2pm-ec62608840a4/rpc            ✓                      ✗
N/{portal}/#                             ✗                      ✓
tele/#                                   ✗                      ✓
stat/#                                   ✗                      ✓
shellypro2pm-ec62608840a4/#              ✗                      ✓
daly-bms-shelly/rpc                      ✗                      ✓
─────────────────────────────────────────────────────────────────────────────
```

---

## 6. Installation RMQTT sur Pi5 (aarch64)

### Option A — Binary release GitHub (recommandé)

```bash
# Vérifier la dernière version sur https://github.com/rmqtt/rmqtt/releases
RMQTT_VERSION="0.20.0"

# Télécharger le binaire aarch64
wget https://github.com/rmqtt/rmqtt/releases/download/v${RMQTT_VERSION}/rmqttd-linux-aarch64.tar.gz \
  -O /tmp/rmqttd.tar.gz

# Extraire
tar -xzf /tmp/rmqttd.tar.gz -C /tmp/rmqttd/

# Installer le binaire
sudo cp /tmp/rmqttd/rmqttd /usr/local/bin/rmqttd
sudo chmod 755 /usr/local/bin/rmqttd

# Vérifier
rmqttd --version
```

> **Note** : Si le binaire aarch64 n'est pas disponible pour cette version, compiler depuis les sources (Option B).

### Option B — Compilation depuis les sources (cross-compilation depuis la machine de dev)

```bash
# Sur la machine de développement (x86_64)
cargo install cross --git https://github.com/cross-rs/cross

# Dans le répertoire du projet rmqtt cloné
git clone https://github.com/rmqtt/rmqtt.git
cd rmqtt

cross build --release --target aarch64-unknown-linux-gnu --bin rmqttd

# Copier vers Pi5
scp target/aarch64-unknown-linux-gnu/release/rmqttd pi5compute@192.168.1.141:/tmp/
ssh pi5compute@192.168.1.141 "sudo cp /tmp/rmqttd /usr/local/bin/ && sudo chmod 755 /usr/local/bin/rmqttd"
```

### Option C — cargo install sur Pi5 directement

```bash
# Sur le Pi5 (compilation native, ~15-20 min)
cargo install rmqttd

# Le binaire sera dans ~/.cargo/bin/
sudo cp ~/.cargo/bin/rmqttd /usr/local/bin/
```

### Créer l'utilisateur système et les répertoires

```bash
# Utilisateur système
sudo useradd --system --no-create-home --shell /usr/sbin/nologin rmqtt

# Répertoires
sudo mkdir -p /etc/rmqtt/plugins
sudo mkdir -p /var/lib/rmqtt
sudo chown rmqtt:rmqtt /var/lib/rmqtt
sudo chmod 750 /var/lib/rmqtt
```

---

## 7. Configuration rmqtt.toml complète

Créer `/etc/rmqtt/rmqtt.toml` :

```toml
# =============================================================================
# rmqtt.toml — Configuration broker RMQTT pour DalyBMS / Pi5
# Version : 0.20.0
# =============================================================================

[node]
id = 1
plugins_dir = "/etc/rmqtt/plugins"
plugins_default_startups = [
    "rmqtt-retainer",
    "rmqtt-http-api",
    "rmqtt-bridge-ingress-mqtt",
    "rmqtt-bridge-egress-mqtt",
]

[log]
level = "info"
# Rotation des logs
to = "file"
dir = "/var/log/rmqtt"

# ── Listener MQTT TCP :1883 ───────────────────────────────────────────────────
[[listeners.tcp]]
name = "external"
addr = "0.0.0.0:1883"
# Connexions simultanées max
max_connections = 500

  [listeners.tcp.options]
  connect_timeout = "15s"
  max_packet_size = "1mb"
  # Pas d'authentification (accès local uniquement)
  allow_anonymous = true

# ── Listener WebSocket MQTT :9001 ─────────────────────────────────────────────
[[listeners.ws]]
name = "websocket"
addr = "0.0.0.0:9001"

  [listeners.ws.options]
  connect_timeout = "15s"
  max_packet_size = "1mb"
  allow_anonymous = true
```

---

## 8. Configuration bridge ingress (NanoPi → Pi5)

Créer `/etc/rmqtt/plugins/rmqtt-bridge-ingress-mqtt.toml` :

```toml
# =============================================================================
# Bridge INGRESS : NanoPi (192.168.1.120) → Pi5 local
#
# ⚠ RÈGLE ANTI-BOUCLE : Ces topics doivent être ABSENTS du bridge egress.
#   NanoPi publie : N/{portal}/#, tele/#, stat/#, shellypro2pm/#, daly-bms-shelly/rpc
#   Pi5 publie   : santuario/*, cmnd/*, W/*, R/*, shellypro2pm.../rpc
# =============================================================================

[[bridges]]
enable = true
name = "nanopi-to-pi5"
client_id_prefix = "dalybms-bridge-in"
server = "192.168.1.120:1883"
# Pas d'auth sur Mosquitto NanoPi
# username = ""
# password = ""

connect_timeout = "20s"
keepalive = "60s"
reconnect_interval = "5s"
expiry_interval = "5m"
mqtt_ver = "v4"

  [bridges.options]
  clean_session = true
  concurrent_client_limit = 1

  # ── Venus OS télémétriques (Cerbo GX) ────────────────────────────────────
  [[bridges.entries]]
  [bridges.entries.remote]
  # c0619ab9929a = portal_id du Cerbo GX (vérifier dans Config.toml)
  topic = "N/c0619ab9929a/#"
  qos = 0
  [bridges.entries.local]
  topic = "${remote.topic}"
  qos = 0
  retain = false

  # ── Tasmota / Tongou — mesures énergie ───────────────────────────────────
  [[bridges.entries]]
  [bridges.entries.remote]
  topic = "tele/#"
  qos = 0
  [bridges.entries.local]
  topic = "${remote.topic}"
  qos = 0
  retain = false

  # ── Tasmota / Tongou — état relais ───────────────────────────────────────
  [[bridges.entries]]
  [bridges.entries.remote]
  topic = "stat/#"
  qos = 0
  [bridges.entries.local]
  topic = "${remote.topic}"
  qos = 0
  retain = false

  # ── Shelly Pro 2PM — status + events ─────────────────────────────────────
  [[bridges.entries]]
  [bridges.entries.remote]
  topic = "shellypro2pm-ec62608840a4/#"
  qos = 0
  [bridges.entries.local]
  topic = "${remote.topic}"
  qos = 0
  retain = false

  # ── Réponses RPC Shelly ───────────────────────────────────────────────────
  [[bridges.entries]]
  [bridges.entries.remote]
  topic = "daly-bms-shelly/rpc"
  qos = 0
  [bridges.entries.local]
  topic = "${remote.topic}"
  qos = 0
  retain = false
```

---

## 9. Configuration bridge egress (Pi5 → NanoPi)

Créer `/etc/rmqtt/plugins/rmqtt-bridge-egress-mqtt.toml` :

```toml
# =============================================================================
# Bridge EGRESS : Pi5 local → NanoPi (192.168.1.120)
#
# ⚠ RÈGLE ANTI-BOUCLE : Ces topics doivent être ABSENTS du bridge ingress.
#   Tous les topics santuario/* viennent de Pi5, JAMAIS de NanoPi.
# =============================================================================

[[bridges]]
enable = true
name = "pi5-to-nanopi"
client_id_prefix = "dalybms-bridge-out"
server = "192.168.1.120:1883"

connect_timeout = "20s"
keepalive = "60s"
reconnect_interval = "5s"
message_channel_capacity = 1000
mqtt_ver = "v4"

  [bridges.options]
  clean_session = true
  concurrent_client_limit = 1

  # ── BMS → battery.mqtt_1 / battery.mqtt_2 ────────────────────────────────
  [[bridges.entries]]
  [bridges.entries.local]
  topic = "santuario/bms/#"
  qos = 1
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 1
  retain = true

  # ── ET112 Micro-Onduleurs → pvinverter.mqtt_7 ─────────────────────────────
  [[bridges.entries]]
  [bridges.entries.local]
  topic = "santuario/pvinverter/#"
  qos = 0
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 0
  retain = true

  # ── ET112 Maison + Réseau → heatpump.mqtt_8/9 ────────────────────────────
  [[bridges.entries]]
  [bridges.entries.local]
  topic = "santuario/heatpump/#"
  qos = 0
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 0
  retain = true

  # ── Température extérieure → temperature.mqtt_1 ───────────────────────────
  [[bridges.entries]]
  [bridges.entries.local]
  topic = "santuario/heat/#"
  qos = 0
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 0
  retain = true

  # ── ATS CHINT + Tongou → switch.mqtt_1/2/3/4/5/6 ────────────────────────
  [[bridges.entries]]
  [bridges.entries.local]
  topic = "santuario/switch/#"
  qos = 0
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 0
  retain = true

  # ── Compteurs réseau → grid.mqtt_n ───────────────────────────────────────
  [[bridges.entries]]
  [bridges.entries.local]
  topic = "santuario/grid/#"
  qos = 0
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 0
  retain = true

  # ── Irradiance PRALRAN → meteo ───────────────────────────────────────────
  [[bridges.entries]]
  [bridges.entries.local]
  topic = "santuario/meteo/#"
  qos = 0
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 0
  retain = true

  # ── Platform Pi5, Inverter, System ───────────────────────────────────────
  [[bridges.entries]]
  [bridges.entries.local]
  topic = "santuario/platform/#"
  qos = 0
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 0
  retain = false

  [[bridges.entries]]
  [bridges.entries.local]
  topic = "santuario/inverter/#"
  qos = 0
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 0
  retain = false

  [[bridges.entries]]
  [bridges.entries.local]
  topic = "santuario/system/#"
  qos = 0
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 0
  retain = false

  # ── Venus OS commandes (écriture D-Bus) — QoS 1 ──────────────────────────
  [[bridges.entries]]
  [bridges.entries.local]
  topic = "W/c0619ab9929a/#"
  qos = 1
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 1
  retain = false

  # ── Venus OS keepalive (lecture D-Bus) — QoS 1 ───────────────────────────
  [[bridges.entries]]
  [bridges.entries.local]
  topic = "R/c0619ab9929a/#"
  qos = 1
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 1
  retain = false

  # ── Commandes ON/OFF Tongou depuis Pi5 web ────────────────────────────────
  [[bridges.entries]]
  [bridges.entries.local]
  topic = "cmnd/#"
  qos = 1
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 1
  retain = false

  # ── Commandes RPC Shelly (Pi5 → Shelly via NanoPi) ───────────────────────
  [[bridges.entries]]
  [bridges.entries.local]
  topic = "shellypro2pm-ec62608840a4/rpc"
  qos = 0
  [bridges.entries.remote]
  topic = "${local.topic}"
  qos = 0
  retain = false
```

> **Important** : Remplacer `c0619ab9929a` par le `portal_id` réel du Cerbo GX.
> Le vérifier dans `Config.toml` section `[mqtt_bridge]` ou avec :
> `ssh root@192.168.1.120 "dbus -y | grep N/" | head -3`

---

## 10. Service systemd mqtt-broker

Créer `/etc/systemd/system/rmqtt-broker.service` :

```ini
[Unit]
Description=RMQTT Broker — remplace Mosquitto/Docker sur Pi5
Documentation=https://github.com/rmqtt/rmqtt
After=network.target
Before=daly-bms.service energy-manager.service
StartLimitIntervalSec=0

[Service]
Type=simple
User=rmqtt
Group=rmqtt

ExecStart=/usr/local/bin/rmqttd -c /etc/rmqtt/rmqtt.toml

WorkingDirectory=/var/lib/rmqtt

Environment=RUST_LOG=rmqtt=info

Restart=on-failure
RestartSec=5s

LimitNOFILE=65536
MemoryMax=200M

[Install]
WantedBy=multi-user.target
```

Ajouter la dépendance dans `daly-bms.service` :

```ini
[Unit]
...
After=network.target rmqtt-broker.service
Requires=rmqtt-broker.service
```

---

## 11. Suppression du bridge custom (mqtt-bridge)

Avec RMQTT, le crate `mqtt-bridge` n'est plus nécessaire. La suppression est propre :

```bash
# Sur Pi5 — désactiver et supprimer le service
sudo systemctl stop mqtt-bridge
sudo systemctl disable mqtt-bridge
sudo rm /etc/systemd/system/mqtt-bridge.service
sudo rm /usr/local/bin/mqtt-bridge
sudo systemctl daemon-reload
```

Dans le `Cargo.toml` du workspace, retirer :
```toml
# Supprimer ces lignes :
"crates/mqtt-bridge",
```

Et dans `Makefile`, retirer les targets `build-mqtt-arm` liés à `mqtt-bridge`.

> Le crate `mqtt-broker` est également supprimé — RMQTT est maintenant le broker.

---

## 12. Mise à jour Config.toml Pi5

Les sections MQTT dans `Config.toml` doivent pointer vers le broker local (`127.0.0.1:1883`) :

```toml
# Section principale MQTT (daly-bms-server)
[mqtt]
host     = "127.0.0.1"
port     = 1883
# Pas de username/password — RMQTT en mode anonymous sur localhost

# Section energy-manager
[energy_manager]
  [energy_manager.mqtt]
  host = "127.0.0.1"
  port = 1883

# Section mqtt_bridge — SUPPRIMER cette section entière
# (le bridge est maintenant géré par RMQTT nativement)
# [mqtt_bridge]    ← à supprimer
```

Après modification :
```bash
sudo cp Config.toml /etc/daly-bms/config.toml
```

---

## 13. Déploiement pas à pas

### Étape 1 — Préparer l'environnement

```bash
cd ~/Daly-BMS-Rust
make sync

# Vérifier que Docker Mosquitto tourne encore
docker ps | grep mosquitto
```

### Étape 2 — Installer RMQTT

```bash
# Voir section 6 pour les options d'installation
# Vérifier :
rmqttd --version
# Attendu : rmqttd 0.20.0
```

### Étape 3 — Déployer les configurations

```bash
sudo mkdir -p /etc/rmqtt/plugins /var/log/rmqtt

# Config principale
sudo cp crates/rmqtt-broker/rmqtt.toml /etc/rmqtt/rmqtt.toml

# Bridge ingress (NanoPi→Pi5)
sudo cp crates/rmqtt-broker/plugins/rmqtt-bridge-ingress-mqtt.toml \
    /etc/rmqtt/plugins/

# Bridge egress (Pi5→NanoPi)
sudo cp crates/rmqtt-broker/plugins/rmqtt-bridge-egress-mqtt.toml \
    /etc/rmqtt/plugins/

sudo chown -R rmqtt:rmqtt /etc/rmqtt /var/log/rmqtt /var/lib/rmqtt
```

### Étape 4 — Arrêter Docker Mosquitto

```bash
# ⚠ Ne pas le faire avant que RMQTT soit prêt à démarrer
docker stop dalybms-mosquitto
docker rm dalybms-mosquitto
# Vérifier que le port est libéré
ss -tlnp | grep 1883
# Doit être vide
```

### Étape 5 — Démarrer RMQTT

```bash
sudo systemctl daemon-reload
sudo systemctl enable rmqtt-broker
sudo systemctl start rmqtt-broker
sleep 5

# Vérifier
systemctl status rmqtt-broker --no-pager
journalctl -u rmqtt-broker -n 30 --no-pager
```

### Étape 6 — Vérifier le broker

```bash
# TCP :1883 actif ?
ss -tlnp | grep 1883

# API HTTP actif ?
curl -s http://localhost:8083/api/v1/status | jq .

# Test connexion MQTT locale
mosquitto_sub -h localhost -p 1883 -t "test/#" -v &
mosquitto_pub -h localhost -p 1883 -t "test/ping" -m "hello"
# Attendu : test/ping hello
```

### Étape 7 — Vérifier les bridges

```bash
# Bridge ingress (NanoPi→Pi5) actif ?
# Sur Pi5, souscrire à N/{portal}/#
mosquitto_sub -h localhost -p 1883 -t "N/#" -v
# Doit voir les données Venus OS arriver

# Bridge egress (Pi5→NanoPi) actif ?
# Sur NanoPi, souscrire à santuario/bms/#
ssh root@192.168.1.120 "mosquitto_sub -h localhost -t 'santuario/bms/#' -v -C 3"
# Doit voir les données BMS arriver depuis Pi5
```

### Étape 8 — Déployer daly-bms-server et energy-manager

```bash
make build-arm
sudo systemctl stop daly-bms
sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/
sudo systemctl start daly-bms
sleep 5

make build-energy-arm
sudo systemctl stop energy-manager
sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/
sudo systemctl start energy-manager
```

---

## 14. Vérification et validation

### Checklist VRM (vérifier sur https://vrm.victronenergy.com)

```
□ Battery Monitor [151] "just now"    ← BMS 360Ah
□ Battery Monitor [152] "just now"    ← BMS 320Ah
□ ET112-Micro-Onduleurs "just now"    ← pvinverter.mqtt_7
□ PAC Chauffe-eau "just now"          ← heatpump.mqtt_8
□ PAC Climatisation "just now"        ← heatpump.mqtt_9
□ ATS CHINT "just now"                ← switch.mqtt_1
□ Tongou 1-5 "just now"               ← switch.mqtt_2/3/4/5/6
□ Capteur météo actif                 ← meteo
□ Réseau affiché correctement         ← grid
```

### Checklist Pi5 Dashboard

```
□ Page /dashboard/visualization — tous les nodes actifs (pas de "Hors ligne")
□ Page /dashboard/tasmota — Tongou 1-5 : puissance, tension, courant affichés
□ Page /dashboard/tasmota — commande ON/OFF fonctionne
□ Page /dashboard/shelly — Shelly Pro 2PM : données affichées
□ Page /dashboard/bms/1 et /2 — données BMS actualisées
□ Explorateur MQTT WebSocket :9001 — PAS de flood (< 5 msg/s en veille)
```

### Vérifier l'absence de boucle

```bash
# Compter les messages santuario/bms/# sur Pi5 pendant 10s
timeout 10 mosquitto_sub -h localhost -p 1883 -t "santuario/bms/#" -v | wc -l
# Attendu : ~4 messages (2 BMS × 2 updates en 10s)
# Si > 50 messages → boucle bridge → vérifier la config ingress
```

### Logs à surveiller

```bash
# Broker RMQTT
journalctl -u rmqtt-broker -f

# BMS server
journalctl -u daly-bms -f | grep -E "MQTT|bms|bridge|error"

# NanoPi dbus-mqtt-venus
ssh root@192.168.1.120 "tail -f /var/log/dbus-mqtt-venus/current"
```

---

## 15. Procédure de rollback

En cas d'échec, retour à Mosquitto Docker en **< 5 minutes** :

```bash
# 1. Arrêter RMQTT
sudo systemctl stop rmqtt-broker
sudo systemctl disable rmqtt-broker

# 2. Relancer Docker Mosquitto
cd ~/Daly-BMS-Rust
docker compose -f docker-compose.infra.yml up -d

# 3. Vérifier Mosquitto actif
docker ps | grep mosquitto
ss -tlnp | grep 1883

# 4. Redémarrer les services
sudo systemctl restart daly-bms energy-manager

# 5. Vérifier
systemctl status daly-bms energy-manager
```

---

## 16. Checklist finale

### Avant de commencer la migration

```
□ Faire un git commit de tout le code stable
□ Créer un tag git : git tag -a v-pre-rmqtt -m "avant migration RMQTT"
□ Noter l'heure : si problème > 30min → rollback immédiat
□ Vérifier que NanoPi est accessible : ping 192.168.1.120
□ Vérifier le portal_id dans Config.toml [mqtt_bridge]
□ Préparer le binaire rmqttd en avance (ne pas compiler pendant la migration)
```

### Pendant la migration

```
□ Arrêter Docker Mosquitto SEULEMENT quand RMQTT est prêt (pas avant)
□ Tester le broker RMQTT avec mosquitto_pub/sub avant de redémarrer daly-bms
□ Vérifier les bridges (ingress ET egress) avant de redémarrer energy-manager
□ Surveiller les logs en temps réel pendant 5 minutes après chaque démarrage
```

### Après la migration

```
□ VRM : tous les devices "just now" (pas "an hour ago")
□ Dashboard Pi5 : pas de "Hors ligne" ou "En attente de données"
□ Tasmota : commandes ON/OFF fonctionnent depuis Pi5 web
□ Shelly : données affichées + contrôle DEYE fonctionne
□ Pas de flood MQTT (vérifier avec explorateur :9001)
□ Mettre à jour CLAUDE.md section architecture avec la nouvelle config
```

---

## Références

- [GitHub rmqtt/rmqtt](https://github.com/rmqtt/rmqtt)
- [Bridge Egress MQTT](https://github.com/rmqtt/rmqtt/blob/master/docs/en_US/bridge-egress-mqtt.md)
- [Bridge Ingress MQTT](https://github.com/rmqtt/rmqtt/blob/master/docs/en_US/bridge-ingress-mqtt.md)
- [Installation](https://github.com/rmqtt/rmqtt/blob/master/docs/en_US/install.md)
- [Releases](https://github.com/rmqtt/rmqtt/releases)
- [rmqttd sur lib.rs](https://lib.rs/crates/rmqttd)
- [DeepWiki rmqtt](https://deepwiki.com/rmqtt/rmqtt)
