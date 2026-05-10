# Plan de Migration — Mosquitto Docker vers Mosquitto Natif

> **Objectif** : Réduire l'empreinte RAM, CPU et disque en supprimant le conteneur Docker Mosquitto au profit d'une installation native (package Debian).
>
> **Date du document** : mai 2026
> **Version Mosquitto cible** : 2.0.x (Debian Bookworm)
> **Hôte** : Raspberry Pi 5 CM (aarch64, Raspberry Pi OS Lite 64-bit)
> **NanoPi** : 192.168.1.120 (Venus OS — inchangé)

---

## Table des matières

1. [Pourquoi cette migration](#1-pourquoi-cette-migration)
2. [Architecture cible](#2-architecture-cible)
3. [Prérequis et préparation](#3-prérequis-et-préparation)
4. [Sauvegarde de l'état actuel](#4-sauvegarde-de-létat-actuel)
5. [Installation Mosquitto natif](#5-installation-mosquitto-natif)
6. [Configuration mosquitto.conf complète](#6-configuration-mosquitto-conf-complète)
7. [Service systemd personnalisé](#7-service-systemd-personnalisé)
8. [Mise à jour des dépendances systemd](#8-mise-à-jour-des-dépendances-systemd)
9. [Mise à jour Config.toml](#9-mise-à-jour-configtoml)
10. [Déploiement pas à pas](#10-déploiement-pas-à-pas)
11. [Vérification et validation](#11-vérification-et-validation)
12. [Anti-boucle — vérification automatique](#12-anti-boucle--vérification-automatique)
13. [Procédure de rollback](#13-procédure-de-rollback)
14. [Nettoyage post-migration](#14-nettoyage-post-migration)
15. [Checklist finale](#15-checklist-finale)

---

## 1. Pourquoi cette migration

### Empreinte actuelle (Docker Mosquitto)

| Ressource | Consommation |
|-----------|-------------|
| Image Docker `eclipse-mosquitto` | ~10 Mo téléchargée |
| Overhead moteur Docker (runtime) | ~50–100 Mo RAM |
| Mosquitto dans conteneur | ~10 Mo RAM |
| Overlayfs + volumes Docker | ~50 Mo disque |
| **Total** | **~70–120 Mo RAM, ~60 Mo disque** |

### Empreinte cible (Mosquitto natif)

| Ressource | Consommation |
|-----------|-------------|
| Binaire `/usr/sbin/mosquitto` | ~200 Ko |
| RAM en fonctionnement | ~3–5 Mo |
| Config + logs + persistence | ~2 Mo |
| **Total** | **~5 Mo RAM, ~2 Mo disque** |

### Gain attendu

- **RAM** : libération de ~50–100 Mo (suppression Docker + conteneur)
- **Disque** : suppression de l'image Docker et des volumes overlay
- **CPU** : moins de context switches (pas de namespace Docker)
- **Démarrage** : plus rapide (pas d'initialisation du runtime container)

### Pourquoi PAS RMQTT ou NanoMQ ?

| Critère | Mosquitto natif | RMQTT | NanoMQ |
|---------|-----------------|-------|--------|
| Binaire | 200 Ko | ~15 Mo | ~500 Ko |
| RAM idle | ~3 Mo | ~20 Mo | ~3 Mo |
| Maturité production | ⭐⭐⭐⭐⭐ (10+ ans) | ⭐⭐⭐ (jeune) | ⭐⭐⭐⭐ |
| Bridge natif | ✅ Oui | ✅ Plugin | ✅ Plugin |
| Package Debian | ✅ `apt install` | ❌ Compilation manuelle | ❌ Binaire GitHub |
| Config existante | ✅ Réutilisable | ❌ À réécrire | ❌ À réécrire |
| Maintenance | `apt upgrade` | Manuelle | Manuelle |

> **Verdict** : Mosquitto natif offre le meilleur rapport gain/risque pour un système de production énergétique critique.

---

## 2. Architecture cible

```
Pi5 (192.168.1.141)
  mosquitto (systemd, natif)
    ├── TCP  :1883  ← daly-bms-server, energy-manager (localhost)
    ├── WS   :9001  ← explorateur dashboard JS
    ├── Persistence /var/lib/mosquitto/ (retained messages)
    ├── Bridge EGRESS : Pi5 → NanoPi (192.168.1.120:1883)
    │     santuario/#, W/{portal}/#, R/{portal}/#, cmnd/#,
    │     shellypro2pm-ec62608840a4/rpc
    └── Bridge INGRESS : NanoPi → Pi5
          N/{portal}/#, tele/#, stat/#,
          shellypro2pm-ec62608840a4/#, daly-bms-shelly/rpc

  daly-bms-server (systemd, :8080)
    ├── RS485 → publie santuario/* sur localhost:1883
    └── subscribe tele/+/SENSOR, stat/+/POWER, shellypro2pm.../status/*

  energy-manager (systemd, :8081)
    └── subscribe/publish santuario/* sur localhost:1883

NanoPi (192.168.1.120)
  Mosquitto (existant, natif Venus OS) :1883
    └── dbus-mqtt-venus subscribe santuario/* → D-Bus Victron
```

### Flux de données (inchangés vs. Docker)

```
BMS Daly (RS485)
  → daly-bms-server lit les données
  → publie santuario/bms/1/venus sur localhost:1883 (mosquitto natif)
  → bridge EGRESS forward vers NanoPi:1883
  → dbus-mqtt-venus subscribe → D-Bus → VRM

Tongou (sur NanoPi)
  → publie tele/tongou_3BC764/SENSOR
  → bridge INGRESS ramène sur Pi5 localhost:1883
  → daly-bms-server subscribe → dashboard
```

---

## 3. Prérequis et préparation

### Sur la machine de développement

```bash
cd ~/Daly-BMS-Rust
git checkout main  # ou la branche de production
git pull

# Créer un tag de l'état stable avant migration
git tag -a v-pre-mosquitto-native -m "État stable avant migration Mosquitto natif"
git push origin v-pre-mosquitto-native

# Vérifier le portal_id (nécessaire pour la config bridge)
grep '^portal_id' Config.toml
# → doit afficher : portal_id = "c0619ab9929a"
```

### Sur le Pi5 (vérifications préalables)

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
# S'assurer d'avoir au moins 500 Mo libres (marge de sécurité)

# Vérifier la connectivité NanoPi
ping -c 3 192.168.1.120
ssh root@192.168.1.120 "systemctl is-active mosquitto"
```

### Inventaire des topics bridge (à ne PAS modifier)

> Ces topics sont identiques à la configuration Docker actuelle. La migration ne change que le transport (Docker → natif), pas la logique de routage.

**EGRESS (Pi5 → NanoPi)** :
```
santuario/bms/#
santuario/pvinverter/#
santuario/heatpump/#
santuario/heat/#
santuario/switch/#
santuario/grid/#
santuario/meteo/#
santuario/platform/#
santuario/inverter/#
santuario/system/#
W/c0619ab9929a/#
R/c0619ab9929a/#
cmnd/#
shellypro2pm-ec62608840a4/rpc
```

**INGRESS (NanoPi → Pi5)** :
```
N/c0619ab9929a/#
tele/#
stat/#
shellypro2pm-ec62608840a4/#
daly-bms-shelly/rpc
```

---

## 4. Sauvegarde de l'état actuel

### Sauvegarder la config Docker actuelle

```bash
cd ~/Daly-BMS-Rust

# Copier la config actuelle en backup
cp docker/mosquitto/config/mosquitto.conf docker/mosquitto/config/mosquitto.conf.bak.$(date +%Y%m%d_%H%M%S)

# Sauvegarder les données retained (si pertinent)
docker exec dalybms-mosquitto cat /mosquitto/data/mosquitto.db > /tmp/mosquitto-retained-backup.db 2>/dev/null || true

# Noter les variables d'environnement Docker
cat .env | grep -i mosquitto > /tmp/mosquitto-env-backup.txt 2>/dev/null || true
```

### Sauvegarder les fichiers de versionnement

```bash
# Créer le répertoire de contribution pour Mosquitto natif
mkdir -p contrib/mosquitto

# Copier la config actuelle comme référence
cp docker/mosquitto/config/mosquitto.conf contrib/mosquitto/mosquitto.conf.reference
```

---

## 5. Installation Mosquitto natif

### Étape 5.1 — Supprimer Docker Mosquitto (PAS encore)

> **IMPORTANT** : Ne PAS supprimer Docker Mosquitto maintenant. Le broker doit rester actif jusqu'à ce que le natif soit prêt.

### Étape 5.2 — Installer les packages Debian

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

### Étape 5.3 — Créer les répertoires de données

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

## 6. Configuration mosquitto.conf complète

Créer `/etc/mosquitto/mosquitto.conf` :

```conf
# =============================================================================
# mosquitto.conf — Broker natif Pi5 (remplace Docker Mosquitto)
# Version : 2.0.x (Debian Bookworm)
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
# ⚠ RÈGLE ANTI-BOUCLE : Ces topics ne doivent PAS être dans le bridge INGRESS.
#   Tous les topics santuario/* viennent de Pi5, JAMAIS de NanoPi.
# =============================================================================
connection nanopi-egress
address 192.168.1.120:1883
bridge_protocol_version mqttv311
# Démarrage automatique
start_type automatic
# Notification de l'état du bridge
notifications true
notification_topic $SYS/broker/bridge/nanopi-egress/state
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

# Température extérieure → temperature.mqtt_1
topic santuario/heat/# out 0 "" ""

# ATS CHINT + Tongou → switch.mqtt_1/2/3/4/5/6
topic santuario/switch/# out 0 "" ""

# Compteurs réseau → grid.mqtt_n
topic santuario/grid/# out 0 "" ""

# Irradiance PRALRAN → meteo
topic santuario/meteo/# out 0 "" ""

# Platform Pi5, Inverter, System
topic santuario/platform/# out 0 "" ""
topic santuario/inverter/# out 0 "" ""
topic santuario/system/# out 0 "" ""

# Venus OS commandes (écriture D-Bus) — QoS 1
topic W/c0619ab9929a/# out 1 "" ""

# Venus OS keepalive (lecture D-Bus) — QoS 1
topic R/c0619ab9929a/# out 1 "" ""

# Commandes ON/OFF Tongou depuis Pi5 web
topic cmnd/# out 1 "" ""

# Commandes RPC Shelly (Pi5 → Shelly via NanoPi)
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
# ⚠ RÈGLE ANTI-BOUCLE : Ces topics doivent être ABSENTS du bridge EGRESS.
#   NanoPi publie : N/{portal}/#, tele/#, stat/#, shellypro2pm/#, daly-bms-shelly/rpc
#   Pi5 publie   : santuario/*, cmnd/*, W/*, R/*, shellypro2pm.../rpc
# =============================================================================
connection nanopi-ingress
address 192.168.1.120:1883
bridge_protocol_version mqttv311
start_type automatic
notifications true
notification_topic $SYS/broker/bridge/nanopi-ingress/state
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

> **Note** : Remplacer `c0619ab9929a` par le `portal_id` réel du Cerbo GX. Vérifier avec :
> ```bash
> grep '^portal_id' /etc/daly-bms/config.toml
> # ou côté NanoPi :
> ssh root@192.168.1.120 "mosquitto_sub -h localhost -t 'N/+/system/0/Serial' -C 1 -W 5"
> ```

---

## 7. Service systemd personnalisé

Créer `contrib/mosquitto-broker.service` (versionné dans le dépôt) :

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
> - Utilise `ProtectSystem=strict` (Mosquitto est audité depuis des années, contrairement à RMQTT)

---

## 8. Mise à jour des dépendances systemd

### 8.1 Patch `contrib/daly-bms.service`

Modifier l'en-tête pour dépendre du nouveau broker natif :

```ini
[Unit]
Description=DalyBMS Server — Rust RS485 BMS monitor
Documentation=https://github.com/thieryus007-cloud/Daly-BMS-Rust
After=network.target mosquitto-broker.service
Wants=network.target
Requires=mosquitto-broker.service
```

### 8.2 Patch `contrib/energy-manager.service`

Remplacer la référence à Docker/Mosquitto par le service natif :

```ini
[Unit]
Description=Energy Manager — Gestionnaire d'énergie Rust (remplace Node-RED)
Documentation=https://github.com/thieryus007-cloud/Daly-BMS-Rust
After=network-online.target mosquitto-broker.service daly-bms.service
Wants=network-online.target
Requires=mosquitto-broker.service
PartOf=daly-bms.service
```

### 8.3 Déploiement des unités

```bash
sudo cp contrib/mosquitto-broker.service /etc/systemd/system/
sudo cp contrib/daly-bms.service         /etc/systemd/system/
sudo cp contrib/energy-manager.service   /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable mosquitto-broker
```

---

## 9. Mise à jour Config.toml Pi5

### 9.1 Section `[mqtt]` (daly-bms-server) — ligne ~74

**Avant :**
```toml
[mqtt]
enabled = true
host = "192.168.1.120"          # ← NanoPi (via bridge Docker)
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

### 9.2 Section `[energy_manager.mqtt]` — ligne ~528

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

### 9.3 Déployer

```bash
# Commit + push depuis machine de dev
git add Config.toml contrib/mosquitto-broker.service contrib/daly-bms.service contrib/energy-manager.service
git commit -m "chore(config): broker MQTT local 127.0.0.1 (migration Mosquitto natif)"
git push

# Sur le Pi5
cd ~/Daly-BMS-Rust && make sync
sudo cp Config.toml /etc/daly-bms/config.toml
```

---

## 10. Déploiement pas à pas

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

### Étape 4 — Mettre à jour Config.toml (cf. §9)

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
ss -tlnp | grep -E ':(1883|9001)'
# ← doit être VIDE

# Démarrer Mosquitto natif
sudo systemctl start mosquitto-broker
sleep 3

# Vérification
systemctl status mosquitto-broker --no-pager
journalctl -u mosquitto-broker -n 30 --no-pager
ss -tlnp | grep -E ':(1883|9001)'   # 2 lignes attendues
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
# → doit afficher "Connecting bridge nanopi-egress..." et "Connecting bridge nanopi-ingress..."

# Vérifier l'état des bridges via topics système
timeout 5 mosquitto_sub -h 127.0.0.1 -t '$SYS/broker/bridge/+/state' -v -C 2
# → Attendu : $SYS/broker/bridge/nanopi-egress/state 1
#             $SYS/broker/bridge/nanopi-ingress/state 1
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

### Étape 8 — Redémarrer les services métier

```bash
sudo systemctl start daly-bms
sleep 5
journalctl -u daly-bms -n 30 --no-pager | grep -iE 'mqtt|connect|error'

sudo systemctl start energy-manager
sleep 5
journalctl -u energy-manager -n 30 --no-pager | grep -iE 'mqtt|connect|error'
```

### Étape 9 — Vérifier la réduction d'empreinte

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

## 11. Vérification et validation

### 11.1 Checklist VRM (https://vrm.victronenergy.com)

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

### 11.2 Checklist Pi5 Dashboard

```
□ Page /dashboard/visualization — tous les nodes actifs (pas de "Hors ligne")
□ Page /dashboard/tasmota — Tongou 1-5 : puissance, tension, courant affichés
□ Page /dashboard/tasmota — commande ON/OFF fonctionne
□ Page /dashboard/shelly — Shelly Pro 2PM : données affichées
□ Page /dashboard/bms/1 et /2 — données BMS actualisées
□ Explorateur MQTT WebSocket :9001 — PAS de flood (< 5 msg/s en veille)
```

### 11.3 Vérifier l'absence de boucle

```bash
# Compter les messages santuario/bms/# sur Pi5 pendant 10s
timeout 10 mosquitto_sub -h localhost -p 1883 -t "santuario/bms/#" -v | wc -l
# Attendu : ~4 messages (2 BMS × 2 updates en 10s)
# Si > 50 messages → boucle bridge → vérifier la config (topics en double in/out)
```

### 11.4 Logs à surveiller

```bash
# Broker Mosquitto natif
journalctl -u mosquitto-broker -f

# BMS server
journalctl -u daly-bms -f | grep -E "MQTT|bms|error"

# NanoPi dbus-mqtt-venus
ssh root@192.168.1.120 "tail -f /var/log/dbus-mqtt-venus/current"
```

---

## 12. Anti-boucle — vérification automatique

### Script de vérification (à exécuter après chaque modification de config)

Créer `contrib/mosquitto/verify-no-loop.sh` :

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

### Utilisation

```bash
chmod +x contrib/mosquitto/verify-no-loop.sh
sudo cp contrib/mosquitto/verify-no-loop.sh /usr/local/bin/

# Vérifier après chaque modification
sudo /usr/local/bin/verify-no-loop.sh
```

---

## 13. Procédure de rollback

En cas d'échec, retour à Mosquitto Docker en **< 5 minutes** :

```bash
# 1. Arrêter Mosquitto natif
sudo systemctl stop mosquitto-broker
sudo systemctl disable mosquitto-broker

# 2. Relancer Docker Mosquitto
cd ~/Daly-BMS-Rust
docker compose -f docker-compose.infra.yml up -d

# 3. Vérifier Mosquitto Docker actif
docker ps | grep mosquitto
ss -tlnp | grep 1883

# 4. Restaurer Config.toml avec les anciens hôtes
# (si modifié — sinon les services se reconnectent automatiquement)
sudo sed -i 's/host = "127.0.0.1"/host = "192.168.1.120"/' /etc/daly-bms/config.toml

# 5. Redémarrer les services
sudo systemctl restart daly-bms energy-manager

# 6. Vérifier
systemctl status daly-bms energy-manager
```

---

## 14. Nettoyage post-migration

> **À effectuer UNIQUEMENT après 24h de stabilité confirmée.**

### 14.1 Retirer les fichiers Docker Mosquitto

```bash
cd ~/Daly-BMS-Rust

# Archiver (pas supprimer immédiatement)
git mv docker-compose.infra.yml docker-compose.infra.yml.bak.$(date +%Y%m%d)
git mv docker/mosquitto docker/mosquitto.bak.$(date +%Y%m%d)

# Commit
git add -A
git commit -m "chore(infra): archive Docker Mosquitto (migration natif)"
```

### 14.2 Mettre à jour le Makefile

Les targets `make up` / `make down` / `make logs` / `make ps` deviennent obsolètes. Deux options :

**Option A (recommandée)** — Remplacer par des wrappers `systemctl` :

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

**Option B** — Supprimer les targets obsolètes et documenter dans `CLAUDE.md`.

### 14.3 Mettre à jour `CLAUDE.md`

Sections à modifier :
- Section 1 (Architecture) : remplacer `Docker: mosquitto:1883` par `systemd: mosquitto-broker.service (1883/9001)`.
- Section 0 (Commandes rapides) : remplacer `make up / make down` par les nouvelles commandes.
- Section 8 (Problèmes courants) : ajouter `journalctl -u mosquitto-broker -n 50`.

### 14.4 Désinstaller Mosquitto natif (si rollback définitif)

```bash
# Si vous décidez de revenir définitivement à Docker
sudo apt remove --purge mosquitto mosquitto-clients
sudo rm -rf /var/lib/mosquitto /var/log/mosquitto
sudo rm /etc/systemd/system/mosquitto-broker.service
sudo systemctl daemon-reload
```

---

## 15. Checklist finale

### Avant de commencer la migration

```
□ Faire un git commit de tout le code stable
□ Créer un tag git : git tag -a v-pre-mosquitto-native -m "avant migration Mosquitto natif"
□ Noter l'heure : si problème > 30min → rollback immédiat
□ Vérifier que NanoPi est accessible : ping 192.168.1.120
□ Vérifier le portal_id dans Config.toml (section [energy_manager])
□ Vérifier que mosquitto n'est pas déjà installé nativement
□ Avoir un accès physique ou SSH de secours au Pi5
```

### Pendant la migration

```
□ Installer Mosquitto natif AVANT d'arrêter Docker
□ Tester la syntaxe de mosquitto.conf avec -t
□ Vérifier les bridges avec $SYS/broker/bridge/+/state
□ Tester pub/sub local avant de redémarrer daly-bms
□ Vérifier les bridges (ingress ET egress) avant energy-manager
□ Surveiller les logs en temps réel pendant 5 minutes après chaque démarrage
□ Exécuter verify-no-loop.sh après chaque modif de config
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
```

---

## Références

- [Documentation Mosquitto](https://mosquitto.org/documentation/)
- [Configuration bridges](https://mosquitto.org/man/mosquitto-conf-5.html)
- [Man page mosquitto](https://mosquitto.org/man/mosquitto-8.html)
- [Debian package mosquitto](https://packages.debian.org/bookworm/mosquitto)
- [Guide anti-boucle MQTT bridges](https://mosquitto.org/man/mosquitto-conf-5.html#idm459)
