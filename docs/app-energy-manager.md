# Energy-Manager — Référence Complète — Daly-BMS-Rust

> Document de référence du binaire **energy-manager** : automatisation énergie Pi5 (port 8081, remplace Node-RED).
> Couvre l'architecture interne, les 12 modules logiques, les 7 règles GRL, les clients HTTP, le MQTT, le WebSocket live, la configuration et le dépannage.
> Fait partie de l'[architecture documentaire](./ARCHITECTURE.md).
> Dernière consolidation : 2026-06-07.

---

## Table des matières

- [1. Vue d'ensemble](#1-vue-densemble)
  - [1.1 Rôle et positionnement](#11-rôle-et-positionnement)
  - [1.2 Flux de données général](#12-flux-de-données-général)
  - [1.3 AppBus — le bus central](#13-appbus--le-bus-central)
  - [1.4 EnergyState — état partagé](#14-energystate--état-partagé)
- [2. Structure du crate](#2-structure-du-crate)
  - [2.1 Arborescence des fichiers clés](#21-arborescence-des-fichiers-clés)
  - [2.2 Supervision fail-fast (supervise.rs / spawn_critical)](#22-supervision-fail-fast-supervisers--spawn_critical)
  - [2.3 Démarrage séquentiel (main.rs)](#23-démarrage-séquentiel-mainrs)
- [3. Modules logiques — inventaire](#3-modules-logiques--inventaire)
- [4. Modules avec règles GRL](#4-modules-avec-règles-grl)
  - [4.1 INVERTER — ⚠️ rule engine dead code](#41-inverter---️-rule-engine-dead-code)
  - [4.2 CHARGE_CURRENT](#42-charge_current)
  - [4.3 DEYE_COMMAND](#43-deye_command)
  - [4.4 WATER_HEATER](#44-water_heater)
  - [4.5 SOLAR_POWER](#45-solar_power)
  - [4.6 SMARTSHUNT](#46-smartshunt)
  - [4.7 IRRADIANCE](#47-irradiance)
- [5. Modules sans règles GRL](#5-modules-sans-règles-grl)
  - [5.1 METEO](#51-meteo)
  - [5.2 TASMOTA](#52-tasmota)
  - [5.3 SWITCH_ATS — ⚠️ dead code (keepalive seul)](#53-switch_ats---️-dead-code-keepalive-seul)
  - [5.4 PLATFORM](#54-platform)
  - [5.5 VICTRON_KEEPALIVE](#55-victron_keepalive)
- [6. Sources de données externes](#6-sources-de-données-externes)
  - [6.1 Open-Meteo (météo)](#61-open-meteo-météo)
  - [6.2 LG ThinQ (PAC chauffe-eau)](#62-lg-thinq-pac-chauffe-eau)
- [7. Topics MQTT — référence complète](#7-topics-mqtt--référence-complète)
  - [7.1 Topics souscrits (dynamiques)](#71-topics-souscrits-dynamiques)
  - [7.2 Topics souscrits (hardcodés)](#72-topics-souscrits-hardcodés)
  - [7.3 Topics publiés par energy-manager](#73-topics-publiés-par-energy-manager)
  - [7.4 Topics de persistance](#74-topics-de-persistance)
- [8. Measurements écrits dans metrics-store (redb)](#8-measurements-écrits-dans-metrics-store-redb)
  - [8.1 solar_power](#81-solar_power)
  - [8.2 solar_persist](#82-solar_persist)
  - [8.3 battery_status](#83-battery_status)
  - [8.4 inverter_status](#84-inverter_status)
  - [8.5 switch_ats](#85-switch_ats)
  - [8.6 deye_relay](#86-deye_relay)
- [9. WebSocket live events (:8081/live)](#9-websocket-live-events-8081live)
- [10. API HTTP (:8081)](#10-api-http-8081)
  - [10.1 Endpoints disponibles](#101-endpoints-disponibles)
  - [10.2 Payload /api/rules-status](#102-payload-apirules-status)
- [11. Persistance et restauration au démarrage](#11-persistance-et-restauration-au-démarrage)
- [12. Configuration — [energy_manager]](#12-configuration--energy_manager)
  - [12.1 Sections et paramètres complets](#121-sections-et-paramètres-complets)
  - [12.2 Secrets (.env)](#122-secrets-env)
- [13. Guide : modifier une fonctionnalité existante](#13-guide--modifier-une-fonctionnalité-existante)
  - [13.1 Changer un seuil ou délai (sans recompiler)](#131-changer-un-seuil-ou-délai-sans-recompiler)
  - [13.2 Hot-reload des règles GRL (sans recompilation)](#132-hot-reload-des-règles-grl-sans-recompilation)
  - [13.3 Changer la logique métier (recompilation requise)](#133-changer-la-logique-métier-recompilation-requise)
- [14. Guide : ajouter un nouveau module logique](#14-guide--ajouter-un-nouveau-module-logique)
- [15. Guide : ajouter un nouveau publish MQTT](#15-guide--ajouter-un-nouveau-publish-mqtt)
- [16. Guide : supprimer un module](#16-guide--supprimer-un-module)
- [17. Tests unitaires des règles](#17-tests-unitaires-des-règles)
- [18. Installation initiale du service](#18-installation-initiale-du-service)
- [19. Débogage](#19-débogage)
- [20. Dépannage spécifique](#20-dépannage-spécifique)
- [21. Annexe historique — IMPLEMENTATION_VERIFICATION.md (OBSOLÈTE)](#21-annexe-historique--implementation_verificationmd-obsolète)
- [22. Annexe historique — DASHBOARD_EXTENSION_GUIDE.md, parties energy-manager (OBSOLÈTE)](#22-annexe-historique--dashboard_extension_guidemd-parties-energy-manager-obsolète)
- [Voir aussi](#voir-aussi)
- [Sources consolidées](#sources-consolidées)

---

## 1. Vue d'ensemble

### 1.1 Rôle et positionnement

`energy-manager` est un binaire Rust autonome qui **remplace Node-RED** (anciennement utilisé comme orchestrateur de flux d'automatisation énergétique). Il tourne en service systemd sur le Pi5 (`energy-manager.service`), écoute le broker MQTT Mosquitto local, applique la logique métier via des règles GRL et des modules Rust, et publie sur MQTT et WebSocket.

Positionnement dans l'architecture :
```
Pi5 (192.168.1.141, pi5compute)
  energy-manager (systemd, :8081)
    ├── MQTT subscribe/publish → 127.0.0.1:1883 (broker local Mosquitto natif)
    ├── Logique solaire, DEYE, chauffe-eau, charge, météo
    ├── WebSocket live events :8081/live
    └── publication MQTT → consommée par daly-bms-server (writes metrics-store)
```

Voir [./mqtt-mosquitto.md] pour l'architecture détaillée du broker MQTT et du bridge NanoPi.
Voir [./metriques-redb-architecture.md] pour le détail de metrics-store/redb.
Voir [./deploiement-exploitation.md] pour le déploiement et la gestion systemd.

### 1.2 Flux de données général

```
MQTT (Mosquitto :1883)
        │ Publish (N/... Victron, stat/... Tasmota, etc.)
        ▼
  mqtt/client.rs  ──broadcast──▶  AppBus.mqtt_in
                                      │
                        ┌─────────────┼────────────── ... ──┐
                        ▼             ▼                      ▼
                  logic/solar   logic/deye      logic/water_heater ...
                  _power.rs     _command.rs         (11 modules)
                        │             │                      │
             ┌──────────┴─────────────┴──────────────────────┘
             │         │                     │
             ▼         ▼                     ▼
     AppBus.  AppBus.mqtt_out     AppBus.live
             │         │                     │
      /      mqtt/client.rs       live_ws/server.rs
      client.rs    (publisher)          (WebSocket :8081/live)
             │         │
            MQTT publish
               (W/... Victron, santuario/...)
```

### 1.3 AppBus — le bus central

`bus.rs` expose **4 canaux** :

| Canal | Type | Usage |
|-------|------|-------|
| `mqtt_in` | `broadcast::Sender<MqttIncoming>` | Tous les messages MQTT entrants → tous les modules |
| `mqtt_out` | `mpsc::Sender<MqttOutgoing>` | Publish MQTT depuis n'importe quel module |
| `live` | `broadcast::Sender<LiveEvent>` | Événements WebSocket live |
| `rule_reload` | `broadcast::Sender<String>` | Signal hot-reload → tous les modules de règles |

Cloner `AppBus` est gratuit (tous les champs sont `Arc`-backed).

### 1.4 EnergyState — état partagé

`Arc<RwLock<EnergyState>>` dans `types.rs` — struct avec tous les champs mesurés. Les modules écrivent via `.write().await`, lisent via `.read().await`.

| Groupe | Champs |
|--------|--------|
| Solaire | `mppt_power_273_w`, `mppt_power_289_w`, `pvinverter_power_w`, `solar_total_w`, `house_power_w` |
| Batterie | `soc_pct`, `battery_voltage_v`, `battery_current_a`, `battery_power_w`, `battery_state`, `time_to_go_sec` |
| AC/Grid | `ac_ignore` (0=réseau, 1=hors-réseau), `ac_frequency_hz` |
| VEBus | `dc_voltage_v`, `dc_current_a`, `dc_power_w`, `ac_out_voltage_v`, `ac_out_current_a`, `ac_out_power_w` |
| Chauffe-eau | `water_heater_mode`, `water_heater_temp_c`, `water_heater_target_c`, `water_heater_last_read`, `water_heater_last_change`, `water_heater_send_count`, `water_heater_temp_max_since` |
| DEYE | `deye_on`, `deye_last_change`, `deye_lockout_until` |
| Compteurs | `total_yield_today_kwh`, `pvinv_baseline_kwh`, `yield_yesterday_kwh`, `ah_charged_today`, `ah_discharged_today`, `shunt_charged_today_kwh`, `shunt_discharged_today_kwh` |

---

## 2. Structure du crate

### 2.1 Arborescence des fichiers clés

```
crates/energy-manager/
  src/
    config.rs          ← chargement [energy_manager] depuis Config.toml
    types.rs           ← types partagés (EnergyState, MqttIncoming, ...)
    bus.rs             ← AppBus (broadcast MQTT)
    main.rs            ← démarrage séquentiel de tous les modules
    monitoring.rs      ← métriques système + tokio (TODO : republier dans metrics-store)
    supervise.rs       ← spawn_critical (supervision fail-fast)
    logic/
      charge_current.rs
      deye_command.rs
      inverter/
        mod.rs
        rules.rs       ← ⚠️ dead code (jamais appelé depuis mod.rs)
      irradiance.rs
      meteo.rs
      platform.rs
      smartshunt.rs
      solar_power.rs
      switch_ats.rs    ← ⚠️ ne lit jamais le MQTT (keepalive seul)
      tasmota.rs
      victron_keepalive/
        mod.rs
      water_heater.rs
    mqtt/
      client.rs        ← client MQTT rumqttc
      topics.rs        ← constantes et fonctions de topics
    http_clients/
      open_meteo.rs    ← client Open-Meteo (météo)
      lg_thinq.rs      ← client LG ThinQ (PAC)
    live_ws/
      server.rs        ← WebSocket live events (:8081/live)
    persist/
      mod.rs           ← restauration baselines au démarrage
  rules/
    charge_current.grl
    deye_command.grl
    inverter.grl       ← ⚠️ défini mais jamais chargé (dead code)
    irradiance.grl
    smartshunt.grl
    solar_power.grl
    water_heater.grl
```

### 2.2 Supervision fail-fast (supervise.rs / spawn_critical)

Les boucles de service longue durée passent par `spawn_critical` (défini dans `supervise.rs`). Si une boucle retourne ou panique (via `panic=abort`), le process s'arrête → redémarrage automatique par systemd (`Restart=on-failure`).

**Règle absolue** : ne jamais appeler `spawn_critical` sur une tâche transitoire (one-shot, timer, traitement par-snapshot) : elle se termine normalement, ce qui provoquerait un exit indésirable.

Conséquence : plus de boucle de polling morte silencieuse pendant que le service paraît « up ».

### 2.3 Démarrage séquentiel (main.rs)

Les modules sont démarrés séquentiellement dans `main.rs`, chacun recevant un clone de `AppBus` et un `Arc<RwLock<EnergyState>>` :

```
1. Client MQTT (connexion broker local)
2. Restauration baselines (persist/)
3. Démarrage séquentiel des 12 modules logic/ via spawn_critical ou tokio::spawn
4. Démarrage serveur HTTP/WS Axum (:8081)
5. Run principal (boucle tokio)
```

---

## 3. Modules logiques — inventaire

| Fichier | Rôle | Entrées MQTT | Sorties |
|---------|------|--------------|---------|
| `solar_power.rs` | Puissance solaire temps réel, baseline journalière | MPPT power/yield, PVInverter power/energy | `solar_power` (1/s), MQTT `santuario/em/solar` (1/s), LiveEvent `solar` |
| `meteo.rs` | Publication météo Venus + reset minuit | état partagé | MQTT `santuario/meteo/venus`, `santuario/heat/1/venus`, `solar_persist` (1/jour) |
| `inverter.rs` | Données onduleur VEBus | `N/.../vebus/...` | EnergyState, LiveEvent `inverter`, MQTT `santuario/inverter/venus` |
| `smartshunt.rs` | Données batterie SmartShunt + intégration Ah | `N/.../system/0/Dc/Battery/...` | EnergyState, LiveEvent `battery`, MQTT `santuario/system/venus` |
| `irradiance.rs` | Capteur irradiance PRALRAN | HTTP GET daly-bms-server (30s) | EnergyState, LiveEvent `irradiance` |
| `tasmota.rs` | Relais chauffe-eau Tasmota | `stat/{id}/POWER`, `tele/{id}/SENSOR` | EnergyState, LiveEvent `tasmota_wh*` |
| `deye_command.rs` | Coupure DEYE — fréquence AC + état MPPT (seuil unique 51,0) | `N/.../vebus/.../Ac/Out/L1/F`, état MPPT | MQTT Shelly RPC, EnergyState |
| `water_heater.rs` | Contrôle mode chauffe-eau LG ThinQ | état partagé (SOC, solaire, grid) | MQTT `santuario/heatpump/1/venus`, API LG ThinQ |
| `charge_current.rs` | Courant de charge VEBus | `IgnoreAcIn1`, PV power, consumption | MQTT `W/.../MaxChargeCurrent`, `W/.../PowerAssistEnabled` |
| `switch_ats.rs` | ⚠️ Keepalive ATS CHINT (ne lit JAMAIS MQTT) | aucune (timer 60s) | MQTT `santuario/switch/1/venus` |
| `platform.rs` | Statut plateforme (backup) | aucune (timer configurable) | MQTT `santuario/platform/venus` |
| `victron_keepalive/mod.rs` | Keepalive Venus OS (GX broker) | aucune (timer 30s) | MQTT `R/{portal_id}/keepalive` (vide) |

---

## 4. Modules avec règles GRL

Les règles `.grl` utilisent le crate `rust-rule-engine`. Le répertoire des règles est `crates/energy-manager/rules/`. Un mécanisme de **hot-reload** (sans redémarrage) est disponible si `[energy_manager.rules] dir` est configuré (voir §13.2).

### 4.1 INVERTER — ⚠️ rule engine dead code

**Fichiers** : `logic/inverter/mod.rs` + `logic/inverter/rules.rs`

> ⚠️ **Note de correction** : La règle GRL `INV_AC_Power_Ready` est définie dans `rules.rs` mais **jamais appelée** — `mod rules;` est absent de `mod.rs`. Le rule engine d'INVERTER est **dead code**.

**Rôle effectif** : Lire et publier les mesures VEBus vers MQTT (le module fonctionne, seul le rule engine est inutilisé).

**Topics en entrée** : `N/{pid}/vebus/{vb}/*` (voltage DC/AC, courant, puissance, fréquence, état, IgnoreAcIn1)

**Publication MQTT retained** (`santuario/inverter/venus`) :
```json
{
  "Voltage":     "<dc_voltage_v>",
  "Current":     "<dc_current_a>",
  "Power":       "<dc_power_w>",
  "AcVoltage":   "<ac_out_voltage_v>",
  "AcCurrent":   "<ac_out_current_a>",
  "AcPower":     "<ac_out_power_w>",
  "AcFrequency": "<ac_frequency_hz>",
  "State":       "on",
  "Mode":        "inverter",
  "IgnoreAcIn":  "<ac_ignore>",
  "VebusState":  "<vebus_state>"
}
```

Champs `State` et `Mode` sont **hardcodés** (`"on"` / `"inverter"`).

**Événement live WebSocket** : stream `"inverter"`.

---

### 4.2 CHARGE_CURRENT

**Fichier** : `logic/charge_current.rs`

**Rôle** : Ajuster le courant de charge VEBus selon l'état réseau et l'excédent PV.

**Topics en entrée** :
- `N/{pid}/vebus/{vb}/Ac/State/IgnoreAcIn1` → `ac_ignore`
- `N/{pid}/system/0/Ac/PvOnOutput/L1/Power` → `mppt_power_273_w` (**champ partagé avec SOLAR_POWER**)
- `N/{pid}/system/0/Ac/ConsumptionOnOutput/L1/Power` → `house_power_w`

**Règles GRL** (`charge_current.grl`) :

```
CC_Offgrid       : ac_ignore==1 → mode="offgrid"
CC_Grid_PV_Excess: ac_ignore==0 && pv_excess==true → mode="grid_pv_excess"
CC_Grid_No_Excess: ac_ignore==0 && pv_excess==false → mode="grid_no_excess"

pv_excess = (pv_w - cons_w) > pv_excess_threshold_w
```

**Arbre de décision** :
```
ac_ignore == 1 ?
├─ OUI → offgrid (70A, assist=1)
└─ NON → (pv_w - cons_w) > 50W ?
         ├─ OUI → grid_pv_excess (4A, assist=0, feed_in=0)
         └─ NON → grid_no_excess (0A, assist=0, feed_in=0)
```

**Sorties MQTT transient** :
- `W/{pid}/vebus/{vb}/Dc/0/MaxChargeCurrent` → charge_a (70A / 4A / 0A selon mode)
- `W/{pid}/vebus/{vb}/Settings/PowerAssistEnabled` → 1 (offgrid) ou 0
- `W/{pid}/settings/0/Settings/CGwacs/MaxFeedInPower` → 0 (si réseau)

| Mode | Condition | charge_a | power_assist |
|------|-----------|----------|--------------|
| offgrid | ac_ignore==1 | 70A (déf) | 1 |
| grid_pv_excess | réseau + excédent | 4A (déf) | 0 |
| grid_no_excess | réseau + pas excédent | 0A (déf) | 0 |

**Config** : `[energy_manager.charge_current]` — `offgrid_max_a`, `grid_pv_excess_a`, `grid_no_excess_a`, `pv_excess_threshold_w=50W`

---

### 4.3 DEYE_COMMAND

> ⚠️ **MISE À JOUR 2026-06 — logique native simple (plus de moteur GRL ni de machine d'états latchable).**
> L'ancienne implémentation (moteur `rust-rule-engine` + machine à 5 états `On→PendingCut→Lockout→Off→PendingRestore→On`)
> pouvait **rester coincée en `Lockout`** (relais figé OFF des heures, même fréquence redescendue et MPPT en Bulk),
> car la sortie du `Lockout` dépendait d'un timer évalué dans la boucle. Remplacée par une **fonction pure
> ré-évaluée chaque seconde** — `DeyeController::evaluate()` dans `mod.rs` :
>
> ```
> should_cut = freq_hz >= freq_high_hz  OU  mppt_full        // OFF
> restore    = freq_hz <  freq_high_hz  ET  !mppt_full        // ON
>   • coupe immédiate si freq >= freq_hard_hz (51,3)
>   • sinon coupe après cut_delay_secs (3 s) de condition « cut » soutenue
>   • restaure après reenable_delay_secs (45 s) de condition « clair » soutenue
> ```
>
> **Il n'y a aucun état qui se latch** : le relais SUIT en permanence la décision dérivée des entrées.
> Tant que le ticker 1 Hz tourne, le relais converge toujours → **impossible de rester coincé**.
> Gardes de fraîcheur conservées : freq périmée → traitée nominale (50 Hz, restaure permise) ;
> état MPPT périmé → traité « pas plein » (ne bloque pas). Plus de `lockout_secs`/`mppt_cut_delay_secs`,
> plus de `deye_command.grl`. Les sections « Règles GRL » / « machine d'états » ci-dessous sont **obsolètes**
> (gardées pour l'historique).

**Fichier** : `logic/deye_command/mod.rs` (logique native, sans moteur de règles)

**Rôle** : Couper/restaurer les onduleurs DEYE via un Shelly Pro 2PM (**un canal par DEYE**). Le but est de **pré-empter l'auto-coupure des DEYE à 51,5 Hz** (qui provoque des micro-coupures sur AC Out) par une déconnexion relais propre et déterministe, dès 51,0 Hz.

> **Décision : Fréquence AC + état des MPPT, UNIQUEMENT.** Aucune autre variable
> n'intervient : ni réseau (`grid_connected`/`ac_ignore`/`ac_connected`), ni SmartShunt
> (SOC/irradiance/courant). La décision se résume à deux signaux :
> 1. la **fréquence AC-Out** (seuil unique 51,0 Hz, immédiat à 51,3) ;
> 2. l'**état des MPPT** : « batterie pleine » dès qu'un MPPT (273 **ou** 289) passe en
>    `4`=Absorption, `5`=Float ou `6`=Storage (`mppt_full_states`).

> 📘 **Contexte — curtailment PV : AC-couplé vs DC-couplé**
> L'installation a deux sources PV avec deux mécanismes de bridage **distincts** :
> - **DC-couplé** (MPPT Victron sur Lynx, inst. 273/289) : bridage **continu et progressif** par la régulation de charge du MPPT (Bulk → Absorption → Float). Batterie pleine ⇒ le MPPT tient sa tension de consigne en **sortant du point de puissance maximale (MPP)** → courant réduit, **sans à-coup**. DVCC (désactivé ici) n'ajouterait qu'un contrôle centralisé CVL/CCL depuis le GX ; il **n'est pas requis** pour ce bridage ni pour publier l'état.
> - **AC-couplé** (micro-onduleurs DEYE sur AC Out) : onduleurs réseau **non pilotables en courant**. Seul levier = le **décalage de fréquence** du MultiPlus (≈ 50,2 → 51,5 Hz), avec un **trip dur et brutal à 51,5 Hz** → micro-coupures. D'où ce module + le relais Shelly.
>
> Conséquence : le Victron a un levier **fin et continu** sur le DC-couplé (courant), mais seulement un levier **grossier et discret** sur l'AC-couplé (fréquence).
>
> **Signal « batterie pleine » exploitable (sans DVCC)** : l'état des MPPT
> `N/{pid}/solarcharger/{273,289}/State` est publié sur MQTT **indépendamment de DVCC**
> (la télémétrie ≠ le contrôle) et **déjà** stocké dans `EnergyState`
> (`mppt_273.state`, `mppt_289.state`). Codes : `0`=Off, `2`=Fault, `3`=Bulk,
> `4`=Absorption, `5`=Float, `6`=Storage, `7`=Equalize, `11`=Other(Hub-1),
> `252`=External control. **`Absorption`/`Float`/`Storage` (4/5/6) ⇒ batterie pleine**
> (MPPT en régulation de tension) → coupe/maintien coupé ; **`Bulk` (3) ⇒ la batterie
> accepte encore de la charge** → autorise la restauration.
> Voir [./integration-materiel.md] pour le détail matériel.

**Topics en entrée** :
- `N/{pid}/vebus/{vb}/Ac/Out/L1/F` → fréquence AC (Hz) — **autorité de coupure**. Le ticker (1 Hz) recale la fréquence de décision sur la valeur partagée `ac_frequency_hz` (maintenue par le module `inverter`, **identique au widget**) via `decision_freq()` : la décision ne peut donc jamais diverger de l'affichage ni rester figée sur une dernière valeur locale haute si l'abonnement propre à la boucle deye se tarit.
- État des MPPT (`mppt_273.state`, `mppt_289.state` dans `EnergyState`, alimentés par `solar_power`) — second et **seul autre** signal de décision
- `N/{pid}/vebus/{vb}/Ac/ActiveIn/Connected` → `ac_connected` — **purement informatif** (ligne « Réseau » du widget), **n'intervient plus** dans la décision
- `Ac/State/IgnoreAcIn1` → `ac_ignore` — **purement informatif** (idem)

> ℹ️ **Réseau retiré de la décision** : la fonction `is_grid_connected(ac_ignore, ac_connected)`
> est conservée **uniquement pour l'affichage** (widget monitor → ligne « Réseau (info) »).
> Elle ne supprime plus aucune coupure et ne force plus aucune reconnexion. Conséquence
> assumée : si un MPPT atteint un palier plein **alors que le réseau EDF est présent**, les
> DEYE sont quand même coupés.

**Conception** : une seule couche, deux signaux.
- **Fréquence** (autorité = mesure Victron, règle projet #13) — coupe/restaure sur un **seuil unique** `freq_high_hz` (51,0 Hz) : `≥ 51,0` → côté coupe (débounce `cut_delay_secs`, ou immédiat à `freq_hard_hz`=51,3) ; `< 51,0` → côté restauration. **Pas de zone morte.**
- **État MPPT** — coupe anticipée quand la batterie est pleine (`mppt_cut`, débouncé), et gate de restauration (`restore_blocked`).

**Anti-rebattement (mécanisme anti-déclenchements répétés du Shelly)** : purement **temporel**,
puisque la frontière de fréquence est unique. `cut_delay_secs` (3 s, débounce avant coupure douce)
+ `lockout_secs` (120 s, **temps mort obligatoire après coupure** — l'état `Lockout` interdit toute
restauration pendant ce délai) + `reenable_delay_secs` (45 s sous le seuil avant restauration). Cycle
coupe→restauration minimal ≈ **165 s** → fréquence de bascule du relais bornée. Réglable via `lockout_secs`.

**Gardes de fraîcheur (anti-blocage relais sur télémétrie figée)** : la décision n'utilise jamais une
entrée périmée. Chaque écriture horodate sa source (`ac_frequency_last_ts` par le module `inverter`,
`mppt_*.state_last_ts` par `solar_power`). Au-delà de `input_max_age_secs` (90 s, soit 3× le keepalive
Venus 30 s), l'entrée est « périmée » (topic muet alors que la boucle reste vivante) et :
- **état MPPT périmé** → traité comme **NON plein** (`effective_mppt_full`) : un état figé « batterie pleine »
  ne peut plus verrouiller le relais ouvert (c'est exactement la classe de bug évitée) ;
- **fréquence périmée** → traitée comme **nominale (50 Hz)** (`effective_freq`) : restauration permise,
  aucune coupure fréquence — le filet est l'**auto-trip matériel DEYE à 51,5 Hz** (politique ops).

Côté fréquence, la décision recale aussi `last_freq` sur la valeur partagée `ac_frequency_hz` (= widget,
via `decision_freq`) pour ne jamais diverger de l'affichage. Les drapeaux `freq_stale`/`mppt_stale` sont
exposés (`/api/rules-status`) et signalés en rouge dans le widget « Gestion Relais DEYE ».

**Règles GRL** (`deye_command.grl`) :

| État courant | Condition | → Nouvel état | Relay | Salience |
|---|---|---|---|---|
| On / PendingCut | freq ≥ `freq_hard_hz` (51,3) | Lockout | **OFF** (les 2 canaux) | 150 |
| On / PendingCut | `mppt_cut==true` (batterie pleine côté MPPT, débouncé) | Lockout | **OFF** (les 2 canaux) | 130 |
| On | freq ≥ `freq_high_hz` (51,0), < hard, `mppt_cut==false` | PendingCut | — | 100 |
| PendingCut | `cut_delay_secs` (3 s) écoulé + freq ≥ 51,0 | Lockout | **OFF** | 100 |
| PendingCut | freq < 51,0 (annule, si `mppt_cut==false`) | On | — | 100 |
| Lockout | `lockout_secs` (120 s) écoulé | Off | — | 100 |
| Off | freq < `freq_high_hz` (51,0) **et `restore_blocked==false`** | PendingRestore | — | 100 |
| PendingRestore | freq ≥ 51,0 (annule) | Off | — | 100 |
| PendingRestore | `restore_blocked==true` (batterie redevenue pleine) | Off | — | 100 |
| PendingRestore | `reenable_delay_secs` (45 s) écoulé + freq < 51,0 + `restore_blocked==false` | On | **ON** (les 2 canaux) | 100 |

> Aucune règle « réseau » : les anciennes règles de reconnexion (salience 200) et les gardes
> `grid_connected==false` ont été supprimées.

**`mppt_cut`** (coupure anticipée, salience 130 — désactivable via `mppt_cut_enabled`) : un MPPT (273/289) est dans un état « batterie pleine » (`mppt_full_states`, défaut `[4,5,6]` = Absorption/Float/Storage), maintenu `mppt_cut_delay_secs` (10 s). But : **couper les DEYE dès le palier d'absorption** pour terminer la charge sur le seul MPPT (DC-couplé, sans à-coup), **avant** toute montée en fréquence. La fréquence (51,0/51,3) reste en filet.

**`restore_blocked`** (pré-calculé en Rust) = `mppt_battery_full` **uniquement** (un MPPT dans `mppt_full_states`). Tant qu'elle est vraie, pas de restauration → les DEYE restent coupés jusqu'à ce que les MPPT repassent en `Bulk` (3) **et** que la fréquence soit < 51,0. (Plus aucune garde SmartShunt : un MPPT en Bulk autorise toujours la restauration.)

**Diagramme de la machine d'états** :

> Coupure **immédiate** (relay_off) depuis `On` **ou** `PendingCut` : `freq ≥ 51,3 Hz`
> (salience 150) **ou** `mppt_cut` (batterie pleine côté MPPT, salience 130) → `Lockout`.
> Le chemin temporisé ci-dessous gère la coupure douce (≥ 51,0 Hz) et la restauration.

```
   ┌────────┐  freq ≥ 51,0 Hz (3 s)   ┌────────────┐  3 s + freq ≥ 51,0 Hz   ┌─────────┐
   │   On   │ ──────────────────────► │ PendingCut │ ──────────────────────► │ Lockout │
   └────────┘ ◄── freq < 51,0 Hz ──── └────────────┘     (relay_off ×2)      └────┬────┘
       ▲                                                                          │ expire 120 s
       │ relay_on ×2                                                              ▼
       │ (45 s + freq < 51,0 Hz                                              ┌─────────┐
       │  ET restore_blocked = false)                                       │   Off   │
   ┌───┴────────────┐                                                       └────┬────┘
   │ PendingRestore │ ◄── freq < 51,0 Hz  ET  restore_blocked = false ───────────┘
   └────────────────┘
       │ annulation : freq ≥ 51,0 Hz  OU  restore_blocked = true
       └──────────────────────────────► Off
```

**Commande Shelly MQTT transient** (envoyée sur **chaque** canal) :
```
Topic   : {shelly_id}/rpc  (ex: shellypro2pm-ec62608840a4/rpc)
Payload (pour chaque canal de shelly_deye_channels) :
{
  "id": 1,
  "src": "energy-manager",
  "method": "Switch.Set",
  "params": { "id": <channel>, "on": true|false }
}
```

**Re-synchronisation idempotente** : toutes les `relay_resync_secs` (60 s), l'état logique courant est ré-émis sur tous les canaux → le relais physique reconverge après un message MQTT manqué ou un reboot du Shelly. La première émission a lieu au démarrage (affirme l'état restauré). Log en `DEBUG` ; seules les transitions réelles sont en `INFO`.

**Persistance de l'état DEYE** : L'état du relais (On/Off) est persisté en MQTT retained sur `santuario/persist/deye_state`. Au redémarrage, le service attend 3 secondes avant d'activer la logique DEYE pour laisser le broker MQTT livrer le message retained.

États persistés :
- `"On"` — DEYE actif (états `On` ou `PendingCut`)
- `"Off"` — DEYE coupé (états `Off`, `Lockout`, `PendingRestore`)

**Config** (`[energy_manager.deye]`) : `freq_high_hz=51.0` (seuil unique coupe/restaure), `freq_hard_hz=51.3` (coupe immédiate), `cut_delay_secs=3`, `reenable_delay_secs=45`, `lockout_secs=120`, `relay_resync_secs=60`, `mppt_cut_enabled=true`, `mppt_full_states=[4,5,6]`, `mppt_cut_delay_secs=10`, `input_max_age_secs=90` (garde de fraîcheur freq + MPPT). *(Retirés : `freq_low_hz`, `restore_soc_pct`, `restore_irradiance_wm2`, `corroboration_max_age_secs`, `mppt_charging_states` — plus de zone morte ni de garde SmartShunt.)*
Canaux : `[energy_manager.victron] shelly_deye_channels = [0, 1]` (un canal par DEYE ; fallback mono-canal `shelly_deye_channel` si la liste est vide).

---

### 4.4 WATER_HEATER

**Fichier** : `logic/water_heater.rs`

**Rôle** : Piloter la PAC LG ThinQ (modes `HEAT_PUMP` / `VACATION`) selon les conditions énergétiques.

**Deux tâches Tokio** : keepalive (toutes les 25 s) + control task (toutes les 5 min).

**Entrées règle** :
- `grid_connected` ← `ac_ignore==0` (réseau connecté)
- `soc_pct` ← EnergyState
- `irradiance_low` ← `irradiance_wm2 < 300 W/m²`

**Règles GRL** (`water_heater.grl`) :
```
Conditions (salience 100) :
  si grid_connected==true  → want_vacation=true
  si soc_pct < 90          → want_vacation=true
  si irradiance_low==true  → want_vacation=true
  si temp_max_reached==true → want_vacation=true   (cuve à température cible)

Décision (salience 200) :
  si want_vacation==true   → target_mode="VACATION"
  sinon                    → target_mode="HEAT_PUMP"
```

**Logique condensée** : `HEAT_PUMP` requiert **les 4 conditions simultanément** :
```
HEAT_PUMP exige les 4 SIMULTANÉMENT :
✓ grid_connected == false (ac_ignore=1)
✓ soc_pct ≥ 90
✓ irradiance_wm2 ≥ 300 W/m²
✓ temp_max_reached == false (cuve PAS encore à température cible)

Sinon → VACATION
```

**Condition « cuve à température cible »** : la température actuelle de la cuve
est lue régulièrement (poller LG ThinQ + control_task 5 min) et stockée dans
`water_heater_temp_c`. Le control_task date le premier instant où elle atteint
`temp_max_c` (défaut **60 °C**) via `water_heater_temp_max_since`. Si elle reste
≥ ce seuil pendant ≥ `temp_max_hold_secs` (défaut **600 s = 10 min**), le fait
`temp_max_reached` passe à `true` → la PAC bascule en `VACATION` (inutile de
continuer à chauffer). Le suivi est réarmé dès que la température redescend.

**Flux de contrôle** (toutes les 5 min) :
1. `lg.get_state()` → actual_mode, temp, target_temp
2. Vérifier données MQTT (ac_ignore, soc) — skip si absent
3. `rule_engine.evaluate(...)` → target_mode
4. Si target != actual **ET** cooldown (900 s) expiré :
5. `lg.set_mode(target_mode)` ← synchrone
6. Sleep 15 s (dans une tâche Tokio séparée **non-bloquante**)
7. `lg.set_target_temp(...)` dans la tâche séparée

> ⚠️ Les étapes 6-7 s'exécutent en **arrière-plan** via `tokio::spawn` — la boucle principale continue.

**Sorties** :
- API LG ThinQ POST `/control` (mode + temp)
- MQTT retained `santuario/heatpump/1/venus` (keepalive toutes les 25 s)
- Métriques redb via metrics-store (toutes les 5 min)
- WebSocket live `"water_heater_venus"`

**Config** : `irradiance_min_wm2=300`, `mode_change_min_secs=900`, `heat_pump_target_c=60`, `vacation_target_c=45`, `temp_set_delay_secs=15`, `keepalive_secs=25`, `temp_max_c=60`, `temp_max_hold_secs=600`

---

### 4.5 SOLAR_POWER

**Fichier** : `logic/solar_power.rs`

**Rôle** : Agréger la puissance MPPT + ET112 micro-onduleurs, gérer la baseline journalière, publier vers daly-bms-server.

**Deux tâches** : `mqtt_task` (écoute MQTT + gestion baseline) + `writer_task` (POST HTTP toutes les 1 s).

**Topics en entrée** :
- MPPT 273 : power, yield today, state, pv voltage, pv current
- MPPT 289 : power, yield today, state, pv voltage, pv current
- ET112 energy forward (`N/{pid}/pvinverter/{pv}/#`)
- Consommation maison (`N/{pid}/system/0/Ac/ConsumptionOnOutput/L1/Power`)

**Règles GRL** (`solar_power.grl`) :
```
SOLAR_Reset_On_NewDay    : new_day==true → reset=true, capture=true
SOLAR_Capture_When_Absent: baseline_absent==true → capture=true
```

**Logique baseline ET112** :
```
pvinv_yield_today_kwh = (kwh_current - baseline).max(0.0)
```

**Sorties** :
- MQTT retained `santuario/persist/pvinv_baseline` (format `"{day}:{kwh:.3}"`)
- MQTT `santuario/em/solar` (toutes les 1 s) → consommé par daly-bms-server
  (remplace l'ancien POST HTTP `/api/v1/solar/mppt-yield`, conservé en fallback
  côté serveur — la rétention par requête HTTP était le moteur de la fuite RSS
  résiduelle, cf. `docs/diagnostic-depannage.md` §17)
- WebSocket live `"solar"`

**Nom du measurement** configurable : `solar.power_measurement` (défaut : `"solar_power"`).

---

### 4.6 SMARTSHUNT

**Fichier** : `logic/smartshunt.rs`

**Rôle** : Monitorer la batterie, intégrer les Ah, utiliser les compteurs kWh natifs du SmartShunt.

**Topics en entrée** :
- `N/{pid}/battery/{shunt}/Dc/0/*` (voltage, current, power, soc, state)
- `N/{pid}/battery/{shunt}/History/ChargedEnergy`, `DischargedEnergy`
- Fallback : `N/{pid}/system/0/Dc/Battery/*`

> ⚠️ **Note de correction** : Le code **n'a pas de priorité explicite** entre la source primaire (shunt) et le fallback (system). Les deux sources sont acceptées — "dernier écrit gagne". La documentation originale était trompeuse en parlant de "fallback avec priorité".

**Règles GRL** (`smartshunt.grl`) :
```
Capture baseline si : new_day==true OU baseline_absent==true
```

**Intégration Ah** (time-based, sans règle GRL) :
```
Si delta_ms ∈ [1ms, 600_000ms] :
  ah_charged    += current_a * delta_h
  ah_discharged += |current_a| * delta_h
```

**Sortie MQTT retained** : `santuario/system/venus`
**Événement live WebSocket** : stream `"battery"`

---

### 4.7 IRRADIANCE

**Fichier** : `logic/irradiance.rs`

**Rôle** : Récupérer la valeur d'irradiance du capteur RS485 PRALRAN (adresse 0x05) via daly-bms-server.

**Source** : HTTP GET `{bms_server}/api/v1/irradiance/status` (toutes les 30 s)

**Règle GRL** (`irradiance.grl`) :
```
IR_Valid_Range : raw ∈ [0, 2000] W/m² → valid=true
```

**Comportement** :
- `irradiance_wm2` toujours mis à jour dans EnergyState (même hors plage)
- LiveEvent WebSocket `"irradiance"` émis uniquement si `valid==true`

> ⚠️ **Note de correction** : Le topic MQTT `santuario/irradiance/raw` est **souscrit** dans `mqtt/topics.rs` mais **aucun module ne le traite** — dead code. La source réelle est le polling HTTP vers daly-bms-server.

---

## 5. Modules sans règles GRL

### 5.1 METEO

**Fichier** : `logic/meteo.rs`

**Rôle** : Publier la météo vers Venus OS et effectuer le reset de minuit.

**Deux tâches** :
- Publish (toutes les 60 s) → MQTT `santuario/heat/1/venus` + `santuario/meteo/venus`
- Reset minuit (5 s après minuit) → sauvegarde `yield_yesterday`, efface les baselines journalières

**Contenu MQTT `santuario/meteo/venus`** : IrradianceWm2, MpptPower, TodaysYield (agrégat depuis EnergyState)

---

### 5.2 TASMOTA

**Fichier** : `logic/tasmota.rs`

**Rôle** : Surveiller le relais chauffe-eau Tasmota (Tongou).

**Topics en entrée (hardcodés)** :
- `stat/tongou_3BC764/POWER` → état ON/OFF
- `tele/tongou_3BC764/SENSOR` → métriques énergie

**Sorties WebSocket** :
- `"tasmota_wh"` — état du relais (on: bool)
- `"tasmota_wh_energy"` — métriques (power_w, voltage_v, current_a, today_kwh, total_kwh)

---

### 5.3 SWITCH_ATS — ⚠️ dead code (keepalive seul)

**Fichier** : `logic/switch_ats.rs`

> ⚠️ **Note de correction** : Ce module **ne lit JAMAIS** le MQTT. Il publie uniquement des valeurs par défaut (keepalive). Il n'y a **pas de suivi d'état** de l'ATS CHINT depuis ce module. La fonction `set_position()` est marquée `#[allow(dead_code)]` — jamais appelée.

**Rôle réel** : Keepalive uniquement. Toutes les 60 s, publie MQTT retained `santuario/switch/1/venus` :
```json
{"Position": 0, "State": 0}
```

Les champs `ats_position` et `ats_state` restent à leur valeur par défaut (0) dans EnergyState. Aucune source MQTT ne les met à jour.

> ⚠️ La documentation originale (DASHBOARD_EXTENSION_GUIDE.md) était totalement incorrecte sur ce module — elle présentait un "suivi ATS" inexistant.

---

### 5.4 PLATFORM

**Fichier** : `logic/platform.rs`

**Rôle** : Publish d'un statut plateforme (backup/heartbeat).

**Comportement** : Toutes les 60 s (configurable via `[energy_manager.platform] publish_interval_secs`), publie MQTT `santuario/platform/venus` avec `Status=0` (idle).

---

### 5.5 VICTRON_KEEPALIVE

**Fichier** : `logic/victron_keepalive/mod.rs`

**Rôle** : Maintenir le flux de données Venus OS actif.

Venus OS (Cerbo GX) publie les topics télémétriques `N/{portal_id}/...` en continu **tant qu'un client MQTT externe publie périodiquement sur `R/{portal_id}/keepalive`**. Sans ce signal, le GX cesse d'émettre après ~60 secondes, coupant toutes les données Venus OS reçues par energy-manager et daly-bms-server.

**Comportement** :
- Topic publié : `R/c0619ab9929a/keepalive` (payload vide `""`, **non retained**)
- Fréquence : **30 s** (marge confortable avant le timeout de 60 s du GX)
- Démarré au lancement d'energy-manager — aucune condition préalable
- Journalisé en `DEBUG` uniquement (pas de pollution des logs en production)

**Configuration** : le `portal_id` est l'identifiant GX Victron visible dans VRM :
```toml
[energy_manager.victron]
portal_id = "c0619ab9929a"   # ID Cerbo GX — ne pas modifier sans màj VRM
```

**Diagnostic** :
```bash
# Vérifier que les topics N/ arrivent bien (signe que le keepalive fonctionne)
mosquitto_sub -h 127.0.0.1 -p 1883 -t 'N/c0619ab9929a/#' -C 5 -v

# Si rien n'arrive : vérifier que energy-manager tourne
systemctl status energy-manager
journalctl -u energy-manager --since "2 minutes ago" | grep -i keepalive

# Forcer un keepalive manuel (test ou dépannage)
mosquitto_pub -h 127.0.0.1 -p 1883 -t 'R/c0619ab9929a/keepalive' -m ''
```

**Historique — suppression du conteneur Docker** : Avant mai 2026, ce keepalive était assuré par un conteneur Docker manuel `dalybms-venus-keepalive` (image `eclipse-mosquitto:2.0.18`, hors docker-compose) qui exécutait :
```sh
while true; do
  mosquitto_pub -h mosquitto -p 1883 -t 'R/c0619ab9929a/keepalive' -m '' -q 0
  sleep 55
done
```
Ce conteneur pointait sur le hostname Docker `mosquitto` (l'ancien broker conteneurisé). Lors de la migration vers Mosquitto natif (mai 2026), il est devenu inopérant ("Unable to connect (Lookup error.)") **sans impacter le système** car le module Rust prenait déjà le relais. Le conteneur a été **supprimé définitivement** avec `cleanup-docker.sh`. Le keepalive est maintenant entièrement géré par energy-manager.

---

## 6. Sources de données externes

### 6.1 Open-Meteo (météo)

**Fichier** : `http_clients/open_meteo.rs`

**Config** : `[energy_manager.open_meteo]`
- `enabled` (défaut : `true`)
- `latitude` (défaut : `43.9025`)
- `longitude` (défaut : `7.8364`)
- `poll_interval_secs` (défaut : `300` — 5 min)

**Données collectées** : `temperature_c`, `humidity_pct`, `pressure_hpa`, `wind_speed_ms`

**Événement live WebSocket** : stream `"weather"` avec `temperature_c`, `humidity_pct`, `pressure_hpa`, `wind_speed_ms`

---

### 6.2 LG ThinQ (PAC chauffe-eau)

**Fichier** : `http_clients/lg_thinq.rs`

**Config** : `[energy_manager.lg_thinq]`
- `enabled` (défaut : `false` — doit être mis à `true` pour activer)
- `base_url`
- `device_id` — dans `/etc/daly-bms/.env`
- `bearer_token` — dans `/etc/daly-bms/.env`
- `api_key` — dans `/etc/daly-bms/.env`
- `poll_interval_secs` (défaut : `600` — 10 min)

**Endpoints utilisés** :
- `GET /devices/{device_id}/state` — lecture de l'état actuel (mode, temp, target_temp)
- `POST /devices/{device_id}/control` — écriture mode + température cible

**Utilisation** : uniquement par le module `water_heater.rs` pour piloter la PAC. Si `lg_thinq.enabled=false`, le water_heater publie quand même le keepalive MQTT mais ne fait aucun appel API.

---

## 7. Topics MQTT — référence complète

Le broker est Mosquitto natif systemd sur Pi5 (:1883). Voir [./mqtt-mosquitto.md] pour l'architecture détaillée du broker et du bridge NanoPi.

### 7.1 Topics souscrits (dynamiques)

Construits dynamiquement avec `portal_id` et les instances Victron :

```
N/{pid}/vebus/{vb}/#
  (IgnoreAcIn1, Ac/State/IgnoreAcIn1, Ac/Out/L1/F, Ac/ActiveIn/Connected,
   Dc/0/Voltage, Dc/0/Current, Dc/0/Power, Ac/Out/L1/V, Ac/Out/L1/I,
   Ac/Out/L1/P, Ac/Out/L1/F, VebusMode, State)

N/{pid}/battery/{shunt}/#
  (Dc/0/Voltage, Dc/0/Current, Dc/0/Power, Soc, State,
   History/ChargedEnergy, History/DischargedEnergy)

N/{pid}/solarcharger/{mppt1}/#  (power, yield, state, pv voltage, current)
N/{pid}/solarcharger/{mppt2}/#

N/{pid}/pvinverter/{pv}/#  (power, energy forward)

N/{pid}/system/0/#
  (Dc/Battery/Soc, Dc/Battery/Voltage, Dc/Battery/Current, Dc/Battery/Power,
   Ac/PvOnOutput/L1/Power, Ac/ConsumptionOnOutput/L1/Power)
```

Instances configurées dans `[energy_manager.victron]` :
- `vebus_instance = 275`
- `mppt1_instance = 273`
- `mppt2_instance = 289`
- `pvinverter_instance = 32`
- `smartshunt = 274`

### 7.2 Topics souscrits (hardcodés)

```
santuario/irradiance/raw         ⚠️ souscrit mais AUCUN module ne traite ce topic — dead code
santuario/persist/pvinv_baseline  (persistance baseline)
santuario/persist/yield_yesterday (persistance production hier)
santuario/persist/deye_state      (persistance état relais DEYE)
shellypro2pm-ec62608840a4/events/rpc  (réponses Shelly — hardcodé)
stat/tongou_3BC764/POWER          (Tasmota — hardcodé)
tele/tongou_3BC764/SENSOR         (Tasmota — hardcodé)
```

**ABSENT** du code : `santuario/switch/+/venus` (contrairement à ce que pouvait laisser croire l'ancienne documentation).

### 7.3 Topics publiés par energy-manager

| Topic | Type | Contenu | Fréquence |
|-------|------|---------|-----------|
| `santuario/inverter/venus` | retained | JSON VEBus (tensions, courants, état) | À chaque mise à jour |
| `santuario/system/venus` | retained | JSON SmartShunt (tension, courant, SOC) | À chaque mise à jour |
| `santuario/meteo/venus` | retained | JSON météo + agrégats solaires | Toutes les 60 s |
| `santuario/heat/1/venus` | retained | JSON température capteur ext. | Toutes les 60 s |
| `santuario/heatpump/1/venus` | retained | JSON PAC chauffe-eau | Toutes les 25 s (keepalive) |
| `santuario/switch/1/venus` | retained | JSON ATS keepalive `{"Position":0,"State":0}` | Toutes les 60 s |
| `santuario/platform/venus` | retained | JSON statut `{"Status":0}` | Configurable (60 s déf.) |
| `R/{portal_id}/keepalive` | transient | vide `""` | Toutes les 30 s |
| `W/{pid}/vebus/{vb}/Dc/0/MaxChargeCurrent` | transient | charge_a | Sur changement |
| `W/{pid}/vebus/{vb}/Settings/PowerAssistEnabled` | transient | 0 ou 1 | Sur changement |
| `W/{pid}/settings/0/Settings/CGwacs/MaxFeedInPower` | transient | 0 | Sur changement |
| `{shelly_id}/rpc` | transient | JSON Switch.Set | Sur coupure/restauration DEYE |
| `santuario/persist/pvinv_baseline` | retained | `"{day}:{kwh:.3}"` | Sur reset journalier |

### 7.4 Topics de persistance

| Topic | Format | Restauré au démarrage |
|-------|--------|----------------------|
| `santuario/persist/pvinv_baseline` | `"{day}:{kwh:.3}"` | Oui, si même jour |
| `santuario/persist/yield_yesterday` | `"{kwh:.3}"` | Oui |
| `santuario/persist/deye_state` | `"On"` ou `"Off"` | Oui, après 3 s |

---

## 8. Measurements écrits dans metrics-store (redb)

Les métriques sont publiées via HTTP POST vers `daly-bms-server`, qui les écrit dans redb. Voir [./metriques-redb-architecture.md] pour les détails du moteur de stockage.

### 8.1 solar_power

> Source : `logic/solar_power.rs` — fréquence : **1 point par seconde**

| Type | Nom | Valeur |
|------|-----|--------|
| Tag | `day` | Date locale `YYYY-MM-DD` |
| Tag | `host` | Valeur de `solar.host_tag` (défaut : `"pi5"`) |
| Field (f64) | `solar_total_w` | Puissance solaire totale (MPPT + PVInverter) |
| Field (f64) | `mppt_power_w` | Puissance MPPT totale (273 + 289) |
| Field (f64) | `mppt_273_w` | Puissance MPPT 273 seul (W) |
| Field (f64) | `mppt_273_voltage_v` | Tension PV MPPT 273 (V) |
| Field (f64) | `mppt_273_current_a` | Courant DC MPPT 273 (A) |
| Field (f64) | `mppt_273_yield_kwh` | Production MPPT 273 du jour (kWh) |
| Field (i64) | `mppt_273_state` | État MPPT 273 (0=OFF, 3=Bulk, 4=Abs, 5=Float) |
| Field (f64) | `mppt_289_w` | Puissance MPPT 289 seul (W) |
| Field (f64) | `mppt_289_voltage_v` | Tension PV MPPT 289 (V) |
| Field (f64) | `mppt_289_current_a` | Courant DC MPPT 289 (A) |
| Field (f64) | `mppt_289_yield_kwh` | Production MPPT 289 du jour (kWh) |
| Field (i64) | `mppt_289_state` | État MPPT 289 |
| Field (f64) | `pvinv_power_w` | Puissance micro-onduleurs ET112 (W) |
| Field (f64) | `pvinv_yield_kwh` | Production micro-onduleurs du jour (kWh) |
| Field (f64) | `total_yield_kwh` | Production totale du jour (kWh) |
| Field (f64) | `house_power_w` | Consommation maison (Ac/ConsumptionOnOutput) |

Nom du measurement configurable : `solar.power_measurement` (défaut : `"solar_power"`).

### 8.2 solar_persist

> Source : `logic/meteo.rs` — fréquence : **1 point par jour, à minuit**

| Type | Nom | Valeur |
|------|-----|--------|
| Tag | `day` | Date du jour qui se termine `YYYY-MM-DD` |
| Tag | `host` | Valeur de `solar.host_tag` (défaut : `"pi5"`) |
| Field (f64) | `total_yield_today_kwh` | Production totale du jour (kWh) |
| Field (f64) | `mppt_yield_today_kwh` | Production MPPT seuls (kWh) |
| Field (f64) | `pvinv_yield_today_kwh` | Production micro-onduleurs (kWh) |

Nom du measurement configurable : `solar.persist_measurement` (défaut : `"solar_persist"`).

### 8.3 battery_status

> Source : `logic/smartshunt.rs` — fréquence : **à chaque mise à jour MQTT** (~1/s)

| Type | Nom | Valeur |
|------|-----|--------|
| Tag | `host` | `"pi5"` |
| Field (f64) | `soc_pct` | État de charge batterie (%) |
| Field (f64) | `voltage_v` | Tension batterie (V) |
| Field (f64) | `current_a` | Courant batterie (A, + = charge, - = décharge) |
| Field (f64) | `power_w` | Puissance batterie (W) |
| Field (i64) | `state` | État : 0=idle, 1=charging, 2=discharging |
| Field (i64) | `time_to_go_sec` | Temps restant (s, -1 si inconnu) |

### 8.4 inverter_status

> Source : `logic/inverter.rs` — fréquence : **à chaque mise à jour MQTT** (~1/s)

| Type | Nom | Valeur |
|------|-----|--------|
| Tag | `host` | `"pi5"` |
| Field (f64) | `dc_voltage_v` | Tension DC bus (V) |
| Field (f64) | `dc_current_a` | Courant DC (A) |
| Field (f64) | `dc_power_w` | Puissance DC (W) |
| Field (f64) | `ac_out_voltage_v` | Tension AC sortie (V) |
| Field (f64) | `ac_out_current_a` | Courant AC sortie (A) |
| Field (f64) | `ac_out_power_w` | Puissance AC sortie (W) |
| Field (f64) | `ac_frequency_hz` | Fréquence AC sortie (Hz) |
| Field (i64) | `vebus_state` | État VEBus (2=inverter, 3=on, 9=passthrough…) |
| Field (i64) | `ac_ignore` | 0=grid connecté, 1=mode îlot |

### 8.5 switch_ats

> Source : `logic/switch_ats.rs` — fréquence : **toutes les 60 secondes** (keepalive)

| Type | Nom | Valeur |
|------|-----|--------|
| Tag | `host` | `"pi5"` |
| Field (i64) | `position` | Position ATS : 0=Réseau, 1=Génératrice (toujours 0 — dead code) |
| Field (i64) | `state` | État ATS : 0=inactif, 1=actif, 2=alerte (toujours 0 — dead code) |

### 8.6 deye_relay

> Source : `logic/deye_command.rs` — fréquence : **à chaque changement d'état** (coupure ou réactivation)

| Type | Nom | Valeur |
|------|-----|--------|
| Tag | `host` | `"pi5"` |
| Tag | `shelly_id` | ID du Shelly (ex: `"shellypro2pm-ec62608840a4"`) |
| Field (i64) | `on` | État relais DEYE : 1=ON, 0=OFF (coupé) |

---

## 9. WebSocket live events (:8081/live)

**Endpoint** : `ws://<pi5>:8081/live`

Chaque événement est un JSON :
```json
{ "stream": "<nom>", "ts": "<ISO8601>", "data": {...} }
```

| Stream | Émis par | Contenu `data` |
|--------|----------|----------------|
| `solar` | `solar_power.rs` (1/s) | `solar_total_w`, `mppt_power_w`, `house_power_w` |
| `inverter` | `inverter.rs` | tension/courant/puissance DC+AC, état VEBus, `ac_ignore` |
| `battery` | `smartshunt.rs` | `soc_pct`, `current_a`, `voltage_v`, `battery_state`, `time_to_go_sec` |
| `irradiance` | `irradiance.rs` | `wm2` |
| `weather` | `open_meteo.rs` | `temperature_c`, `humidity_pct`, `pressure_hpa`, `wind_speed_ms` |
| `tasmota_wh` | `tasmota.rs` | `on` (bool) |
| `tasmota_wh_energy` | `tasmota.rs` | `power_w`, `voltage_v`, `current_a`, `today_kwh`, `total_kwh` |
| `water_heater_venus` | `water_heater.rs` | état PAC (mode, températures) |

**Diagnostic WebSocket** :
```bash
# Avec websocat (à installer si absent)
websocat ws://192.168.1.141:8081/live
```

---

## 10. API HTTP (:8081)

**Serveur** : Axum (écoute sur `0.0.0.0:8081`)

### 10.1 Endpoints disponibles

| Endpoint | Méthode | Description |
|----------|---------|-------------|
| `/live` | WS | Stream LiveEvent (voir §9) |
| `/health` | GET | Retourne `"energy-manager ok"` |
| `/api/water-heater` | GET | État PAC LG ThinQ (JSON) |
| `/api/water-heater/mode` | POST | Set mode PAC — body: `{"mode": "HEAT_PUMP"|"VACATION"|"TURBO"}` |
| `/api/rules-status` | GET | Snapshot état des règles (JSON) |
| `/api/v1/em/rules` | GET | Lister les règles chargées (origine: disk ou embedded, timestamp) |
| `/api/v1/em/rules/reload` | POST | Recharger une ou toutes les règles |

### 10.2 Payload /api/rules-status

```json
{
  "water_heater": {
    "mode": "VACATION|HEAT_PUMP|TURBO",
    "current_temp_c": null,
    "target_temp_c": null,
    "last_read_ts": null,
    "last_change_ts": null,
    "send_count": 0,
    "lg_enabled": false
  },
  "charge_current": {
    "current_a": null,
    "power_assist": null,
    "last_ts": null
  },
  "deye": {
    "on": true,
    "state": "On",
    "last_change": null,
    "restore_blocked": false,
    "grid_connected": true,
    "freq_hz": 50.01,
    "ac_connected": 1,
    "mppt_full": false,
    "mppt_273_state": 3,
    "mppt_289_state": 3,
    "freq_stale": false,
    "mppt_stale": false
  },
  "soc_pct": null,
  "irradiance_wm2": null,
  "ac_ignore": null
}
```

Champs `deye` :
- `state` — état machine : `On` / `PendingCut` / `Lockout` / `Off` / `PendingRestore`.
- `restore_blocked` — restauration différée car la batterie est pleine côté MPPT (un MPPT en `mppt_full_states` = 4/5/6). Seul gate de restauration (plus de garde SmartShunt).
- `grid_connected` — prédicat d'îlotage combiné (`ac_ignore != 1` **et** `ac_connected != 0`). **Informatif uniquement** : n'intervient plus dans la décision DEYE.
- `freq_hz` — fréquence AC-Out pilotant le seuil unique (≥51,0 coupe / <51,0 restaure).
- `ac_connected` — connexion physique réseau (`1`=connecté, `0`=panne). **Informatif.**
- `mppt_full` — au moins un MPPT signale « batterie pleine » (état dans `mppt_full_states`) → pilote la coupe anticipée et bloque la restauration.
- `mppt_273_state` / `mppt_289_state` — codes State des MPPT (`3`=Bulk, `4`=Absorption, `5`=Float, `6`=Storage, etc. — liste non exhaustive ; aussi `0`=Off, `2`=Fault, `7`=Equalize…).
- `freq_stale` — la télémétrie fréquence est périmée (topic muet > `input_max_age_secs`) → la décision la traite comme nominale (restauration permise ; filet = auto-trip DEYE 51,5 Hz).
- `mppt_stale` — la télémétrie d'état MPPT est périmée → traitée comme « pas plein » (ne bloque pas la restauration → anti-blocage relais).

Ces champs alimentent la carte **« Règles système → Gestion Relais DEYE »** de `/dashboard/monitor`.

---

## 11. Persistance et restauration au démarrage

Le module `persist/` écoute les topics MQTT retained au démarrage et restaure les baselines dans EnergyState.

**Topics restaurés** :

| Topic | Format | Condition de restauration |
|-------|--------|--------------------------|
| `santuario/persist/pvinv_baseline` | `"{day}:{kwh:.3}"` | Même jour que la date courante |
| `santuario/persist/yield_yesterday` | `"{kwh:.3}"` | Toujours restauré |
| `santuario/persist/deye_state` | `"On"` ou `"Off"` | Restauré après 3 s (attente broker) |

**SmartShunt** : baselines Ah en mémoire seulement — recalculées chaque jour, non persistées en MQTT.

**Mécanisme** : `spawn_persist_watcher` écoute les topics MQTT retained immédiatement après la connexion au broker, puis charge les valeurs si elles correspondent au jour courant.

---

## 12. Configuration — [energy_manager]

Le service lit `/etc/daly-bms/config.toml`, **PAS** `~/Daly-BMS-Rust/Config.toml`.

Après toute modification :
```bash
sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart energy-manager
```

### 12.1 Sections et paramètres complets

```toml
[energy_manager.mqtt]
host                 = "192.168.1.141"   # Broker MQTT (déf: 192.168.1.141)
port                 = 1883              # Port MQTT
# client_id          = ""               # Auto-généré UUID si absent
# username           = ""               # Optionnel
# password           = ""               # Optionnel
keep_alive_secs      = 60
reconnect_delay_secs = 5

[energy_manager.victron]
portal_id            = "c0619ab9929a"   # ID GX portal Victron (obligatoire)
vebus_instance       = 275              # Instance VEBus
mppt1_instance       = 273             # MPPT 1
mppt2_instance       = 289             # MPPT 2
pvinverter_instance  = 32
smartshunt           = 274             # Instance SmartShunt (batterie)
shelly_deye_id       = "shellypro2pm-ec62608840a4"
shelly_deye_channel  = 0               # Fallback mono-canal (hérité)
shelly_deye_channels = [0, 1]          # Un canal par DEYE — coupe/restaure les deux
tasmota_waterheater_id = "tongou_3BC764"

[energy_manager.deye]
# Décision : Fréquence AC + état des MPPT UNIQUEMENT (ni réseau, ni SmartShunt).
freq_high_hz               = 51.0   # Seuil UNIQUE coupe/restaure (pas de zone morte ; sous l'auto-trip DEYE 51.5)
freq_hard_hz               = 51.3   # Seuil de coupure immédiate (filet pré-trip)
cut_delay_secs             = 3      # Débounce avant coupure douce (PendingCut → Lockout)
reenable_delay_secs        = 45     # Bas-soutenu sous le seuil avant réactivation (PendingRestore → On)
lockout_secs               = 120    # Temps mort obligatoire après coupure (anti-rebattement principal)
relay_resync_secs          = 60     # Ré-affirmation périodique de l'état des 2 canaux
mppt_cut_enabled           = true   # Coupe les DEYE sur l'état de charge MPPT (sans DVCC)
mppt_full_states           = [4, 5, 6]  # Codes State « batterie pleine » (4=Absorption,5=Float,6=Storage)
mppt_cut_delay_secs        = 10     # Débounce du signal MPPT-plein avant coupure

[energy_manager.water_heater]
solar_min_w            = 2000.0   # Production min pour HEAT_PUMP (non utilisé par la règle GRL)
debounce_secs          = 300      # Délai de stabilisation (5 min)
mode_change_min_secs   = 900      # Intervalle minimal entre changements de mode (15 min)
heat_pump_target_c     = 60.0     # Température cible mode HEAT_PUMP
vacation_target_c      = 45.0     # Température cible mode VACATION
soc_min_pct            = 90.0     # SOC minimum pour activer HEAT_PUMP
irradiance_min_wm2     = 300      # Irradiance minimale pour HEAT_PUMP
temp_max_c             = 60.0     # Seuil « cuve à température cible »
temp_max_hold_secs     = 600      # Maintien ≥ ce seuil → VACATION (10 min)
temp_set_delay_secs    = 15       # Délai entre set_mode et set_target_temp
keepalive_secs         = 25       # Fréquence keepalive MQTT PAC
vm_url                 = "http://127.0.0.1:8080"  # URL daly-bms-server → metrics-store redb

[energy_manager.rules]
# Optionnel — si défini, les règles .grl sont lues depuis ce répertoire
# (hot-reload sans recompilation). Fallback sur les règles embarquées.
# dir = "/etc/daly-bms/rules"

[energy_manager.charge_current]
offgrid_max_a          = 70.0    # Courant max en mode hors-réseau (A)
grid_pv_excess_a       = 4.0    # Courant en mode réseau + excédent PV (A)
grid_no_excess_a       = 0.0    # Courant en mode réseau sans excédent (A)
pv_excess_threshold_w  = 50.0   # Seuil excédent PV (W)

[energy_manager.lg_thinq]
enabled                = false   # true si LG ThinQ utilisé
# base_url, device_id, bearer_token, api_key → voir §12.2

[energy_manager.open_meteo]
enabled                = true
latitude               = 43.9025
longitude              = 7.8364
poll_interval_secs     = 300

[energy_manager.solar]
bms_server_url         = "http://192.168.1.141:8080"  # URL daly-bms-server
# power_measurement    = "solar_power"                # Nom du measurement redb
# persist_measurement  = "solar_persist"              # Nom du measurement persist
# host_tag             = "pi5"                        # Tag host dans les métriques

[energy_manager.platform]
publish_interval_secs  = 60
```

### 12.2 Secrets (.env)

Les valeurs sensibles sont lues depuis `/etc/daly-bms/.env` — **ne jamais committer ce fichier** :

```env
LG_DEVICE_ID=<ID appareil LG>
LG_BEARER_TOKEN=<Bearer token LG>
LG_API_KEY=<API key LG>
```

---

## 13. Guide : modifier une fonctionnalité existante

### 13.1 Changer un seuil ou délai (sans recompiler)

Si le paramètre est exposé dans `Config.toml` (voir §12) :

```bash
# 1. Éditer Config.toml (localement)
# 2. Copier sur Pi5
sudo cp Config.toml /etc/daly-bms/config.toml
sudo systemctl restart energy-manager
```

### 13.2 Hot-reload des règles GRL (sans recompilation)

Les règles `.grl` peuvent être modifiées **en production** sans recompiler ni redémarrer.

**Prérequis** — configurer un répertoire dans `Config.toml` :
```toml
[energy_manager.rules]
dir = "/etc/daly-bms/rules"
```

Copier les règles initiales vers ce répertoire :
```bash
sudo mkdir -p /etc/daly-bms/rules
sudo cp crates/energy-manager/rules/*.grl /etc/daly-bms/rules/
```

Modifier la règle souhaitée, puis recharger :
```bash
# Recharger UNE règle (ex: water_heater)
curl -s -X POST http://192.168.1.141:8081/api/v1/em/rules/reload \
     -H "Content-Type: application/json" \
     -d '{"name":"water_heater"}'

# Recharger TOUTES les règles
curl -s -X POST http://192.168.1.141:8081/api/v1/em/rules/reload \
     -H "Content-Type: application/json" \
     -d '{"name":"*"}'

# Lister les règles chargées (origine: disk ou embedded, timestamp)
curl -s http://192.168.1.141:8081/api/v1/em/rules | python3 -m json.tool
```

Si `rules.dir` n'est pas configuré ou si le fichier n'existe pas sur disque, le système utilise automatiquement la règle **embarquée dans le binaire**.

> **Seuils chauffe-eau rechargeables à chaud** — l'appel `reload` ci-dessus
> (`{"name":"water_heater"}` ou `{"name":"*"}`) recharge **aussi** les seuils de
> la section `[energy_manager.water_heater]` depuis `Config.toml`, en plus du
> fichier `.grl`. Les paramètres `temp_max_c` (60 °C par défaut),
> `temp_max_hold_secs`, `irradiance_min_wm2`, `soc_min_pct`,
> `mode_change_min_secs`, `heat_pump_target_c`, `vacation_target_c` et
> `temp_set_delay_secs` prennent effet **sans recompiler ni redémarrer** le
> service. Le control_task et l'endpoint `/api/rules-status` partagent la même
> config (`Arc<RwLock<WaterHeaterConfig>>`) → la page « Règles système » reflète
> immédiatement la nouvelle valeur. Procédure :
>
> ```bash
> # 1. Éditer le seuil dans la config active du service
> sudo nano /etc/daly-bms/config.toml      # [energy_manager.water_heater] temp_max_c = 58.0
> # 2. Recharger à chaud (aucun restart)
> curl -s -X POST http://192.168.1.141:8081/api/v1/em/rules/reload \
>      -H "Content-Type: application/json" -d '{"name":"water_heater"}'
> # 3. Vérifier la valeur prise en compte
> curl -s http://192.168.1.141:8081/api/rules-status | python3 -m json.tool | grep temp_max
> ```
>
> Seul `keepalive_secs` (intervalle du ticker keepalive Venus) reste fixé au
> démarrage et nécessite un `systemctl restart energy-manager`.

### 13.3 Changer la logique métier (recompilation requise)

| Besoin | Fichier | Ce qu'il faut changer |
|--------|---------|----------------------|
| Seuil DEYE fréquence | `config.rs` (DeyeConfig) + `Config.toml` | Paramètre `freq_high_hz` |
| Seuils chauffe-eau (temp_max_c, soc_min_pct, irradiance_min_wm2…) | `Config.toml` `[energy_manager.water_heater]` | **Aucune recompilation** — éditer + `POST /api/v1/em/rules/reload` (hot-reload, cf. §13.2) |
| Ajouter un MPPT | `config.rs` (VictronConfig), `mqtt/topics.rs`, `solar_power.rs`, `types.rs` | Nouvelle instance + topic |
| Nouveau topic Victron à surveiller | `mqtt/topics.rs` (`all_subscriptions`) + module concerné | Abonnement + handler |
| Changer fréquence d'écriture DB | `logic/solar_power.rs` ligne `interval(Duration::from_secs(1))` | Valeur en secondes |
| Ajouter un champ DB | Module concerné, appel `.field_f(...)` sur `DB` | Nouveau field |

Après modification :
```bash
make build-energy-arm
sudo systemctl stop energy-manager
sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/
sudo systemctl start energy-manager
journalctl -u energy-manager -f
```

---

## 14. Guide : ajouter un nouveau module logique

**Étape 1 — Créer le fichier** `crates/energy-manager/src/logic/mon_module.rs` :

```rust
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::bus::AppBus;
use crate::types::EnergyState;

pub async fn spawn(bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    tokio::spawn(run(bus, state));
}

async fn run(bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    let mut rx = bus.subscribe_mqtt();
    loop {
        let msg = match rx.recv().await {
            Ok(m) => m,
            Err(_) => continue,
        };
        if msg.topic != "mon/topic" {
            continue;
        }
        // Traitement...
        let mut s = state.write().await;
        // s.mon_champ = ...;
    }
}
```

**Étape 2 — Déclarer dans `logic/mod.rs`** :
```rust
pub mod mon_module;
```

**Étape 3 — Brancher dans `main.rs`** :
```rust
logic::mon_module::spawn(bus.clone(), state.clone()).await;
```

**Étape 4 — Si configuration nécessaire**, ajouter dans `config.rs` :
```rust
#[derive(Debug, Clone, Deserialize, Default)]
pub struct MonModuleConfig {
    #[serde(default = "default_ma_valeur")]
    pub ma_valeur: f64,
}
fn default_ma_valeur() -> f64 { 42.0 }
```

Ajouter `pub mon_module: MonModuleConfig` dans `EnergyManagerConfig` avec `#[serde(default)]`.
Ajouter `[energy_manager.mon_module]` dans `Config.toml`.
Passer `cfg.mon_module.clone()` au `spawn()`.

**Étape 5 — Si le module écrit des champs EnergyState**, ajouter dans `types.rs` :
```rust
pub struct EnergyState {
    // ...
    pub mon_champ: Option<f64>,
}
```

**Étape 6 — Si le module abonne un topic MQTT**, l'ajouter dans `mqtt/topics.rs` :
```rust
pub fn all_subscriptions(...) -> Vec<String> {
    vec![
        // ...
        "mon/nouveau/topic".to_string(),
    ]
}
```

**Étape 7 — Compile et déploie** :
```bash
make build-energy-arm
sudo systemctl stop energy-manager
sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/
sudo systemctl start energy-manager
journalctl -u energy-manager -f
```

---

## 15. Guide : ajouter un nouveau publish MQTT

**Étape 1 — Déclarer le topic** dans `mqtt/topics.rs` :
```rust
pub mod publish {
    // Constante simple :
    pub const MON_TOPIC: &str = "santuario/mon/venus";
    // Ou fonction dynamique :
    pub fn mon_topic_dynamique(id: u32) -> String {
        format!("santuario/mon/{id}/venus")
    }
}
```

**Étape 2 — Publier depuis un module** :
```rust
use crate::mqtt::topics::publish;
use crate::types::MqttOutgoing;

// Retained (Venus OS keepalive) :
bus.publish(MqttOutgoing::retained(publish::MON_TOPIC, &payload)).await;

// Non-retained (événement ponctuel) :
bus.publish(MqttOutgoing::transient(publish::MON_TOPIC, &payload)).await;

// Texte brut :
bus.publish(MqttOutgoing::raw(publish::MON_TOPIC, "valeur", false)).await;
```

---

## 16. Guide : supprimer un module

1. Supprimer le fichier `logic/mon_module.rs`
2. Retirer `pub mod mon_module;` dans `logic/mod.rs`
3. Retirer la ligne `logic::mon_module::spawn(...)` dans `main.rs`
4. Retirer la section de config dans `config.rs` et `Config.toml` (si applicable)
5. Retirer les champs orphelins dans `EnergyState` (`types.rs`) si plus utilisés
6. Retirer les topics MQTT orphelins dans `mqtt/topics.rs`
7. Recompiler et déployer

---

## 17. Tests unitaires des règles

Chaque module de règles `.grl` a une suite de tests unitaires dans son fichier `rules.rs`.

```bash
# Lancer tous les tests (34 tests, ~0,3 s)
cargo test -p energy-manager

# Lancer les tests d'une règle spécifique
cargo test -p energy-manager deye_command
cargo test -p energy-manager water_heater
```

Couverture des tests :

| Module | Nb tests | Cas couverts |
|--------|---------|--------------|
| `charge_current` | 4 | offgrid, grid+PV, grid sans PV, etc. |
| `deye_command` | 26 | transitions, seuil unique (zéro zone morte), coupe MPPT, lockout, scénario 51,5 Hz |
| `irradiance` | 5 | valides/invalides, plage 0–2000 W/m² |
| `smartshunt` | 6 | capture baseline chargée/déchargée |
| `solar_power` | 4 | nouveau jour, baseline absente |
| `water_heater` | 6 | grid connecté, SOC bas, irradiance faible, cuve à température cible |

**Ajouter un test** — dans le module `rules.rs` concerné, dans `#[cfg(test)] mod tests` :

```rust
#[test]
fn mon_nouveau_cas() {
    let mut e = MonRuleEngine::new().unwrap();
    let result = e.evaluate(/* params */);
    assert_eq!(result.unwrap(), "EXPECTED");
}
```

---

## 18. Installation initiale du service

```bash
# Depuis le repo sur Pi5 :
make build-energy-arm
sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/
sudo cp contrib/energy-manager.service /etc/systemd/system/
sudo cp Config.toml /etc/daly-bms/config.toml
# Créer /etc/daly-bms/.env avec les secrets LG ThinQ si activé
sudo systemctl daemon-reload
sudo systemctl enable energy-manager
sudo systemctl start energy-manager
journalctl -u energy-manager -f
```

Pour un déploiement complet incluant mosquitto et Grafana : voir [./deploiement-exploitation.md].

---

## 19. Débogage

```bash
# Logs en continu
journalctl -u energy-manager -f

# Dernières 50 lignes
journalctl -u energy-manager -n 50

# Augmenter le niveau de log (sans recompiler)
sudo systemctl edit energy-manager
# Ajouter dans le fichier :
# [Service]
# Environment=RUST_LOG=debug

# Voir les messages MQTT entrants en temps réel
mosquitto_sub -h 192.168.1.141 -t "santuario/#" -v
mosquitto_sub -h 192.168.1.141 -t "N/c0619ab9929a/#" -v

# WebSocket live events
websocat ws://192.168.1.141:8081/live

# Vérifier l'état des règles
curl -s http://192.168.1.141:8081/api/rules-status | python3 -m json.tool

# Vérifier les règles chargées et leur origine (disk/embedded)
curl -s http://192.168.1.141:8081/api/v1/em/rules | python3 -m json.tool

# Santé du service
curl -s http://192.168.1.141:8081/health

# Vérifier que le keepalive Venus OS fonctionne
mosquitto_sub -h 127.0.0.1 -p 1883 -t 'N/c0619ab9929a/#' -C 5 -v
```

---

## 20. Dépannage spécifique

| Symptôme | Cause probable | Solution |
|----------|---------------|---------|
| `energy-manager ne démarre pas` | TOML manquant, section absente, ou `.env` absent | `journalctl -u energy-manager -n 50` — analyser l'erreur précise |
| `missing field energy_manager` | Section `[energy_manager]` absente du fichier de config | `sudo cp Config.toml /etc/daly-bms/config.toml` — vérifier que la section `[energy_manager]` est présente |
| `energy-manager ne reçoit pas MQTT` | `portal_id` incorrect, Mosquitto inaccessible sur `mqtt.host` | Vérifier `portal_id` dans Config.toml ; vérifier `systemctl status mosquitto-broker` ; tester `mosquitto_sub -h 127.0.0.1 -p 1883 -t '#' -v` |
| `LG ThinQ ne répond pas` | Bearer token expiré, API key invalide | Vérifier `LG_BEARER_TOKEN` et `LG_API_KEY` dans `/etc/daly-bms/.env` |
| Dashboard affiche cumul brut (pvinv) | Baseline MQTT absente ou incorrecte | Vérifier `pvinv_baseline` retained MQTT : `mosquitto_sub -h 127.0.0.1 -t 'santuario/persist/pvinv_baseline' -C 1` |
| Données Venus OS absentes (N/... silencieux) | Keepalive non reçu par le GX | Vérifier energy-manager actif ; forcer keepalive : `mosquitto_pub -h 127.0.0.1 -p 1883 -t 'R/c0619ab9929a/keepalive' -m ''` |
| Mode PAC LG ne change pas | Cooldown (900 s) non expiré, ou irradiance/SOC non mis à jour | Vérifier `/api/rules-status` ; vérifier logs `journalctl -u energy-manager -f` |
| DEYE ne se coupe pas malgré fréquence haute | Fréquence lue sur mauvais topic VEBus | Vérifier `vebus_instance` dans Config.toml et topic `N/{pid}/vebus/{vb}/Ac/Out/L1/F` |
| `rules.dir` ignoré | Chemin invalide ou permissions | Vérifier que le répertoire existe et contient les `.grl` ; checker les logs |
| `panic=abort` → redémarrage systemd | Comportement normal (supervision fail-fast) | Inspecter les logs avant l'arrêt pour identifier la cause |

---

## 21. Annexe historique — IMPLEMENTATION_VERIFICATION.md (OBSOLÈTE)

> Statut : MIGRATION TERMINÉE — section historique, conservée pour référence.
> Ce document décrivait l'ancienne architecture à base de flows Node-RED (energy-manager).
> La référence actuelle est ce document (`docs/app-energy-manager.md`).
> Date de l'implémentation décrite : 2026-04-05.

### Contexte historique

Au départ (avant l'implémentation décrite dans ce document), le système n'avait **aucune** remontée temps réel des métriques Victron vers le Pi5 :
- Les batteries BMS s'affichaient (via RS485 direct)
- Les MPPT, SmartShunt, Onduleur Victron restaient invisibles
- Le dashboard affichait "En attente de données" pour ces appareils
- Aucune intégration D-Bus Victron → API web

### Checklist d'implémentation (état à 2026-04-05)

**Core Data Structures (`state.rs` — daly-bms-server)**
- [x] `VenusInverter` struct — DC voltage/current/power + AC output measurements + state/mode
- [x] `VenusMppt` struct — power, voltage, current, yield, status
- [x] `VenusSmartShunt` struct — voltage, current, power, SOC, state
- [x] `VenusTemperature` struct — temperature value + type + status
- [x] `AppState` fields pour chaque type avec `Arc<RwLock<Option<T>>>`
- [x] Helper methods : `on_venus_*()`, `venus_*_get()` pour chaque type

**MQTT Handlers (`bridges/mqtt.rs` — daly-bms-server)**
- [x] Subscribe à `santuario/inverter/venus` → `handle_inverter_topic()`
- [x] Subscribe à `santuario/system/venus` → `handle_system_topic()` (SmartShunt)
- [x] Subscribe à `santuario/meteo/venus` → `handle_meteo_topic()` (MPPT aggregates)
- [x] Parse JSON payloads → Rust structs
- [x] Store dans AppState avec timestamp
- [x] Correction calcul MPPT power (champ `MpptPower` réel, pas irradiance × 0,9)

**REST API Endpoints (`api/system.rs` — daly-bms-server)**
- [x] `GET /api/v1/venus/inverter`
- [x] `GET /api/v1/venus/smartshunt`
- [x] `GET /api/v1/venus/mppt`
- [x] `GET /api/v1/venus/temperatures`
- [x] Tous les endpoints incluent `"connected": true/false`

**ET112 Integration**
- [x] `"connected": true` dans `GET /api/v1/et112/{addr}/status`
- [x] `"connected": true` dans `GET /api/v1/et112` list

**Compilation**
- [x] `cargo build --release -p daly-bms-server` — 0 warnings, 0 errors
- [x] Warning `unused mut` retiré de `et112.rs`

**Git**
- [x] Tous les changements commités sur `claude/realtime-metrics-dashboard-lUKF3`
- [x] Branche à jour avec origin

### Topics MQTT publiés par l'ancienne architecture (flows Node-RED → Rust)

| Topic | Source originelle | Payload |
|-------|---|---|
| `santuario/inverter/venus` | `inverter.json` (energy-manager flow) | `{Voltage, Current, Power, AcVoltage, AcCurrent, AcPower, State, Mode}` |
| `santuario/system/venus` | `smartshunt.json` (energy-manager flow) | `{Voltage, Current, Power, SOC, State}` |
| `santuario/meteo/venus` | `Solar_power.json` + `meteo.json` | `{MpptPower, TodaysYield, IrradianceWm2, Irradiance}` |

Ces topics sont maintenant publiés par le binaire Rust `energy-manager` (modules `inverter.rs`, `smartshunt.rs`, `meteo.rs`/`solar_power.rs`).

### Tests de validation (référence historique)

Ces tests restent valides pour vérifier la chaîne complète :

```bash
# Test endpoint inverter
curl http://192.168.1.141:8080/api/v1/venus/inverter | jq '.'
# Attendu : "connected": true avec valeurs non-null

# Test endpoint SmartShunt
curl http://192.168.1.141:8080/api/v1/venus/smartshunt | jq '.'
# Attendu : "connected": true avec voltage/current/SOC

# Test endpoint MPPT
curl http://192.168.1.141:8080/api/v1/venus/mppt | jq '.'
# Attendu : count >= 1 avec power_w > 0

# Test endpoint ET112
curl http://192.168.1.141:8080/api/v1/et112/7/status | jq '.'
# Attendu : "connected": true avec power_w

# Vérifier topics MQTT
mosquitto_sub -h 192.168.1.141 -t 'santuario/inverter/venus' -C 1 | jq '.'
mosquitto_sub -h 192.168.1.141 -t 'santuario/system/venus' -C 1 | jq '.'
mosquitto_sub -h 192.168.1.141 -t 'santuario/meteo/venus' -C 1 | jq '.'

# Vérifier logs energy-manager (anciennement via Docker)
# Désormais via : journalctl -u energy-manager -f
```

### Matériel requis (référence validation)

| Appareil | Service D-Bus | Requis pour |
|----------|---|---|
| MultiPlus (Victron AC/DC inverter) | `com.victronenergy.system` | Données onduleur |
| SmartShunt (Victron battery monitor) | `com.victronenergy.system` | SOC/courant batterie |
| MPPT 273 (SolarCharger) | `com.victronenergy.solarcharger.ttyUSB*` | Puissance solaire MPPT1 |
| MPPT 289 (SolarCharger) | `com.victronenergy.solarcharger.ttyUSB*` | Puissance solaire MPPT2 |
| ET112 Energy Counters | RS485 Modbus RTU | Métriques compteurs AC |

### Branche Git historique

```
Branche : claude/realtime-metrics-dashboard-lUKF3
Statut  : Mergée dans main (état à 2026-04-05)
Commits : 12+ commits incluant feat(state), feat(mqtt), feat(api), feat(visualization),
          fix(et112), fix(mqtt), feat(energy-manager)
```

---

## 22. Annexe historique — DASHBOARD_EXTENSION_GUIDE.md, parties energy-manager (OBSOLÈTE)

> Statut : ancienne architecture, conservée pour référence.
> Ce document décrivait les flows Node-RED (energy-manager) — remplacés par le binaire Rust.
> La référence actuelle est ce document (`docs/app-energy-manager.md`).
> Version : 2.0 — Date : 2026-04-05.

### Architecture historique (4 étapes Node-RED → Rust)

```
ÉTAPE 1 : COLLECTE (NanoPi D-Bus)
  Victron Hardware D-Bus:
    com.victronenergy.system/Dc/Voltage          → 48.2V
    com.victronenergy.system/Dc/Current          → -12.4A
    com.victronenergy.system/Ac/Out/L1/V         → 229.8V
    com.victronenergy.system/Ac/Out/L1/P         → 1286W

ÉTAPE 2 : AGGRÉGATION (energy-manager - Pi5 Docker — maintenant Rust natif)
  Flows:
    inverter.json   → subscribe D-Bus → aggregate → publish MQTT
    smartshunt.json → subscribe D-Bus → aggregate → publish MQTT
    Solar_power.json → subscribe D-Bus → aggregate → publish MQTT
  Topics générés:
    santuario/inverter/venus
    santuario/system/venus
    santuario/meteo/venus

ÉTAPE 3 : STOCKAGE (daly-bms-server)
  MQTT Handlers :
    handle_inverter_topic()  → VenusInverter struct
    handle_system_topic()    → VenusSmartShunt struct
    handle_meteo_topic()     → MPPT metrics

ÉTAPE 4 : EXPOSITION (REST API daly-bms-server)
  GET /api/v1/venus/inverter
  GET /api/v1/venus/smartshunt
  GET /api/v1/venus/mppt
  GET /api/v1/venus/temperatures

ÉTAPE 5 : AFFICHAGE (ReactFlow Dashboard)
  visualization.html — fetch + setNodes + temps réel via WebSocket (40ms) ou polling (2s)
```

### Structures Rust historiques (maintenant dans daly-bms-server/src/state.rs)

```rust
// Inverter (MultiPlus Victron)
pub struct VenusInverter {
    pub voltage_v: Option<f32>,
    pub current_a: Option<f32>,
    pub power_w: Option<f32>,
    pub ac_output_voltage_v: Option<f32>,
    pub ac_output_current_a: Option<f32>,
    pub ac_output_power_w: Option<f32>,  // ← AFFICHÉ SUR DASHBOARD
    pub state: String,
    pub mode: String,
    pub timestamp: DateTime<Utc>,
}

// SmartShunt (Victron Battery Monitor)
pub struct VenusSmartShunt {
    pub voltage_v: Option<f32>,
    pub current_a: Option<f32>,      // ← AFFICHÉ
    pub power_w: Option<f32>,
    pub soc_percent: Option<f32>,
    pub state: String,
    pub timestamp: DateTime<Utc>,
}

// MPPT Solar Charger
pub struct VenusMppt {
    pub address: String,
    pub power_w: f32,                // ← AFFICHÉ
    pub voltage_v: f32,
    pub current_a: f32,
    pub yield_today_kwh: f32,
    pub status: String,
    pub timestamp: DateTime<Utc>,
}
```

### Flows Node-RED historiques (maintenant remplacés par les modules Rust)

| Fichier Flow | Module Rust de remplacement |
|---|---|
| `flux-energy-manager/inverter.json` | `logic/inverter.rs` |
| `flux-energy-manager/smartshunt.json` | `logic/smartshunt.rs` |
| `flux-energy-manager/Solar_power.json` | `logic/solar_power.rs` |
| `flux-energy-manager/meteo.json` | `logic/meteo.rs` |

### Checklist générique d'ajout de métrique (adaptée à l'architecture Rust courante)

```
1. Identifier la source (D-Bus NanoPi / Pi5 / RS485 / API externe)
2. Ajouter la structure Rust (types.rs ou fichier dédié)
3. Ajouter un module logic/ (ou étendre un existant)
4. Brancher dans main.rs
5. Si topic MQTT → ajouter dans mqtt/topics.rs all_subscriptions()
6. Exposer via API REST :8081 si nécessaire
7. Émettre un LiveEvent si nécessaire
8. Compiler, déployer, vérifier les logs
```

### Guide de dépannage historique (applicable partiellement à l'architecture Rust)

| Problème | Symptôme | Solution |
|----------|----------|---------|
| Endpoint retourne `"connected": false` | API null | Vérifier que energy-manager tourne et publie sur le topic attendu |
| Dashboard affiche `"—"` | Valeur absente | Vérifier `curl http://192.168.1.141:8080/api/v1/venus/inverter \| jq '.'` |
| Compilation échoue (struct sans Serialize) | Erreur rustc | Ajouter `#[derive(Clone, Debug, Serialize, Deserialize)]` |
| Struct sans Clone | Erreur rustc | Ajouter `Clone` dans le derive |

---

## Voir aussi

- [./ARCHITECTURE.md](./ARCHITECTURE.md) — Vue d'ensemble système + index de toute la documentation.
- [./app-daly-bms-server.md](./app-daly-bms-server.md) — Serveur principal Pi5 (RS485, API REST, AppState, ring buffer). Consommateur des topics MQTT publiés par energy-manager.
- [./mqtt-mosquitto.md](./mqtt-mosquitto.md) — Architecture MQTT : Mosquitto natif, topics détaillés, bridge NanoPi, anti-boucle.
- [./metriques-redb-architecture.md](./metriques-redb-architecture.md) — TSDB redb : moteur de stockage, tables, encodage, write path. Energy-manager écrit via POST HTTP vers daly-bms-server.
- [./deploiement-exploitation.md](./deploiement-exploitation.md) — Déploiement Pi5 : `make build-energy-arm`, systemd, workflow complet, logs/rétention.
- [./app-dbus-mqtt-venus.md](./app-dbus-mqtt-venus.md) — Bridge NanoPi : MQTT → D-Bus Venus OS. Source des topics `N/{portal_id}/...` consommés par energy-manager.
- `CLAUDE.md` §8 (problèmes courants energy-manager), §9 règle 16 (supervision fail-fast) — conservé comme référence projet.

---

## Sources consolidées

Ce document fusionne et **remplace** les anciens fichiers suivants :
`docs/energy-manager-guide.md`, `docs/energy-manager-rules-reference.md`, `IMPLEMENTATION_VERIFICATION.md`, et les parties energy-manager de `DASHBOARD_EXTENSION_GUIDE.md`.

`CLAUDE.md` n'est **pas remplacé** — il est conservé comme mémoire projet transverse. Voir aussi `CLAUDE.md`.
