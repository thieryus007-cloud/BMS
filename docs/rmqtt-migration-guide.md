# Guide Exhaustif — Migration MQTT vers RMQTT (DalyBMS Pi5)

> **Capitalisé sur la migration rumqttd (mai 2026) — lire l'historique des erreurs avant de commencer.**
> Version rmqtt documentée : **0.20.0** (v2025.04)
>
> **Mise à jour mai 2026 — corrections vs. l'état réel du dépôt :**
> - L'état initial n'a **pas** de crates `mqtt-bridge` ni `mqtt-broker` à supprimer (le bridge actuel est dans la conf Mosquitto Docker).
> - Le broker actuel est **Mosquitto en Docker** (`dalybms-mosquitto`), avec un bridge configuré dans `docker/mosquitto/config/mosquitto.conf` vers NanoPi.
> - `Config.toml` actuel : `[mqtt].host = "192.168.1.120"` (NanoPi) et `[energy_manager.mqtt].host = "192.168.1.141"` (Pi5). Les deux doivent passer à `127.0.0.1`.
> - Il n'existe **pas** de section `[mqtt_bridge]` dans `Config.toml` — `portal_id` est dans `[energy_manager]` (ligne 542).
> - L'arrêt de Docker Mosquitto désactive automatiquement son bridge `nanopi-venus-bridge` — il n'y a donc rien à reconfigurer côté NanoPi.

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

### 7.1 Plugin HTTP API — `/etc/rmqtt/plugins/rmqtt-http-api.toml`

> **Sans ce fichier**, le plugin `rmqtt-http-api` se charge mais ne bind aucun port :
> les vérifications `curl http://localhost:8083/...` du §13.6 échoueront. À créer
> obligatoirement, sinon retirer `rmqtt-http-api` de `plugins_default_startups`.

```toml
# Plugin REST de management — utile pour debug ("clients", "subscriptions",
# "metrics", "stats"). Bind localhost uniquement (pas d'auth → ne pas exposer).
http_laddr = "127.0.0.1:8083"
workers = 1
max_row_limit = 10_000
message_type = 1
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
> Vérifier dans `Config.toml` section `[energy_manager]` (clé `portal_id`) :
> ```bash
> grep '^portal_id' Config.toml
> # ou côté NanoPi :
> ssh root@192.168.1.120 "mosquitto_sub -h localhost -t 'N/+/system/0/Serial' -C 1 -W 5"
> ```

---

## 10. Services systemd

### 10.1 Nouveau service `rmqtt-broker.service`

Créer `contrib/rmqtt-broker.service` (versionné dans le dépôt) :

```ini
[Unit]
Description=RMQTT Broker — remplace Mosquitto/Docker sur Pi5
Documentation=https://github.com/rmqtt/rmqtt
After=network-online.target
Wants=network-online.target
Before=daly-bms.service energy-manager.service
StartLimitIntervalSec=0

[Service]
Type=simple
User=rmqtt
Group=rmqtt

ExecStart=/usr/local/bin/rmqttd -f /etc/rmqtt/rmqtt.toml

WorkingDirectory=/var/lib/rmqtt

Environment=RUST_LOG=rmqtt=info,rmqtt_bridge_ingress_mqtt=info,rmqtt_bridge_egress_mqtt=info

Restart=on-failure
RestartSec=5s

LimitNOFILE=65536
# 200 Mo : marge confortable pour bridge + retainer + WS (mesuré ~80 Mo en charge)
MemoryMax=200M

# Sécurité — assez permissif pour pouvoir écrire /var/log/rmqtt et /var/lib/rmqtt
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=full
ProtectHome=true
ReadWritePaths=/var/lib/rmqtt /var/log/rmqtt

StandardOutput=journal
StandardError=journal
SyslogIdentifier=rmqtt-broker

[Install]
WantedBy=multi-user.target
```

> **Pièges connus** :
> - L'option CLI est `-f` (file) sur rmqttd ≥ 0.20, **pas** `-c`. Vérifier avec `rmqttd --help`.
> - Ne **pas** ajouter `ProtectSystem=strict` ni `AmbientCapabilities=` (échecs silencieux constatés en migration rumqttd, voir §2.3).
> - `StartLimitIntervalSec=0` est correct dans `[Unit]` (pas dans `[Service]`).

### 10.2 Patch `contrib/daly-bms.service`

Le fichier actuel (`contrib/daly-bms.service`) n'a aucune dépendance MQTT. Modifier l'en-tête :

```ini
[Unit]
Description=DalyBMS Server — Rust RS485 BMS monitor
Documentation=https://github.com/thieryus007-cloud/Daly-BMS-Rust
After=network.target rmqtt-broker.service
Wants=network.target
Requires=rmqtt-broker.service
```

(Le reste du fichier — `[Service]`, `[Install]` — est inchangé.)

### 10.3 Patch `contrib/energy-manager.service`

Le fichier actuel référence `mosquitto.service` (qui n'a jamais existé en tant que service systemd hôte sur le Pi5 — Mosquitto tournait en Docker). Remplacer la ligne `After=` :

```ini
[Unit]
Description=Energy Manager — Gestionnaire d'énergie Rust (remplace Node-RED)
Documentation=https://github.com/thieryus007-cloud/Daly-BMS-Rust
After=network-online.target rmqtt-broker.service daly-bms.service
Wants=network-online.target
Requires=rmqtt-broker.service
PartOf=daly-bms.service
```

### 10.4 Déploiement des unités

```bash
sudo cp contrib/rmqtt-broker.service /etc/systemd/system/
sudo cp contrib/daly-bms.service     /etc/systemd/system/
sudo cp contrib/energy-manager.service /etc/systemd/system/
sudo systemctl daemon-reload
```

---

## 11. Retrait de Mosquitto Docker (le vrai « ancien bridge »)

> ⚠ **Correction vs. version initiale du guide** : il n'existe **pas** de crates
> `mqtt-bridge` ni `mqtt-broker` dans le workspace, donc rien à retirer dans
> `Cargo.toml`. Le « bridge custom » à supprimer est en réalité **la
> configuration bridge dans Mosquitto Docker** (`docker/mosquitto/config/mosquitto.conf`,
> bloc `connection nanopi-venus-bridge`).

### 11.1 Stopper et supprimer le conteneur

```bash
cd ~/Daly-BMS-Rust
docker compose -f docker-compose.infra.yml down
# (ne supprime PAS les volumes — utile pour rollback). Pour purge complète :
# docker compose -f docker-compose.infra.yml down -v
docker ps -a | grep mosquitto   # doit être vide
```

### 11.2 Retirer (ou archiver) les fichiers Docker

Une fois la migration validée 24h en production :

```bash
# Garder une trace dans git plutôt que rm immédiat
git mv docker-compose.infra.yml docker-compose.infra.yml.bak
git mv docker/mosquitto         docker/mosquitto.bak
```

### 11.3 Mettre à jour le Makefile

Les targets `make up` / `make down` / `make logs` / `make reset` / `make restart` / `make ps` (lignes 47-64 de `Makefile`) deviennent obsolètes. Deux options :

- **Option A (recommandée)** : remplacer par des wrappers `systemctl` :
  ```makefile
  up:
  	sudo systemctl start rmqtt-broker
  down:
  	sudo systemctl stop rmqtt-broker
  logs:
  	journalctl -u rmqtt-broker -f
  restart:
  	sudo systemctl restart rmqtt-broker
  ps:
  	systemctl status rmqtt-broker --no-pager
  ```
- **Option B** : supprimer les targets et les remplacer par la commande directe dans `CLAUDE.md`.

> Penser aussi à mettre à jour la ligne 5 (`# make up → démarrer l'infra Docker`) et le bloc d'aide ligne 370.

### 11.4 Mettre à jour `CLAUDE.md`

Sections à modifier :
- Section 1 (Architecture) : remplacer `Docker: mosquitto:1883` par `systemd: rmqtt-broker.service (1883/9001/8083)`.
- Section 0 (Commandes rapides) : retirer/remplacer la ligne `Docker start/stop/logs : make up / make down / make logs`.
- Section 8 (Problèmes courants) : ajouter une entrée pour `rmqtt-broker` (`journalctl -u rmqtt-broker -n 50`).

---

## 12. Mise à jour Config.toml Pi5

Les deux sections MQTT actuelles pointent vers des hôtes différents et doivent toutes deux passer à `127.0.0.1` (broker local RMQTT) :

### 12.1 Section `[mqtt]` (daly-bms-server) — ligne ~74

**Avant :**
```toml
[mqtt]
enabled = true
host = "192.168.1.120"          # ← NanoPi
port = 1883
topic_prefix = "santuario/bms"
publish_interval_sec = 1
format = "json"
```

**Après :**
```toml
[mqtt]
enabled = true
host = "127.0.0.1"              # ← RMQTT local (était 192.168.1.120)
port = 1883
topic_prefix = "santuario/bms"  # NE PAS CHANGER (utilisé par dbus-mqtt-venus côté NanoPi)
publish_interval_sec = 1
format = "json"
```

### 12.2 Section `[energy_manager.mqtt]` — ligne ~528

**Avant :**
```toml
[energy_manager.mqtt]
host = "192.168.1.141"          # ← Pi5 (loop via interface réseau)
port = 1883
keep_alive_secs = 60
reconnect_delay_secs = 10
```

**Après :**
```toml
[energy_manager.mqtt]
host = "127.0.0.1"              # ← localhost (était 192.168.1.141)
port = 1883
keep_alive_secs = 60
reconnect_delay_secs = 10
```

### 12.3 `portal_id` — à laisser tel quel

```toml
[energy_manager]
portal_id = "c0619ab9929a"      # Ne pas modifier — utilisé par les bridges RMQTT (§8/§9)
```

> **Note** : le guide initial mentionnait une section `[mqtt_bridge]` à supprimer.
> Cette section **n'existe pas** dans le `Config.toml` actuel — rien à faire.

### 12.4 Déployer

```bash
# Commit + push depuis machine de dev
git add Config.toml
git commit -m "chore(config): broker MQTT local 127.0.0.1 (migration RMQTT)"
git push -u origin claude/complete-migration-guide-k1iQS

# Sur le Pi5
make sync
sudo cp Config.toml /etc/daly-bms/config.toml
# (les services sont redémarrés à l'étape 8 du déploiement, §13)
```

---

## 13. Déploiement pas à pas

> **Convention** : les fichiers de configuration RMQTT sont versionnés dans le dépôt
> sous `contrib/rmqtt/` (à créer). Ce répertoire ne contient **pas de code Rust** —
> ce ne doit donc **pas** être un crate du workspace. Structure attendue :
> ```
> contrib/
>   rmqtt-broker.service
>   rmqtt/
>     rmqtt.toml
>     plugins/
>       rmqtt-http-api.toml
>       rmqtt-bridge-ingress-mqtt.toml
>       rmqtt-bridge-egress-mqtt.toml
> ```

### Étape 0 — Pré-flight (machine de dev)

```bash
cd ~/Daly-BMS-Rust
git checkout claude/complete-migration-guide-k1iQS
git tag -a v-pre-rmqtt -m "État stable avant migration RMQTT" 2>/dev/null || true

# Vérifier le portal_id (doit valoir c0619ab9929a)
grep '^portal_id' Config.toml

# Vérifier le binaire rmqttd disponible (sinon §6)
which rmqttd && rmqttd --version
```

### Étape 1 — Préparer le Pi5

```bash
# Sur Pi5
cd ~/Daly-BMS-Rust
make sync

# État actuel
docker ps | grep mosquitto         # doit afficher dalybms-mosquitto running
systemctl is-active daly-bms       # active
systemctl is-active energy-manager # active
```

### Étape 2 — Installer le binaire RMQTT (Pi5)

Voir §6. Vérifier :
```bash
rmqttd --version    # → rmqttd 0.20.0
rmqttd --help | grep -E '^\s*-[fc]'   # confirmer si l'option config est -f ou -c
```

### Étape 3 — Créer utilisateur, répertoires, fichiers de config

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin rmqtt 2>/dev/null || true
sudo mkdir -p /etc/rmqtt/plugins /var/log/rmqtt /var/lib/rmqtt

# Configs (chemins versionnés dans le dépôt, voir encadré ci-dessus)
sudo cp contrib/rmqtt/rmqtt.toml                                /etc/rmqtt/rmqtt.toml
sudo cp contrib/rmqtt/plugins/rmqtt-http-api.toml               /etc/rmqtt/plugins/
sudo cp contrib/rmqtt/plugins/rmqtt-bridge-ingress-mqtt.toml    /etc/rmqtt/plugins/
sudo cp contrib/rmqtt/plugins/rmqtt-bridge-egress-mqtt.toml     /etc/rmqtt/plugins/

sudo chown -R rmqtt:rmqtt /etc/rmqtt /var/log/rmqtt /var/lib/rmqtt
sudo chmod 750 /var/lib/rmqtt /var/log/rmqtt
```

### Étape 4 — Déployer les unités systemd

```bash
sudo cp contrib/rmqtt-broker.service   /etc/systemd/system/
sudo cp contrib/daly-bms.service       /etc/systemd/system/
sudo cp contrib/energy-manager.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable rmqtt-broker
```

### Étape 5 — Mettre à jour `Config.toml` (cf. §12)

```bash
sudo cp Config.toml /etc/daly-bms/config.toml
```
Ne **pas** redémarrer les services maintenant : Docker Mosquitto occupe encore :1883.

### Étape 6 — Arrêter Docker Mosquitto **et** démarrer RMQTT (fenêtre courte)

```bash
# Stopper proprement les clients (ils vont se reconnecter automatiquement)
sudo systemctl stop daly-bms energy-manager

# Stopper Docker Mosquitto (libère :1883 et :9001)
docker compose -f docker-compose.infra.yml down
ss -tlnp | grep -E ':(1883|9001)\b'   # ← doit être vide

# Démarrer RMQTT
sudo systemctl start rmqtt-broker
sleep 3

# Vérification
systemctl status rmqtt-broker --no-pager
journalctl -u rmqtt-broker -n 50 --no-pager
ss -tlnp | grep -E ':(1883|9001|8083)\b'   # 3 lignes attendues
```

### Étape 7 — Vérifier le broker

```bash
# API HTTP (si rmqtt-http-api.toml a bien été déployé, cf. §7.1)
curl -s http://127.0.0.1:8083/api/v1/stats | head

# Test pub/sub local
mosquitto_sub -h 127.0.0.1 -p 1883 -t "test/#" -v -C 1 &
sleep 1
mosquitto_pub -h 127.0.0.1 -p 1883 -t "test/ping" -m "hello"
wait
# Attendu : test/ping hello
```

### Étape 8 — Vérifier les bridges

```bash
# Bridge INGRESS (NanoPi → Pi5) — Venus OS doit arriver localement
timeout 10 mosquitto_sub -h 127.0.0.1 -p 1883 -t "N/c0619ab9929a/#" -v | head

# Bridge EGRESS (Pi5 → NanoPi) — publication test depuis Pi5
mosquitto_pub -h 127.0.0.1 -p 1883 -t "santuario/bms/test" -m '{"probe":1}' -q 1
ssh root@192.168.1.120 \
  "timeout 5 mosquitto_sub -h localhost -t 'santuario/bms/test' -v -C 1"
# Attendu : santuario/bms/test {"probe":1}
```

### Étape 9 — Redémarrer les services métier

```bash
sudo systemctl start daly-bms
sleep 5
journalctl -u daly-bms -n 30 --no-pager | grep -iE 'mqtt|connect'

sudo systemctl start energy-manager
sleep 5
journalctl -u energy-manager -n 30 --no-pager | grep -iE 'mqtt|connect'
```

### Étape 10 — (Optionnel) Recompiler si binaire absent

Les binaires existants restent compatibles (ils lisent simplement `host = "127.0.0.1"`).
Recompiler n'est nécessaire **que** si une modification de code accompagne la migration :

```bash
make build-arm
sudo systemctl stop daly-bms
sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/
sudo systemctl start daly-bms

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
□ Vérifier le portal_id dans Config.toml section [energy_manager] (clé portal_id)
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
