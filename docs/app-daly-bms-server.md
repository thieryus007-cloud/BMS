# daly-bms-server — Référence Serveur Principal Pi5 — Daly-BMS-Rust

> Document de référence complet du binaire **daly-bms-server** (port 8080, Pi5 aarch64).
> Couvre : architecture interne, protocole Daly UART, API REST/WebSocket complète, dashboard SSR,
> bridges MQTT/metrics-store/AlertEngine, supervision fail-fast, réouverture port série.
> Fait partie de l'[architecture documentaire](./ARCHITECTURE.md).
> Dernière consolidation : 2026-06-07.

## Table des matières

- [1. Rôle et périmètre](#1-role-et-perimetre)
- [2. Architecture interne](#2-architecture-interne)
  - [2.1 Démarrage séquentiel (main.rs)](#21-demarrage-sequentiel-mainrs)
  - [2.2 AppState et ring buffer](#22-appstate-et-ring-buffer)
  - [2.3 Bus RS485 partagé et réouverture automatique](#23-bus-rs485-partage-et-reouverture-automatique)
  - [2.4 Supervision fail-fast (spawn_critical)](#24-supervision-fail-fast-spawn_critical)
  - [2.5 Flux de données BMS](#25-flux-de-donnees-bms)
  - [2.6 Flux de données capteurs additionnels](#26-flux-de-donnees-capteurs-additionnels)
- [3. Structure des fichiers sources](#3-structure-des-fichiers-sources)
- [4. Protocole Daly UART implémenté](#4-protocole-daly-uart-implemente)
  - [4.1 Format trame (13 octets)](#41-format-trame-13-octets)
  - [4.2 Commandes de lecture (0x90 → 0x98)](#42-commandes-de-lecture-0x90--0x98)
  - [4.3 Commandes d'écriture](#43-commandes-decriture)
- [5. Inventaire RS485 et services D-Bus](#5-inventaire-rs485-et-services-d-bus)
- [6. API REST — Surface complète](#6-api-rest--surface-complete)
  - [6.1 Système](#61-systeme)
  - [6.2 Venus (cache D-Bus / MQTT)](#62-venus-cache-d-bus--mqtt)
  - [6.3 Monitor (santé RS485, logs)](#63-monitor-sante-rs485-logs)
  - [6.4 BMS — Lecture](#64-bms--lecture)
  - [6.5 BMS — Écriture](#65-bms--ecriture)
  - [6.6 BMS — Paramètres alarmes](#66-bms--parametres-alarmes)
  - [6.7 ATS CHINT](#67-ats-chint)
  - [6.8 ET112 (compteurs énergie)](#68-et112-compteurs-energie)
  - [6.9 Charts / History](#69-charts--history)
  - [6.10 Tasmota / Shelly](#610-tasmota--shelly)
  - [6.11 PromQL (compatibilité Grafana)](#611-promql-compatibilite-grafana)
  - [6.12 Alertes](#612-alertes)
  - [6.13 Health et métriques redb](#613-health-et-metriques-redb)
- [7. WebSocket](#7-websocket)
- [8. Dashboard SSR (Askama + ECharts)](#8-dashboard-ssr-askama--echarts)
  - [8.1 Routes dashboard](#81-routes-dashboard)
  - [8.2 Fonctionnalités](#82-fonctionnalites)
  - [8.3 Génération ECharts et pipeline temps réel](#83-generation-echarts-et-pipeline-temps-reel)
- [9. Bridges internes](#9-bridges-internes)
  - [9.1 Bridge MQTT (publisher)](#91-bridge-mqtt-publisher)
  - [9.2 Écriture metrics-store (redb)](#92-ecriture-metrics-store-redb)
  - [9.3 AlertEngine](#93-alertengine)
- [10. Structures de données Rust clés](#10-structures-de-donnees-rust-cles)
- [11. Alertes configurables](#11-alertes-configurables)
- [12. Configuration (Config.toml)](#12-configuration-configtoml)
- [13. Commandes Make (binaire daly-bms-server)](#13-commandes-make-binaire-daly-bms-server)
- [14. Démarrage rapide](#14-demarrage-rapide)
- [15. Dépannage](#15-depannage)
- [16. Estimation mémoire](#16-estimation-memoire)
- [Annexe historique — Architecture temps réel DASHBOARD_EXTENSION_GUIDE](#annexe-historique--architecture-temps-reel-dashboard_extension_guide)
  - [A.1 Flux de données complet (ancienne description)](#a1-flux-de-donnees-complet-ancienne-description)
  - [A.2 Structures de données Rust (état documenté en 2026-04-05)](#a2-structures-de-donnees-rust-etat-documente-en-2026-04-05)
  - [A.3 Topics MQTT et payloads attendus](#a3-topics-mqtt-et-payloads-attendus)
  - [A.4 Guide d'ajout d'une nouvelle métrique (checklist générique)](#a4-guide-dajout-dune-nouvelle-metrique-checklist-generique)
  - [A.5 Dépannage spécifique dashboard](#a5-depannage-specifique-dashboard)
- [Voir aussi](#voir-aussi)
- [Sources consolidées](#sources-consolidees)

---

## 1. Rôle et périmètre

**daly-bms-server** est le binaire principal du Pi5 (Raspberry Pi Compute Module 5, 192.168.1.141). Il est le seul service à interroger directement le bus RS485 `/dev/ttyUSB0`, expose l'intégralité de l'API REST et WebSocket sur le port **8080**, héberge le dashboard SSR intégré (Askama + ECharts), et embarque le metrics-store (redb) comme TSDB interne.

Responsabilités :

- **Polling RS485** : 3 BMS Daly (0x01/0x02/0x03), 3 compteurs ET112 (0x07/0x08/0x09), capteur irradiance PRALRAN (0x05), ATS CHINT.
- **API REST/WS** : exposer toutes les données en temps réel aux clients externes (dashboard web, Grafana, scripts).
- **Dashboard SSR** : interface web embarquée dans le binaire, sans npm, sans serveur web séparé.
- **Bridge MQTT** : publier les snapshots vers le broker Mosquitto local (127.0.0.1:1883), préfixe `santuario/`, relayé vers le NanoPi pour l'intégration Venus OS.
- **Écriture metrics-store** : persister toutes les métriques dans redb (`/mnt/nvme/daly-bms/metrics.redb`) avec tiering raw 30 j / hourly 365 j / daily 5 ans.
- **AlertEngine** : évaluer les règles d'alarme, notifier (Telegram, SMTP), journaliser en SQLite.
- **Interface PromQL** : shim de compatibilité Grafana sur `/api/v1/query`, `/api/v1/query_range`, `/api/v1/labels`.

Ce que ce document ne détaille **pas** (renvois) :

- Internals redb/tiering → voir [./metriques-redb-architecture.md](./metriques-redb-architecture.md)
- Langage de requêtes PromQL → voir [./metriques-promql-reference.md](./metriques-promql-reference.md)
- Règles d'alerte et notifications (détail) → voir [./alertes.md](./alertes.md)
- Spécificités matériel ET112/ATS/PRALRAN et maintenance → voir [./integration-materiel.md](./integration-materiel.md)
- energy-manager → voir [./app-energy-manager.md](./app-energy-manager.md)
- Déploiement/build/systemd → voir [./deploiement-exploitation.md](./deploiement-exploitation.md)

---

## 2. Architecture interne

### 2.1 Démarrage séquentiel (main.rs)

`main.rs` démarre tous les sous-systèmes de façon séquentielle avant de lancer l'écoute HTTP :

1. Lecture de la configuration depuis `/etc/daly-bms/config.toml` (le service **ne lit pas** `~/Daly-BMS-Rust/Config.toml`).
2. Ouverture du bus RS485 partagé (`SharedBus`) sur `/dev/ttyUSB0` (baud 9600).
3. Construction de l'`AppState` (ring buffers par BMS, maps Venus, état ATS/ET112/irradiance).
4. Démarrage des boucles de polling critiques via `spawn_critical` :
   - `daly_bms_core::poll_loop()` pour chaque BMS configuré.
   - `et112::poll_loop()` pour chaque compteur ET112.
   - `irradiance::poll_loop()` pour le capteur PRALRAN.
   - `ats::poll_loop()` pour l'ATS CHINT.
5. Démarrage du bridge MQTT (rumqttc, connexion au broker local).
6. Démarrage de la tâche de maintenance tiering redb (4×/jour).
7. Démarrage du listener Axum (REST + WebSocket + Dashboard SSR) sur `0.0.0.0:8080`.

> **Important** : Les templates Askama (`templates/*.html`) sont **compilés dans le binaire** à la compilation. Tout changement HTML nécessite `make build-arm` + redéploiement binaire.

### 2.2 AppState et ring buffer

`AppState` (défini dans `state.rs`) est partagé entre toutes les tâches via `Arc<AppState>`. Structure principale :

```rust
pub struct AppState {
    // Ring buffer par BMS : 3600 snapshots (environ 1h à 1 snap/s)
    pub bms_snapshots: Arc<RwLock<HashMap<u8, VecDeque<BmsSnapshot>>>>,

    // Données Venus OS (lues via MQTT depuis energy-manager / NanoPi)
    pub venus_inverter:    Arc<RwLock<Option<VenusInverter>>>,
    pub venus_smartshunt:  Arc<RwLock<Option<VenusSmartShunt>>>,
    pub venus_mppts:       Arc<RwLock<Vec<VenusMppt>>>,
    pub venus_temperatures: Arc<RwLock<Vec<VenusTemperature>>>,

    // Broadcast tokio pour WebSocket et bridges
    pub broadcast_tx: broadcast::Sender<BmsSnapshot>,

    // Autres sous-états : ET112, ATS, irradiance, Tasmota, Shelly, alertes...
}
```

À chaque snapshot BMS reçu, `AppState::on_snapshot()` :
1. Insère dans le ring buffer (capacité 3600, FIFO).
2. Émet sur le channel `broadcast_tx` tokio.
3. Les abonnés (bridge MQTT, écriture redb, AlertEngine, WebSocket) consomment chacun indépendamment.

### 2.3 Bus RS485 partagé et réouverture automatique

Le bus `/dev/ttyUSB0` est accessible via `SharedBus` (crate `rs485-bus`), protégé par un mutex tokio pour éviter les collisions entre les différentes boucles de polling (BMS, ET112, ATS, PRALRAN).

**Réouverture automatique** (règle 17 de CLAUDE.md) :

- `SharedBus::reopen()` rouvre le port après une déconnexion USB ou une ré-énumération.
- `poll_loop` déclenche `reopen()` sur les erreurs `DalyError::Serial` **et** `DalyError::Io`, avec backoff exponentiel.
- Comme le bus est partagé, le reopen bénéficie simultanément à ET112, ATS et PRALRAN.
- Plus besoin de redémarrer le service manuellement après un débranchement USB.

### 2.4 Supervision fail-fast (spawn_critical)

Les boucles de service longue durée passent par `spawn_critical` (helper défini dans `main.rs`) :

```
spawn_critical("poll_bms_1", poll_loop_bms(...))
spawn_critical("bridge_mqtt", mqtt_bridge_loop(...))
spawn_critical("et112_poll", et112_poll_loop(...))
...
```

Comportement :

- Si une boucle critique **retourne** (même sans erreur) ou **panique** (`panic=abort` activé), le process entier s'arrête.
- systemd (`Restart=on-failure`) redémarre le service automatiquement.
- Conséquence : **plus de bridge ou de poll mort silencieux** pendant que le service paraît « up ».

**Règle importante** : ne jamais utiliser `spawn_critical` pour une tâche one-shot ou transitoire (timer, traitement par snapshot) — elle se terminerait normalement et provoquerait un exit non désiré.

### 2.5 Flux de données BMS

```
BMS UART  ──► daly_bms_core::poll_loop()   (mode hardware RS485/USB)
                    │
                    ▼  on_snapshot(snap)
             AppState::on_snapshot()
              ┌──────┴──────────────────────────────────┐
              ▼                                         ▼
       ring_buffer                              broadcast (tokio)
       (3600 snaps/BMS)           ┌────────────┬────────┴──────┬───────────┐
                                  ▼            ▼               ▼           ▼
                            MqttBridge    metrics-store    AlertEngine  WebSocket
                            (rumqttc)     (redb, embarqué) (SQLite)     (/ws/bms/*)
                                │              │
                                ▼              ▼
                          Mosquitto :1883   /mnt/nvme/daly-bms/metrics.redb
                                │
                      dbus-mqtt-venus (NanoPi)
                      com.victronenergy.battery.*
                           (Venus OS / D-Bus)
```

### 2.6 Flux de données capteurs additionnels

```
ET112 Modbus RTU (0x07/0x08/0x09) ──► daly-bms-server::et112::poll_loop()
                                          │
                              ┌───────────┴────────────────┐
                              ▼                            ▼
                        MqttBridge                  metrics-store (redb)
                        ├── santuario/pvinverter/{n}/venus
                        └── santuario/heatpump/{n}/venus (ou grid/)
                              │
                        dbus-mqtt-venus (pvinverter / heatpump / grid services)

PRALRAN RS485 (0x05)   ──► daly-bms-server::irradiance::poll_loop()
                                          │
                              ┌───────────┴────────────────┐
                              ▼                            ▼
                        MqttBridge                  metrics-store (redb)
                        └── santuario/meteo/venus
                              │
                        dbus-mqtt-venus (meteo service)

ATS CHINT RS485        ──► daly-bms-server::ats (lecture + commandes)
                                          │
                              ┌───────────┴────────────────┐
                              ▼                            ▼
                        MqttBridge                  metrics-store (redb)
                        └── santuario/switch/{n}/venus
                              │
                        dbus-mqtt-venus (switch service)

LG ThinQ API           ──► energy-manager::http_clients::lg_thinq
                                          │
                        MqttBridge ──► santuario/heat/{n}/venus
                                          │
                        dbus-mqtt-venus (temperature service)
                              (NB: LG ThinQ géré par energy-manager, pas daly-bms-server)
```

---

## 3. Structure des fichiers sources

```
crates/daly-bms-server/
├── src/
│   ├── main.rs                ← Point d'entrée : démarrage séquentiel, spawn_critical
│   ├── config.rs              ← Chargement Config.toml → struct AppConfig
│   ├── state.rs               ← AppState, ring buffer, helpers on_snapshot / venus_*
│   ├── autodetect.rs          ← Auto-détection port série et adresses BMS
│   ├── monitor.rs             ← Santé RS485, statistiques bus, logs internes
│   ├── console.rs             ← WebSocket console (logs en temps réel)
│   ├── redb_writes.rs         ← Write path vers metrics-store redb
│   │                            (labels : address="0x07" hex pour ET112)
│   ├── api/                   ← Router Axum
│   │   ├── mod.rs             ← build_router() — toutes les routes enregistrées
│   │   ├── bms.rs             ← Handlers BMS lecture + écriture
│   │   ├── system.rs          ← Handlers système, Venus endpoints
│   │   ├── ats.rs             ← Handlers ATS CHINT
│   │   ├── et112.rs           ← Handlers ET112
│   │   ├── chart.rs           ← Handlers charts / history
│   │   ├── alerts.rs          ← Handlers AlertEngine
│   │   ├── tasmota.rs         ← Handlers Tasmota
│   │   ├── shelly.rs          ← Handlers Shelly
│   │   ├── promql.rs          ← Shim PromQL (/api/v1/query, /query_range, /labels)
│   │   ├── console.rs         ← Handler WebSocket console
│   │   └── health.rs          ← /health + /-/healthy
│   ├── ats/                   ← Module ATS CHINT (RS485 + commandes)
│   ├── et112/                 ← Module ET112 (Modbus RTU poll + parsing)
│   ├── irradiance/            ← Module capteur PRALRAN (RS485)
│   ├── tasmota/               ← Module Tasmota (capteurs / relais MQTT)
│   ├── shelly/                ← Module Shelly (switches MQTT)
│   ├── bridges/               ← Bridge MQTT (rumqttc) + AlertEngine SQLite
│   └── dashboard/             ← Routes SSR + génération ECharts
└── templates/                 ← Templates Askama (.html) — compilés dans le binaire
    ├── base.html
    ├── dashboard.html         → /dashboard
    ├── bms_detail.html        → /dashboard/bms/:id
    ├── et112.html             → /dashboard/et112
    ├── et112_detail.html      → /dashboard/et112/:addr
    ├── tasmota.html           → /dashboard/tasmota
    ├── tasmota_detail.html    → /dashboard/tasmota/:id
    ├── ats.html               → /dashboard/ats
    ├── monitor.html           → /dashboard/monitor
    ├── console.html           → /dashboard/console
    ├── visualization.html     → /dashboard/visualization
    ├── history.html           → /dashboard/history
    ├── alerts.html            → /dashboard/alerts
    ├── logs.html              → /dashboard/logs
    └── settings.html          → /dashboard/settings
```

> **Note `redb_writes.rs`** : le backend écrit le label `address="0x07"` (hexadécimal) pour ET112. Les requêtes PromQL doivent utiliser `address="0x07"`, jamais `address="7"` (décimal → zéro série retournée). Vérification : `curl -s 'localhost:8080/api/v1/query?query=et112_power_w' | jq '.data.result[].metric'`.

---

## 4. Protocole Daly UART implémenté

Référence : *Daly UART/485 Communications Protocol V1.21*

Runtime : [tokio-serial](https://docs.rs/tokio-serial/latest/tokio_serial/), [Axum](https://docs.rs/axum/), [rumqttc](https://docs.rs/rumqttc/)

### 4.1 Format trame (13 octets)

```
┌──────┬──────┬──────────┬──────────────────────────────┬──────────┐
│ 0xA5 │ ADDR │ DATA_ID  │ DATA (8 octets, 0x00 lecture)│ CHECKSUM │
└──────┴──────┴──────────┴──────────────────────────────┴──────────┘
  1B     1B     1B          8B                              1B
```

- **Baud rate** : 9600
- **Checksum** : somme de tous les octets de la trame (modulo 256)
- **ADDR** : adresse Modbus du BMS (0x01 = BMS-360Ah, 0x02 = BMS-320Ah, 0x03 = BMS-620Ah)
- **DATA** : 8 octets à 0x00 pour les commandes de lecture ; données réelles pour l'écriture

### 4.2 Commandes de lecture (0x90 → 0x98)

| Data ID | Description | Parsing / Notes |
|---------|-------------|-----------------|
| `0x90` | Tension pack, courant, SOC | tension : `uint16 / 10` (V) ; courant : `uint16 / 10` avec offset 30000 (A) ; SOC : `uint16 / 10` (%) |
| `0x91` | Min/max tension cellule + numéro de cellule | tension : `uint16 / 1000` (V) ; index cellule : `octet` |
| `0x92` | Min/max température + numéro capteur | température : `byte - 40` (°C) ; index capteur : `octet` |
| `0x93` | État MOS charge/décharge, cycles, capacité résiduelle | état MOS : `bits` ; cycles : `uint16` ; capacité résiduelle : `uint32` (mAh) |
| `0x94` | Nombre cellules, nombre capteurs, état charge | champs : `octets` individuels |
| `0x95` | Tensions individuelles (3 cellules par trame, multi-trames) | chaque tension : `uint16 / 1000` (V) ; lecture multi-réponses jusqu'à N cellules |
| `0x96` | Températures individuelles (7 capteurs par trame, multi-trames) | chaque température : `byte - 40` (°C) |
| `0x97` | Flags équilibrage (jusqu'à 48 cellules max) | bits little-endian ; 1 bit par cellule |
| `0x98` | Alarmes de protection (7 octets de flags) | flags booléens sur surintensité, surtension, sous-tension, surtemp, etc. |

### 4.3 Commandes d'écriture

| Data ID | Description | Notes |
|---------|-------------|-------|
| `0xD9` | MOS décharge ON / OFF | champ DATA[0] = 1 (on) ou 0 (off) |
| `0xDA` | MOS charge ON / OFF | champ DATA[0] = 1 (on) ou 0 (off) |
| `0x21` | Calibration SOC | valeur × 10, encodée `uint16` big-endian dans DATA[0:1] |
| `0x00` | Reset BMS | trame DATA = 0x00 × 8 ; confirmation `confirm: true` requise via API |

> **Statut production** : les commandes de lecture sont validées en production (RPi5, données BMS 0x01/0x02/0x03 confirmées dans metrics-store depuis mars 2026). La validation des commandes d'écriture (MOS, SOC, reset) sur hardware réel est encore en cours (cf. Roadmap Phase 2).

---

## 5. Inventaire RS485 et services D-Bus

Bus `/dev/ttyUSB0` — source de vérité : CLAUDE.md §5.

| Addr | Appareil | Type D-Bus | Topic MQTT (préfixe `santuario/`) | Instance D-Bus |
|------|----------|------------|-----------------------------------|----------------|
| `0x01` | BMS-360Ah | `battery.mqtt_1` | `bms/1/venus` | 151 |
| `0x02` | BMS-320Ah | `battery.mqtt_2` | `bms/2/venus` | 152 |
| `0x03` | BMS-620Ah | `battery.mqtt_3` | `bms/3/venus` | 153 |
| `0x05` | PRALRAN irradiance | `meteo` (singleton) | `irradiance/raw` → `meteo/venus` | 40 |
| `0x07` | ET112-Micro-Onduleurs (SN 119253X) | `pvinverter.mqtt_7` | `pvinverter/7/venus` | 32 |
| `0x08` | ET112-Maison (SN 119215X) | `acload.mqtt_8` | `grid/8/venus` | 30 |
| `0x09` | ET112-Réseau (SN 061077X) | `grid.mqtt_9` | `grid/9/venus` | 31 |

> **Divergence Readme vs CLAUDE.md** : le Readme mentionne parfois 2 BMS (état antérieur à mai 2026). La production compte **3 BMS** (0x01, 0x02, 0x03). CLAUDE.md §5 fait autorité.

Services D-Bus actifs nominaux sur le NanoPi :

```
com.victronenergy.battery.mqtt_1          BMS-360Ah (inst. 151)
com.victronenergy.battery.mqtt_2          BMS-320Ah (inst. 152)
com.victronenergy.battery.mqtt_3          BMS-620Ah (inst. 153)
com.victronenergy.pvinverter.mqtt_7       ET112-Micro-Onduleurs (inst. 32)
com.victronenergy.acload.mqtt_8           ET112-Maison / Consommation AC (inst. 30)
com.victronenergy.grid.mqtt_9             ET112-Réseau / Compteur réseau EDF (inst. 31)
com.victronenergy.temperature.mqtt_1      Capteur ext. (type 4, inst. 20)
com.victronenergy.switch.mqtt_1           ATS CHINT (inst. 60)
com.victronenergy.switch.mqtt_2           Tongou Switch1 (inst. 61)
com.victronenergy.switch.mqtt_3           Tongou Switch2 (inst. 62)
com.victronenergy.switch.mqtt_4           Tongou Switch3 (inst. 63)
com.victronenergy.switch.mqtt_5           Tongou Switch4 (inst. 64)
com.victronenergy.switch.mqtt_6           Tongou Switch5 / tongou_3ACC34 (inst. 65)
com.victronenergy.meteo                   Irradiance PRALRAN + TodaysYield (inst. 40)
com.victronenergy.pvinverter.cgwacs_ttyUSB0_mb2   Onduleur PV Victron direct
```

> **Nom exact** de l'onduleur Victron direct sur D-Bus : `cgwacs_ttyUSB0_mb2` (pas `rs485`).

Diagnostic rapide (depuis Pi5) :

```bash
ssh root@192.168.1.120 "dbus -y | grep victronenergy"
```

---

## 6. API REST — Surface complète

Toutes les routes sont enregistrées dans `crates/daly-bms-server/src/api/mod.rs` via `build_router()`. Base URL : `http://192.168.1.141:8080`.

### 6.1 Système

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| `GET` | `/api/v1/system/status` | État global du serveur (BMS online, polling actif, version binaire) |
| `GET` | `/api/v1/system/totals` | Agrégats système (puissance totale, énergie, SOC moyen) |
| `GET` | `/api/v1/system/logs` | Logs récents du serveur |
| `GET` | `/api/v1/config` | Configuration active (sans secrets, champs sensibles masqués) |
| `GET` | `/api/v1/discover` | Découverte live des adresses actives sur le bus RS485 |
| `GET` | `/api/v1/irradiance/status` | État et dernière mesure du capteur PRALRAN (irradiance W/m²) |
| `POST` | `/api/v1/solar/mppt-yield` | Injecter / mettre à jour le yield MPPT (depuis energy-manager) |

### 6.2 Venus (cache D-Bus / MQTT)

Ces endpoints exposent les données Victron reçues via MQTT (publiées par energy-manager depuis D-Bus).

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| `GET` | `/api/v1/venus/mppt` | Liste des chargeurs MPPT solaires (puissance, tension, yield today) |
| `GET` | `/api/v1/venus/smartshunt` | Moniteur de batterie SmartShunt (courant, tension, SOC, état) |
| `GET` | `/api/v1/venus/inverter` | Onduleur Victron (tension DC, courant DC, puissance AC sortie) |
| `GET` | `/api/v1/venus/temperatures` | Capteurs de température Venus (ext., batterie, etc.) |
| `GET` | `/api/v1/venus/heatpumps` | Données pompe à chaleur (issues du bridge energy-manager) |

Format de réponse type (`/api/v1/venus/inverter`) :

```json
{
  "connected": true,
  "inverter": {
    "voltage_v": 48.2,
    "current_a": 3.5,
    "power_w": 168.7,
    "ac_output_voltage_v": 229.8,
    "ac_output_current_a": 5.6,
    "ac_output_power_w": 1286.0,
    "state": "on",
    "mode": "inverter",
    "timestamp": "2026-04-05T14:32:45.123Z"
  }
}
```

### 6.3 Monitor (santé RS485, logs)

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| `GET` | `/api/v1/monitor/status` | État global du monitoring (uptime, erreurs RS485) |
| `GET` | `/api/v1/monitor/rs485-health` | Santé détaillée du bus RS485 (CRC errors, timeouts, succès/taux par appareil) |
| `GET` | `/api/v1/monitor/logs` | Liste des fichiers de logs disponibles |
| `GET` | `/api/v1/monitor/logs/content` | Contenu des logs (paramètre `?file=...`) |

### 6.4 BMS — Lecture

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| `GET` | `/api/v1/bms/:id/status` | Snapshot complet : SOC, tension pack, courant, puissance, température max |
| `GET` | `/api/v1/bms/:id/cells` | Tensions individuelles de toutes les cellules + delta max + flags équilibrage |
| `GET` | `/api/v1/bms/:id/temperatures` | Températures par capteur (numérotés) |
| `GET` | `/api/v1/bms/:id/alarms` | Flags d'alarme actifs + booléen `any_alarm` |
| `GET` | `/api/v1/bms/:id/mos` | État MOS charge/décharge + compteur de cycles |
| `GET` | `/api/v1/bms/:id/history` | Ring buffer : jusqu'à 3600 snapshots (environ 1 heure à 1 snap/s) |
| `GET` | `/api/v1/bms/:id/history/summary` | Statistiques sur le ring buffer : min/max/avg tension, courant, SOC |
| `GET` | `/api/v1/bms/:id/export/csv` | Export CSV du ring buffer courant (pour analyse offline) |
| `GET` | `/api/v1/bms/:id/settings` | Paramètres configurés du BMS (seuils alarmes, balancing) |
| `GET` | `/api/v1/bms/compare` | Comparaison côte-à-côte de tous les BMS (SOC, tensions, températures) |

`:id` = identifiant numérique du BMS (1 = BMS-360Ah, 2 = BMS-320Ah, 3 = BMS-620Ah).

### 6.5 BMS — Écriture

> Les endpoints d'écriture nécessitent le header `X-Api-Key: <api_key>` si `api_key` est configurée dans `Config.toml`. En l'absence de configuration `api_key`, les endpoints sont accessibles sans authentification.

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| `POST` | `/api/v1/bms/:id/mos` | Activer/désactiver MOS charge ou décharge. Body : `{"charge": true, "discharge": true}` |
| `POST` | `/api/v1/bms/:id/soc` | Calibrer le SOC manuellement. Body : `{"soc_percent": 85.0}` |
| `POST` | `/api/v1/bms/:id/soc/full` | Forcer SOC → 100% (BMS signale pleine charge) |
| `POST` | `/api/v1/bms/:id/soc/empty` | Forcer SOC → 0% (BMS signale décharge complète) |
| `POST` | `/api/v1/bms/:id/reset` | Reset BMS. Body : `{"confirm": true}` obligatoire |

Correspond aux commandes d'écriture UART : 0xDA (charge MOS), 0xD9 (discharge MOS), 0x21 (SOC), 0x00 (reset).

### 6.6 BMS — Paramètres alarmes

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| `POST` | `/api/v1/bms/:id/settings/cell-voltage-alarms` | Seuils alarme tension cellule (ovp/uvp, warn/alarm niveaux) |
| `POST` | `/api/v1/bms/:id/settings/pack-voltage-alarms` | Seuils alarme tension pack total |
| `POST` | `/api/v1/bms/:id/settings/current-alarms` | Seuils alarme courant (surintensité charge/décharge) |
| `POST` | `/api/v1/bms/:id/settings/delta-alarms` | Seuil alarme delta de tension entre cellules |
| `POST` | `/api/v1/bms/:id/settings/balancing` | Configuration équilibrage (seuil déclenchement, delta cible) |

### 6.7 ATS CHINT

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| `GET` | `/api/v1/ats/status` | État ATS : source active, tensions L1/L2/L3, fréquence, défauts |
| `POST` | `/api/v1/ats/remote_on` | Activer le mode télécommande ATS |
| `POST` | `/api/v1/ats/remote_off` | Désactiver le mode télécommande ATS |
| `POST` | `/api/v1/ats/force_source1` | Forcer ATS sur source 1 (réseau EDF) |
| `POST` | `/api/v1/ats/force_source2` | Forcer ATS sur source 2 (onduleur Victron / groupe) |
| `POST` | `/api/v1/ats/force_double` | Forcer ATS en mode double alimentation |
| `POST` | `/api/v1/ats/send_raw` | Envoyer une trame RS485 brute à l'ATS (debug) |
| `GET` | `/api/v1/ats/debug_on` | Activer les logs verbeux ATS |
| `GET` | `/api/v1/ats/debug_off` | Désactiver les logs verbeux ATS |

Voir [./integration-materiel.md](./integration-materiel.md) pour les détails protocole ATS CHINT.

### 6.8 ET112 (compteurs énergie)

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| `GET` | `/api/v1/et112` | Liste de tous les compteurs ET112 détectés (adresses, état) |
| `GET` | `/api/v1/et112/:addr/status` | Mesures actuelles : puissance, tension, courant, énergie (L1/L2/L3) |
| `GET` | `/api/v1/et112/:addr/history` | Historique des mesures ET112 depuis metrics-store |

`:addr` = adresse Modbus en hexadécimal (ex. `0x07`, `0x08`, `0x09`).

> **Important** : le label `address` dans les métriques redb est stocké en hex (`"0x07"`). Les requêtes PromQL Grafana doivent utiliser `address="0x07"`, jamais `address="7"`.

Voir [./integration-materiel.md](./integration-materiel.md) pour les registres Modbus ET112.

### 6.9 Charts / History

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| `GET` | `/api/v1/chart/history` | Données historiques agrégées pour graphiques (paramètres : `metric`, `from`, `to`, `step`) |
| `GET` | `/api/v1/chart/edge-history` | Historique des transitions / événements (alarmes, changements état MOS) |
| `GET` | `/api/v1/history/energy` | Bilan énergétique historique (production, consommation, balance réseau) |

### 6.10 Tasmota / Shelly

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| `GET` | `/api/v1/tasmota` | Liste des switches/capteurs Tasmota configurés (état, puissance) |
| `GET` | `/api/v1/tasmota/:id/status` | État détaillé d'un appareil Tasmota (relais, mesures énergie) |
| `GET` | `/api/v1/tasmota/:id/history` | Historique des mesures Tasmota |
| `POST` | `/api/v1/tasmota/:id/control` | Commander un relais Tasmota (on/off/toggle) |
| `GET` | `/api/v1/shelly` | Liste des switches/capteurs Shelly configurés |
| `GET` | `/api/v1/shelly/:id/status` | État détaillé d'un appareil Shelly |
| `POST` | `/api/v1/shelly/:id/channel/:ch/control` | Commander un canal Shelly (on/off) |

### 6.11 PromQL (compatibilité Grafana)

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| `GET` | `/api/v1/query` | Requête instantanée PromQL (paramètre `?query=...&time=...`) |
| `GET` | `/api/v1/query_range` | Requête sur plage temporelle (paramètres `query`, `start`, `end`, `step`) |
| `GET` | `/api/v1/labels` | Liste des noms de labels disponibles (pour auto-complétion Grafana) |

Ce shim sert de datasource Grafana configurée avec l'UID `daly-metrics` pointant sur `http://127.0.0.1:8080`.

Voir [./metriques-promql-reference.md](./metriques-promql-reference.md) pour le catalogue complet des métriques et les conventions de labels.

### 6.12 Alertes

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| `GET` | `/api/v1/alerts/list` | Liste des alertes actives et historique récent (avec état, timestamp, valeur déclenchante) |
| `GET` | `/api/v1/alerts/stats` | Statistiques AlertEngine (décompte par règle, taux déclenchement) |
| `POST` | `/api/v1/alerts/:id/acknowledge` | Acquitter une alerte (supprime de la liste active) |

Voir [./alertes.md](./alertes.md) pour le détail des règles, hysteresis et notifications.

### 6.13 Health et métriques redb

| Méthode | Endpoint | Description |
|---------|----------|-------------|
| `GET` | `/health` | Healthcheck HTTP simple (200 OK si le serveur répond) |
| `GET` | `/-/healthy` | Healthcheck backend redb (vérifie que la base est accessible et opérationnelle) |
| `GET` | `/api/v1/redb/series` | Liste toutes les séries métriques stockées dans redb (pour diagnostic) |

Commandes de vérification rapide :

```bash
curl -s http://localhost:8080/-/healthy
curl -s http://localhost:8080/api/v1/redb/series | jq '.data | length'
```

---

## 7. WebSocket

| Endpoint | Description |
|----------|-------------|
| `WS /ws/bms/stream` | Flux temps réel de tous les BMS — broadcast à chaque nouveau snapshot (fréquence = polling_interval_ms) |
| `WS /ws/bms/:id/stream` | Flux temps réel d'un seul BMS (`:id` = 1, 2 ou 3) |
| `WS /ws/venus/stream` | Flux temps réel des données Venus OS (inverter, smartshunt, mppt) |
| `WS /ws/console` | Console de logs en temps réel (messages structurés JSON) |

Les WebSocket utilisent le channel `broadcast_tx` tokio de l'`AppState`. À chaque snapshot, le serveur émet sur le channel ; tous les clients WebSocket connectés reçoivent la mise à jour simultanément (broadcast vrai, pas de polling client).

Test WebSocket :

```bash
wscat -c ws://localhost:8080/ws/bms/stream
wscat -c ws://localhost:8080/ws/bms/1/stream
```

---

## 8. Dashboard SSR (Askama + ECharts)

### 8.1 Routes dashboard

| URL | Template | Description |
|-----|----------|-------------|
| `/dashboard` | `dashboard.html` | Vue synthèse de tous les BMS (cartes SOC/tension/courant/température) |
| `/dashboard/bms/:id` | `bms_detail.html` | Détail BMS : cellules, températures, historique ring buffer, graphiques |
| `/dashboard/et112` | `et112.html` | Liste des compteurs ET112 (puissance instantanée, énergie) |
| `/dashboard/et112/:addr` | `et112_detail.html` | Détail ET112 par adresse : courbes puissance L1/L2/L3, historique |
| `/dashboard/tasmota` | `tasmota.html` | Liste des appareils Tasmota |
| `/dashboard/tasmota/:id` | `tasmota_detail.html` | Détail Tasmota (état relais, mesures énergie) |
| `/dashboard/ats` | `ats.html` | État ATS CHINT (source active, tensions, commandes) |
| `/dashboard/monitor` | `monitor.html` | Santé RS485 (CRC errors, timeouts, taux succès par appareil) |
| `/dashboard/console` | `console.html` | Console logs WebSocket temps réel |
| `/dashboard/visualization` | `visualization.html` | Diagramme flux d'énergie (ReactFlow / schéma SVG) |
| `/dashboard/history` | `history.html` | Historique long terme (requête redb, sélecteur de période) |
| `/dashboard/alerts` | `alerts.html` | Alertes actives + historique |
| `/dashboard/logs` | `logs.html` | Logs systèmes (rotation fichiers) |
| `/dashboard/settings` | `settings.html` | Paramètres configurables (seuils alarmes, affichage) |

### 8.2 Fonctionnalités

Fonctionnalités du dashboard principal (`/dashboard`) :

- Cartes par BMS : SOC (gauge), tension pack, courant, puissance calculée, température maximale.
- Indicateur badge RS485 multi-BMS (vert = réponse OK, rouge = timeout/erreur).
- Noms personnalisés par BMS lus depuis `Config.toml` (champ `name = "BMS-360Ah"`).
- Thème clair, responsive.

Fonctionnalités du détail BMS (`/dashboard/bms/:id`) :

- Boxplot tensions cellules (min/max/avg) avec colorisation selon seuils (vert/orange/rouge).
- Indicateur d'équilibrage actif (cellules hautes/basses mises en évidence).
- Profil des températures par capteur.
- Historique temps réel basé sur le ring buffer 3600 snapshots (environ 1 heure).
- Graphiques ECharts SVG (pas de canvas, compatible SSR).

Dashboard historique (`/dashboard/history`) :

- Remplace Perses (retiré en mai 2026).
- Visualisation native des séries redb sans dépendance externe.
- Sélecteur de période : 6h, 24h, 7j, 30j, custom.
- Métriques disponibles : SOC, tensions, courant, puissance, ET112, irradiance.

### 8.3 Génération ECharts et pipeline temps réel

Le pipeline SSR → temps réel fonctionne en deux temps :

1. **Rendu initial SSR** : Askama génère le HTML complet côté serveur avec les données actuelles de l'`AppState`. Aucun aller-retour JSON au premier chargement.
2. **Mise à jour temps réel** : le template JavaScript embarqué dans le HTML ouvre un WebSocket ou effectue un polling REST selon la page ; les données arrivent et les graphiques ECharts sont mis à jour via l'API `chart.setOption(...)` en mode patch (delta, pas de re-rendu complet).

Génération ECharts :

- Les options de graphiques sont sérialisées en JSON côté Rust (dans `dashboard/`) et injectées dans la réponse HTML via un bloc `<script>`.
- ECharts est chargé depuis CDN ou embarqué selon la configuration build.
- Les graphiques utilisent uniquement SVG (pas Canvas) pour compatibilité maximale.

---

## 9. Bridges internes

### 9.1 Bridge MQTT (publisher)

Fichier : `crates/daly-bms-server/src/bridges/mqtt.rs`

Le bridge MQTT tourne comme une tâche `spawn_critical` indépendante. Il :

1. Se connecte au broker local `127.0.0.1:1883` (rumqttc, reconnexion automatique).
2. S'abonne aux topics de retour (données Venus/energy-manager) :
   - `santuario/inverter/venus`
   - `santuario/system/venus`
   - `santuario/meteo/venus`
3. Écoute le channel `broadcast_rx` tokio (abonné au broadcast de l'`AppState`).
4. À chaque snapshot reçu, publie avec `retain = true` :
   - `santuario/bms/{n}/venus` : payload JSON BMS (SOC, tension, courant, température, état MOS, alarmes).
   - `santuario/pvinverter/{n}/venus` : payload ET112 micro-onduleurs.
   - `santuario/grid/{n}/venus` : payload ET112 réseau/maison.
   - `santuario/meteo/venus` : payload irradiance PRALRAN.
   - `santuario/switch/{n}/venus` : payload ATS/Tongou.

Topics de publication (préfixe `santuario/`) :

| Topic | Source (daly-bms-server) | Cible D-Bus Venus |
|-------|--------------------------|-------------------|
| `bms/{n}/venus` | Snapshot BMS Daly | `battery.mqtt_{n}` |
| `pvinverter/{n}/venus` | ET112-Micro-Onduleurs | `pvinverter.mqtt_{n}` |
| `grid/{n}/venus` | ET112-Réseau / ET112-Maison | `grid.mqtt_{n}` / `acload.mqtt_{n}` |
| `switch/{n}/venus` | ATS CHINT / Tongou | `switch.mqtt_{n}` |
| `meteo/venus` | PRALRAN irradiance | `meteo` (singleton) |

### 9.2 Écriture metrics-store (redb)

Fichier : `crates/daly-bms-server/src/redb_writes.rs`

Chaque snapshot déclenche une écriture dans la base redb (`/mnt/nvme/daly-bms/metrics.redb`) :

- Une écriture par métrique et par BMS/appareil.
- Format des labels : texte UTF-8, séparés par virgules (`metric_name{label1="val1",label2="val2"}`).
- Label `address` pour ET112 : toujours en hexadécimal (`"0x07"`, `"0x08"`, `"0x09"`).
- Tiering automatique : raw 30 j → hourly 365 j → daily 5 ans.

Voir [./metriques-redb-architecture.md](./metriques-redb-architecture.md) pour le schéma des tables, l'encodage, et la procédure de maintenance.

### 9.3 AlertEngine

Fichier : `crates/daly-bms-server/src/bridges/` (AlertEngine SQLite)

L'AlertEngine est abonné au broadcast tokio. À chaque snapshot :

1. Évalue toutes les règles configurées (seuils + hysteresis).
2. Si déclenchement : insère dans la base SQLite locale (journal persistant).
3. Envoie les notifications configurées : Telegram Bot et/ou SMTP email.
4. Les alertes actives sont exposées via l'API REST (`/api/v1/alerts/list`).

Voir [./alertes.md](./alertes.md) pour le catalogue des règles, les seuils par défaut et la configuration des canaux de notification.

---

## 10. Structures de données Rust clés

Fichier principal : `crates/daly-bms-server/src/state.rs`

```rust
// Snapshot BMS (crate daly-bms-core, types.rs)
pub struct BmsSnapshot {
    pub address: u8,                    // Adresse Modbus (0x01, 0x02, 0x03)
    pub voltage_v: f32,                 // Tension pack (V)
    pub current_a: f32,                 // Courant (A, positif=charge, négatif=décharge)
    pub soc_percent: f32,               // SOC (%)
    pub cell_voltages: Vec<f32>,        // Tensions cellules individuelles (V)
    pub temperatures: Vec<f32>,         // Températures capteurs (°C)
    pub cell_balance_active: Vec<bool>, // Flags équilibrage par cellule
    pub charge_mos: bool,               // État MOS charge
    pub discharge_mos: bool,            // État MOS décharge
    pub cycle_count: u16,               // Nombre de cycles
    pub residual_capacity_ah: f32,      // Capacité résiduelle (Ah)
    pub alarms: BmsAlarms,              // Flags alarmes (struct de bits)
    pub timestamp: DateTime<Utc>,
}

// Données Venus OS — Onduleur Victron (MultiPlus)
pub struct VenusInverter {
    pub voltage_v: Option<f32>,           // Tension DC (V)
    pub current_a: Option<f32>,           // Courant DC (A)
    pub power_w: Option<f32>,             // Puissance DC (W)
    pub ac_output_voltage_v: Option<f32>, // Tension AC sortie L1 (V)
    pub ac_output_current_a: Option<f32>, // Courant AC sortie L1 (A)
    pub ac_output_power_w: Option<f32>,   // Puissance AC sortie L1 (W) ← affiché dashboard
    pub state: String,                    // "on" / "off" / "fault"
    pub mode: String,                     // "inverter" / "charger" / "passthrough"
    pub timestamp: DateTime<Utc>,
}

// Données Venus OS — SmartShunt
pub struct VenusSmartShunt {
    pub voltage_v: Option<f32>,      // Tension batterie (V)
    pub current_a: Option<f32>,      // Courant batterie (A, négatif=décharge) ← affiché
    pub power_w: Option<f32>,        // Puissance batterie (W)
    pub soc_percent: Option<f32>,    // SOC (%)
    pub state: String,               // "charging" / "discharging" / "idle"
    pub timestamp: DateTime<Utc>,
}

// Données Venus OS — MPPT solaire
pub struct VenusMppt {
    pub address: String,             // Adresse / instance D-Bus
    pub power_w: f32,                // Puissance sortie MPPT (W) ← affiché
    pub voltage_v: f32,              // Tension entrée PV (V)
    pub current_a: f32,              // Courant entrée PV (A)
    pub yield_today_kwh: f32,        // Énergie produite aujourd'hui (kWh)
    pub status: String,              // "ON" / "OFF" / "FAULTED"
    pub timestamp: DateTime<Utc>,
}

// Données Venus OS — Capteur température
pub struct VenusTemperature {
    pub address: String,             // Adresse D-Bus
    pub name: String,                // "Outdoor" / "Battery" / etc.
    pub temperature_c: f32,          // Température (°C) ← affiché
    pub type_num: i32,               // 0=batterie 1=frigo 2=générique 3=pièce 4=extérieur
    pub status: String,              // "connected" / "disconnected"
    pub timestamp: DateTime<Utc>,
}
```

---

## 11. Alertes configurables

| Règle | Seuil déclenchement | Hysteresis retour |
|-------|---------------------|-------------------|
| `cell_ovp` | > 3,60 V | -50 mV |
| `cell_uvp` | < 2,90 V | +50 mV |
| `cell_imbalance` | > 100 mV (delta max−min) | -10 mV |
| `soc_low` | < 20 % | +5 % |
| `soc_critical` | < 10 % | +2 % |
| `temp_high` | > 45 °C | -2 °C |
| `high_current` | > 80 A | -5 A |

Canaux de notification : Telegram Bot + SMTP email + journal SQLite.

Voir [./alertes.md](./alertes.md) pour la configuration des credentials (Telegram token, SMTP) et la gestion du journal.

---

## 12. Configuration (Config.toml)

Le service lit `/etc/daly-bms/config.toml` au démarrage (**pas** `~/Daly-BMS-Rust/Config.toml`).

```bash
# Appliquer une modification de config
sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart daly-bms
```

Extraits de structure :

```toml
[server]
port = 8080
# api_key = "..."           # optionnel : protège les endpoints d'écriture BMS

[rs485]
port = "/dev/ttyUSB0"
baud_rate = 9600

[[bms]]
address = 1                 # 0x01
name = "BMS-360Ah"
cells = 16

[[bms]]
address = 2                 # 0x02
name = "BMS-320Ah"
cells = 15

[[bms]]
address = 3                 # 0x03
name = "BMS-620Ah"
cells = 20

[mqtt]
host = "127.0.0.1"
port = 1883
publish_interval_sec = 1    # temps réel (1s)

[metrics_store]
path = "/mnt/nvme/daly-bms/metrics.redb"

[alerts]
# telegram_token = "..."
# telegram_chat_id = "..."
# smtp_host = "..."
```

> **Secrets** : ne jamais committer `.env`. Les tokens Telegram, clés API LG ThinQ sont dans `/etc/daly-bms/.env`, jamais dans `Config.toml` commité.

---

## 13. Commandes Make (binaire daly-bms-server)

```bash
make build              # Compiler daly-bms-server (release, local host)
make build-arm          # Cross-compiler daly-bms-server pour aarch64 (Pi5)
make build-arm-debug    # Build aarch64 avec symboles (profile release-debug)
make build-arm-musl     # Build aarch64 statique (musl)
make run                # Lancer daly-bms-server (release, local)
make run-debug          # Lancer daly-bms-server avec RUST_LOG=debug
make test               # Tests unitaires (workspace complet)
make test-core          # Tests daly-bms-core uniquement
make test-verbose       # Tests avec --nocapture
make lint               # Clippy (--all-targets, -D warnings)
make fmt                # cargo fmt
make check              # cargo check + fmt + clippy
make deploy             # Cross-compile + scp + restart sur Pi5
make deploy-musl        # Idem en build musl
make sync               # git pull côté Pi5 (exécuté sur la cible)
make install            # Installer daly-bms-server systemd (contrib/daly-bms.service)
make uninstall          # Désinstaller daly-bms-server systemd
make doc                # cargo doc (workspace, --open)
```

Commandes de déploiement manuel sur Pi5 :

```bash
# Compiler (depuis poste de dev ou Pi5)
make build-arm

# Déployer sur Pi5
sudo systemctl stop daly-bms
sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/
sudo systemctl start daly-bms

# Vérifier
journalctl -u daly-bms -f
```

---

## 14. Démarrage rapide

### Broker MQTT (prérequis)

```bash
# Vérifier que le broker est actif
systemctl status mosquitto-broker
# Logs broker
journalctl -u mosquitto-broker -f
```

### Première installation

```bash
# Créer le dossier de configuration
sudo mkdir -p /etc/daly-bms

# Copier et adapter la configuration
sudo cp Config.toml /etc/daly-bms/config.toml
sudo nano /etc/daly-bms/config.toml   # adapter port série, adresses BMS, chemin redb

# Ajouter l'utilisateur au groupe dialout (permissions port série)
sudo usermod -aG dialout $USER
# (nécessite déconnexion/reconnexion pour prendre effet)
```

### Compilation et lancement

```bash
# Développement local (natif)
make run-debug

# Production Pi5 (cross-compilation)
make build-arm
make deploy PI_HOST=pi@192.168.1.141

# Ou déploiement manuel
sudo systemctl stop daly-bms
sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/
sudo systemctl start daly-bms
```

### Installation systemd

```bash
make install        # copie le binaire + installe contrib/daly-bms.service
journalctl -u daly-bms -f
```

### Vérification rapide

```bash
# Healthcheck
curl -s http://localhost:8080/-/healthy

# État système
curl http://localhost:8080/api/v1/system/status | jq

# Nb séries en base
curl -s http://localhost:8080/api/v1/redb/series | jq '.data | length'

# Dashboard web
open http://192.168.1.141:8080/dashboard
```

---

## 15. Dépannage

### Port série

```bash
# Lister les ports série et vérifier les permissions
ls -l /dev/ttyUSB*
groups $USER

# Ajouter au groupe dialout (si "Permission denied")
sudo usermod -aG dialout $USER
# Se déconnecter/reconnecter pour appliquer

# Tester le bus RS485 (arrêter daly-bms d'abord !)
sudo systemctl stop daly-bms
mbpoll -m rtu -a 1:15 -b 9600 -t 3:float -r 1 -c 1 /dev/ttyUSB0
# mbpoll sans réponse = daly-bms monopolise encore le port (vérifier systemctl)
```

### Service et logs

```bash
# Logs en temps réel
journalctl -u daly-bms -f

# 50 dernières lignes (diagnostic démarrage)
journalctl -u daly-bms -n 50

# Logs depuis une date
journalctl -u daly-bms --since "2026-03-17 00:00:00"

# Vérifier état de tous les services liés
systemctl status daly-bms mosquitto-broker energy-manager

# Redémarrer Mosquitto
sudo systemctl restart mosquitto-broker

# Niveau de log augmenté (lancement manuel)
RUST_LOG=debug daly-bms-server
```

### API et WebSocket

```bash
# Test API système
curl http://localhost:8080/api/v1/system/status | jq

# Test BMS
curl http://localhost:8080/api/v1/bms/1/status | jq

# Test ET112
curl http://localhost:8080/api/v1/et112 | jq

# Test WebSocket
wscat -c ws://localhost:8080/ws/bms/stream
```

### Problèmes courants (tableau de référence)

| Symptôme | Cause probable | Solution |
|----------|----------------|----------|
| Service BMS ne démarre pas | Config manquante ou TOML invalide | `journalctl -u daly-bms -n 50` ; vérifier `/etc/daly-bms/config.toml` |
| Config ignorée après modif | Modif faite dans `~/Daly-BMS-Rust/Config.toml` seulement | `sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart daly-bms` |
| ET112 "en attente de données" | Mauvaise adresse Modbus | `sudo systemctl stop daly-bms && mbpoll -m rtu -a 1:15 -b 9600 -t 3:float -r 1 -c 1 /dev/ttyUSB0` |
| Dashboard Grafana ET112 vide (données existent) | Label `address` en décimal au lieu de hex | Requêtes PromQL : utiliser `address="0x07"` jamais `address="7"`. Vérif : `curl -s 'localhost:8080/api/v1/query?query=et112_power_w' | jq '.data.result[].metric'` |
| `mbpoll` sans réponse | daly-bms monopolise le port | `sudo systemctl stop daly-bms` d'abord |
| Dashboard affiche cumul brut (pas delta journalier) | Baseline MQTT manquante | Vérifier `pvinv_baseline` retained MQTT (`santuario/persist/pvinv_baseline`) |
| Widget météo "Température: -" | Limitation Venus OS | Inévitable, non fixable (Venus OS ne supporte pas ce champ via MQTT) |
| Disque racine Pi5 > 45 % | Builds Rust cumulés dans `target/` | `rm -rf target/armv7-unknown-linux-gnueabihf target/debug target/release && rm -rf ~/.cargo/registry/cache ~/.cargo/registry/src && sudo apt-get clean` (≈ -2,6 Go). Ne jamais supprimer `~/.cargo/bin`, `/usr/local/bin/*`, ni `/mnt/nvme/.../metrics.redb`. |
| `make sync` → "Permission denied" | Permissions dossier corrompues | `sudo chown -R pi5compute:pi5compute ~/Daly-BMS-Rust/ && git reset --hard origin/<branch>` |

### Rétention des logs systemd

```bash
# Taille du journal
journalctl --disk-usage

# Purger les anciens logs
sudo journalctl --vacuum-time=7d
sudo journalctl --vacuum-size=100M

# Limiter la rétention (dans /etc/systemd/journald.conf)
# SystemMaxUse=200M
# MaxRetentionSec=7day
sudo systemctl restart systemd-journald
```

### Reset usine (DONNÉES PERDUES)

```bash
# Arrêter tous les services
sudo systemctl stop daly-bms energy-manager mosquitto-broker

# Supprimer la base metrics-store et le broker (IRRÉVERSIBLE)
sudo rm -rf /mnt/nvme/daly-bms/metrics.redb /var/lib/mosquitto/mosquitto.db

# Redémarrer
sudo systemctl start mosquitto-broker energy-manager daly-bms
```

---

## 16. Estimation mémoire

### Pi5 (services natifs systemd) — mesure réelle ≈ 20 % RAM utilisée

| Service | RAM minimale | RAM confortable |
|---------|-------------|-----------------|
| daly-bms-server (Rust, redb embarqué) | ~25 Mo | ~50 Mo |
| energy-manager (Rust) | ~30 Mo | ~100 Mo (LimitMemoryMax=100M) |
| mosquitto-broker (natif) | ~8 Mo | ~15 Mo |
| metrics-store (redb) | embarqué dans daly-bms-server | ~0 Mo RSS additionnel (gain ~135 Mo vs ex-VictoriaMetrics) |
| OS Raspberry Pi OS Lite | ~150 Mo | ~200 Mo |
| Marge tampon / cache | ~200 Mo | ~400 Mo |
| **TOTAL Pi5** | **~413 Mo** | **~765 Mo** |

### NanoPi Neo3 (Venus OS)

| Service | RAM | Notes |
|---------|-----|-------|
| dbus-mqtt-venus | ~5–8 Mo | Binaire statique musl, zéro dépendance système |
| Venus OS + systemcalc-py | ~150 Mo | Existant |
| **Impact additionnel** | **~5 Mo** | Négligeable |

### Espace disque metrics-store (redb)

Prévision de croissance `/mnt/nvme/daly-bms/metrics.redb` :

- À 30 jours raw : ~200–400 Mo
- À horizon 5 ans (tiering raw/hourly/daily) : ~2 Go maximum

```bash
# Vérifier la taille actuelle
du -sh /mnt/nvme/daly-bms/metrics.redb
```

---

## Annexe historique — Architecture temps réel DASHBOARD_EXTENSION_GUIDE

> **Statut : OBSOLÈTE** — Ce document décrivait l'ancienne architecture avec energy-manager sous Docker et Node-RED (flows JSON `.json`). L'architecture actuelle utilise energy-manager Rust natif (systemd, port 8081). La section ci-dessous est conservée pour référence historique complète.

> Source d'origine : `DASHBOARD_EXTENSION_GUIDE.md` — Version 2.0, 2026-04-05, statut OBSOLÈTE.

### A.1 Flux de données complet (ancienne description)

L'ancienne architecture décrivait 5 étapes :

```
ÉTAPE 1: COLLECTE (NanoPi D-Bus)
  Victron Hardware D-Bus:
    com.victronenergy.system/Dc/Voltage          → 48.2V
    com.victronenergy.system/Dc/Current          → -12.4A
    com.victronenergy.system/Ac/Out/L1/V         → 229.8V
    com.victronenergy.system/Ac/Out/L1/P         → 1286W
    (et 100+ autres chemins D-Bus)

ÉTAPE 2: AGRÉGATION (energy-manager — Pi5)
  Flows (anciennement Node-RED JSON, maintenant Rust natif) :
    inverter.json   → subscribe D-Bus → aggregate → publish MQTT santuario/inverter/venus
    smartshunt.json → subscribe D-Bus → aggregate → publish MQTT santuario/system/venus
    Solar_power.json → subscribe D-Bus → aggregate → publish MQTT santuario/meteo/venus

ÉTAPE 3: STOCKAGE (daly-bms-server)
  MQTT Handlers (bridges/mqtt.rs) :
    handle_inverter_topic()  → parse JSON → struct VenusInverter → AppState
    handle_system_topic()    → parse JSON → struct VenusSmartShunt → AppState
    handle_meteo_topic()     → parse JSON → update MPPT metrics → AppState

ÉTAPE 4: EXPOSITION (REST API)
  GET /api/v1/venus/inverter     → VenusInverter + connected status
  GET /api/v1/venus/smartshunt   → VenusSmartShunt + connected status
  GET /api/v1/venus/mppt         → Vec<VenusMppt> + total power
  GET /api/v1/venus/temperatures → Vec<VenusTemperature>

ÉTAPE 5: AFFICHAGE (ReactFlow Dashboard)
  JavaScript fetch → poll REST toutes les 2s
  WebSocket /ws/venus/stream (40ms) ou polling fallback
  Indicateurs .live (vert si connected: true)
  Animations edges (direction flux d'énergie)
```

### A.2 Structures de données Rust (état documenté en 2026-04-05)

Ces structures sont décrites dans l'état de l'architecture à la date du guide. Elles sont toujours présentes dans `state.rs` mais l'architecture de leur alimentation a évolué (energy-manager Rust vs Node-RED flows).

```rust
// Onduleur Victron (MultiPlus)
pub struct VenusInverter {
    pub voltage_v: Option<f32>,           // DC voltage
    pub current_a: Option<f32>,           // DC current
    pub power_w: Option<f32>,             // DC power
    pub ac_output_voltage_v: Option<f32>, // AC output voltage
    pub ac_output_current_a: Option<f32>, // AC output current
    pub ac_output_power_w: Option<f32>,   // AC output power ← AFFICHÉ SUR DASHBOARD
    pub state: String,                    // "on" / "off" / "fault"
    pub mode: String,                     // "inverter" / "charger" / "passthrough"
    pub timestamp: DateTime<Utc>,
}

// SmartShunt (Victron Battery Monitor)
pub struct VenusSmartShunt {
    pub voltage_v: Option<f32>,      // Battery voltage
    pub current_a: Option<f32>,      // Battery current ← AFFICHÉ (négatif = décharge)
    pub power_w: Option<f32>,        // Battery power
    pub soc_percent: Option<f32>,    // State of charge
    pub state: String,               // "charging" / "discharging" / "idle"
    pub timestamp: DateTime<Utc>,
}

// MPPT Solar Charger
pub struct VenusMppt {
    pub address: String,             // Device address / instance
    pub power_w: f32,                // Output power ← AFFICHÉ
    pub voltage_v: f32,              // Input voltage
    pub current_a: f32,              // Input current
    pub yield_today_kwh: f32,        // Energy generated today
    pub status: String,              // "ON" / "OFF" / "FAULTED"
    pub timestamp: DateTime<Utc>,
}

// Temperature Sensor
pub struct VenusTemperature {
    pub address: String,
    pub name: String,                // "Outdoor" / "Battery" / etc.
    pub temperature_c: f32,
    pub type_num: i32,               // 0=batterie 1=frigo 2=générique 3=pièce 4=extérieur
    pub status: String,              // "connected" / "disconnected"
    pub timestamp: DateTime<Utc>,
}
```

### A.3 Topics MQTT et payloads attendus

Ces topics sont **toujours actifs** ; c'est leur producteur qui a changé (Node-RED → energy-manager Rust).

**`santuario/inverter/venus`**

Produit par : energy-manager (anciennement `inverter.json` flow Node-RED)
Fréquence : chaque nouveau message D-Bus (~100 ms Victron)

```json
{
  "Voltage": 48.2,
  "Current": 3.5,
  "Power": 168.7,
  "AcVoltage": 229.8,
  "AcCurrent": 5.6,
  "AcPower": 1286.0,
  "State": "on",
  "Mode": "inverter"
}
```

Champs attendus par `handle_inverter_topic()` :
- `Voltage` (f32) — tension DC en volts
- `Current` (f32) — courant DC en ampères
- `Power` (f32) — puissance DC en watts
- `AcVoltage` (f32) — tension AC L1 (V)
- `AcCurrent` (f32) — courant AC L1 (A)
- `AcPower` (f32) — puissance AC L1 (W) ← **affiché sur dashboard**
- `State` (string) — `"on"` ou `"off"`
- `Mode` (string) — `"inverter"`, `"charger"`, `"passthrough"`, etc.

**`santuario/system/venus`**

Produit par : energy-manager (anciennement `smartshunt.json` flow)
Fréquence : chaque nouveau message D-Bus

```json
{
  "Voltage": 48.3,
  "Current": -12.4,
  "Power": -598.0,
  "SOC": 85.5,
  "State": "discharging"
}
```

Champs :
- `Voltage` (f32) — tension batterie
- `Current` (f32) — courant (négatif = décharge) ← **affiché**
- `Power` (f32) — puissance
- `SOC` (f32) — état de charge %
- `State` (string) — `"charging"`, `"discharging"`, `"idle"`

**`santuario/meteo/venus`**

Produit par : energy-manager (anciennement `Solar_power.json` + `meteo.json` flows)
Fréquence : toutes les 25 secondes (keepalive)

```json
{
  "MpptPower": 2345.0,
  "TodaysYield": 12.5,
  "IrradianceWm2": 334.0,
  "Irradiance": 334.0
}
```

Champs :
- `MpptPower` (f32) — puissance solaire TOTALE MPPT agrégée (W) ← **utilisé**
- `TodaysYield` (f32) — production d'aujourd'hui (kWh)
- `IrradianceWm2` (f32) — irradiance capteur (W/m²)
- `Irradiance` (f32) — idem (champ backup)

### A.4 Guide d'ajout d'une nouvelle métrique (checklist générique)

> Cette procédure décrit l'ancienne méthode Node-RED (étapes 1–9 avec flows JSON). Pour l'architecture actuelle (energy-manager Rust), voir [./app-energy-manager.md](./app-energy-manager.md) et `docs/energy-manager-guide.md`. Les étapes côté daly-bms-server (structure Rust, handler MQTT, endpoint API, route) restent identiques.

Checklist générique (parties daly-bms-server, toujours valides) :

```
□ ÉTAPE A: Identifier la source de données
  - D'où vient la donnée ? (D-Bus NanoPi / Pi5 / RS485 / API externe / MQTT)
  - Quel est le chemin exact ? (topic MQTT, /sys path, registre RS485...)
  - Quelle est la fréquence de mise à jour ?

□ ÉTAPE B: Ajouter la structure Rust (state.rs)
  - Créer struct avec #[derive(Clone, Debug, Serialize, Deserialize)]
  - Ajouter Arc<RwLock<Option<MyStruct>>> à AppState
  - Ajouter on_my_struct() et my_struct_get() helpers

□ ÉTAPE C: Ajouter la source de données
  - Si MQTT → ajouter handler dans bridges/mqtt.rs (subscribe + match)
  - Si Pi5 local → ajouter tokio::spawn() polling loop dans main.rs
  - Si API externe → ajouter HTTP client dans http_clients/

□ ÉTAPE D: Créer l'endpoint API (api/system.rs ou api/<module>.rs)
  - Retourner {"connected": true/false, "data": {...}}
  - Enregistrer la route dans api/mod.rs via build_router()

□ ÉTAPE E: Mettre à jour le dashboard SSR
  - Ajouter fetch dans le JS du template Askama concerné
  - Ajouter mapping dans le rendu ECharts / ReactFlow
  - Recompiler : make build-arm (templates compilés dans le binaire)

□ ÉTAPE F: Compiler, tester, commit
  - cargo build --release -p daly-bms-server
  - Redémarrer le service
  - curl endpoint pour vérifier
  - Accès dashboard et vérifier affichage
  - git commit -m "feat(scope): description"
```

Template de structure Rust complète pour un nouveau device :

```rust
// ===================== state.rs =====================
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MyNewDevice {
    pub metric1: Option<f32>,         // Métrique principale
    pub metric2: Option<f32>,         // Métrique secondaire
    pub status: String,               // "connected" / "error"
    pub timestamp: DateTime<Utc>,
}

pub struct AppState {
    // ... champs existants ...
    pub my_device: Arc<RwLock<Option<MyNewDevice>>>,
}

impl AppState {
    pub async fn on_my_device(&self, data: MyNewDevice) {
        *self.my_device.write().await = Some(data);
    }
    pub async fn my_device_get(&self) -> Option<MyNewDevice> {
        self.my_device.read().await.clone()
    }
}

// ===================== api/system.rs =====================
pub async fn get_my_device(State(state): State<AppState>) -> impl IntoResponse {
    match state.my_device_get().await {
        Some(data) => (StatusCode::OK, Json(json!({
            "connected": data.status == "connected",
            "device": data,
        }))),
        None => (StatusCode::OK, Json(json!({
            "connected": false,
            "device": Value::Null,
        }))),
    }
}

// ===================== api/mod.rs =====================
.route("/api/v1/my/endpoint", get(system::get_my_device))

// ===================== bridges/mqtt.rs =====================
mqtt_client.subscribe("santuario/mydevice/venus", QoS::AtLeastOnce).await?;
// Dans le match :
"santuario/mydevice/venus" => handle_mydevice_topic(&state, &json).await,
```

### A.5 Dépannage spécifique dashboard

**Endpoint retourne `"connected": false`**

1. Vérifier que energy-manager publie bien sur le topic MQTT :
   ```bash
   mosquitto_sub -h 127.0.0.1 -p 1883 -t 'santuario/mydevice/venus' -v
   # Si rien pendant 30s → le topic n'est pas publié
   journalctl -u energy-manager -f | grep -i error
   ```
2. Vérifier que le handler MQTT parse correctement :
   ```bash
   journalctl -u daly-bms -f | grep -i "mqtt\|error\|parse"
   ```

**Dashboard affiche `"—"` au lieu de la valeur**

1. Vérifier que l'endpoint n'est pas 404 :
   ```bash
   curl -v http://localhost:8080/api/v1/my/endpoint
   ```
2. Vérifier que la route est enregistrée dans `api/mod.rs` via `build_router()`.
3. Vérifier dans la console navigateur (F12) :
   ```javascript
   fetch('/api/v1/my/endpoint').then(r => r.json()).then(console.log)
   ```

**Erreur de compilation Rust**

```rust
// "struct n'implémente pas Serialize" → ajouter les derives :
#[derive(Clone, Debug, Serialize, Deserialize)]

// "field does not exist" → vérifier l'initialisation dans AppState::new() :
my_device: Arc::new(RwLock::new(None)),
```

---

## Voir aussi

- [./ARCHITECTURE.md](./ARCHITECTURE.md) — Document maître, vue d'ensemble du système et index de toute la documentation.
- [./app-energy-manager.md](./app-energy-manager.md) — energy-manager (port 8081) : modules logic, règles GRL, MQTT, clients HTTP. Produit les topics `santuario/inverter/venus`, `santuario/system/venus`, `santuario/meteo/venus` consommés par daly-bms-server.
- [./app-dbus-mqtt-venus.md](./app-dbus-mqtt-venus.md) — Bridge NanoPi (armv7) : consomme les topics MQTT publiés par daly-bms-server, les enregistre comme services D-Bus Venus OS.
- [./metriques-redb-architecture.md](./metriques-redb-architecture.md) — Internals du metrics-store redb (tables, encodage, tiering, write path).
- [./metriques-promql-reference.md](./metriques-promql-reference.md) — Catalogue des métriques exposées via l'interface PromQL, conventions de labels.
- [./alertes.md](./alertes.md) — AlertEngine : règles complètes, hysteresis, configuration Telegram/SMTP.
- [./integration-materiel.md](./integration-materiel.md) — Matériel RS485 : ET112 registres Modbus, ATS CHINT protocole, PRALRAN, ajout BMS.
- [./deploiement-exploitation.md](./deploiement-exploitation.md) — Build, cross-compilation, déploiement Pi5/NanoPi, systemd, scripts.
- [./mqtt-mosquitto.md](./mqtt-mosquitto.md) — Architecture MQTT : topics, bridge Pi5→NanoPi, anti-boucle.
- [./grafana-dashboards.md](./grafana-dashboards.md) — 20 dashboards Grafana, datasource UID `daly-metrics`, provisioning.
- [./diagnostic-depannage.md](./diagnostic-depannage.md) — Dépannage transverse, netdiag réseau, debug onduleur/SmartShunt.
- `Readme.md` (racine) — Introduction projet, démarrage rapide, roadmap.
- `CLAUDE.md` (racine) — Mémoire projet : commandes rapides, règles de travail, problèmes courants.

---

## Sources consolidées

Ce document fusionne et **remplace** les anciens fichiers suivants :

`DASHBOARD_EXTENSION_GUIDE.md` (parties concernant daly-bms-server : pipeline dashboard temps réel SSR, structures de données Rust, topics MQTT, checklist ajout métrique, guide dépannage dashboard)

Les fichiers suivants ne sont **pas remplacés** par ce document (ils restent en vigueur) :
- `Readme.md` → deviendra `README.md` (introduction projet, démarrage rapide, roadmap)
- `CLAUDE.md` → reste la mémoire projet active (commandes rapides, règles de travail, changelog architecture)
