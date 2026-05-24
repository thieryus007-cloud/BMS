# Energy-Manager — Référence des Rules (corrigée)

> Document exhaustif des 7 règles GRL et 5 modules logiques.
> ⚠️ **Version corrigée** : INVERTER rule engine est dead code, SWITCH_ATS ne lit jamais MQTT, certains topics hardcodés.

---

## Table des matières

1. [Architecture](#1-architecture-globale)
2. [Infrastructure](#2-infrastructure-partagée)
3. [Modules avec GRL](#3-modules-logiques-avec-règles-grl) (INVERTER, CHARGE_CURRENT, DEYE_COMMAND, WATER_HEATER, SOLAR_POWER, SMARTSHUNT, IRRADIANCE)
4. [Modules sans GRL](#4-modules-logiques-sans-règles-grl)
5. [Sources externes](#5-sources-de-données-externes)
6. [Persistance](#6-persistance-et-restauration-au-démarrage)
7. [API](#7-api-http-et-websocket)
8. [Configuration](#8-référence-de-configuration)
9. [Diagrammes](#9-diagrammes-de-flux)

---

## 1. Architecture globale

```
MQTT Broker (192.168.1.120:1883) ◄──► energy-manager (Pi5:8081)
                                      │
        ┌──────────────────────────────┼──────────────────────┐
        │                              │                      │
    12 logic tasks            EnergyState (Arc<RwLock>)   WebSocket live
    (tokio::spawn)         (shared state for all)        (/live)
    • INVERTER
    • CHARGE_CURRENT
    • DEYE_COMMAND
    • WATER_HEATER
    • SOLAR_POWER
    • SMARTSHUNT
    • IRRADIANCE
    • TASMOTA
    • SWITCH_ATS
    • PLATFORM
    • METEO
    • VICTRON_KEEPALIVE

External pollers:
    • LG ThinQ API (600s)
    • Open-Meteo (300s)
    • daly-bms-server HTTP (various intervals)
```

---

## 2. Infrastructure partagée

### AppBus
Central message broker avec 3 canaux :
- `mqtt_in` : broadcast MQTT → modules
- `mqtt_out` : MPSC modules → MQTT broker
- `live` : broadcast modules → WebSocket clients

### Topics MQTT souscrits (dynamiques + hardcodés)

**Dynamiques** (construits avec portal_id + instances Victron) :
```
N/{pid}/vebus/{vb}/#  (IgnoreAcIn1, Ac/State/IgnoreAcIn1, freq, connected, V, I, P, State)
N/{pid}/battery/{shunt}/#  (voltage, current, power, soc, state, History/ChargedEnergy, DischargedEnergy)
N/{pid}/solarcharger/{mppt1,mppt2}/#  (power, yield, state, pv voltage, current)
N/{pid}/pvinverter/{pv}/#  (power, energy forward)
N/{pid}/system/0/#  (battery aggregates, PV power, consumption)
```

**Hardcodés** :
```
santuario/irradiance/raw  (⚠️ souscrit mais NO module ne le traite - dead code)
santuario/persist/pvinv_baseline
santuario/persist/yield_yesterday
shellypro2pm-ec62608840a4/events/rpc  (Shelly DEYE, hardcodé)
stat/tongou_3BC764/POWER  (Tasmota, hardcodé)
tele/tongou_3BC764/SENSOR  (Tasmota, hardcodé)
```

**ABSENT** : `santuario/switch/+/venus` (contrairement au document original)

### EnergyState — Champs clés

| Groupe | Champs |
|--------|--------|
| Solaire | `mppt_power_273_w`, `mppt_power_289_w`, `pvinverter_power_w`, `solar_total_w`, `house_power_w` |
| Batterie | `soc_pct`, `battery_voltage_v`, `battery_current_a`, `battery_power_w`, `battery_state`, `time_to_go_sec` |
| AC/Grid | `ac_ignore` (0=réseau, 1=hors-réseau), `ac_frequency_hz` |
| VEBus | `dc_voltage_v`, `dc_current_a`, `dc_power_w`, `ac_out_voltage_v`, `ac_out_current_a`, `ac_out_power_w` |
| Chauffe-eau | `water_heater_mode`, `water_heater_temp_c`, `water_heater_target_c`, `water_heater_last_read`, `water_heater_last_change`, `water_heater_send_count` |
| DEYE | `deye_on`, `deye_last_change`, `deye_lockout_until` |
| Compteurs | `total_yield_today_kwh`, `pvinv_baseline_kwh`, `yield_yesterday_kwh`, `ah_charged_today`, `ah_discharged_today`, `shunt_charged_today_kwh`, `shunt_discharged_today_kwh` |

---

## 3. Modules logiques avec règles GRL

### 3.1 INVERTER ⚠️ DEAD CODE

**Fichiers** : `logic/inverter/mod.rs` + `logic/inverter/rules.rs` (inutilisé)

**Status** : La règle GRL `INV_AC_Power_Ready` est définie dans `rules.rs` mais **jamais appelée** — `mod rules;` manque dans `mod.rs`. Le rule engine est dead code.

**Rôle** : Lire et publier les mesures VEBus.

**Topics en entrée** : N/{pid}/vebus/{vb}/* (voltage DC/AC, courant, puissance, fréquence, État, IgnoreAcIn1)

**Publication MQTT retained** (`santuario/inverter/venus`) :
```json
{
  "Voltage":     <dc_voltage_v>,
  "Current":     <dc_current_a>,
  "Power":       <dc_power_w>,
  "AcVoltage":   <ac_out_voltage_v>,
  "AcCurrent":   <ac_out_current_a>,
  "AcPower":     <ac_out_power_w>,
  "AcFrequency": <ac_frequency_hz>,
  "State":       "on",  // hardcodé
  "Mode":        "inverter",  // hardcodé
  "IgnoreAcIn":  <ac_ignore>,
  "VebusState":  <vebus_state>
}
```

Événement live WebSocket : stream `"inverter"`

---

### 3.2 CHARGE_CURRENT

**Rôle** : Ajuster le courant de charge VEBus selon l'état réseau et l'excédent PV.

**Topics entrée** :
- `N/{pid}/vebus/{vb}/Ac/State/IgnoreAcIn1` → `ac_ignore`
- `N/{pid}/system/0/Ac/PvOnOutput/L1/Power` → `mppt_power_273_w` **(champ partagé avec SOLAR_POWER)**
- `N/{pid}/system/0/Ac/ConsumptionOnOutput/L1/Power` → `house_power_w`

**Règles GRL** (charge_current.grl) :
```
CC_Offgrid       : ac_ignore==1 → mode="offgrid"
CC_Grid_PV_Excess: ac_ignore==0 && pv_excess==true → mode="grid_pv_excess"
CC_Grid_No_Excess: ac_ignore==0 && pv_excess==false → mode="grid_no_excess"

pv_excess = (pv_w - cons_w) > pv_excess_threshold_w
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

Config : `[energy_manager.charge_current]` — offgrid_max_a, grid_pv_excess_a, grid_no_excess_a, **pv_excess_threshold_w=50W**

---

### 3.3 DEYE_COMMAND

**Rôle** : Machine d'états fréquence → couper/restaurer onduleur DEYE (Shelly relay).

**Topics entrée** :
- `N/{pid}/vebus/{vb}/Ac/Out/L1/F` → fréquence AC (Hz)
- `N/{pid}/vebus/{vb}/Ac/ActiveIn/Connected` → trigger reconnexion réseau (direct, PAS via ac_ignore)

**Règles GRL** (deye_command.grl) — 11 règles :

| État | Condition | → Nouvel état | relay |
|------|-----------|---------------|-------|
| On | freq ≥ 52Hz | PendingCut | - |
| PendingCut | 15s écoulé + freq ≥ 52Hz | Lockout | OFF |
| Lockout | 120s écoulé | Off | - |
| Off | freq ≤ 50.3Hz | PendingRestore | - |
| PendingRestore | 45s écoulé + freq ≤ 50.3Hz | On | ON |
| **Toute** | grid_connected==true (reconnect) | On | ON (priorité 200) |

⚠️ **Important** : `Ac/ActiveIn/Connected` trigger **directement** le rule engine avec `grid_connected=true`. Ce n'est **PAS** une lecture de `ac_ignore` (qui vient de `IgnoreAcIn1`). Les deux sujets sont distincts.

**Commande Shelly MQTT transient** :
```
Topic: {shelly_id}/rpc  (ex: shellypro2pm-ec62608840a4/rpc)
Payload: {
  "id": 1,
  "src": "energy-manager",
  "method": "Switch.Set",
  "params": { "id": <channel>, "on": true|false }
}
```

Config : freq_high_hz=52.0, freq_low_hz=50.3, cut_delay_secs=15, reenable_delay_secs=45, lockout_secs=120

---

### 3.4 WATER_HEATER

**Rôle** : Piloter PAC LG ThinQ (HEAT_PUMP vs VACATION) selon conditions énergétiques.

**Deux tâches** : keepalive (25s) + control task (toutes les 5min)

**Entrées règle** : grid_connected (ac_ignore==0), soc_pct, irradiance_low (irradiance < 300 W/m²)

**Règles GRL** (water_heater.grl) :
```
Conditions (salience 100) :
  si grid_connected==true → want_vacation=true
  si soc_pct < 90 → want_vacation=true
  si irradiance_low==true → want_vacation=true

Décision (salience 200) :
  si want_vacation==true → target_mode="VACATION"
  sinon → target_mode="HEAT_PUMP"
```

**Logique** : HEAT_PUMP requiert **les 3 conditions simultanément** :
- Hors-réseau (ac_ignore=1)
- SOC ≥ 90%
- Irradiance ≥ 300 W/m²

**Flux de contrôle** (5 min) :
1. lg.get_state() → actual_mode, temp, target_temp
2. Vérifier MQTT data (ac_ignore, soc) — skip si absent
3. rule_engine.evaluate(...) → target_mode
4. Si target != actual ET cooldown (900s) expiré :
5. **lg.set_mode(target_mode)** ← synchrone
6. Sleep 15s (dans une tâche Tokio séparée non-bloquante)
7. **lg.set_target_temp(...)** dans la tâche séparée

⚠️ Les étapes 6-7 s'exécutent en **arrière-plan** via `tokio::spawn` — la boucle principale continue

**Sorties** :
- API LG ThinQ POST `/control` (mode + temp)
- MQTT retained `santuario/heatpump/1/venus` (keepalive)
- VictoriaMetrics metrics (toutes les 5min)
- WebSocket live "water_heater_venus"

Config : irradiance_min_wm2=300, mode_change_min_secs=900, heat_pump_target_c=60, vacation_target_c=45, temp_set_delay_secs=15

---

### 3.5 SOLAR_POWER

**Rôle** : Agréger MPPT + ET112, gérer baseline journalière, publier vers daly-bms.

**Deux tâches** : mqtt_task (écoute + baseline) + writer_task (POST 1s)

**Topics entrée** : MPPT 273/289 power/yield/state + ET112 energy forward + consumption

**Règles GRL** (solar_power.grl) :
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
- HTTP POST (1s) → `{bms_server}/api/v1/solar/mppt-yield`
- WebSocket live "solar"

---

### 3.6 SMARTSHUNT

**Rôle** : Monitorer batterie + intégration Ah + compteurs kWh natifs.

**Topics entrée** : `battery/{shunt}/Dc/0/*` + `History/ChargedEnergy` + fallback `system/0/Dc/Battery/*`

⚠️ **Fallback** : le code **n'a pas de priorité explicite** — accepte toute source (shunt primaire OU fallback système). "Dernier écrit gagne". Document original était trompeur en parlant de fallback.

**Règles GRL** (smartshunt.grl) :
```
Capture baseline si : new_day==true OU baseline_absent==true
```

**Intégration Ah** (time-based, sans règle GRL) :
```
Si delta_ms ∈ [1ms, 600_000ms] :
  ah_charged    += current_a * delta_h
  ah_discharged += |current_a| * delta_h
```

**Sortie MQTT retained** `santuario/system/venus` + WebSocket "battery"

---

### 3.7 IRRADIANCE

**Rôle** : Récupérer irradiance du capteur RS485 PRALRAN via daly-bms-server.

**Source** : HTTP GET `{bms_server}/api/v1/irradiance/status` (30s)

**Règle GRL** (irradiance.grl) :
```
IR_Valid_Range : raw ∈ [0, 2000] W/m² → valid=true
```

**Comportement** :
- `irradiance_wm2` toujours mis à jour dans EnergyState (même hors plage)
- Live WebSocket "irradiance" émis si valid==true

⚠️ **Topic MQTT `santuario/irradiance/raw`** est souscrit (topics.rs) mais **aucun module ne le traite** — dead code.

---

## 4. Modules logiques sans règles GRL

### 4.1 METEO

**Rôle** : Pivot météo + reset minuit.

**Tâches** :
- Publish (60s) → `santuario/heat/1/venus` + `santuario/meteo/venus`
- Midnight reset (5s après minuit) → yield_yesterday, clear baselines

---

### 4.2 TASMOTA

**Topics entrée** : `stat/tongou_3BC764/POWER`, `tele/tongou_3BC764/SENSOR`

**Sorties WebSocket** : "tasmota_wh" (relay state), "tasmota_wh_energy" (metrics)

---

### 4.3 SWITCH_ATS ⚠️ DEAD CODE

**Status** : Module **ne lit JAMAIS** le MQTT. Publie uniquement des valeurs par défaut.

**Rôle réel** : Keepalive uniquement. Pas de suivi d'état.

**Tâche** : toutes les 60s, publie MQTT retained `santuario/switch/1/venus` :
```json
{"Position": 0, "State": 0}
```

Les valeurs `ats_position` et `ats_state` restent à leur défaut (0) dans EnergyState. Aucune source MQTT ne les met à jour. La fonction `set_position()` est marquée `#[allow(dead_code)]` — jamais appelée.

⚠️ Document original était totalement erroné : ne dit pas "suivi ATS" — c'est une publication vide.

---

### 4.4 PLATFORM

**Publish (60s)** → `santuario/platform/venus` avec Status=0 (idle) uniquement.

---

### 4.5 VICTRON_KEEPALIVE

**Publish (30s)** → `R/{pid}/keepalive` (payload vide, transient)

---

## 5. Sources de données externes

### LG ThinQ

**Config** : `[energy_manager.lg_thinq]` — enabled, base_url, device_id, bearer_token, api_key

**Polling** : 600s (10 min)

**Endpoints** :
- GET `/devices/{device_id}/state`
- POST `/devices/{device_id}/control`

---

### Open-Meteo

**Config** : `[energy_manager.open_meteo]` — enabled, latitude, longitude, poll_interval_secs (300s déf)

**Données** : température_c, humidity_pct, pressure_hpa, wind_speed_ms

---

## 6. Persistance et restauration au démarrage

**Topics** :
- `santuario/persist/pvinv_baseline` = `"{day}:{kwh:.3}"`
- `santuario/persist/yield_yesterday` = `"{kwh:.3}"`

**Restauration** : spawn_persist_watcher écoute MQTT retained → charge baselines si valides (même jour)

**SmartShunt** : baselines en mémoire seulement (recalculées chaque jour)

---

## 7. API HTTP et WebSocket

**Serveur** : Axum (défaut `0.0.0.0:8081`)

| Endpoint | Méthode | Description |
|----------|---------|-------------|
| `/live` | WS | Stream LiveEvent |
| `/health` | GET | "energy-manager ok" |
| `/api/water-heater` | GET | État PAC (JSON) |
| `/api/water-heater/mode` | POST | Set mode (JSON body) |
| `/api/rules-status` | GET | Snapshot règles (JSON) |

**Payload `/api/rules-status`** :
```json
{
  "water_heater": {
    "mode": "VACATION|HEAT_PUMP|TURBO",
    "current_temp_c": null|f64,
    "target_temp_c": null|f64,
    "last_read_ts": null|ISO8601,
    "last_change_ts": null|ISO8601,
    "send_count": u32,
    "lg_enabled": bool
  },
  "charge_current": {
    "current_a": null|f64,
    "power_assist": null|i64,
    "last_ts": null|ISO8601
  },
  "deye": {
    "on": bool,
    "last_change": null|ISO8601
  },
  "soc_pct": null|f64,
  "irradiance_wm2": null|f64,
  "ac_ignore": null|0|1
}
```

**POST `/api/water-heater/mode`** : `{"mode": "HEAT_PUMP"|"VACATION"|"TURBO"}`

**WebSocket streams** : inverter, battery, solar, weather, irradiance, tasmota_wh, tasmota_wh_energy, water_heater_venus

---

## 8. Référence de configuration

**`[energy_manager.mqtt]`** :
- host (déf. 192.168.1.141)
- port (déf. 1883)
- client_id (auto-généré UUID)
- **username, password** (optionnels)
- keep_alive_secs (60)
- reconnect_delay_secs (5)

**`[energy_manager.victron]`** :
- portal_id (**obligatoire**)
- vebus_instance (275), mppt1 (273), mppt2 (289), pvinverter (32), smartshunt (274)
- shelly_deye_id, shelly_deye_channel
- tasmota_waterheater_id

**`[energy_manager.charge_current]`** :
- offgrid_max_a (70), grid_pv_excess_a (4), grid_no_excess_a (0)
- **pv_excess_threshold_w (50)**

**`[energy_manager.deye]`** :
- freq_high_hz (52.0), freq_low_hz (50.3)
- cut_delay_secs (15), reenable_delay_secs (45), lockout_secs (120)

**`[energy_manager.water_heater]`** :
- irradiance_min_wm2 (300)
- mode_change_min_secs (900)
- heat_pump_target_c (60), vacation_target_c (45)
- temp_set_delay_secs (15)
- keepalive_secs (25)
- vm_url (VictoriaMetrics)

**`[energy_manager.open_meteo]`** :
- enabled (true)
- latitude (43.9025), longitude (7.8364)
- poll_interval_secs (300)

**`[energy_manager.lg_thinq]`** :
- enabled (false)
- base_url, device_id, bearer_token, api_key
- poll_interval_secs (600)

**`[energy_manager.solar]`** :
- bms_server_url (http://192.168.1.141:8080)

**`[energy_manager.platform]`** :
- publish_interval_secs (60)

---

## 9. Diagrammes de flux

### 9.1 DEYE_COMMAND — Machine d'états

```
         Grid reconnect (salience 200, override tout)
         ├─ any state + grid==true → On + relay_on
         │
         ▼
    ┌────────┐
    │   On   │ ◄──────────── freq drops < 52Hz (annule)
    └──┬─────┘
       │ freq ≥ 52Hz
       ▼
    ┌─────────────┐
    │ PendingCut  │ ◄──────── freq drops < 52Hz (annule)
    │   (timer)   │
    └──┬──────────┘
       │ 15s + freq ≥ 52Hz
       ▼
    ┌─────────────┐   lockout_secs=120
    │  Lockout    ├─────────────►  Off  ◄──┐
    │   (120s)    │    (expire)           freq≤50.3Hz
    └─────────────┘                        │
                                      ┌─────────────┐
                                      │PendingRestore│
                                      │   (timer)    │
                                      └──┬──────────┘
                                         │ 45s + freq≤50.3Hz
                                         ▼ relay_on
                                        On
```

### 9.2 CHARGE_CURRENT — Arbre

```
ac_ignore == 1 ?
├─ OUI → offgrid (70A, assist=1)
└─ NON → (pv_w - cons_w) > 50W ?
         ├─ OUI → grid_pv_excess (4A, assist=0, feed_in=0)
         └─ NON → grid_no_excess (0A, assist=0, feed_in=0)
```

### 9.3 WATER_HEATER — Conditions

```
HEAT_PUMP exige les 3 SIMULTANÉMENT :
✓ grid_connected == false (ac_ignore=1)
✓ soc_pct ≥ 90
✓ irradiance_wm2 ≥ 300 W/m²

Sinon → VACATION
```

---

*Corrigé 2026-05-06*
*Erreurs identifiées : INVERTER rule engine=dead code, SWITCH_ATS ne lit jamais MQTT, Shelly topic format, DEYE topic ac_ignore confusion, water_heater async spawn, /api/rules-status payload, topics subscription list*
