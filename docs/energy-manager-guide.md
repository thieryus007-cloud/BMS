# Guide Développeur — energy-manager

Référence complète pour modifier, ajouter ou supprimer une fonctionnalité du crate `energy-manager`.

---

## 1. Vue d'ensemble

`energy-manager` est un binaire Rust autonome qui remplace les flows energy-manager. Il tourne en service systemd sur le Pi5 (`energy-manager.service`), écoute le broker MQTT Mosquitto, applique la logique métier, et publie sur MQT et WebSocket.

### Flux de données

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

### AppBus — le bus central

`bus.rs` expose 4 canaux :

| Canal | Type | Usage |
|-------|------|-------|
| `mqtt_in` | `broadcast::Sender<MqttIncoming>` | Tous les messages MQTT entrants → tous les modules |
| `mqtt_out` | `mpsc::Sender<MqttOutgoing>` | Publish MQTT depuis n'importe quel module |
| `live` | `broadcast::Sender<LiveEvent>` | Événements WebSocket live |
| `rule_reload` | `broadcast::Sender<String>` | Signal hot-reload → tous les modules de règles |

Cloner `AppBus` est gratuit (tous les champs sont `Arc`-backed).

### État partagé

`Arc<RwLock<EnergyState>>` dans `types.rs` — struct avec tous les champs mesurés (solaire, batterie, onduleur, météo, chauffe-eau, DEYE, etc.). Les modules écrivent via `.write().await`, lisent via `.read().await`.

---

## 2. Modules logiques — inventaire

| Fichier | Rôle | Entrées MQTT | Sorties |
|---------|------|--------------|---------|
| `solar_power.rs` | Puissance solaire temps réel | MPPT power/yield, PVInverter power/energy |  `solar_power` (1/s), POST daly-bms, LiveEvent `solar` |
| `meteo.rs` | Publication météo Venus + reset minuit | état partagé | MQTT `santuario/meteo/venus`, `santuario/heat/1/venus`,  `solar_persist` (1/jour) |
| `inverter.rs` | Données onduleur VEBus | N/.../vebus/... | EnergyState, LiveEvent `inverter` |
| `smartshunt.rs` | Données batterie SmartShunt | N/.../system/0/Dc/Battery/... | EnergyState, LiveEvent `battery` |
| `irradiance.rs` | Capteur irradiance PRALRAN | `santuario/irradiance/raw` | EnergyState, LiveEvent `irradiance` |
| `tasmota.rs` | Relais chauffe-eau Tasmota | `stat/{id}/POWER`, `tele/{id}/SENSOR` | EnergyState, LiveEvent `tasmota_wh*` |
| `deye_command.rs` | Coupure DEYE anti-surcharge fréquence | `N/.../vebus/.../Ac/Out/L1/F` | MQTT Shelly RPC, EnergyState |
| `water_heater.rs` | Contrôle mode chauffe-eau LG | état partagé (SOC, solaire, grid) | MQTT `santuario/heatpump/1/venus`, API LG ThinQ |
| `charge_current.rs` | Courant de charge VEBus | `IgnoreAcIn1`, PV power, consumption | MQTT `W/.../MaxChargeCurrent`, `W/.../PowerAssistEnabled` |
| `switch_ats.rs` | ATS CHINT keepalive | aucune (timer 60s) | MQTT `santuario/switch/1/venus` |
| `platform.rs` | Statut plateforme (backup) | aucune (timer configurable) | MQTT `santuario/platform/venus` |

---

## 3. Configuration — `Config.toml`

Toute la configuration du gestionnaire se trouve dans la section `[energy_manager]` du fichier `Config.toml`.  
Après modification : `sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart energy-manager`

### Sections et paramètres clés

```toml
[energy_manager.victron]
portal_id         = "c0619ab9929a"   # ID GX portal Victron
vebus_instance    = 275              # Instance VEBus
mppt1_instance    = 273             # MPPT 1
mppt2_instance    = 289             # MPPT 2
pvinverter_instance = 32
shelly_deye_id    = "shellypro2pm-ec62608840a4"
tasmota_waterheater_id = "tongou_3BC764"

[energy_manager.deye]
freq_high_hz      = 52.0   # Seuil coupure DEYE
freq_low_hz       = 50.3   # Seuil réactivation
cut_delay_secs    = 15     # Délai avant coupure
reenable_delay_secs = 45   # Délai avant réactivation
lockout_secs      = 120    # Anti-oscillation

[energy_manager.water_heater]
solar_min_w         = 2000.0   # Production min pour HEAT_PUMP
debounce_secs       = 300      # Délai de stabilisation (5 min)
mode_change_min_secs = 900     # Intervalle min entre changements (15 min)
heat_pump_target_c  = 60.0
vacation_target_c   = 45.0
soc_min_pct         = 90.0     # SOC minimum pour activer HEAT_PUMP (défaut 90%)

[energy_manager.rules]
# Optionnel — si défini, les règles .grl sont lues depuis ce répertoire
# (hot-reload sans recompilation). Fallback sur les règles embarquées.
# dir = "/etc/daly-bms/rules"

[energy_manager.charge_current]
offgrid_max_a        = 70.0
grid_pv_excess_a     = 4.0
grid_no_excess_a     = 0.0
pv_excess_threshold_w = 50.0

[energy_manager.lg_thinq]
enabled          = false   # true si LG ThinQ utilisé
# device_id / bearer_token / api_key → /etc/daly-bms/.env
```

### Secrets (`.env`)

Les valeurs sensibles sont lues depuis `/etc/daly-bms/.env` :

```env
LG_DEVICE_ID=<ID appareil LG>
LG_BEARER_TOKEN=<Bearer token LG>
LG_API_KEY=<API key LG>
```

---

## 4. — inventaire complet des écritures

### Measurement `solar_power`

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

### Measurement `solar_persist`

> Source : `logic/meteo.rs` — fréquence : **1 point par jour, à minuit**

| Type | Nom | Valeur |
|------|-----|--------|
| Tag | `day` | Date du jour qui se termine `YYYY-MM-DD` |
| Tag | `host` | Valeur de `solar.host_tag` (défaut : `"pi5"`) |
| Field (f64) | `total_yield_today_kwh` | Production totale du jour (kWh) |
| Field (f64) | `mppt_yield_today_kwh` | Production MPPT seuls (kWh) |
| Field (f64) | `pvinv_yield_today_kwh` | Production micro-onduleurs (kWh) |

Nom du measurement configurable : `solar.persist_measurement` (défaut : `"solar_persist"`).

### Measurement `battery_status`

> Source : `logic/smartshunt.rs` — fréquence : **à chaque mise à jour MQTT** (chaque seconde env.)

| Type | Nom | Valeur |
|------|-----|--------|
| Tag | `host` | `"pi5"` |
| Field (f64) | `soc_pct` | État de charge batterie (%) |
| Field (f64) | `voltage_v` | Tension batterie (V) |
| Field (f64) | `current_a` | Courant batterie (A, + = charge, - = décharge) |
| Field (f64) | `power_w` | Puissance batterie (W) |
| Field (i64) | `state` | État : 0=idle, 1=charging, 2=discharging |
| Field (i64) | `time_to_go_sec` | Temps restant (s, -1 si inconnu) |

### Measurement `inverter_status`

> Source : `logic/inverter.rs` — fréquence : **à chaque mise à jour MQTT** (chaque seconde env.)

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

### Measurement `switch_ats`

> Source : `logic/switch_ats.rs` — fréquence : **toutes les 60 secondes** (keepalive)

| Type | Nom | Valeur |
|------|-----|--------|
| Tag | `host` | `"pi5"` |
| Field (i64) | `position` | Position ATS : 0=Réseau, 1=Génératrice |
| Field (i64) | `state` | État ATS : 0=inactif, 1=actif, 2=alerte |

### Measurement `deye_relay`

> Source : `logic/deye_command.rs` — fréquence : **à chaque changement d'état** (coupure ou réactivation)

| Type | Nom | Valeur |
|------|-----|--------|
| Tag | `host` | `"pi5"` |
| Tag | `shelly_id` | ID du Shelly (ex: `"shellypro2pm-ec62608840a4"`) |
| Field (i64) | `on` | État relais DEYE : 1=ON, 0=OFF (coupé) |

---

## 5. WebSocket live events

Endpoint : `ws://<pi5>:8081/live`  
Chaque événement est un JSON `{ "stream": "<nom>", "ts": "<ISO8601>", "data": {...} }`.

| Stream | Émis par | Contenu |
|--------|----------|---------|
| `solar` | `solar_power.rs` (1/s) | `solar_total_w`, `mppt_power_w`, `house_power_w` |
| `inverter` | `inverter.rs` | tension/courant/puissance DC+AC, état VEBus |
| `battery` | `smartshunt.rs` | SOC, courant, tension, état, time_to_go |
| `irradiance` | `irradiance.rs` | `wm2` |
| `weather` | `open_meteo.rs` | `temperature_c`, `humidity_pct`, `pressure_hpa`, `wind_speed_ms` |
| `tasmota_wh` | `tasmota.rs` | `on` (bool) |
| `tasmota_wh_energy` | `tasmota.rs` | `power_w`, `voltage_v`, `current_a`, `today_kwh`, `total_kwh` |

---

## 6. Ajouter un nouveau module logique

### Étapes

**1. Créer le fichier** `crates/energy-manager/src/logic/mon_module.rs`

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

**2. Déclarer dans `logic/mod.rs`**

```rust
pub mod mon_module;
```

**3. Brancher dans `main.rs`**

```rust
logic::mon_module::spawn(bus.clone(), state.clone()).await;
```

**4. Si le module a besoin de configuration**, ajouter une section dans `config.rs` :

```rust
#[derive(Debug, Clone, Deserialize, Default)]
pub struct MonModuleConfig {
    #[serde(default = "default_ma_valeur")]
    pub ma_valeur: f64,
}
fn default_ma_valeur() -> f64 { 42.0 }
```

Ajouter `pub mon_module: MonModuleConfig` dans `EnergyManagerConfig` et `#[serde(default)]`.  
Ajouter `[energy_manager.mon_module]` dans `Config.toml`.  
Passer `cfg.mon_module.clone()` au `spawn()`.

**5. Si le module écrit des champs sur `EnergyState`**, ajouter les champs dans `types.rs` :

```rust
pub struct EnergyState {
    // ...
    pub mon_champ: Option<f64>,
}
```

**6. Si le module abonne un nouveau topic MQTT**, l'ajouter dans `mqtt/topics.rs` :

```rust
pub fn all_subscriptions(...) -> Vec<String> {
    vec![
        // ...
        "mon/nouveau/topic".to_string(),
    ]
}
```

**7. Commit, push, déployer** :

```bash
make build-energy-arm
sudo systemctl stop energy-manager
sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/
sudo systemctl start energy-manager
journalctl -u energy-manager -f
```

---

## 7. Modifier un module existant

### Changer un seuil ou délai (sans recompiler)

Si le paramètre est déjà exposé dans `Config.toml` (voir section 3) :

```bash
# 1. Éditer Config.toml
# 2. Copier sur Pi5
sudo cp Config.toml /etc/daly-bms/config.toml
sudo systemctl restart energy-manager
```

### Hot-reload des règles (sans recompilation)

Les règles `.grl` peuvent être modifiées **en production** sans recompiler ni redémarrer.

**Prérequis** : configurer un répertoire dans `Config.toml` :

```toml
[energy_manager.rules]
dir = "/etc/daly-bms/rules"
```

Copier les règles initiales :
```bash
sudo mkdir -p /etc/daly-bms/rules
sudo cp crates/energy-manager/rules/*.grl /etc/daly-bms/rules/
```

**Recharger après modification d'un fichier** :
```bash
# Recharger UNE règle (ex: water_heater)
curl -s -X POST http://192.168.1.141:8081/api/v1/em/rules/reload \
     -H "Content-Type: application/json" \
     -d '{"name":"water_heater"}'

# Recharger TOUTES les règles
curl -s -X POST http://192.168.1.141:8081/api/v1/em/rules/reload \
     -H "Content-Type: application/json" \
     -d '{"name":"*"}'
```

**Lister les règles chargées** (origine: disk ou embedded, timestamp) :
```bash
curl -s http://192.168.1.141:8081/api/v1/em/rules | python3 -m json.tool
```

Si `rules.dir` n'est pas configuré ou si le fichier n'existe pas sur disque, le système utilise automatiquement la règle embarquée dans le binaire.

### Changer la logique métier (recompilation requise)

Exemples de modifications courantes :

| Besoin | Fichier | Ce qu'il faut changer |
|--------|---------|-----------------------|
| Seuil DEYE fréquence | `config.rs` (DeyeConfig) + `Config.toml` | Paramètre `freq_high_hz` |
| Délai debounce chauffe-eau | `config.rs` (WaterHeaterConfig) + `Config.toml` | `debounce_secs` |
| Ajouter un MPPT | `config.rs` (VictronConfig), `mqtt/topics.rs`, `solar_power.rs`, `types.rs` | Nouvelle instance + topic |
| Nouveau topic Victron à surveiller | `mqtt/topics.rs` (`all_subscriptions`) + module concerné | Abonnement + handler |
| Changer fréquence d'écriture DB | `logic/solar_power.rs` ligne `interval(Duration::from_secs(1))` | Valeur en secondes |
| Ajouter un champ DB | Module concerné, appel `.field_f(...)` sur `DB` | Nouveau field |

---

## 8. Tests unitaires des règles

Chaque module de règles `.grl` a une suite de tests unitaires dans son fichier `rules.rs`.

```bash
# Lancer tous les tests (34 tests, ~0.3s)
cargo test -p energy-manager

# Lancer les tests d'une règle spécifique
cargo test -p energy-manager deye_command
cargo test -p energy-manager water_heater
```

Les tests couvrent :
- `charge_current` — 4 tests (offgrid, grid+PV, grid sans PV, etc.)
- `deye_command` — 9 tests (toutes transitions, grille reconnectée, lockout)
- `irradiance` — 5 tests (valides/invalides, plage 0–2000 W/m²)
- `smartshunt` — 6 tests (capture baseline chargée/déchargée)
- `solar_power` — 4 tests (nouveau jour, baseline absente)
- `water_heater` — 5 tests (grid connecté, SOC bas, irradiance faible)

**Ajouter un test** : dans le module `rules.rs` concerné, ajouter dans `#[cfg(test)] mod tests`:

```rust
#[test]
fn mon_nouveau_cas() {
    let mut e = MonRuleEngine::new().unwrap();
    let result = e.evaluate(/* params */);
    assert_eq!(result.unwrap(), "EXPECTED");
}
```

---

## 9. Persistance d'état DEYE

L'état du relais DEYE (On/Off) est persisté en MQTT retained sur `santuario/persist/deye_state`.  
Au redémarrage, le service attend 3 secondes avant d'activer la logique DEYE pour laisser
le temps au broker MQTT de livrer le message retained.

États persistés :
- `"On"` — DEYE actif (états `On` ou `PendingCut`)
- `"Off"` — DEYE coupé (états `Off`, `Lockout`, `PendingRestore`)

---

## 10. Supprimer un module

1. Supprimer le fichier `logic/mon_module.rs`
2. Retirer `pub mod mon_module;` dans `logic/mod.rs`
3. Retirer la ligne `logic::mon_module::spawn(...)` dans `main.rs`
4. Retirer la section de config dans `config.rs` et `Config.toml` (si applicable)
5. Retirer les champs orphelins dans `EnergyState` (types.rs) si plus utilisés
6. Retirer les topics MQTT orphelins dans `mqtt/topics.rs`
7. Recompiler et déployer

---

## 11. Ajouter un nouveau publish MQTT

**1. Déclarer le topic** dans `mqtt/topics.rs` :

```rust
pub mod publish {
    // ...
    pub const MON_TOPIC: &str = "santuario/mon/venus";
    // ou fonction dynamique :
    pub fn mon_topic_dynamique(id: u32) -> String {
        format!("santuario/mon/{id}/venus")
    }
}
```

**2. Publier depuis un module** :

```rust
use crate::mqtt::topics::publish;
use crate::types::MqttOutgoing;

// Retained (Venus OS keepalive) :
bus.publish(MqttOutgoing::retained(publish::MON_TOPIC, &payload)).await;

// Non-retained (événement) :
bus.publish(MqttOutgoing::transient(publish::MON_TOPIC, &payload)).await;

// Texte brut :
bus.publish(MqttOutgoing::raw(publish::MON_TOPIC, "valeur", false)).await;
```

---


---

## 12. Débogage

```bash
# Logs en continu
journalctl -u energy-manager -f

# Augmenter le niveau de log (sans recompiler)
sudo systemctl edit energy-manager
# Ajouter :
# [Service]
# Environment=RUST_LOG=debug

# Voir les messages MQTT en temps réel
mosquitto_sub -h 192.168.1.141 -t "santuario/#" -v
mosquitto_sub -h 192.168.1.141 -t "N/c0619ab9929a/#" -v

# WebSocket live events
websocat ws://192.168.1.141:8081/live

```

---

## 13. Installation initiale du service (première fois)

```bash
# Depuis le repo sur Pi5 :
make build-energy-arm
sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/
sudo cp contrib/energy-manager.service /etc/systemd/system/
sudo cp Config.toml /etc/daly-bms/config.toml
# Créer /etc/daly-bms/.env  etc.
sudo systemctl daemon-reload
sudo systemctl enable energy-manager
sudo systemctl start energy-manager
journalctl -u energy-manager -f
```
