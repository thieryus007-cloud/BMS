# Intégration Toshiba SHORAI EDGE R32 — ESP32 + ESPHome (CN22 → MQTT → Mosquitto Pi5)

> **🚫 VOIE NON RETENUE (décision juillet 2026).** Le runtime ESP32 retenu est un
> **firmware Rust natif**, PAS ESPHome → **voir `docs/toshiba-suzumi-rs-plan.md`**
> (référence d'implémentation du projet). **Ce document reste utile en RÉFÉRENCE** pour
> ses parties **indépendantes du runtime**, toujours valables et réutilisées par la voie
> Rust : **§1 BOM, §2 brochage CN22, §3 câblage/sécurité, §5 schéma de topics MQTT
> `santuario/toshiba/<zone>`, §6 module Pi5 `logic/toshiba_ac`, §7 mesure conso
> Tongou/Tasmota**. **Ignorer §4 (YAML ESPHome)** — remplacé par le firmware Rust.
>
> **Statut** : plan de mise en œuvre (non déployé — système livré, pas encore installé).
> **Topologie** : **multi‑split** = **1 unité extérieure** (compresseur) + **3 unités
> intérieures**. Contrôle **local** (sans cloud Toshiba) via **un ESP32 par unité
> intérieure** branché sur le connecteur **CN22**, publiant en **MQTT** vers le broker
> Mosquitto du Pi5.
> **Protocole série** : identique quel que soit le runtime (ESPHome ou Rust) — spec
> vérifiée dans `docs/toshiba-suzumi-rs-plan.md` §6. Le Shorai Edge est couvert.
> **Mesure puissance/énergie** : **PAS de capteur sur les ESP32** (ni ACS712, ni PZEM,
> ni pince). Sur un multi‑split la conso est quasi‑totale à l'unité extérieure → elle
> est mesurée par le **switch Tongou (Tasmota) placé sur l'alim de l'unité extérieure**
> — voir §7. Les ESP32 ne font que le **contrôle/télémétrie climatique**.
>
> Contexte / comparatif des voies d'intégration (cloud KaSroka vs local ESP32 vs IR) :
> voir la discussion projet. Ce document ne traite **que** la voie ESP32/ESPHome locale.

---

## 0. Objectif & principe

Rendre les 3 climatiseurs **pilotables et observables** par l'`energy-manager` du Pi5,
**sans dépendance Internet ni cloud Toshiba**, en réutilisant l'infrastructure MQTT
existante (broker Mosquitto natif `mosquitto-broker.service`, `:1883`).

Chaque ESP32 se substitue au module Wi‑Fi Toshiba : il parle le **bus série TCC‑Link/AB**
de l'unité intérieure via CN22 (UART 9600 bauds, parité paire), expose une entité
`climate` ESPHome (mode, consigne, température ambiante, ventilation, capteurs de
diagnostic), et publie/souscrit en MQTT vers le Pi5. L'`energy-manager` consomme
ces topics (télémétrie → metrics-store/Grafana) et peut émettre des commandes
(logique solaire / effacement, sur le modèle `logic/water_heater` et `logic/deye_command`).

### Schéma de flux

```
CONTRÔLE CLIMATIQUE (×3 unités intérieures)      MESURE CONSO (×1 unité extérieure)
┌──────── Unité intérieure SHORAI EDGE (×3) ─┐   ┌──── Unité extérieure (compresseur) ────┐
│ Carte cmd ─ CN22 (JST PA 2.0, +5V/GND/TX/RX)│   │ Alim secteur ── Switch Tongou (Tasmota) │
└───────────────┬────────────────────────────┘   │   metering V/I/P/kWh ── relais on/off    │
                │  UART 5V, 9600 8E1               └───────────────┬─────────────────────────┘
        ┌───────▼────────┐                                         │ MQTT tele/{id}/SENSOR
        │ Level shifter  │ 5V ↔ 3.3V                               │ (power, today kWh…)
        └───────┬────────┘                                         │
        ┌───────▼────────┐   WiFi StarTh                           │
        │ ESP32-WROOM-32 │  (ESPHome + toshiba_suzumi)             │
        │ GPIO33=TX/32=RX│──────────┐ MQTT (climate)               │
        └────────────────┘          ▼                              ▼
                            Mosquitto Pi5 192.168.1.141:1883 ◄──────┘
                            santuario/toshiba/<zone>  +  tele/<tongou_ac>/SENSOR
                                           │
                            ┌──────────────┼───────────────┐
                            ▼              ▼               ▼
                     energy-manager   Grafana via      (optionnel)
                     logic/toshiba_ac daly-bms-server  VRM Victron
                     + logic/tasmota  → redb           heatpump.mqtt_n
                     (clim ↔ conso)
```

> ⚠️ **Ne PAS bridger** ces topics vers le NanoPi. Le bridge Mosquitto ne relaie
> que des topics `santuario/...` ciblés (`heatpump/#`, `switch/#`, `heat/+/venus`, …).
> Un préfixe **local** `santuario/toshiba/#` reste local. Valider après toute modif :
> `sudo /usr/local/bin/verify-no-loop.sh` (règle projet #11).

---

## 1. Nomenclature matérielle (BOM) — quantités pour 3 unités

Les 3 sous‑ensembles sont **identiques**. Prévoir **+1 de chaque en rechange**
(prototypage/casse) sur les petits composants.

| # | Composant | Réf. type / spéc. | Qté (3 u.) | Qté conseillée (+spare) | Notes |
|---|-----------|-------------------|:----------:|:-----------------------:|-------|
| 1 | Carte ESP32 | **ESP32‑WROOM‑32 DevKitC** (30/38 pins) | 3 | 4 | Régulateur 3V3 embarqué ; accepte 5V sur `VIN`/`5V`. GPIO32/33 = défaut du composant (routables). |
| 2 | Convertisseur de niveau logique | **bidirectionnel 4 canaux, BSS138** (ex. « 4‑channel logic level converter ») ou **TXS0108E** 8 canaux | 3 | 4 | 2 canaux utiles (TX, RX). BSS138 = open‑drain, OK pour UART 9600. |
| 3 | Connecteur d'accouplement CN22 | **Boîtier femelle JST PA 2.0 mm, 5 voies** + contacts à sertir, **ou** pigtail JST‑PA 2.0 5P pré‑serti (≈20 cm) | 3 | 4 | CN22 = embase mâle sur PCB. À défaut, récupérer sur un module Wi‑Fi tiers. **Vérifier le pas/format sur l'unité réelle avant achat.** |
| 4 | Fil de câblage | fil silicone AWG26–28 multibrin (rouge/noir/2 couleurs signal) | — | 1 bobine set | Liaison CN22↔shifter↔ESP32. |
| 5 | Support de montage | **PCB perforée** 3×7 cm **ou** petit PCB dédié | 3 | 4 | Fixer ESP32 + shifter + connecteur. |
| 6 | Boîtier | **boîtier ABS** ~60×40×20 mm | 3 | 3 | Isolation, fixation près de l'unité. |
| 7 | Câble USB données | micro‑USB (ou USB‑C selon devkit) | 1 | 1 | **Flash initial uniquement** (OTA ensuite). |
| 8 | Divers | dupont F/F, gaine thermo, ferrules, colliers, double‑face | — | 1 lot | — |
| 9 | *(option)* Alim 5V externe | mini‑alim USB 5V/1A + câble | 0 | 3 | **Repli** si la ligne +5V de CN22 provoque des brownouts WiFi (voir §3.3). |

**Alimentation** : par défaut, l'ESP32 est **alimenté par le +5V de CN22** (pin 3) →
un seul faisceau, pas d'alim externe. Le +5V alimente aussi le côté **HV** du level
shifter ; le côté **LV** est alimenté par la broche **3V3** de l'ESP32.

**Coût indicatif** (hors outillage) : ~12–18 € par unité → **~40–55 € pour 3** (+ spares).
Outillage éventuel : pince à sertir JST‑PA, fer à souder, multimètre.

---

## 2. Brochage CN22 (source : composant `esphome_toshiba_suzumi`)

Connecteur **JST PA 2.0 mm, 5 voies**. Vérifier sur l'unité réelle avant câblage.

| Pin | Couleur (typ.) | Fonction | Niveau | Vers |
|:---:|----------------|----------|:------:|------|
| 1 | Bleu | **TX** (unité → ESP) | 5V | Level shifter HV → RX ESP |
| 2 | Rose | **GND** | 0V | GND commun (ESP + shifter) |
| 3 | Noir | **+5V** (alim) | 5V | `VIN`/`5V` ESP + HV shifter |
| 4 | Blanc | **RX** (ESP → unité) | 5V | Level shifter HV ← TX ESP |
| 5 | Rose | **NE PAS CONNECTER** | — | — |

> ⚠️ **Pin 5 : ne jamais connecter.** Un mauvais raccordement risque d'endommager
> la carte de commande de l'unité intérieure. Les couleurs sont indicatives —
> **vérifier au multimètre** (repérer GND et +5V) avant de brancher l'ESP32.

---

## 3. Câblage détaillé (par unité)

### 3.1 UART via level shifter

| CN22 | Level shifter (HV) | Level shifter (LV) | ESP32 | Rôle |
|------|--------------------|--------------------|-------|------|
| Pin 3 (+5V) | `HV` + `VccA/HV` | — | `VIN`/`5V` | Alim 5V |
| Pin 2 (GND) | `GND` (HV) | `GND` (LV) | `GND` | Masse commune |
| — | — | `LV` ← 3V3 | `3V3` | Réf. basse tension |
| Pin 1 (TX unité) | `HVn` (canal A) | `LVn` (canal A) | **GPIO32 (RX)** | Unité → ESP |
| Pin 4 (RX unité) | `HVn` (canal B) | `LVn` (canal B) | **GPIO33 (TX)** | ESP → unité |

- **UART : 9600 bauds, 8 bits, parité PAIRE (EVEN), 1 stop (8E1).**
- GPIO32/33 sont les valeurs par défaut du composant ; l'ESP32 route l'UART sur
  presque n'importe quel GPIO → ajustables si besoin, garder cohérent avec le YAML.

### 3.2 Croisement TX/RX

Croisement classique : **TX unité → RX ESP** et **TX ESP → RX unité**. En cas
d'absence de trame reçue au 1er boot, inverser Pin1/Pin4 côté shifter (ou GPIO32/33
dans le YAML) — c'est l'erreur de câblage la plus fréquente.

### 3.3 Alimentation & robustesse

- Le +5V de CN22 alimente en général l'ESP32 sans souci. **Risque** : pics de
  courant WiFi de l'ESP32 (jusqu'à ~500 mA) → si l'unité redémarre, si le WiFi
  décroche ou si des brownouts apparaissent au boot, **basculer sur une alim 5V
  externe** (BOM #9), en gardant **GND commun** avec CN22 et en **ne connectant plus
  le +5V de CN22** à l'ESP.
- Ajouter un **condensateur électrolytique 470–1000 µF** entre 5V et GND près de
  l'ESP32 si instabilité.

### 3.4 Sécurité électrique (impératif)

1. **Couper l'alimentation secteur de l'unité intérieure** (disjoncteur) avant
   d'ouvrir le capot et de brancher/débrancher CN22 ou l'ESP32.
2. Ne **jamais** brancher/débrancher l'ESP32 sur CN22 unité sous tension.
3. **Pin 5 = interdite.** Double‑vérifier chaque liaison avant remise sous tension.
4. Un level shifter **est requis** (5V ↔ 3.3V) : brancher l'UART 5V direct sur un
   GPIO ESP32 (3.3V max) l'endommage.
5. **Garantie** : l'intervention sur CN22 peut affecter la garantie constructeur.
   À arbitrer avant installation (surtout matériel neuf).

---

## 4. Firmware ESP32 — ESPHome

### 4.1 Poste de build/flash

ESPHome peut tourner **sur le Pi5** (pip ou conteneur) ou sur un portable. Flash
initial en **USB** ; ensuite **OTA** (Wi‑Fi). Une installation ESPHome (CLI ou
add‑on) suffit pour les 3 nœuds.

### 4.2 Fichiers de configuration (1 par unité)

Nommage proposé, une **zone** par unité (à adapter) : `salon`, `chambre`, `bureau`.
Chaque nœud a un **hostname unique** et un **`topic_prefix` MQTT unique**.

`secrets.yaml` (commun, **non commité** — voir §8) :

```yaml
wifi_ssid: "StarTh"
wifi_password: "***"
mqtt_host: "192.168.1.141"   # broker Mosquitto Pi5
# mqtt_user / mqtt_password : NON requis (broker en allow_anonymous, cf. §5)
ota_password: "***"
```

`toshiba-salon.yaml` (dupliquer en `-chambre` / `-bureau` avec `name`,
`friendly_name`, `topic_prefix` différents) :

```yaml
esphome:
  name: toshiba-salon
  friendly_name: Toshiba Salon

esp32:
  board: esp32dev

logger:
  baud_rate: 0          # libère l'UART matériel pour le bus Toshiba

wifi:
  ssid: !secret wifi_ssid
  password: !secret wifi_password
  # IP statique conseillée (réservation ou manual) pour un parc stable
  # manual_ip:
  #   static_ip: 192.168.1.151
  #   gateway: 192.168.1.1
  #   subnet: 255.255.255.0

ota:
  platform: esphome
  password: !secret ota_password

mqtt:
  broker: !secret mqtt_host
  # username / password omis : broker en allow_anonymous (cf. §5)
  topic_prefix: santuario/toshiba/salon   # UNIQUE par unité
  discovery: false        # pas de Home Assistant ici → pas de topics homeassistant/#
  # birth/last-will par défaut d'ESPHome : .../status = online/offline

external_components:
  - source:
      type: git
      url: https://github.com/pedobry/esphome_toshiba_suzumi
    components: [toshiba_suzumi]

uart:
  id: uart_bus
  tx_pin: GPIO33
  rx_pin: GPIO32
  baud_rate: 9600
  parity: EVEN

climate:
  - platform: toshiba_suzumi
    name: "Climatiseur Salon"
    uart_id: uart_bus
    # Capteurs de diagnostic optionnels (auto‑remplis quand l'unité les émet) :
    # power_sensor:            { name: "Salon Puissance" }
    # current_sensor:          { name: "Salon Courant" }
    # outdoor_temp_sensor:     { name: "Salon Temp. extérieure" }
```

**À figer par unité** : `esphome.name`, `friendly_name`, `mqtt.topic_prefix`,
`climate.name`, IP statique. Tout le reste est identique.

### 4.3 Modèle & capteurs

- `toshiba_suzumi` supporte explicitement **Shorai Edge** (+ Suzumi Plus, Shorai
  Premium, Seiya, Daiseikai 9).
- Entité `climate` : modes (off/heat/cool/dry/fan/auto), consigne, **température
  ambiante**, ventilation, swing.
- Capteurs de diagnostic optionnels selon firmware : puissance/charge compresseur (%),
  courant, températures (refoulement/aspiration/évaporateur), régime ventilateur (RPM).
  À activer au besoin — utiles pour la logique énergie et Grafana.

---

## 5. MQTT — schéma de topics & Mosquitto

### 5.1 Broker

- Mosquitto Pi5 : `listener 1883 0.0.0.0`, **`allow_anonymous true`**
  (`contrib/mosquitto/mosquitto.conf`) → **aucun identifiant requis** pour les ESP32.
- Si une politique d'ACL est ajoutée plus tard, prévoir un utilisateur dédié
  `toshiba` + mot de passe et renseigner `mqtt.username/password` dans `secrets.yaml`.

### 5.2 Espace de nommage (préfixe LOCAL, non bridgé)

```
santuario/toshiba/salon/...      (unité 1)
santuario/toshiba/chambre/...    (unité 2)
santuario/toshiba/bureau/...     (unité 3)
```

Choisir un **préfixe distinct** de ceux relayés par le bridge NanoPi pour éviter
tout risque de boucle. `santuario/toshiba/#` **n'est pas** dans la liste `out` du
bridge (`docs/mqtt-mosquitto.md`) → il reste **local au Pi5**. ✅

> **Action de validation** : `discovery: false` supprime les topics `homeassistant/#`.
> Après le 1er boot, **énumérer les topics réels** publiés par ESPHome :
> `mosquitto_sub -h 192.168.1.141 -t 'santuario/toshiba/#' -v`
> puis **figer** dans le module `logic/toshiba_ac` les topics d'état (souscription)
> et de commande (publication). ESPHome/climate suit le schéma HA MQTT‑climate
> (sous‑topics `mode/state`, `mode/command`, `target_temperature/state|command`,
> `current_temperature/state`, `fan_mode/…`, `action/state`, `.../status`) — **le
> vérifier empiriquement** plutôt que le supposer.

### 5.3 Anti‑boucle

Après toute modif de topics/bridge : `sudo /usr/local/bin/verify-no-loop.sh` (règle #11).

---

## 6. Logiciel Pi5 — module `energy-manager` `logic/toshiba_ac`

> ✅ **Phase lecture seule IMPLÉMENTÉE** (2026-07) : `crates/energy-manager/src/logic/
> toshiba_ac/` (`mod.rs` + `rules.rs`), config `[energy_manager.toshiba_ac]`, souscription
> `santuario/toshiba/+/state`, remplissage `EnergyState.toshiba_ac[zone]` + event live.
> Tests verts, clippy propre, `--check-config` OK. **Phase contrôle** (`control_enabled`)
> = à faire (dépend de la stratégie énergie, §10). Détail/journal :
> `docs/toshiba-suzumi-rs-plan.md` §0.

Module calqué sur `logic/tasmota` (consommateur MQTT lecture seule) ; le futur contrôle
suivra `logic/deye_command` (règles **Rust pures, stateless**).

### 6.1 Arborescence

```
crates/energy-manager/src/
  logic/toshiba_ac/
    mod.rs      ← spawn(), tâches: parse état MQTT, publier commande, télémétrie
    rules.rs    ← décisions Rust pures + tests (effacement/solaire, si activé)
  config.rs     ← + ToshibaAcConfig (section [energy_manager.toshiba_ac])
  types.rs      ← + ToshibaAcSnapshot, ToshibaMode ; champs dans EnergyState
  mqtt/topics.rs← + souscriptions santuario/toshiba/<zone>/... + helpers publish
  main.rs       ← toshiba_ac::spawn(...) au démarrage séquentiel
```

### 6.2 Config (Config.toml)

```toml
[energy_manager.toshiba_ac]
enabled = true
# 3 unités : zone = segment de topic ESPHome (santuario/toshiba/<zone>)
units = [
  { zone = "salon",   venus_instance = 70 },
  { zone = "chambre", venus_instance = 71 },
  { zone = "bureau",  venus_instance = 72 },
]
# Logique d'effacement/solaire (optionnelle — désactivée par défaut) :
control_enabled       = false   # true = l'EM pilote les clim
mode_change_min_secs  = 300     # anti‑rebattement entre 2 commandes
# Exemples de garde‑fous (à définir §7) :
# solar_surplus_min_w = 1500
# soc_min_pct         = 80
# freq_high_hz        = 51.0    # cohérent avec la coupe DEYE
```

> Après modif : `sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart energy-manager`
> (règle #4). Valider en amont : `ENERGY_CONFIG=Config.toml energy-manager --check-config`.
> Une section manquante fait échouer le démarrage (`missing field ...`) → livrer la
> section **en même temps** que le binaire.

### 6.3 Comportement

1. **Phase lecture seule (recommandée d'abord)** : souscrire aux topics d'état,
   remplir `EnergyState` (mode, temp ambiante, consigne, puissance si dispo),
   republier en télémétrie interne (`santuario/em/toshiba_ac/<zone>` ou intégré à
   `em/metrics`) → écrit dans metrics‑store par daly‑bms‑server → Grafana.
2. **Phase contrôle (opt‑in `control_enabled`)** : `rules.rs::decide()` **pur et
   stateless** mappe (surplus solaire, SOC, fréquence AC, plage horaire) → commande
   `mode`/`consigne`, publiée sur le topic de commande ESPHome. Débounce
   `mode_change_min_secs`. Alignement possible avec la coupe DEYE (couper/limiter la
   clim quand `freq ≥ freq_high_hz`).

### 6.4 Règles projet à respecter

- **#16 supervision** : boucles longue durée via `spawn_critical` ; tâches one‑shot/timer
  **jamais** en `spawn_critical`.
- **#13 mesure prioritaire** : température ambiante/consigne viennent de l'unité (via
  ESP/CN22) ; **puissance & énergie viennent du Tongou‑Tasmota** (§7.1) — **ne jamais**
  recalculer/écraser ces mesures firmware.
- **#15 CI** : `clippy -D warnings`, tests des règles pures, cross‑build aarch64/armv7.
- **Robustesse source morte** (audit §18) : publier un
  `em_source_last_update_age_seconds{source="toshiba_salon"}` (âge > 5× l'intervalle
  = unité muette → ESP/UART/WiFi à vérifier).

---

## 7. Mesure puissance & énergie, métriques & Grafana

### 7.1 Mesure conso via le switch Tongou (Tasmota) existant — PAS de PZEM

**Décision (topologie multi‑split)** : la consommation étant quasi‑totale à l'unité
extérieure (compresseur + ventilo ext.), un **seul point de mesure sur l'alim de
l'unité extérieure** capture **tout le système AC** (compresseur + ventilo + les 3
unités intérieures, celles‑ci étant alimentées depuis l'extérieure sur un multi‑split).
Ce point de mesure est le **switch Tongou** placé sur l'alim de l'unité extérieure.
→ **Aucun capteur de courant sur les ESP32** (pas d'ACS712, pas de PZEM, pas de pince).

> **Fait projet à retenir** : **TOUS les switches Tongou actuellement en place sont du
> MÊME modèle** — des disjoncteurs/switchs intelligents **flashés Tasmota** qui
> **mesurent TOUT** : tension, courant, **puissance (W)**, **énergie (kWh)**, +
> protections. Ils sont tous visibles sur la **page Tasmota du dashboard**
> (`/dashboard/tasmota`, API `GET /api/v1/tasmota`). Le Tongou destiné à l'unité
> extérieure est **le même modèle** → il fournit nativement la puissance instantanée
> et l'énergie du jour, sans matériel supplémentaire.

**Le pipeline ingère déjà ces données.** `logic/tasmota` parse la télémétrie
`tele/{id}/SENSOR` d'un Tongou‑Tasmota et remonte exactement les deux besoins :

```rust
// crates/energy-manager/src/logic/tasmota/mod.rs — struct TasmotaEnergy
power:   Option<f64>,   // → puissance instantanée (W)
today:   Option<f64>,   // → énergie consommée DANS LA JOURNÉE (kWh)
voltage, current, total // + tension, courant, cumul total
```

Conforme à la **règle #13** : le Tongou fournit des **W réels mesurés** (pas un
`I × V` recalculé).

**Évolution de code nécessaire (petite)** : le module `logic/tasmota` ne gère
aujourd'hui **qu'un seul** appareil (le chauffe‑eau `tasmota_waterheater_id`,
p. ex. `tongou_3BC764`). Pour ingérer aussi le Tongou de la clim, le **généraliser à
plusieurs devices** (liste de `{id, role}` en config) — ou ajouter un handler dédié —
alimentant les champs `EnergyState` et les séries `toshiba_ac_power_w` /
`toshiba_ac_energy_today_kwh`. Pattern déjà en place, changement trivial.

**À confirmer à l'installation** :
- L'alim secteur arrive bien **sur l'unité extérieure** et les intérieures en sont
  alimentées (cas standard multi‑split) — sinon on manque les ventilos intérieurs (~négligeable).
- **Calibre** du Tongou suffisant pour le courant de l'unité extérieure (démarrage inclus).
- **ID Tasmota** du nouveau Tongou (topic `tele/<id>/SENSOR`) → à renseigner en config.

*Plan B (si ce Tongou n'était PAS à comptage / pas Tasmota)* : un **PZEM‑004T v3.0
unique** sur l'alim de l'unité extérieure, lu par un ESP32 (2ᵉ UART) — voir discussion
projet. Non nécessaire tant que le Tongou reste le modèle metering standard.

### 7.2 Séries & Grafana

- Écriture metrics‑store via daly‑bms‑server (déjà en place pour `em/*` et Tasmota).
- Séries proposées : `toshiba_ac_mode`, `toshiba_ac_current_temp_c`,
  `toshiba_ac_target_temp_c`, `toshiba_ac_power_w` (Tongou),
  `toshiba_ac_energy_today_kwh` (Tongou), `..._compressor_pct` (si télémétrie unité).
- Dashboard Grafana dédié (format **provisioning**, datasource UID `daly-metrics`,
  cf. règle #14) : mode/temp des 3 pièces, **puissance & conso du jour du système**,
  corrélation surplus PV / marche clim.

### 7.3 VRM Victron (optionnel, avancé)

Exposer les clim sur VRM via `santuario/heatpump/{n}/venus` →
`com.victronenergy.heatpump.mqtt_{n}` (bridge dbus‑mqtt‑venus). **Instances D‑Bus
libres uniquement** — 1 (chauffe‑eau LG), 8/9 (ET112), 151–153 (BMS), 20/30‑32/40/60‑65
sont pris. Utiliser p. ex. **heatpump.mqtt_2/3/4** (instances VRM ex. 70/71/72). À
traiter en dernier, une fois la boucle MQTT locale validée.

---

## 8. Étapes de mise en œuvre (par phases)

| Phase | Objet | Livrable / critère de sortie |
|:-----:|-------|------------------------------|
| **0** | Achats BOM (×3 + spares) | Composants reçus ; format CN22 confirmé sur unité réelle |
| **1** | Prototype **banc, 1 unité** | ESP32 flashé (USB), UART lit l'état : `mosquitto_sub -t 'santuario/toshiba/salon/#' -v` renvoie mode/temp |
| **2** | Contrôle banc | Commande MQTT change mode/consigne, retour d'état cohérent ; OTA OK |
| **3** | Install **unité #1** (dans l'unité intérieure) | Secteur coupé → câblage CN22 → boîtier → WiFi/MQTT stables 24 h, pas de brownout |
| **4** | Réplication **×3** | 3 nœuds `salon/chambre/bureau`, IP statiques, topics distincts |
| **5** | EM **lecture seule** | Module `logic/toshiba_ac` (télémétrie) ; séries visibles en base ; `--check-config` OK ; CI verte |
| **5b** | **Mesure conso Tongou** | Tongou (Tasmota) posé sur alim unité extérieure ; `tele/<id>/SENSOR` reçu ; `logic/tasmota` multi‑device ingère `power`+`today` → `toshiba_ac_power_w`/`_energy_today_kwh` ; visible sur `/dashboard/tasmota` |
| **6** | EM **contrôle** (opt‑in) | `rules.rs` + tests ; `control_enabled=true` ; débounce validé |
| **7** | Grafana (+ VRM opt.) | Dashboard provisionné ; (option) 3 heatpump D‑Bus sur VRM |

**Secrets / Git** : `secrets.yaml` ESPHome et `/etc/daly-bms/.env` **ne sont jamais
commités** (règle #12). Committer : YAML nœuds **sans secrets**, module Rust, section
`Config.toml`, dashboard. Convention de commit : `feat(toshiba):`.

---

## 9. Checklist de validation

**Matériel/ESP** : [ ] GND et +5V CN22 confirmés au multimètre ・ [ ] Pin5 non
connectée ・ [ ] level shifter HV=5V / LV=3V3 ・ [ ] pas de brownout au boot WiFi.

**MQTT** : [ ] topics `santuario/toshiba/<zone>/#` visibles ・ [ ] `santuario/toshiba/#`
**absent** du bridge (`verify-no-loop.sh`) ・ [ ] `.../status` bascule online/offline
(will) ・ [ ] commande → changement réel + retour d'état.

**EM/Pi5** : [ ] `energy-manager --check-config` OK ・ [ ] séries en base
(`/api/v1/redb/series`) ・ [ ] `em_source_last_update_age_seconds` borné ・ [ ]
`clippy -D warnings` + tests verts.

**Mesure conso (Tongou)** : [ ] Tongou de l'unité extérieure = même modèle metering
(V/I/P/kWh) ・ [ ] flashé Tasmota, `tele/<id>/SENSOR` publié ・ [ ] visible sur
`/dashboard/tasmota` ・ [ ] calibre ≥ courant unité extérieure ・ [ ] `power` et
`today` (kWh) non nuls sous charge.

---

## 10. Points ouverts (décisions à trancher)

**Déjà tranché** :
- **Topologie** = multi‑split (1 extérieure / 3 intérieures).
- **Mesure puissance/énergie** = via le **switch Tongou (Tasmota) de l'unité
  extérieure** (tous les Tongou en place sont le même modèle metering) → **pas de PZEM
  ni de capteur sur les ESP32** (§7.1). Reste à confirmer à l'install : calibre + ID
  Tasmota du Tongou + alim des intérieures depuis l'extérieure.

**À trancher** :
1. **Zones/nommage** des 3 unités (`salon/chambre/bureau` proposés).
2. **Schéma de topics final** : conserver le schéma ESPHome/HA natif, ou remapper via
   lambdas ESPHome vers `santuario/em/toshiba_ac/<n>/...` (plus homogène avec `em/*`) —
   **à figer après observation du 1er boot** (§5.2).
3. **Alimentation ESP32** : +5V CN22 (défaut) vs alim 5V externe (si instabilité).
4. **Stratégie énergie** : effacement sur fréquence AC (comme DEYE) et/ou marche sur
   surplus PV ? seuils SOC / surplus / plages horaires ?
5. **`logic/tasmota` multi‑device** : liste de devices en config vs handler dédié (§7.1).
6. **Exposition VRM** (heatpump.mqtt_2/3/4) : oui/non.
7. **Garantie** constructeur vs intervention CN22 (matériel neuf).

---

## 11. Références

- Composant ESPHome : `pedobry/esphome_toshiba_suzumi` — https://github.com/pedobry/esphome_toshiba_suzumi
- Contexte MQTT/bridge/anti‑boucle : `docs/mqtt-mosquitto.md`
- Pattern module EM (contrôle + télémétrie) : `crates/energy-manager/src/logic/water_heater/`
- Décision Rust pure/stateless : `crates/energy-manager/src/logic/deye_command/rules.rs`
- Config EM : `crates/energy-manager/src/config.rs`, section `[energy_manager]` de `Config.toml`
- **Mesure Tongou/Tasmota** : `crates/energy-manager/src/logic/tasmota/mod.rs` (parse `power`/`today`),
  page dashboard `/dashboard/tasmota` (API `GET /api/v1/tasmota`) — tous les Tongou = même modèle metering
- Conventions Grafana provisioning : `docs/grafana-dashboards.md`, règle projet #14
