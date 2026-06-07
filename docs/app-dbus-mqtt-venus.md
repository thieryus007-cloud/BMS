# dbus-mqtt-venus — Bridge MQTT → D-Bus Venus OS — Daly-BMS-Rust

> Document de référence du binaire `dbus-mqtt-venus` : bridge Rust pur (zbus) qui
> expose tous les capteurs de l'installation sur le bus D-Bus de Venus OS (NanoPi
> ARMv7), en souscrivant aux topics MQTT publiés par le Pi5.
> Fait partie de l'[architecture documentaire](./ARCHITECTURE.md).
> Dernière consolidation : 2026-06-07.

## Table des matières

- [1. Vue d'ensemble et rôle](#1-vue-densemble-et-role)
- [2. Architecture du bridge](#2-architecture-du-bridge)
  - [2.1 Flux de données global](#21-flux-de-donnees-global)
  - [2.2 Structure interne du binaire](#22-structure-interne-du-binaire)
  - [2.3 Supervision des tâches critiques — spawn_critical](#23-supervision-des-taches-critiques--spawn_critical)
- [3. Interface D-Bus Venus OS — com.victronenergy.BusItem](#3-interface-d-bus-venus-os--comvictronenergybusitem)
  - [3.1 Méthodes exposées](#31-methodes-exposees)
  - [3.2 Signal ItemsChanged](#32-signal-itemschanged)
  - [3.3 Objets racine et feuilles](#33-objets-racine-et-feuilles)
  - [3.4 Point critique — enregistrement des chemins au démarrage](#34-point-critique--enregistrement-des-chemins-au-demarrage)
- [4. Watchdog MQTT et keepalive D-Bus](#4-watchdog-mqtt-et-keepalive-d-bus)
- [5. Inventaire complet des devices — mapping topics → D-Bus](#5-inventaire-complet-des-devices--mapping-topics--d-bus)
  - [5.1 Tableau de synthèse](#51-tableau-de-synthese)
  - [5.2 Batteries Daly (battery)](#52-batteries-daly-battery)
  - [5.3 Capteurs de température (temperature)](#53-capteurs-de-temperature-temperature)
  - [5.4 Pompe à chaleur / chauffe-eau (heatpump)](#54-pompe-a-chaleur--chauffe-eau-heatpump)
  - [5.5 Capteur météo / irradiance (meteo)](#55-capteur-meteo--irradiance-meteo)
  - [5.6 Switches / ATS CHINT et Tongou (switch)](#56-switches--ats-chint-et-tongou-switch)
  - [5.7 Compteurs réseau — grid et acload](#57-compteurs-reseau--grid-et-acload)
  - [5.8 Onduleurs PV / compteurs ET112 (pvinverter)](#58-onduleurs-pv--compteurs-et112-pvinverter)
  - [5.9 Platform backup/restore Pi5 (platform)](#59-platform-backuprestore-pi5-platform)
- [6. Configuration TOML — référence complète](#6-configuration-toml--reference-complete)
  - [6.1 Section [mqtt]](#61-section-mqtt)
  - [6.2 Section [venus]](#62-section-venus)
  - [6.3 Sections batteries [[bms]]](#63-sections-batteries-bms)
  - [6.4 Section [heat] et [[sensors]]](#64-section-heat-et-sensors)
  - [6.5 Section [heatpump] et [[heatpumps]]](#65-section-heatpump-et-heatpumps)
  - [6.6 Section [meteo]](#66-section-meteo)
  - [6.7 Section [switch] et [[switches]]](#67-section-switch-et-switches)
  - [6.8 Section [grid] et [[grids]]](#68-section-grid-et-grids)
  - [6.9 Section [pvinverter] et [[pvinverters]]](#69-section-pvinverter-et-pvinverters)
  - [6.10 Section [platform]](#610-section-platform)
  - [6.11 Fichier de production nanoPi/config-nanopi.toml](#611-fichier-de-production-nanopiconfig-nanopitoml)
- [7. Payloads MQTT — format JSON par type de device](#7-payloads-mqtt--format-json-par-type-de-device)
  - [7.1 Payload batterie (VenusPayload)](#71-payload-batterie-venuspayload)
  - [7.2 Payload température (HeatPayload)](#72-payload-temperature-heatpayload)
  - [7.3 Payload heatpump (HeatpumpPayload)](#73-payload-heatpump-heatpumppayload)
  - [7.4 Payload météo (MeteoPayload)](#74-payload-meteo-meteopayload)
  - [7.5 Payload switch (SwitchPayload)](#75-payload-switch-switchpayload)
  - [7.6 Payload grid/acload (GridPayload)](#76-payload-gridacload-gridpayload)
  - [7.7 Payload pvinverter (PvinverterPayload)](#77-payload-pvinverter-pvinverterpayload)
  - [7.8 Payload platform (PlatformPayload)](#78-payload-platform-platformpayload)
- [8. Procédure pas-à-pas — intégrer un nouveau type de device](#8-procedure-pas-a-pas--integrer-un-nouveau-type-de-device)
  - [8.1 Fichiers Rust à créer ou modifier](#81-fichiers-rust-a-creer-ou-modifier)
  - [8.2 Étapes détaillées](#82-etapes-detaillees)
  - [8.3 Règle de configuration du bridge Mosquitto](#83-regle-de-configuration-du-bridge-mosquitto)
- [9. Build et déploiement ARMv7](#9-build-et-deploiement-armv7)
  - [9.1 Contrainte CRITIQUE — jamais target-cpu=native pour ARMv7](#91-contrainte-critique--jamais-target-cpunative-pour-armv7)
  - [9.2 Prérequis cross-compilation](#92-prerequis-cross-compilation)
  - [9.3 Commandes Make](#93-commandes-make)
  - [9.4 Déploiement manuel pas-à-pas](#94-deploiement-manuel-pas-a-pas)
  - [9.5 Script install-venus.sh — détail des étapes](#95-script-install-venussh--detail-des-etapes)
  - [9.6 Structure des fichiers sur le NanoPi](#96-structure-des-fichiers-sur-le-nanopi)
  - [9.7 Init system — daemontools (svc/svstat)](#97-init-system--daemontools-svcsvstat)
  - [9.8 Persistance au reboot](#98-persistance-au-reboot)
- [10. Particularités et cas spéciaux](#10-particularites-et-cas-speciaux)
  - [10.1 grid_service — monophasé et /Ac/NumberOfPhases](#101-grid_service--monophase-et-acnumberofphases)
  - [10.2 meteo — singleton sans index](#102-meteo--singleton-sans-index)
  - [10.3 Température — limitation Venus OS](#103-temperature--limitation-venus-os)
  - [10.4 switch — command_topic et contrôle ON/OFF depuis la console Venus](#104-switch--command_topic-et-controle-onoff-depuis-la-console-venus)
  - [10.5 Batterie 620Ah virtuelle (BMS-3 agrégé)](#105-batterie-620ah-virtuelle-bms-3-agrege)
  - [10.6 Instances D-Bus réservées — héritage dbus-mqtt-battery Python](#106-instances-d-bus-reservees--heritage-dbus-mqtt-battery-python)
- [11. Commandes de vérification et diagnostic](#11-commandes-de-verification-et-diagnostic)
  - [11.1 État du service runit](#111-etat-du-service-runit)
  - [11.2 Vérification D-Bus par service](#112-verification-d-bus-par-service)
  - [11.3 Vérification MQTT sur NanoPi](#113-verification-mqtt-sur-nanopi)
  - [11.4 Logs du service Rust](#114-logs-du-service-rust)
  - [11.5 Ressources système NanoPi](#115-ressources-systeme-nanopi)
- [12. Dépannage NanoPi](#12-depannage-nanopi)
  - [12.1 Service D-Bus non visible](#121-service-d-bus-non-visible)
  - [12.2 /Connected = 0 (device déconnecté dans VRM)](#122-connected--0-device-deconnecte-dans-vrm)
  - [12.3 Crash-loop SIGILL (exit 132) — architecture mismatch](#123-crash-loop-sigill-exit-132--architecture-mismatch)
  - [12.4 Exec format error — mauvaise architecture](#124-exec-format-error--mauvaise-architecture)
  - [12.5 scp échoue avec "Failure"](#125-scp-echoue-avec-failure)
  - [12.6 name already taken on the bus](#126-name-already-taken-on-the-bus)
  - [12.7 Symlink Venus disparu après mise à jour firmware](#127-symlink-venus-disparu-apres-mise-a-jour-firmware)
  - [12.8 sv introuvable (commande inconnue)](#128-sv-introuvable-commande-inconnue)
  - [12.9 logread non fonctionnel (BusyBox)](#129-logread-non-fonctionnel-busybox)
  - [12.10 ps aux non supporté (BusyBox)](#1210-ps-aux-non-supporte-busybox)
  - [12.11 Run script incorrect — crash loop](#1211-run-script-incorrect--crash-loop)
  - [12.12 Permission denied sur install-venus.sh](#1212-permission-denied-sur-install-venussh)
  - [12.13 energy-manager ne démarre pas après redémarrage Pi5](#1213-energy-manager-ne-demarre-pas-apres-redemarrage-pi5)
- [13. Annexe — Paramètres Victron switch complets](#13-annexe--parametres-victron-switch-complets)
- [14. Annexe historique — driver Python dbus-mqtt-battery](#14-annexe-historique--driver-python-dbus-mqtt-battery)
- [Voir aussi](#voir-aussi)
- [Sources consolidées](#sources-consolidees)

---

## 1. Vue d'ensemble et rôle

`dbus-mqtt-venus` est le **binaire unique sur le NanoPi** (Venus OS, NanoPi Neo3,
architecture ARMv7 32-bit). Il souscrit à tous les topics MQTT publiés par le Pi5 et
enregistre les services D-Bus correspondants sur le bus système Venus OS, rendant tous
les capteurs visibles dans l'interface VRM Portal, la console locale Venus et
`systemcalc-py` (calcul DVCC charge/décharge).

Caractéristiques principales :

- Un seul processus, **~5–8 Mo RAM** (binaire statique musl, zéro dépendance système).
- Implémenté en **Rust pur** avec la crate `zbus` (pas de `libdbus-1` système).
- Gère **9 types de device** : battery, temperature, heatpump, meteo, switch, grid,
  acload, pvinverter, platform.
- Watchdog MQTT (30s) + keepalive D-Bus (25s) pour chaque service.
- Supervision `spawn_critical` : si une boucle de bridge s'arrête, le process s'arrête
  → redémarrage propre par daemontools.

Ce binaire **remplace intégralement** l'ancien driver Python `dbus-mqtt-battery` pour
les batteries, et étend la couverture à tous les autres types de capteurs. Voir
[section 14](#14-annexe-historique--driver-python-dbus-mqtt-battery) pour l'historique.

---

## 2. Architecture du bridge

### 2.1 Flux de données global

```
[Capteurs RS485 Pi5]      [energy-manager Pi5]     [API cloud Pi5]
  BMS Daly (RS485)          Open-Meteo               LG ThinQ
  ET112 (Modbus RTU)        ATS/Tongou (MQTT)
  PRALRAN irradiance
        │                         │                       │
        ▼                         ▼                       ▼
  daly-bms-server :8080     energy-manager :8081
        │                         │
        │ MQTT publish             │ MQTT publish
        │ santuario/{type}/{n}/venus   santuario/heat/{n}/venus
        ▼                         ▼
  Mosquitto Pi5 :1883 (mosquitto-broker.service)
        │
        │ Bridge MQTT out (Pi5 → NanoPi)
        │ topics : santuario/bms/# out
        │           santuario/pvinverter/# out
        │           santuario/grid/# out
        │           santuario/switch/# out
        │           santuario/meteo/# out
        │           santuario/heat/# out
        │           santuario/heatpump/# out
        │           santuario/platform/# out
        ▼
  Mosquitto NanoPi :1883 (broker local 127.0.0.1)
        │
        │ MQTT subscribe
        ▼
  dbus-mqtt-venus (Rust, zbus pur) — NanoPi ARMv7
        │
        │ zbus / D-Bus system bus
        ▼
  Venus OS D-Bus
        ├─ com.victronenergy.battery.mqtt_1    BMS-360Ah  [151]
        ├─ com.victronenergy.battery.mqtt_2    BMS-320Ah  [152]
        ├─ com.victronenergy.battery.mqtt_3    BMS-620Ah  [153]
        ├─ com.victronenergy.temperature.mqtt_1 Temp ext. [20]
        ├─ com.victronenergy.heatpump.mqtt_1   (optionnel)
        ├─ com.victronenergy.meteo             Irradiance [40]
        ├─ com.victronenergy.switch.mqtt_1     ATS CHINT  [60]
        ├─ com.victronenergy.switch.mqtt_2…6   Tongou 1-5 [61-65]
        ├─ com.victronenergy.acload.mqtt_8     ET112-Maison [30]
        ├─ com.victronenergy.grid.mqtt_9       ET112-Réseau [31]
        ├─ com.victronenergy.pvinverter.mqtt_7 ET112-PV   [32]
        └─ com.victronenergy.platform          Pi5 backup [50]
                │
                ▼
        systemcalc-py ── VRM Portal ── Venus GUI ── hub4-control (DVCC)
```

### 2.2 Structure interne du binaire

```
crates/dbus-mqtt-venus/src/
├── main.rs                  ← Point d'entrée : démarrage séquentiel de tous les bridges
├── config.rs                ← Chargement VenusServiceConfig depuis config.toml
├── types.rs                 ← Structs payload MQTT (serde Deserialize)
├── mqtt_source.rs           ← Abonnement MQTT (rumqttc), envoi vers canaux mpsc
├── manager.rs               ← BatteryManager (battery)
├── battery_service.rs       ← Service D-Bus com.victronenergy.battery.*
├── sensor_manager.rs        ← SensorManager (temperature)
├── temperature_service.rs   ← Service D-Bus com.victronenergy.temperature.*
├── heatpump_manager.rs      ← HeatpumpManager
├── heatpump_service.rs      ← Service D-Bus com.victronenergy.heatpump.*
├── meteo_manager.rs         ← MeteoManager (singleton)
├── meteo_service.rs         ← Service D-Bus com.victronenergy.meteo
├── switch_manager.rs        ← SwitchManager (publie aussi vers MQTT pour commandes)
├── switch_service.rs        ← Service D-Bus com.victronenergy.switch.*
├── grid_manager.rs          ← GridManager (grid + acload)
├── grid_service.rs          ← Service D-Bus com.victronenergy.grid.* / acload.*
├── pvinverter_manager.rs    ← PvinverterManager
├── pvinverter_service.rs    ← Service D-Bus com.victronenergy.pvinverter.*
├── platform_manager.rs      ← PlatformManager (singleton — bloque le thread principal)
└── platform_service.rs      ← Service D-Bus com.victronenergy.platform
```

Chaque type de device suit le même patron :

| Composant | Rôle |
|-----------|------|
| `{type}_manager.rs` | Boucle tokio : reçoit `MqttEvent` via canal `mpsc`, crée le service D-Bus au 1er message, gère watchdog + keepalive |
| `{type}_service.rs` | Enregistrement D-Bus zbus : objet racine `/` + objets feuilles ; expose `GetValue/GetText/SetValue/GetItems/ItemsChanged` |
| `mqtt_source.rs` | Fonctions `start_{type}_mqtt_source` : connexion rumqttc, souscription, désérialisation JSON, envoi dans `mpsc::Sender` |
| `types.rs` | Struct payload serde pour le type (ex: `HeatPayload`, `GridPayload`) |
| `config.rs` | Struct de configuration TOML pour le type (ex: `SwitchRef`, `GridRef`) |

### 2.3 Supervision des tâches critiques — spawn_critical

Toutes les boucles de bridge longue durée sont lancées via `spawn_critical` :

```rust
fn spawn_critical<F>(fut: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::task::spawn(async move {
        fut.await;
        error!(
            "Une tâche critique de bridge s'est terminée de façon inattendue — \
             arrêt du process pour redémarrage par le superviseur"
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        std::process::exit(1);
    });
}
```

Si une boucle de bridge retourne (D-Bus perdu, erreur fatale), le process s'arrête
entièrement. Le superviseur daemontools le redémarre. Sans `spawn_critical`, un bridge
mort passerait inaperçu alors que le service resterait apparemment « up ».

Le `PlatformManager` est un cas particulier : il est lancé en dernier et bloque le
thread principal (`main.rs`). Son arrêt provoque directement `std::process::exit(1)`.

**Règle** : `spawn_critical` ne s'applique qu'aux boucles de service longue durée.
Ne jamais l'utiliser pour une tâche one-shot ou un timer transitoire.

---

## 3. Interface D-Bus Venus OS — com.victronenergy.BusItem

### 3.1 Méthodes exposées

Chaque service `com.victronenergy.{type}.{suffix}` implémente l'interface
`com.victronenergy.BusItem` sur deux niveaux :

**Objet racine `/`** :

| Méthode | Signature D-Bus | Description |
|---------|----------------|-------------|
| `GetItems()` | `→ a{sa{sv}}` | Retourne tous les chemins avec valeur et texte. Appelé par `systemcalc-py` au démarrage. |
| `GetValue()` | `→ v` | Retourne 0 (la racine n'est pas une feuille). |
| `GetText()` | `→ s` | Retourne `""`. |
| `SetValue(v)` | `i →` | Lecture seule : retourne 1. |
| `ItemsChanged` | signal `a{sa{sv}}` | Émis à chaque mise à jour MQTT et lors du keepalive. |

**Objets feuilles** (ex: `/Soc`, `/Dc/0/Voltage`) :

| Méthode | Signature D-Bus | Description |
|---------|----------------|-------------|
| `GetValue()` | `→ v` | Valeur typée du chemin. |
| `GetText()` | `→ s` | Représentation texte avec unité (ex: `"56.4 %"`). |
| `SetValue(v)` | `i →` | Lecture seule : retourne 1. |

### 3.2 Signal ItemsChanged

Format du signal `ItemsChanged(a{sa{sv}})` :

```
{
  "/Soc":          {"Value": <56.4f64>, "Text": <"56.40 %">},
  "/Dc/0/Voltage": {"Value": <48.1f64>, "Text": <"48.10 V">},
  "/Connected":    {"Value": <1i32>,    "Text": <"1">},
  ...
}
```

Mapping de types JSON → D-Bus :

| Type JSON | Type D-Bus | Signature |
|-----------|-----------|-----------|
| f64 | double | `d` |
| u64 (petit) | uint32 | `u` |
| i64 (≤ i32::MAX) | int32 | `i` |
| i64 (grand) | int64 | `x` |
| string | string | `s` |

### 3.3 Objets racine et feuilles

À la création du service, deux niveaux d'objets sont enregistrés sur le bus :

1. L'objet racine `/` est enregistré avec `BatteryRootIface` (ou son équivalent par type).
2. Un objet feuille `BusItemLeaf` est enregistré pour **chaque chemin** retourné par
   `to_items()` (ex: `/Soc`, `/Dc/0/Voltage`, `/Alarms/LowVoltage`, …).

Les deux partagent un `Arc<Mutex<BatteryValues>>` (ou équivalent). La mise à jour
consiste à écrire dans le mutex puis émettre `ItemsChanged` sur l'objet racine.

### 3.4 Point critique — enregistrement des chemins au démarrage

Les objets feuilles D-Bus sont enregistrés **une seule fois** à la création du service,
depuis l'état initial `disconnected()`. **Tous les chemins** doivent être présents dans
`to_items()` même à l'état déconnecté, avec une valeur par défaut.

```rust
// CORRECT : toujours inclus, 0.0 si absent
m.insert("/Humidity".into(), DbusItem::f64(self.humidity.unwrap_or(0.0), "%"));

// INCORRECT : chemin non enregistré si None au démarrage → GetValue échoue
if let Some(h) = self.humidity {
    m.insert("/Humidity".into(), DbusItem::f64(h, "%"));
}
```

`GetItems()` sur `/` fonctionne dans les deux cas car il appelle `to_items()` au moment
de la requête. Mais `GetValue()` sur un chemin individuel retourne `"Unknown object"` si
l'objet feuille n'est pas enregistré.

---

## 4. Watchdog MQTT et keepalive D-Bus

Le `BatteryManager` (et ses équivalents pour chaque type) gère deux intervalles
configurables dans la section `[venus]` du `config.toml` :

| Paramètre | Défaut | Rôle |
|-----------|--------|------|
| `republish_sec` | 25 s | Réémet `ItemsChanged` vers D-Bus même sans nouveau message MQTT — garantit que Venus OS ne perd pas le device. |
| `watchdog_sec` | 30 s | Après ce délai sans message MQTT reçu, met `/Connected = 0` sur le service D-Bus. |

La boucle principale utilise `tokio::select!` pour traiter en parallèle les événements
MQTT et le tick du minuteur `republish_tick` :

```rust
loop {
    tokio::select! {
        Some(evt) = self.rx.recv() => {
            self.handle_mqtt_event(evt).await?;
        }
        _ = republish_tick.tick() => {
            self.republish_and_watchdog(watchdog_dur).await;
        }
    }
}
```

À chaque tick du minuteur :
- Si `now - last_update > watchdog_dur` → appel `set_disconnected()` → `/Connected = 0`.
- Sinon → appel `republish()` → émission `ItemsChanged` avec les valeurs courantes.

**Règle absolue** : la source MQTT (energy-manager ou daly-bms-server) doit publier le
topic concerné au moins une fois par `watchdog_sec`. Pour les sources lentes (Open-Meteo
= 15 min), un keepalive périodique (25s) est obligatoire côté energy-manager.

Si le keepalive est trop long (ex: 60s > 30s watchdog), le service Rust met
`/Connected = 0` entre deux publications et le device disparaît du VRM.

---

## 5. Inventaire complet des devices — mapping topics → D-Bus

### 5.1 Tableau de synthèse

(Source de vérité : `CLAUDE.md §5` + `nanoPi/config-nanopi.toml`)

| Topic MQTT (préfixe `santuario/`) | Service D-Bus | Instance | Device | Source Pi5 |
|----------------------------------|--------------|---------|--------|-----------|
| `bms/1/venus` | `com.victronenergy.battery.mqtt_1` | 151 | BMS-360Ah (RS485 0x01) | daly-bms-server |
| `bms/2/venus` | `com.victronenergy.battery.mqtt_2` | 152 | BMS-320Ah (RS485 0x02) | daly-bms-server |
| `bms/3/venus` | `com.victronenergy.battery.mqtt_3` | 153 | BMS-620Ah (RS485 0x03) | daly-bms-server |
| `heat/1/venus` | `com.victronenergy.temperature.mqtt_1` | 20 | Temp. extérieure (Open-Meteo) | energy-manager |
| `heatpump/1/venus` | `com.victronenergy.heatpump.mqtt_1` | — | Chauffe-eau LG ThinQ (optionnel) | energy-manager |
| `meteo/venus` | `com.victronenergy.meteo` | 40 | Irradiance PRALRAN (RS485 0x05) | daly-bms-server |
| `switch/1/venus` | `com.victronenergy.switch.mqtt_1` | 60 | ATS CHINT | daly-bms-server |
| `switch/2/venus` | `com.victronenergy.switch.mqtt_2` | 61 | Tongou Switch1 (tongou_3BC764) | energy-manager |
| `switch/3/venus` | `com.victronenergy.switch.mqtt_3` | 62 | Tongou Switch2 (tongou_0A3FA0) | energy-manager |
| `switch/4/venus` | `com.victronenergy.switch.mqtt_4` | 63 | Tongou Switch3 (tongou_0A3C14) | energy-manager |
| `switch/5/venus` | `com.victronenergy.switch.mqtt_5` | 64 | Tongou Switch4 (tongou_0A4040) | energy-manager |
| `switch/6/venus` | `com.victronenergy.switch.mqtt_6` | 65 | Tongou Switch5 (tongou_3ACC34) | energy-manager |
| `grid/8/venus` | `com.victronenergy.acload.mqtt_8` | 30 | ET112-Maison (RS485 0x08, SN 119215X) | daly-bms-server |
| `grid/9/venus` | `com.victronenergy.grid.mqtt_9` | 31 | ET112-Réseau (RS485 0x09, SN 061077X) | daly-bms-server |
| `pvinverter/7/venus` | `com.victronenergy.pvinverter.mqtt_7` | 32 | ET112-Micro-Onduleurs (RS485 0x07, SN 119253X) | daly-bms-server |
| `platform/venus` | `com.victronenergy.platform` | 50 | Pi5 backup/restore | energy-manager |

### 5.2 Batteries Daly (battery)

Service D-Bus : `com.victronenergy.battery.{prefix}_{mqtt_index}`

Chemins D-Bus exposés :

| Chemin | Unité | Description |
|--------|-------|-------------|
| `/Connected` | 0/1 | Connexion active |
| `/ProductName` | string | Nom du BMS |
| `/ProductId` | uint32 | 0 |
| `/FirmwareVersion` | string | `"Daly-RS485"` |
| `/DeviceInstance` | uint32 | Instance D-Bus (151/152/153) |
| `/Mgmt/ProcessName` | string | `"dbus-mqtt-venus"` |
| `/Mgmt/ProcessVersion` | string | Version du binaire |
| `/Mgmt/Connection` | string | `"MQTT"` |
| `/Soc` | % | État de charge |
| `/Dc/0/Voltage` | V | Tension pack |
| `/Dc/0/Current` | A | Courant (+ = charge) |
| `/Dc/0/Power` | W | Puissance |
| `/Dc/0/Temperature` | °C | Température BMS |
| `/Capacity` | Ah | Capacité restante |
| `/InstalledCapacity` | Ah | Capacité nominale |
| `/ConsumedAmphours` | Ah | Ampères-heures consommés |
| `/TimeToGo` | s | Temps restant estimé |
| `/Balancing` | 0/1 | Équilibrage actif |
| `/SystemSwitch` | 0/1 | Interrupteur système |
| `/Io/AllowToCharge` | 0/1 | DVCC : autoriser charge |
| `/Io/AllowToDischarge` | 0/1 | DVCC : autoriser décharge |
| `/Alarms/LowVoltage` | 0/1/2 | Alarme sous-tension |
| `/Alarms/HighVoltage` | 0/1/2 | Alarme surtension |
| `/Alarms/LowSoc` | 0/1/2 | Alarme SOC bas |
| `/Alarms/HighTemperature` | 0/1/2 | Alarme température haute |
| `/Alarms/LowTemperature` | 0/1/2 | Alarme température basse |
| `/Alarms/CellImbalance` | 0/1/2 | Alarme déséquilibre cellules |
| `/System/MinCellVoltage` | V | Tension cellule minimale |
| `/System/MaxCellVoltage` | V | Tension cellule maximale |
| `/System/MinCellTemperature` | °C | Température cellule minimale |
| `/System/MaxCellTemperature` | °C | Température cellule maximale |

### 5.3 Capteurs de température (temperature)

Service D-Bus : `com.victronenergy.temperature.{prefix}_{mqtt_index}`

Chemins D-Bus exposés :

| Chemin | Unité | Description |
|--------|-------|-------------|
| `/Connected` | 0/1 | Connexion active |
| `/ProductName` | string | Nom du capteur |
| `/DeviceInstance` | uint32 | Instance D-Bus |
| `/Temperature` | °C | Température courante |
| `/TemperatureType` | int | 0=battery 1=fridge 2=generic 3=Room 4=Outdoor 5=WaterHeater 6=Freezer |
| `/Humidity` | % | Humidité relative (0.0 si absente) |
| `/Pressure` | hPa | Pression atmosphérique (0.0 si absente) |
| `/CustomName` | string | Nom personnalisé |
| `/Status` | 0/1 | 0=OK, 1=Disconnected |

Production : instance 20, type 4 (Outdoor), température Open-Meteo publiée par
energy-manager toutes les 15 min avec keepalive 25s.

### 5.4 Pompe à chaleur / chauffe-eau (heatpump)

Service D-Bus : `com.victronenergy.heatpump.{prefix}_{mqtt_index}`

Chemins D-Bus exposés :

| Chemin | Unité | Description |
|--------|-------|-------------|
| `/Connected` | 0/1 | Connexion active |
| `/ProductName` | string | Nom de l'appareil |
| `/DeviceInstance` | uint32 | Instance D-Bus |
| `/State` | int | 0=Off/Vacation, 1=HeatPump (normal), 2=Turbo/Boost |
| `/Temperature` | °C | Température eau courante (0.0 si inconnue) |
| `/TargetTemperature` | °C | Température cible (0.0 si inconnue) |
| `/Ac/Power` | W | Puissance consommée |
| `/Ac/Energy/Forward` | kWh | Énergie totale consommée |
| `/Position` | 0/1 | 0=AC Output, 1=AC Input |

Mapping état LG ThinQ → Victron :

| Valeur Venus | Signification | Mode LG ThinQ | Opération |
|-------------|---------------|---------------|-----------|
| 0 | Off / Vacation | VACATION ou POWER_OFF | — |
| 1 | Heat Pump (normal) | HEAT_PUMP | POWER_ON |
| 2 | Turbo / Boost | TURBO | POWER_ON |

Source de données : API REST LG ThinQ, récupérée toutes les 10 min par energy-manager.

En production, la section `[[heatpumps]]` est commentée dans `config-nanopi.toml` ; la
décommenter pour activer le service heatpump.

### 5.5 Capteur météo / irradiance (meteo)

Service D-Bus : `com.victronenergy.meteo` (singleton, sans suffixe d'index).

Chemins D-Bus exposés :

| Chemin | Unité | Description |
|--------|-------|-------------|
| `/Connected` | 0/1 | Connexion active |
| `/ProductName` | string | `"Capteur Irradiance"` |
| `/DeviceInstance` | uint32 | 40 |
| `/Irradiance` | W/m² | Irradiance courante |
| `/TodaysYield` | kWh | Production du jour (depuis lever du soleil) |
| `/ExternalTemperature` | °C | Température extérieure (Open-Meteo, optionnel) |
| `/WindDirection` | ° | Direction du vent 0–360 (optionnel) |
| `/WindSpeed` | m/s | Vitesse du vent (optionnel) |

Topic fixe (sans index) : `santuario/meteo/venus`. Voir
[section 10.2](#102-meteo--singleton-sans-index) pour la particularité de configuration.

### 5.6 Switches / ATS CHINT et Tongou (switch)

Service D-Bus : `com.victronenergy.switch.{prefix}_{mqtt_index}`

Chemins D-Bus exposés (base) :

| Chemin | Valeurs | Description |
|--------|---------|-------------|
| `/Connected` | 0/1 | Connexion active |
| `/ProductName` | string | Nom du switch |
| `/DeviceInstance` | uint32 | Instance D-Bus |
| `/Position` | 0/1 | 0=AC1 (onduleur), 1=AC2 (réseau) |
| `/State` | 0/1/2 | 0=inactive, 1=active, 2=alerted |

Si `command_topic` est défini dans la config, le service expose en plus les chemins
`/SwitchableOutput/0/State` et `/SwitchableOutput/0/Settings/CustomName` pour permettre
le contrôle ON/OFF depuis la console Venus OS. Voir
[section 10.4](#104-switch--command_topic-et-controle-onoff-depuis-la-console-venus).

Production en 6 switches : ATS CHINT (lecture seule, pas de `command_topic`) + 5 Tongou
(contrôlables via `cmnd/tongou_{id}/Power`).

### 5.7 Compteurs réseau — grid et acload

Service D-Bus :
- `com.victronenergy.grid.{prefix}_{mqtt_index}` pour `service_type = "grid"`
- `com.victronenergy.acload.{prefix}_{mqtt_index}` pour `service_type = "acload"`

Chemins D-Bus exposés (L1, L2, L3 — tous enregistrés au démarrage) :

| Chemin | Unité | Description |
|--------|-------|-------------|
| `/Ac/L1/Voltage` | V | Tension phase L1 |
| `/Ac/L1/Current` | A | Courant phase L1 |
| `/Ac/L1/Power` | W | Puissance réelle L1 |
| `/Ac/L1/Energy/Forward` | kWh | Énergie consommée L1 |
| `/Ac/L1/Energy/Reverse` | kWh | Énergie injectée L1 |
| `/Ac/L2/...` | — | Même structure (0.0 si monophasé) |
| `/Ac/L3/...` | — | Même structure (0.0 si monophasé) |
| `/DeviceType` | int | 340 = generic energy meter |
| `/IsGenericEnergyMeter` | 0/1 | 1 si compteur générique masquerade |

Sémantique Victron (`grid` vs `acload`) :

- `grid` = point de connexion réseau EDF (compté dans import/export, ESS, bilan).
  Un seul compteur grid par système.
- `acload` = consommation AC locale, monitoring indépendant (non compté dans le bilan
  grid).

Mapping production :
- `0x08` "ET112-Maison" (`mqtt_index=8`) → `acload` → `com.victronenergy.acload.mqtt_8` (inst. 30)
- `0x09` "ET112-Réseau" (`mqtt_index=9`) → `grid` → `com.victronenergy.grid.mqtt_9` (inst. 31)

Voir [section 10.1](#101-grid_service--monophase-et-acnumberofphases) pour la
particularité monophasée.

### 5.8 Onduleurs PV / compteurs ET112 (pvinverter)

Service D-Bus : `com.victronenergy.pvinverter.{prefix}_{mqtt_index}`

Chemins D-Bus exposés :

| Chemin | Unité | Description |
|--------|-------|-------------|
| `/Connected` | 0/1 | Connexion active |
| `/ProductName` | string | Nom de l'onduleur/compteur |
| `/DeviceInstance` | uint32 | Instance D-Bus |
| `/Ac/Power` | W | Puissance AC totale |
| `/Ac/Energy/Forward` | kWh | Énergie produite totale |
| `/Ac/L1/Voltage` | V | Tension L1 |
| `/Ac/L1/Current` | A | Courant L1 |
| `/Ac/L1/Power` | W | Puissance L1 |
| `/Ac/L1/Energy/Forward` | kWh | Énergie L1 |
| `/StatusCode` | int | 0=Startup…7=Running |
| `/ErrorCode` | int | 0=No Error |
| `/Position` | 0/1 | 0=AC Input, 1=AC Output |
| `/IsGenericEnergyMeter` | 0/1 | 1 (ET112 masquerade en pvinverter) |

Production : ET112 addr=0x07 (SN 119253X), `mqtt_index=7`, instance 32,
`StatusCode=7` (Running), `Position=1` (AC Output).

Ce service remplace `com.victronenergy.pvinverter.cgwacs_ttyUSB0_mb2` (onduleur Victron
direct) qui reste visible en parallèle dans le VRM si toujours présent.

### 5.9 Platform backup/restore Pi5 (platform)

Service D-Bus : `com.victronenergy.platform` (singleton).

Chemins D-Bus exposés :

| Chemin | Valeurs | Description |
|--------|---------|-------------|
| `/Backup/Status` | 0=idle 1=running 2=OK 3=error | État du backup |
| `/Backup/LastRun` | timestamp Unix | Dernier backup terminé |
| `/Restore/Status` | 0=idle 1=running 2=OK 3=error | État du restore |
| `/Restore/LastRun` | timestamp Unix | Dernier restore terminé |

Topic fixe : `santuario/platform/venus`. Instance 50.

---

## 6. Configuration TOML — référence complète

Le binaire utilise le même `config.toml` que `daly-bms-server` (section `[mqtt]`
partagée). Chemin de déploiement sur le NanoPi : `/data/daly-bms/config.toml`.

Recherche de config (dans l'ordre) :
1. Option CLI `--config <chemin>`
2. Variable d'environnement `DALY_CONFIG`
3. `Config.toml` dans le répertoire courant
4. `/etc/daly-bms/config.toml`

### 6.1 Section [mqtt]

```toml
[mqtt]
host         = "127.0.0.1"   # broker local (NanoPi)
port         = 1883
topic_prefix = "santuario/bms"  # préfixe BMS uniquement

# Optionnel :
# username = "user"
# password = "secret"
```

### 6.2 Section [venus]

```toml
[venus]
enabled        = true
dbus_bus       = "system"    # "system" = production Venus OS ; "session" = dev/test
service_prefix = "mqtt"      # → mqtt_1, mqtt_2, etc.
watchdog_sec   = 30          # timeout déconnexion (s)
republish_sec  = 25          # republication forcée (s)
```

Valeurs par défaut si la section est absente : `enabled=true`, `dbus_bus="system"`,
`service_prefix="mqtt"`, `watchdog_sec=30`, `republish_sec=25`.

Override CLI possible : `--dbus-bus session` (ou `VENUS_DBUS_BUS=session`).

### 6.3 Sections batteries [[bms]]

```toml
[[bms]]
address         = "0x01"      # adresse RS485 (informative, logs uniquement)
name            = "BMS-360Ah"
mqtt_index      = 1           # → topic santuario/bms/1/venus → service mqtt_1
device_instance = 151         # DeviceInstance unique sur D-Bus
capacity_ah     = 360.0
max_charge_a    = 200.0
max_discharge_a = 120.0

[[bms]]
address         = "0x02"
name            = "BMS-320Ah"
mqtt_index      = 2
device_instance = 152
capacity_ah     = 320.0
max_charge_a    = 200.0
max_discharge_a = 120.0

[[bms]]
address         = "0x03"
name            = "BMS-620Ah"
mqtt_index      = 3
device_instance = 153
capacity_ah     = 620.0
max_charge_a    = 200.0
max_discharge_a = 120.0
```

**Attention** : instances 151/152/153 — différentes des instances 141/142 réservées à
l'ancien service Python `dbus-mqtt-battery`.

### 6.4 Section [heat] et [[sensors]]

```toml
[heat]
topic_prefix = "santuario/heat"

[[sensors]]
mqtt_index       = 1
name             = "Temperature Exterieure"
temperature_type = 4            # 4 = Outdoor
device_instance  = 20

# Exemple capteur supplémentaire :
# [[sensors]]
# mqtt_index       = 2
# name             = "Eau chaude sanitaire"
# temperature_type = 5           # 5 = WaterHeater
# device_instance  = 21
```

`temperature_type` est prioritaire sur la valeur `TemperatureType` contenue dans le
payload MQTT. Types disponibles : 0=battery, 1=fridge, 2=generic, 3=Room, 4=Outdoor,
5=WaterHeater, 6=Freezer.

### 6.5 Section [heatpump] et [[heatpumps]]

```toml
[heatpump]
topic_prefix = "santuario/heatpump"

# Décommenter pour activer le chauffe-eau LG ThinQ :
# [[heatpumps]]
# mqtt_index      = 1
# name            = "Chauffe-eau LG ThinQ"
# device_instance = 30
```

Un index n'est enregistré sur D-Bus que s'il possède une entrée `[[heatpumps]]`
correspondante (`heatpump_manager::is_configured`).

### 6.6 Section [meteo]

```toml
[meteo]
topic           = "santuario/meteo/venus"   # topic fixe, sans index
product_name    = "Capteur Irradiance"
device_instance = 40
```

### 6.7 Section [switch] et [[switches]]

```toml
[switch]
topic_prefix = "santuario/switch"

[[switches]]
mqtt_index      = 1
name            = "ATS CHINT"
device_instance = 60
# Pas de command_topic → lecture seule dans Venus

[[switches]]
mqtt_index      = 2
name            = "tongou_3BC764"              # 192.168.1.115
device_instance = 61
command_topic   = "cmnd/tongou_3BC764/Power"   # → contrôlable depuis console Venus
custom_name     = "Tongou 1"                   # nom affiché dans la console Venus OS
group           = "Tongou"                     # regroupe sur une même carte

[[switches]]
mqtt_index      = 3
name            = "tongou_0A3FA0"              # 192.168.1.72
device_instance = 62
command_topic   = "cmnd/tongou_0A3FA0/Power"
custom_name     = "Tongou 2"
group           = "Tongou"

[[switches]]
mqtt_index      = 4
name            = "tongou_0A3C14"              # 192.168.1.250
device_instance = 63
command_topic   = "cmnd/tongou_0A3C14/Power"
custom_name     = "Tongou 3"
group           = "Tongou"

[[switches]]
mqtt_index      = 5
name            = "tongou_0A4040"              # 192.168.1.53
device_instance = 64
command_topic   = "cmnd/tongou_0A4040/Power"
custom_name     = "Tongou 4"
group           = "Tongou"

[[switches]]
mqtt_index      = 6
name            = "tongou_3ACC34"              # 192.168.1.80
device_instance = 65
command_topic   = "cmnd/tongou_3ACC34/Power"
custom_name     = "Tongou 5"
group           = "Tongou"
```

### 6.8 Section [grid] et [[grids]]

```toml
[grid]
topic_prefix = "santuario/grid"

[[grids]]
mqtt_index      = 8             # ET112 addr=0x08 — Maison / consommation AC
name            = "ET112-Maison"
device_instance = 30
service_type    = "acload"     # → com.victronenergy.acload.mqtt_8

[[grids]]
mqtt_index      = 9             # ET112 addr=0x09 — Réseau / compteur EDF
name            = "ET112-Réseau"
device_instance = 31
service_type    = "grid"       # → com.victronenergy.grid.mqtt_9
```

`service_type` accepte `"grid"` (défaut) ou `"acload"`. Le payload JSON est identique
(`GridPayload`) ; seul `service_type` décide du nom de service D-Bus.

### 6.9 Section [pvinverter] et [[pvinverters]]

```toml
[pvinverter]
topic_prefix = "santuario/pvinverter"

[[pvinverters]]
mqtt_index      = 7             # ET112 addr=0x07 (SN 119253X) — Micro-Onduleurs
name            = "Micro Onduleurs"
device_instance = 32
```

### 6.10 Section [platform]

```toml
[platform]
topic           = "santuario/platform/venus"
product_name    = "Pi5 Platform"
device_instance = 50
enabled         = true
```

### 6.11 Fichier de production nanoPi/config-nanopi.toml

Le fichier `nanoPi/config-nanopi.toml` est la configuration de référence déployée sur
le NanoPi. Déploiement depuis le Pi5 :

```bash
scp nanoPi/config-nanopi.toml root@192.168.1.120:/data/daly-bms/config.toml
ssh root@192.168.1.120 "svc -t /service/dbus-mqtt-venus"
```

Vérification post-déploiement :

```bash
ssh root@192.168.1.120 "dbus -y | grep victronenergy"
```

---

## 7. Payloads MQTT — format JSON par type de device

### 7.1 Payload batterie (VenusPayload)

Topic : `santuario/bms/{n}/venus` — publié par `daly-bms-server`.

```json
{
  "Dc": {
    "Power": -125.5,
    "Voltage": 48.3,
    "Current": -2.6,
    "Temperature": 23.4
  },
  "InstalledCapacity": 360.0,
  "ConsumedAmphours": 45.2,
  "Capacity": 314.8,
  "Soc": 87.4,
  "TimeToGo": 86400,
  "Balancing": 0,
  "SystemSwitch": 1,
  "Alarms": {
    "LowVoltage": 0,
    "HighVoltage": 0,
    "LowSoc": 0,
    "HighChargeCurrent": 0,
    "HighDischargeCurrent": 0,
    "HighCurrent": 0,
    "CellImbalance": 0,
    "HighChargeTemperature": 0,
    "LowChargeTemperature": 0,
    "LowCellVoltage": 0,
    "LowTemperature": 0,
    "HighTemperature": 0,
    "FuseBlown": 0
  },
  "System": {
    "MinVoltageCellId": 3,
    "MinCellVoltage": 3.24,
    "MaxVoltageCellId": 1,
    "MaxCellVoltage": 3.28,
    "MinTemperatureCellId": 1,
    "MinCellTemperature": 22.0,
    "MaxTemperatureCellId": 2,
    "MaxCellTemperature": 24.0,
    "NrOfCellsPerBattery": 16,
    "NrOfModulesOnline": 1,
    "NrOfModulesOffline": 0,
    "NrOfModulesBlockingCharge": 0,
    "NrOfModulesBlockingDischarge": 0
  },
  "Io": {
    "AllowToCharge": 1,
    "AllowToDischarge": 1,
    "AllowToBalance": 1,
    "ExternalRelay": 0
  }
}
```

### 7.2 Payload température (HeatPayload)

Topic : `santuario/heat/{n}/venus` — publié par `energy-manager`.

```json
{"Temperature": 11.5, "Humidity": 42.0}
```

Champs optionnels : `TemperatureType` (int), `Pressure` (hPa), `CustomName` (string).

### 7.3 Payload heatpump (HeatpumpPayload)

Topic : `santuario/heatpump/{n}/venus` — publié par `energy-manager`.

Payload minimal :
```json
{
  "State": 1,
  "Temperature": 60.0,
  "TargetTemperature": 52.0,
  "Position": 0
}
```

Payload étendu avec puissance :
```json
{
  "State": 1,
  "Temperature": 60.0,
  "TargetTemperature": 52.0,
  "Ac": { "Power": 1200.0, "Energy": { "Forward": 125.5 } },
  "Position": 0
}
```

### 7.4 Payload météo (MeteoPayload)

Topic : `santuario/meteo/venus` — publié par `daly-bms-server` (PRALRAN RS485).

```json
{
  "Irradiance": 756.3,
  "TodaysYield": 14.2,
  "ExternalTemperature": 22.1,
  "WindDirection": 180.0,
  "WindSpeed": 3.5
}
```

Champs optionnels : `ExternalTemperature`, `WindDirection`, `WindSpeed`, `YieldYesterday`.

### 7.5 Payload switch (SwitchPayload)

Topic : `santuario/switch/{n}/venus`.

```json
{"Position": 0, "State": 1}
```

| Position | Signification |
|----------|--------------|
| 0 | AC1 — onduleur |
| 1 | AC2 — réseau |

### 7.6 Payload grid/acload (GridPayload)

Topic : `santuario/grid/{n}/venus` — publié par `daly-bms-server` (ET112).

Payload monophasé (ET112) :
```json
{
  "Ac": {
    "L1": {
      "Voltage": 230.0,
      "Current": 5.2,
      "Power": 1196.0,
      "Energy": {"Forward": 1234.5, "Reverse": 0.0}
    }
  },
  "DeviceType": 340,
  "IsGenericEnergyMeter": 0
}
```

Payload triphasé :
```json
{
  "Ac": {
    "L1": {"Voltage": 230.0, "Current": 5.2, "Power": 1196.0, "Energy": {"Forward": 400.0}},
    "L2": {"Voltage": 231.0, "Current": 4.8, "Power": 1108.8, "Energy": {"Forward": 380.0}},
    "L3": {"Voltage": 229.0, "Current": 6.1, "Power": 1396.9, "Energy": {"Forward": 450.0}}
  }
}
```

### 7.7 Payload pvinverter (PvinverterPayload)

Topic : `santuario/pvinverter/{n}/venus` — publié par `daly-bms-server` (ET112).

```json
{
  "Ac": {
    "L1": {
      "Voltage": 230.5,
      "Current": 8.7,
      "Power": 2004.0,
      "Energy": {"Forward": 5678.9}
    },
    "Power": 2004.0,
    "Energy": {"Forward": 5678.9}
  },
  "StatusCode": 7,
  "ErrorCode": 0,
  "Position": 1,
  "IsGenericEnergyMeter": 1,
  "ProductName": "ET112 addr=0x07"
}
```

### 7.8 Payload platform (PlatformPayload)

Topic : `santuario/platform/venus`.

```json
{
  "Backup":  {"Status": 2, "LastRun": 1710000000},
  "Restore": {"Status": 0, "LastRun": 0}
}
```

---

## 8. Procédure pas-à-pas — intégrer un nouveau type de device

### 8.1 Fichiers Rust à créer ou modifier

| Fichier | Action | Contenu |
|---------|--------|---------|
| `crates/dbus-mqtt-venus/src/types.rs` | Ajouter | Struct payload MQTT (serde `Deserialize`) |
| `crates/dbus-mqtt-venus/src/config.rs` | Ajouter | Structs config TOML (`{Type}Config` + `{Type}Ref`) + impls Default |
| `crates/dbus-mqtt-venus/src/{type}_service.rs` | Créer | Struct valeurs, `to_items()`, interfaces zbus (`#[zbus::interface]`), `create_{type}_service()` |
| `crates/dbus-mqtt-venus/src/{type}_manager.rs` | Créer | Struct Manager, `run()` avec `tokio::select!`, watchdog, keepalive |
| `crates/dbus-mqtt-venus/src/mqtt_source.rs` | Ajouter | Fonction `start_{type}_mqtt_source()` |
| `crates/dbus-mqtt-venus/src/main.rs` | Modifier | Ajouter canal `mpsc`, appels `spawn_critical`, instanciation du Manager |

### 8.2 Étapes détaillées

**Étape 1 — Définir le payload MQTT dans `types.rs`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonNouvelAppareilPayload {
    #[serde(rename = "Champ1")]
    pub champ1: f64,
    #[serde(rename = "Champ2", default)]
    pub champ2: Option<f64>,
}
```

**Étape 2 — Définir la configuration dans `config.rs`**

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NouvelAppareilConfig {
    pub topic_prefix: String,
}
impl Default for NouvelAppareilConfig {
    fn default() -> Self { Self { topic_prefix: "santuario/nouvel_appareil".to_string() } }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct NouvelAppareilRef {
    pub mqtt_index:      Option<u8>,
    pub name:            Option<String>,
    pub device_instance: Option<u32>,
}
```

Ajouter les champs dans `VenusServiceConfig` :
```rust
pub nouvel_appareil: NouvelAppareilConfig,
pub nouveaux_appareils: Vec<NouvelAppareilRef>,
```

**Étape 3 — Créer `{type}_service.rs`**

Reprendre le patron de `battery_service.rs` :
- Struct `{Type}Values` avec tous les champs y compris les optionnels (avec valeur par
  défaut) ; méthode `disconnected()` qui initialise tous les champs.
- Méthode `to_items()` : retourne `HashMap<String, DbusItem>` avec **tous les chemins**
  présents, même déconnecté (valeurs 0.0 pour les floats, 0 pour les entiers, `""` pour
  les strings).
- Interface zbus `#[zbus::interface(name = "com.victronenergy.BusItem")]` sur un struct
  `{Type}RootIface`.
- Interface zbus sur `BusItemLeaf` (partagé ou dupliqué selon les besoins).
- Fonction `create_{type}_service()` qui enregistre racine + feuilles.

**Étape 4 — Créer `{type}_manager.rs`**

```rust
pub struct NouvelAppareilManager {
    cfg:     VenusConfig,
    refs:    Vec<NouvelAppareilRef>,
    services: HashMap<u8, NouvelAppareilServiceHandle>,
    rx:      mpsc::Receiver<MqttEvent>,
}

impl NouvelAppareilManager {
    pub async fn run(mut self) -> Result<()> {
        // Même structure que BatteryManager :
        // tokio::select! { rx.recv() => handle_mqtt_event | republish_tick => watchdog }
    }
}
```

**Étape 5 — Ajouter dans `mqtt_source.rs`**

```rust
pub async fn start_nouvel_appareil_mqtt_source(
    cfg:    MqttRef,
    prefix: String,
    tx:     mpsc::Sender<MqttEvent>,
) {
    // Connexion rumqttc, souscription à "{prefix}/+/venus",
    // désérialisation JSON en MonNouvelAppareilPayload (via MqttEvent),
    // envoi dans tx.
}
```

**Étape 6 — Brancher dans `main.rs`**

```rust
// Dans main() après les autres bridges :
let (nouvel_appareil_tx, nouvel_appareil_rx) = mpsc::channel(32);
let mqtt_cfgN = cfg.mqtt.clone();
let nouvel_appareil_prefix = cfg.nouvel_appareil.topic_prefix.clone();
spawn_critical(async move {
    start_nouvel_appareil_mqtt_source(mqtt_cfgN, nouvel_appareil_prefix, nouvel_appareil_tx).await;
});
let nouvel_appareil_manager = NouvelAppareilManager::new(
    cfg.venus.clone(), cfg.nouveaux_appareils, nouvel_appareil_rx
);
spawn_critical(async move {
    if let Err(e) = nouvel_appareil_manager.run().await {
        error!("NouvelAppareilManager terminé avec erreur : {:#}", e);
    }
});
```

**Étape 7 — Ajouter la configuration dans `config-nanopi.toml`**

```toml
[nouvel_appareil]
topic_prefix = "santuario/nouvel_appareil"

[[nouveaux_appareils]]
mqtt_index      = 1
name            = "Mon appareil"
device_instance = 90  # unique sur tout le bus D-Bus
```

**Étape 8 — Configurer le bridge Mosquitto Pi5 (si le topic vient du Pi5)**

Voir [section 8.3](#83-regle-de-configuration-du-bridge-mosquitto).

### 8.3 Règle de configuration du bridge Mosquitto

Chaque nouveau type de device nécessite une règle `out` dans
`contrib/mosquitto/mosquitto.conf` (déployé vers `/etc/mosquitto/mosquitto.conf` sur le
Pi5) :

```
topic santuario/nouvel_appareil/# out 0
```

**Règle critique** : ne jamais utiliser `santuario/# both` pour éviter les boucles de
messages. Utiliser des règles `out` spécifiques par type.

Après modification :
```bash
sudo cp contrib/mosquitto/mosquitto.conf /etc/mosquitto/mosquitto.conf
sudo systemctl restart mosquitto-broker
# Valider l'absence de boucle :
sudo bash scripts/netdiag.sh
```

---

## 9. Build et déploiement ARMv7

### 9.1 Contrainte CRITIQUE — jamais target-cpu=native pour ARMv7

> **AVERTISSEMENT** : ne jamais utiliser `-C target-cpu=native` lors de la
> cross-compilation pour `armv7-unknown-linux-gnueabihf`. L'hôte est `aarch64` (Pi5)
> et génère des instructions LSE/ARMv8 (ex: `+lse`) incompatibles avec l'ARMv7 du
> NanoPi → **SIGILL (exit 132)** au démarrage, crash-loop immédiat.

Le Makefile utilise uniquement `-C link-arg=-Wl,--as-needed` pour le build ARMv7.
Indice à la compilation : warnings `'+lse' is not a recognized feature` signalent un
problème de flags.

### 9.2 Prérequis cross-compilation

À exécuter une seule fois sur le Pi5 :

```bash
# Installer le cross-compilateur ARM
sudo apt-get install -y gcc-arm-linux-gnueabihf

# Ajouter la target Rust
rustup target add armv7-unknown-linux-gnueabihf
```

Vérification :
```bash
rustup target list --installed | grep armv7
```

### 9.3 Commandes Make

| Commande | Description |
|----------|-------------|
| `make build-venus-v7` | Cross-compile `dbus-mqtt-venus` pour ARMv7 (NanoPi) |
| `make install-venus-v7` | build-venus-v7 + déploiement via `install-venus.sh` sur `192.168.1.120` |
| `make install-venus-v7 GX_IP=x.x.x.x` | Idem avec IP personnalisée |
| `make build-venus-armv7` | Alias de `build-venus-v7` |
| `make build-venus` | Build natif (hôte, pour dev/test D-Bus session) |
| `make build-venus-arm` | Build pour aarch64 (64-bit, non compatible NanoPi) |
| `make install-venus` | Déployer la version aarch64 (ne fonctionne pas sur NanoPi) |

Commande bas niveau (sans Makefile) :
```bash
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  RUSTFLAGS="-C link-arg=-Wl,--as-needed" \
  cargo build --release \
  --target armv7-unknown-linux-gnueabihf \
  -p dbus-mqtt-venus
```

Binaire produit : `target/armv7-unknown-linux-gnueabihf/release/dbus-mqtt-venus`

### 9.4 Déploiement manuel pas-à-pas

**Ordre obligatoire : arrêter le service avant de copier le binaire** (sinon erreur
"scp: Failure" car le fichier est verrouillé).

```bash
# Étape 1 — Récupérer le dernier code (Pi5)
cd ~/Daly-BMS-Rust
make sync

# Étape 2 — Compiler pour ARMv7 (Pi5)
make build-venus-v7

# Étape 3a — Arrêter le service sur NanoPi
ssh root@192.168.1.120 "svc -d /data/etc/sv/dbus-mqtt-venus"

# Étape 3b — Copier le binaire
scp target/armv7-unknown-linux-gnueabihf/release/dbus-mqtt-venus \
    root@192.168.1.120:/data/daly-bms/dbus-mqtt-venus

# Étape 3c — Redémarrer le service
ssh root@192.168.1.120 "svc -u /data/etc/sv/dbus-mqtt-venus"

# Vérification
ssh root@192.168.1.120 "svstat /service/dbus-mqtt-venus"
# Résultat attendu : "up (pid XXXXX) N seconds"
```

**Si la configuration a été modifiée** :

```bash
# Déployer la config sur NanoPi
scp nanoPi/config-nanopi.toml root@192.168.1.120:/data/daly-bms/config.toml

# Redémarrer
ssh root@192.168.1.120 "svc -t /service/dbus-mqtt-venus"
```

### 9.5 Script install-venus.sh — détail des étapes

Le script `nanoPi/install-venus.sh` automatise le déploiement. Invoqué par
`make install-venus-v7`. Il effectue les opérations suivantes dans l'ordre :

1. Crée `/data/daly-bms/` et `/data/etc/sv/dbus-mqtt-venus/` sur le NanoPi.
2. Arrête `dbus-mqtt-venus` s'il est actif (`svc -d`).
3. Supprime `daly-bms-server` du NanoPi s'il est présent (ce service ne doit tourner
   **que sur le Pi5**).
4. Copie le binaire `dbus-mqtt-venus` vers `/data/daly-bms/` et le rend exécutable.
5. Copie `Config.toml` vers `/data/daly-bms/config.toml` **uniquement si absent** (pour
   préserver la config existante).
6. Installe le script `run` daemontools depuis `nanoPi/sv/dbus-mqtt-venus/run`.
7. Nettoie l'ancien symlink `daly-bms-venus` (ancien nom du crate, maintenant renommé
   `dbus-mqtt-venus`).
8. Active ou redémarre le service (`ln -sf` ou `svc -u`).
9. Ajoute la ligne de persistance dans `/data/rc.local`.

Le script utilise un SSH ControlMaster pour n'ouvrir qu'une seule connexion et éviter
les demandes de mot de passe répétées.

**Prérequis** : ajouter la clé SSH avec `ssh-copy-id root@192.168.1.120` avant la
première utilisation. Si `install-venus.sh` retourne "Permission denied", vérifier le
bit exécutable :

```bash
chmod +x nanoPi/install-venus.sh
# ou :
ARCH=armv7 bash nanoPi/install-venus.sh 192.168.1.120
```

### 9.6 Structure des fichiers sur le NanoPi

```
/data/
  daly-bms/
    dbus-mqtt-venus       ← binaire ARMv7
    config.toml           ← configuration (conservée entre déploiements)

  etc/
    sv/
      dbus-mqtt-venus/
        run               ← script daemontools

  rc.local                ← ln -sf /data/etc/sv/dbus-mqtt-venus /service/dbus-mqtt-venus

/service/
  dbus-mqtt-venus  →  /data/etc/sv/dbus-mqtt-venus   (symlink ou répertoire)
    supervise/
```

Contenu du script `run` :

```sh
#!/bin/sh
exec /data/daly-bms/dbus-mqtt-venus \
    --config /data/daly-bms/config.toml \
    2>&1
```

### 9.7 Init system — daemontools (svc/svstat)

Venus OS utilise **daemontools** (non runit). La commande `sv` est inconnue. Utiliser
exclusivement `svc` et `svstat` :

```bash
svstat /service/dbus-mqtt-venus    # état (affiche "up N seconds" si OK)
svc -t /service/dbus-mqtt-venus    # restart (SIGTERM)
svc -d /service/dbus-mqtt-venus    # stop
svc -u /service/dbus-mqtt-venus    # start
```

### 9.8 Persistance au reboot

Venus OS efface `/service/` à chaque reboot. La persistance est assurée par
`/data/rc.local` (espace `/data/` survit aux mises à jour firmware) :

```bash
# Vérifier la ligne de persistance
cat /data/rc.local
# Doit contenir :
# ln -sf /data/etc/sv/dbus-mqtt-venus /service/dbus-mqtt-venus
```

Si le symlink disparaît après une mise à jour firmware :

```bash
ssh root@192.168.1.120 "ln -sf /data/etc/sv/dbus-mqtt-venus /service/dbus-mqtt-venus"
```

---

## 10. Particularités et cas spéciaux

### 10.1 grid_service — monophasé et /Ac/NumberOfPhases

Les ET112 sont des compteurs **monophasés** : seule la phase L1 contient des données ;
L2 et L3 sont à 0.0. Le service enregistre quand même tous les chemins L1/L2/L3 au
démarrage (valeur 0.0 par défaut) pour satisfaire la contrainte d'enregistrement
complet.

La gestion du champ `/Ac/NumberOfPhases` (dérivé du payload : si L2/L3 absents → 1,
sinon 3) évite que L2/L3 fantômes à 0 W apparaissent dans le VRM. Si des valeurs L2/L3
persistent dans le VRM après un changement, rafraîchir le cache VRM.

### 10.2 meteo — singleton sans index

`com.victronenergy.meteo` est un service **singleton** : un seul capteur d'irradiance
par installation Victron. Son topic est fixe (`santuario/meteo/venus`), sans index
numérique. La config utilise une section scalaire `[meteo]` (pas de `[[meteos]]`).

### 10.3 Température — limitation Venus OS

> **Note** : le widget météo Venus OS peut afficher "Température: -" même si le service
> `com.victronenergy.temperature.mqtt_1` publie correctement `/Temperature`. Cette
> limitation est inhérente à Venus OS et non fixable côté `dbus-mqtt-venus`.

### 10.4 switch — command_topic et contrôle ON/OFF depuis la console Venus

Si un `[[switches]]` possède un `command_topic`, le service enregistre des chemins
`/SwitchableOutput/0/State` et les chemins de configuration associés sur D-Bus. Quand
l'utilisateur bascule le switch dans la console Venus OS, le service publie `"ON"` ou
`"OFF"` sur ce topic MQTT (format Tasmota).

- Sans `command_topic` : affichage lecture seule (ex: ATS CHINT géré en RS485).
- Avec `command_topic` : switch bidirectionnel (ex: Tongou via Tasmota).

Le champ `group` regroupe plusieurs switches sur la même carte dans la console Venus.

### 10.5 Batterie 620Ah virtuelle (BMS-3 agrégé)

Le BMS-3 (`mqtt_index=3`, instance 153) correspond à une batterie **620Ah virtuelle
agrégée**, calculée par `energy-manager` à partir des données des BMS physiques. Il est
publié sur `santuario/bms/3/venus` comme un BMS physique ordinaire. Le service
`dbus-mqtt-venus` ne fait aucune distinction ; il enregistre simplement
`com.victronenergy.battery.mqtt_3` avec les données reçues.

> ⚠️ Divergence sources : `VENUS-DEVICE-INTEGRATION.md` mentionne les instances
> 141/142 pour les BMS-1/2 et une capacité 628Ah pour BMS-3. La source de vérité est
> `CLAUDE.md §5` et `config-nanopi.toml` (production) : instances 151/152/153,
> capacité 360/320/620 Ah. Les instances 141/142 sont réservées à l'ancien service
> Python `dbus-mqtt-battery` (voir section 14).

### 10.6 Instances D-Bus réservées — héritage dbus-mqtt-battery Python

Les instances D-Bus **141** et **142** sont réservées à l'ancien service Python
`dbus-mqtt-battery` (instances `mqtt_battery_141` et `mqtt_battery_142`). Si ce service
Python est encore actif en parallèle, ces instances coexistent avec `mqtt_1` (151) et
`mqtt_2` (152) sur le bus. La recommandation est de supprimer le service Python une fois
`dbus-mqtt-venus` Rust validé (voir section 14).

---

## 11. Commandes de vérification et diagnostic

### 11.1 État du service runit

```bash
# Depuis Pi5 via SSH
ssh root@192.168.1.120 "svstat /service/dbus-mqtt-venus"
# Résultat attendu : "up (pid XXXXX) N seconds"

# Processus actifs (BusyBox — pas de -aux)
ssh root@192.168.1.120 "ps | grep daly"
# Doit afficher : /data/daly-bms/dbus-mqtt-venus --config /data/daly-bms/config.toml
```

### 11.2 Vérification D-Bus par service

**Lister tous les services Venus actifs :**

```bash
ssh root@192.168.1.120 "dbus -y | grep victronenergy"
```

**Lire toutes les valeurs d'un service (méthode principale — VRM utilise GetItems) :**

```bash
dbus -y com.victronenergy.battery.mqtt_1 / GetItems
dbus -y com.victronenergy.battery.mqtt_2 / GetItems
dbus -y com.victronenergy.battery.mqtt_3 / GetItems
dbus -y com.victronenergy.temperature.mqtt_1 / GetItems
dbus -y com.victronenergy.meteo / GetItems
dbus -y com.victronenergy.switch.mqtt_1 / GetItems
dbus -y com.victronenergy.acload.mqtt_8 / GetItems
dbus -y com.victronenergy.grid.mqtt_9 / GetItems
dbus -y com.victronenergy.pvinverter.mqtt_7 / GetItems
dbus -y com.victronenergy.platform / GetItems
```

**Lire une valeur individuelle :**

```bash
# Batteries
dbus -y com.victronenergy.battery.mqtt_1 /Soc GetValue
dbus -y com.victronenergy.battery.mqtt_1 /Dc/0/Voltage GetValue
dbus -y com.victronenergy.battery.mqtt_2 /Soc GetValue
dbus -y com.victronenergy.battery.mqtt_3 /Soc GetValue
dbus -y com.victronenergy.battery.mqtt_1 /Connected GetValue

# Température
dbus -y com.victronenergy.temperature.mqtt_1 /Temperature GetValue
dbus -y com.victronenergy.temperature.mqtt_1 /Humidity GetValue
dbus -y com.victronenergy.temperature.mqtt_1 /Connected GetValue

# Heatpump
dbus -y com.victronenergy.heatpump.mqtt_1 /State GetValue
dbus -y com.victronenergy.heatpump.mqtt_1 /Temperature GetValue
dbus -y com.victronenergy.heatpump.mqtt_1 /TargetTemperature GetValue
dbus -y com.victronenergy.heatpump.mqtt_1 /Ac/Power GetValue
dbus -y com.victronenergy.heatpump.mqtt_1 /Position GetValue
dbus -y com.victronenergy.heatpump.mqtt_1 /Connected GetValue

# Switch
dbus -y com.victronenergy.switch.mqtt_1 /Position GetValue
dbus -y com.victronenergy.switch.mqtt_1 /State GetValue

# Grid/ACload
dbus -y com.victronenergy.grid.mqtt_9 /Ac/L1/Power GetValue
dbus -y com.victronenergy.grid.mqtt_9 /Ac/L1/Voltage GetValue
dbus -y com.victronenergy.acload.mqtt_8 /Ac/L1/Power GetValue

# PV Inverter
dbus -y com.victronenergy.pvinverter.mqtt_7 /Ac/Power GetValue
dbus -y com.victronenergy.pvinverter.mqtt_7 /Ac/L1/Power GetValue

# Platform
dbus -y com.victronenergy.platform /Backup/Status GetValue
dbus -y com.victronenergy.platform /Backup/LastRun GetValue
```

> **Note** : `GetValue` sur un chemin individuel échoue avec "Unknown object" si
> l'objet feuille n'est pas enregistré. `GetItems` sur `/` fonctionne toujours.

### 11.3 Vérification MQTT sur NanoPi

```bash
# Souscrire et attendre un message (doit arriver en < 2s si daly-bms-server tourne)
mosquitto_sub -h 127.0.0.1 -p 1883 -t "santuario/bms/1/venus" -C 1 -v
mosquitto_sub -h 127.0.0.1 -p 1883 -t "santuario/heat/1/venus" -v
mosquitto_sub -h 127.0.0.1 -p 1883 -t "santuario/meteo/venus" -v
mosquitto_sub -h 127.0.0.1 -p 1883 -t "santuario/#" -v
```

**Test MQTT direct (sans energy-manager) :**

```bash
# Depuis Pi5 ou NanoPi
mosquitto_pub -h localhost -t "santuario/heatpump/1/venus" \
  -m '{"State":1,"Temperature":60.0,"TargetTemperature":52.0,"Position":0}'
```

### 11.4 Logs du service Rust

Le service utilise `supervise` daemontools sans fichier log dédié.

```bash
# Lancer manuellement pour voir les logs (service STOPPÉ au préalable)
svc -d /service/dbus-mqtt-venus
/data/daly-bms/dbus-mqtt-venus --config /data/daly-bms/config.toml

# Niveau debug
RUST_LOG=debug /data/daly-bms/dbus-mqtt-venus --config /data/daly-bms/config.toml
```

Les traces apparaissent aussi dans `readproctitle` :

```bash
ps | grep readproctitle
```

### 11.5 Ressources système NanoPi

```bash
# Vue CPU + RAM temps réel (BusyBox compatible)
top -b -n 1 | head -n 20

# RAM disponible
cat /proc/meminfo | grep -E "MemTotal|MemFree|MemAvailable"
```

Empreinte mémoire attendue :

| Process | RAM | CPU |
|---------|-----|-----|
| `dbus-mqtt-venus` | ~5–8 MB | < 1% |
| Venus OS + systemcalc-py | ~150 MB | existant |

---

## 12. Dépannage NanoPi

### 12.1 Service D-Bus non visible

1. Vérifier que le service Rust tourne : `ps | grep dbus-mqtt-venus`
2. Vérifier qu'au moins un message MQTT a été reçu — le service D-Bus est créé au
   **premier message** (création dynamique).
3. Vérifier le bridge Mosquitto Pi5 : règle `out` présente pour le topic concerné.
4. Vérifier que `daly-bms-server` et/ou `energy-manager` publient bien sur le topic.

Diagnostic rapide :
```bash
mosquitto_sub -h 127.0.0.1 -p 1883 -t "santuario/#" -v
# Doit recevoir des messages toutes les 1–2 secondes pour les BMS
```

### 12.2 /Connected = 0 (device déconnecté dans VRM)

Le keepalive côté source MQTT est trop long (> `watchdog_sec` = 30s). Le service Rust
met `/Connected = 0` entre les publications.

Solution : vérifier que `energy-manager` publie bien toutes les 25s. Pour les sources
lentes (Open-Meteo = 15 min), un nœud keepalive 25s est obligatoire.

### 12.3 Crash-loop SIGILL (exit 132) — architecture mismatch

**Symptôme** : `svstat` affiche uptime ≈ 0, tous les services D-Bus absents. Lancement
manuel du binaire → `Illegal instruction` ou sortie immédiate.

**Cause** : binaire compilé avec `target-cpu=native` (hôte `aarch64`) → instructions
LSE/ARMv8 incompatibles avec ARMv7 NanoPi.

**Diagnostic** :
```bash
/data/daly-bms/dbus-mqtt-venus --config /data/daly-bms/config.toml
# Affiche "Illegal instruction" ou le processus sort avec code 132
```

**Solution** : recompiler sans `target-cpu=native` :
```bash
# Sur Pi5
make build-venus-v7   # utilise -C link-arg=-Wl,--as-needed uniquement
make install-venus-v7
```

### 12.4 Exec format error — mauvaise architecture

**Symptôme** :
```
/data/daly-bms/dbus-mqtt-venus: cannot execute binary file: Exec format error
```

**Cause** : binaire `aarch64` (64-bit) déployé sur NanoPi ARMv7 (32-bit).

**Solution** : toujours utiliser `make build-venus-v7` (cible `armv7-unknown-linux-gnueabihf`).

### 12.5 scp échoue avec "Failure"

Le service est actif et verrouille le binaire. Arrêter d'abord :
```bash
ssh root@192.168.1.120 "svc -d /data/etc/sv/dbus-mqtt-venus"
# puis scp …
ssh root@192.168.1.120 "svc -u /data/etc/sv/dbus-mqtt-venus"
```

### 12.6 name already taken on the bus

**Symptôme** :
```
ERROR dbus_mqtt_venus::manager: Erreur traitement événement MQTT : name already taken on the bus
```

**Cause** : le binaire a été lancé **manuellement** alors que le daemon tournait déjà.
Les noms D-Bus ne peuvent être pris qu'une seule fois.

**Solution** : ne jamais lancer le binaire manuellement si le service daemon est actif.
Vérifier avec `svstat` avant tout test manuel ; utiliser `svc -d` pour arrêter le daemon.

### 12.7 Symlink Venus disparu après mise à jour firmware

Venus OS peut supprimer les symlinks dans `/service/` lors d'une mise à jour.
`/data/rc.local` les recrée au boot, mais si le boot a eu lieu avant la correction :

```bash
ssh root@192.168.1.120 "ln -sf /data/etc/sv/dbus-mqtt-venus /service/dbus-mqtt-venus"
```

Vérifier que `/data/rc.local` contient bien la ligne de persistance :

```bash
ssh root@192.168.1.120 "cat /data/rc.local"
```

### 12.8 sv introuvable (commande inconnue)

**Symptôme** : `-sh: sv: command not found`

**Cause** : Venus OS utilise **daemontools**, pas runit. La commande `sv` n'existe pas.

**Solution** : utiliser `svc` et `svstat` à la place.

```bash
# Équivalences :
# sv status dbus-mqtt-venus  → svstat /service/dbus-mqtt-venus
# sv restart dbus-mqtt-venus → svc -t /service/dbus-mqtt-venus
# sv stop dbus-mqtt-venus    → svc -d /service/dbus-mqtt-venus
# sv start dbus-mqtt-venus   → svc -u /service/dbus-mqtt-venus
```

### 12.9 logread non fonctionnel (BusyBox)

**Symptôme** : `logread: can't find syslogd buffer: No such file or directory`

**Solution** : lancer le binaire manuellement (service stoppé) pour voir les logs en
direct :
```bash
svc -d /service/dbus-mqtt-venus
/data/daly-bms/dbus-mqtt-venus --config /data/daly-bms/config.toml 2>&1 | head -30
```

### 12.10 ps aux non supporté (BusyBox)

**Symptôme** : `ps: invalid option -- 'a'`

**Solution** : utiliser `ps` sans options (BusyBox sh ne supporte pas les options POSIX
de `ps aux`).

```bash
ps | grep daly
```

### 12.11 Run script incorrect — crash loop

**Symptôme** : crash loop — le service redémarre toutes les secondes.

**Cause** : `/service/dbus-mqtt-venus/run` pointe vers un mauvais chemin (ex:
`/data/dbus-mqtt-venus` au lieu de `/data/daly-bms/dbus-mqtt-venus`).

**Solution** : corriger le script run directement sur le NanoPi :

```bash
cat > /service/dbus-mqtt-venus/run << 'EOF'
#!/bin/sh
exec /data/daly-bms/dbus-mqtt-venus \
    --config /data/daly-bms/config.toml \
    2>&1
EOF
chmod +x /service/dbus-mqtt-venus/run
svc -t /service/dbus-mqtt-venus
```

### 12.12 Permission denied sur install-venus.sh

```bash
chmod +x nanoPi/install-venus.sh
# ou contournement sans chmod :
ARCH=armv7 bash nanoPi/install-venus.sh 192.168.1.120
```

### 12.13 energy-manager ne démarre pas après redémarrage Pi5

```bash
systemctl status energy-manager
journalctl -u energy-manager -n 30
# Vérifier la présence de config.toml :
ls -la /etc/daly-bms/config.toml
```

---

## 13. Annexe — Paramètres Victron switch complets

Référence wiki Victron pour `com.victronenergy.switch` :

**Chemins génériques :**

| Chemin | R/W | Description |
|--------|-----|-------------|
| `/State` | R | État global du module. `0x100`=Connected, `0x101`=Over temp, `0x102`=Temp warning, `0x103`=Channel fault, `0x104`=Channel Tripped, `0x105`=Under Voltage |

**Chemins de configuration par canal (x = index canal) :**

| Chemin | R/W | Description |
|--------|-----|-------------|
| `/Channel/x/Direction` | RW (opt.) | Direction du canal : 0=output, 1=input, -1=not defined |

**Chemins opérationnels par canal :**

| Chemin | R/W | Description |
|--------|-----|-------------|
| `/SwitchableOutput/x/State` | RW (opt.) | État ON/OFF demandé du canal |
| `/SwitchableOutput/x/Dimming` | RW (opt.) | 0–100%, lecture/écriture (sorties dimmables uniquement) |
| `/SwitchableOutput/x/LightControls` | RW (opt.) | Tableau d'entiers : [Hue 0–360°, Saturation 0–100%, Brightness 0–100%, White 0–100%, ColorTemp 0–6500K] (multi-canal dimmers types 11/12/13) |
| `/SwitchableOutput/x/Measurement` | R (opt.) | Valeur mesurée de l'actionneur (ex: température si setpoint) |
| `/SwitchableOutput/x/Name` | R | Nom par défaut du canal (non modifiable) |
| `/SwitchableOutput/x/Status` | R | État du canal : `0x00`=Off, `0x09`=On, `0x02`=Tripped, `0x04`=Over temp, `0x01`=Powered, `0x08`=Output fault, `0x10`=Short fault, `0x20`=Disabled, `0x40`=Bypassed, `0x80`=Ext. control |
| `/SwitchableOutput/x/Auto` | RW (opt.) | Mode auto : 0=Manuel (défaut), 1=Auto |
| `/SwitchableOutput/x/Temperature` | R (opt.) | Température du switch |
| `/SwitchableOutput/x/Voltage` | R (opt.) | Tension de sortie |
| `/SwitchableOutput/x/Current` | R (opt.) | Courant en ampères |

**Paramètres de configuration par canal :**

| Chemin | R/W | Description |
|--------|-----|-------------|
| `/SwitchableOutput/x/Settings/Adjustable` | R (opt.) | 0=paramètres non modifiables |
| `/SwitchableOutput/x/Settings/Group` | RW | Groupe d'affichage (max 32 bytes UTF-8) |
| `/SwitchableOutput/x/Settings/CustomName` | RW | Étiquette (max 32 bytes UTF-8) |
| `/SwitchableOutput/x/Settings/ShowUIControl` | RW | Affichage UI : `0b001`=tous UI, `0b000`=masqué, `0b010`=UI local, `0b100`=VRM |
| `/SwitchableOutput/x/Settings/Type` | RW | Type de sortie : 0=momentary, 1=toggle, 2=dimmable (PWM), 3=Temp setpoint, 4=Stepped switch, 5=Slave (ES), 6=Dropdown, 7=Basic slider, 8=Numeric input, 9=Three-state, 10=Bilge pump, 11=RGB, 12=CCT, 13=RGBW |
| `/SwitchableOutput/x/Settings/ValidTypes` | R | Champ binaire des types valides pour l'UI |
| `/SwitchableOutput/x/Settings/Function` | RW (opt.) | 0=Alarm, 1=Generator start/stop, 2=Manual, 3=Tank pump, 4=Temperature, 5=Genset helper relay, 6=Opportunity load |
| `/SwitchableOutput/x/Settings/ValidFunctions` | R | Champ binaire des fonctions valides |
| `/SwitchableOutput/x/Settings/FuseRating` | RW (opt.) | Courant de coupure fusible en A |
| `/SwitchableOutput/x/Settings/DimmingMin` | RW (opt.) | Valeur dimming minimum (défaut: 0) |
| `/SwitchableOutput/x/Settings/DimmingMax` | RW (opt.) | Valeur dimming maximum (défaut: 100) |
| `/SwitchableOutput/x/Settings/StepSize` | RW (opt.) | Pas de la sortie dimmable (défaut: 1) |
| `/SwitchableOutput/x/Settings/Decimals` | RW (opt.) | Nombre de décimales pour l'affichage UI |
| `/SwitchableOutput/x/Settings/Unit` | RW (opt.) | Unité affichée. Mots-clés spéciaux : `"/Speed"` (base m/s), `"/Temperature"` (base °C), `"/Volume"` (base m³) |
| `/SwitchableOutput/x/Settings/Polarity` | RW (opt.) | 0=Active high/Normally open, 1=Active low/Normally closed |
| `/SwitchableOutput/x/Settings/StartupState` | RW (opt.) | État au démarrage : 0=Off, 1=On, 2=Restore from memory (défaut) |
| `/SwitchableOutput/x/Settings/StartupDimming` | RW (opt.) | Dim au démarrage : 0–100 ou -1=Restore from memory (défaut) |
| `/SwitchableOutput/x/Settings/DimCurve` | RW (opt.) | Courbe de dimming : 0=Linear, 1=Optical |
| `/SwitchableOutput/x/Settings/OutputLimitMin` | RW (opt.) | Duty cycle PWM pour 0% dim (float 0–100%) |
| `/SwitchableOutput/x/Settings/OutputLimitMax` | RW (opt.) | Duty cycle PWM pour 100% dim (float 0–100%) |
| `/SwitchableOutput/x/Settings/FuseDetection` | RW (opt.) | Détection fusible : 0=Disabled, 1=Enabled, 2=Only when off |
| `/SwitchableOutput/x/Settings/Labels` | RW (opt.) | Étiquettes multi-option switch (tableau JSON de strings) |

---

## 14. Annexe historique — driver Python dbus-mqtt-battery

> Statut : MIGRATION TERMINÉE — section historique, conservée pour référence.

Avant le crate `dbus-mqtt-venus` Rust (Phase 3 de la roadmap), les batteries Daly
étaient exposées sur D-Bus via le driver Python
[dbus-mqtt-battery](https://github.com/mr-manuel/venus-os_dbus-mqtt-battery)
installé sur le NanoPi.

### Architecture originale (Python)

```
PC Windows (Rust RS485 natif)
       │
       ▼ MQTT publish (toutes les 1s, retain=true)
FlashMQ — 192.168.1.120:1883 (broker intégré Venus OS)
  santuario/bms/1/venus
  santuario/bms/2/venus
       │
       ▼ subscribe (dbus-mqtt-battery)
dbus-mqtt-battery-41  →  com.victronenergy.battery.mqtt_battery_141
dbus-mqtt-battery-42  →  com.victronenergy.battery.mqtt_battery_142
       │
       ▼ D-Bus Venus OS
  GUI / VRM Portal / systemcalc / hub4control
```

### Configuration Python (obsolète)

Les fichiers de configuration déployés sur le NanoPi :

**BMS 1 (360Ah — adresse RS485 0x01)** :
```ini
# /data/etc/dbus-mqtt-battery-41/config.ini
[DEFAULT]
logging = WARNING
device_name = MQTT Battery 360Ah
device_instance = 141
timeout = 0

[MQTT]
broker_address = 127.0.0.1
broker_port = 1883
topic = santuario/bms/1/venus
```

**BMS 2 (320Ah — adresse RS485 0x02)** :
```ini
# /data/etc/dbus-mqtt-battery-42/config.ini
[DEFAULT]
logging = WARNING
device_name = MQTT Battery 320Ah
device_instance = 142
timeout = 0

[MQTT]
broker_address = 127.0.0.1
broker_port = 1883
topic = santuario/bms/2/venus
```

- `broker_address = 127.0.0.1` : loopback plus fiable que l'IP au démarrage car
  l'interface réseau peut ne pas être configurée quand les services démarrent.
- `timeout = 0` : désactive la déconnexion automatique si aucune donnée reçue.

### Services D-Bus Python (instances héritées)

| Service D-Bus | Device Instance | Batterie |
|---------------|----------------|---------|
| `com.victronenergy.battery.mqtt_battery_141` | 141 | BMS-360Ah |
| `com.victronenergy.battery.mqtt_battery_142` | 142 | BMS-320Ah |

### Commandes de gestion Python (obsolètes)

```bash
svc -t /service/dbus-mqtt-battery-41  # restart
svc -t /service/dbus-mqtt-battery-42  # restart
svstat /service/dbus-mqtt-battery-41
svstat /service/dbus-mqtt-battery-42
tail -50 /var/log/dbus-mqtt-battery-41/current
```

### Migration CAN → MQTT (historique)

Lors du passage de `dbus-serialbattery.py can0` (CAN bus) vers le flux MQTT :

```bash
# Arrêter le service CAN (récupère ~60 MB RAM + 6% CPU)
svc -d /service/dbus-canbattery.can0
svstat /service/dbus-canbattery.can0   # doit afficher "down"
```

### Ressources système NanoPi avec Python (historique)

| Process | RAM | CPU | Notes |
|---------|-----|-----|-------|
| `flashmq` | ~49 MB | 0% | Broker MQTT local (port 1883) |
| `dbus-mqtt-battery-41/42` | ~42 MB × 2 | 1% | Bridge MQTT → D-Bus Python |
| `dbus-canbattery.can0` | ~60 MB | 0% | **STOPPÉ** — remplacé par MQTT |
| `dbus-mqtt-venus` (Rust) | ~5–8 MB | <1% | Remplace tout ce qui précède |

### État de la migration

- Phase 3 terminée : `dbus-mqtt-venus` Rust couvre toutes les batteries + tous les
  capteurs additionnels.
- La phase 4 (nettoyage NanoPi, retrait services Python) peut nécessiter une action
  manuelle si les instances Python sont encore actives.
- Les instances 141/142 (`mqtt_battery_141`, `mqtt_battery_142`) peuvent coexister
  temporairement avec 151/152 (Rust) ; supprimer les services Python une fois la
  stabilité Rust confirmée.

> ⚠️ Divergence : `nanoPi/README.md` décrit l'architecture Python (`dbus-mqtt-battery`
> avec instances 141/142). C'est l'état **antérieur** à la Phase 3. L'état courant
> (autorité) est le service Rust `dbus-mqtt-venus` avec instances 151/152/153.
> `nanoPi/README.md` est **conservé** comme référence historique.

---

## Voir aussi

- [./integration-materiel.md](./integration-materiel.md) — Ajout d'un BMS Daly côté
  RS485 Pi5 et configuration NanoPi (la partie D-Bus Venus est ici, la partie RS485 est
  là-bas).
- [./deploiement-exploitation.md](./deploiement-exploitation.md) — Workflow complet de
  déploiement Pi5 + NanoPi, systemd, logs.
- [./mqtt-mosquitto.md](./mqtt-mosquitto.md) — Architecture MQTT, bridge Pi5→NanoPi,
  anti-boucle, migration Docker→natif.
- [./app-energy-manager.md](./app-energy-manager.md) — energy-manager Pi5 : source de
  plusieurs topics MQTT consommés par dbus-mqtt-venus (temperature, heatpump, switch,
  platform).
- [./app-daly-bms-server.md](./app-daly-bms-server.md) — daly-bms-server Pi5 : source
  des topics BMS, ET112, ATS, PRALRAN.
- [nanoPi/README.md](../nanoPi/README.md) — Configuration Venus OS, driver Python
  historique (voir aussi section 14).
- Références Victron D-Bus :
  - <https://github.com/victronenergy/venus/wiki/dbus>
  - <https://github.com/sebdehne/dbus-mqtt-services>

---

## Sources consolidées

Ce document fusionne et **remplace** les anciens fichiers suivants :
`docs/VENUS-DEVICE-INTEGRATION.md`, `docs/DEPLOY-VENUS-ARMV7.md`.

Les fichiers suivants sont **conservés** (non remplacés) et référencés en « Voir aussi » :
`nanoPi/README.md`, `CLAUDE.md`, `Readme.md`.
