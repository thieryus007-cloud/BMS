# Guide d'intégration d'un device Venus OS via MQTT → D-Bus (Rust)

Ce document décrit exactement ce qui a été mis en place pour intégrer un nouveau type
de device sur le bus D-Bus de Venus OS, en utilisant le bridge MQTT → Rust → D-Bus.
Il sert de référence pour toute future intégration.

---
# référence:

https://github.com/victronenergy/venus/wiki/dbus

https://github.com/sebdehne/dbus-mqtt-services

https://www.waveshare.com/wiki/RS485_CAN_HAT_(B)

## A implementer:

- com.victronenergy.battery             360Ah & 320Ah & 628Ah
- com.victronenergy.meteo               irradiance
- com.victronenergy.temperatures        temperature & humidité     
- com.victronenergy.heatpump            Chauffeau & PAC
- com.victronenergy.switch              ATS & autres
- com.victronenergy.platform            Backup to & Restore from Pi5

-- Grid (and acload and genset) meter
com.victronenergy.grid
com.victronenergy.acload (when used as consumer to measure an acload)
/Ac/L1/Current         <- A AC
/Ac/L1/Energy/Forward  <- kWh
/Ac/L1/Power           <- W, real power
/Ac/L1/Voltage         <- V AC
/DeviceType
/IsGenericEnergyMeter  <- When an energy meter masquarades as a genset or acload, this is set to 1.
---

## Architecture générale

```
[Source de données]
        │
        │ (HTTP, RS485, Shelly, LG ThinQ API...)
        ▼
[energy-manager sur Pi5]
        │
        │ MQTT publish  topic: santuario/{type}/{index}/venus
        │               payload: JSON {"Champ": valeur, ...}
        ▼
[Mosquitto Pi5 - dalybms-mosquitto:1883]
        │
        │ Bridge MQTT  direction: out  (Pi5 → NanoPi)
        ▼
[Mosquitto NanoPi - 192.168.1.120:1883]
        │
        │ MQTT subscribe
        ▼
[dbus-mqtt-venus (Rust) sur NanoPi]
        │
        │ zbus / D-Bus system bus
        ▼
[Venus OS D-Bus]
        │
        ├─ com.victronenergy.{type}.{prefix}_{index}
        │     /Connected, /ProductName, /DeviceInstance
        │     /Mgmt/ProcessName, /Mgmt/ProcessVersion, /Mgmt/Connection
        │     + chemins spécifiques au type de device
        ▼
[VRM Portal / GX Device local UI]
```

---

## Infrastructure réseau

| Machine | IP | Rôle |
|---|---|---|
| Pi5 (Raspberry Pi 5) | 192.168.1.141 | Docker : Mosquitto, energy-manager |
| NanoPi Neo3 | 192.168.1.120 | Venus OS, service Rust dbus-mqtt-venus, D-Bus |

---

## Exemple complet : Capteur de température extérieure

### 1. Type D-Bus Victron utilisé

`com.victronenergy.temperature` — wiki Victron :
<https://github.com/victronenergy/venus/wiki/dbus#temperatures>

Chemins exposés :
- `/Temperature` — °C (float)
- `/TemperatureType` — 0=battery 1=fridge 2=generic 3=Room 4=Outdoor 5=WaterHeater 6=Freezer
- `/CustomName` — chaîne libre
- `/Humidity` — % humidité (float, 0.0 si absent)
- `/Pressure` — kPa (float, 0.0 si absent)
- `/Status` — 0=OK, 1=Disconnected
- `/Connected` — 0 ou 1
- `/ProductName`, `/ProductId`, `/DeviceInstance`
- `/Mgmt/ProcessName`, `/Mgmt/ProcessVersion`, `/Mgmt/Connection`

### 2. Configuration Config.toml (NanoPi)

```toml
[heat]
topic_prefix = "santuario/heat"

[[sensors]]
mqtt_index       = 1
name             = "Temperature Exterieure"
temperature_type = 4        # 4 = Outdoor
device_instance  = 20       # doit être unique sur le bus D-Bus
```

### 3. Topic MQTT

```
santuario/heat/1/venus
```

Payload JSON (publié par energy-manager) :
```json
{"Temperature": 11.5, "Humidity": 42.0}
```

### 4. Nom du service D-Bus résultant

```
com.victronenergy.temperature.mqtt_1
```

### 5. Flux energy-manager (meteo.json)

**Inject → HTTP Open-Meteo → Extraire température → mqtt out**

Fréquence de fetch : toutes les 15 minutes (900s)
Keepalive MQTT : toutes les **25 secondes** (< watchdog Rust de 30s)

Fonction "Extraire température" :
```javascript
const temp     = msg.payload.current.temperature_2m;
const humidity = msg.payload.current.relative_humidity_2m;

global.set('outdoor_temp', temp);
global.set('outdoor_humidity', humidity);

node.status({fill: 'green', shape: 'dot', text: `${temp}°C — ${humidity}%`});

return {
    topic:   'santuario/heat/1/venus',
    payload: JSON.stringify({ Temperature: temp, Humidity: humidity })
};
```

Fonction "Republier depuis contexte" (keepalive 25s) :
```javascript
const temp     = global.get('outdoor_temp');
const humidity = global.get('outdoor_humidity');

if (temp === undefined || temp === null) { return null; }

return {
    topic:   'santuario/heat/1/venus',
    payload: JSON.stringify({ Temperature: temp, Humidity: humidity })
};
```

**Point critique :** le keepalive doit être < `watchdog_sec` (30s par défaut).
Si le keepalive est trop long (ex: 60s), le service Rust met `/Connected = 0`
entre les publications et le device disparaît du VRM.

---

## Configuration Mosquitto bridge (Pi5)

Fichier : `docker/mosquitto/config/mosquitto.conf`

### Direction NanoPi → Pi5 (données publiées par le Rust)
```
topic santuario/# in 0
```
Sert à victoria pour lire les données BMS.

### Direction Pi5 → NanoPi (commandes energy-manager → service Rust)
```
topic santuario/heat/#     out 0
topic santuario/heatpump/# out 0
topic santuario/meteo/#    out 0
```

**Règle :** chaque nouveau type de device nécessite une règle `out` spécifique.
Ne pas utiliser `santuario/# both` pour éviter les boucles de messages.

---

## Watchdog et keepalive

Le service Rust gère deux intervalles (configurables dans Config.toml section `[venus]`) :

| Paramètre | Défaut | Rôle |
|---|---|---|
| `republish_sec` | 25s | Réémet `ItemsChanged` vers D-Bus même sans nouveau MQTT |
| `watchdog_sec` | 30s | Après ce délai sans MQTT, met `/Connected = 0` |

energy-manager doit publier le topic au moins une fois par `watchdog_sec`.
Pour les sources lentes (Open-Meteo = 15 min), un nœud keepalive est obligatoire.

---

## Fichiers Rust impactés pour un nouveau device

| Fichier | Rôle |
|---|---|
| `crates/dbus-mqtt-venus/src/types.rs` | Struct payload MQTT (serde Deserialize) |
| `crates/dbus-mqtt-venus/src/config.rs` | Config TOML : `[heat]`, `[[sensors]]`, etc. |
| `crates/dbus-mqtt-venus/src/{type}_service.rs` | Enregistrement D-Bus zbus |
| `crates/dbus-mqtt-venus/src/{type}_manager.rs` | Boucle MQTT → D-Bus, watchdog |
| `crates/dbus-mqtt-venus/src/mqtt_source.rs` | Abonnement MQTT, événements |
| `crates/dbus-mqtt-venus/src/main.rs` | Lancement du manager en tâche Tokio |

### Point important sur l'enregistrement des chemins D-Bus

Les objets feuilles D-Bus sont enregistrés **une seule fois** à la création du service,
depuis l'état initial `disconnected()`. Il faut donc que **tous les chemins** soient
présents dans `to_items()` même à l'état déconnecté, avec une valeur par défaut.

```rust
// CORRECT : toujours inclus, 0.0 si absent
m.insert("/Humidity".into(), DbusItem::f64(self.humidity.unwrap_or(0.0), "%"));

// INCORRECT : chemin non enregistré si None au démarrage
if let Some(h) = self.humidity {
    m.insert("/Humidity".into(), DbusItem::f64(h, "%"));
}
```

La méthode `GetItems()` sur la racine `/` (utilisée par VRM) fonctionne dans les deux
cas car elle appelle `to_items()` au moment de la requête. Mais `GetValue()` sur un
chemin individuel échoue avec "Unknown object" si l'objet feuille n'est pas enregistré.

---

## Procédure de déploiement (compilation ARMv7 → NanoPi)

Le NanoPi est en architecture **ARMv7 32-bit** (`armv7-unknown-linux-gnueabihf`).
La compilation cross-platform se fait sur le Pi5.

**Flux complet : GitHub → Pi5 (git pull + compile) → NanoPi (scp)**

### Prérequis (une seule fois)

```bash
# Installer le cross-compilateur ARM
apt-get install -y gcc-arm-linux-gnueabihf

# Ajouter la target Rust
rustup target add armv7-unknown-linux-gnueabihf
```

### Étape 1 — Récupérer les dernières modifications (Pi5)

```bash
cd ~/Daly-BMS-Rust
git pull origin claude/migrate-energy-manager-pi5-91idx
```

### Étape 2 — Compiler pour ARMv7 (Pi5)

```bash
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --release \
  --target armv7-unknown-linux-gnueabihf \
  -p dbus-mqtt-venus
```

Binaire produit : `target/armv7-unknown-linux-gnueabihf/release/dbus-mqtt-venus`

### Étape 3 — Déployer le binaire sur NanoPi

**Ordre obligatoire : arrêter avant de copier** (sinon erreur "Failure" scp).

```bash
# 3a. Arrêter le service sur NanoPi
ssh root@192.168.1.120 "svc -d /data/etc/sv/dbus-mqtt-venus"

# 3b. Copier le binaire depuis Pi5
scp target/armv7-unknown-linux-gnueabihf/release/dbus-mqtt-venus \
    root@192.168.1.120:/data/daly-bms/dbus-mqtt-venus

# 3c. Redémarrer le service sur NanoPi
ssh root@192.168.1.120 "svc -u /data/etc/sv/dbus-mqtt-venus"
```

### Étape 4 — Déployer la configuration si modifiée (Pi5 → NanoPi)

Le fichier `Config.toml` est partagé par `daly-bms-server` et `dbus-mqtt-venus`.

```bash
scp Config.toml root@192.168.1.120:/data/daly-bms/config.toml

# Redémarrer les deux services
ssh root@192.168.1.120 "svc -d /data/etc/sv/dbus-mqtt-venus && \
                        svc -d /data/etc/sv/daly-bms-server && \
                        svc -u /data/etc/sv/daly-bms-server && \
                        svc -u /data/etc/sv/dbus-mqtt-venus"
```

### Étape 5 — Déployer mosquitto.conf si modifié (Pi5 Docker)

```bash
cd ~/Daly-BMS-Rust
git pull origin claude/migrate-energy-manager-pi5-91idx
docker compose restart mosquitto
```

### Étape 6 — Mettre à jour les flux energy-manager si modifiés

```bash
# Sur Pi5 — récupérer les derniers JSON de flux
cd ~/Daly-BMS-Rust
git pull origin claude/migrate-energy-manager-pi5-91idx
# Puis importer manuellement dans energy-manager (voir section ci-dessous)
```

---

## Procédure d'import d'un flux energy-manager

1. Ouvrir energy-manager : http://192.168.1.141:8081
2. Double-clic sur l'onglet existant → clic **Delete** → confirmer
3. Menu ≡ → **Import** → coller le JSON → **Import**
4. Vérifier les nœuds (broker connecté, topics corrects)
5. Cliquer **Deploy**

---

## Commandes de vérification (NanoPi)

### Vérifier que le service tourne

```bash
ps | grep daly
# Doit afficher : /data/daly-bms/dbus-mqtt-venus --config /data/daly-bms/config.toml
```

### Lister tous les services D-Bus Victron actifs

```bash
dbus -y | grep victronenergy
```

### Lire toutes les valeurs d'un service (méthode principale)

```bash
dbus -y com.victronenergy.temperature.mqtt_1 / GetItems
```

Retourne un dictionnaire de tous les chemins avec valeur et texte.

### Lire une valeur individuelle

```bash
dbus -y com.victronenergy.temperature.mqtt_1 /Temperature GetValue
dbus -y com.victronenergy.temperature.mqtt_1 /Humidity    GetValue
dbus -y com.victronenergy.temperature.mqtt_1 /Connected   GetValue
```

**Note :** `GetValue` sur un chemin individuel nécessite que l'objet feuille
soit enregistré dans zbus. Si non, erreur "Unknown object". `GetItems` sur `/`
fonctionne toujours. VRM utilise `GetItems`.

### Vérifier la réception MQTT sur NanoPi

```bash
mosquitto_sub -h localhost -t "santuario/heat/1/venus" -v
```

### Vérifier les logs du service Rust

Le service utilise `supervise` (runit) sans fichier log dédié.
Les traces apparaissent dans `readproctitle` :

```bash
ps | grep readproctitle
```

---

---

## Exemple complet : Chauffe-eau Victron HeatPump (LG ThinQ)

### 1. Type D-Bus Victron utilisé

`com.victronenergy.heatpump` — wiki Victron :
<https://github.com/victronenergy/venus/wiki/dbus#heatpump>

Chemins exposés (tous obligatoirement enregistrés au démarrage) :
- `/State` — état de la pompe (enum, voir table ci-dessous)
- `/Temperature` — température eau courante °C (0.0 si inconnu)
- `/TargetTemperature` — température cible °C (0.0 si inconnue)
- `/Ac/Power` — puissance consommée W
- `/Ac/Energy/Forward` — énergie totale kWh
- `/Position` — 0=AC Output, 1=AC Input

### 2. Table State (mapping LG ThinQ → Victron)

| Valeur | Signification | Mode LG ThinQ | Opération |
|---|---|---|---|
| 0 | Off / Vacation | VACATION ou POWER_OFF | — |
| 1 | Heat Pump (normal) | HEAT_PUMP | POWER_ON |
| 2 | Turbo / Boost | TURBO | POWER_ON |

### 3. Configuration Config.toml

```toml
[heatpump]
topic_prefix = "santuario/heatpump"

[[heatpumps]]
mqtt_index      = 1       # Topic : santuario/heatpump/1/venus
name            = "Chauffe-eau"
device_instance = 30      # DeviceInstance unique sur D-Bus
```

### 4. Topic MQTT

```
santuario/heatpump/1/venus
```

Payload JSON (publié par energy-manager) :
```json
{
  "State": 1,
  "Temperature": 60.0,
  "TargetTemperature": 52.0,
  "Position": 0
}
```

Payload étendu (si puissance disponible via compteur externe) :
```json
{
  "State": 1,
  "Temperature": 60.0,
  "TargetTemperature": 52.0,
  "Ac": { "Power": 1200.0, "Energy": { "Forward": 125.5 } },
  "Position": 0
}
```

### 5. Nom du service D-Bus résultant

```
com.victronenergy.heatpump.mqtt_1
```

### 6. Source de données : LG ThinQ API

L'état est récupéré toutes les 10 minutes via l'API REST LG ThinQ :

```
GET https://api-eic.lgthinq.com/devices/{device_id}/state
Authorization: Bearer {thinqpat_token}
```

Réponse utilisée :
```json
{
  "response": {
    "waterHeaterJobMode": { "currentJobMode": "HEAT_PUMP" },
    "operation":          { "waterHeaterOperationMode": "POWER_ON" },
    "temperature":        { "currentTemperature": 60, "targetTemperature": 52 }
  }
}
```

### 7. Commandes SET disponibles dans energy-manager

```
POST https://api-eic.lgthinq.com/devices/{device_id}/control
```

| Commande | Payload |
|---|---|
| Activer mode HEAT_PUMP | `{"waterHeaterJobMode": {"currentJobMode": "HEAT_PUMP"}}` |
| Activer mode TURBO | `{"waterHeaterJobMode": {"currentJobMode": "TURBO"}}` |
| Régler température 40°C | `{"temperature": {"targetTemperature": 40}}` |
| Régler température 55°C | `{"temperature": {"targetTemperature": 55}}` |

### 8. Flux energy-manager (setwaterheater.json)

**Structure :**
```
Inject (poll 600s + oneshot 5s)
Inject (test manuel)
    └─► Préparer GET état → GET /state LG ThinQ → Parser état → HeatpumpPayload
            ├─► mqtt out : santuario/heatpump/1/venus    ◄─ keepalive 25s
            └─► debug complet

Inject keepalive 25s → Republier depuis global context → mqtt out (même nœud)

Inject SET TURBO      → POST /control → debug
Inject SET HEAT_PUMP  → POST /control → debug
Inject SET 40°C Nuit  → POST /control → debug
Inject SET 55°C Jour  → POST /control → debug
```

Fonction de parsing (extrait) :
```javascript
const stateMapping = { 'HEAT_PUMP': 1, 'TURBO': 2, 'VACATION': 0 };
const isPoweredOn = operation === 'POWER_ON';
const state = isPoweredOn ? (stateMapping[mode] ?? 1) : 0;

const payload = {
    State:             state,
    Temperature:       currentTemp,
    TargetTemperature: targetTemp,
    Position:          0
};
global.set('heatpump_payload', payload);  // pour keepalive
```

### 9. Commandes de vérification D-Bus

```bash
# Lister tous les chemins du service
dbus -y com.victronenergy.heatpump.mqtt_1 / GetItems

# Valeurs individuelles
dbus -y com.victronenergy.heatpump.mqtt_1 /State              GetValue
dbus -y com.victronenergy.heatpump.mqtt_1 /Temperature        GetValue
dbus -y com.victronenergy.heatpump.mqtt_1 /TargetTemperature  GetValue
dbus -y com.victronenergy.heatpump.mqtt_1 /Ac/Power           GetValue
dbus -y com.victronenergy.heatpump.mqtt_1 /Position           GetValue
dbus -y com.victronenergy.heatpump.mqtt_1 /Connected          GetValue
```

### 10. Test MQTT direct (sans energy-manager)

```bash
# Depuis Pi5 ou NanoPi
mosquitto_pub -h localhost -t "santuario/heatpump/1/venus" \
  -m '{"State":1,"Temperature":60.0,"TargetTemperature":52.0,"Position":0}'

# Vérifier la réception sur NanoPi
mosquitto_sub -h localhost -t "santuario/heatpump/1/venus" -v
```

---

## Devices implémentés

| Device | Service D-Bus | Topic MQTT | Index config |
|---|---|---|---|
| Batterie Daly (360Ah, 320Ah, 628Ah) | `com.victronenergy.battery.mqtt_{n}` | `santuario/bms/{n}/venus` | `[[bms]]` |
| Température / Humidité | `com.victronenergy.temperature.mqtt_{n}` | `santuario/heat/{n}/venus` | `[[sensors]]` |
| Chauffe-eau (HeatPump) | `com.victronenergy.heatpump.mqtt_{n}` | `santuario/heatpump/{n}/venus` | `[[heatpumps]]` |
| Irradiance (Meteo) | `com.victronenergy.meteo` | `santuario/meteo/venus` | `[meteo]` |
| Switch / ATS CHINT | `com.victronenergy.switch.mqtt_{n}` | `santuario/switch/{n}/venus` | `[[switches]]` |
| Compteur réseau (Grid) | `com.victronenergy.grid.mqtt_{n}` | `santuario/grid/{n}/venus` | `[[grids]]` |
| Compteur consommation (ACload) | `com.victronenergy.acload.mqtt_{n}` | `santuario/grid/{n}/venus` | `[[grids]]` (service_type="acload") |
| Backup/Restore Pi5 | `com.victronenergy.platform` | `santuario/platform/venus` | `[platform]` |

---

## Exemple : Switch / ATS CHINT. -- voir annexe victron switch parameter -- bas de page

### 1. Type D-Bus utilisé

`com.victronenergy.switch` — wiki Victron :
<https://github.com/victronenergy/venus/wiki/dbus#switches>

Chemins exposés :
- `/Position` — 0=AC1 (onduleur), 1=AC2 (réseau)
- `/State` — 0=inactive, 1=active, 2=alerted
- `/Connected`, `/ProductName`, `/DeviceInstance`

### 2. Configuration Config.toml

```toml
[switch]
topic_prefix = "santuario/switch"

[[switches]]
mqtt_index      = 1
name            = "ATS CHINT"
device_instance = 60

[[switches]]
mqtt_index      = 2
name            = "tongou_3BC764"
device_instance = 61

[[switches]]
mqtt_index      = 3
name            = "tongou_0A3FA0"
device_instance = 62

[[switches]]
mqtt_index      = 4
name            = "tongou_0A3C14"
device_instance = 63

[[switches]]
mqtt_index      = 5
name            = "tongou_0A4040"
device_instance = 64
```

### 3. Topic MQTT

```
santuario/switch/1/venus
```

Payload JSON (publié par energy-manager) :
```json
{"Position": 0, "State": 1}
```

| Position | Signification |
|---|---|
| 0 | AC1 — onduleur |
| 1 | AC2 — réseau |

### 4. Service D-Bus résultant

```
com.victronenergy.switch.mqtt_1
```

### 5. Vérification D-Bus

```bash
dbus -y com.victronenergy.switch.mqtt_1 / GetItems
dbus -y com.victronenergy.switch.mqtt_1 /Position GetValue
dbus -y com.victronenergy.switch.mqtt_1 /State    GetValue
```

---

## Exemple : Compteur réseau (Grid / ACload)

### 1. Type D-Bus utilisé

`com.victronenergy.grid` ou `com.victronenergy.acload` :
<https://github.com/victronenergy/venus/wiki/dbus#grid-and-acload-and-genset-meter>

Chemins exposés (L1, L2, L3 — tous enregistrés dès le démarrage) :
- `/Ac/L1/Voltage` — V AC
- `/Ac/L1/Current` — A
- `/Ac/L1/Power` — W (puissance réelle)
- `/Ac/L1/Energy/Forward` — kWh consommés
- `/Ac/L1/Energy/Reverse` — kWh injectés
- `/Ac/L2/...` et `/Ac/L3/...` — même structure (0.0 si monophasé)
- `/DeviceType` — 340 = generic energy meter
- `/IsGenericEnergyMeter` — 1 si compteur générique masquerade

### 2. Configuration Config.toml

```toml
[grid]
topic_prefix = "santuario/grid"

[[grids]]
mqtt_index      = 1
name            = "Compteur EDF"
device_instance = 70
service_type    = "grid"    # ou "acload"

[[grids]]
mqtt_index      = 2
name            = "Consommation AC"
device_instance = 71
service_type    = "acload"
```

### 3. Topic MQTT

```
santuario/grid/1/venus
```

Payload JSON monophasé :
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

Payload JSON triphasé :
```json
{
  "Ac": {
    "L1": {"Voltage": 230.0, "Current": 5.2, "Power": 1196.0, "Energy": {"Forward": 400.0}},
    "L2": {"Voltage": 231.0, "Current": 4.8, "Power": 1108.8, "Energy": {"Forward": 380.0}},
    "L3": {"Voltage": 229.0, "Current": 6.1, "Power": 1396.9, "Energy": {"Forward": 450.0}}
  }
}
```

### 4. Service D-Bus résultant

```
com.victronenergy.grid.mqtt_1     (si service_type = "grid")
com.victronenergy.acload.mqtt_2   (si service_type = "acload")
```

### 5. Vérification D-Bus

```bash
dbus -y com.victronenergy.grid.mqtt_1 / GetItems
dbus -y com.victronenergy.grid.mqtt_1 /Ac/L1/Power   GetValue
dbus -y com.victronenergy.grid.mqtt_1 /Ac/L1/Voltage GetValue
```

---

## Exemple : Platform Backup/Restore Pi5

### 1. Service D-Bus utilisé

`com.victronenergy.platform` (singleton — service custom)

Chemins exposés :
- `/Backup/Status` — 0=idle, 1=running, 2=OK, 3=error
- `/Backup/LastRun` — timestamp Unix (secondes)
- `/Restore/Status` — 0=idle, 1=running, 2=OK, 3=error
- `/Restore/LastRun` — timestamp Unix (secondes)

### 2. Configuration Config.toml

```toml
[platform]
topic           = "santuario/platform/venus"
product_name    = "Pi5 Platform"
device_instance = 50
enabled         = true
```

### 3. Topic MQTT

```
santuario/platform/venus
```

Payload JSON :
```json
{
  "Backup":  {"Status": 2, "LastRun": 1710000000},
  "Restore": {"Status": 0, "LastRun": 0}
}
```

### 4. Flux energy-manager (exemple)

```javascript
// Déclencher un backup Pi5 via script SSH ou API
// Puis publier le statut :
return {
  topic:   'santuario/platform/venus',
  payload: JSON.stringify({
    Backup:  { Status: 2, LastRun: Math.floor(Date.now() / 1000) },
    Restore: { Status: 0, LastRun: 0 }
  })
};
```

### 5. Vérification D-Bus

```bash
dbus -y com.victronenergy.platform / GetItems
dbus -y com.victronenergy.platform /Backup/Status  GetValue
dbus -y com.victronenergy.platform /Backup/LastRun GetValue
```

---

## Batterie agrégée 628Ah (BMS-3 virtuel)

La batterie 628Ah est une batterie **virtuelle agrégée** calculée par energy-manager
à partir des données BMS-1 (360Ah) et BMS-2 (268Ah) ou des deux BMS réels.

Configuration dans Config.toml :

```toml
[[bms]]
address         = "0x03"
name            = "BMS-628Ah"
capacity_ah     = 628.0
mqtt_index      = 3
device_instance = 143
```

Flux energy-manager — calcul agrégé et publication sur `santuario/bms/3/venus` :
```javascript
const bms1 = global.get('bms1_snapshot');
const bms2 = global.get('bms2_snapshot');
// Combiner les données et publier le payload VenusPayload agrégé
```

---

## Résolution des problèmes courants

### Service D-Bus non visible

1. Vérifier que le service Rust tourne : `ps | grep dbus-mqtt-venus`
2. Vérifier qu'un message MQTT a été reçu (le service D-Bus est créé au 1er message)
3. Vérifier le bridge Mosquitto : règle `out` présente pour le topic concerné
4. Vérifier que energy-manager est déployé et le nœud connecté (vert "Connecté")

### /Connected = 0 (device déconnecté dans VRM)

Le keepalive energy-manager est trop long (> `watchdog_sec` = 30s).
Réduire le repeat de l'inject keepalive à 25s maximum.

### scp échoue avec "Failure"

Le service cible est actif et verrouille le binaire. Faire `svc -d` avant le `scp`.

### git pull échoue (local changes)

```bash
# Si fichier appartient à un autre utilisateur (ex: mosquitto Docker)
sudo chown $(whoami):$(whoami) docker/mosquitto/config/
sudo chown $(whoami):$(whoami) docker/mosquitto/config/mosquitto.conf
```

### Architecture mismatch (binaire invalide)

NanoPi = ARMv7 32-bit. Le binaire Pi5 (aarch64) ne fonctionne pas.
Toujours compiler avec `--target armv7-unknown-linux-gnueabihf`.

### Onglet energy-manager vide après docker compose down/up

Les volumes energy-manager sont persistants. Si les flux disparaissent :
1. Vérifier `docker volume ls | grep energy-manager`
2. Réimporter depuis `flux-energy-manager/*.json`


### annexe: Victron switch parameters

com.victronenergy.switch

Generic:
/State          <-- Current state of the whole module. Visible in the UI in the Device List -> 
                    SmartSwitch -> Settings. Not visible on the switch card, also not necessary
                    because in case of a module level problem, all channels will indicate disabled.
                    Values offset by 0x100 to allow common state component in QML
                    0x100 = Connected
                    0x101 = Over temperature
                    0x102 = Temperature warning
                    0x103 = Channel fault
                    0x104 = Channel Tripped
                    0x105 = Under Voltage

CONFIGURATION PATHS PER CHANNEL:
There may be channel wide configuration which must be set before the channel can identify 
itself as a switchable output or as a generic input. For this, the following paths can be used. 
Make sure the same channel index ('x') is used both here and in the 
switchable output or generic input API. 

/Channel/x/Direction                 <-- RW (optional) Set the channel direction. 0: output, 1: input, -1: not defined (yet)

OPERATIONAL PATHS PER CHANNEL:
Note that <x> should be a clear label of the output channel, not necessarily an integer. 
<x> can be used in the UI to display the output when /SwitchableOutput/x/Settings/CustomName 
is not set or not valid.

/SwitchableOutput/x/State             <-- RW (optional) / Requested on/off state of channel, separate from dimming.
                                          In rare cases, state can be absent/invalid. If this is the case, the 
                                          UI element in the switch pane should not be shown.
/SwitchableOutput/x/Dimming           <-- RW (optional) / 0 to 100%, read/write.
                                          Optional: required only for dimmable outputs, otherwise invalid or
                                          doesn't exist. 
/SwitchableOutput/x/LightControls     <-- RW (optional) 
                                          Used for multi-channel dimmers (types 11, 12, 13), this is 
                                          an array of ints with the following values:
                                          0 | Hue               | 0 - 360 degrees
                                          1 | Saturation        | 0 - 100 %
                                          2 | Brightness        | 0 - 100 %
                                          3 | White             | 0 - 100 %
                                          4 | Color temperature | 0 - 6500 K
                                          Producers should change the parameters relevant for their type (check /Type)
                                          Do not set the unused parameters to invalid or NULL.
/SwitchableOutput/x/Measurement       <-- R (optional) / Measured value of the actuator that is controlled by /Dimming. 
                                          e.g. for a temperature setpoint slider, /Dimming holds the setpoint value and
                                          /Measurement holds the measured temperature, if available.
                                          GUI will display the measured value if this path is populated. 
/SwitchableOutput/x/Name              <-- R / Channel default name, must be set by the driver and is not writable.
/SwitchableOutput/x/Status            <-- R / Channel status

                       Normal states, visible in the component itself:
                       0x00 = Off
                       0x09 = On <- OR-ed state of Active and Input Active. Note that the 0x8 output fault bit is set in normal operation.

                       Exceptional states visible in the component itself

                       0x02 = Tripped
                       0x04 = Over temperature

                       Exceptional states, made visible by a extra label in the UI:
                       0x01 = Powered <- Voltage present on the Channels supply in the case where the 
                                         channels are individually supplied or if not and channel output
                                         is being back fed. In latter case query if input/analog input 
                                         and if they go on dbus as com.victronenergy.digitalinput
                       0x08 = Output fault <- Generic output fault.
                       0x10 = Short fault  <- A certain hardware error that the switch self-diagnoses (ES Smart switch specific)
                       0x20 = Disabled     <- The hardware indicates this status in case for some 
                                              reason the switch is disabled. For example in case
                                              the whole module is in over temperature.
                       0x40 = Bypassed     <- The switching circuit is bypassed, so the channel is permanently on.
                       0x80 = Ext. control <- The switching circuit is externally controlled and dimming/value might not be valid. 
                                              For example for a RGB light that is in sound mode, it is not needed to continuously update the color.
/SwitchableOutput/x/Auto           <-- RW (optional) Used by the three-state switch (9) and bilge pump control (10). 
                                       When set, the driver or another service will control `/State`, and 
                                       the user cannot control the state from the UI.
                                       0: Manual mode, user can control the output from the UI. (default)
                                       1: Auto mode, user cannot control the output from the UI.
/SwitchableOutput/x/Temperature    <-- R / temperature of the switch,
                                       Optional: not all output types will feature temp. measurement.
/SwitchableOutput/x/Voltage        <-- R / voltage of its output
                                       Optional: not all output types will feature voltage measurement.
/SwitchableOutput/x/Current        <-- R / the current in amps. optional
                                       Optional: not all output types will feature current measurement.


SETTINGS:
/SwitchableOutput/x/Settings/Adjustable        <-- R (optional) / Indicates if ALL settings are writable / adjustable or not. 
                                                   Setting the path explicitly to 0 makes all settings read-only.
                                                   0: Settings are not adjustable
                                                   1 / any other value / not present / invalid: Settings are adjustable

/SwitchableOutput/x/Settings/Group             <-- RW / max 32 bytes utf8 long string, free input, used by the 
                                                   UI to group switches with the same group name onto 
                                                   the same card. When left blank (dbus-invalid), the UI 
                                                   falls back to grouping them by dbus service.
/SwitchableOutput/x/Settings/CustomName        <-- RW / the label; max 32 bytes utf8 long string. Preferably stored on the device itself.
/SwitchableOutput/x/Settings/ShowUIControl     <-- RW / integer

                                                   (optional) Indicate if the control is shown in the local  and in the remote UI
                                                   If used, set to 1 by default.
                                                   (for usage in energy-manager, but not in the gui)

                                                   Values:
                                                   0bxx1: Show control in all UI's
                                                   0b000: Hide in all UI's
                                                   0bx10: Show in local UI's (GUI running natively on GX, MFD and WASM over local LAN)
                                                   0b1x0: Show in remote UI's (VRM remote console and VRM switch pane)

/SwitchableOutput/x/Settings/Type              <-- RW / Specifies the output type:
                                                       0 = momentary
                                                       1 = toggle
                                                       2 = dimmable (pwm)
                                                       3 = Temperature setpoint
                                                       4 = Stepped switch
                                                       5 = Slave mode (ES Smartswitch only)
                                                       6 = Dropdown
                                                       7 = Basic slider
                                                       8 = Numeric input box
                                                       9 = Three-state switch
                                                       10 = Bilge pump control
                                                       11 = RGB color wheel
                                                       12 = CCT color wheel
                                                       13 = RGBW color wheel
                                                   Preferably stored on the device itself. 
                                                   The device should reset the output to its inactive state
                                                   when the type is changed to momentary to prevent the 
                                                   output being in the active state while the user is not 
                                                   pressing the button.
/SwitchableOutput/x/Settings/ValidTypes        <-- R / binary field where each bit corresponds to the 
                                                   enum of xx/type indicates which options the UI should 
                                                   provide to the user.

                                                   In case the output is not controllable, set /ValidTypes to 0 and invalidate /Type.
/SwitchableOutput/x/Settings/Function          <-- RW (optional) / Set the function of the digital output. The function is
                                                   currently only used with digital outputs (of type "toggle"),
                                                   so not with dimmable outputs.

                                                   Note that currently only the GX internal relays support
                                                   functions other than "Manual".
                                                   When the path is invalid or absent, the function is set to manual

                                                   0: Alarm
                                                   1: Generator start/stop
                                                   2: Manual, 
                                                   3: Tank pump
                                                   4: Temperature
                                                   5: Genset helper relay
                                                   6: Opportunity load

                                                   ES SmartSwitch: only manual
                                                   GX: builtin relays all options, second relay at 
                                                   least manual, alarm (new) and temperature
                                                   IO extender: only manual
                                                   BMV, smartsolar, other: only manual 
                                                   
/SwitchableOutput/x/Settings/ValidFunctions    <-- R / binary field where each bit corresponds 
                                                   to the enum of xx/function indicates which options the UI 
                                                   should provide to the user.
/SwitchableOutput/x/Settings/FuseRating        <-- RW (optional) Channel trip rating in amps; 
                                                   Preferably stored on the device. 
                                                   GetMin and GetMax are implemented to get the limits.
/SwitchableOutput/x/Settings/DimmingMin        <-- RW (optional) Only used for dimmable outputs. Defines
                                                   the minimum dimvalue. 0 will be used if omitted.
/SwitchableOutput/x/Settings/DimmingMax        <-- RW (optional) Only used for dimmable outputs. Defines
                                                   the maximum dimvalue. 100 will be used if omitted.
/SwitchableOutput/x/Settings/StepSize          <-- RW (optional) Only used for dimmable outputs. Defines 
                                                   the stepsize of the output. If not present, 
                                                   a stepsize of 1 should be used.
/SwitchableOutput/x/Settings/Decimals          <-- RW (optional) Only used for dimmable outputs. Defines the number 
                                                   of decimals to use when the GUI cannot accurately determine 
                                                   it from the stepsize path. Set this to enforce the number of decimals.
/SwitchableOutput/x/Settings/Unit              <-- RW (optional) Text field with the unit to display.
                                                   Only applicable to the Basic Slider (7) and Numeric input (8).

                                                   There are three units configurable by the user in Venus OS:
                                                   Speed (Knots, km/h, ..), Temperature (Celcius, Fahrenheit),
                                                   Volume (Litres, m3, ... ).

                                                   To all switches/energy-manager and so forth systems to have controls
                                                   in the units that are configured system wide by the user, we
                                                   introduce special texts that the UI control will recognize
                                                   and when used it will use the user configured unit:

                                                   1. Speed: keyword: "/Speed", base unit: m/s
                                                   2. Temperature: keyword: "/Temperature", base unit: Degrees Celsius
                                                   3. Volume: keyword: "/Volume", base unit: m3

                                                   The data (dimming, min, max, stepsize, actual value) is then to
                                                   be sent in the corresponding base unit.

                                                   For other use cases, when its not wanted, then there is still the
                                                   freedom that we had: path is set to which ever string/unit, and
                                                   data is sent is in the same unit.

                                                   Note that the temperature slider is already doing this, and doesn't
                                                   have this Settings/Unit path.
/SwitchableOutput/x/Settings/Polarity          <-- RW (optional) Polarity of the output.
                                                     0: Active high / Normally open
                                                     1: Active low / Normally closed
/SwitchableOutput/x/Settings/StartupState      <-- RW (optional) Defines the state of the output when the 
                                                   device is powered on.

                                                   0: Output off
                                                   1: Output on
                                                   2: Restore from memory (default)
/SwitchableOutput/x/Settings/StartupDimming    <-- RW (optional) Defines the dim level of a dimmable output
                                                   when the device is powered on.

                                                   0-100: Fixed value to be written at startup
                                                   -1: Restore from memory (default)
/SwitchableOutput/x/Settings/StartupState          RW (optional): Defines the initial state of the output
                                                   0: off
                                                   1: on
                                                   -1: restore last state from memory
/SwitchableOutput/x/Settings/DimCurve              RW (optional): Defines the dimming curve
                                                   0: Linear
                                                   1: Optical
/SwitchableOutput/x/Settings/OutputLimitMin        RW (optional): Float value between 0 and 100%. 4
                                                   PWM duty-cycle corresponding to 0% dimlevel
/SwitchableOutput/x/Settings/OutputLimitMax        RW (optional): Float value between 0 and 100%. 
                                                   PWM duty-cycle corresponding to 100% dimlevel
/SwitchableOutput/x/Settings/FuseDetection     <-- RW (optional) Set fuse detection mode on the DC Distribution board
                                                   0: Disabled
                                                   1: Enabled
                                                   2: Only when the output is off
/SwitchableOutput/x/Settings/Labels            <-- RW (optional) Define the labels of the multi-option switch.
                                                   For a multi-option switch, the `min` of the `/Dimming` path must be 0.
                                                   The `max` of the `/Dimming` path then defines the number of options.
                                                   The `../Labels/` path defines the labels of the presented options 
                                                   as a string array:
                                                   ["Label 1","Label 2", "Label 3"]
                                                   


