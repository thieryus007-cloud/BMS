# Intégration Matériel — Daly-BMS-Rust

> Inventaire RS485/D-Bus complet, procédures d'ajout de BMS Daly, maintenance ATS CHINT,
> intégration ET112/PRALRAN/Tasmota/Shelly et guide générique d'extension capteur/métrique.
> Fait partie de l'[architecture documentaire](./ARCHITECTURE.md).
> Dernière consolidation : 2026-06-07.

## Table des matières

- [1. Vue d'ensemble du bus RS485](#1-vue-densemble-du-bus-rs485)
- [2. Inventaire RS485 — table de référence](#2-inventaire-rs485--table-de-reference)
- [3. Services D-Bus production (NanoPi)](#3-services-d-bus-production-nanopi)
  - [3.1 Liste complète des services nominaux](#31-liste-complete-des-services-nominaux)
  - [3.2 Diagnostic rapide D-Bus](#32-diagnostic-rapide-d-bus)
- [4. Topics MQTT (préfixe `santuario/`)](#4-topics-mqtt-prefixe-santuario)
- [5. Flux de données par type d'appareil](#5-flux-de-donnees-par-type-dappareil)
  - [5.1 BMS Daly](#51-bms-daly)
  - [5.2 ET112 (compteurs énergie Modbus RTU)](#52-et112-compteurs-energie-modbus-rtu)
  - [5.3 PRALRAN (capteur irradiance RS485)](#53-pralran-capteur-irradiance-rs485)
  - [5.4 ATS CHINT (commutateur automatique de sources)](#54-ats-chint-commutateur-automatique-de-sources)
  - [5.5 LG ThinQ (PAC chauffe-eau — API cloud)](#55-lg-thinq-pac-chauffe-eau--api-cloud)
  - [5.6 Tasmota / Tongou (relais MQTT)](#56-tasmota--tongou-relais-mqtt)
  - [5.7 Shelly Pro 2PM](#57-shelly-pro-2pm)
- [6. Procédure : ajouter un BMS Daly](#6-procedure--ajouter-un-bms-daly)
  - [6.0 Choisir les identifiants](#60-choisir-les-identifiants)
  - [6.1 Matériel — régler l'adresse du BMS](#61-materiel--regler-ladresse-du-bms)
  - [6.2 Pi5 — daly-bms-server](#62-pi5--daly-bms-server)
  - [6.3 NanoPi / Venus OS — dbus-mqtt-venus](#63-nanopi--venus-os--dbus-mqtt-venus)
  - [6.4 Bridge Mosquitto — rien à faire](#64-bridge-mosquitto--rien-a-faire)
  - [6.5 Récapitulatif des fichiers](#65-recapitulatif-des-fichiers)
  - [6.6 Checklist finale](#66-checklist-finale)
  - [6.7 Rollback](#67-rollback)
  - [6.8 Pièges à éviter](#68-pieges-a-eviter)
- [7. ATS CHINT NXZB/NXZBN — Maintenance opérationnelle](#7-ats-chint-nxzbnxzbn--maintenance-operationnelle)
  - [7.1 Architecture d'intégration](#71-architecture-dintegration)
  - [7.2 Configuration matérielle](#72-configuration-materielle)
  - [7.3 Registres Modbus](#73-registres-modbus)
  - [7.4 Interfaces de contrôle](#74-interfaces-de-controle)
  - [7.5 Diagnostic et surveillance](#75-diagnostic-et-surveillance)
  - [7.6 Dépannage ATS](#76-depannage-ats)
  - [7.7 Procédures d'exploitation](#77-procedures-dexploitation)
  - [7.8 Checklist de déploiement initial](#78-checklist-de-deploiement-initial)
  - [7.9 État nominal — logs attendus](#79-etat-nominal--logs-attendus)
- [8. ET112 — Intégration et dépannage](#8-et112--integration-et-depannage)
  - [8.1 Paramètres Modbus RTU](#81-parametres-modbus-rtu)
  - [8.2 Adressage et rôles](#82-adressage-et-roles)
  - [8.3 Monophasé — phases L2/L3 fantômes](#83-monophase--phases-l2l3-fantomes)
  - [8.4 Diagnostic mbpoll](#84-diagnostic-mbpoll)
  - [8.5 Label `address` en hexadécimal](#85-label-address-en-hexadecimal)
- [9. PRALRAN — Capteur irradiance RS485](#9-pralran--capteur-irradiance-rs485)
- [10. Résumé inventaire séries temporelles](#10-resume-inventaire-series-temporelles)
- [11. Annexe historique — Guide d'extension (ancienne architecture)](#11-annexe-historique--guide-dextension-ancienne-architecture)
  - [11.1 Contexte et limites historiques](#111-contexte-et-limites-historiques)
  - [11.2 Flux de données — ancienne architecture](#112-flux-de-donnees--ancienne-architecture)
  - [11.3 Structures de données Rust (état au 2026-04-05)](#113-structures-de-donnees-rust-etat-au-2026-04-05)
  - [11.4 Topics MQTT et payloads de l'ancienne architecture](#114-topics-mqtt-et-payloads-de-lancienne-architecture)
  - [11.5 Guide générique — ajouter un appareil/métrique de bout en bout](#115-guide-generique--ajouter-un-appareilmetrique-de-bout-en-bout)
  - [11.6 Procédures détaillées d'intégration](#116-procedures-detaillees-dintegration)
  - [11.7 Dépannage extension métrique](#117-depannage-extension-metrique)
  - [11.8 Cas d'usage réels (exemples)](#118-cas-dusage-reels-exemples)
- [Voir aussi](#voir-aussi)
- [Sources consolidées](#sources-consolidees)

---

## 1. Vue d'ensemble du bus RS485

```
Bus RS485 unifié /dev/ttyUSB0 (9600-8N1)
│
├── 0x01 → BMS-360Ah (Daly)
├── 0x02 → BMS-320Ah (Daly)
├── 0x03 → BMS-620Ah (Daly)
├── 0x05 → PRALRAN irradiance (Modbus RTU)
├── 0x06 → ATS CHINT NXZB (Modbus RTU)
├── 0x07 → ET112 Micro-Onduleurs (SN 119253X)
├── 0x08 → ET112 Maison (SN 119215X)
└── 0x09 → ET112 Réseau (SN 061077X)
                │
                │ Modbus RTU / Daly UART (polling)
                ▼
daly-bms-server (Pi5 :8080)
  ├── REST API + WebSocket
  ├── metrics-store (redb, /mnt/nvme/daly-bms/metrics.redb)
  └── MQTT publish → santuario/* → broker :1883
                │
                │ MQTT bridge pi5-nanopi
                ▼
dbus-mqtt-venus (NanoPi 192.168.1.120)
  └── MQTT subscribe → D-Bus com.victronenergy.*
```

Le Pi5 est le **maître** de tous les capteurs RS485. Le NanoPi reste dédié à Venus OS et n'héberge que `dbus-mqtt-venus`. Voir [./app-daly-bms-server.md] pour le détail du serveur RS485/API et [./app-dbus-mqtt-venus.md] pour le bridge D-Bus.

---

## 2. Inventaire RS485 — table de référence

> Source de vérité : `CLAUDE.md` §5. En cas de divergence avec d'autres documents, cette table fait autorité.

Bus `/dev/ttyUSB0` — paramètres : 9600 bauds, 8N1 :

| Addr | Appareil | Type D-Bus | Topic MQTT (préfixe `santuario/`) | Instance D-Bus |
|------|----------|-----------|-----------------------------------|---------------|
| `0x01` | BMS-360Ah (Daly) | `battery.mqtt_1` | `bms/1/venus` | 151 |
| `0x02` | BMS-320Ah (Daly) | `battery.mqtt_2` | `bms/2/venus` | 152 |
| `0x03` | BMS-620Ah (Daly) | `battery.mqtt_3` | `bms/3/venus` | 153 |
| `0x05` | PRALRAN irradiance | `meteo` (singleton) | `irradiance/raw` | 40 |
| `0x06` | ATS CHINT NXZB | `switch.mqtt_1` | `switch/1/venus` | 60 |
| `0x07` | ET112-Micro-Onduleurs (SN 119253X) | `pvinverter.mqtt_7` | `pvinverter/7/venus` | 32 |
| `0x08` | ET112-Maison (SN 119215X) | `acload.mqtt_8` | `grid/8/venus` | 30 |
| `0x09` | ET112-Réseau (SN 061077X) | `grid.mqtt_9` | `grid/9/venus` | 31 |

> **Note :** L'adresse `0x06` (ATS CHINT) n'est pas listée dans la table CLAUDE.md §5 mais est documentée dans la section **ATS CHINT** plus bas dans ce document (schéma ASCII §1). Elle est réservée sur le bus.

> **Adresses déjà prises (à ne pas réutiliser)** : `0x01`, `0x02`, `0x03`, `0x05`, `0x06`, `0x07`, `0x08`, `0x09`.

### 2.1 Chemin série stable `/dev/serial/by-id` (audit 2026-06 §14)

`/dev/ttyUSB0` est un nom **instable** : après un débranchement/rebranchement
de l'adaptateur USB-RS485 (ou un glitch USB), le kernel peut ré-énumérer le
périphérique en `/dev/ttyUSB1`. `SharedBus::reopen()` rouvre alors l'ancien
chemin — qui n'existe plus, ou pire, désigne **un autre adaptateur**.

udev crée automatiquement un symlink stable par identité matérielle
(VID:PID + numéro de série) :

```bash
ls -l /dev/serial/by-id/
# ex : usb-FTDI_USB-RS485_Cable_FT0ABCDE-if00-port0 -> ../../ttyUSB0
```

Recommandation : référencer ce chemin dans `Config.toml` —

```toml
[serial]
port = "/dev/serial/by-id/usb-FTDI_USB-RS485_Cable_FT0ABCDE-if00-port0"
```

Aucun changement de code nécessaire : `reopen()` rouvre le même symlink,
qui suit le périphérique quelle que soit sa ré-énumération. (Si l'adaptateur
n'expose pas de numéro de série unique, utiliser `/dev/serial/by-path/` —
stable tant que le port USB physique ne change pas.)

---

## 3. Services D-Bus production (NanoPi)

### 3.1 Liste complète des services nominaux

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

> **Attention** : le nom D-Bus exact de l'onduleur PV Victron direct est `cgwacs_ttyUSB0_mb2`, jamais `rs485`. Voir la règle 10 de `CLAUDE.md`.

### 3.2 Diagnostic rapide D-Bus

Depuis le Pi5 (SSH vers le NanoPi) :

```bash
ssh root@192.168.1.120 "dbus -y | grep victronenergy"
```

Depuis le NanoPi directement :

```bash
# Lister tous les services Victron actifs
dbus -y | grep victronenergy

# Vérifier une batterie spécifique
dbus -y com.victronenergy.battery.mqtt_3 / GetItems | grep -E "Soc|Dc/0/Voltage|DeviceInstance"

# Vérifier le switch ATS
dbus -y com.victronenergy.switch.mqtt_1 /Position GetValue
dbus -y com.victronenergy.switch.mqtt_1 /State GetValue
dbus -y com.victronenergy.switch.mqtt_1 /Connected GetValue
```

---

## 4. Topics MQTT (préfixe `santuario/`)

Tous les topics utilisent le préfixe `santuario/`. Le bridge Mosquitto `pi5-nanopi` relaie les topics `santuario/#` du Pi5 vers le NanoPi.

| Topic MQTT complet (santuario/…) | Source (Pi5) | Service D-Bus cible |
|-----------------------------------|--------------|---------------------|
| `bms/{n}/venus` | `daly-bms-server` | `com.victronenergy.battery.mqtt_{n}` |
| `pvinverter/{n}/venus` | `daly-bms-server` (ET112) | `com.victronenergy.pvinverter.mqtt_{n}` |
| `grid/{n}/venus` | `daly-bms-server` (ET112) | `com.victronenergy.grid.mqtt_{n}` |
| `heatpump/{n}/venus` | `daly-bms-server` (ET112) | `com.victronenergy.heatpump.mqtt_{n}` |
| `heat/{n}/venus` | `energy-manager` (LG ThinQ) | `com.victronenergy.temperature.mqtt_{n}` |
| `switch/{n}/venus` | `daly-bms-server` (ATS / Tongou) | `com.victronenergy.switch.mqtt_{n}` |
| `meteo/venus` | `daly-bms-server` (PRALRAN) | `com.victronenergy.meteo` (singleton) |
| `irradiance/raw` | `daly-bms-server` (PRALRAN) | — (interne metrics-store) |

Pour l'architecture MQTT détaillée (Mosquitto natif, bridge, anti-boucle, migration Docker→natif) voir [./mqtt-mosquitto.md].

---

## 5. Flux de données par type d'appareil

### 5.1 BMS Daly

```
BMS UART (RS485 /dev/ttyUSB0, 9600-8N1)
  └── daly_bms_core::poll_loop()
        │
        ▼ on_snapshot(snap)
  AppState::on_snapshot()
    ├── ring_buffer (3600 snaps/BMS)
    └── broadcast (tokio)
          ├── MqttBridge → santuario/bms/{n}/venus (retain=true)
          │     └── dbus-mqtt-venus → com.victronenergy.battery.mqtt_{n}
          ├── metrics-store (redb)
          ├── AlertEngine
          └── WebSocket /ws/bms/{id}/stream
```

Protocole Daly : trame 13 octets (`0xA5 | ADDR | DATA_ID | DATA(8) | CHECKSUM`), 9600 bauds, checksum = somme des octets mod 256. Commandes de lecture : `0x90` (pack tension/courant/SOC), `0x91` (min/max cellule), `0x92` (températures), `0x93` (MOS/cycles), `0x94` (config), `0x95` (tensions individuelles), `0x96` (températures individuelles), `0x97` (équilibrage), `0x98` (alarmes). Commandes d'écriture : `0xD9` (MOS décharge), `0xDA` (MOS charge), `0x21` (calibration SOC), `0x00` (reset).

### 5.2 ET112 (compteurs énergie Modbus RTU)

```
ET112 Modbus RTU (0x07 / 0x08 / 0x09)
  └── daly-bms-server::et112::poll_loop()
        │
        ├── MqttBridge → santuario/pvinverter/{n}/venus
        │                 santuario/heatpump/{n}/venus
        │                 santuario/grid/{n}/venus
        └── dbus-mqtt-venus → com.victronenergy.pvinverter / acload / grid
```

Paramètres : 9600 bauds, 8N1, FC=03 (lecture registres float). Voir §8 pour le détail des adresses, du diagnostic mbpoll et du dépannage.

### 5.3 PRALRAN (capteur irradiance RS485)

```
PRALRAN RS485 (0x05)
  └── daly-bms-server::irradiance::poll_loop()
        │
        ├── MqttBridge → santuario/meteo/venus
        │                 santuario/irradiance/raw
        └── dbus-mqtt-venus → com.victronenergy.meteo (inst. 40)
```

Voir §9 pour le détail.

### 5.4 ATS CHINT (commutateur automatique de sources)

```
ATS CHINT RS485 (0x06)
  └── daly-bms-server::ats (lecture FC=03 + commandes FC=06, 5s)
        │
        ├── API REST /api/v1/ats/*
        ├── Dashboard SSR /dashboard/ats
        └── MqttBridge → santuario/switch/1/venus (retain=true)
              └── dbus-mqtt-venus → com.victronenergy.switch.mqtt_1 (inst. 60)
```

Voir §7 pour la maintenance complète (registres, commandes, dépannage).

### 5.5 LG ThinQ (PAC chauffe-eau — API cloud)

```
LG ThinQ API (PAC chauffe-eau)
  └── energy-manager::http_clients::lg_thinq
        │
        └── MqttBridge → santuario/heat/{n}/venus
              └── dbus-mqtt-venus → com.victronenergy.temperature.mqtt_{n}
```

Authentification : `LG_BEARER_TOKEN` et `LG_API_KEY` dans `/etc/daly-bms/.env`. En cas d'échec → `journalctl -u energy-manager -n 50`.

### 5.6 Tasmota / Tongou (relais MQTT)

Les switches Tongou (`mqtt_2` à `mqtt_6`, instances 61–65) publient leurs états via MQTT. `daly-bms-server` gère les topics Tasmota côté polling et API :

```
Tasmota / Tongou (WiFi MQTT)
  └── daly-bms-server::tasmota
        ├── API REST /api/v1/tasmota/:id/status
        ├── API REST POST /api/v1/tasmota/:id/control
        └── MqttBridge → santuario/switch/{n}/venus
              └── dbus-mqtt-venus → com.victronenergy.switch.mqtt_{n}
```

Instances Tongou : `switch.mqtt_2` (inst. 61), `switch.mqtt_3` (inst. 62), `switch.mqtt_4` (inst. 63), `switch.mqtt_5` (inst. 64), `switch.mqtt_6` / tongou_3ACC34 (inst. 65).

Pour le catalogue détaillé des métriques Tasmota → [./metriques-promql-reference.md].

### 5.7 Shelly Pro 2PM

```
Shelly Pro 2PM (WiFi HTTP/MQTT)
  └── daly-bms-server::shelly
        ├── API REST /api/v1/shelly/:id/status
        └── API REST POST /api/v1/shelly/:id/channel/:ch/control
```

2 canaux, métriques : état (ON/OFF) × 2, puissance × 2, énergie totale × 2. Pour les endpoints REST complets → [./app-daly-bms-server.md].

Les deux canaux pilotent **un DEYE chacun** ; ils sont commandés ensemble par la règle `deye_command` (cf. [./app-energy-manager.md] §4.3) — voir le mécanisme de curtailment ci-dessous.

### 5.8 Curtailment PV — AC-couplé vs DC-couplé

L'installation a **deux sources PV** avec deux mécanismes de réduction de production **fondamentalement différents**. Comprendre cette dissymétrie explique pourquoi un relais Shelly est nécessaire côté DEYE alors que les MPPT n'en ont pas besoin.

| | **DC-couplé** — MPPT Victron (inst. 273/289) | **AC-couplé** — micro-onduleurs DEYE (AC Out) |
|---|---|---|
| Bridage | Régulation de charge (courant) | Décalage de fréquence |
| Signal | Tension bus DC / ordres GX | Fréquence AC Out (≈ 50,2 → 51,5 Hz) |
| Granularité | Continue, progressive | Continue jusqu'au **trip dur à 51,5 Hz** |
| Transitoire | Aucun à-coup | Brutal au trip → micro-coupures |
| Levier logiciel | (DVCC, **désactivé ici**) | Relais Shelly (ce projet) |

**Côté MPPT (DC-couplé)** — pourquoi pas de relais :
- Un MPPT est une source de courant régulée vers une **consigne de tension**. Batterie pleine ⇒ il tient la tension en **sortant du point de puissance maximale (MPP)** (vers Voc) → courant réduit, sans à-coup. C'est la régulation 3 étages **Bulk → Absorption → Float** (fin d'absorption au *tail current*, 2 A par défaut).
- **DVCC** (Distributed Voltage and Current Control), s'il était activé, ferait distribuer par le GX/Cerbo les limites **CVL/CCL/DCL** (issues du BMS) à tous les chargeurs, qui « *disable their own charging algorithms and follow the battery's instructions directly* ». **DVCC n'est pas activé sur cette installation** et n'est **pas requis** : le bridage MPPT fonctionne via la courbe de charge propre du contrôleur.

**Côté DEYE (AC-couplé)** — pourquoi un relais :
- Ce sont des onduleurs réseau **non pilotables en courant**. Le seul levier Victron est le **décalage de fréquence** du MultiPlus, et leur **auto-coupure dure à 51,5 Hz** provoque les micro-coupures. → relais Shelly pour couper proprement **avant** (cf. §4.3 energy-manager).

**Signal « batterie pleine » exploitable sans DVCC** : l'état de charge des MPPT est publié sur MQTT **indépendamment de DVCC** (la télémétrie n'a pas besoin du contrôle) :
```
N/{portal_id}/solarcharger/{273,289}/State   →  EnergyState.mppt_273.state / mppt_289.state
```
| Code | État | Interprétation côté DEYE |
|---|---|---|
| 0 | Off | MPPT inactif (nuit) |
| 2 | Fault | défaut |
| 3 | Bulk | batterie **en charge** (pas pleine) → DEYE utiles |
| 4 | Absorption | batterie **presque pleine** |
| 5 | Float | **batterie pleine** → excédent imminent |
| 6 | Storage | **batterie pleine** (float prolongé) |
| 7 | Equalize | égalisation (manuel) |
| 11 | Other (Hub-1) | — |
| 252 | External control | piloté par DVCC (non utilisé ici) |

`Float`/`Storage` ⇒ batterie pleine, MPPT déjà bridé : c'est le **signal racine** qui *précède* la montée en fréquence côté DEYE.

**Exploité par `deye_command`** (cf. [./app-energy-manager.md] §4.3, désactivable via `mppt_cut_enabled`) : dès qu'**un** MPPT atteint un état de `mppt_full_states` (défaut `[4,5,6]` = Absorption/Float/Storage), maintenu `mppt_cut_delay_secs` (10 s), les DEYE sont coupés (relais Shelly) pour **terminer la charge sur le seul MPPT** sans atteindre la fréquence haute. La fréquence (51,0/51,3 Hz) reste en filet de sécurité. Couper dès `Absorption` (4) plutôt que `Float` (5) coupe **plus tôt** (batterie ~85-90 %) — compromis : moins d'auto-consommation DEYE pendant le palier, mais aucune micro-coupure. Ajuster `mppt_full_states=[5,6]` pour ne couper qu'à `Float`.

---

## 6. Procédure : ajouter un BMS Daly

> **Bonne nouvelle : aucune recompilation nécessaire.** Le support BMS est générique (piloté par la config). C'est une opération **config-only** des deux côtés. (Contrairement à un nouveau *type* de device qui demande du code.)

Exemple fil rouge utilisé dans cette procédure : adresse `0x03`, batterie « BMS-628Ah », `mqtt_index = 3`, `device_instance = 153`.

### 6.0 Choisir les identifiants

| Paramètre | Valeur exemple | Règle |
|---|---|---|
| Adresse RS485 (Modbus) | `0x03` | Unique sur le bus `/dev/ttyUSB0` (déjà pris : 0x01, 0x02 BMS · 0x05 PRALRAN · 0x06 ATS · 0x07/08/09 ET112) |
| `mqtt_index` | `3` | Topic → `santuario/bms/3/venus` |
| `device_instance` | `153` | **Unique** sur le D-Bus Victron. Suite logique 151→152→**153**. ⚠ NE PAS réutiliser 141/142 (legacy dbus-mqtt-battery) ni 143 (réservé à l'exemple batterie virtuelle agrégée 628Ah) |
| Service D-Bus résultant | `com.victronenergy.battery.mqtt_3` | dérivé de `mqtt_index` |

### 6.1 Matériel — régler l'adresse du BMS

1. Régler l'**adresse Modbus du BMS à 3** (via l'app Daly Bluetooth/Smart BMS, ou l'outil série constructeur). Deux BMS ne doivent **jamais** partager la même adresse sur le bus.
2. Câbler le BMS sur le bus RS485 partagé `/dev/ttyUSB0` (A/B en parallèle des autres BMS, masse commune).
3. (Optionnel, à froid) Vérifier la réponse Modbus **avant** d'intégrer :
   ```bash
   sudo systemctl stop daly-bms          # libère le port
   mbpoll -m rtu -a 3 -b 9600 -t 3:float -r 1 -c 1 /dev/ttyUSB0
   sudo systemctl start daly-bms
   ```

### 6.2 Pi5 — daly-bms-server

#### 6.2.1 Éditer `Config.toml` (dépôt)

**a) Ajouter l'adresse à la liste de scan** `[serial].addresses` — ⚠ **étape la plus souvent oubliée** : sans elle, le BMS n'est jamais interrogé.

```toml
[serial]
addresses = ["0x01", "0x02", "0x03"]   # ← ajouter "0x03"
```

**b) Ajouter le bloc `[[bms]]`** (décommenter celui déjà présent) :

```toml
[[bms]]
address         = "0x03"
name            = "BMS-628Ah"
capacity_ah     = 628.0
max_charge_a    = 200.0
max_discharge_a = 120.0
mqtt_index      = 3
device_instance = 153
```

#### 6.2.2 Déployer la config sur le Pi5

> `deploy-pi5.sh` **n'écrase pas** `/etc/daly-bms/config.toml` s'il existe (protège la prod). La copie est donc **manuelle**.

```bash
cd ~/Daly-BMS-Rust
# (commit/push les modifs de Config.toml d'abord, puis sur le Pi5 :)
make sync
sudo cp Config.toml /etc/daly-bms/config.toml
sudo systemctl restart daly-bms
```

#### 6.2.3 Vérifier côté Pi5

```bash
# Le BMS est lu (status RS485)
curl -s http://localhost:8080/api/v1/bms/3/status | jq '.data.soc, .data.dc'
# Métriques en base (clé = adresse)
curl -s 'http://localhost:8080/api/v1/query?query=bms_power' | jq '.data.result[].metric'
# Topic MQTT publié
timeout 5 mosquitto_sub -h 127.0.0.1 -t 'santuario/bms/3/venus' -v
# Santé RS485 (le nouveau doit apparaître, sans timeout)
curl -s http://localhost:8080/api/v1/monitor/rs485-health | jq
```

### 6.3 NanoPi / Venus OS — dbus-mqtt-venus

#### 6.3.1 Éditer `nanoPi/config-nanopi.toml` (dépôt)

Ajouter un bloc `[[bms]]` **identique** (mêmes `mqtt_index`/`device_instance`) :

```toml
[[bms]]
address         = "0x03"
name            = "BMS-628Ah"
mqtt_index      = 3
device_instance = 153
capacity_ah     = 628.0
max_charge_a    = 200.0
max_discharge_a = 120.0
```

#### 6.3.2 Déployer la config sur le NanoPi

> **Pas de recompilation** : seul le fichier de config change. (Si tu modifies du code Rust un jour → `make install-venus-v7`, et **jamais** `target-cpu=native` pour l'armv7, cf. CLAUDE.md §8 SIGILL.)

```bash
# Depuis le Pi5, après make sync :
scp nanoPi/config-nanopi.toml root@192.168.1.120:/data/daly-bms/config.toml
ssh root@192.168.1.120 'svc -t /service/dbus-mqtt-venus'
```

#### 6.3.3 Vérifier côté NanoPi

```bash
# Le nouveau service D-Bus batterie doit apparaître
ssh root@192.168.1.120 'dbus -y | grep battery'
#   attendu : com.victronenergy.battery.mqtt_1 / mqtt_2 / mqtt_3
# Valeurs exposées
ssh root@192.168.1.120 'dbus -y com.victronenergy.battery.mqtt_3 / GetItems | grep -E "Soc|Dc/0/Voltage|DeviceInstance"'
```

Puis dans **VRM / console GX** : la nouvelle batterie apparaît dans la liste des appareils (instance 153). Rafraîchir si cache.

### 6.4 Bridge Mosquitto — rien à faire

La règle egress couvre déjà **tous** les index BMS :

```
topic santuario/bms/# out 1 "" ""
```

`santuario/bms/3/venus` est donc automatiquement bridgé Pi5 → NanoPi. Aucune modification de `contrib/mosquitto/mosquitto.conf`.

### 6.5 Récapitulatif des fichiers

| Fichier | Modif | Déploiement |
|---|---|---|
| `Config.toml` (Pi5) | `[serial].addresses` + bloc `[[bms]]` | `sudo cp … /etc/daly-bms/config.toml` + restart daly-bms |
| `nanoPi/config-nanopi.toml` | bloc `[[bms]]` | `scp … :/data/daly-bms/config.toml` + `svc -t` |
| `contrib/mosquitto/mosquitto.conf` | — (déjà `bms/# out`) | — |
| Code Rust | — | — (pas de build) |

### 6.6 Checklist finale

- [ ] Adresse Modbus du BMS réglée à `3`, câblé sur `/dev/ttyUSB0`
- [ ] `Config.toml` : `"0x03"` dans `[serial].addresses` **et** bloc `[[bms]]`
- [ ] `device_instance = 153` (unique — pas 141/142/143)
- [ ] `Config.toml` copié vers `/etc/daly-bms/config.toml` + `daly-bms` redémarré
- [ ] `config-nanopi.toml` : bloc `[[bms]]` ajouté, scp + `svc -t`
- [ ] `curl /api/v1/bms/3/status` renvoie des données
- [ ] `dbus -y | grep battery` montre `battery.mqtt_3`
- [ ] Batterie visible dans VRM (instance 153)
- [ ] `make sync` à jour, commit/push de `Config.toml` + `config-nanopi.toml`

### 6.7 Rollback

```bash
# Pi5
sudo cp /etc/daly-bms/config.toml.bak-<date> /etc/daly-bms/config.toml   # si backup
#   ou retirer "0x03" de [serial].addresses + le bloc [[bms]], puis :
sudo systemctl restart daly-bms
# NanoPi (retirer le bloc [[bms]] 0x03, redéployer)
scp nanoPi/config-nanopi.toml root@192.168.1.120:/data/daly-bms/config.toml
ssh root@192.168.1.120 'svc -t /service/dbus-mqtt-venus'
```

Le service D-Bus `battery.mqtt_3` disparaît au redémarrage de `dbus-mqtt-venus` (un D-Bus ne survit pas au restart). Penser à purger le retained si besoin :

```bash
mosquitto_pub -h 127.0.0.1 -t santuario/bms/3/venus -r -n
ssh root@192.168.1.120 'mosquitto_pub -h localhost -t santuario/bms/3/venus -r -n'
```

### 6.8 Pièges à éviter

| Piège | Conséquence | Solution |
|---|---|---|
| Oublier `"0x03"` dans `[serial].addresses` | BMS jamais interrogé (aucune donnée, mais pas d'erreur visible) | Ajouter à la liste de scan |
| `device_instance` dupliqué (141/142/143) | Conflit D-Bus / batterie fantôme dans VRM | Utiliser **153** |
| Copier `Config.toml` via `deploy-pi5.sh` | Ne s'applique PAS (préservation prod) | `sudo cp` manuel |
| Recompiler « par précaution » | Inutile + risque (armv7 SIGILL si `target-cpu=native`) | Config-only, pas de build |
| Adresse Modbus du BMS pas réglée à 3 | Pas de réponse / collision avec un autre device | Régler via app Daly avant câblage |
| `mqtt_index`/`device_instance` différents entre Pi5 et NanoPi | Topic publié ≠ topic attendu → batterie absente de VRM | Garder **identiques** des deux côtés |

---

## 7. ATS CHINT NXZB/NXZBN — Maintenance opérationnelle

> Document de référence pour l'exploitation, la surveillance et le dépannage du commutateur automatique de sources CHINT intégré dans le système ESS Santuario.

### 7.1 Architecture d'intégration

```
┌─────────────────────────────────────────────────────────────────────┐
│  Bus RS485 unifié  /dev/ttyUSB0  (9600-8N1)                        │
│                                                                     │
│  Addr 0x01 → BMS-360Ah (Daly)                                      │
│  Addr 0x02 → BMS-320Ah (Daly)                                      │
│  Addr 0x03 → BMS-620Ah (Daly)                                      │
│  Addr 0x05 → Irradiance PRALRAN                                     │
│  Addr 0x06 → ATS CHINT NXZB  ◄── ici                              │
│  Addr 0x07 → ET112 Micro-Onduleurs                                  │
│  Addr 0x08 → ET112 Maison                                           │
│  Addr 0x09 → ET112 Réseau                                           │
└──────────────────┬──────────────────────────────────────────────────┘
                   │ Modbus RTU FC=03 (polling 5s)
                   │ Modbus RTU FC=06 (commandes à la demande)
                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  daly-bms-server (Pi5 — systemd)                                    │
│                                                                     │
│  ATS polling loop (5s)                                              │
│    ├── API REST  GET  /api/v1/ats/status                            │
│    ├── API REST  POST /api/v1/ats/remote_on|off|force_*             │
│    ├── Dashboard SSR  /dashboard/ats  (schéma unifilaire SVG)       │
│    └── MQTT publish   santuario/switch/1/venus  (retain=true)       │
└──────────────────┬──────────────────────────────────────────────────┘
                   │ MQTT → broker NanoPi 192.168.1.120:1883
                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  dbus-mqtt-venus (NanoPi)                                           │
│                                                                     │
│  com.victronenergy.switch.mqtt_1  (device_instance = 60)           │
│    ├── /Position   : 0 = AC1/Réseau, 1 = AC2/Onduleur              │
│    ├── /State      : 0 = inactif, 1 = actif, 2 = alerte            │
│    ├── /Connected  : 0 ou 1                                         │
│    ├── /CustomName : "ATS CHINT NXZB"                               │
│    └── /ProductName: "ATS CHINT"                                    │
└─────────────────────────────────────────────────────────────────────┘
                   │ D-Bus Venus OS
                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  Victron GX (NanoPi) → Venus OS GUI + VRM Portal                   │
│  Affichage : "ATS CHINT NXZB" dans liste des équipements           │
│  Position source visible dans l'énergie (AC1 / AC2)                │
└─────────────────────────────────────────────────────────────────────┘
```

### 7.2 Configuration matérielle

#### Câblage RS485

| Fil | Signal | Connecteur ATS |
|-----|--------|----------------|
| A+  | Data + | Borne A (RS485) |
| B-  | Data − | Borne B (RS485) |
| GND | Masse  | Borne GND (si présente) |

> Câbler sur le même bus que les autres appareils RS485.
> Terminaison 120 Ω en bout de ligne si l'ATS est le dernier appareil.

#### Configuration de l'ATS (bouton Setup physique)

| Paramètre | Valeur requise | Défaut usine |
|-----------|----------------|--------------|
| Adresse Modbus | 6 | 1 |
| Baud rate | 9600 | 9600 |
| **Parité** | **None (8N1)** | **Even (8E1)** — **déjà modifié en prod** |

> ⚠ **CRITIQUE** : La parité DOIT être configurée à **None** sur l'ATS.
> Le bus unifié Pi5 tourne en 8N1. Si l'ATS reste en Even, il ne répondra pas.

#### Configurer la parité via Modbus (si accès temporaire en Even)

Si l'ATS est encore en parité Even, le reconfigurer via un PC Windows avec le logiciel Carlo Gavazzi UCS ou via `mbpoll` sur un port dédié temporaire :

```bash
# Sur Pi5 — arrêter daly-bms avant d'utiliser mbpoll
sudo systemctl stop daly-bms

# Écrire parité = None (0x0000) dans le registre 0x000E
mbpoll -m rtu -a 6 -b 9600 -P E -t 4 -r 0x000E /dev/ttyUSB0 0

# Redémarrer
sudo systemctl start daly-bms
```

### 7.3 Registres Modbus

#### Lecture FC=03

##### Bloc A — Tensions et identification (0x0006, 13 registres)

| Registre | Nom | Unité | Description |
|----------|-----|-------|-------------|
| 0x0006 | v1a | V | Tension Source 1 phase A |
| 0x0007 | v1b | V | Tension Source 1 phase B |
| 0x0008 | v1c | V | Tension Source 1 phase C |
| 0x0009 | v2a | V | Tension Source 2 phase A |
| 0x000A | v2b | V | Tension Source 2 phase B |
| 0x000B | v2c | V | Tension Source 2 phase C |
| 0x000C | sw_version | — | Version logicielle (÷100) |
| 0x000D | freq | — | Fréquences hi=f1/lo=f2 (MN only) |
| 0x000E | parity_code | — | 0=None, 1=Odd, 2=Even |
| 0x000F | max1_v | V | Tension max enregistrée Source 1 |
| 0x0010 | — | — | (réservé) |
| 0x0011 | — | — | (réservé) |
| 0x0012 | max2_v | V | Tension max enregistrée Source 2 |

##### Bloc C — Compteurs (0x0015, 3 registres)

| Registre | Nom | Description |
|----------|-----|-------------|
| 0x0015 | cnt1 | Compteur commutations Source1→Source2 |
| 0x0016 | cnt2 | Compteur commutations Source2→Source1 |
| 0x0017 | runtime_h | Durée de fonctionnement totale (heures) |

##### Bloc D — Statut (0x004F, 2 registres)

| Registre | Nom | Description |
|----------|-----|-------------|
| 0x004F | pwr_status | Bitfield statut tensions par phase |
| 0x0050 | sw_status | Bitfield statut commutation |

**Décodage pwr_status (0x004F)** — 2 bits par phase, 3 phases × 2 sources :

| Bits | Phase | Source | Valeur 0 | Valeur 1 | Valeur 2 | Valeur 3 |
|------|-------|--------|----------|----------|----------|----------|
| 1:0 | A | 1 | Normal | Sous-tension | Sur-tension | Erreur |
| 3:2 | B | 1 | Normal | Sous-tension | Sur-tension | Erreur |
| 5:4 | C | 1 | Normal | Sous-tension | Sur-tension | Erreur |
| 7:6 | A | 2 | Normal | Sous-tension | Sur-tension | Erreur |
| 9:8 | B | 2 | Normal | Sous-tension | Sur-tension | Erreur |
| 11:10 | C | 2 | Normal | Sous-tension | Sur-tension | Erreur |

**Décodage sw_status (0x0050)** :

| Bit | Nom | 0 | 1 |
|-----|-----|---|---|
| 0 | sw_mode | Manuel | Auto |
| 3 | sw1_bit | SW1 fermé | SW1 ouvert |
| 4 | sw2_bit | SW2 fermé | SW2 ouvert |
| 7:5 | fault | 0=Aucun, 1=Incendie, 2=Surcharge moteur, 3=Disj. I, 4=Disj. II, 5=Fermeture anormale, 6=Phase anormale I, 7=Phase anormale II |
| 8 | remote | Télécommande Off | Télécommande On |

**Position de commutation** (déduite des bits 3 et 4) :

| sw1_bit | sw2_bit | État |
|---------|---------|------|
| 0 | 0 | Position centrale / neutre (middle-off) |
| 0 | 1 | **SW1 fermé → Source 1 (Onduleur) active** |
| 1 | 0 | **SW2 fermé → Source 2 (Réseau) active** |
| 1 | 1 | Les deux ouverts (transition en cours) |

##### Bloc E — Config Modbus (0x0100, 2 registres)

| Registre | Nom | Description |
|----------|-----|-------------|
| 0x0100 | modbus_addr | Adresse Modbus configurée sur l'ATS |
| 0x0101 | modbus_baud | Code baud (0=4800, 1=9600, 2=19200, 3=38400) |

##### Bloc F — Paramètres MN uniquement (0x2065, 9 registres)

> Lire ce bloc en premier pour détecter le modèle : succès = MN, timeout = BN.

| Registre | Nom | Unité | Description |
|----------|-----|-------|-------------|
| 0x2065 | uv1 | V | Seuil sous-tension Source 1 |
| 0x2066 | uv2 | V | Seuil sous-tension Source 2 |
| 0x2067 | ov1 | V | Seuil sur-tension Source 1 |
| 0x2068 | ov2 | V | Seuil sur-tension Source 2 |
| 0x2069 | t1 | s | Délai commutation Source1→Source2 |
| 0x206A | t2 | s | Délai retour Source1 |
| 0x206B | t3 | s | Délai commutation Source2→Source1 |
| 0x206C | t4 | s | Délai retour Source2 |
| 0x206D | op_mode | — | Mode : 0=Auto-réarm, 1=Auto-no-réarm, 2=Secours, 3=Générateur, 4=Gén-no-réarm, 5=Gén-secours |

#### Écriture FC=06

| Registre | Valeur | Commande | Prérequis |
|----------|--------|----------|-----------|
| 0x2800 | 0x0004 | Activer télécommande | — |
| 0x2800 | 0x0000 | Désactiver télécommande | — |
| 0x2700 | 0x0000 | Forcer Source 1 (Onduleur) | Télécommande active |
| 0x2700 | 0x00AA | Forcer Source 2 (Réseau) | Télécommande active |
| 0x2700 | 0x00FF | Forcer double déclenché | Télécommande active |

> **Ordre obligatoire pour forçage** :
> 1. `POST /api/v1/ats/remote_on` (activer télécommande)
> 2. `POST /api/v1/ats/force_source1` ou `force_source2` ou `force_double`
> 3. `POST /api/v1/ats/remote_off` (rendre l'ATS en Auto)

### 7.4 Interfaces de contrôle

#### Dashboard Web (Pi5)

URL : `http://192.168.1.141:8080/dashboard/ats`

**Panneau gauche — Schéma unifilaire SVG** :
- Source 1 (Onduleur) avec tension phase A
- Source 2 (Réseau) avec tension phase A
- SW1 et SW2 : FERMÉ (vert) / OUVERT (rouge)
- Mode AUTO / MANUEL
- Source active alimentant la charge
- Compteurs de commutations et runtime

**Panneau droit — État détaillé** :
- Toutes les tensions par phase (3 phases × 2 sources)
- Code de défaut et statut
- Paramètres MN : seuils UV/OV, délais T1-T4
- Fréquences, version logicielle, adresse Modbus

**Commandes disponibles** (boutons) :
- Télécommande ON / OFF
- Forcer Onduleur (Source 1)
- Forcer Réseau (Source 2)
- Forcer Double Déclenché

> La page se rafraîchit automatiquement toutes les **3 secondes** via polling JS.
> Les boutons de commande envoient immédiatement la commande Modbus FC=06.

#### API REST (Pi5)

```bash
# Lecture état ATS
curl http://192.168.1.141:8080/api/v1/ats/status

# Activer télécommande
curl -X POST http://192.168.1.141:8080/api/v1/ats/remote_on

# Désactiver télécommande (retour en Auto)
curl -X POST http://192.168.1.141:8080/api/v1/ats/remote_off

# Forcer sur Source 1 (Onduleur)
curl -X POST http://192.168.1.141:8080/api/v1/ats/force_source1

# Forcer sur Source 2 (Réseau)
curl -X POST http://192.168.1.141:8080/api/v1/ats/force_source2

# Forcer double déclenché
curl -X POST http://192.168.1.141:8080/api/v1/ats/force_double
```

Pour l'inventaire complet des endpoints ATS → [./app-daly-bms-server.md].

#### Venus OS GUI (NanoPi)

L'ATS apparaît sous le nom **"ATS CHINT NXZB"** dans la liste des équipements.

Chemin VRM : `Device list → Switches → ATS CHINT NXZB (instance 60)`

Valeurs visibles dans Venus OS :
- `/Position` : `AC Input 1` (Réseau) ou `AC Input 2` (Onduleur)
- `/State` : `Inactive` (0) / `Active` (1) / `Alerted` (2)
- `/Connected` : `Connected` (1) / `Disconnected` (0)
- `/CustomName` : "ATS CHINT NXZB"

### 7.5 Diagnostic et surveillance

#### Vérification état (Pi5)

```bash
# Logs du service en direct
journalctl -u daly-bms -f | grep -i ats

# Vérifier que le polling tourne
journalctl -u daly-bms --since "5 minutes ago" | grep -i "ATS\|0x06"

# Appel API direct
curl -s http://192.168.1.141:8080/api/v1/ats/status | python3 -m json.tool
```

#### Vérification MQTT (Pi5 ou NanoPi)

```bash
# Voir le payload ATS publié
mosquitto_sub -h 192.168.1.120 -p 1883 -t 'santuario/switch/1/venus' -v

# Résultat attendu :
# santuario/switch/1/venus {"Position":0,"State":1,"ProductName":"ATS CHINT","CustomName":"ATS CHINT NXZB"}
```

#### Vérification D-Bus Venus OS (NanoPi)

```bash
ssh root@192.168.1.120

# Service présent ?
dbus -y | grep switch

# Valeurs D-Bus
dbus -y com.victronenergy.switch.mqtt_1 /Position GetValue
dbus -y com.victronenergy.switch.mqtt_1 /State GetValue
dbus -y com.victronenergy.switch.mqtt_1 /Connected GetValue
dbus -y com.victronenergy.switch.mqtt_1 / GetItems
```

#### Test Modbus direct (Pi5 — STOP service avant)

```bash
# Arrêter le service pour libérer le port
sudo systemctl stop daly-bms

# Lire les tensions (registre 0x0006, 6 regs) depuis adresse 6
mbpoll -m rtu -a 6 -b 9600 -P N -t 3 -r 6 -c 6 /dev/ttyUSB0

# Lire le statut de commutation (0x0050)
mbpoll -m rtu -a 6 -b 9600 -P N -t 3 -r 0x0050 -c 1 /dev/ttyUSB0

# Redémarrer
sudo systemctl start daly-bms
```

### 7.6 Dépannage ATS

#### Problème : "Aucune donnée ATS" dans le dashboard

**Symptôme** : `/api/v1/ats/status` retourne 404, logs montrent des timeouts.

**Causes possibles** :

| Cause | Diagnostic | Solution |
|-------|------------|----------|
| ATS encore en parité Even | `mbpoll ... -P E` répond, `... -P N` ne répond pas | Configurer parité None sur l'ATS |
| Adresse Modbus incorrecte | Scanner toutes les adresses | Voir §7.6.1 |
| Câble RS485 débranché | Aucun appareil ne répond | Vérifier câble A/B |
| ATS hors tension | LED verte éteinte | Alimenter l'ATS |
| Terminaison 120Ω manquante | Réponses intermittentes | Ajouter résistance en bout de ligne |

##### 7.6.1 Scanner l'adresse réelle de l'ATS

```bash
sudo systemctl stop daly-bms

# Scanner toutes les adresses 1-15
mbpoll -m rtu -a 1:15 -b 9600 -P N -t 3 -r 6 -c 1 /dev/ttyUSB0
# → L'adresse qui retourne ~230 V (tension réseau) est l'adresse de l'ATS

sudo systemctl start daly-bms
```

Si une adresse répond, mettre à jour `/etc/daly-bms/config.toml` :

```toml
[ats]
address = <nouvelle_adresse>
```

Puis `sudo systemctl restart daly-bms`.

#### Problème : Commandes ignorées (pas d'effet)

**Cause la plus fréquente** : Télécommande non activée avant le forçage.

**Solution** :

```bash
# Toujours dans cet ordre :
curl -X POST http://192.168.1.141:8080/api/v1/ats/remote_on
# Attendre 1 seconde
curl -X POST http://192.168.1.141:8080/api/v1/ats/force_source1
# Quand terminé, repasser en auto :
curl -X POST http://192.168.1.141:8080/api/v1/ats/remote_off
```

#### Problème : ATS absent du Venus OS GUI

**Causes** :

1. **MQTT non publié** — vérifier `santuario/switch/1/venus` sur le broker
2. **dbus-mqtt-venus ne tourne pas** — `svstat /service/dbus-mqtt-venus` sur NanoPi
3. **`[[switches]]` absent de config-nanopi.toml** — vérifier le fichier sur NanoPi

**Vérification config NanoPi** :

```bash
ssh root@192.168.1.120 "cat /data/daly-bms/config.toml" | grep -A 5 switches
```

Doit contenir :

```toml
[[switches]]
mqtt_index      = 1
name            = "ATS CHINT"
custom_name     = "ATS CHINT NXZB"
device_instance = 60
```

Si absent, ajouter et redémarrer :

```bash
scp nanoPi/config-nanopi.toml root@192.168.1.120:/data/daly-bms/config.toml
ssh root@192.168.1.120 "svc -t /service/dbus-mqtt-venus"
```

#### Problème : Code de défaut persistant

| Code défaut | Signification | Action |
|-------------|---------------|--------|
| Interconnexion incendie | Entrée incendie déclenchée | Vérifier/réinitialiser détecteur incendie |
| Surcharge moteur | Moteur ATS en surcharge | Inspection mécanique — contacter maintenance |
| Disjonction I (Onduleur) | Disjoncteur côté Onduleur déclenché | Réarmer disjoncteur aval onduleur |
| Disjonction II (Réseau) | Disjoncteur côté Réseau déclenché | Réarmer disjoncteur aval réseau |
| Fermeture anormale | Fermeture non commandée | Inspection électrique urgente |
| Phase anormale I/II | Anomalie séquence phases | Vérifier rotation phases source |

### 7.7 Procédures d'exploitation

#### Basculement manuel d'urgence (Réseau → Onduleur)

```bash
# Via API
curl -X POST http://192.168.1.141:8080/api/v1/ats/remote_on
curl -X POST http://192.168.1.141:8080/api/v1/ats/force_source1

# Vérifier position
curl -s http://192.168.1.141:8080/api/v1/ats/status | python3 -c \
  "import sys,json; d=json.load(sys.stdin)['data']; print(d['active_source'], d['sw1_closed'], d['sw2_closed'])"
```

#### Retour en mode automatique

```bash
curl -X POST http://192.168.1.141:8080/api/v1/ats/remote_off
```

#### Test hebdomadaire de commutation

```bash
echo "=== Test commutation ATS ==="
echo "Source initiale :"
curl -s http://192.168.1.141:8080/api/v1/ats/status | python3 -c \
  "import sys,json; d=json.load(sys.stdin)['data']; print(f\"  Source active : {d['active_source']}, SW1={d['sw1_closed']}, SW2={d['sw2_closed']}\")"

echo "Activation télécommande + forçage Source 1..."
curl -sX POST http://192.168.1.141:8080/api/v1/ats/remote_on
sleep 2
curl -sX POST http://192.168.1.141:8080/api/v1/ats/force_source1
sleep 3

echo "Après forçage Source 1 :"
curl -s http://192.168.1.141:8080/api/v1/ats/status | python3 -c \
  "import sys,json; d=json.load(sys.stdin)['data']; print(f\"  Source active : {d['active_source']}, SW1={d['sw1_closed']}, SW2={d['sw2_closed']}\")"

echo "Retour Auto..."
curl -sX POST http://192.168.1.141:8080/api/v1/ats/remote_off
sleep 3
echo "Fin test."
curl -s http://192.168.1.141:8080/api/v1/ats/status | python3 -c \
  "import sys,json; d=json.load(sys.stdin)['data']; print(f\"  Source finale : {d['active_source']}, Défaut : {d['fault']}\")"
```

### 7.8 Checklist de déploiement initial

#### Sur Pi5

- [ ] ATS câblé sur le bus RS485 `/dev/ttyUSB0` (même bus que BMS/ET112)
- [ ] Parité ATS configurée à **None** (8N1) — registre 0x000E = 0
- [ ] Adresse Modbus ATS = 6 (ou adapter `[ats].address` dans config.toml)
- [ ] `Config.toml` → `[ats]` avec `enabled = true`
- [ ] Copier vers production : `sudo cp Config.toml /etc/daly-bms/config.toml`
- [ ] Recompiler si besoin : `make build-arm`
- [ ] Déployer : `sudo cp target/aarch64.../daly-bms-server /usr/local/bin/`
- [ ] Redémarrer : `sudo systemctl restart daly-bms`
- [ ] Vérifier logs : `journalctl -u daly-bms -f | grep -i ats`
- [ ] Tester API : `curl http://192.168.1.141:8080/api/v1/ats/status`
- [ ] Tester dashboard : `http://192.168.1.141:8080/dashboard/ats`

#### Sur NanoPi

- [ ] `[[switches]]` présent dans `/data/daly-bms/config.toml` (mqtt_index=1, device_instance=60)
- [ ] `svc -t /service/dbus-mqtt-venus` pour redémarrer le bridge
- [ ] Vérifier D-Bus : `dbus -y | grep switch`
- [ ] Vérifier dans Venus OS GUI : Device list → Switches → "ATS CHINT NXZB"

### 7.9 État nominal — logs attendus

```
INFO daly_bms_server::ats::poll: ATS CHINT polling démarré (bus RS485 unifié) addr=0x06 name=ATS CHINT NXZB
INFO daly_bms_server::ats::poll: Modèle ATS détecté addr=0x06 model=MN
INFO daly_bms_server::bridges::mqtt: ATS CHINT publié → santuario/switch/1/venus
```

**Payload MQTT nominal** :

```json
{
  "Position": 0,
  "State": 1,
  "ProductName": "ATS CHINT",
  "CustomName": "ATS CHINT NXZB"
}
```

(`Position=0` = Source 2/Réseau active, `State=1` = actif)

---

## 8. ET112 — Intégration et dépannage

### 8.1 Paramètres Modbus RTU

| Paramètre | Valeur |
|-----------|--------|
| Baud rate | 9600 |
| Format | 8N1 |
| Protocole | Modbus RTU |
| Fonction de lecture | FC=03 (registres float) |

### 8.2 Adressage et rôles

| Adresse | Appareil | Numéro de série | Rôle | Type D-Bus | Instance |
|---------|----------|-----------------|------|-----------|---------|
| `0x07` | ET112-Micro-Onduleurs | SN 119253X | Mesure production micro-onduleurs PV | `pvinverter.mqtt_7` | 32 |
| `0x08` | ET112-Maison | SN 119215X | Mesure consommation maison | `acload.mqtt_8` | 30 |
| `0x09` | ET112-Réseau | SN 061077X | Mesure import/export réseau EDF | `grid.mqtt_9` | 31 |

### 8.3 Monophasé — phases L2/L3 fantômes

Les ET112 sont des compteurs **monophasés**. Le service `grid_service` sur le NanoPi n'expose que les phases présentes via `/Ac/NumberOfPhases` (dérivé du payload). Si L2/L3 persistent à 0 W dans VRM → rafraîchir VRM (cache console).

### 8.4 Diagnostic mbpoll

> ⚠ `mbpoll` sans réponse → `daly-bms` monopolise le port. Toujours stopper le service d'abord.

```bash
# Arrêter le service pour libérer le port
sudo systemctl stop daly-bms

# Scanner toutes les adresses 1-15 (trouver un ET112 ou tout autre device)
mbpoll -m rtu -a 1:15 -b 9600 -t 3:float -r 1 -c 1 /dev/ttyUSB0

# Lire spécifiquement les registres d'un ET112 (adresse 7, registre 1, 1 valeur)
mbpoll -m rtu -a 7 -b 9600 -t 3:float -r 1 -c 1 /dev/ttyUSB0

# Redémarrer le service après le diagnostic
sudo systemctl start daly-bms
```

### 8.5 Label `address` en hexadécimal

Le backend écrit les labels `address` en **hexadécimal** dans `redb_writes.rs::write_et112`. Les requêtes PromQL doivent utiliser ce format :

```
address="0x07"    # ← CORRECT
address="7"       # ← INCORRECT (décimal → 0 série retournée)
```

Vérification :

```bash
curl -s 'localhost:8080/api/v1/query?query=et112_power_w' | jq '.data.result[].metric'
# Doit afficher : {"__name__": "et112_power_w", "address": "0x07", ...}
```

Pour le catalogue complet des métriques ET112 → [./metriques-promql-reference.md].

---

## 9. PRALRAN — Capteur irradiance RS485

| Paramètre | Valeur |
|-----------|--------|
| Adresse RS485 | `0x05` |
| Type D-Bus | `com.victronenergy.meteo` (singleton) |
| Topic MQTT interne | `santuario/irradiance/raw` |
| Topic MQTT D-Bus | `santuario/meteo/venus` |
| Instance D-Bus | 40 |

Flux :

```
PRALRAN RS485 (0x05)
  └── daly-bms-server::irradiance::poll_loop()
        ├── santuario/irradiance/raw         (lecture brute)
        └── santuario/meteo/venus            (agrégé avec TodaysYield)
              └── dbus-mqtt-venus → com.victronenergy.meteo (inst. 40)
```

Le service D-Bus `com.victronenergy.meteo` expose l'irradiance PRALRAN et la production du jour (`TodaysYield`). Le widget météo "Température: -" dans Venus OS est une limitation de Venus OS — inévitable, non fixable.

API de lecture : `GET /api/v1/irradiance/status`.

---

## 10. Résumé inventaire séries temporelles

Estimation de production du système complet (données issues de l'inventaire des séries, cf. [`metriques-redb-architecture.md`](./metriques-redb-architecture.md)) :

| Appareil | Séries temporelles |
|----|---|
| 3 × Daly BMS 16 cellules | ~120 (40 métriques × 3) |
| 3 × ET112 (micro-onduleurs, maison, réseau) | ~18 (6 métriques × 3) |
| 1 × Capteur irradiance PRALRAN | 1 |
| 2 × Chauffe-eau & Climatisation LG | 8 (4 métriques × 2) |
| 1 × ATS CHINT | ~10 |
| 2 × MPPT Victron | ~10 (5 métriques × 2) |
| 1 × SmartShunt Victron | 6 |
| 1 × Easysolar II GX Victron | 9 |
| 1 × Capteur Température/Humidité | 2 |
| 6 × Switches Tasmota Tongou | 30 (5 métriques × 6) |
| 1 × Switch Shelly Pro 2PM | 6 |
| **Total** | **~200–240** |

> La valeur exacte dépend des labels supplémentaires dans le metrics-store redb. Pour le catalogue détaillé des métriques et leurs conventions de labels → [./metriques-promql-reference.md].

---

## 11. Annexe historique — Guide d'extension (ancienne architecture)

> Statut : MIGRATION TERMINÉE — section historique, conservée pour référence.
>
> Ce guide décrit l'ancienne architecture energy-manager (flows JSON, ancienne stack Node-RED-like). La référence actuelle pour energy-manager est [`app-energy-manager.md`](./app-energy-manager.md). La section §11.5 (checklist générique d'extension) et les modèles de code Rust restent valables comme référence de principe pour tout développement futur.
>
> Version : 2.0 — Date : 2026-04-05 — Statut : **OBSOLÈTE** (remplacé par energy-manager Rust)

### 11.1 Contexte et limites historiques

Au départ (avant l'implémentation de l'architecture energy-manager), le système n'avait **aucune** remontée temps réel des métriques Victron vers le Pi5 :
- Les batteries BMS s'affichaient (via RS485 direct)
- Les MPPT, SmartShunt, Onduleur Victron restaient invisibles
- Le dashboard affichait "En attente de données" pour ces appareils
- Aucune intégration de D-Bus Victron vers l'API web

La solution implémentée a créé une architecture complète en 4 étapes : NanoPi D-Bus → flows energy-manager → MQTT → daly-bms-server AppState → REST API + WebSocket → dashboard.

### 11.2 Flux de données — ancienne architecture

```
NanoPi D-Bus (Victron)
    ↓ energy-manager flows (ancienne version JSON)
MQTT Topics (santuario/*)
    ↓ daly-bms-server
AppState (données en mémoire)
    ↓ REST API + WebSocket
Dashboard (ReactFlow — ancienne UI)
```

**Composants implémentés à l'époque** :

| Composant | Rôle | Technologie |
|-----------|------|-------------|
| `inverter.json` | Agréger MultiPlus D-Bus → MQTT | energy-manager (ancien) |
| `smartshunt.json` | Agréger SmartShunt D-Bus → MQTT | energy-manager (ancien) |
| `Solar_power.json` | Agréger MPPT D-Bus → MQTT | energy-manager (ancien) |
| `VenusInverter struct` | Stocker inverter en Rust | Serde |
| `MQTT handlers` | Parser JSON MQTT → Rust | async/tokio |
| `API endpoints` | Exposer via REST | Axum |
| `visualization.html` | Afficher en temps réel | ReactFlow |

### 11.3 Structures de données Rust (état au 2026-04-05)

Ces structures illustrent le modèle de données Venus côté daly-bms-server (toujours valables comme référence) :

```rust
// Onduleur (MultiPlus Victron)
pub struct VenusInverter {
    pub voltage_v: Option<f32>,           // DC voltage
    pub current_a: Option<f32>,           // DC current
    pub power_w: Option<f32>,             // DC power
    pub ac_output_voltage_v: Option<f32>, // AC output voltage
    pub ac_output_current_a: Option<f32>, // AC output current
    pub ac_output_power_w: Option<f32>,   // AC output power ← AFFICHÉ
    pub state: String,                    // "on" / "off" / "fault"
    pub mode: String,                     // "inverter" / "charger" / "passthrough"
    pub timestamp: DateTime<Utc>,
}

// SmartShunt (Victron Battery Monitor)
pub struct VenusSmartShunt {
    pub voltage_v: Option<f32>,      // Battery voltage
    pub current_a: Option<f32>,      // Battery current ← AFFICHÉ
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
    pub address: String,             // Device address
    pub name: String,                // "Outdoor" / "Battery" / etc
    pub temperature_c: f32,          // Temperature value ← AFFICHÉ
    pub type_num: i32,               // 0=battery 1=fridge 2=generic 3=room 4=outdoor
    pub status: String,              // "connected" / "disconnected"
    pub timestamp: DateTime<Utc>,
}
```

### 11.4 Topics MQTT et payloads de l'ancienne architecture

#### santuario/inverter/venus

Publié par : `inverter.json` (ancienne version energy-manager)

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

#### santuario/system/venus

Publié par : `smartshunt.json`

```json
{
  "Voltage": 48.3,
  "Current": -12.4,
  "Power": -598.0,
  "SOC": 85.5,
  "State": "discharging"
}
```

#### santuario/meteo/venus

Publié par : `Solar_power.json` + `meteo.json`, fréquence : toutes les 25 secondes (keepalive).

```json
{
  "MpptPower": 2345.0,
  "TodaysYield": 12.5,
  "IrradianceWm2": 334.0,
  "Irradiance": 334.0
}
```

Champs clés :
- `MpptPower` (f32) — puissance solaire totale MPPT (273 + 289 W)
- `TodaysYield` (f32) — production d'aujourd'hui en kWh
- `IrradianceWm2` (f32) — irradiance du capteur
- `Irradiance` (f32) — idem (backup field name)

### 11.5 Guide générique — ajouter un appareil/métrique de bout en bout

Ce guide générique reste valable pour comprendre l'architecture bout-en-bout, même si les flows JSON ont été remplacés par des modules Rust dans energy-manager.

#### Checklist générique

```
□ ÉTAPE 1: Identifier la source
  - D'où vient la donnée? (D-Bus NanoPi / Pi5 / RS485 / API externe)
  - Quel est le chemin exact? (dbus path / /sys path / topic MQTT)
  - Quelle est la fréquence de mise à jour?

□ ÉTAPE 2: Ajouter la structure Rust
  - Créer struct dans state.rs (ou fichier dédié)
  - Ajouter Arc<RwLock<>> à AppState
  - Ajouter on_*() et *_get() helpers

□ ÉTAPE 3: Ajouter la source de données
  - Si RS485 → ajouter module dans daly-bms-server (poll_loop, Modbus RTU)
  - Si NanoPi D-Bus → ajouter module dans energy-manager (logic/)
  - Si Pi5 → ajouter tokio::spawn() polling loop dans main.rs
  - Si MQTT → ajouter handler dans bridges/mqtt.rs
  - Si API externe → ajouter HTTP client dans http_clients/

□ ÉTAPE 4: Créer l'API endpoint
  - Ajouter handler dans api/system.rs ou api/<module>.rs
  - Retourner {"connected": true/false, "data": {...}}
  - Enregistrer route dans api/mod.rs

□ ÉTAPE 5: Écrire dans le metrics-store (redb)
  - Ajouter la série dans redb_writes.rs
  - Nommer les labels de façon cohérente (address en hex si applicable)
  - Vérifier avec curl /api/v1/query?query=<metric_name>

□ ÉTAPE 6: Mettre à jour le dashboard (si applicable)
  - Ajouter le fetch dans fetchAll() (templates visualization.html)
  - Ajouter le nœud / widget dans le template SSR correspondant

□ ÉTAPE 7: Si besoin D-Bus NanoPi → étendre dbus-mqtt-venus
  - Ajouter le service dans crates/dbus-mqtt-venus/src/
  - Ajouter le bloc de config dans nanoPi/config-nanopi.toml
  - Voir ./app-dbus-mqtt-venus.md pour les détails

□ ÉTAPE 8: Compiler et tester
  - make build-arm (daly-bms-server)
  - Redémarrer le service
  - curl endpoint pour vérifier
  - Accès dashboard et vérifier affichage

□ ÉTAPE 9: Commit et push
  - git add (fichiers concernés, jamais .env)
  - git commit -m "feat(scope): description"
  - git push origin <branche>
```

#### Template de structure Rust complète (modèle)

```rust
// ===================== state.rs =====================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MyNewDevice {
    pub metric1: Option<f32>,         // Metrique principale
    pub metric2: Option<f32>,         // Metrique secondaire
    pub status: String,               // "connected" / "error"
    pub timestamp: DateTime<Utc>,     // Quand mis à jour
}

// Ajouter au AppState struct
pub struct AppState {
    // ... existing fields ...
    pub my_device: Arc<RwLock<Option<MyNewDevice>>>,
}

// Ajouter les helpers
impl AppState {
    pub async fn on_my_device(&self, data: MyNewDevice) {
        *self.my_device.write().await = Some(data);
        info!("Updated my_device: {} status={}", data.metric1.unwrap_or(0.0), data.status);
    }

    pub async fn my_device_get(&self) -> Option<MyNewDevice> {
        self.my_device.read().await.clone()
    }
}

// ===================== api/system.rs =====================

pub async fn get_my_device(State(state): State<AppState>) -> impl IntoResponse {
    match state.my_device_get().await {
        Some(data) => (
            StatusCode::OK,
            Json(json!({
                "connected": data.status == "connected",
                "device": data,
            })),
        ),
        None => (
            StatusCode::OK,
            Json(json!({
                "connected": false,
                "device": Value::Null,
            })),
        ),
    }
}

// ===================== api/mod.rs =====================

.route("/api/v1/my/endpoint", get(system::get_my_device))
```

#### Procédure energy-manager — MQTT (modèle JSON, ancienne architecture)

> Ce JSON est un exemple de flow de l'ancienne version energy-manager (avant migration Rust). Conservé à titre de référence pour comprendre la logique de transformation MQTT.

```json
[
  {
    "id": "node-id-1",
    "type": "mqtt in",
    "name": "Input from MQTT",
    "topic": "source/topic/path",
    "qos": "0",
    "datatype": "json",
    "x": 150, "y": 100,
    "wires": [["function-node-1"]]
  },
  {
    "id": "function-node-1",
    "type": "function",
    "name": "Parse and aggregate",
    "func": "const value1 = msg.payload.field1;\nconst value2 = msg.payload.field2;\nflow.set('my_value1', value1);\nflow.set('my_value2', value2);\nnode.status({fill:'blue', text:`Value1: ${value1}`});\nreturn msg;",
    "outputs": 1,
    "x": 350, "y": 100,
    "wires": [["publish-node-1"]]
  },
  {
    "id": "publish-node-1",
    "type": "function",
    "name": "Create MQTT payload",
    "func": "const val1 = flow.get('my_value1') || 0;\nconst val2 = flow.get('my_value2') || 0;\nconst msg_out = {\n    topic: 'santuario/mydevice/venus',\n    payload: JSON.stringify({\n        Metric1: val1,\n        Metric2: val2,\n        Status: val1 > 0 ? 1 : 0,\n        Timestamp: new Date().toISOString()\n    }),\n    retain: true\n};\nreturn msg_out;",
    "outputs": 1,
    "x": 550, "y": 100,
    "wires": [["mqtt-out-1"]]
  },
  {
    "id": "mqtt-out-1",
    "type": "mqtt out",
    "name": "Output to Mosquitto",
    "topic": "",
    "qos": "1",
    "retain": true,
    "broker": "pi5_mqtt_broker",
    "x": 750, "y": 100,
    "wires": []
  }
]
```

### 11.6 Procédures détaillées d'intégration

#### Scénario : ajouter une métrique depuis le NanoPi D-Bus

**Étape 1 — Vérifier que la métrique existe sur D-Bus** :

```bash
ssh root@192.168.1.120

# Lister tous les services Victron
dbus -y | grep victronenergy

# Explorer un service spécifique
dbus -y com.victronenergy.generator.XX / GetItems | grep -i temperature
```

**Étapes 2 à 6** : voir la checklist générique §11.5 ci-dessus.

**Étape 7 — Déployer** :

```bash
# Sur Pi5
cd ~/Daly-BMS-Rust
make build-arm
sudo systemctl stop daly-bms
sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/
sudo systemctl start daly-bms
# Vérifier
curl http://localhost:8080/api/v1/<nouvel_endpoint> | jq '.'
```

#### Scénario : ajouter une métrique locale Pi5 (ex. température CPU)

```rust
// Dans main.rs tokio::spawn:
let state_clone = state.clone();
tokio::spawn(async move {
    loop {
        // Lecture température CPU toutes les 10 secondes
        if let Ok(temp_str) = fs::read_to_string("/sys/class/thermal/thermal_zone0/temp") {
            if let Ok(temp_millidegrees) = temp_str.trim().parse::<f32>() {
                let temp_c = temp_millidegrees / 1000.0;
                state_clone.on_system_temperature(SystemTemperature {
                    cpu_temp_c: temp_c,
                    source: "/sys/class/thermal/thermal_zone0/temp".to_string(),
                    timestamp: Utc::now(),
                }).await;
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }
});
```

### 11.7 Dépannage extension métrique

#### Nouveau endpoint retourne `"connected": false`

1. **energy-manager ne publie pas le topic** :
   ```bash
   journalctl -u energy-manager -n 50 | grep -i error
   mosquitto_sub -h 192.168.1.120 -p 1883 -t 'santuario/mydevice/venus' -v
   # Si rien n'apparaît pendant 30s → le topic n'est pas publié
   ```

2. **Handler MQTT n'a pas parsé le JSON** :
   ```bash
   journalctl -u daly-bms -f | grep -i error
   # Chercher des erreurs de parsing JSON dans le handler MQTT
   ```

3. **AppState n'a pas reçu la donnée** — ajouter un log temporaire dans le handler :
   ```rust
   info!("Received MQTT message: {:?}", json);
   ```

#### Dashboard affiche "—" au lieu de la valeur

1. **API endpoint 404** :
   ```bash
   curl -v http://localhost:8080/api/v1/my/endpoint
   # Si 404, vérifier la route dans api/mod.rs
   ```

2. **API retourne null** :
   ```bash
   curl http://localhost:8080/api/v1/my/endpoint | jq '.'
   # Si .device est null → debug MQTT handlers
   ```

3. **Erreur de compilation courante — struct n'implémente pas Serialize** :
   ```rust
   // Solution : ajouter derive macros
   #[derive(Clone, Debug, Serialize, Deserialize)]
   pub struct MyDevice {
       // fields
   }
   ```

4. **Champ manquant dans AppState initialization** :
   ```rust
   let app_state = AppState {
       // ...
       my_device: Arc::new(RwLock::new(None)),  // ← à ajouter
       // ...
   };
   ```

### 11.8 Cas d'usage réels (exemples)

#### Cas 1 : Intégrer un onduleur PV Fronius (API cloud)

```rust
// Dans main.rs tokio::spawn:
let state_clone = state.clone();
tokio::spawn(async move {
    let client = reqwest::Client::new();
    loop {
        match client
            .get("https://api.fronius.com/v1/GetPowerFlowRealtimeData.json")
            .query(&[("Symo", "SERIAL123")])
            .send()
            .await
        {
            Ok(resp) => {
                if let Ok(data) = resp.json::<FroniusResponse>().await {
                    let pv = VenusPvInverter {
                        power_w: Some(data.Body.Data.PAC.Value),
                        yield_today_kwh: Some(data.Body.Data.DailyEnergy.Value / 1000.0),
                        status: "OK".to_string(),
                        timestamp: Utc::now(),
                    };
                    state_clone.on_venus_pv_inverter(pv).await;
                }
            }
            Err(e) => error!("Fronius API error: {}", e),
        }
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
});
```

#### Cas 2 : Intégrer un Shelly H&T (WiFi thermomètre, ancienne méthode flow)

> Modèle JSON historique (ancienne architecture flows) :

```javascript
// Dans energy-manager (ancienne version) :
[
  {
    "id": "shelly-http",
    "type": "http request",
    "name": "Fetch Shelly",
    "method": "GET",
    "url": "http://192.168.1.50/status",
    "x": 150, "y": 100,
    "wires": [["parse-shelly"]]
  },
  {
    "id": "parse-shelly",
    "type": "function",
    "name": "Parse Shelly JSON",
    "func": "const temp = msg.payload.tmp?.tC || 0;\nconst humidity = msg.payload.hum?.value || 0;\nconst battery = msg.payload.bat?.value || 100;\nflow.set('shelly_temp', temp);\nflow.set('shelly_humidity', humidity);\nflow.set('shelly_battery', battery);\nreturn msg;",
    "x": 350, "y": 100,
    "wires": [["publish-shelly"]]
  },
  {
    "id": "publish-shelly",
    "type": "function",
    "name": "Create MQTT",
    "func": "const msg_out = {\n    topic: 'santuario/shelly_sensor/venus',\n    payload: JSON.stringify({\n        Temperature: flow.get('shelly_temp'),\n        Humidity: flow.get('shelly_humidity'),\n        BatteryPercent: flow.get('shelly_battery')\n    }),\n    retain: true\n};\nreturn msg_out;",
    "x": 550, "y": 100,
    "wires": [["mqtt-publish"]]
  }
]
```

#### Cas 3 : Intégrer un compteur Linky (Teleinfo RS485)

```rust
// Ajouter à main.rs (tokio::spawn):
use tokio_serial::{SerialPort, SerialPortBuilder};

let state_clone = state.clone();
tokio::spawn(async move {
    if let Ok(port) = SerialPortBuilder::new("/dev/ttyUSB2", 1200)
        .timeout(Duration::from_secs(5))
        .open_native()
    {
        // Lire les trames Teleinfo
        // Parser et extraire les valeurs de puissance
        // Mettre à jour AppState
    }
});
```

---

## Voir aussi

- [./app-daly-bms-server.md] — Serveur principal Pi5 : RS485, protocole Daly, API REST/WS, détail des endpoints ATS/ET112/BMS.
- [./app-dbus-mqtt-venus.md] — Bridge NanoPi : intégration device MQTT → D-Bus Venus OS (zbus), déploiement armv7, services D-Bus par type.
- [./metriques-promql-reference.md] — Catalogue complet des métriques, conventions de labels (address hex), requêtes PromQL.
- [./mqtt-mosquitto.md] — Architecture MQTT : Mosquitto natif, topics, bridge NanoPi, anti-boucle.
- [./deploiement-exploitation.md] — Workflow complet Pi5 + NanoPi, procédures systemd.
- [./ARCHITECTURE.md] — Vue d'ensemble système, index de toute la documentation.
- [`app-energy-manager.md`](./app-energy-manager.md) — Modifier/ajouter/retirer une fonctionnalité dans energy-manager Rust actuel.

---

## Sources consolidées

Ce document fusionne et **remplace** les anciens fichiers suivants :
`docs/AJOUT-BMS.md`, `docs/ATS_CHINT_MAINTENANCE.md`, `DASHBOARD_EXTENSION_GUIDE.md` (parties matériel et extension de bout en bout).
