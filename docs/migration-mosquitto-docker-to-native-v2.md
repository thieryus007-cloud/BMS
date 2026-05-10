# Plan de Migration Détaillé — Mosquitto Docker vers Mosquitto Natif

> **Objectif** : Décharger le NanoPi du rôle de hub MQTT central, rationaliser les flux, et réduire l'empreinte RAM/CPU/disk.
>
> **Date** : mai 2026
> **Version Mosquitto cible** : 2.0.x (Debian Bookworm)
> **Hôte** : Raspberry Pi 5 CM (aarch64, Raspberry Pi OS Lite 64-bit)
> **NanoPi** : 192.168.1.120 (Venus OS — inchangé)
> **Portal ID** : c0619ab9929a
>
> **Prérequis** : Copie du dépôt GitHub faite avant migration (rollback possible via `git clone` ou fork).

---

## Table des matières

1. [Architecture actuelle (détaillée)](#1-architecture-actuelle-détaillée)
2. [Problèmes identifiés](#2-problèmes-identifiés)
3. [Architecture cible](#3-architecture-cible)
4. [Tableau complet des flux MQTT](#4-tableau-complet-des-flux-mqtt)
5. [Prérequis et préparation](#5-prérequis-et-préparation)
6. [Sauvegarde de l'état actuel](#6-sauvegarde-de-létat-actuel)
7. [Installation Mosquitto natif](#7-installation-mosquitto-natif)
8. [Configuration mosquitto.conf complète](#8-configuration-mosquitto-conf-complète)
9. [Service systemd mosquitto-broker](#9-service-systemd-mosquitto-broker)
10. [Mise à jour des dépendances systemd](#10-mise-à-jour-des-dépendances-systemd)
11. [Mise à jour Config.toml](#11-mise-à-jour-configtoml)
12. [Déploiement pas à pas](#12-déploiement-pas-à-pas)
13. [Vérification flux par flux](#13-vérification-flux-par-flux)
14. [Anti-boucle — vérification automatique](#14-anti-boucle--vérification-automatique)
15. [Procédure de rollback](#15-procédure-de-rollback)
16. [Nettoyage post-migration](#16-nettoyage-post-migration)
17. [Checklist finale](#17-checklist-finale)

---

## 1. Architecture actuelle (détaillée)

### 1.1 Topologie réseau réelle

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              RÉSEAU 192.168.1.0/24                          │
│                                                                             │
│  ┌─────────────────────────────┐              ┌─────────────────────────────┐ │
│  │   PI5 (192.168.1.141)       │              │  NANOPI (192.168.1.120)     │ │
│  │                             │              │                             │ │
│  │  ┌─────────────────────┐    │   MQTT       │  ┌─────────────────────┐   │ │
│  │  │ daly-bms-server     │────┼──►192.168.1.120│  │                     │   │ │
│  │  │ PUBLIE sur NanoPi   │    │   (TCP)      │  │  mosquitto (Venus)  │   │ │
│  │  └─────────────────────┘    │              │  │  dbus-mqtt-venus    │   │ │
│  │         │                 │              │  │    (Rust/zbus)        │   │ │
│  │         │                 │              │  │         │             │   │ │
│  │  ┌─────────────────────┐  │              │  │    D-Bus Victron    │   │ │
│  │  │ energy-manager      │──┼──►192.168.1.141│  │         │             │   │ │
│  │  │ PUBLIE sur Pi5      │  │   (loopback    │  │    VRM Portal       │   │ │
│  │  │ via IP réseau       │  │   matériel)    │  │                     │   │ │
│  │  └─────────────────────┘  │              │  └─────────────────────┘   │ │
│  │         │                 │              │         ▲                   │ │
│  │         │                 │              │         │                   │ │
│  │  ┌─────────────────────┐  │   Bridge     │    Tasmota/Shelly         │ │
│  │  │ mosquitto DOCKER    │◄─┼──────────────┼──── (WiFi direct)         │ │
│  │  │   :1883  :9001      │  │   Docker     │                           │ │
│  │  │                     │  │   mosquitto  │                           │ │
│  │  │  ┌───────────────┐  │  │   bridge     │                           │ │
│  │  │  │ Bridge vers   │──┼──┼──► NanoPi    │                           │ │
│  │  │  │ 192.168.1.120 │  │  │   (config    │                           │ │
│  │  │  └───────────────┘  │  │   dans       │                           │ │
│  │  │                     │  │   mosquitto  │                           │ │
│  │  │  ┌───────────────┐  │  │   .conf)     │                           │ │
│  │  │  │ Dashboard JS  │◄─┼──┘              │                           │ │
│  │  │  │ WebSocket:9001│  │                 │                           │ │
│  │  │  └───────────────┘  │                 │                           │ │
│  │  └─────────────────────┘  │                 │                           │ │
│  └─────────────────────────────┘                 │                           │ │
│                                                  │                           │ │
└──────────────────────────────────────────────────┼───────────────────────────┘
                                                   │
```

### 1.2 Configuration Config.toml actuelle

```toml
[mqtt]
host = "192.168.1.120"        # ← daly-bms-server publie sur NANOPI
port = 1883

[energy_manager.mqtt]
host = "192.168.1.141"        # ← energy-manager publie sur PI5 via réseau
port = 1883
```

### 1.3 Configuration mosquitto.conf Docker (bridge)

```conf
connection nanopi-venus-bridge
address 192.168.1.120:1883

# INGRESS (NanoPi → Pi5)
topic N/c0619ab9929a/# in 0
topic santuario/# in 0          # ← PROBLÈME : daly-bms-server publie sur NanoPi,
                                #   le bridge les ramène. Double-hop inutile.

# EGRESS (Pi5 → NanoPi)
topic W/c0619ab9929a/# out 1
topic R/c0619ab9929a/# out 1
topic santuario/heat/# out 0    # ← CHEVAUCHEMENT avec santuario/# in
topic santuario/heatpump/# out 0
topic santuario/meteo/# out 0
topic santuario/switch/# out 0
topic santuario/grid/# out 0
topic santuario/platform/# out 0
topic santuario/inverter/# out 0
topic santuario/pvinverter/# out 0

# BIDIRECTIONNEL (DANGER)
topic shellypro2pm-ec62608840a4/# both 0
```

### 1.4 Services systemd actuels

```ini
# contrib/daly-bms.service
[Unit]
After=network.target              # ← AUCUNE dépendance mosquitto

# contrib/energy-manager.service
[Unit]
After=network-online.target mosquitto.service   # ← mosquitto.service INEXISTANT
```

---

## 2. Problèmes identifiés

| # | Problème | Fichier | Impact | Sévérité |
|---|----------|---------|--------|----------|
| 1 | **daly-bms-server publie sur NanoPi** | `Config.toml` `[mqtt].host` | Si NanoPi down, BMS ne publie **nulle part** — pas de données locales | 🔴 Critique |
| 2 | **Double-hop BMS** | `mosquitto.conf` bridge | Pi5 → NanoPi → bridge → Pi5. Latence + fragilité | 🟡 Majeur |
| 3 | **energy-manager via IP réseau** | `Config.toml` `[energy_manager.mqtt]` | Si WiFi down, MQTT local tombe | 🟡 Majeur |
| 4 | **Topics en double direction** | `mosquitto.conf` | `santuario/heat/#` en IN + OUT = risque boucle | 🟡 Majeur |
| 5 | **`shellypro2pm... both 0`** | `mosquitto.conf` | Bidirectionnel = boucle possible | 🟡 Majeur |
| 6 | **Dépendance `mosquitto.service` inexistante** | `energy-manager.service` | systemd ignore silencieusement | 🟢 Mineur |
| 7 | **Overhead Docker** | `docker-compose.infra.yml` | ~50-100 Mo RAM, ~10s démarrage | 🟡 Majeur |

---

## 3. Architecture cible

### 3.1 Principe directeur

> **Tout ce qui est produit sur le Pi5 est publié sur le broker LOCAL (127.0.0.1).**
> **Le bridge ne sert qu'à échanger avec le NanoPi.**
> **Le NanoPi reste le point d'entrée pour Tasmota/Shelly (WiFi).**

### 3.2 Topologie cible

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              PI5 (192.168.1.141)                            │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────────┐│
│  │                     mosquitto (NATIF)                                   ││
│  │  ┌─────────────────────────────────────────────────────────────────┐   ││
│  │  │  Services locaux : daly-bms-server, energy-manager, dashboard   │   ││
│  │  │  Tous connectés en 127.0.0.1:1883 (loopback)                    │   ││
│  │  └─────────────────────────────────────────────────────────────────┘   ││
│  │                              │                                         ││
│  │  ┌───────────────────────────┼─────────────────────────────────────┐   ││
│  │  │  Bridge EGRESS            │  → 192.168.1.120 (NanoPi)         │   ││
│  │  │  santuario/bms/#          │  santuario/pvinverter/#           │   ││
│  │  │  santuario/switch/#       │  santuario/heatpump/#             │   ││
│  │  │  santuario/irradiance/raw │  santuario/meteo/venus            │   ││
│  │  │  santuario/heat/+/venus   │  santuario/system/venus             │   ││
│  │  │  santuario/platform/venus │  W/{portal}/#                     │   ││
│  │  │  cmnd/#                   │  shellypro2pm-ec62608840a4/rpc     │   ││
│  │  └───────────────────────────┼─────────────────────────────────────┘   ││
│  │                              │                                         ││
│  │  ┌───────────────────────────┼─────────────────────────────────────┐   ││
│  │  │  Bridge INGRESS           │  ← 192.168.1.120 (NanoPi)          │   ││
│  │  │  N/{portal}/#           │  tele/#                             │   ││
│  │  │  stat/#                 │  shellypro2pm-ec62608840a4/#       │   ││
│  │  │  daly-bms-shelly/rpc    │                                     │   ││
│  │  └───────────────────────────┼─────────────────────────────────────┘   ││
│  └──────────────────────────────┼─────────────────────────────────────────┘│
│                                 │                                           │
│  ┌──────────────────────┐   127.0.0.1:1883   ┌──────────────────────┐      │
│  │ daly-bms-server      │◄──────────────────►│ PUBLIE/ABONNE local   │      │
│  │ RS485 → MQTT         │    loopback        │                       │      │
│  └──────────────────────┘                    └──────────────────────┘      │
│           ▲                                                                  │
│           │                                                                  │
│  ┌──────────────────────┐   127.0.0.1:1883   ┌──────────────────────┐      │
│  │ energy-manager       │◄──────────────────►│ PUBLIE/ABONNE local   │      │
│  │ API cloud + règles   │    loopback        │                       │      │
│  └──────────────────────┘                    └──────────────────────┘      │
│           │                                                                  │
│           ▼                                                                  │
│  Dashboard JS (:9001 WebSocket)                                            │
│  VictoriaMetrics (:8428)                                                     │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
                              │
                              │ MQTT bridge (uniquement)
                              ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           NANOPI (192.168.1.120)                            │
│                                                                             │
│  ┌──────────────────┐     ┌──────────────────┐     ┌─────────────────────┐  │
│  │ mosquitto          │◄────│ dbus-mqtt-venus  │────►│ D-Bus Victron       │  │
│  │ (reçoit bridge)    │     │ (souscrit local) │     │ systemcalc-py       │  │
│  │ (Tasmota/Shelly    │     │                  │     │       │             │  │
│  │  toujours ici)     │     │                  │     │   VRM Portal        │  │
│  └──────────────────┘     └──────────────────┘     └─────────────────────┘  │
│         ▲                                                                   │
│         │                                                                   │
│    Tasmota (WiFi)  Shelly (WiFi)                                           │
│    192.168.1.115   192.168.1.136                                           │
│                                                                             │
│  ⚠️ Tasmota/Shelly restent sur NanoPi (config WiFi fixe)                   │
│     → Migration possible si reconfiguration des devices                    │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 4. Tableau complet des flux MQTT

### 4.1 Flux EGRESS (Pi5 → NanoPi)

| Source | Topic | QoS | Retain | Producteur | Consommateur NanoPi |
|--------|-------|-----|--------|------------|---------------------|
| BMS 0x01 | `santuario/bms/1/venus` | 1 | true | daly-bms-server | dbus-mqtt-venus → D-Bus battery.mqtt_1 |
| BMS 0x02 | `santuario/bms/2/venus` | 1 | true | daly-bms-server | dbus-mqtt-venus → D-Bus battery.mqtt_2 |
| ET112 0x07 | `santuario/pvinverter/7/venus` | 0 | true | daly-bms-server | dbus-mqtt-venus → D-Bus pvinverter.mqtt_7 |
| ET112 0x08 | `santuario/heatpump/8/venus` | 0 | true | daly-bms-server | dbus-mqtt-venus → D-Bus heatpump.mqtt_8 |
| ET112 0x09 | `santuario/heatpump/9/venus` | 0 | true | daly-bms-server | dbus-mqtt-venus → D-Bus heatpump.mqtt_9 |
| ATS CHINT | `santuario/switch/1/venus` | 0 | true | daly-bms-server | dbus-mqtt-venus → D-Bus switch.mqtt_1 |
| Irradiance | `santuario/irradiance/raw` | 1 | true | daly-bms-server | energy-manager (local) + ancien Node-RED |
| Météo | `santuario/meteo/venus` | 0 | true | energy-manager | dbus-mqtt-venus → D-Bus meteo |
| Température | `santuario/heat/1/venus` | 0 | true | energy-manager | dbus-mqtt-venus → D-Bus temperature.mqtt_1 |
| Chauffe-eau | `santuario/heatpump/1/venus` | 0 | true | energy-manager | dbus-mqtt-venus → D-Bus heatpump.mqtt_1 |
| SmartShunt | `santuario/system/venus` | 0 | true | energy-manager | daly-bms-server (subscriber local) |
| Platform | `santuario/platform/venus` | 0 | true | energy-manager | dbus-mqtt-venus → D-Bus platform |
| Commandes VEBus | `W/c0619ab9929a/#` | 1 | false | energy-manager | dbus-mqtt-venus → D-Bus VEBus |
| Keepalive | `R/c0619ab9929a/#` | 1 | false | energy-manager | dbus-mqtt-venus → D-Bus lecture |
| Commandes Tongou | `cmnd/tongou_*/POWER` | 1 | false | energy-manager | Tasmota (sur NanoPi) |
| RPC Shelly | `shellypro2pm-ec62608840a4/rpc` | 0 | false | energy-manager | Shelly (sur NanoPi) |

### 4.2 Flux INGRESS (NanoPi → Pi5)

| Source | Topic | QoS | Retain | Producteur NanoPi | Consommateur Pi5 |
|--------|-------|-----|--------|-------------------|------------------|
| Venus OS | `N/c0619ab9929a/#` | 0 | false | dbus-mqtt-venus (publish) | energy-manager |
| Tongou mesures | `tele/tongou_*/SENSOR` | 0 | false | Tasmota | daly-bms-server + energy-manager |
| Tongou état | `stat/tongou_*/POWER` | 0 | false | Tasmota | energy-manager |
| Shelly status | `shellypro2pm-ec62608840a4/#` | 0 | false | Shelly | energy-manager |
| Shelly RPC réponse | `daly-bms-shelly/rpc` | 0 | false | Shelly | energy-manager |

### 4.3 Flux LOCAL uniquement (pas de bridge)

| Source | Topic | Destination | Utilisation |
|--------|-------|-------------|-------------|
| energy-manager | `santuario/persist/pvinv_baseline` | daly-bms-server | Baselines PV |
| energy-manager | `santuario/persist/yield_yesterday` | daly-bms-server | Historique |
| energy-manager | `santuario/persist/deye_state` | daly-bms-server | État relais DEYE |

---

## 5. Prérequis et préparation

### 5.1 Sur la machine de développement

```bash
cd ~/Daly-BMS-Rust
git checkout main
git pull

# Vérifier l'état du dépôt
git status

# Créer un tag de l'état stable AVANT migration
git tag -a v-pre-mosquitto-native-$(date +%Y%m%d) -m "État stable avant migration Mosquitto natif"
git push origin v-pre-mosquitto-native-$(date +%Y%m%d)

# Vérifier le portal_id
grep '^portal_id' Config.toml
# → doit afficher : portal_id = "c0619ab9929a"

# Vérifier les adresses BMS
grep -A5 '^\[\[bms\]\]' Config.toml | head -20
```

### 5.2 Sur le Pi5 (vérifications préalables)

```bash
ssh pi5compute@192.168.1.141

# Vérifier l'état actuel
docker ps | grep mosquitto         # doit afficher dalybms-mosquitto running
systemctl is-active daly-bms       # active
systemctl is-active energy-manager # active

# Vérifier que Mosquitto natif n'est PAS déjà installé
dpkg -l | grep mosquitto           # doit être vide ou désinstallé
which mosquitto                    # doit être vide

# Vérifier l'espace disque
df -h /
# S'assurer d'avoir au moins 500 Mo libres

# Vérifier la connectivité NanoPi
ping -c 3 192.168.1.120
ssh root@192.168.1.120 "systemctl is-active mosquitto"

# Vérifier les ports utilisés
ss -tlnp | grep -E ':(1883|9001)'
```

### 5.3 Inventaire des topics (à ne PAS modifier)

**EGRESS (Pi5 → NanoPi)** — Ce que le Pi5 produit :
```
santuario/bms/#
santuario/pvinverter/#
santuario/heatpump/#
santuario/switch/#
santuario/irradiance/raw
santuario/meteo/venus
santuario/heat/+/venus
santuario/system/venus
santuario/platform/venus
W/c0619ab9929a/#
R/c0619ab9929a/#
cmnd/#
shellypro2pm-ec62608840a4/rpc
```

**INGRESS (NanoPi → Pi5)** — Ce que le NanoPi produit :
```
N/c0619ab9929a/#
tele/#
stat/#
shellypro2pm-ec62608840a4/#
daly-bms-shelly/rpc
```

---

## 6. Sauvegarde de l'état actuel

### 6.1 Sauvegarder la config Docker actuelle

```bash
cd ~/Daly-BMS-Rust

# Copier la config actuelle en backup timestampé
cp docker/mosquitto/config/mosquitto.conf \
   docker/mosquitto/config/mosquitto.conf.bak.$(date +%Y%m%d_%H%M%S)

# Sauvegarder les données retained (messages persistés)
docker exec dalybms-mosquitto cat /mosquitto/data/mosquitto.db \
   > /tmp/mosquitto-retained-backup.db 2>/dev/null || true

# Noter les variables d'environnement Docker
cat .env | grep -i mosquitto > /tmp/mosquitto-env-backup.txt 2>/dev/null || true

# Sauvegarder la config TOML actuelle
cp Config.toml Config.toml.bak.$(date +%Y%m%d_%H%M%S)
```

### 6.2 Créer le répertoire de contribution pour Mosquitto natif  ✅ OK Fait

```bash
mkdir -p contrib/mosquitto

# Copier la config actuelle comme référence
cp docker/mosquitto/config/mosquitto.conf contrib/mosquitto/mosquitto.conf.reference

# Créer le script de vérification anti-boucle
cat > contrib/mosquitto/verify-no-loop.sh << 'SCRIPT'
#!/bin/bash
# verify-no-loop.sh
# Vérifie qu'aucun topic n'est présent à la fois en IN et en OUT

CONFIG="/etc/mosquitto/mosquitto.conf"

if [ ! -f "$CONFIG" ]; then
    echo "ERREUR : $CONFIG introuvable"
    exit 1
fi

echo "=== Topics EGRESS (out) ==="
OUT_TOPICS=$(grep -E '^\s*topic\s+\S+\s+out\s' "$CONFIG" | awk '{print $2}' | sort -u)
echo "$OUT_TOPICS"

echo ""
echo "=== Topics INGRESS (in) ==="
IN_TOPICS=$(grep -E '^\s*topic\s+\S+\s+in\s' "$CONFIG" | awk '{print $2}' | sort -u)
echo "$IN_TOPICS"

echo ""
echo "=== INTERSECTION (DANGER — topics en double) ==="
INTERSECTION=$(comm -12 <(echo "$OUT_TOPICS") <(echo "$IN_TOPICS"))

if [ -n "$INTERSECTION" ]; then
    echo "❌ ERREUR FATALE : Topics présents dans les deux directions :"
    echo "$INTERSECTION"
    echo ""
    echo "Cela créera une BOUCLE INFINIE. Corriger mosquitto.conf immédiatement."
    exit 1
else
    echo "✅ OK : Aucun topic en double. Pas de risque de boucle."
    exit 0
fi
SCRIPT

chmod +x contrib/mosquitto/verify-no-loop.sh
```

---

## 7. Installation Mosquitto natif

### 7.1 Supprimer Docker Mosquitto (PAS encore)

> **IMPORTANT** : Ne PAS supprimer Docker Mosquitto maintenant. Le broker doit rester actif jusqu'à ce que le natif soit prêt.

### 7.2 Installer les packages Debian

```bash
sudo apt update
sudo apt install -y mosquitto mosquitto-clients

# Vérifier l'installation
mosquitto -h | head -n 3
# → doit afficher la version (ex: mosquitto version 2.0.11)

# Empêcher le démarrage automatique immédiat
sudo systemctl stop mosquitto
sudo systemctl disable mosquitto
# (on utilisera notre propre service systemd plus restrictif)
```

### 7.3 Créer les répertoires de données

```bash
# Répertoires requis
sudo mkdir -p /var/lib/mosquitto
sudo mkdir -p /var/log/mosquitto

# Permissions (Mosquitto s'exécute sous l'utilisateur mosquitto)
sudo chown mosquitto:mosquitto /var/lib/mosquitto
sudo chown mosquitto:mosquitto /var/log/mosquitto
sudo chmod 750 /var/lib/mosquitto
sudo chmod 755 /var/log/mosquitto
```

---

## 8. Configuration mosquitto.conf complète ✅ OK Fait

Créer `/etc/mosquitto/mosquitto.conf` :

```conf
# =============================================================================
# mosquitto.conf — Broker natif Pi5 (remplace Docker Mosquitto)
# Version : 2.0.x (Debian Bookworm)
# Date : mai 2026
# =============================================================================

# --- Général ---
# Identifiant du broker (utile pour le debugging)
broker_id pi5-mosquitto

# --- Listener MQTT TCP :1883 ---
listener 1883 0.0.0.0
# Pas d'authentification (LAN privé de confiance uniquement)
allow_anonymous true
# Taille max des messages
max_packet_size 1048576
# Connexions simultanées
max_connections 500
# QoS 2 messages en attente max
max_inflight_messages 40
max_queued_messages 1000

# --- Listener WebSocket :9001 ---
listener 9001 0.0.0.0
protocol websockets
allow_anonymous true

# --- Logs ---
# Destination : syslog (journalctl)
log_dest syslog
# Types de logs
log_type error
log_type warning
log_type information
log_type subscribe
log_type unsubscribe
# Messages de connexion/déconnexion
connection_messages true

# --- Persistence (retained messages survivent au redémarrage) ---
persistence true
persistence_location /var/lib/mosquitto/
# Sauvegarde automatique toutes les 5 minutes
autosave_interval 300
# Fichier de persistence
persistence_file mosquitto.db

# --- Performance ---
# Intervalle de nettoyage des sessions expirées
persistent_client_expiration 1d
# Intervalle entre les checks de keepalive
retry_interval 20
# Timeout des messages QoS 1/2 non acquittés
message_retry_timeout 20

# =============================================================================
# BRIDGE EGRESS : Pi5 → NanoPi (192.168.1.120:1883)
#
# ⚠ RÈGLE ABSOLUE : Ces topics ne doivent PAS être dans le bridge INGRESS.
#   Tous les topics santuario/* viennent de Pi5, JAMAIS de NanoPi.
# =============================================================================
connection pi5-to-nanopi
address 192.168.1.120:1883
bridge_protocol_version mqttv311
# Démarrage automatique
start_type automatic
# Notification de l'état du bridge
notifications true
notification_topic $SYS/broker/bridge/pi5-to-nanopi/state
# Essayer de signaler au broker distant qu'on est un bridge
# (évite les boucles si le distant est aussi un bridge)
try_private true

# --- Topics EGRESS ---
# Format : topic <pattern> <direction> <qos> <local-prefix> <remote-prefix>

# BMS → battery.mqtt_1 / battery.mqtt_2
topic santuario/bms/# out 1 "" ""

# ET112 Micro-Onduleurs → pvinverter.mqtt_7
topic santuario/pvinverter/# out 0 "" ""

# ET112 Maison + Réseau → heatpump.mqtt_8/9
topic santuario/heatpump/# out 0 "" ""

# ATS CHINT + Tongou → switch.mqtt_1/2/3/4/5/6
topic santuario/switch/# out 0 "" ""

# Irradiance PRALRAN → energy-manager (local) + ancien Node-RED
topic santuario/irradiance/raw out 1 "" ""

# Température extérieure → temperature.mqtt_1
topic santuario/heat/+/venus out 0 "" ""

# Météo/irradiance → meteo
topic santuario/meteo/venus out 0 "" ""

# SmartShunt calculé → daly-bms-server (subscriber local)
topic santuario/system/venus out 0 "" ""

# Platform Pi5 → platform
topic santuario/platform/venus out 0 "" ""

# Venus OS commandes (écriture D-Bus) — QoS 1
topic W/c0619ab9929a/# out 1 "" ""

# Venus OS keepalive (lecture D-Bus) — QoS 1
topic R/c0619ab9929a/# out 1 "" ""

# Commandes ON/OFF Tongou depuis energy-manager
topic cmnd/# out 1 "" ""

# Commandes RPC Shelly (energy-manager → Shelly via NanoPi)
topic shellypro2pm-ec62608840a4/rpc out 0 "" ""

# --- Options bridge EGRESS ---
# Ne pas tenter de se désabonner à la déconnexion
bridge_attempt_unsubscribe false
# Reconnexion automatique
restart_timeout 10 30
# Keepalive
keepalive_interval 60

# =============================================================================
# BRIDGE INGRESS : NanoPi → Pi5 (192.168.1.120:1883)
#
# ⚠ RÈGLE ABSOLUE : Ces topics doivent être ABSENTS du bridge EGRESS.
#   NanoPi publie : N/{portal}/#, tele/#, stat/#, shellypro2pm/#, daly-bms-shelly/rpc
#   Pi5 publie   : santuario/*, cmnd/*, W/*, R/*, shellypro2pm.../rpc
# =============================================================================
connection nanopi-to-pi5
address 192.168.1.120:1883
bridge_protocol_version mqttv311
start_type automatic
notifications true
notification_topic $SYS/broker/bridge/nanopi-to-pi5/state
try_private true

# --- Topics INGRESS ---

# Venus OS télémétriques (Cerbo GX)
topic N/c0619ab9929a/# in 0 "" ""

# Tasmota / Tongou — mesures énergie
topic tele/# in 0 "" ""

# Tasmota / Tongou — état relais
topic stat/# in 0 "" ""

# Shelly Pro 2PM — status + events
topic shellypro2pm-ec62608840a4/# in 0 "" ""

# Réponses RPC Shelly
topic daly-bms-shelly/rpc in 0 "" ""

# --- Options bridge INGRESS ---
bridge_attempt_unsubscribe false
restart_timeout 10 30
keepalive_interval 60

# =============================================================================
# SÉCURITÉ (LAN privé — ajustez si exposition publique)
# =============================================================================
# Pas de TLS en interne (performances)
# Si exposition externe, décommenter et configurer :
# listener 8883
# cafile /etc/mosquitto/ca_certificates/ca.crt
# certfile /etc/mosquitto/certs/server.crt
# keyfile /etc/mosquitto/certs/server.key
# require_certificate false
# tls_version tlsv1.2
```

> **Note** : Remplacer `c0619ab9929a` par le `portal_id` réel. Vérifier avec :
> ```bash
> grep '^portal_id' /etc/daly-bms/config.toml
> # ou côté NanoPi :
> ssh root@192.168.1.120 "mosquitto_sub -h localhost -t 'N/+/system/0/Serial' -C 1 -W 5"
> ```

---

## 9. Service systemd mosquitto-broker ✅ OK Fait 

Créer `contrib/mosquitto-broker.service` :

```ini
[Unit]
Description=Mosquitto MQTT Broker (natif, remplace Docker)
Documentation=https://mosquitto.org/documentation/
After=network-online.target
Wants=network-online.target
Before=daly-bms.service energy-manager.service

[Service]
Type=notify
ExecStart=/usr/sbin/mosquitto -c /etc/mosquitto/mosquitto.conf
ExecReload=/bin/kill -HUP $MAINPID

# Redémarrage automatique en cas d'échec
Restart=on-failure
RestartSec=5s

# Utilisateur mosquitto (créé par le package Debian)
User=mosquitto
Group=mosquitto

# Ressources (Mosquitto est très léger)
MemoryMax=50M
LimitNOFILE=65536

# Sécurité — assez permissif pour un broker réseau
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
# Chemins en lecture/écriture nécessaires
ReadWritePaths=/var/lib/mosquitto /var/log/mosquitto

# Journal
StandardOutput=journal
StandardError=journal
SyslogIdentifier=mosquitto-broker

[Install]
WantedBy=multi-user.target
```

> **Pourquoi un service personnalisé ?** Le service `mosquitto.service` installé par Debian est générique. Notre version :
> - Démarre **avant** `daly-bms` et `energy-manager` (`Before=`)
> - Limite la mémoire à 50 Mo (marge confortable)
> - Utilise `ProtectSystem=strict` (Mosquitto est audité depuis des années)

---

## 10. Mise à jour des dépendances systemd

### 10.1 Patch `contrib/daly-bms.service`

Modifier l'en-tête pour dépendre du nouveau broker natif :

```ini
[Unit]
Description=DalyBMS Server — Rust RS485 BMS monitor
Documentation=https://github.com/thieryus007-cloud/Daly-BMS-Rust
After=network.target mosquitto-broker.service
Wants=network.target
Requires=mosquitto-broker.service
```

### 10.2 Patch `contrib/energy-manager.service`

Remplacer la référence à `mosquitto.service` (inexistant) par `mosquitto-broker.service` :

```ini
[Unit]
Description=Energy Manager — Gestionnaire d'énergie Rust (remplace Node-RED)
Documentation=https://github.com/thieryus007-cloud/Daly-BMS-Rust
After=network-online.target mosquitto-broker.service daly-bms.service
Wants=network-online.target
Requires=mosquitto-broker.service
PartOf=daly-bms.service
```

### 10.3 Déploiement des unités

```bash
sudo cp contrib/mosquitto-broker.service /etc/systemd/system/
sudo cp contrib/daly-bms.service         /etc/systemd/system/
sudo cp contrib/energy-manager.service   /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable mosquitto-broker
```

---

## 11. Mise à jour Config.toml Pi5

### 11.1 Section `[mqtt]` (daly-bms-server) — ligne ~74

**Avant :**
```toml
[mqtt]
enabled = true
host = "192.168.1.120"          # ← NanoPi (via réseau)
port = 1883
topic_prefix = "santuario/bms"
publish_interval_sec = 1
format = "json"
```

**Après :**
```toml
[mqtt]
enabled = true
host = "127.0.0.1"              # ← Mosquitto natif local (était 192.168.1.120)
port = 1883
topic_prefix = "santuario/bms"  # NE PAS CHANGER (utilisé par dbus-mqtt-venus côté NanoPi)
publish_interval_sec = 1
format = "json"
```

### 11.2 Section `[energy_manager.mqtt]` — ligne ~528

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

### 11.3 `portal_id` — à laisser tel quel

```toml
[energy_manager]
portal_id = "c0619ab9929a"      # Ne pas modifier — utilisé par les bridges
```

### 11.4 Déployer

```bash
# Commit + push depuis machine de dev
git add Config.toml contrib/mosquitto-broker.service contrib/daly-bms.service contrib/energy-manager.service contrib/mosquitto/
git commit -m "chore(config): migration Mosquitto natif + flux rationalisés 127.0.0.1"
git push

# Sur le Pi5
cd ~/Daly-BMS-Rust && make sync
sudo cp Config.toml /etc/daly-bms/config.toml
```

---

## 12. Déploiement pas à pas

> **Convention** : Les fichiers de configuration Mosquitto natif sont versionnés dans le dépôt sous `contrib/mosquitto/`.

### Étape 0 — Pré-flight (Pi5)

```bash
ssh pi5compute@192.168.1.141

cd ~/Daly-BMS-Rust
make sync

# État actuel
docker ps | grep mosquitto         # doit afficher dalybms-mosquitto running
systemctl is-active daly-bms       # active
systemctl is-active energy-manager # active

# Vérifier le binaire mosquitto
which mosquitto && mosquitto -h | head -n 1
```

### Étape 1 — Installer Mosquitto natif

```bash
sudo apt update
sudo apt install -y mosquitto mosquitto-clients

# Vérifier
mosquitto -h | head -n 3

# Stopper le service Debian par défaut
sudo systemctl stop mosquitto
sudo systemctl disable mosquitto
```

### Étape 2 — Créer répertoires et déployer la config

```bash
sudo mkdir -p /var/lib/mosquitto /var/log/mosquitto
sudo chown mosquitto:mosquitto /var/lib/mosquitto /var/log/mosquitto
sudo chmod 750 /var/lib/mosquitto

# Déployer la configuration
sudo cp contrib/mosquitto/mosquitto.conf /etc/mosquitto/mosquitto.conf

# Vérifier la syntaxe
sudo -u mosquitto mosquitto -c /etc/mosquitto/mosquitto.conf -t
# → doit afficher "Configuration file syntax OK" et quitter
```

### Étape 3 — Déployer les unités systemd

```bash
sudo cp contrib/mosquitto-broker.service /etc/systemd/system/
sudo cp contrib/daly-bms.service         /etc/systemd/system/
sudo cp contrib/energy-manager.service   /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable mosquitto-broker
```

### Étape 4 — Mettre à jour Config.toml (cf. §11)

```bash
sudo cp Config.toml /etc/daly-bms/config.toml
```

> Ne PAS redémarrer les services maintenant : Docker Mosquitto occupe encore :1883.

### Étape 5 — Arrêter Docker Mosquitto et démarrer le natif (fenêtre courte)

```bash
# Stopper proprement les clients (ils vont se reconnecter automatiquement)
sudo systemctl stop daly-bms energy-manager

# Stopper Docker Mosquitto (libère :1883 et :9001)
docker compose -f docker-compose.infra.yml down

# Vérifier que les ports sont libres
ss -tlnp | grep -E ':(1883|9001)\b'
# ← doit être VIDE

# Démarrer Mosquitto natif
sudo systemctl start mosquitto-broker
sleep 3

# Vérification
systemctl status mosquitto-broker --no-pager
journalctl -u mosquitto-broker -n 30 --no-pager
ss -tlnp | grep -E ':(1883|9001)\b'   # 2 lignes attendues
```

### Étape 6 — Vérifier le broker local

```bash
# Test pub/sub local
mosquitto_sub -h 127.0.0.1 -p 1883 -t "test/#" -v -C 1 &
sleep 1
mosquitto_pub -h 127.0.0.1 -p 1883 -t "test/ping" -m "hello"
wait
# Attendu : test/ping hello

# Vérifier les bridges
journalctl -u mosquitto-broker | grep -i "bridge"
# → doit afficher "Connecting bridge pi5-to-nanopi..." et "Connecting bridge nanopi-to-pi5..."

# Vérifier l'état des bridges via topics système
timeout 5 mosquitto_sub -h 127.0.0.1 -t '$SYS/broker/bridge/+/state' -v -C 2
# → Attendu : $SYS/broker/bridge/pi5-to-nanopi/state 1
#             $SYS/broker/bridge/nanopi-to-pi5/state 1
```

### Étape 7 — Vérifier les bridges avec NanoPi

```bash
# Bridge INGRESS (NanoPi → Pi5) — Venus OS doit arriver localement
timeout 10 mosquitto_sub -h 127.0.0.1 -p 1883 -t "N/c0619ab9929a/#" -v | head

# Bridge EGRESS (Pi5 → NanoPi) — publication test depuis Pi5
mosquitto_pub -h 127.0.0.1 -p 1883 -t "santuario/bms/test" -m '{"probe":1}' -q 1
ssh root@192.168.1.120 \
  "timeout 5 mosquitto_sub -h localhost -t 'santuario/bms/test' -v -C 1"
# Attendu : santuario/bms/test {"probe":1}
```

### Étape 8 — Vérifier l'anti-boucle

```bash
# Exécuter le script de vérification
sudo /usr/local/bin/verify-no-loop.sh
# → Attendu : ✅ OK

# Compter les messages santuario/bms/# sur Pi5 pendant 10s
timeout 10 mosquitto_sub -h localhost -p 1883 -t "santuario/bms/#" -v | wc -l
# Attendu : ~4 messages (2 BMS × 2 updates en 10s)
# Si > 50 messages → boucle bridge → vérifier la config
```

### Étape 9 — Redémarrer les services métier

```bash
sudo systemctl start daly-bms
sleep 5
journalctl -u daly-bms -n 30 --no-pager | grep -iE 'mqtt|connect|error'

sudo systemctl start energy-manager
sleep 5
journalctl -u energy-manager -n 30 --no-pager | grep -iE 'mqtt|connect|error'
```

### Étape 10 — Vérifier la réduction d'empreinte

```bash
# Comparer la consommation RAM
free -h

# Vérifier que Docker Mosquitto est bien arrêté
docker ps | grep mosquitto   # doit être VIDE

# Vérifier la consommation du process natif
ps aux | grep mosquitto | grep -v grep
# → RSS doit être ~3-5 Mo
```

---

## 13. Vérification flux par flux

### 13.1 Checklist VRM (https://vrm.victronenergy.com)

```
□ Battery Monitor [151] "just now"    ← BMS 360Ah (santuario/bms/1/venus)
□ Battery Monitor [152] "just now"    ← BMS 320Ah (santuario/bms/2/venus)
□ ET112-Micro-Onduleurs "just now"    ← pvinverter.mqtt_7 (santuario/pvinverter/7/venus)
□ PAC Chauffe-eau "just now"          ← heatpump.mqtt_8 (santuario/heatpump/8/venus)
□ PAC Climatisation "just now"        ← heatpump.mqtt_9 (santuario/heatpump/9/venus)
□ ATS CHINT "just now"                ← switch.mqtt_1 (santuario/switch/1/venus)
□ Tongou 1-5 "just now"               ← switch.mqtt_2/3/4/5/6 (santuario/switch/{n}/venus)
□ Capteur météo actif                 ← meteo (santuario/meteo/venus)
□ Température extérieure              ← temperature.mqtt_1 (santuario/heat/1/venus)
□ Platform Pi5 visible                ← platform (santuario/platform/venus)
```

### 13.2 Checklist Pi5 Dashboard

```
□ Page /dashboard/visualization — tous les nodes actifs (pas de "Hors ligne")
□ Page /dashboard/tasmota — Tongou 1-5 : puissance, tension, courant affichés
□ Page /dashboard/tasmota — commande ON/OFF fonctionne depuis energy-manager
□ Page /dashboard/shelly — Shelly Pro 2PM : données affichées
□ Page /dashboard/bms/1 et /2 — données BMS actualisées
□ Explorateur MQTT WebSocket :9001 — PAS de flood (< 5 msg/s en veille)
□ VictoriaMetrics — données BMS ET112 présentes
```

### 13.3 Vérification flux spécifiques

```bash
# 1. BMS → VRM (le plus critique)
timeout 10 mosquitto_sub -h 127.0.0.1 -t "santuario/bms/1/venus" -v | head -n 3
# → Doit afficher des payloads JSON avec Soc, Voltage, Current

# 2. ET112 → VRM
timeout 5 mosquitto_sub -h 127.0.0.1 -t "santuario/pvinverter/7/venus" -v | head -n 1

# 3. Commande Tongou depuis energy-manager
# Publier manuellement comme le ferait energy-manager :
mosquitto_pub -h 127.0.0.1 -t "cmnd/tongou_3BC764/POWER" -m "ON" -q 1
# Vérifier sur le dashboard que le relais s'est activé

# 4. Réception données Venus OS (NanoPi → Pi5)
timeout 5 mosquitto_sub -h 127.0.0.1 -t "N/c0619ab9929a/system/0/Dc/Battery/Soc" -v | head -n 1

# 5. Réception Tasmota (NanoPi → Pi5)
timeout 5 mosquitto_sub -h 127.0.0.1 -t "tele/tongou_3BC764/SENSOR" -v | head -n 1

# 6. Réception Shelly (NanoPi → Pi5)
timeout 5 mosquitto_sub -h 127.0.0.1 -t "shellypro2pm-ec62608840a4/status/switch:0" -v | head -n 1
```

### 13.4 Logs à surveiller

```bash
# Broker Mosquitto natif
journalctl -u mosquitto-broker -f

# BMS server
journalctl -u daly-bms -f | grep -E "MQTT|bms|error"

# Energy manager
journalctl -u energy-manager -f | grep -E "MQTT|connect|error"

# NanoPi dbus-mqtt-venus
ssh root@192.168.1.120 "tail -f /var/log/dbus-mqtt-venus/current"
```

---

## 14. Anti-boucle — vérification automatique

### 14.1 Script de vérification

Le script `contrib/mosquitto/verify-no-loop.sh` est déployé en §6.2. Utilisation :

```bash
# Vérifier après chaque modification de mosquitto.conf
sudo /usr/local/bin/verify-no-loop.sh

# Résultat attendu :
# === Topics EGRESS (out) ===
# santuario/bms/#
# santuario/pvinverter/#
# ...
# === Topics INGRESS (in) ===
# N/c0619ab9929a/#
# tele/#
# ...
# === INTERSECTION (DANGER — topics en double) ===
# ✅ OK : Aucun topic en double. Pas de risque de boucle.
```

### 14.2 Vérification manuelle complémentaire

```bash
# Compter les messages par seconde sur un topic santuario
# En veille, doit être < 5 msg/s (2 BMS × 1 msg/s + retained)
timeout 10 mosquitto_sub -h 127.0.0.1 -t "santuario/#" -v | wc -l
# Attendu : ~10-20 messages en 10s (selon nombre de capteurs)
# Si > 100 → possible boucle

# Vérifier que les bridges n'ont pas de messages en boucle
journalctl -u mosquitto-broker | grep -i "loop\|duplicate\|dropped"
# Doit être vide
```

---

## 15. Procédure de rollback

En cas d'échec, retour à Mosquitto Docker en **< 5 minutes** :

```bash
# 1. Arrêter Mosquitto natif
sudo systemctl stop mosquitto-broker
sudo systemctl disable mosquitto-broker

# 2. Restaurer Config.toml avec les anciens hôtes
sudo sed -i 's/host = "127.0.0.1"/host = "192.168.1.120"/' /etc/daly-bms/config.toml
sudo sed -i 's/host = "127.0.0.1"/host = "192.168.1.141"/' /etc/daly-bms/config.toml

# 3. Relancer Docker Mosquitto
cd ~/Daly-BMS-Rust
docker compose -f docker-compose.infra.yml up -d

# 4. Vérifier Mosquitto Docker actif
docker ps | grep mosquitto
ss -tlnp | grep 1883

# 5. Redémarrer les services
sudo systemctl restart daly-bms energy-manager

# 6. Vérifier
systemctl status daly-bms energy-manager
```

---

## 16. Nettoyage post-migration

> **À effectuer UNIQUEMENT après 24h de stabilité confirmée.**

### 16.1 Retirer les fichiers Docker Mosquitto

```bash
cd ~/Daly-BMS-Rust

# Archiver (pas supprimer immédiatement)
git mv docker-compose.infra.yml docker-compose.infra.yml.bak.$(date +%Y%m%d)
git mv docker/mosquitto docker/mosquitto.bak.$(date +%Y%m%d)

# Commit
git add -A
git commit -m "chore(infra): archive Docker Mosquitto (migration natif stable)"
```

### 16.2 Mettre à jour le Makefile

Les targets `make up` / `make down` / `make logs` / `make ps` deviennent obsolètes. Remplacer par des wrappers `systemctl` :

```makefile
# Infra MQTT (Mosquitto natif)
up:
	sudo systemctl start mosquitto-broker

down:
	sudo systemctl stop mosquitto-broker

logs:
	journalctl -u mosquitto-broker -f

restart:
	sudo systemctl restart mosquitto-broker

ps:
	systemctl status mosquitto-broker --no-pager
```

### 16.3 Mettre à jour `CLAUDE.md`

Sections à modifier :
- Section 1 (Architecture) : remplacer `Docker: mosquitto:1883` par `systemd: mosquitto-broker.service (1883/9001)`.
- Section 0 (Commandes rapides) : remplacer `make up / make down` par les nouvelles commandes.
- Section 8 (Problèmes courants) : ajouter `journalctl -u mosquitto-broker -n 50`.

### 16.4 Désinstaller Mosquitto natif (si rollback définitif)

```bash
# Si vous décidez de revenir définitivement à Docker
sudo apt remove --purge mosquitto mosquitto-clients
sudo rm -rf /var/lib/mosquitto /var/log/mosquitto
sudo rm /etc/systemd/system/mosquitto-broker.service
sudo systemctl daemon-reload
```

---

## 17. Checklist finale

### Avant de commencer la migration

```
□ Faire un git commit de tout le code stable
□ Créer un tag git : git tag -a v-pre-mosquitto-native -m "avant migration Mosquitto natif"
□ Copie du dépôt GitHub faite (fork ou clone de secours)
□ Noter l'heure : si problème > 30min → rollback immédiat
□ Vérifier que NanoPi est accessible : ping 192.168.1.120
□ Vérifier le portal_id dans Config.toml (section [energy_manager])
□ Vérifier que mosquitto n'est pas déjà installé nativement
□ Avoir un accès physique ou SSH de secours au Pi5
□ Préparer le document de rollback (§15) sous les yeux
```

### Pendant la migration

```
□ Installer Mosquitto natif AVANT d'arrêter Docker
□ Tester la syntaxe de mosquitto.conf avec -t
□ Vérifier les bridges avec $SYS/broker/bridge/+/state
□ Exécuter verify-no-loop.sh après chaque modif
□ Tester pub/sub local avant de redémarrer daly-bms
□ Vérifier les bridges (ingress ET egress) avant energy-manager
□ Surveiller les logs en temps réel pendant 5 minutes après chaque démarrage
□ Vérifier VRM : tous les devices "just now"
□ Vérifier dashboard Pi5 : pas de "Hors ligne"
```

### Après la migration (24h)

```
□ VRM : tous les devices "just now" (pas "an hour ago")
□ Dashboard Pi5 : pas de "Hors ligne" ou "En attente de données"
□ Tasmota : commandes ON/OFF fonctionnent depuis Pi5 web
□ Shelly : données affichées + contrôle DEYE fonctionne
□ Pas de flood MQTT (vérifier avec mosquitto_sub ou WS :9001)
□ RAM totale du Pi5 réduite de ~50-100 Mo (vérifier avec free -h)
□ Aucun conteneur mosquitto actif (docker ps | grep mosquitto → vide)
□ Mettre à jour CLAUDE.md section architecture
□ Archiver docker-compose.infra.yml et docker/mosquitto/ après 24h de stabilité
□ Supprimer les backups .bak une semaine après confirmation
```

---

## Références

- [Documentation Mosquitto](https://mosquitto.org/documentation/)
- [Configuration bridges](https://mosquitto.org/man/mosquitto-conf-5.html)
- [Man page mosquitto](https://mosquitto.org/man/mosquitto-8.html)
- [Debian package mosquitto](https://packages.debian.org/bookworm/mosquitto)
- [Guide anti-boucle MQTT bridges](https://mosquitto.org/man/mosquitto-conf-5.html#idm459)
