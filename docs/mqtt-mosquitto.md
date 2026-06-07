# Architecture MQTT — Mosquitto Natif — Daly-BMS-Rust

> Ce document décrit l'infrastructure MQTT du projet : broker Mosquitto natif systemd sur le Pi5
> (ports 1883 TCP / 9001 WebSocket), bridge unique `pi5-to-nanopi` / `nanopi-to-pi5`, table
> complète des topics `santuario/*`, producteurs/consommateurs, validation anti-boucle, et
> l'historique complet de la migration Docker→natif (mai 2026).
> Fait partie de l'[architecture documentaire](./ARCHITECTURE.md).
> Dernière consolidation : 2026-06-07.

## Table des matières

- [1. Vue d'ensemble](#1-vue-densemble)
  - [1.1 Topologie courante](#11-topologie-courante)
  - [1.2 Rôle de chaque composant](#12-role-de-chaque-composant)
- [2. Broker Mosquitto natif systemd](#2-broker-mosquitto-natif-systemd)
  - [2.1 Installation et packages](#21-installation-et-packages)
  - [2.2 Configuration mosquitto.conf complète](#22-configuration-mosquittoconf-complete)
  - [2.3 Service systemd mosquitto-broker](#23-service-systemd-mosquitto-broker)
  - [2.4 Répertoires et permissions](#24-repertoires-et-permissions)
  - [2.5 Commandes d'exploitation courantes](#25-commandes-dexploitation-courantes)
- [3. Topics MQTT — table complète](#3-topics-mqtt--table-complete)
  - [3.1 Table principale santuario/* → services D-Bus](#31-table-principale-santuario--services-d-bus)
  - [3.2 Flux EGRESS — Pi5 vers NanoPi (bridge out)](#32-flux-egress--pi5-vers-nanopi-bridge-out)
  - [3.3 Flux INGRESS — NanoPi vers Pi5 (bridge in)](#33-flux-ingress--nanopi-vers-pi5-bridge-in)
  - [3.4 Flux locaux uniquement (pas de bridge)](#34-flux-locaux-uniquement-pas-de-bridge)
- [4. Bridge MQTT Pi5 ↔ NanoPi](#4-bridge-mqtt-pi5--nanopi)
  - [4.1 Principe du bridge](#41-principe-du-bridge)
  - [4.2 Configuration du bridge dans mosquitto.conf](#42-configuration-du-bridge-dans-mosquittoconf)
  - [4.3 Vérification de l'état des bridges](#43-verification-de-letat-des-bridges)
- [5. Producteurs et consommateurs MQTT](#5-producteurs-et-consommateurs-mqtt)
  - [5.1 daly-bms-server (Pi5 :8080)](#51-daly-bms-server-pi5-8080)
  - [5.2 energy-manager (Pi5 :8081)](#52-energy-manager-pi5-8081)
  - [5.3 dbus-mqtt-venus (NanoPi)](#53-dbus-mqtt-venus-nanopi)
  - [5.4 Tasmota / Shelly (via NanoPi)](#54-tasmota--shelly-via-nanopi)
- [6. Config.toml — paramètres MQTT](#6-configtoml--parametres-mqtt)
  - [6.1 Section [mqtt] — daly-bms-server](#61-section-mqtt--daly-bms-server)
  - [6.2 Section [energy_manager.mqtt]](#62-section-energy_managermqtt)
  - [6.3 portal_id — identifiant Venus OS](#63-portal_id--identifiant-venus-os)
- [7. Dépendances systemd](#7-dependances-systemd)
  - [7.1 daly-bms.service](#71-daly-bmsservice)
  - [7.2 energy-manager.service](#72-energy-managerservice)
- [8. Validation anti-boucle](#8-validation-anti-boucle)
  - [8.1 Principe du risque](#81-principe-du-risque)
  - [8.2 Script verify-no-loop.sh](#82-script-verify-no-loopsh)
  - [8.3 Vérification manuelle complémentaire](#83-verification-manuelle-complementaire)
- [9. Dépannage MQTT](#9-depannage-mqtt)
  - [9.1 Problèmes courants](#91-problemes-courants)
  - [9.2 Commandes de diagnostic](#92-commandes-de-diagnostic)
- [10. Annexe historique — Migration Docker→Natif (mai 2026)](#10-annexe-historique--migration-dockernatif-mai-2026)
  - [10.1 Architecture avant migration (Docker)](#101-architecture-avant-migration-docker)
  - [10.2 Problèmes identifiés dans l'ancienne architecture](#102-problemes-identifies-dans-lancienne-architecture)
  - [10.3 Architecture cible](#103-architecture-cible)
  - [10.4 Prérequis et préparation](#104-prerequis-et-preparation)
  - [10.5 Sauvegarde de l'état actuel](#105-sauvegarde-de-letat-actuel)
  - [10.6 Installation Mosquitto natif](#106-installation-mosquitto-natif)
  - [10.7 Déploiement pas à pas (référence manuelle)](#107-deploiement-pas-a-pas-reference-manuelle)
  - [10.8 Vérification flux par flux](#108-verification-flux-par-flux)
  - [10.9 Procédure de rollback](#109-procedure-de-rollback)
  - [10.10 Nettoyage post-migration](#1010-nettoyage-post-migration)
  - [10.11 Checklist complète de migration](#1011-checklist-complete-de-migration)
  - [10.12 Journal d'exécution — 12 mai 2026 (migration réussie)](#1012-journal-dexecution--12-mai-2026-migration-reussie)

---

## 1. Vue d'ensemble

### 1.1 Topologie courante

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              PI5 (192.168.1.141)                            │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────────┐│
│  │               mosquitto-broker.service (NATIF systemd)                  ││
│  │  ┌─────────────────────────────────────────────────────────────────┐   ││
│  │  │  Services locaux : daly-bms-server, energy-manager, dashboard   │   ││
│  │  │  Tous connectés en 127.0.0.1:1883 (loopback)                    │   ││
│  │  └─────────────────────────────────────────────────────────────────┘   ││
│  │                              │                                         ││
│  │  ┌───────────────────────────┼─────────────────────────────────────┐   ││
│  │  │  Bridge EGRESS (out)      │  → 192.168.1.120 (NanoPi)           │   ││
│  │  │  santuario/bms/#          │  santuario/pvinverter/#             │   ││
│  │  │  santuario/switch/#       │  santuario/heatpump/#               │   ││
│  │  │  santuario/irradiance/raw │  santuario/meteo/venus              │   ││
│  │  │  santuario/heat/+/venus   │  santuario/system/venus             │   ││
│  │  │  santuario/platform/venus │  W/{portal}/#                       │   ││
│  │  │  R/{portal}/#             │  cmnd/#                             │   ││
│  │  │  shellypro2pm-.../rpc     │                                     │   ││
│  │  └───────────────────────────┼─────────────────────────────────────┘   ││
│  │                              │                                         ││
│  │  ┌───────────────────────────┼─────────────────────────────────────┐   ││
│  │  │  Bridge INGRESS (in)      │  ← 192.168.1.120 (NanoPi)           │   ││
│  │  │  N/{portal}/#             │  tele/#                             │   ││
│  │  │  stat/#                   │  shellypro2pm-ec62608840a4/#        │   ││
│  │  │  daly-bms-shelly/rpc      │                                     │   ││
│  │  └───────────────────────────┼─────────────────────────────────────┘   ││
│  └──────────────────────────────┼─────────────────────────────────────────┘│
│                                 │                                           │
│  ┌──────────────────────┐   127.0.0.1:1883                                  │
│  │ daly-bms-server      │◄──────────────────►  PUBLIE/ABONNE local          │
│  │ RS485 → MQTT :8080   │    loopback                                        │
│  └──────────────────────┘                                                   │
│           ▲                                                                  │
│  ┌──────────────────────┐   127.0.0.1:1883                                  │
│  │ energy-manager       │◄──────────────────►  PUBLIE/ABONNE local          │
│  │ API cloud + règles   │    loopback                                        │
│  └──────────────────────┘                                                   │
│           │                                                                  │
│  Dashboard JS (WebSocket :9001)                                              │
│  metrics-store redb (embarqué dans daly-bms-server :8080)                   │
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
│    Tasmota (WiFi)  Shelly (WiFi)                                           │
│    192.168.1.115   192.168.1.136                                           │
│                                                                             │
│  ⚠️ Tasmota/Shelly restent sur NanoPi (config WiFi fixe)                   │
│     → Migration possible si reconfiguration des devices                    │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 1.2 Rôle de chaque composant

| Composant | Hôte | Ports | Rôle MQTT |
|-----------|------|-------|-----------|
| **mosquitto-broker.service** | Pi5 | 1883 (TCP), 9001 (WS) | Broker central natif systemd — hub de tous les messages locaux + bridge vers/depuis NanoPi |
| **daly-bms-server** | Pi5 | — | Publie les snapshots RS485 (BMS, ET112, ATS, irradiance) en local 127.0.0.1:1883 |
| **energy-manager** | Pi5 | — | Publie météo, température, commandes VEBus, keepalive, baselines en local 127.0.0.1:1883 ; souscrit N/{portal}/# pour les télémétriques Venus OS |
| **dbus-mqtt-venus** | NanoPi | — | Souscrit sur broker NanoPi local aux topics `santuario/*/venus` bridgés depuis Pi5, enregistre les services D-Bus Victron |
| **Tasmota / Tongou** | NanoPi (WiFi) | — | Publie `tele/#` / `stat/#` directement sur broker NanoPi ; bridgé INGRESS vers Pi5 |
| **Shelly Pro 2PM** | NanoPi (WiFi) | — | Publie `shellypro2pm-ec62608840a4/#` sur broker NanoPi ; bridgé INGRESS vers Pi5 |

> **Principe directeur** : Tout ce qui est produit sur le Pi5 est publié sur le broker LOCAL (127.0.0.1).
> Le bridge ne sert qu'à échanger avec le NanoPi.
> Le NanoPi reste le point d'entrée pour Tasmota/Shelly (config WiFi fixe).

---

## 2. Broker Mosquitto natif systemd

### 2.1 Installation et packages

```bash
sudo apt update
sudo apt install -y mosquitto mosquitto-clients

# Vérifier la version
mosquitto -h | head -n 3
# → mosquitto version 2.0.21

# Désactiver le service Debian par défaut (on utilise notre unité mosquitto-broker)
sudo systemctl stop mosquitto
sudo systemctl disable mosquitto
```

Packages requis :
- `mosquitto` — version 2.0.x (Debian Bookworm)
- `mosquitto-clients` — outils `mosquitto_pub` / `mosquitto_sub` pour les diagnostics

### 2.2 Configuration mosquitto.conf complète

Fichier déployé vers `/etc/mosquitto/mosquitto.conf` depuis `contrib/mosquitto/mosquitto.conf`.

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

> **Note** : Remplacer `c0619ab9929a` par le `portal_id` réel si nécessaire. Vérifier avec :
> ```bash
> grep '^portal_id' /etc/daly-bms/config.toml
> # ou côté NanoPi :
> ssh root@192.168.1.120 "mosquitto_sub -h localhost -t 'N/+/system/0/Serial' -C 1 -W 5"
> ```

> **Déploiement** : La config versionée est dans `contrib/mosquitto/mosquitto.conf`.
> Déployer vers `/etc/mosquitto/mosquitto.conf` manuellement ou via `contrib/mosquitto/deploy-mosquitto-native.sh`.
> **Après toute modification** : exécuter `sudo /usr/local/bin/verify-no-loop.sh` (voir §8).

### 2.3 Service systemd mosquitto-broker

Fichier : `contrib/mosquitto-broker.service` (déployé vers `/etc/systemd/system/mosquitto-broker.service`).

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

**Pourquoi un service personnalisé ?** Le service `mosquitto.service` installé par Debian est générique. Notre version :
- Démarre **avant** `daly-bms` et `energy-manager` (`Before=`)
- Limite la mémoire à 50 Mo (marge confortable — mesure réelle ~8 Mo)
- Utilise `ProtectSystem=strict` (Mosquitto est audité depuis des années)
- Utilise `SyslogIdentifier=mosquitto-broker` (journalctl -u mosquitto-broker)

### 2.4 Répertoires et permissions

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

| Chemin | Contenu | Propriétaire |
|--------|---------|--------------|
| `/etc/mosquitto/mosquitto.conf` | Configuration principale (bridge + listeners) | root |
| `/var/lib/mosquitto/mosquitto.db` | Messages retained (persistence) | mosquitto |
| `/var/log/mosquitto/` | Logs (si `log_dest file` — actuellement vers syslog) | mosquitto |
| `/usr/local/bin/verify-no-loop.sh` | Script de vérification anti-boucle | root |
| `contrib/mosquitto/mosquitto.conf` | Source versionnée dans le dépôt | — |
| `contrib/mosquitto-broker.service` | Unité systemd versionnée | — |

### 2.5 Commandes d'exploitation courantes

```bash
# État du broker
systemctl status mosquitto-broker

# Logs en temps réel
journalctl -u mosquitto-broker -f

# Logs des dernières 30 lignes
journalctl -u mosquitto-broker -n 30 --no-pager

# Redémarrer
sudo systemctl restart mosquitto-broker

# Recharger la config (SIGHUP — sans coupure)
sudo systemctl reload mosquitto-broker

# Activer au démarrage
sudo systemctl enable mosquitto-broker

# Vérifier les ports
ss -tlnp | grep -E ':(1883|9001)'

# Test pub/sub local rapide
mosquitto_sub -h 127.0.0.1 -p 1883 -t "test/#" -v -C 1 &
sleep 1
mosquitto_pub -h 127.0.0.1 -p 1883 -t "test/ping" -m "hello"
wait

# Vérifier l'état des bridges
timeout 5 mosquitto_sub -h 127.0.0.1 -t '$SYS/broker/bridge/+/state' -v -C 2
# Attendu :
#   $SYS/broker/bridge/pi5-to-nanopi/state 1
#   $SYS/broker/bridge/nanopi-to-pi5/state 1
```

---

## 3. Topics MQTT — table complète

### 3.1 Table principale santuario/* → services D-Bus

Préfixe obligatoire : `santuario/`

| Topic MQTT (sans préfixe) | Source (Pi5) | Bridge NanoPi | Cible D-Bus Venus |
|---|---|---|---|
| `bms/{n}/venus` | `daly-bms-server` (RS485 BMS) | `dbus-mqtt-venus` | `com.victronenergy.battery.mqtt_{n}` |
| `pvinverter/{n}/venus` | `daly-bms-server` (ET112 0x07) | `dbus-mqtt-venus` | `com.victronenergy.pvinverter.mqtt_{n}` |
| `heatpump/{n}/venus` | `daly-bms-server` (ET112 0x08/0x09) | `dbus-mqtt-venus` | `com.victronenergy.heatpump.mqtt_{n}` |
| `heat/{n}/venus` | `energy-manager` (LG ThinQ) | `dbus-mqtt-venus` | `com.victronenergy.temperature.mqtt_{n}` |
| `switch/{n}/venus` | `daly-bms-server` (ATS / Tongou) | `dbus-mqtt-venus` | `com.victronenergy.switch.mqtt_{n}` |
| `meteo/venus` | `daly-bms-server` (PRALRAN 0x05) | `dbus-mqtt-venus` | `com.victronenergy.meteo` |
| `system/venus` | `energy-manager` (SmartShunt) | `dbus-mqtt-venus` | `com.victronenergy.system` |
| `platform/venus` | `energy-manager` (platform Pi5) | `dbus-mqtt-venus` | `com.victronenergy.platform` |
| `irradiance/raw` | `daly-bms-server` (PRALRAN 0x05) | bridgé EGRESS | energy-manager (local) |

> `dbus-mqtt-venus` est le **seul binaire sur le NanoPi** — il souscrit à tous les topics
> `santuario/*/venus` reçus via le bridge et enregistre tous les services D-Bus.
> Un seul processus, ~5–8 Mo RAM (binaire statique musl).

### 3.2 Flux EGRESS — Pi5 vers NanoPi (bridge out)

Tableau complet avec QoS, retain, producteur et consommateur NanoPi :

| Source | Topic complet | QoS | Retain | Producteur | Consommateur NanoPi |
|--------|---------------|-----|--------|------------|---------------------|
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

### 3.3 Flux INGRESS — NanoPi vers Pi5 (bridge in)

| Source | Topic | QoS | Retain | Producteur NanoPi | Consommateur Pi5 |
|--------|-------|-----|--------|-------------------|------------------|
| Venus OS | `N/c0619ab9929a/#` | 0 | false | dbus-mqtt-venus (publish) | energy-manager |
| Tongou mesures | `tele/tongou_*/SENSOR` | 0 | false | Tasmota | daly-bms-server + energy-manager |
| Tongou état | `stat/tongou_*/POWER` | 0 | false | Tasmota | energy-manager |
| Shelly status | `shellypro2pm-ec62608840a4/#` | 0 | false | Shelly | energy-manager |
| Shelly RPC réponse | `daly-bms-shelly/rpc` | 0 | false | Shelly | energy-manager |

### 3.4 Flux locaux uniquement (pas de bridge)

Ces topics restent sur le broker Pi5 local et ne sont **jamais** bridgés vers le NanoPi :

| Source | Topic | Destination | Utilisation |
|--------|-------|-------------|-------------|
| energy-manager | `santuario/persist/pvinv_baseline` | daly-bms-server | Baselines PV (retained) |
| energy-manager | `santuario/persist/yield_yesterday` | daly-bms-server | Historique production PV |
| energy-manager | `santuario/persist/deye_state` | daly-bms-server | État relais DEYE |

> Ces topics `santuario/persist/*` sont publiés avec le flag `retain=true`. Ils permettent à
> daly-bms-server de restaurer ses baselines au démarrage (voir `crates/energy-manager/src/persist/`).

---

## 4. Bridge MQTT Pi5 ↔ NanoPi

### 4.1 Principe du bridge

Le bridge Mosquitto est une connexion TCP persistante entre le broker Pi5 (127.0.0.1:1883) et le broker NanoPi (192.168.1.120:1883).

**Règles fondamentales** :
- Un topic `out` (EGRESS) ne doit **jamais** figurer dans la liste `in` (INGRESS) — risque de boucle infinie.
- Tous les topics `santuario/*` sont **exclusivement produits sur le Pi5** et vont en EGRESS vers le NanoPi.
- Les topics `N/{portal}/#`, `tele/#`, `stat/#`, `shellypro2pm/#`, `daly-bms-shelly/rpc` sont **exclusivement produits sur le NanoPi** et viennent en INGRESS.
- `try_private true` signale au broker distant qu'on est un bridge (aide à prévenir les boucles si le distant dispose aussi d'un bridge symétrique).

La configuration utilise **deux connexions de bridge distinctes** nommées `pi5-to-nanopi` (EGRESS) et `nanopi-to-pi5` (INGRESS), ce qui facilite le monitoring de l'état de chaque direction via :
```
$SYS/broker/bridge/pi5-to-nanopi/state
$SYS/broker/bridge/nanopi-to-pi5/state
```

### 4.2 Configuration du bridge dans mosquitto.conf

Voir §2.2 pour la configuration complète. Les points clés :

**EGRESS (Pi5 → NanoPi)** — topics produits sur Pi5 :
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

**INGRESS (NanoPi → Pi5)** — topics produits sur NanoPi :
```
N/c0619ab9929a/#
tele/#
stat/#
shellypro2pm-ec62608840a4/#
daly-bms-shelly/rpc
```

### 4.3 Vérification de l'état des bridges

```bash
# État en temps réel (valeur "1" = connecté, "0" = déconnecté)
timeout 5 mosquitto_sub -h 127.0.0.1 -t '$SYS/broker/bridge/+/state' -v -C 2

# Logs de connexion des bridges
journalctl -u mosquitto-broker | grep -i "bridge"
# → doit afficher "Connecting bridge pi5-to-nanopi..." et "Connecting bridge nanopi-to-pi5..."

# Trafic reçu depuis le NanoPi (Venus OS)
timeout 5 mosquitto_sub -h 127.0.0.1 -t "N/c0619ab9929a/#" -v | head -5

# Test EGRESS : publier depuis Pi5, vérifier sur NanoPi
mosquitto_pub -h 127.0.0.1 -p 1883 -t "santuario/bms/test" -m '{"probe":1}' -q 1
ssh root@192.168.1.120 \
  "timeout 5 mosquitto_sub -h localhost -t 'santuario/bms/test' -v -C 1"
# Attendu : santuario/bms/test {"probe":1}
```

---

## 5. Producteurs et consommateurs MQTT

### 5.1 daly-bms-server (Pi5 :8080)

**Connexion** : `127.0.0.1:1883` (loopback, client rumqttc)
**Bibliothèque** : `rumqttc` (async Rust)
**Config** : section `[mqtt]` dans `/etc/daly-bms/config.toml`

**Topics publiés** :

| Topic | Cadence | QoS | Retain | Contenu |
|-------|---------|-----|--------|---------|
| `santuario/bms/{n}/venus` | 1s | 1 | true | Snapshot BMS complet (SOC, tension, courant, cellules, alarmes) |
| `santuario/pvinverter/7/venus` | polling | 0 | true | ET112 0x07 — Puissance micro-onduleurs |
| `santuario/heatpump/8/venus` | polling | 0 | true | ET112 0x08 — Consommation maison |
| `santuario/heatpump/9/venus` | polling | 0 | true | ET112 0x09 — Réseau EDF |
| `santuario/switch/1/venus` | polling | 0 | true | ATS CHINT — état bascule |
| `santuario/irradiance/raw` | polling | 1 | true | PRALRAN 0x05 — irradiance brute |

**Topics souscrits** :

| Topic | Source | Usage |
|-------|--------|-------|
| `tele/tongou_*/SENSOR` | Tasmota (via bridge INGRESS) | Mesures puissance Tongou |
| `stat/tongou_*/POWER` | Tasmota (via bridge INGRESS) | État relais Tongou |
| `shellypro2pm-ec62608840a4/#` | Shelly (via bridge INGRESS) | Données Shelly Pro 2PM |
| `santuario/system/venus` | energy-manager (local) | SmartShunt / bilan système |
| `santuario/persist/*` | energy-manager (local) | Restauration baselines PV |

> Voir `crates/daly-bms-server/src/bridges/` pour l'implémentation du client MQTT.
> Voir [./app-daly-bms-server.md] pour l'architecture complète du serveur.

### 5.2 energy-manager (Pi5 :8081)

**Connexion** : `127.0.0.1:1883` (loopback, client rumqttc)
**Bibliothèque** : `rumqttc` (async Rust)
**Config** : section `[energy_manager.mqtt]` dans `/etc/daly-bms/config.toml`

**Topics publiés** :

| Topic | Cadence | QoS | Retain | Contenu |
|-------|---------|-----|--------|---------|
| `santuario/meteo/venus` | périodique | 0 | true | Météo Open-Meteo + irradiance calculée |
| `santuario/heat/1/venus` | périodique | 0 | true | Température extérieure LG ThinQ |
| `santuario/heatpump/1/venus` | périodique | 0 | true | Données chauffe-eau PAC |
| `santuario/system/venus` | périodique | 0 | true | SmartShunt / bilan DC bus |
| `santuario/platform/venus` | périodique | 0 | true | Infos platform Pi5 |
| `W/c0619ab9929a/#` | sur événement | 1 | false | Commandes D-Bus VEBus (DVCC) |
| `R/c0619ab9929a/#` | périodique | 1 | false | Keepalive lecture D-Bus |
| `cmnd/tongou_*/POWER` | sur décision | 1 | false | Commandes ON/OFF Tongou |
| `shellypro2pm-ec62608840a4/rpc` | sur décision | 0 | false | Commandes RPC Shelly DEYE |
| `santuario/persist/pvinv_baseline` | sur changement | 1 | true | Baseline PV (retained) |
| `santuario/persist/yield_yesterday` | quotidien | 1 | true | Production PV J-1 (retained) |
| `santuario/persist/deye_state` | sur changement | 1 | true | État relais DEYE (retained) |

**Topics souscrits** :

| Topic | Source | Usage |
|-------|--------|-------|
| `N/c0619ab9929a/#` | Venus OS (via bridge INGRESS) | Télémétriques Cerbo GX (MPPT, SmartShunt, onduleur) |
| `tele/tongou_*/SENSOR` | Tasmota (via bridge INGRESS) | Mesures puissance Tongou |
| `stat/tongou_*/POWER` | Tasmota (via bridge INGRESS) | État relais Tongou |
| `shellypro2pm-ec62608840a4/#` | Shelly (via bridge INGRESS) | Données Shelly Pro 2PM |
| `daly-bms-shelly/rpc` | Shelly (via bridge INGRESS) | Réponses RPC Shelly |
| `santuario/irradiance/raw` | daly-bms-server (local) | Irradiance PRALRAN |

> Voir [./app-energy-manager.md] pour l'architecture complète de l'energy-manager.

### 5.3 dbus-mqtt-venus (NanoPi)

**Connexion** : `localhost:1883` (broker NanoPi local — reçoit les topics bridgés depuis Pi5)
**Bibliothèque** : `rumqttc` (async Rust)

**Topics souscrits** (reçus via bridge EGRESS depuis Pi5) :

| Topic | Service D-Bus créé |
|-------|-------------------|
| `santuario/bms/{n}/venus` | `com.victronenergy.battery.mqtt_{n}` |
| `santuario/pvinverter/{n}/venus` | `com.victronenergy.pvinverter.mqtt_{n}` |
| `santuario/heatpump/{n}/venus` | `com.victronenergy.heatpump.mqtt_{n}` |
| `santuario/heat/{n}/venus` | `com.victronenergy.temperature.mqtt_{n}` |
| `santuario/switch/{n}/venus` | `com.victronenergy.switch.mqtt_{n}` |
| `santuario/meteo/venus` | `com.victronenergy.meteo` |
| `santuario/platform/venus` | `com.victronenergy.platform` |

> Voir [./app-dbus-mqtt-venus.md] pour l'architecture complète du bridge D-Bus.

### 5.4 Tasmota / Shelly (via NanoPi)

Les devices WiFi Tasmota (Tongou) et Shelly Pro 2PM sont configurés pour publier **directement sur le broker NanoPi** (192.168.1.120:1883). Leur config WiFi MQTT est fixe.

Le broker NanoPi les reçoit, et le bridge INGRESS `nanopi-to-pi5` les achemine vers le broker Pi5.

**Impact** : Si le NanoPi est indisponible, les données Tasmota/Shelly ne parviennent plus au Pi5. Migration possible si les devices sont reconfigurés pour pointer directement vers 192.168.1.141:1883.

---

## 6. Config.toml — paramètres MQTT

### 6.1 Section [mqtt] — daly-bms-server

```toml
[mqtt]
enabled = true
host = "127.0.0.1"              # ← Mosquitto natif local (loopback)
port = 1883
topic_prefix = "santuario/bms"  # NE PAS CHANGER (utilisé par dbus-mqtt-venus côté NanoPi)
publish_interval_sec = 1
format = "json"
```

> **Important** : `topic_prefix = "santuario/bms"` est utilisé par `dbus-mqtt-venus` sur le NanoPi.
> Ne pas modifier ce préfixe sans mettre à jour simultanément la config du NanoPi.

### 6.2 Section [energy_manager.mqtt]

```toml
[energy_manager.mqtt]
host = "127.0.0.1"              # ← localhost (loopback)
port = 1883
keep_alive_secs = 60
reconnect_delay_secs = 10
```

### 6.3 portal_id — identifiant Venus OS

```toml
[energy_manager]
portal_id = "c0619ab9929a"      # Ne pas modifier — utilisé par les bridges W/R/N
```

Le `portal_id` identifie le Cerbo GX (ou équivalent Venus OS) sur le réseau VRM. Il est utilisé dans les topics `W/c0619ab9929a/#`, `R/c0619ab9929a/#` et `N/c0619ab9929a/#`. Toute modification nécessite de mettre à jour simultanément `mosquitto.conf` (patterns de bridge) et `Config.toml`.

---

## 7. Dépendances systemd

### 7.1 daly-bms.service

```ini
[Unit]
Description=DalyBMS Server — Rust RS485 BMS monitor
Documentation=https://github.com/thieryus007-cloud/Daly-BMS-Rust
After=network.target mosquitto-broker.service
Wants=network.target
Requires=mosquitto-broker.service
```

`Requires=mosquitto-broker.service` garantit que le broker est démarré avant daly-bms-server.
Si le broker s'arrête, daly-bms-server s'arrête aussi (systemd dependency).

### 7.2 energy-manager.service

```ini
[Unit]
Description=Energy Manager — Gestionnaire d'énergie Rust (remplace Node-RED)
Documentation=https://github.com/thieryus007-cloud/Daly-BMS-Rust
After=network-online.target mosquitto-broker.service daly-bms.service
Wants=network-online.target
Requires=mosquitto-broker.service
PartOf=daly-bms.service
```

`PartOf=daly-bms.service` signifie que si daly-bms.service s'arrête/redémarre, energy-manager s'arrête/redémarre aussi.

**Ordre de démarrage** :
```
network-online.target
    └── mosquitto-broker.service
            ├── daly-bms.service
            └── energy-manager.service
```

---

## 8. Validation anti-boucle

### 8.1 Principe du risque

Une boucle MQTT se produit quand un topic est configuré à la fois en `out` et en `in` dans le bridge. Le message publié sur Pi5 est bridgé vers NanoPi, qui le reçoit et le re-bridgé vers Pi5, qui le re-publie... créant un flood infini.

Les anciennes configurations Docker présentaient ce risque avec `santuario/heat/#` configuré à la fois en INGRESS et EGRESS. Ce problème est résolu dans l'architecture courante (séparation stricte des directions).

**Règle absolue** :
- Les topics `santuario/*` ne doivent **jamais** figurer dans la liste INGRESS.
- Les topics `N/{portal}/#`, `tele/#`, `stat/#`, `shellypro2pm/#`, `daly-bms-shelly/rpc` ne doivent **jamais** figurer dans la liste EGRESS.

### 8.2 Script verify-no-loop.sh

Fichier source : `contrib/mosquitto/verify-no-loop.sh`
Installé dans : `/usr/local/bin/verify-no-loop.sh` (via `deploy-mosquitto-native.sh`)

```bash
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
```

**Usage** :
```bash
# Vérifier après chaque modification de mosquitto.conf
sudo /usr/local/bin/verify-no-loop.sh

# Résultat attendu (état validé en production) :
# === Topics EGRESS (out) ===
# R/c0619ab9929a/#
# W/c0619ab9929a/#
# cmnd/#
# santuario/bms/#
# santuario/heatpump/#
# santuario/heat/+/venus
# santuario/irradiance/raw
# santuario/meteo/venus
# santuario/platform/venus
# santuario/pvinverter/#
# santuario/switch/#
# santuario/system/venus
# shellypro2pm-ec62608840a4/rpc
# === Topics INGRESS (in) ===
# N/c0619ab9929a/#
# daly-bms-shelly/rpc
# shellypro2pm-ec62608840a4/#
# stat/#
# tele/#
# === INTERSECTION (DANGER — topics en double) ===
# ✅ OK : Aucun topic en double. Pas de risque de boucle.
```

L'intersection validée en production compte **13 topics EGRESS, 5 topics INGRESS, intersection vide**.

### 8.3 Vérification manuelle complémentaire

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

## 9. Dépannage MQTT

### 9.1 Problèmes courants

| Symptôme | Cause probable | Solution |
|----------|---------------|----------|
| `daly-bms` ne démarre pas après `mosquitto-broker` | Dépendance systemd non satisfaite | `journalctl -u mosquitto-broker -n 50` — vérifier que le broker est bien actif |
| `energy-manager` ne reçoit pas MQTT | `portal_id` incorrect ou Mosquitto inaccessible | Vérifier `portal_id` dans `Config.toml` et que Mosquitto est accessible sur `mqtt.host` |
| Bridge non connecté (`state 0`) | NanoPi inaccessible ou broker NanoPi down | `ping 192.168.1.120` ; `ssh root@192.168.1.120 "systemctl is-active mosquitto"` |
| Flood MQTT (> 100 msg/s sur `santuario/#`) | Boucle bridge | `sudo /usr/local/bin/verify-no-loop.sh` — vérifier l'absence d'intersection IN/OUT |
| VRM n'affiche plus les devices | Bridge EGRESS coupé ou `dbus-mqtt-venus` inactif | Vérifier `$SYS/broker/bridge/pi5-to-nanopi/state` ; `ssh root@192.168.1.120 "svstat /service/dbus-mqtt-venus"` |
| Dashboard affiche cumul brut | `pvinv_baseline` retained absent ou corrompu | Vérifier `santuario/persist/pvinv_baseline` retained MQTT : `mosquitto_sub -h 127.0.0.1 -t 'santuario/persist/#' -v -C 3` |
| Commandes Tongou sans effet | Bridge EGRESS `cmnd/#` coupé ou Tasmota down | Vérifier l'état du bridge ; `timeout 5 mosquitto_sub -h 127.0.0.1 -t 'stat/tongou_#' -v` |
| Réponses Shelly non reçues | Bridge INGRESS `daly-bms-shelly/rpc` absent | Vérifier que le topic est bien dans la liste INGRESS de `mosquitto.conf` |

### 9.2 Commandes de diagnostic

```bash
# Broker Mosquitto natif — status et logs
systemctl status mosquitto-broker
journalctl -u mosquitto-broker -f
journalctl -u mosquitto-broker -n 50 --no-pager

# Vérifier les ports
ss -tlnp | grep -E ':(1883|9001)'

# État des bridges
timeout 5 mosquitto_sub -h 127.0.0.1 -t '$SYS/broker/bridge/+/state' -v -C 2

# Flux BMS en temps réel
timeout 10 mosquitto_sub -h 127.0.0.1 -t "santuario/bms/1/venus" -v | head -n 3

# Flux ET112
timeout 5 mosquitto_sub -h 127.0.0.1 -t "santuario/pvinverter/7/venus" -v | head -n 1

# Flux Venus OS (NanoPi → Pi5)
timeout 5 mosquitto_sub -h 127.0.0.1 -t "N/c0619ab9929a/system/0/Dc/Battery/Soc" -v | head -n 1

# Flux Tasmota (NanoPi → Pi5)
timeout 5 mosquitto_sub -h 127.0.0.1 -t "tele/tongou_3BC764/SENSOR" -v | head -n 1

# Flux Shelly (NanoPi → Pi5)
timeout 5 mosquitto_sub -h 127.0.0.1 -t "shellypro2pm-ec62608840a4/status/switch:0" -v | head -n 1

# Vérification anti-boucle
sudo /usr/local/bin/verify-no-loop.sh

# Logs BMS server (côté MQTT)
journalctl -u daly-bms -f | grep -iE 'mqtt|connect|error'

# Logs energy-manager (côté MQTT)
journalctl -u energy-manager -f | grep -iE 'mqtt|connect|error'

# Logs dbus-mqtt-venus sur NanoPi
ssh root@192.168.1.120 "tail -f /var/log/dbus-mqtt-venus/current"

# Redémarrer le broker (fenêtre indisponible ~2s)
sudo systemctl restart mosquitto-broker

# Nettoyage complet (DONNÉES RETAINED PERDUES)
sudo systemctl stop mosquitto-broker
sudo rm -f /var/lib/mosquitto/mosquitto.db
sudo systemctl start mosquitto-broker
```

---

## 10. Annexe historique — Migration Docker→Natif (mai 2026)

> **Statut : MIGRATION TERMINÉE le 12 mai 2026 — section historique, conservée pour référence.**
> L'état courant (Mosquitto natif systemd) est décrit dans les sections 1 à 9 ci-dessus.
> Cette annexe documente la motivation, les étapes et le journal d'exécution de la migration.

### 10.1 Architecture avant migration (Docker)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              RÉSEAU 192.168.1.0/24                          │
│                                                                             │
│  ┌─────────────────────────────┐              ┌─────────────────────────────┐│
│  │   PI5 (192.168.1.141)       │              │  NANOPI (192.168.1.120)     ││
│  │                             │              │                             ││
│  │  ┌─────────────────────┐    │   MQTT       │  ┌─────────────────────┐   ││
│  │  │ daly-bms-server     │────┼──►192.168.1.120  │ mosquitto (Venus)   │   ││
│  │  │ PUBLIE sur NanoPi   │    │   (TCP)      │  │ dbus-mqtt-venus      │   ││
│  │  └─────────────────────┘    │              │  │   (Rust/zbus)         │   ││
│  │         │                   │              │  │        │              │   ││
│  │  ┌─────────────────────┐    │              │  │   D-Bus Victron       │   ││
│  │  │ energy-manager      │────┼──►192.168.1.141  │        │              │   ││
│  │  │ PUBLIE sur Pi5      │    │   (loopback    │  │   VRM Portal         │   ││
│  │  │ via IP réseau       │    │   matériel)    │  │                      │   ││
│  │  └─────────────────────┘    │              │  └─────────────────────┘   ││
│  │                             │              │         ▲                  ││
│  │  ┌─────────────────────┐    │   Bridge     │    Tasmota/Shelly          ││
│  │  │ mosquitto DOCKER    │◄───┼──────────────┼──── (WiFi direct)          ││
│  │  │   :1883  :9001      │    │   Docker     │                            ││
│  │  │  Bridge vers NanoPi │────┼──► NanoPi    │                            ││
│  │  └─────────────────────┘    │              │                            ││
│  └─────────────────────────────┘              └─────────────────────────────┘│
└─────────────────────────────────────────────────────────────────────────────┘
```

**Config.toml avant migration** :
```toml
[mqtt]
host = "192.168.1.120"        # ← daly-bms-server publiait sur NanoPi
port = 1883

[energy_manager.mqtt]
host = "192.168.1.141"        # ← energy-manager publiait sur Pi5 via réseau
port = 1883
```

**Config mosquitto.conf Docker (bridge) avant migration** :
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

**Unités systemd avant migration** :
```ini
# contrib/daly-bms.service
[Unit]
After=network.target              # ← AUCUNE dépendance mosquitto

# contrib/energy-manager.service
[Unit]
After=network-online.target mosquitto.service   # ← mosquitto.service INEXISTANT
```

### 10.2 Problèmes identifiés dans l'ancienne architecture

| # | Problème | Fichier | Impact | Sévérité |
|---|----------|---------|--------|----------|
| 1 | **daly-bms-server publie sur NanoPi** | `Config.toml` `[mqtt].host` | Si NanoPi down, BMS ne publie **nulle part** — pas de données locales | Critique |
| 2 | **Double-hop BMS** | `mosquitto.conf` bridge | Pi5 → NanoPi → bridge → Pi5. Latence + fragilité | Majeur |
| 3 | **energy-manager via IP réseau** | `Config.toml` `[energy_manager.mqtt]` | Si WiFi down, MQTT local tombe | Majeur |
| 4 | **Topics en double direction** | `mosquitto.conf` | `santuario/heat/#` en IN + OUT = risque boucle | Majeur |
| 5 | **`shellypro2pm... both 0`** | `mosquitto.conf` | Bidirectionnel = boucle possible | Majeur |
| 6 | **Dépendance `mosquitto.service` inexistante** | `energy-manager.service` | systemd ignore silencieusement | Mineur |
| 7 | **Overhead Docker** | `docker-compose.infra.yml` | ~50-100 Mo RAM, ~10s démarrage | Majeur |

### 10.3 Architecture cible

**Principe directeur** :
> Tout ce qui est produit sur le Pi5 est publié sur le broker LOCAL (127.0.0.1).
> Le bridge ne sert qu'à échanger avec le NanoPi.
> Le NanoPi reste le point d'entrée pour Tasmota/Shelly (WiFi fixe).

L'architecture cible est celle décrite dans les sections 1 à 9 du présent document.

**Gains obtenus** :
- RAM réduite de ~50-100 Mo (suppression Docker Mosquitto)
- Démarrage plus rapide (~10s de moins)
- Résilience : si NanoPi down, le Pi5 continue de stocker localement
- Élimination du double-hop BMS (Pi5 → NanoPi → bridge → Pi5)
- Élimination du risque de boucle `santuario/# both`
- Dépendances systemd correctes (`Requires=mosquitto-broker.service`)

### 10.4 Prérequis et préparation

#### Sur la machine de développement

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

#### Sur le Pi5 (vérifications préalables)

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

#### Inventaire des topics (référence pré-migration)

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

### 10.5 Sauvegarde de l'état actuel

```bash
cd ~/Daly-BMS-Rust

# Copier la config actuelle en backup timestampé
cp docker/mosquitto/config/mosquitto.conf \
   docker/mosquitto/config/mosquitto.conf.bak.$(date +%Y%m%d_%H%M%S)

# Sauvegarder les données retained (messages persistés)
docker exec dalybms-mosquitto cat /mosquitto/data/mosquitto.db \
   > /tmp/mosquitto-retained-backup.db 2>/dev/null || true

# Sauvegarder la config TOML actuelle
cp Config.toml Config.toml.bak.$(date +%Y%m%d_%H%M%S)
```

Créer le répertoire de contribution pour Mosquitto natif :
```bash
mkdir -p contrib/mosquitto

# Copier la config actuelle comme référence
cp docker/mosquitto/config/mosquitto.conf contrib/mosquitto/mosquitto.conf.reference
```

### 10.6 Installation Mosquitto natif

> **IMPORTANT** : Ne PAS supprimer Docker Mosquitto avant que le natif soit prêt.
> Le broker doit rester actif jusqu'à la bascule (étape 5 de §10.7).

```bash
sudo apt update
sudo apt install -y mosquitto mosquitto-clients

# Vérifier l'installation
mosquitto -h | head -n 3
# → doit afficher la version (ex: mosquitto version 2.0.21)

# Empêcher le démarrage automatique immédiat
sudo systemctl stop mosquitto
sudo systemctl disable mosquitto
# (on utilisera notre propre service systemd mosquitto-broker)
```

Créer les répertoires de données :
```bash
sudo mkdir -p /var/lib/mosquitto
sudo mkdir -p /var/log/mosquitto

# Permissions (Mosquitto s'exécute sous l'utilisateur mosquitto)
sudo chown mosquitto:mosquitto /var/lib/mosquitto
sudo chown mosquitto:mosquitto /var/log/mosquitto
sudo chmod 750 /var/lib/mosquitto
sudo chmod 755 /var/log/mosquitto
```

### 10.7 Déploiement pas à pas (référence manuelle)

> **Note** : Toutes les étapes ci-dessous ont été automatisées par
> `contrib/mosquitto/deploy-mosquitto-native.sh`. Voir §10.12 pour le journal
> d'exécution réel. Cette section est conservée comme référence pour exécution manuelle.

#### Étape 0 — Pré-flight (Pi5)

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

#### Étape 1 — Installer Mosquitto natif

```bash
sudo apt update
sudo apt install -y mosquitto mosquitto-clients

# Vérifier
mosquitto -h | head -n 3

# Stopper le service Debian par défaut
sudo systemctl stop mosquitto
sudo systemctl disable mosquitto
```

#### Étape 2 — Créer répertoires et déployer la config

```bash
sudo mkdir -p /var/lib/mosquitto /var/log/mosquitto
sudo chown mosquitto:mosquitto /var/lib/mosquitto /var/log/mosquitto
sudo chmod 750 /var/lib/mosquitto

# Déployer la configuration
sudo cp contrib/mosquitto/mosquitto.conf /etc/mosquitto/mosquitto.conf

# Vérifier la syntaxe (Mosquitto 2.0 — démarrage bref)
sudo -u mosquitto timeout 2 mosquitto -c /etc/mosquitto/mosquitto.conf
# → Erreur de config → message immédiat (Unknown configuration variable, Error found at)
# → "Address already in use" sur 1883/9001 → config OK (Docker tient encore le port)
# → Aucune erreur jusqu'au timeout → config OK
```

#### Étape 3 — Déployer les unités systemd

```bash
sudo cp contrib/mosquitto-broker.service /etc/systemd/system/
sudo cp contrib/daly-bms.service         /etc/systemd/system/
sudo cp contrib/energy-manager.service   /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable mosquitto-broker
```

#### Étape 4 — Mettre à jour Config.toml

```bash
sudo cp Config.toml /etc/daly-bms/config.toml
```

> Ne PAS redémarrer les services maintenant : Docker Mosquitto occupe encore :1883.

#### Étape 5 — Arrêter Docker Mosquitto et démarrer le natif (fenêtre courte ~10-30s)

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

#### Étape 6 — Vérifier le broker local

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

#### Étape 7 — Vérifier les bridges avec NanoPi

```bash
# Bridge INGRESS (NanoPi → Pi5) — Venus OS doit arriver localement
timeout 10 mosquitto_sub -h 127.0.0.1 -p 1883 -t "N/c0619ab9929a/#" -v | head

# Bridge EGRESS (Pi5 → NanoPi) — publication test depuis Pi5
mosquitto_pub -h 127.0.0.1 -p 1883 -t "santuario/bms/test" -m '{"probe":1}' -q 1
ssh root@192.168.1.120 \
  "timeout 5 mosquitto_sub -h localhost -t 'santuario/bms/test' -v -C 1"
# Attendu : santuario/bms/test {"probe":1}
```

#### Étape 8 — Vérifier l'anti-boucle

```bash
# Exécuter le script de vérification
sudo /usr/local/bin/verify-no-loop.sh
# → Attendu : ✅ OK

# Compter les messages santuario/bms/# sur Pi5 pendant 10s
timeout 10 mosquitto_sub -h localhost -p 1883 -t "santuario/bms/#" -v | wc -l
# Attendu : ~4 messages (2 BMS × 2 updates en 10s)
# Si > 50 messages → boucle bridge → vérifier la config
```

#### Étape 9 — Redémarrer les services métier

```bash
sudo systemctl start daly-bms
sleep 5
journalctl -u daly-bms -n 30 --no-pager | grep -iE 'mqtt|connect|error'

sudo systemctl start energy-manager
sleep 5
journalctl -u energy-manager -n 30 --no-pager | grep -iE 'mqtt|connect|error'
```

#### Étape 10 — Vérifier la réduction d'empreinte

```bash
# Comparer la consommation RAM
free -h

# Vérifier que Docker Mosquitto est bien arrêté
docker ps | grep mosquitto   # doit être VIDE

# Vérifier la consommation du process natif
ps aux | grep mosquitto | grep -v grep
# → RSS doit être ~3-5 Mo
```

### 10.8 Vérification flux par flux

#### Checklist VRM (https://vrm.victronenergy.com)

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

#### Checklist Pi5 Dashboard

```
□ Page /dashboard/visualization — tous les nodes actifs (pas de "Hors ligne")
□ Page /dashboard/tasmota — Tongou 1-5 : puissance, tension, courant affichés
□ Page /dashboard/tasmota — commande ON/OFF fonctionne depuis energy-manager
□ Page /dashboard/shelly — Shelly Pro 2PM : données affichées
□ Page /dashboard/bms/1 et /2 — données BMS actualisées
□ Explorateur MQTT WebSocket :9001 — PAS de flood (< 5 msg/s en veille)
□ metrics-store redb — données BMS ET112 présentes (curl :8080/api/v1/redb/series)
```

#### Vérification flux spécifiques

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

#### Logs à surveiller

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

### 10.9 Procédure de rollback

En cas d'échec, retour à Mosquitto Docker en **< 5 minutes** :

```bash
# 1. Arrêter Mosquitto natif
sudo systemctl stop mosquitto-broker
sudo systemctl disable mosquitto-broker

# 2. Restaurer Config.toml avec les anciens hôtes
# Section [mqtt] (daly-bms-server) → 192.168.1.120 (NanoPi)
sudo sed -i '/^\[mqtt\]$/,/^\[/ s/host = "127.0.0.1"/host = "192.168.1.120"/' /etc/daly-bms/config.toml

# Section [energy_manager.mqtt] → 192.168.1.141 (Pi5 via réseau)
sudo sed -i '/^\[energy_manager\.mqtt\]$/,/^\[/ s/host = "127.0.0.1"/host = "192.168.1.141"/' /etc/daly-bms/config.toml

# Vérifier le résultat
grep -A2 '^\[mqtt\]$' /etc/daly-bms/config.toml
grep -A2 '^\[energy_manager\.mqtt\]$' /etc/daly-bms/config.toml

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

### 10.10 Nettoyage post-migration

> **À effectuer UNIQUEMENT après 24h de stabilité confirmée.**

#### Retirer les fichiers Docker Mosquitto

```bash
cd ~/Daly-BMS-Rust

# Archiver (pas supprimer immédiatement)
git mv docker-compose.infra.yml docker-compose.infra.yml.bak.$(date +%Y%m%d)
git mv docker/mosquitto docker/mosquitto.bak.$(date +%Y%m%d)

# Commit
git add -A
git commit -m "chore(infra): archive Docker Mosquitto (migration natif stable)"
```

#### Mettre à jour le Makefile

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

#### Désinstaller Mosquitto natif (si rollback définitif)

```bash
# Si vous décidez de revenir définitivement à Docker
sudo apt remove --purge mosquitto mosquitto-clients
sudo rm -rf /var/lib/mosquitto /var/log/mosquitto
sudo rm /etc/systemd/system/mosquitto-broker.service
sudo systemctl daemon-reload
```

### 10.11 Checklist complète de migration

#### Avant de commencer la migration

```
□ Faire un git commit de tout le code stable
□ Créer un tag git : git tag -a v-pre-mosquitto-native -m "avant migration Mosquitto natif"
□ Copie du dépôt GitHub faite (fork ou clone de secours)
□ Noter l'heure : si problème > 30min → rollback immédiat
□ Vérifier que NanoPi est accessible : ping 192.168.1.120
□ Vérifier le portal_id dans Config.toml (section [energy_manager])
□ Vérifier que mosquitto n'est pas déjà installé nativement
□ Avoir un accès physique ou SSH de secours au Pi5
□ Préparer le document de rollback (§10.9) sous les yeux
```

#### Pendant la migration

```
□ Installer Mosquitto natif AVANT d'arrêter Docker
□ Tester la syntaxe de mosquitto.conf avec démarrage bref
□ Vérifier les bridges avec $SYS/broker/bridge/+/state
□ Exécuter verify-no-loop.sh après chaque modif
□ Tester pub/sub local avant de redémarrer daly-bms
□ Vérifier les bridges (ingress ET egress) avant energy-manager
□ Surveiller les logs en temps réel pendant 5 minutes après chaque démarrage
□ Vérifier VRM : tous les devices "just now"
□ Vérifier dashboard Pi5 : pas de "Hors ligne"
```

#### Après la migration (24h)

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

### 10.12 Journal d'exécution — 12 mai 2026 (migration réussie)

Migration exécutée via `./contrib/mosquitto/deploy-mosquitto-native.sh`
depuis `~/Daly-BMS-Rust` (user `pi5compute@pi5compute`).

#### Sortie console

```text
$ ./contrib/mosquitto/deploy-mosquitto-native.sh
[INFO] Vérifications préalables
[ OK ] Tous les fichiers source présents
===== Plan d'exécution =====
 1. Backups + déploiement /etc/mosquitto/mosquitto.conf
 2. mosquitto -c ... -t (vérif syntaxe)
 3. Installation verify-no-loop.sh -> /usr/local/bin/
 4. Anti-boucle (verify-no-loop.sh)
 5. Installation 3 unités systemd + daemon-reload + enable mosquitto-broker
 6. Déploiement Config.toml -> /etc/daly-bms/config.toml
 7. BASCULE : arrêt daly-bms+energy-manager+Docker -> démarrage natif
 8. Tests pub/sub local + état bridges
 9. Redémarrage daly-bms + energy-manager
Démarrer ? (y/N) y
[INFO] [1/9] Déploiement /etc/mosquitto/mosquitto.conf
[ OK ] Backup /etc/mosquitto/mosquitto.conf.bak.20260512_182350
[ OK ] /etc/mosquitto/mosquitto.conf déployé
[INFO] [2/9] Vérification syntaxe mosquitto.conf
[ OK ] Syntaxe OK
[INFO] [3/9] Installation verify-no-loop.sh -> /usr/local/bin/
[ OK ] /usr/local/bin/verify-no-loop.sh installé
[INFO] [4/9] Vérification anti-boucle
=== Topics EGRESS (out) ===
R/c0619ab9929a/#
W/c0619ab9929a/#
cmnd/#
santuario/bms/#
santuario/heatpump/#
santuario/heat/+/venus
santuario/irradiance/raw
santuario/meteo/venus
santuario/platform/venus
santuario/pvinverter/#
santuario/switch/#
santuario/system/venus
shellypro2pm-ec62608840a4/rpc
=== Topics INGRESS (in) ===
N/c0619ab9929a/#
daly-bms-shelly/rpc
shellypro2pm-ec62608840a4/#
stat/#
tele/#
=== INTERSECTION (DANGER — topics en double) ===
✅ OK : Aucun topic en double. Pas de risque de boucle.
[ OK ] Aucun topic en double IN/OUT
[INFO] [5/9] Déploiement unités systemd
[ OK ] /etc/systemd/system/mosquitto-broker.service
[ OK ] /etc/systemd/system/daly-bms.service
[ OK ] /etc/systemd/system/energy-manager.service
Created symlink '/etc/systemd/system/multi-user.target.wants/mosquitto-broker.service' → '/etc/systemd/system/mosquitto-broker.service'.
[ OK ] daemon-reload + enable mosquitto-broker
[INFO] [6/9] Déploiement Config.toml -> /etc/daly-bms/config.toml
[ OK ] Backup /etc/daly-bms/config.toml.bak.20260512_182350
[ OK ] /etc/daly-bms/config.toml déployé
===== BASCULE — fenêtre indisponible ~10-30s =====
  - Arrêt daly-bms + energy-manager
  - docker compose down (Mosquitto)
  - Démarrage mosquitto-broker.service
  - Redémarrage daly-bms + energy-manager
Basculer maintenant ? (y/N) y
[INFO] [7/9] Arrêt daly-bms + energy-manager (s'ils tournent)
[INFO] Arrêt Docker Mosquitto
[+] down 2/2
 ✔ Container dalybms-mosquitto   Removed                                   0.6s
 ✔ Network daly-bms-rust_default Removed                                   0.2s
[ OK ] Ports 1883/9001 libres
[INFO] Démarrage mosquitto-broker.service
[ OK ] mosquitto-broker actif
[INFO] [8/9] Test pub/sub local
[ OK ] Pub/sub local OK
[INFO] État des bridges (5s d'écoute)
      $SYS/broker/bridge/pi5-to-nanopi/state 1
      $SYS/broker/bridge/nanopi-to-pi5/state 1
[INFO] Trafic NanoPi -> Pi5 (sample 3s sur N/c0619ab9929a/#)
[ OK ] Bridge INGRESS reçoit du NanoPi (119 msg/3s)
[INFO] [9/9] Redémarrage daly-bms + energy-manager
===== État final =====
  mosquitto-broker       active
  daly-bms               active
  energy-manager         active
[ OK ] Aucun conteneur mosquitto actif
[ OK ] Migration appliquée.
```

#### Validation post-migration

| Indicateur | État |
|------------|------|
| `mosquitto.conf` déployé (`/etc/mosquitto/mosquitto.conf`) | OK |
| Syntaxe Mosquitto 2.0.21 valide | OK |
| `verify-no-loop.sh` installé `/usr/local/bin/` | OK |
| Aucun chevauchement IN/OUT (13 OUT, 5 IN, intersection vide) | OK |
| 3 unités systemd installées + `mosquitto-broker` enable | OK |
| `Config.toml` déployé `/etc/daly-bms/config.toml` (backup horodaté) | OK |
| `dalybms-mosquitto` Docker arrêté + réseau supprimé | OK |
| Ports 1883/9001 libérés puis re-bindés par natif | OK |
| `mosquitto-broker.service` actif | OK |
| Pub/sub local 127.0.0.1 fonctionnel | OK |
| Bridge EGRESS `pi5-to-nanopi` état = 1 | OK |
| Bridge INGRESS `nanopi-to-pi5` état = 1 | OK |
| Trafic NanoPi reçu : **119 messages en 3s** sur `N/c0619ab9929a/#` | OK |
| `daly-bms.service` actif après bascule | OK |
| `energy-manager.service` actif après bascule | OK |
| Aucun conteneur `mosquitto` résiduel | OK |

#### Backups conservés (à supprimer après 7j de stabilité confirmée)

```
/etc/mosquitto/mosquitto.conf.bak.20260512_182350
/etc/daly-bms/config.toml.bak.20260512_182350
```

#### Étapes de nettoyage restantes (post-stabilité 24h)

Voir §10.10 — Nettoyage post-migration :
- Archiver `docker-compose.infra.yml` et `docker/mosquitto/`
- Réécrire les targets `make up/down/logs/ps` en wrappers systemctl
- Mettre à jour `CLAUDE.md` (architecture + commandes rapides)

---

## Voir aussi

- [./app-dbus-mqtt-venus.md](./app-dbus-mqtt-venus.md) — Bridge NanoPi : intégration device MQTT → D-Bus Venus OS (zbus), déploiement armv7, services D-Bus enregistrés.
- [./app-daly-bms-server.md](./app-daly-bms-server.md) — Serveur principal Pi5 : RS485, publication MQTT BMS/ET112/ATS/irradiance, AppState, bridges.
- [./app-energy-manager.md](./app-energy-manager.md) — Automatisation énergie Pi5 : modules logic, publication MQTT météo/commandes/baselines, souscription Venus OS.
- [./deploiement-exploitation.md](./deploiement-exploitation.md) — Workflow déploiement complet, systemd, logs rétention.
- [./integration-materiel.md](./integration-materiel.md) — Inventaire RS485/D-Bus, adresses Modbus, instances Victron.
- [./diagnostic-depannage.md](./diagnostic-depannage.md) — Dépannage transverse, problèmes courants, debug MQTT.
- [./ARCHITECTURE.md](./ARCHITECTURE.md) — Document maître : vue d'ensemble système et index de toute la documentation.

## Sources consolidées

Ce document fusionne et **remplace** l'ancien fichier suivant :
`docs/migration-mosquitto-docker-to-native-v2.md`
