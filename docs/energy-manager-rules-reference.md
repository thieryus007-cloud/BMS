# Energy-Manager — Référence exhaustive des Rules

> Document généré depuis le code source de `crates/energy-manager/src/`.
> Toutes les règles GRL, leurs entrées, seuils, sorties et interactions sont détaillées.

---

## Table des matières

1. [Architecture globale](#1-architecture-globale)
2. [Infrastructure partagée](#2-infrastructure-partagée)
   - AppBus · EnergyState · MQTT · Persist
3. [Modules logiques avec règles GRL](#3-modules-logiques-avec-règles-grl)
   - [3.1 INVERTER](#31-inverter)
   - [3.2 CHARGE_CURRENT](#32-charge_current)
   - [3.3 DEYE_COMMAND](#33-deye_command)
   - [3.4 WATER_HEATER](#34-water_heater)
   - [3.5 SOLAR_POWER](#35-solar_power)
   - [3.6 SMARTSHUNT](#36-smartshunt)
   - [3.7 IRRADIANCE](#37-irradiance)
4. [Modules logiques sans règles GRL](#4-modules-logiques-sans-règles-grl)
   - [4.1 METEO](#41-meteo)
   - [4.2 TASMOTA](#42-tasmota)
   - [4.3 SWITCH_ATS](#43-switch_ats)
   - [4.4 PLATFORM](#44-platform)
   - [4.5 VICTRON_KEEPALIVE](#45-victron_keepalive)
5. [Sources de données externes](#5-sources-de-données-externes)
   - [5.1 LG ThinQ](#51-lg-thinq)
   - [5.2 Open-Meteo](#52-open-meteo)
6. [Persistance et restauration au démarrage](#6-persistance-et-restauration-au-démarrage)
7. [API HTTP et WebSocket](#7-api-http-et-websocket)
8. [Référence de configuration](#8-référence-de-configuration)
9. [Diagrammes de flux](#9-diagrammes-de-flux)
   - [9.1 Vue d'ensemble des interactions](#91-vue-densemble-des-interactions)
   - [9.2 DEYE_COMMAND — machine d'états](#92-deye_command--machine-détats)
   - [9.3 CHARGE_CURRENT — arbre de décision](#93-charge_current--arbre-de-décision)
   - [9.4 WATER_HEATER — arbre de décision](#94-water_heater--arbre-de-décision)
   - [9.5 Cycle de vie quotidien des baselines](#95-cycle-de-vie-quotidien-des-baselines)

---

## 1. Architecture globale

```
                    ┌─────────────────────────────────────────┐
                    │           energy-manager (Pi5)          │
                    │                                         │
  MQTT Broker ◄────►│  mqtt::client         AppBus            │
  192.168.1.120     │  (rumqttc)      ┌────────────────────┐  │
                    │                │  mqtt_in (broadcast)│  │
  Venus OS ──────►  │                │  mqtt_out (mpsc)    │  │
  N/{pid}/...       │                │  live    (broadcast)│  │
                    │                └────────────────────┘  │
                    │                         │               │
                    │          ┌──────────────┼──────────────┐│
                    │          ▼              ▼              ▼▼│
                    │  logic tasks(12)    EnergyState    live_ws│
                    │  (tokio::spawn)  Arc<RwLock<>>    WebSocket│
                    │                                         │
  LG ThinQ API ◄───►│  http_clients/lg_thinq                  │
  Open-Meteo API ◄──►│  http_clients/open_meteo               │
  daly-bms-server ◄─►│  solar_power writer + irradiance        │
                    └─────────────────────────────────────────┘
```

**Démarrage séquentiel** (`main.rs`) :

1. Chargement config (`/etc/daly-bms/config.toml` ou `./Config.toml`)
2. Création `EnergyState::default()` (Arc<RwLock>)
3. Création `AppBus` (canaux broadcast/mpsc)
4. Spawn client MQTT + abonnements
5. Spawn `persist_watcher` (restauration baselines retained)
6. Spawn `LgThinqClient` poller (si activé)
7. Spawn `OpenMeteo` poller (si activé)
8. Spawn 12 modules logiques
9. Spawn serveur HTTP + WebSocket (`:8081`)

---

## 2. Infrastructure partagée

### AppBus (`bus.rs`)

Canal central de communication entre tous les modules :

| Canal | Type | Direction | Usage |
|-------|------|-----------|-------|
| `mqtt_in` | `broadcast::Sender<MqttIncoming>` | MQTT→modules | Chaque module s'abonne et filtre ses topics |
| `mqtt_out` | `mpsc::Sender<MqttOutgoing>` | modules→MQTT | Publication vers le broker |
| `live` | `broadcast::Sender<LiveEvent>` | modules→WebSocket | Événements temps réel pour clients WS |

### EnergyState (`types.rs`)

Structure partagée en lecture/écriture (Arc<RwLock>) contenant l'état courant :

| Groupe | Champs clés |
|--------|------------|
| Solaire / PV | `mppt_power_273_w`, `mppt_power_289_w`, `pvinverter_power_w`, `solar_total_w`, `house_power_w` |
| Détail MPPT | `mppt_273`, `mppt_289` (MpptState: power_w, pv_voltage_v, dc_current_a, yield_today_kwh, state) |
| Batterie | `soc_pct`, `battery_current_a`, `battery_voltage_v`, `battery_power_w`, `battery_state`, `time_to_go_sec` |
| Grid / AC | `ac_ignore` (0=réseau, 1=hors-réseau), `ac_connected`, `ac_frequency_hz` |
| VEBus | `dc_voltage_v`, `dc_current_a`, `dc_power_w`, `ac_out_voltage_v`, `ac_out_current_a`, `ac_out_power_w`, `vebus_state` |
| Chauffe-eau | `water_heater_mode`, `water_heater_temp_c`, `water_heater_target_c`, `water_heater_last_change`, `water_heater_send_count` |
| DEYE | `deye_on`, `deye_last_change`, `deye_lockout_until` |
| Irradiance | `irradiance_wm2` |
| Météo | `temperature_c`, `humidity_pct`, `pressure_hpa`, `wind_speed_ms` |
| Compteurs solaires | `mppt_yield_today_kwh`, `pvinv_yield_today_kwh`, `pvinv_baseline_kwh`, `pvinv_baseline_day`, `total_yield_today_kwh`, `yield_yesterday_kwh` |
| Tasmota | `tasmota_wh_on`, `tasmota_wh_power_w`, `tasmota_wh_energy_today_kwh` |
| ATS | `ats_position` (0=réseau, 1=génératrice), `ats_state` |
| Platform | `platform_backup_status` |
| Charge | `last_charge_current_a`, `last_power_assist`, `last_charge_ts` |
| Ah SmartShunt | `ah_charged_today`, `ah_discharged_today`, `ah_last_ts`, `ah_last_day` |
| kWh SmartShunt | `shunt_charged_today_kwh`, `shunt_discharged_today_kwh`, `shunt_charged_baseline_kwh`, `shunt_discharged_baseline_kwh`, `shunt_charged_day`, `shunt_discharged_day` |

### Topics MQTT souscrits (`mqtt/topics.rs`)

Tous les abonnements sont calculés dynamiquement à partir du `portal_id` et des instances Victron :

```
N/{pid}/vebus/{vebus}/#
N/{pid}/solarcharger/{mppt1}/#
N/{pid}/solarcharger/{mppt2}/#
N/{pid}/pvinverter/{pvinv}/#
N/{pid}/battery/{shunt}/#
N/{pid}/system/0/#
stat/{tasmota_id}/POWER
tele/{tasmota_id}/SENSOR
santuario/switch/+/venus
santuario/persist/+
```

### MqttOutgoing — types de publication

| Méthode | retain | Usage |
|---------|--------|-------|
| `MqttOutgoing::retained(topic, payload)` | `true` | Topics persistants Venus OS / persist |
| `MqttOutgoing::transient(topic, payload)` | `false` | Commandes VEBus (éphémères) |
| `MqttOutgoing::raw(topic, payload_str, retain)` | configurable | Baselines texte brut |

---

## 3. Modules logiques avec règles GRL

Le moteur de règles utilisé est **rust-rule-engine** avec des fichiers `.grl` (Grule Rule Language).
Chaque règle possède une **salience** (priorité, plus haut = évalué en premier) et peut être `no-loop` pour éviter les ré-évaluations.

---

### 3.1 INVERTER

**Fichiers** : `logic/inverter/mod.rs` + `logic/inverter/rules.rs` + `rules/inverter.grl`

**Rôle** : Agréger les mesures DC et AC du VEBus Victron et valider la disponibilité des données.

#### Topics MQTT en entrée

| Topic | Champ EnergyState | Description |
|-------|-------------------|-------------|
| `N/{pid}/vebus/{vb}/Dc/0/Voltage` | `dc_voltage_v` | Tension DC batterie côté onduleur |
| `N/{pid}/vebus/{vb}/Dc/0/Current` | `dc_current_a` | Courant DC |
| `N/{pid}/vebus/{vb}/Dc/0/Power` | `dc_power_w` | Puissance DC |
| `N/{pid}/vebus/{vb}/Ac/Out/L1/V` | `ac_out_voltage_v` | Tension AC sortie L1 |
| `N/{pid}/vebus/{vb}/Ac/Out/L1/I` | `ac_out_current_a` | Courant AC sortie L1 |
| `N/{pid}/vebus/{vb}/Ac/Out/L1/F` | `ac_frequency_hz` | Fréquence AC (aussi utilisée par DEYE) |
| `N/{pid}/vebus/{vb}/Ac/Out/L1/P` | `ac_out_power_w` | Puissance AC sortie |
| `N/{pid}/vebus/{vb}/State` | `vebus_state` | État VEBus (0=off, 2=fault, 3=bulk, 4=absorp, 5=float...) |
| `N/{pid}/vebus/{vb}/Ac/State/IgnoreAcIn1` | `ac_ignore` | **0=réseau connecté, 1=hors-réseau** |

#### Règle GRL (`rules/inverter.grl`)

```
rule "INV_AC_Power_Ready" salience 100 no-loop {
    when
        INV.ac_voltage_present == true && INV.ac_current_present == true
    then
        INV.ac_power_ready = true;
}
```

| Fait entrant | Type | Source |
|-------------|------|--------|
| `INV.ac_voltage_present` | bool | `ac_out_voltage_v.is_some()` |
| `INV.ac_current_present` | bool | `ac_out_current_a.is_some()` |

| Fait sortant | Type | Signification |
|-------------|------|---------------|
| `INV.ac_power_ready` | bool | `true` si puissance AC calculable (V × I) |

**Décision** : La puissance AC est calculable uniquement si **les deux** mesures tension et courant sont disponibles.

#### Sorties

| Sortie | Type | Topic / Destination |
|--------|------|---------------------|
| Agrégat inverter | MQTT retained | `santuario/inverter/venus` |
| Événement live | WebSocket | stream `"inverter"` |

#### Utilisation dans le programme

Ce module alimente `EnergyState` avec les mesures DC/AC et surtout `ac_ignore` qui est **la clé de basculement réseau/hors-réseau** lue par CHARGE_CURRENT, DEYE_COMMAND et WATER_HEATER.

---

### 3.2 CHARGE_CURRENT

**Fichiers** : `logic/charge_current/mod.rs` + `logic/charge_current/rules.rs` + `rules/charge_current.grl`

**Rôle** : Ajuster dynamiquement le courant de charge maximal de l'onduleur VEBus en fonction de l'état réseau et de l'excédent solaire.

#### Topics MQTT en entrée

| Topic | Champ EnergyState | Description |
|-------|-------------------|-------------|
| `N/{pid}/vebus/{vb}/Ac/State/IgnoreAcIn1` | `ac_ignore` | 0=réseau, 1=hors-réseau |
| `N/{pid}/system/0/Ac/PvOnOutput/L1/Power` | `mppt_power_273_w` | Puissance PV totale injectée sur sortie AC |
| `N/{pid}/system/0/Ac/ConsumptionOnOutput/L1/Power` | `house_power_w` | Consommation maison |

**Note** : `pv_excess` est **pré-calculé en Rust** avant injection dans le moteur de règles :
```rust
let pv_excess = (pv_w - cons_w) > cfg.pv_excess_threshold_w;
```

#### Règles GRL (`rules/charge_current.grl`)

```
rule "CC_Offgrid" salience 100 no-loop {
    when  CC.offgrid == true
    then  CC.mode = "offgrid";
}

rule "CC_Grid_PV_Excess" salience 100 no-loop {
    when  CC.offgrid == false && CC.pv_excess == true
    then  CC.mode = "grid_pv_excess";
}

rule "CC_Grid_No_Excess" salience 100 no-loop {
    when  CC.offgrid == false && CC.pv_excess == false
    then  CC.mode = "grid_no_excess";
}
```

| Fait entrant | Type | Calcul |
|-------------|------|--------|
| `CC.offgrid` | bool | `ac_ignore == 1` |
| `CC.pv_excess` | bool | `(pv_w - cons_w) > pv_excess_threshold_w` |

| Fait sortant | Type | Valeurs possibles |
|-------------|------|-------------------|
| `CC.mode` | string | `"offgrid"` · `"grid_pv_excess"` · `"grid_no_excess"` |

**Défaut si règle échoue** : `"grid_no_excess"` (courant de charge = 0 A).

#### Table de décision

| Mode | Condition | `charge_a` | `power_assist` | `feed_in` |
|------|-----------|-----------|----------------|-----------|
| `offgrid` | `ac_ignore == 1` | `offgrid_max_a` (déf. 70A) | `1` | Non envoyé |
| `grid_pv_excess` | réseau + excédent PV | `grid_pv_excess_a` (déf. 4A) | `0` | `0` |
| `grid_no_excess` | réseau + pas d'excédent | `grid_no_excess_a` (déf. 0A) | `0` | `0` |

#### Seuil clé

| Paramètre | Défaut | Config key | Signification |
|-----------|--------|------------|---------------|
| `pv_excess_threshold_w` | 50 W | `[energy_manager.charge_current]` | Minimum d'excédent PV pour déclencher le mode `grid_pv_excess` |

#### Sorties MQTT (transient — non retained)

| Topic | Payload | Signification |
|-------|---------|---------------|
| `W/{pid}/vebus/{vb}/Dc/0/MaxChargeCurrent` | `{"value": <A>}` | Courant max charge VEBus |
| `W/{pid}/vebus/{vb}/Settings/PowerAssistEnabled` | `{"value": 0\|1}` | Active/désactive le power assist |
| `W/{pid}/settings/0/Settings/CGwacs/MaxFeedInPower` | `{"value": 0}` | Désactive l'injection réseau |

**Optimisation** : Publication uniquement si `charge_a` ou `power_assist` ont changé depuis le dernier envoi (comparaison avec `last_charge_current_a` et `last_power_assist`).

#### Déclenchement

Réactif aux messages MQTT : déclenché à **chaque réception** des 3 topics surveillés.

---

### 3.3 DEYE_COMMAND

**Fichiers** : `logic/deye_command/mod.rs` + `logic/deye_command/rules.rs` + `rules/deye_command.grl`

**Rôle** : Piloter un relais Shelly Pro 2PM pour couper/restaurer l'onduleur DEYE en cas de sur-fréquence hors réseau (protection de l'installation lors de production solaire excessive en îlotage).

#### Topics MQTT en entrée

| Topic | Variable locale | Description |
|-------|-----------------|-------------|
| `N/{pid}/vebus/{vb}/Ac/Out/L1/F` | `last_freq` | Fréquence AC sortie onduleur (Hz) |
| `N/{pid}/vebus/{vb}/Ac/ActiveIn/Connected` | via `ac_ignore` | 1=réseau reconnecté |

**Condition de désactivation** : Si `vic.shelly_deye_id` est vide, le module se termine immédiatement.

**Note importante** : La logique fréquence ne s'applique qu'**en mode hors-réseau** (`ac_ignore == 1`). En mode réseau, la reconnexion déclenche un retour immédiat à l'état `On`.

#### Machine d'états interne (Rust)

```rust
enum DeyeState {
    On,
    PendingCut(DateTime<Utc>),      // timestamp entrée état
    Off,
    PendingRestore(DateTime<Utc>),  // timestamp entrée état
    Lockout(DateTime<Utc>),         // timestamp fin de lockout
}
```

#### Règles GRL (`rules/deye_command.grl`)

**Faits entrants** (pré-calculés en Rust) :

| Fait | Type | Calcul Rust |
|------|------|-------------|
| `DY.state` | string | `state_name(&deye_sm)` |
| `DY.freq_high_exceeded` | bool | `freq_hz >= cfg.freq_high_hz` |
| `DY.freq_low_reached` | bool | `freq_hz <= cfg.freq_low_hz` |
| `DY.cut_delay_elapsed` | bool | `time_in_state_secs >= cfg.cut_delay_secs` |
| `DY.reenable_delay_elapsed` | bool | `time_in_state_secs >= cfg.reenable_delay_secs` |
| `DY.lockout_expired` | bool | `now >= lockout_until` |
| `DY.grid_connected` | bool | `ac_ignore == 0` |

**Faits sortants** :

| Fait | Type | Action |
|------|------|--------|
| `DY.next_state` | string | Transition vers cet état |
| `DY.relay_on` | bool | Envoyer commande relay ON au Shelly |
| `DY.relay_off` | bool | Envoyer commande relay OFF au Shelly |

**Règles normales (salience 100)** :

| Règle | État actuel | Condition | Transition | Commande relay |
|-------|-------------|-----------|------------|----------------|
| `DY_On_HighFreq` | `On` | freq_high_exceeded | → `PendingCut` | aucune |
| `DY_PendingCut_FreqDrop` | `PendingCut` | !freq_high_exceeded | → `On` (annulation) | aucune |
| `DY_PendingCut_Elapsed` | `PendingCut` | cut_delay_elapsed && freq_high_exceeded | → `Lockout` | **relay_off** |
| `DY_Lockout_Expired` | `Lockout` | lockout_expired | → `Off` | aucune |
| `DY_Off_LowFreq` | `Off` | freq_low_reached | → `PendingRestore` | aucune |
| `DY_PendingRestore_FreqClimb` | `PendingRestore` | !freq_low_reached | → `Off` (annulation) | aucune |
| `DY_PendingRestore_Elapsed` | `PendingRestore` | reenable_delay_elapsed && freq_low_reached | → `On` | **relay_on** |

**Règles de reconnexion réseau (salience 200 — priorité haute)** :

| Règle | Condition | Transition | Commande relay |
|-------|-----------|------------|----------------|
| `DY_GridReconnect_From_Off` | grid_connected && état=Off | → `On` | **relay_on** |
| `DY_GridReconnect_From_PendingCut` | grid_connected && état=PendingCut | → `On` | **relay_on** |
| `DY_GridReconnect_From_PendingRestore` | grid_connected && état=PendingRestore | → `On` | **relay_on** |
| `DY_GridReconnect_From_Lockout` | grid_connected && état=Lockout | → `On` | **relay_on** |

**Salience 200 > 100** : La reconnexion réseau prend toujours le dessus sur les transitions normales.

#### Seuils de configuration

| Paramètre | Défaut | Description |
|-----------|--------|-------------|
| `freq_high_hz` | 52.0 Hz | Seuil de sur-fréquence → déclenchement coupure |
| `freq_low_hz` | 50.3 Hz | Seuil de retour → déclenchement restauration |
| `cut_delay_secs` | 15 s | Hystérésis avant coupure effective |
| `reenable_delay_secs` | 45 s | Hystérésis avant restauration |
| `lockout_secs` | 120 s | Durée du lockout anti-oscillation |

#### Commande Shelly MQTT (transient)

```json
Topic: shellies/{shelly_id}/rpc
Payload: {
  "id": 1,
  "src": "energy-manager",
  "method": "Switch.Set",
  "params": { "id": <channel>, "on": true|false }
}
```

#### Déclenchement

- Sur **réception de chaque message fréquence** (`Ac/Out/L1/F`)
- Sur **réception d'un message de reconnexion réseau** (`Ac/ActiveIn/Connected` = 1)
- Sur **tick d'1 seconde** (ticker périodique pour réévaluer les timeouts)

---

### 3.4 WATER_HEATER

**Fichiers** : `logic/water_heater/mod.rs` + `logic/water_heater/rules.rs` + `rules/water_heater.grl`

**Rôle** : Piloter automatiquement la pompe à chaleur LG ThinQ (chauffe-eau) en basculant entre modes `HEAT_PUMP` (rendement maximal) et `VACATION` (veille) selon les conditions énergétiques.

**Structure** : Deux tâches Tokio distinctes :
- `keepalive_task` : Publication périodique vers Venus OS
- `control_task` : Évaluation et commande LG ThinQ (toutes les 5 minutes)

#### Entrées du moteur de règles

Le module lit depuis `EnergyState` :

| Champ | Source MQTT/capteur | Usage règle |
|-------|---------------------|------------|
| `ac_ignore` | `N/{pid}/vebus/{vb}/Ac/State/IgnoreAcIn1` | `grid_connected = (ac_ignore == 0)` |
| `soc_pct` | `N/{pid}/battery/{shunt}/Soc` | SOC batterie en % |
| `irradiance_wm2` | HTTP GET daly-bms-server | W/m² irradiance solaire |

**Condition bloquante** : Si `ac_ignore_opt` ou `soc_opt` sont `None` (pas encore reçus), le tick est **ignoré** avec un avertissement.

**Irradiance absente** : Si `irradiance_wm2 == None`, `irradiance_low = true` (mode conservateur → VACATION).

#### Règles GRL (`rules/water_heater.grl`)

**Faits entrants** :

| Fait | Type | Initialisation | Source |
|------|------|----------------|--------|
| `WH.want_vacation` | bool | `false` | Mis à `true` par les règles conditions |
| `WH.grid_connected` | bool | calculé | `ac_ignore == 0` |
| `WH.soc_pct` | number | calculé | `soc_pct` |
| `WH.irradiance_low` | bool | calculé | `irradiance_wm2 < irradiance_min_wm2` |

**Règles conditions (salience 100)** — mettent `want_vacation = true` si **une seule** condition est vraie :

| Règle | Condition | Effet |
|-------|-----------|-------|
| `WH_Cond_Grid` | `grid_connected == true` | `want_vacation = true` |
| `WH_Cond_SOC_Low` | `soc_pct < 90` | `want_vacation = true` |
| `WH_Cond_Irradiance_Low` | `irradiance_low == true` | `want_vacation = true` |

**Règles de décision (salience 200)** — évaluées après les conditions :

| Règle | Condition | Sortie |
|-------|-----------|--------|
| `WH_Decide_Vacation` | `want_vacation == true` | `WH.target_mode = "VACATION"` |
| `WH_Decide_HeatPump` | `want_vacation == false` | `WH.target_mode = "HEAT_PUMP"` |

**Logique** : `HEAT_PUMP` requiert **les trois conditions simultanément** : hors-réseau (`ac_ignore=1`) + SOC ≥ 90% + irradiance ≥ 300 W/m².

**Fait sortant** :

| Fait | Type | Valeurs |
|------|------|---------|
| `WH.target_mode` | string | `"HEAT_PUMP"` · `"VACATION"` |

**Défaut si règle échoue** : `"VACATION"` (mode conservateur).

#### Flux de contrôle (toutes les 5 minutes)

```
1. lg.get_state()  →  actual_mode, current_temp_c, target_temp_c
   (si erreur → utiliser cache EnergyState)

2. Vérifier disponibilité données MQTT (ac_ignore, soc_pct)
   → si absent : skip tick

3. Calculer irradiance_low = (irradiance_wm2 < irradiance_min_wm2)
   → si irradiance_wm2 absent : irradiance_low = true

4. rule_engine.evaluate(grid_connected, soc, irradiance_low)
   → target_mode

5. Si actual_mode == target_mode → rien à faire

6. Vérifier cooldown (mode_change_min_secs = 900s)
   → si cooldown actif : skip

7. lg.set_mode(target_mode)
   → si erreur : incrémenter consecutive_fails
   → si consecutive_fails >= 3 : warning

8. Délai (temp_set_delay_secs = 15s)

9. lg.set_target_temp(heat_pump_target_c OU vacation_target_c)

10. publish_to_venus() → MQTT + live WS
```

#### Seuils de configuration

| Paramètre | Défaut | Description |
|-----------|--------|-------------|
| `irradiance_min_wm2` | 300 W/m² | Irradiance minimum pour autoriser HEAT_PUMP |
| `mode_change_min_secs` | 900 s | Anti-flapping : délai minimum entre deux changements |
| `heat_pump_target_c` | 60 °C | Température cible en mode HEAT_PUMP |
| `vacation_target_c` | 45 °C | Température cible en mode VACATION |
| `temp_set_delay_secs` | 15 s | Délai après changement de mode avant envoi de la consigne température |
| `keepalive_secs` | 25 s | Intervalle keepalive Venus OS |

#### Sorties

| Sortie | Type | Topic / Destination |
|--------|------|---------------------|
| Mode chauffe-eau | API HTTP LG ThinQ POST `/control` | Commande mode |
| Température cible | API HTTP LG ThinQ POST `/control` | Consigne température |
| Keepalive Venus | MQTT retained | `santuario/heatpump/1/venus` |
| Événement live | WebSocket | stream `"water_heater_venus"` |
| Métriques | HTTP POST VictoriaMetrics | `wh_mode{}`, `wh_current_temp_c{}`, `wh_target_temp_c{}` |

#### Publication Venus OS (MQTT retained `santuario/heatpump/1/venus`)

```json
{
  "State": 0|1|2,         // 0=Vacation, 1=HeatPump, 2=Turbo
  "Temperature": <°C>,     // température actuelle
  "TargetTemperature": <°C>,
  "Position": 0
}
```

---

### 3.5 SOLAR_POWER

**Fichiers** : `logic/solar_power/mod.rs` + `logic/solar_power/rules.rs` + `rules/solar_power.grl`

**Rôle** : Agréger la production solaire (2 MPPT + onduleur PV ET112), calculer le rendement journalier avec mécanisme de baseline, et publier vers daly-bms-server et les clients WebSocket.

**Structure** : Deux tâches :
- `mqtt_task` : Écoute MQTT, gère les baselines, met à jour EnergyState
- `writer_task` : Publie toutes les 1 seconde vers daly-bms-server + WebSocket live

#### Topics MQTT en entrée

| Topic | Champ EnergyState | Description |
|-------|-------------------|-------------|
| `N/{pid}/solarcharger/{m1}/Yield/Power` | `mppt_273.power_w` | Puissance MPPT 273 (W) |
| `N/{pid}/solarcharger/{m2}/Yield/Power` | `mppt_289.power_w` | Puissance MPPT 289 (W) |
| `N/{pid}/pvinverter/{pv}/Ac/L1/Power` | `pvinverter_power_w` | Puissance onduleur micro PV (W) |
| `N/{pid}/pvinverter/{pv}/Ac/Energy/Forward` | baseline logic | Énergie cumulée ET112 (kWh) |
| `N/{pid}/solarcharger/{m1}/History/Daily/0/Yield` | `mppt_273.yield_today_kwh` | Rendement journalier MPPT 273 |
| `N/{pid}/solarcharger/{m2}/History/Daily/0/Yield` | `mppt_289.yield_today_kwh` | Rendement journalier MPPT 289 |
| `N/{pid}/solarcharger/{m1}/State` | `mppt_273.state` | État MPPT 273 |
| `N/{pid}/solarcharger/{m2}/State` | `mppt_289.state` | État MPPT 289 |
| `N/{pid}/solarcharger/{m1}/Pv/V` | `mppt_273.pv_voltage_v` | Tension panneau MPPT 273 |
| `N/{pid}/solarcharger/{m2}/Pv/V` | `mppt_289.pv_voltage_v` | Tension panneau MPPT 289 |
| `N/{pid}/solarcharger/{m1}/Dc/0/Current` | `mppt_273.dc_current_a` | Courant DC MPPT 273 |
| `N/{pid}/solarcharger/{m2}/Dc/0/Current` | `mppt_289.dc_current_a` | Courant DC MPPT 289 |
| `N/{pid}/system/0/Ac/ConsumptionOnOutput/L1/Power` | `house_power_w` | Consommation maison |

#### Règles GRL (`rules/solar_power.grl`)

```
rule "SOLAR_Reset_On_NewDay" salience 100 no-loop {
    when  SOLAR.new_day == true
    then  SOLAR.reset = true; SOLAR.capture = true;
}

rule "SOLAR_Capture_When_Absent" salience 100 no-loop {
    when  SOLAR.baseline_absent == true
    then  SOLAR.capture = true;
}
```

| Fait entrant | Type | Calcul Rust |
|-------------|------|-------------|
| `SOLAR.new_day` | bool | `pvinv_baseline_day != today` (num_days_from_ce) |
| `SOLAR.baseline_absent` | bool | `pvinv_baseline_kwh.is_none()` |

| Fait sortant | Type | Action |
|-------------|------|--------|
| `SOLAR.reset` | bool | Effacer `pvinv_baseline_kwh = None` |
| `SOLAR.capture` | bool | Capturer `pvinv_baseline_kwh = kwh_actuel` |

**Logique de calcul du rendement journalier ET112** :
```
pvinv_yield_today_kwh = (kwh_actuel - baseline).max(0.0)
```

**Agrégations** :
```
solar_total_w         = mppt_273.power + mppt_289.power + pvinverter_power
mppt_yield_today_kwh  = mppt_273.yield + mppt_289.yield  (fourni directement par Venus OS)
total_yield_today_kwh = mppt_yield_today_kwh + pvinv_yield_today_kwh
```

#### Sorties

| Sortie | Type | Topic / Destination |
|--------|------|---------------------|
| Baseline ET112 | MQTT retained | `santuario/persist/pvinv_baseline` (format `"{day}:{kwh:.3}"`) |
| Données solaires | HTTP POST (1s) | `{bms_server_url}/api/v1/solar/mppt-yield` |
| Événement live | WebSocket | stream `"solar"` |

**Payload HTTP POST** :
```json
{
  "solar_total_w":   <W>,
  "mppt_power_w":    <W>,
  "total_yield_kwh": <kWh>,
  "house_power_w":   <W>
}
```

**Payload live WebSocket** :
```json
{
  "solar_total_w": <W>,
  "mppt_273_w":    <W>,
  "mppt_289_w":    <W>,
  "mppt_power_w":  <W>,
  "pvinv_w":       <W>,
  "house_power_w": <W>
}
```

---

### 3.6 SMARTSHUNT

**Fichiers** : `logic/smartshunt/mod.rs` + `logic/smartshunt/rules.rs` + `rules/smartshunt.grl`

**Rôle** : Monitorer la batterie via le SmartShunt Victron. Deux mécanismes parallèles : intégration Ah (temps réel) et compteurs kWh natifs (journalier avec baseline), avec fallback sur les agrégats système Venus OS.

#### Topics MQTT en entrée

**Source primaire — SmartShunt direct** :

| Topic | Champ EnergyState | Description |
|-------|-------------------|-------------|
| `N/{pid}/battery/{shunt}/Dc/0/Voltage` | `battery_voltage_v` | Tension batterie |
| `N/{pid}/battery/{shunt}/Dc/0/Current` | `battery_current_a` | Courant (+ = charge, - = décharge) |
| `N/{pid}/battery/{shunt}/Dc/0/Power` | `battery_power_w` | Puissance |
| `N/{pid}/battery/{shunt}/Soc` | `soc_pct` | SOC % |
| `N/{pid}/battery/{shunt}/TimeToGo` | `time_to_go_sec` | Temps restant (secondes) |
| `N/{pid}/battery/{shunt}/State` | `battery_state` | État batterie |
| `N/{pid}/battery/{shunt}/History/ChargedEnergy` | baseline logic | Énergie chargée cumulée (kWh) |
| `N/{pid}/battery/{shunt}/History/DischargedEnergy` | baseline logic | Énergie déchargée cumulée (kWh) |

**Source fallback — Agrégats système** (si topics shunt absents) :

| Topic | Fallback pour | Description |
|-------|---------------|-------------|
| `N/{pid}/system/0/Dc/Battery/Soc` | `soc_pct` | SOC agrégé |
| `N/{pid}/system/0/Dc/Battery/Current` | `battery_current_a` | Courant agrégé |
| `N/{pid}/system/0/Dc/Battery/State` | `battery_state` | État agrégé |
| `N/{pid}/system/0/Dc/Battery/TimeToGo` | `time_to_go_sec` | TTG agrégé |
| `N/{pid}/vebus/{vb}/Dc/0/Voltage` | `battery_voltage_v` | Tension VEBus |
| `N/{pid}/vebus/{vb}/Dc/0/Power` | `battery_power_w` | Puissance VEBus |

#### Règles GRL (`rules/smartshunt.grl`)

```
rule "SHUNT_Charged_Baseline_NewDay" salience 100 no-loop {
    when  SHUNT.charged_new_day == true
    then  SHUNT.capture_charged = true;
}
rule "SHUNT_Charged_Baseline_Missing" salience 100 no-loop {
    when  SHUNT.charged_baseline_absent == true
    then  SHUNT.capture_charged = true;
}
rule "SHUNT_Discharged_Baseline_NewDay" salience 100 no-loop {
    when  SHUNT.discharged_new_day == true
    then  SHUNT.capture_discharged = true;
}
rule "SHUNT_Discharged_Baseline_Missing" salience 100 no-loop {
    when  SHUNT.discharged_baseline_absent == true
    then  SHUNT.capture_discharged = true;
}
```

| Fait entrant | Type | Calcul Rust |
|-------------|------|-------------|
| `SHUNT.charged_new_day` | bool | `shunt_charged_day != today` |
| `SHUNT.charged_baseline_absent` | bool | `shunt_charged_baseline_kwh.is_none()` |
| `SHUNT.discharged_new_day` | bool | `shunt_discharged_day != today` |
| `SHUNT.discharged_baseline_absent` | bool | `shunt_discharged_baseline_kwh.is_none()` |

| Fait sortant | Type | Action |
|-------------|------|--------|
| `SHUNT.capture_charged` | bool | Capturer baseline chargé = kWh actuel |
| `SHUNT.capture_discharged` | bool | Capturer baseline déchargé = kWh actuel |

**Calcul rendement journalier** :
```
shunt_charged_today_kwh    = (kwh_charged    - charged_baseline).max(0.0)
shunt_discharged_today_kwh = (kwh_discharged - discharged_baseline).max(0.0)
```

#### Intégration Ah (mécanisme complémentaire, sans règle GRL)

```rust
fn integrate_ah(s, current_a, now):
    si jour changé → reset ah_charged_today, ah_discharged_today
    delta_h = (now - prev_ts).ms / 3_600_000
    si delta_ms dans [1ms, 600_000ms]:  // rejeter les gaps > 10 min
        si current_a > 0 → ah_charged_today    += current_a * delta_h
        si current_a < 0 → ah_discharged_today += (-current_a) * delta_h
```

Déclenché sur chaque réception de courant (topics `Dc/0/Current`).

#### Sortie MQTT (retained `santuario/system/venus`)

```json
{
  "Soc":                 <0-100>,
  "Voltage":             <V>,
  "Current":             <A>,
  "Power":               <W>,
  "State":               <0-6>,
  "TimeToGo":            <s>,
  "ChargedTodayKwh":     <kWh>,
  "DischargedTodayKwh":  <kWh>,
  "AhChargedToday":      <Ah>,
  "AhDischargedToday":   <Ah>
}
```

Événement live WebSocket : stream `"battery"`.

---

### 3.7 IRRADIANCE

**Fichiers** : `logic/irradiance/mod.rs` + `logic/irradiance/rules.rs` + `rules/irradiance.grl`

**Rôle** : Récupérer la mesure d'irradiance solaire depuis le capteur RS485 PRALRAN via daly-bms-server, valider la plage et mettre à jour l'état partagé.

#### Source de données

- **HTTP GET** vers `{bms_server_url}/api/v1/irradiance/status` toutes les **30 secondes**
- Réponse attendue : `{"irradiance_wm2": <float>, "connected": <bool>}`

#### Règle GRL (`rules/irradiance.grl`)

```
rule "IR_Valid_Range" salience 100 no-loop {
    when  IR.raw >= 0.0 && IR.raw <= 2000.0
    then  IR.valid = true;
}
```

| Fait entrant | Type | Source |
|-------------|------|--------|
| `IR.raw` | number | Valeur brute en W/m² |

| Fait sortant | Type | Signification |
|-------------|------|---------------|
| `IR.valid` | bool | `true` si 0 ≤ valeur ≤ 2000 W/m² |

**Comportement** :
- La valeur `irradiance_wm2` dans `EnergyState` est **toujours mise à jour** (même si hors plage)
- L'événement live WebSocket n'est émis que si `IR.valid == true`

#### Utilisation dans le programme

`irradiance_wm2` est lue par **WATER_HEATER** pour calculer `irradiance_low = (irradiance_wm2 < irradiance_min_wm2)`, ce qui influence la décision mode PAC.

---

## 4. Modules logiques sans règles GRL

---

### 4.1 METEO

**Fichiers** : `logic/meteo/mod.rs`

**Rôle** : Pivot central météo + solaire. Publie les données agrégées vers Venus OS et gère le reset de minuit.

**Structure** : Deux tâches :
- `publish_task` : toutes les 60 secondes
- `midnight_reset_task` : déclenché 5 secondes après minuit

#### Publish périodique (60s)

**Publication MQTT retained `santuario/heat/1/venus`** (capteur température Venus OS) :
```json
{
  "Temperature":     <°C>,    // de Open-Meteo
  "Humidity":        <0-100>, // de Open-Meteo
  "Pressure":        <hPa>,   // de Open-Meteo
  "TemperatureType": 4        // 4 = Extérieur (type Venus OS)
}
```

**Publication MQTT retained `santuario/meteo/venus`** (irradiance + solaire + vent) :
```json
{
  "Irradiance":     <W/m²>,
  "TodaysYield":    <kWh>,
  "YieldYesterday": <kWh>,
  "WindSpeed":      <m/s>,
  "MpptPower":      <W>,
  "SolarTotal":     <W>,
  "Mppts": [
    {
      "Instance": 273,
      "State":    <code>,
      "PvVoltage": <V>,
      "DcCurrent": <A>,
      "Power":     <W>,
      "YieldToday": <kWh>
    },
    { "Instance": 289, ... }
  ]
}
```

#### Reset de minuit

Déclenché à **minuit + 5 secondes** (heure locale) :

```
1. Lire total_yield_today_kwh
2. yield_yesterday_kwh = total_yield_today_kwh
3. Remettre à zéro : total_yield_today_kwh, mppt_yield_today_kwh, pvinv_yield_today_kwh
4. pvinv_baseline_kwh = None  (sera recapturé sur prochain message ET112)
5. mppt_273.yield_today_kwh = 0.0, mppt_289.yield_today_kwh = 0.0
6. Publier retained "santuario/persist/yield_yesterday" = "{total:.3}"
7. Publier retained "santuario/persist/pvinv_baseline" = ""  (effacement)
```

---

### 4.2 TASMOTA

**Fichiers** : `logic/tasmota/mod.rs`

**Rôle** : Lire l'état du relais chauffe-eau et sa consommation depuis un dispositif Tasmota (tongou_3BC764).

**Condition** : Si `vic.tasmota_waterheater_id` est vide, le module se termine immédiatement.

#### Topics MQTT en entrée

| Topic | Champ EnergyState | Description |
|-------|-------------------|-------------|
| `stat/{id}/POWER` | `tasmota_wh_on` | État relais (ON/OFF → bool) |
| `tele/{id}/SENSOR` | `tasmota_wh_power_w`, `tasmota_wh_energy_today_kwh` | JSON avec métriques énergie |

#### Sorties WebSocket

| Événement | Stream | Données |
|-----------|--------|---------|
| Changement état relais | `"tasmota_wh"` | `{on: bool}` |
| Métriques énergie | `"tasmota_wh_energy"` | `{power_w, voltage_v, current_a, today_kwh, total_kwh}` |

---

### 4.3 SWITCH_ATS

**Fichiers** : `logic/switch_ats/mod.rs`

**Rôle** : Suivre l'état de l'ATS (commutateur automatique réseau/groupe électrogène CHINT) et maintenir le keepalive Venus OS.

#### Topic MQTT en entrée

```
santuario/switch/1/venus  (retained, écrit par dbus-mqtt-venus sur NanoPi)
Payload: {"Position": <0|1>, "State": <0|1|2>}
```

#### Sortie keepalive MQTT (retained, toutes les 60s)

```
santuario/switch/1/venus
{"Position": <0|1>, "State": <0|1|2>}
```

| Valeur | Signification |
|--------|---------------|
| `Position=0` | Réseau EDF connecté |
| `Position=1` | Groupe électrogène actif |
| `State=0` | Inactif |
| `State=1` | Actif |
| `State=2` | Alerte |

---

### 4.4 PLATFORM

**Fichiers** : `logic/platform/mod.rs`

**Rôle** : Publier un statut mock de sauvegarde plateforme (keepalive infrastructure).

#### Sortie MQTT (retained, toutes les `publish_interval_secs` = 60s)

```
santuario/platform/venus
{
  "Backup":  { "Status": 0, "LastRun": <unix_ts> },
  "Restore": { "Status": 0, "LastRun": <unix_ts> }
}
```

| Status | Signification |
|--------|---------------|
| 0 | Idle |
| 1 | En cours |
| 2 | OK |
| 3 | Erreur |

---

### 4.5 VICTRON_KEEPALIVE

**Fichiers** : `logic/victron_keepalive/mod.rs`

**Rôle** : Empêcher Venus OS de cesser de publier sur les topics `N/...` en maintenant une activité sur le topic `R/...`.

#### Mécanisme

Venus OS arrête de publier les topics notification (`N/`) après ~60 secondes d'inactivité. La publication toutes les **30 secondes** sur `R/{portal_id}/keepalive` (payload vide) maintient le flux actif.

```
Topic: R/{portal_id}/keepalive
Payload: (vide)
Retain: false
Intervalle: 30s
```

---

## 5. Sources de données externes

---

### 5.1 LG ThinQ

**Fichiers** : `http_clients/lg_thinq.rs`

**Activation** : `lg_thinq.enabled = true` + credentials dans `.env`

**Variables d'environnement** (secrets) :
- `LG_DEVICE_ID` : Identifiant appareil
- `LG_BEARER_TOKEN` : Token d'authentification
- `LG_API_KEY` : Clé API

#### Endpoints utilisés

| Méthode | Path | Usage |
|---------|------|-------|
| GET | `/devices/{device_id}/state` | Lire mode + températures |
| POST | `/devices/{device_id}/control` | Écrire mode ou température |

#### En-têtes HTTP

```
Authorization: Bearer {bearer_token}
x-api-key: {api_key}
x-country: FR
x-client-id: energy-manager
x-message-id: (UUID généré)
```

#### Polling

- **Intervalle** : `poll_interval_secs` (défaut 600s = 10 min)
- **Résultat** : Met à jour `EnergyState` + émet live event + pousse métriques VictoriaMetrics

---

### 5.2 Open-Meteo

**Fichiers** : `http_clients/open_meteo.rs`

**Activation** : `open_meteo.enabled = true` (défaut)

**URL** :
```
https://api.open-meteo.com/v1/forecast
  ?latitude={lat}&longitude={lon}
  &current=temperature_2m,relative_humidity_2m,surface_pressure,wind_speed_10m
  &wind_speed_unit=ms
```

**Données mises à jour** :

| Champ API | Champ EnergyState | Description |
|-----------|-------------------|-------------|
| `temperature_2m` | `temperature_c` | Température extérieure |
| `relative_humidity_2m` | `humidity_pct` | Humidité relative |
| `surface_pressure` | `pressure_hpa` | Pression atmosphérique |
| `wind_speed_10m` | `wind_speed_ms` | Vitesse du vent (m/s) |

**Intervalle** : `poll_interval_secs` (défaut 300s = 5 min)
**Événement live** : stream `"weather"`

---

## 6. Persistance et restauration au démarrage

### Mécanisme général

Les compteurs qui doivent survivre aux redémarrages sont stockés en tant que **messages MQTT retained** sur le broker Mosquitto local.

### Topics de persistance

| Topic | Format | Contenu |
|-------|--------|---------|
| `santuario/persist/pvinv_baseline` | `"{day_ordinal}:{kwh:.3}"` | Baseline compteur ET112 pour le jour en cours |
| `santuario/persist/yield_yesterday` | `"{kwh:.3}"` | Production solaire d'hier |

### Restauration au démarrage (`persist/baseline.rs`)

Le `spawn_persist_watcher` dans `main.rs` écoute les messages retained dès la connexion MQTT :

**`pvinv_baseline`** :
1. Vérifier format `"{day}:{kwh}"`
2. Comparer `day` avec aujourd'hui (`num_days_from_ce`)
3. Si stale (jour ≠ aujourd'hui) : **ignorer** (évite de fausser le rendement journalier)
4. Si valide : `pvinv_baseline_kwh = Some(kwh)` et `pvinv_baseline_day = today`

**`yield_yesterday`** :
1. Parser en f64
2. `yield_yesterday_kwh = valeur`

### Baselines SmartShunt

**Non persistées en MQTT** — recalculées à chaque message `ChargedEnergy`/`DischargedEnergy` :
- Si `shunt_charged_day != today` → capturer nouvelle baseline
- Si `shunt_charged_baseline_kwh.is_none()` → capturer baseline

---

## 7. API HTTP et WebSocket

**Serveur** : Axum sur `bind` (défaut `0.0.0.0:8081`)

| Méthode | Path | Description |
|---------|------|-------------|
| `GET` | `/live` | WebSocket — flux de tous les `LiveEvent` |
| `GET` | `/health` | `"energy-manager ok"` |
| `GET` | `/api/water-heater` | État actuel chauffe-eau |
| `POST` | `/api/water-heater/mode` | Forcer mode `{"mode": "HEAT_PUMP"\|"VACATION"}` |
| `GET` | `/api/rules-status` | Snapshot état toutes les règles |

**Payload `/api/rules-status`** :
```json
{
  "water_heater": { "target_mode": "...", "actual_mode": "..." },
  "charge_current": { "mode": "...", "charge_a": ... },
  "deye": { "state": "...", "relay_on": ... },
  "soc_pct": ...,
  "irradiance_wm2": ...,
  "ac_ignore": ...
}
```

**Streams WebSocket** (`/live`) :

| Stream | Émis par | Fréquence |
|--------|----------|-----------|
| `inverter` | logic/inverter | À chaque message MQTT VEBus |
| `battery` | logic/smartshunt | À chaque mesure batterie |
| `solar` | logic/solar_power writer | Toutes les 1 seconde |
| `weather` | http_clients/open_meteo | Toutes les 5 min |
| `irradiance` | logic/irradiance | Toutes les 30s |
| `tasmota_wh` | logic/tasmota | À chaque changement état |
| `tasmota_wh_energy` | logic/tasmota | Sur réception tele/SENSOR |
| `water_heater_venus` | logic/water_heater | Sur changement de mode |

---

## 8. Référence de configuration

Toutes les valeurs lues depuis `[energy_manager]` dans `Config.toml` :

### `[energy_manager.mqtt]`

| Clé | Défaut | Description |
|-----|--------|-------------|
| `host` | `"192.168.1.141"` | Hôte MQTT broker |
| `port` | `1883` | Port MQTT |
| `client_id` | UUID généré | ID client MQTT |
| `keep_alive_secs` | `60` | Intervalle keepalive MQTT |
| `reconnect_delay_secs` | `5` | Délai entre reconnexions |

### `[energy_manager.api]`

| Clé | Défaut | Description |
|-----|--------|-------------|
| `bind` | `"0.0.0.0:8081"` | Adresse d'écoute HTTP/WS |

### `[energy_manager.victron]`

| Clé | Défaut | Description |
|-----|--------|-------------|
| `portal_id` | **obligatoire** | ID portail Victron GX |
| `vebus_instance` | `275` | Instance VEBus |
| `mppt1_instance` | `273` | Instance MPPT 1 |
| `mppt2_instance` | `289` | Instance MPPT 2 |
| `pvinverter_instance` | `32` | Instance onduleur PV ET112 |
| `shelly_deye_id` | `""` | ID Shelly Pro 2PM DEYE (vide = désactivé) |
| `shelly_deye_channel` | `0` | Canal Shelly DEYE |
| `tasmota_waterheater_id` | `""` | ID Tasmota chauffe-eau (vide = désactivé) |
| `smartshunt_instance` | `290` | Instance SmartShunt |

### `[energy_manager.charge_current]`

| Clé | Défaut | Description |
|-----|--------|-------------|
| `offgrid_max_a` | `70.0` | Courant max hors-réseau (A) |
| `grid_pv_excess_a` | `4.0` | Courant charge réseau + excédent PV (A) |
| `grid_no_excess_a` | `0.0` | Courant charge réseau sans excédent (A) |
| `pv_excess_threshold_w` | `50.0` | Seuil excédent PV (W) |

### `[energy_manager.deye]`

| Clé | Défaut | Description |
|-----|--------|-------------|
| `freq_high_hz` | `52.0` | Fréquence déclenchement coupure (Hz) |
| `freq_low_hz` | `50.3` | Fréquence déclenchement restauration (Hz) |
| `cut_delay_secs` | `15` | Délai avant coupure effective (s) |
| `reenable_delay_secs` | `45` | Délai avant restauration effective (s) |
| `lockout_secs` | `120` | Durée lockout anti-oscillation (s) |

### `[energy_manager.water_heater]`

| Clé | Défaut | Description |
|-----|--------|-------------|
| `irradiance_min_wm2` | `300.0` | Irradiance min pour HEAT_PUMP (W/m²) |
| `mode_change_min_secs` | `900` | Cooldown anti-flapping entre modes (s) |
| `heat_pump_target_c` | `60.0` | Consigne température HEAT_PUMP (°C) |
| `vacation_target_c` | `45.0` | Consigne température VACATION (°C) |
| `temp_set_delay_secs` | `15` | Délai envoi consigne après changement mode (s) |
| `keepalive_secs` | `25` | Intervalle keepalive Venus OS (s) |
| `vm_url` | `"http://127.0.0.1:8428"` | URL VictoriaMetrics |

### `[energy_manager.open_meteo]`

| Clé | Défaut | Description |
|-----|--------|-------------|
| `enabled` | `true` | Activer le polling météo |
| `latitude` | `43.9025` | Latitude GPS |
| `longitude` | `7.8364` | Longitude GPS |
| `poll_interval_secs` | `300` | Intervalle polling (s) |

### `[energy_manager.lg_thinq]`

| Clé | Défaut | Description |
|-----|--------|-------------|
| `enabled` | `false` | Activer la gestion LG ThinQ |
| `base_url` | `"https://api-eic.lgthinq.com"` | URL API LG |
| `device_id` | `""` (env `LG_DEVICE_ID`) | ID appareil |
| `bearer_token` | `""` (env `LG_BEARER_TOKEN`) | Token auth |
| `api_key` | `""` (env `LG_API_KEY`) | Clé API |
| `country` | `"FR"` | Code pays |
| `poll_interval_secs` | `600` | Intervalle polling (s) |
| `vm_url` | `"http://127.0.0.1:8428"` | URL VictoriaMetrics |

### `[energy_manager.solar]`

| Clé | Défaut | Description |
|-----|--------|-------------|
| `bms_server_url` | `"http://192.168.1.141:8080"` | URL daly-bms-server |

### `[energy_manager.platform]`

| Clé | Défaut | Description |
|-----|--------|-------------|
| `publish_interval_secs` | `60` | Intervalle publication plateforme (s) |

---

## 9. Diagrammes de flux

### 9.1 Vue d'ensemble des interactions

```
                          ╔═══════════════════════════════════════════════════╗
                          ║              FLUX DE DONNÉES GLOBAL               ║
                          ╚═══════════════════════════════════════════════════╝

Venus OS (GX)                     energy-manager                    Sorties
──────────────                    ──────────────────                ────────
N/{pid}/vebus/#     ──MQTT──►  ┌─ INVERTER ──────────────────►  santuario/inverter/venus (ret.)
                               │     │ ac_ignore (clé globale)
N/{pid}/system/0/# ──MQTT──►  │     ▼
                               ├─ CHARGE_CURRENT ─► W/{pid}/vebus/.../MaxChargeCurrent (trans.)
                               │   (ac_ignore, pv_w, cons_w)    W/{pid}/vebus/.../PowerAssist
                               │
N/{pid}/vebus/F    ──MQTT──►  ├─ DEYE_COMMAND ─────────────►  shellies/{id}/rpc (trans.)
                               │   (freq_hz, grid_connected)
                               │
N/{pid}/battery/#  ──MQTT──►  ├─ SMARTSHUNT ────────────────►  santuario/system/venus (ret.)
                               │   (V, I, P, SOC, kWh hist.)
                               │
N/{pid}/solarcharger/#──MQTT─►├─ SOLAR_POWER ───────────────►  santuario/persist/pvinv_baseline (ret.)
N/{pid}/pvinverter/#──MQTT──►  │   (MPPT1, MPPT2, ET112)        POST http://bms-server/solar
                               │
stat/{id}/POWER    ──MQTT──►  ├─ TASMOTA ────────────────────►  live WS "tasmota_wh"
tele/{id}/SENSOR   ──MQTT──►  │
                               │
santuario/switch/# ──MQTT──►  ├─ SWITCH_ATS ─────────────────►  santuario/switch/1/venus (ret.)
                               │
HTTP ◄─────────────────────── ├─ IRRADIANCE ─────────────────►  EnergyState.irradiance_wm2
  daly-bms-server              │   (poll 30s)                     live WS "irradiance"
                               │
HTTP ◄─────────────────────── ├─ Open-Meteo ─────────────────►  EnergyState.temperature_c
  api.open-meteo.com           │   (poll 5 min)                   live WS "weather"
                               │
HTTP ◄────────────────────────┤─ LG ThinQ ───────────────────►  EnergyState.water_heater_mode
  api-eic.lgthinq.com         │   (poll 10 min)                   VictoriaMetrics
                               │
                               ├─ WATER_HEATER ──────────────►  santuario/heatpump/1/venus (ret.)
                               │   (ac_ignore, soc, irrad.)       POST LG ThinQ control
                               │                                   VictoriaMetrics metrics
                               │
                               ├─ METEO (60s) ───────────────►  santuario/heat/1/venus (ret.)
                               │   (aggrège tout)                  santuario/meteo/venus (ret.)
                               │   + reset minuit
                               │
                               ├─ PLATFORM (60s) ────────────►  santuario/platform/venus (ret.)
                               │
                               └─ VICTRON_KEEPALIVE (30s) ───►  R/{pid}/keepalive (trans.)


Persistance (MQTT retained) :
  santuario/persist/pvinv_baseline  ◄──► SOLAR_POWER + persist_watcher
  santuario/persist/yield_yesterday ◄──► METEO + persist_watcher
```

---

### 9.2 DEYE_COMMAND — machine d'états

```
                    ┌──────────────────────────────────────────┐
                    │  RÈGLE PRIORITÉ 200 (grid reconnect)     │
                    │  Depuis N'IMPORTE QUEL état :            │
                    │  grid_connected == true → On + relay_on  │
                    └──────────────────────────────────────────┘
                                         │ (override)
                                         ▼
         ┌──────────────────────────────────────────────────────┐
         │                                                        │
         │  freq > 52Hz                                          │
    ┌────▼────┐   ──────────────────────────►  ┌──────────────┐ │
    │         │                                 │              │ │
    │   On    │                                 │ PendingCut   │ │
    │         │   ◄──────────────────────────── │ (timer)      │ │
    └─────────┘   freq drops < 52Hz (annule)   └──────┬───────┘ │
         ▲                                             │         │
         │                                  15s + freq > 52Hz   │
         │                                             │         │
         │         ┌──────────────┐                   ▼         │
         │  lockout│              │         ┌──────────────────┐ │
         │  expiré │   Lockout    │◄────────│    relay_off()   │ │
         │         │   (120s)     │         │  → Lockout state │ │
         │         └──────────────┘         └──────────────────┘ │
         │                                                        │
         │         ┌──────────────┐                              │
         │         │              │  freq drops ≤ 50.3Hz        │
         │         │     Off      ├───────────────────────────►  │
         │         │              │                              │
         │         └──────────────┘  ◄──────────────────────── ─┤
         │                           freq climbs > 50.3 (annule) │
         │                                                        │
         │         ┌────────────────┐                           │
         │         │ PendingRestore │                           │
         │         │ (timer)        │                           │
         │         └────────┬───────┘                           │
         │                  │ 45s + freq ≤ 50.3Hz               │
         │                  ▼                                    │
         │         relay_on() → On                              │
         └──────────────────────────────────────────────────────┘
```

---

### 9.3 CHARGE_CURRENT — arbre de décision

```
ENTRÉES MQTT : ac_ignore, pv_power_w, house_power_w

           ┌─────────────────────────────┐
           │  ac_ignore == 1 ?            │
           └─────────────────────────────┘
                     │
            OUI      │      NON
             ▼              ▼
    ┌─────────────────┐   ┌────────────────────────────────────┐
    │  mode: "offgrid" │  │  (pv_w - cons_w) > 50W ?           │
    │  charge_a = 70A  │  └────────────────────────────────────┘
    │  power_assist=1  │            │
    │  (no feed-in cmd)│   OUI      │      NON
    └─────────────────┘    ▼              ▼
                   ┌────────────────┐  ┌───────────────────┐
                   │ "grid_pv_excess"│  │  "grid_no_excess" │
                   │  charge_a = 4A  │  │  charge_a = 0A    │
                   │  power_assist=0 │  │  power_assist=0   │
                   │  feed_in = 0    │  │  feed_in = 0      │
                   └────────────────┘  └───────────────────┘
                            │                    │
                            └────────┬───────────┘
                                     ▼
                      Si changement depuis dernier envoi :
                      ► W/{pid}/vebus/.../MaxChargeCurrent
                      ► W/{pid}/vebus/.../PowerAssistEnabled
                      ► W/{pid}/settings/.../MaxFeedInPower (si réseau)
```

---

### 9.4 WATER_HEATER — arbre de décision

```
ENTRÉES : ac_ignore, soc_pct, irradiance_wm2, irradiance_min_wm2 (300 W/m²)

  ┌──────────────────────────────────────────────────┐
  │  CONDITION 1 : grid_connected = (ac_ignore == 0) │
  │         → si true : want_vacation = true         │
  └──────────────────────────────────────────────────┘
                         │
  ┌──────────────────────────────────────────────────┐
  │  CONDITION 2 : soc_pct < 90 ?                    │
  │         → si true : want_vacation = true         │
  └──────────────────────────────────────────────────┘
                         │
  ┌──────────────────────────────────────────────────┐
  │  CONDITION 3 : irradiance_wm2 < 300 W/m² ?      │
  │      (ou irradiance absente → traité comme low)  │
  │         → si true : want_vacation = true         │
  └──────────────────────────────────────────────────┘
                         │
                    want_vacation ?
                    ┌─────┴─────┐
                   OUI         NON
                    ▼           ▼
              VACATION      HEAT_PUMP
              target=45°C   target=60°C

  ─────────────────────────────────────────────────────
  HEAT_PUMP exige les 3 conditions SIMULTANÉMENT :
  ✓ Hors-réseau (ac_ignore=1)
  ✓ SOC ≥ 90%
  ✓ Irradiance ≥ 300 W/m²
  ─────────────────────────────────────────────────────

  Puis vérification cooldown (900s depuis dernier changement)
  Puis lg.set_mode() + attendre 15s + lg.set_target_temp()
```

---

### 9.5 Cycle de vie quotidien des baselines

```
               JOUR J                            JOUR J+1
  ─────────────────────────────────────────────────────────────────►
  
  Démarrage      Premier msg ET112           Minuit+5s
      │                │                         │
      ▼                ▼                         ▼
  ┌────────┐    ┌─────────────────┐    ┌──────────────────────┐
  │ Cherche│    │ baseline absent │    │ METEO midnight_reset: │
  │retained│    │ → SOLAR.capture │    │ yield_yesterday = X   │
  │MQTT    │    │ baseline = X kWh│    │ baseline = None       │
  │baseline│    │ day = today     │    │ Pub retained:         │
  └──┬─────┘    │ Pub retained    │    │  yield_yesterday=X    │
     │          └────────┬────────┘    │  pvinv_baseline=""    │
     │                   │             └──────────┬────────────┘
     │ retained reçu ?   │                        │
     ├── OUI (même jour)─┤                        │
     │   baseline OK     │                Prochain msg ET112
     │                   │                        │
     ├── NON (vieux jour)│                        ▼
     │   ignorer         │             ┌──────────────────────┐
     │                   │             │ SOLAR_Reset_On_NewDay│
     └───────────────────┘             │ reset + capture      │
                                       │ nouvelle baseline=Y  │
             En cours de journée :     └──────────────────────┘
             ─────────────────────
             Sur chaque msg ET112 :
               pvinv_yield = (current - baseline).max(0)
               
             SmartShunt ChargedEnergy/DischargedEnergy :
               Même logique, baselines en mémoire seulement
               (pas de persistance MQTT)
```

---

*Document généré le 2026-05-06 — basé sur `crates/energy-manager/src/` commit courant.*
