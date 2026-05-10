# Daly-BMS — Rust Edition

**Version Rust complète** — mise à jour mai 2026
Remplacement total de la stack Python/FastAPI **et** des flows Node-RED par **Rust**
(workspace multi-crates : `rs485-bus` + `daly-bms-core` + `daly-bms-server` +
`energy-manager` + `dbus-mqtt-venus`).

> Dashboard intégré **SSR Rust** (Askama + ECharts) — aucun npm.
> Infrastructure Docker **inchangée** (Mosquitto, VictoriaMetrics ).
> Déploiement ultra-léger : **un seul binaire statique** (~12–18 Mo).
> Compatible **Windows** (testé) et **Linux/aarch64** (Raspberry Pi).

---

**Matériel de production** : Raspberry Pi Compute Module 5 Wireless, 4 Go RAM, 32 Go eMMC, Raspberry Pi OS Lite (64-bit)

---

## Architecture globale

### Vue d'ensemble infrastructure

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Raspberry Pi 5 CM  (Master)                      │
│                                                                     │
│  RS485 /dev/ttyUSB0 ── BMS-360Ah  (0x01)                            │
│                     ── BMS-320Ah  (0x02)                            │
│                     ── ET112 PV   (0x07) — Micro-Onduleurs          │
│                     ── ET112 House (0x08) — Consommation maison    │
│                     ── ET112 Grid  (0x09) — Réseau                 │
│                     ── PRALRAN     (0x05) — Irradiance / Météo     │
│                     ── ATS CHINT   (RS485) — bascule grid/inverter │
│                                  │                                  │
│                                  ▼                                  │
│                          daly-bms-server :8080                      │
│                          (REST/WS + Dashboard SSR + AlertEngine)    │
│                                  │                                  │
│  API LG ThinQ ──► energy-manager :8081 (PAC chauffe-eau / Deye)    │
│                                  │                                  │
│                          Mosquitto :1883                            │
│                                  │                                  │
│                                │                                    │
│                       VictoriaMetrics :8428                         │
└────────────────────────────────┬────────────────────────────────────┘
                                 │ MQTT (santuario/*/venus)
                                 ▼
┌────────────────────────────────────────────────────────────────────┐
│                  NanoPi Neo3  (Venus OS — D-Bus bridge)            │
│                                                                    │
│   dbus-mqtt-venus  (binaire unique, zbus pur Rust)                 │
│      ▲                                                             │
│      │  Souscrit à : santuario/bms/{n}/venus                       │
│      │               santuario/heatpump/{n}/venus                  │
│      │               santuario/pvinverter/{n}/venus                │
│      │               santuario/heat/{n}/venus                      │
│      │               santuario/switch/{n}/venus                    │
│      │               santuario/meteo/venus                         │
│      ▼                                                             │
│   com.victronenergy.battery.mqtt_*    (BMS × 2)         ✅         │
│   com.victronenergy.pvinverter.mqtt_* (ET112 PV)        ✅         │
│   com.victronenergy.heatpump.mqtt_*   (ET112 House+Grid) ✅        │
│   com.victronenergy.temperature.mqtt_* (capteurs)        ✅        │
│   com.victronenergy.switch.mqtt_*     (ATS + Tongou)     ✅        │
│   com.victronenergy.meteo             (PRALRAN)          ✅        │
│                │                                                   │
│                ▼                                                   │
│   systemcalc-py ── VRM Portal ── Venus GUI                         │
│   hub4-control  (DVCC charge/discharge)                            │
└────────────────────────────────────────────────────────────────────┘
```

> **Adresses BMS production** : `0x01` (BMS-360Ah) et `0x02` (BMS-320Ah)
> **Validé en production** sur RPi5 au 17 mars 2026 — données confirmées dans VictoriaMetrics.
> **Pi5 = master** : tous les capteurs RS485 y sont connectés, le NanoPi reste dédié Venus OS / D-Bus.

### Flux MQTT par domaine (préfixe `santuario/`)

| Topic MQTT | Source (Pi5) | Bridge NanoPi | Cible D-Bus Venus |
|---|---|---|---|
| `bms/{n}/venus` | `daly-bms-server` ✅ | `dbus-mqtt-venus` ✅ | `com.victronenergy.battery.mqtt_{n}` |
| `pvinverter/{n}/venus` | `daly-bms-server` (ET112) ✅ | `dbus-mqtt-venus` ✅ | `com.victronenergy.pvinverter.mqtt_{n}` |
| `heatpump/{n}/venus` | `daly-bms-server` (ET112) ✅ | `dbus-mqtt-venus` ✅ | `com.victronenergy.heatpump.mqtt_{n}` |
| `heat/{n}/venus` | `energy-manager` (LG ThinQ) ✅ | `dbus-mqtt-venus` ✅ | `com.victronenergy.temperature.mqtt_{n}` |
| `switch/{n}/venus` | `daly-bms-server` (ATS / Tongou) ✅ | `dbus-mqtt-venus` ✅ | `com.victronenergy.switch.mqtt_{n}` |
| `meteo/venus` | `daly-bms-server` (PRALRAN) ✅ | `dbus-mqtt-venus` ✅ | `com.victronenergy.meteo` |

> `dbus-mqtt-venus` est le **seul binaire sur le NanoPi** — il souscrit à tous les topics
> et enregistre tous les services D-Bus. Un seul processus, ~5–8 Mo RAM.

### Rôle de chaque service

| Service | Hôte | Port | Rôle |
|---------|------|------|------|
| **daly-bms-server** | Pi5 | 8080 | Serveur principal Rust : polling RS485, REST API, WebSocket, Dashboard SSR |
| **Mosquitto** | Pi5 | 1883 (MQTT), 9001 (WS) | Broker MQTT — relaye toutes les données capteurs vers Venus OS et energy-manager |
| **VictoriaMetrics** | Pi5 | 8428 | Base de données séries temporelles — stockage 30 jours de métriques |
| **energy-manager** | Pi5 | 8081 | Automatisation — flows MQTT, alertes, webhooks (migré NanoPi → Pi5) |
| **dbus-mqtt-venus** | NanoPi | — | Bridge MQTT → D-Bus Venus OS (Rust pur, zbus) — unique binaire sur NanoPi, enregistre tous les capteurs sur Venus |

> **Note architecture** : Le Pi5 est le **master** de tous les capteurs RS485 et API cloud.
> Le NanoPi reste dédié à Venus OS et héberge uniquement `dbus-mqtt-venus` (Rust statique musl, ~5 Mo).
> energy-manager a été migré du NanoPi vers le Pi5 pour consolider l'infrastructure.

---

## Flux de données détaillé

### BMS (implémenté)

```
BMS UART  ──► daly_bms_core::poll_loop()   ← mode hardware (RS485/USB)
                    │
                    ▼  on_snapshot(snap)
             AppState::on_snapshot()
              ┌──────┴──────────────────────────┐
              ▼                                 ▼
       ring_buffer                      broadcast (tokio)
       (3600 snaps/BMS)         ┌────────┬──────┴──────┬───────────┐
                                ▼        ▼              ▼           ▼
                           MqttBridge              AlertEngine WebSocket
                           (rumqttc)  (VictoriaMetrics2)  (rusqlite)  (/ws/bms/*)
                               │           │
                               ▼           ▼
                          Mosquitto     VictoriaMetrics
                               │           │
             dbus-mqtt-venus   
             com.victronenergy.battery.*
                  (Venus OS / NanoPi)
```

### Capteurs additionnels (implémentés)

```
ET112 Modbus RTU (0x07/0x08/0x09) ──► daly-bms-server::et112::poll_loop()
                                          │
                                    MqttBridge ──► santuario/pvinverter/{n}/venus
                                                  santuario/heatpump/{n}/venus
                                          │
                                    dbus-mqtt-venus (pvinverter / heatpump services)

PRALRAN RS485 (0x05)              ──► daly-bms-server::irradiance::poll_loop()
                                          │
                                    MqttBridge ──► santuario/meteo/venus
                                          │
                                    dbus-mqtt-venus (meteo service)

LG ThinQ API (PAC chauffe-eau)    ──► energy-manager::http_clients::lg_thinq
                                          │
                                    MqttBridge ──► santuario/heat/{n}/venus
                                          │
                                    dbus-mqtt-venus (temperature service)

ATS CHINT RS485                   ──► daly-bms-server::ats (lecture + commandes)
                                          │
                                    MqttBridge ──► santuario/switch/{n}/venus
                                          │
                                    dbus-mqtt-venus (switch service)
```

---

## Workspace Rust

### Crates du workspace

| Crate / Binaire | Hôte | Statut | Rôle |
|---|---|---|---|
| `rs485-bus` | — | ✅ Production | Lib : bus RS485 partagé (mutex tokio) + Modbus RTU pur Rust |
| `daly-bms-core` | — | ✅ Production | Lib : protocole UART Daly, parsing trames, types (`BmsSnapshot`), polling |
| `daly-bms-server` | Pi5 | ✅ Production | Binaire Pi5 : API Axum (REST + WebSocket) + Dashboard SSR + bridges (MQTT, VictoriaMetrics, AlertEngine SQLite) — gère BMS, ET112, ATS, irradiance, Tasmota, Shelly |
| `energy-manager` | Pi5 | ✅ Production | Binaire Pi5 (port 8081) — automatisation énergie via `rust-rule-engine` (charge_current, deye_command, inverter, irradiance, smartshunt, solar_power, water_heater, switch_ats, victron_keepalive) + clients HTTP Open-Meteo et LG ThinQ |
| `dbus-mqtt-venus` | NanoPi (armv7) | ✅ Production | Binaire NanoPi : MQTT → D-Bus Venus OS (zbus pur Rust) — services `battery`, `pvinverter`, `heatpump`, `temperature`, `switch`, `meteo`, `platform` |

---

## Structure du dépôt

```
Daly-BMS-Rust/
├── Cargo.toml                 ← Workspace Rust (résolver v2, Rust 1.88+)
├── Cargo.lock
├── Config.toml                ← Configuration principale (TOML)
├── Makefile                   ← Commandes build/test/deploy
├── docker-compose.infra.yml   ← Infra Mosquitto (broker MQTT)
├── .env.docker                ← Template variables d'environnement (à copier en .env)
├── .gitignore
│
├── crates/
│   ├── rs485-bus/                ← Bus RS485 partagé + Modbus RTU pur Rust
│   │   └── src/{lib.rs, modbus_rtu.rs}
│   │
│   ├── daly-bms-core/            ← Bibliothèque protocole Daly
│   │   └── src/{lib.rs, error.rs, types.rs, protocol.rs, bus.rs,
│   │            commands.rs, write.rs, poll.rs}
│   │
│   ├── daly-bms-server/          ← Serveur principal Pi5 (REST/WS + Dashboard SSR)
│   │   ├── src/
│   │   │   ├── main.rs, config.rs, state.rs, autodetect.rs,
│   │   │   ├── monitor.rs, console.rs, vm_client.rs
│   │   │   ├── api/              ← Router Axum : bms, system, ats, et112,
│   │   │   │                       chart, history, alerts, tasmota, shelly,
│   │   │   │                       promql, console, health
│   │   │   ├── ats/              ← ATS CHINT (RS485 + commandes)
│   │   │   ├── et112/            ← Compteurs énergie ET112 (Modbus RTU)
│   │   │   ├── irradiance/       ← Capteur PRALRAN (RS485)
│   │   │   ├── tasmota/, shelly/ ← Capteurs / relais MQTT
│   │   │   ├── bridges/          ← MQTT publisher + AlertEngine SQLite
│   │   │   └── dashboard/        ← Routes SSR + génération ECharts
│   │   └── templates/            ← Templates Askama (.html) compilés dans le binaire
│   │
│   ├── energy-manager/           ← Automatisation Rust (remplace Node-RED)
│   │   ├── src/
│   │   │   ├── main.rs, config.rs, types.rs, bus.rs, monitoring.rs
│   │   │   ├── mqtt/             ← Client rumqttc + topics
│   │   │   ├── http_clients/     ← Open-Meteo + LG ThinQ
│   │   │   ├── live_ws/          ← WebSocket debug live
│   │   │   ├── persist/          ← Restauration baselines
│   │   │   └── logic/            ← 12 modules de décision
│   │   │       (charge_current, deye_command, inverter, irradiance,
│   │   │        meteo, platform, smartshunt, solar_power, switch_ats,
│   │   │        tasmota, victron_keepalive, water_heater)
│   │   └── rules/                ← Fichiers `.grl` (rust-rule-engine)
│   │
│   ├── dbus-mqtt-venus/          ← Binaire NanoPi : MQTT → D-Bus Venus OS
│   │   └── src/
│   │       ├── main.rs, config.rs, types.rs, manager.rs, mqtt_source.rs,
│   │       ├── battery_service.rs / battery_manager.rs
│   │       ├── pvinverter_service.rs / pvinverter_manager.rs
│   │       ├── heatpump_service.rs / heatpump_manager.rs
│   │       ├── temperature_service.rs
│   │       ├── switch_service.rs / switch_manager.rs
│   │       ├── meteo_service.rs / meteo_manager.rs
│   │       ├── grid_service.rs / grid_manager.rs
│   │       ├── platform_service.rs / platform_manager.rs
│   │       └── sensor_manager.rs
│   │
│
├── contrib/
│   ├── daly-bms.service          ← Unité systemd daly-bms-server
│   ├── energy-manager.service    ← Unité systemd energy-manager
│   ├── node-exporter.service     ← Unité systemd Prometheus node_exporter
│   ├── victoriametrics-scrape.yml← Config scrape Prometheus pour VM
│   ├── install-systemd.sh        ← Script d'installation systemd
│   └── uninstall-systemd.sh      ← Script de désinstallation
├── docker/
│   └── mosquitto/config/mosquitto.conf
├── docs/                         ← Guides détaillés (energy-manager, ATS, Venus, etc.)
└── nanoPi/                       ← Config dbus-mqtt-venus (Venus OS)
    ├── config-nanopi.toml        ← Config production dbus-mqtt-venus
    ├── install-venus.sh          ← Script de déploiement
    ├── cleanup-dbus-serialbattery.sh
    ├── sv/                       ← Templates runit
    └── README.md                 ← Guide installation Venus OS
```

### Estimation mémoire

#### Pi5 (master — Docker).  mesure réelle: 20%

| Service                    | RAM minimale | RAM confortable |
|----------------------------|-------------|-----------------|
| daly-bms-server (Rust)     | ~25 MB      | ~50 MB          |
| energy-manager (Rust)      | ~30 MB      | ~100 MB (LimitMemoryMax=100M) |
| Mosquitto                  | ~12 MB      | ~20 MB          |
| VictoriaMetrics 2.x (Go)   | ~150 MB     | ~350 MB         |
| OS Raspberry Pi OS Lite    | ~150 MB     | ~200 MB         |
| Docker Engine + overhead   | ~100 MB     | ~150 MB         |
| Marge tampon / cache       | ~200 MB     | ~400 MB         |
| **TOTAL**                  | **~667 MB** | **~1270 MB**    |

#### NanoPi Neo3 (Venus OS — services Rust statiques)

| Service                    | RAM minimale | Notes |
|----------------------------|-------------|-------|
| dbus-mqtt-venus     | ~5–8 MB     | Binaire statique musl, zéro dépendance système |
| Venus OS + systemcalc-py   | ~150 MB     | Existant |
| **TOTAL ajouté**           | **~5 MB**   | Impact négligeable |

---

## Prérequis

| Composant | Version | Usage |
|-----------|---------|-------|
| Rust      | 1.80+   | Compilation |
| Docker    | 24+     | Infra MQTT |
| Docker Compose v2 | — | `make up` |
| cross     | dernière | Cross-compilation ARM (optionnel) |

> Le dashboard est **SSR (Askama + ECharts)** — Node.js/npm ne sont plus nécessaires.

**Matériel** : Raspberry Pi CM5 (ou Pi 4/5) + adaptateur USB/RS485
**OS** : Debian Bookworm / Ubuntu 24.04 (aarch64 ou x86_64), **Windows 10/11 supporté**
**Permissions Linux** : `sudo usermod -aG dialout $USER`

### Compatibilité multi-plateforme

| Plateforme | Statut | Notes |
|---|---|---|
| Windows 10/11 (x86_64) | ✅ Testé | Port COMx, auto-détection |
| Linux x86_64 | ✅ Compilé | `/dev/ttyUSB0` |
| Raspberry Pi 5 / CM5 (aarch64) | ✅ Validé production | Cross-compile ou natif |
| Cerbo GX / NanoPi Venus OS | N/A | Sert le MQTT, ne fait pas tourner le serveur |

---

## Démarrage rapide

### Infrastructure Docker (broker MQTT)

```bash
cp .env.docker .env            # adapter les tokens et mots de passe
make up                        # Mosquitto:1883
make ps                        # vérifier l'état des containers
```

### Configuration

```bash
sudo mkdir -p /etc/daly-bms
sudo cp Config.toml /etc/daly-bms/config.toml
sudo nano /etc/daly-bms/config.toml   # adapter port série + adresses BMS
```

### Compilation et Lancement (hardware réel)

```bash
# Développement (local)
make run-debug

# Production sur le Pi (cross-compile)
make build-arm
make deploy PI_HOST=pi@192.168.1.141
```

### Service systemd (Linux/RPi5)

```bash
make install        # copie le binaire + installe daly-bms.service
journalctl -u daly-bms -f
```

### Infrastructure broker MQTT

```bash
make up   # démarre Mosquitto via docker-compose.infra.yml
```

---

## Dashboard intégré

Le dashboard est **embarqué dans le binaire** (SSR Askama + ECharts). Aucun npm, aucun serveur web séparé.

| URL | Description |
|-----|-------------|
| `http://localhost:8080/dashboard` | Vue synthèse de tous les BMS |
| `http://localhost:8080/dashboard/bms/1` | Détail BMS (cellules, températures, historique) |

**Fonctionnalités :**
- Cartes par BMS : SOC, tension, courant, température, puissance
- Boxplot tensions cellules (min/max/avg) avec colorisation
- Indicateur équilibrage actif (cellules hautes/basses)
- Profil températures
- Historique temps réel (ring buffer 3600 snapshots)
- Thème clair, badge RS485 multi-BMS
- Noms personnalisés par BMS (`name = "BMS-360Ah"`)

---

## API REST — Endpoints

### Système

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| GET | `/api/v1/system/status` | État global (BMS online, polling, version) |
| GET | `/api/v1/config` | Configuration active (sans secrets) |
| GET | `/api/v1/discover` | Découverte live sur le bus RS485 |

### BMS — Lecture

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| GET | `/api/v1/bms/{id}/status` | Snapshot complet (SOC, tension, courant…) |
| GET | `/api/v1/bms/{id}/cells` | Tensions individuelles + delta + équilibrage |
| GET | `/api/v1/bms/{id}/temperatures` | Températures par capteur |
| GET | `/api/v1/bms/{id}/alarms` | Flags d'alarme + `any_alarm` |
| GET | `/api/v1/bms/{id}/mos` | État MOS charge/décharge + cycles |
| GET | `/api/v1/bms/{id}/history` | Ring buffer (jusqu'à 3600 snapshots) |
| GET | `/api/v1/bms/{id}/history/summary` | Statistiques min/max/avg |
| GET | `/api/v1/bms/{id}/export/csv` | Export CSV du ring buffer |
| GET | `/api/v1/bms/compare` | Comparaison côte-à-côte de tous les BMS |

### BMS — Écriture (nécessite `api_key` si configurée)

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| POST | `/api/v1/bms/{id}/mos` | Activer/désactiver MOS charge/décharge |
| POST | `/api/v1/bms/{id}/soc` | Calibrer SOC |
| POST | `/api/v1/bms/{id}/soc/full` | SOC → 100% |
| POST | `/api/v1/bms/{id}/soc/empty` | SOC → 0% |
| POST | `/api/v1/bms/{id}/reset` | Reset BMS (avec `confirm: true`) |

### WebSocket

| Endpoint | Description |
|----------|-------------|
| `/ws/bms/stream` | Tous les BMS, broadcast à chaque cycle |
| `/ws/bms/{id}/stream` | Un seul BMS |

---

## Commandes Make

```bash
make up            # Démarrer Docker (infra seule)
make down          # Arrêter Docker
make restart       # Redémarrer les containers
make ps            # État des containers
make logs          # Logs de tous les containers (follow)
make reset         # Arrêter + supprimer volumes + redémarrer (reset complet)

make build              # Compiler (release, local)
make build-arm          # Cross-compiler daly-bms-server pour aarch64 (Pi5)
make build-arm-debug    # Build aarch64 avec symboles (profile release-debug)
make build-arm-musl     # Build aarch64 statique (musl)
make build-arm-v7       # Cross-compiler pour armv7 (NanoPi)
make build-venus        # Compiler dbus-mqtt-venus (host)
make build-venus-arm    # dbus-mqtt-venus aarch64
make build-venus-v7     # dbus-mqtt-venus armv7 (NanoPi)
make install-venus      # Déployer dbus-mqtt-venus sur GX (aarch64)
make install-venus-v7   # Déployer dbus-mqtt-venus sur NanoPi (armv7)
make build-energy       # Compiler energy-manager (host)
make build-energy-arm   # energy-manager aarch64 (Pi5)
make install-energy     # Déployer energy-manager sur Pi5
make run-energy         # Lancer energy-manager localement
make build-all          # Tous les binaires
make run                # Lancer daly-bms-server (release)
make run-debug          # daly-bms-server avec RUST_LOG=debug
make test               # Tests unitaires (workspace)
make test-core          # Tests daly-bms-core uniquement
make test-verbose       # Tests avec --nocapture
make lint               # Clippy (--all-targets, deny warnings)
make fmt                # cargo fmt
make check              # cargo check + fmt + clippy
make deploy             # Cross-compile + scp + restart sur Pi5
make deploy-musl        # Idem en build musl
make sync               # `git pull` côté Pi5 (executé sur la cible)
make install            # Installer daly-bms-server systemd
make uninstall          # Désinstaller daly-bms-server systemd
make install-node-exporter  # Installer Prometheus node_exporter
make profile-setup/-start/-stop  # Profiling perf/flamegraph
make doc                # cargo doc (workspace, --open)
```

---

## Gestion des logs et rétention des données

### Logs Docker (rotation automatique)

Tous les containers utilisent le driver `json-file` avec rotation automatique :

| Service | Max taille | Fichiers |
|---------|-----------|---------|
| dalybms-server | 20 Mo | 5 fichiers |
| Mosquitto | 10 Mo | 3 fichiers |
| VictoriaMetrics | 10 Mo | 3 fichiers |
| energy-manager | 5 Mo | 3 fichiers |

```bash
# Voir les logs en temps réel
make logs

# Logs d'un service spécifique
docker logs dalybms-server -f --tail 100
docker logs dalybms-VictoriaMetrics -f --tail 100
docker logs dalybms-mosquitto -f --tail 100

# Taille des fichiers log Docker
du -sh /var/lib/docker/containers/*/
```

### Logs systemd (déploiement sans Docker)

```bash
# Logs en temps réel
journalctl -u daly-bms -f

# Logs depuis une date
journalctl -u daly-bms --since "2026-03-17 00:00:00"

# Taille du journal systemd
journalctl --disk-usage

# Limiter la rétention (dans /etc/systemd/journald.conf)
# SystemMaxUse=200M
# MaxRetentionSec=7day
sudo systemctl restart systemd-journald

# Purger manuellement les anciens logs
sudo journalctl --vacuum-time=7d
sudo journalctl --vacuum-size=100M
```

### Rétention des données VictoriaMetrics

Par défaut : **30 jours** (720h), configurable dans `.env` :

```bash
# Dans .env
DOCKER_VictoriaMetrics_INIT_RETENTION=720h    # 30 jours (défaut)
# DOCKER_VictoriaMetrics_INIT_RETENTION=2160h  # 90 jours
# DOCKER_VictoriaMetrics_INIT_RETENTION=0      # Infini (déconseillé sur SD/eMMC)
```

### Nettoyage complet (reset usine)

```bash
# Arrêter tout + supprimer tous les volumes (DONNÉES PERDUES)
make reset

# Nettoyer les images Docker inutilisées
docker system prune -f

# Libérer l'espace disque Docker (images + caches)
docker system prune -a --volumes
```

> **Note RPi/eMMC** : Sur Raspberry Pi avec carte SD ou eMMC, surveiller l'espace disque.
> VictoriaMetrics peut écrire ~50–200 Mo/jour selon la fréquence de polling et le nombre de BMS.
> La rétention 30j = environ 1,5–6 Go max.

---

## Protocole Daly implémenté

### Format trame (13 octets)

```
┌──────┬──────┬──────────┬──────────────────────────────┬──────────┐
│ 0xA5 │ ADDR │ DATA_ID  │ DATA (8 octets, 0x00 lecture)│ CHECKSUM │
└──────┴──────┴──────────┴──────────────────────────────┴──────────┘
  1B     1B     1B          8B                              1B
```
- Baud rate : 9600
- Checksum : somme des octets (modulo 256)

### Commandes de lecture

| Data ID | Description | Parsing |
|---------|-------------|---------|
| 0x90 | Tension pack, courant, SOC | uint16/10, offset 30000, uint16/10 |
| 0x91 | Min/max tension cellule + numéro | uint16/1000, octet index |
| 0x92 | Min/max température + capteur | byte-40, octet index |
| 0x93 | État MOS, cycles, capacité résiduelle | bits, uint16, uint32 |
| 0x94 | Nombre cellules, capteurs, état charge | octets |
| 0x95 | Tensions individuelles (3/trame) | uint16/1000, multi-trames |
| 0x96 | Températures individuelles (7/trame) | byte-40, multi-trames |
| 0x97 | Flags équilibrage (48 max) | bits little-endian |
| 0x98 | Alarmes protection (7 octets) | flags |

### Commandes d'écriture

| Data ID | Description |
|---------|-------------|
| 0xD9 | MOS décharge ON/OFF |
| 0xDA | MOS charge ON/OFF |
| 0x21 | Calibration SOC (×10, uint16 BE) |
| 0x00 | Reset BMS |

---

## Alertes configurables

| Règle | Seuil déclenchement | Hysteresis |
|-------|---------------------|------------|
| `cell_ovp` | > 3.60 V | -50 mV |
| `cell_uvp` | < 2.90 V | +50 mV |
| `cell_imbalance` | > 100 mV | -10 mV |
| `soc_low` | < 20% | +5% |
| `soc_critical` | < 10% | +2% |
| `temp_high` | > 45°C | -2°C |
| `high_current` | > 80 A | -5 A |

Notifications : Telegram Bot + SMTP email + journal SQLite.

---

## Dépannage

```bash
# Port série
ls -l /dev/ttyUSB* && groups $USER
sudo usermod -aG dialout $USER  # si permission refusée

# Logs service systemd
journalctl -u daly-bms -f

# Logs Docker
make logs
docker logs dalybms-server -f --tail 100

# Test API
curl http://localhost:8080/api/v1/system/status | jq

# Test WebSocket
wscat -c ws://localhost:8080/ws/bms/stream

# Niveau de logs augmenté
RUST_LOG=debug daly-bms-server

# Vérifier état containers
make ps
docker compose -f docker-compose.infra.yml ps

# Redémarrer Mosquitto
docker compose -f docker-compose.infra.yml restart mosquitto
```

---

## Configuration VictoriaMetrics

### Accès initial

| Service | URL | Identifiants par défaut |
|---------|-----|------------------------|
| VictoriaMetrics | `http://RPi5:8428` | admin / voir `.env` |
| energy-manager | `http://RPi5:8081` | aucun (à sécuriser si exposé) |

> **Après un `make reset`** : utiliser l'URL de base sans chemin (ex. `http://192.168.1.141:8428`).
> L'ancien org ID dans l'URL bookmarkée devient invalide — se reconnecter depuis la page d'accueil.


### Dashboard (in progress)

Le dashboard `DalyBMS — Vue d'ensemble` est provisionné depuis :

Il affiche pour chaque BMS :
- SOC (gauge), tension pack, courant, puissance
- Température max cellules, delta cellules (déséquilibre), état MOS
- Séries temporelles : SOC, tension, courant, puissance
- Historique 15 min (auto-refresh 10s)

---

## Roadmap

### Phase 0 — Fondations Rust ✅

- [x] Structure workspace Rust (Cargo.toml, 5 crates)
- [x] Types de données (BmsSnapshot)
- [x] Protocole UART + checksum + tests unitaires
- [x] API Axum (toutes les routes définies)
- [x] AppState + ring buffer + broadcast WebSocket
- [x] Bridges (MQTT, VictoriaMetrics, AlertEngine)

### Phase 1 — Infrastructure & Intégration ✅

- [x] Infrastructure Mosquitto (Docker)
- [x] Auto-détection port série et adresses BMS
- [x] Dashboard SSR intégré (Askama + ECharts, sans npm)
- [x] MQTT publish_interval_sec réduit à 1s (temps réel)
- [x] Architecture Venus OS confirmée (MQTT → D-Bus)
- [x] Service dbus-canbattery.can0 stoppé sur NanoPi (CAN remplacé par MQTT)

### Phase 2 — Production RPi5 ✅

- [x] RPi5 CM opérationnel — données BMS 0x01 et 0x02 confirmées dans VictoriaMetrics
- [x] Correction adresses BMS (0x28/0x29 → 0x01/0x02)
- [x] Rotation logs Docker configurée + rétention VictoriaMetrics 30j
- [ ] Validation commandes d'écriture (MOS, SOC, reset) sur hardware réel
- [ ] Tests intégration 24h stabilité

### Phase 3 — Venus OS natif Rust ✅

- [x] Crate `dbus-mqtt-venus` : bridge MQTT → D-Bus (zbus pur Rust, sans libdbus)
- [x] Enregistrement `com.victronenergy.battery.*` sur le bus système Venus OS
- [x] Interface `com.victronenergy.BusItem` (GetValue, GetText, SetValue, ItemsChanged)
- [x] Watchdog MQTT (déconnexion propre si source silencieuse > 30s)
- [x] Keepalive D-Bus (republication toutes les 25s)
- [x] Remplacement de `dbus-mqtt-battery` Python par du Rust pur sur le NanoPi
- [x] Décision architecture : binaire unique `dbus-mqtt-venus` sur NanoPi pour tous les devices futurs

### Phase 4 — Migration & Consolidation 🚧 ✅

- [x] Renommer le crate `daly-bms-venus` → `dbus-mqtt-venus` dans le workspace Rust ✅
- [ ] Migration flows energy-manager du NanoPi vers le Pi5 (docker-compose.infra.yml) ✅
- [ ] Nettoyage NanoPi : services Python retirés, seul `dbus-mqtt-venus` reste
- [ ] Validation stabilité 24h post-migration energy-manager

### Phase 5 — Capteur Irradiance & Météo RS485 🔜 ✅

> Objectif : corréler la production PV avec l'ensoleillement et les conditions météo

- [ ] Identifier le modèle exact du capteur (protocole Modbus RTU, registres)
- [ ] Créer crate `santuario-solar` (polling RS485, types `SolarSnapshot`, `MeteoSnapshot`)
- [ ] Bridge MQTT : topics `santuario/solar/{n}/venus` et `santuario/meteo/venus`
- [ ] Extension `dbus-mqtt-venus` : `solar_service.rs` → `com.victronenergy.meteo.*`
- [ ] Dashboard : irradiance vs production Victron (corrélation)
- [ ] Alertes : nuages / ombrage détecté (irradiance < seuil)

### Phase 6 — Pompe à Chaleur Chauffe-Eau LG ✅

> Objectif : optimiser le chauffe-eau via surplus PV + monitoring consommation

- [ ] Étudier l'API LG ThinQ / LG SmartThinQ (authentification OAuth2, endpoints)
- [ ] Créer crate `santuario-heatpump` (poller API LG ThinQ toutes les 60s)
- [ ] Types : `HeatPumpSnapshot` (consigne, temp eau, mode, conso instantanée, COP)
- [ ] Bridge MQTT : `santuario/heat/dhw/venus` (Domestic Hot Water)
- [ ] Extension `dbus-mqtt-venus` : `heat_service.rs` → `com.victronenergy.temperature.dhw`
- [ ] Commandes : activation / consigne depuis Venus OS (DVCC surplus PV → chauffe)
- [ ] Alertes : température eau hors plage, défaut PAC

### Phase 7 — Pompe à Chaleur Climatisation LG 🔜

> Objectif : monitoring clim + potentiel pilotage depuis surplus PV

- [ ] Évaluation intégration LG Multi-Split (même API ThinQ ou Modbus local ?)
- [ ] Types : `AcSnapshot` (mode, consigne, temp ambiante, conso)
- [ ] Bridge MQTT : `santuario/heat/ac/{zone}/venus`
- [ ] Extension `dbus-mqtt-venus` : `com.victronenergy.temperature.ac_{zone}`
- [ ] Définir stratégie : API cloud vs Modbus local (à étudier selon le modèle)

### Phase 8 — ATS (Commutateur de Source Automatique) RS485 🔜 ✅

> Objectif : bascule automatique entre réseau EDF / groupe / Victron Multiplus

- [ ] Identifier le modèle ATS et son protocole RS485 (Modbus RTU probable)
- [ ] Créer crate `santuario-ats` (polling état + commandes bascule)
- [ ] Types : `AtsSnapshot` (source active, tensions, fréquence, défauts)
- [ ] Bridge MQTT : `santuario/ats/venus` + commandes `santuario/ats/cmd`
- [ ] Extension `dbus-mqtt-venus` : `ats_service.rs` → `com.victronenergy.grid`
- [ ] Intégration Venus OS : systemcalc voit l'ATS comme source grid
- [ ] Logique automatique : surplus PV → bascule Victron, nuit/nuage → grid

### Vision long terme 🔭

- [ ] Crate `santuario-core` : trait `DevicePoller` + `VenusPayload` partagés par tous les services
- [ ] Configuration dynamique : ajout capteur sans recompilation (TOML hot-reload)
- [ ] Dashboard SSR unifié : toutes les sources dans un seul écran
- [ ] Alertes corrélées : ex. "irradiance haute mais production faible → ombrage détecté"
- [ ] Export Home Assistant via MQTT Discovery (alternative Venus OS pour certains capteurs)

---

*Référence protocole : Daly UART/485 Communications Protocol V1.21*
*Runtime : [tokio-serial](https://docs.rs/tokio-serial/latest/tokio_serial/) — [Axum](https://docs.rs/axum/) — [rumqttc](https://docs.rs/rumqttc/)*
