# Plan de projet : Transposition du pilote Toshiba SUZUMI en Rust pour ESP32

> **Référence** : [pedobry/esphome_toshiba_suzumi](https://github.com/pedobry/esphome_toshiba_suzumi)
> **Objectif** : Remplacer le composant ESPHome (C++) par un firmware Rust autonome sur ESP32, sans dépendance à Home Assistant.
>
> **✅ DÉCISION ACTÉE (juillet 2026) — la voie retenue est un firmware RUST natif, PAS ESPHome.**
> Ce document est **la référence d'implémentation du projet** pour les 3 unités Toshiba
> Shorai Edge. ESPHome n'est **plus** la solution retenue : le document
> `docs/integration-toshiba-shorai-esphome.md` est **conservé uniquement en référence**
> pour ses parties **toujours valables** (câblage CN22, brochage, schéma de topics MQTT
> `santuario/toshiba/<zone>`, module Pi5 `energy-manager logic/toshiba_ac`, mesure conso
> via Tongou/Tasmota) — mais **le runtime ESP32 sera le firmware Rust décrit ici**, pas
> le composant ESPHome.
>
> **⚠️ Statut de la spécification** : le protocole ci‑dessous (§6, §8) a été
> **vérifié octet par octet contre le code source C++** de référence
> (`toshiba_climate.cpp`, `.h`, `toshiba_climate_mode.cpp`/`.h`, `climate.py` — pedobry)
> **et recoupé avec une seconde implémentation indépendante** (`o0Zz/climate-uart`,
> `src/protocols/toshiba.cpp` — voir §14). Les deux aboutissent au **même protocole** →
> confiance élevée. Les valeurs marquées « ✅ source » sont **extraites du firmware**.

---

## 0. État d'avancement & reprise de session

> 🔁 **NOUVELLE SESSION → LIRE CETTE SECTION EN PREMIER.** Ce document est la
> **mémoire du projet Toshiba** : il porte l'historique complet pour ne rien reperdre
> quand le contexte Claude est réinitialisé. Tenir ce §0 **à jour à chaque session**
> (journal + prochaines étapes), au même titre que CLAUDE.md (règle projet #9).

### 0.1 Contexte matériel

- **Les ESP32 ne sont PAS encore arrivés** → on progresse **sans aucun test matériel**.
  Tout ce qui est fait doit être **validable sur host** (`cargo test`) ou sur le papier.
- Dès réception du matériel : reprendre la Phase 3+ (UART/WiFi/MQTT) et les tests au banc.

### 0.2 Décisions actées

1. **Runtime = firmware Rust natif**, **PAS ESPHome** (voir bandeau en tête).
2. **Protocole SUZUMI vérifié** contre pedobry, **recoupé** avec o0Zz (§14) → fiable.
3. **`issalig`/AB TCC-Link = protocole différent, non applicable** à nos Shorai Edge (§15).
4. **Aucune spec officielle Toshiba** publique → §6 est la meilleure source (§15.1).

### 0.3 État du code

Crate : **`firmware/toshiba-suzumi-rs/`** — **détaché** du workspace Pi5 (`[workspace]`
vide → zéro impact sur le build/CI daly-bms). Tester : 
`cargo test --manifest-path firmware/toshiba-suzumi-rs/Cargo.toml`.

**34 tests host verts** au total (clippy `-D warnings` + rustfmt propres).

| Composant | Fichier | État |
|-----------|---------|------|
| Couche protocole pure (checksum, enums, trames, validation, parsing, ODU/IDU) | `src/protocol.rs` | ✅ **fait** (11 tests) |
| Handshake + timings + bornes température (constantes) | `src/protocol.rs` | ✅ fait |
| Agrégat d'état (`ToshibaState`, applique les `Field`) | `src/state.rs` | ✅ **fait** (4 tests) |
| Accumulateur de trames RX incrémental (`validate_message_`) | `src/framing.rs` | ✅ **fait** (5 tests) |
| Machine à états + file/pacing + watchdog + **simulateur d'unité** | `src/machine.rs` | ✅ **fait** (5 tests, dont handshake→online end-to-end) |
| Codec MQTT applicatif (`ToshibaState`→JSON ; JSON→`Command`→`apply_command`) | `src/mqtt_payload.rs` | ✅ **fait** (8 tests, transport-agnostique) |
| UART ESP-IDF (init 9600 8E1, lecture/écriture octets) | `src/uart.rs` | ⏳ **TODO (attend matériel)** |
| WiFi + MQTT transport (`santuario/toshiba/<zone>`) | `src/{wifi,mqtt}.rs` | ⏳ TODO (matériel) |
| `main.rs` + esp-idf-svc (feature `esp32`) | `src/main.rs` | ⏳ TODO (matériel) |

### 0.4 Prochaines étapes (ordre conseillé, sans matériel)

Faits (session 2026-07-05) : ✅ state machine (`machine.rs`), ✅ accumulateur RX
(`framing.rs`), ✅ agrégat d'état (`state.rs`), ✅ simulateur d'unité (test end-to-end).

Fait aussi (session 2026-07-05) : ✅ codec MQTT applicatif (`mqtt_payload.rs`) +
`Client::apply_command`.

Reste **faisable sans matériel** :
1. **Config** (`config.rs`) : zones/topics/pins/creds — struct + parsing (TOML/NVS),
   host-testable. **Figer le schéma de sous-topics** (aligné `logic/toshiba_ac`, §5.2 de
   l'autre doc) reste à trancher — idéalement confirmé au 1er boot (pas de matériel).

Puis **à réception du matériel** :
2. `uart.rs` (ESP-IDF, 9600 8E1, lecture/écriture octets) — branche `on_rx_byte`/`poll_tx`.
3. `wifi.rs` + `mqtt.rs` (transport esp-idf-svc) + `main.rs` (feature `esp32`).
4. Tests au banc : handshake réel, analyseur logique, commandes bout-en-bout.

### 0.5 Journal des sessions

- **2026-07-05** — Correction complète du protocole §6/§8 contre pedobry (start byte
  `0x02`, codes réels, checksum, handshake, temp, polling 120 s). Validation croisée
  o0Zz (§14) : même protocole ; anomalie `HANDSHAKE[4]` (0xFE vs 0xFB) documentée.
  Clarification paysage protocoles Toshiba (§15) : issalig/AB non applicable.
  **Décision Rust actée.** Bootstrap crate `firmware/toshiba-suzumi-rs/` : couche
  protocole pure `src/protocol.rs` + 11 tests host verts (clippy propre).
- **2026-07-05 (suite)** — Ajout des couches **logiques pures host-testables** :
  `state.rs` (agrégat `ToshibaState`), `framing.rs` (accumulateur RX incrémental),
  `machine.rs` (client : séquence handshake, pacing 100 ms, file de commandes,
  watchdog de re-handshake) **+ simulateur d'unité en mémoire** → test **end-to-end**
  handshake→online→commande sans matériel. **25 tests verts** au total, clippy + fmt OK.
- **2026-07-05 (suite 2)** — `mqtt_payload.rs` : codec **transport-agnostique** (état
  `ToshibaState` → JSON télémétrie ; JSON de commande → `Vec<Command>` → `Client::
  apply_command`), mappings chaîne↔enum stables. Ajout dep `serde`/`serde_json` (std,
  OK sur ESP-IDF). Test end-to-end JSON→unité. **34 tests verts**, clippy + fmt OK.

---

## 1. Objectifs du projet

- **Remplacer** le composant ESPHome C++ par un firmware Rust autonome sur ESP32.
- **Communiquer** avec le climatiseur Toshiba via le connecteur UART CN22 (protocole propriétaire SUZUMI).
- **Publier** l'état du climatiseur sur un broker MQTT (JSON structuré).
- **Recevoir** des commandes MQTT pour piloter le climatiseur (mode, température, puissance, etc.).
- **Fonctionner** sans dépendance à Home Assistant ni à tout autre système central — le broker MQTT est le seul point de contact.
- **Exposer** les capteurs de diagnostic optionnels (ODU/IDU) si supportés par l'unité.

---

## 2. Architecture fonctionnelle

```
┌─────────────┐     UART (CN22)     ┌─────────────┐
│ Climatiseur │ ◄──────────────────►│   ESP32     │
│   Toshiba   │   9600 baud, EVEN     │  (Rust)     │
│   (5V TTL)  │   5V ↔ 3.3V level   │             │
└─────────────┘                     └──────┬──────┘
                                           │ WiFi / Ethernet
                                     ┌─────▼─────┐
                                     │ Broker    │
                                     │  MQTT     │
                                     └───────────┘
```

L'ESP32 agit comme **passerelle protocolaire** :
- Il traduit les commandes MQTT en trames UART Toshiba (protocole SUZUMI).
- Il traduit les trames UART reçues en messages MQTT (état, température, diagnostics).
- Il gère la séquence de handshake initiale et la reconnexion automatique en cas de perte de communication.

---

## 3. Prérequis matériels

### 3.1 Composants nécessaires

| Élément | Référence / Conseil |
|---------|---------------------|
| **ESP32** | DevKitC, WROOM-32, ou tout module avec 2 UART disponibles (UART0 pour debug, UART1/2 pour CN22) |
| **Convertisseur de niveau** | Bidirectionnel 5V ↔ 3.3V (ex: TXB0108, ou simple diviseur résistif + diode Zener en réception) |
| **Alimentation** | Le climatiseur fournit du 5V sur CN22 broche 3. **Vérifier** que l'ESP32 supporte 5V sur VIN, sinon utiliser un régulateur 3.3V (AMS1117-3.3) |
| **Câblage** | 4 fils minimum (TX, RX, GND, 5V) + résistance de pull-up sur RX si niveau 5V |

### 3.2 Connexions CN22 → ESP32

Connecteur **JST PA 2.0 mm, 5 voies**. Le composant ESPHome utilise par défaut
`tx_pin: GPIO33` / `rx_pin: GPIO32` — on garde cette convention pour rester cohérent
avec `docs/integration-toshiba-shorai-esphome.md`.

| CN22 (broche) | Couleur (typ.) | Fonction (côté unité) | ESP32 GPIO | Remarque |
|:-------------:|:--------------:|:----------------------|:----------:|:---------|
| 1 | Bleu | **TX unité** → RX ESP | **GPIO 32 (RX ESP)** | Via level-shifter 5V→3.3V |
| 2 | Rose | GND | GND | Commun |
| 3 | Noir | +5V (alim) | VIN (ou 5V) | Vérifier la capacité de sortie du CN22 (typ. 100-200 mA) |
| 4 | Blanc | **RX unité** ← TX ESP | **GPIO 33 (TX ESP)** | Via level-shifter 3.3V→5V |
| 5 | Rose | **NE PAS CONNECTER** | — | Risque d'endommager la carte de commande |

> **⚠️ Attention critique** :
> 1. Les lignes TX/RX du CN22 sont en **5V TTL**, l'ESP32 en **3.3V** → level-shifter
>    bidirectionnel **obligatoire** sous peine de destruction du GPIO.
> 2. La parité UART est **EVEN** (jamais `None` — le piège n°1, cf. §12).
> 3. **Broche 5 : ne jamais connecter.**
> 4. **Croisement / sens des fils à vérifier au banc.** La seule invariance électrique
>    est le **croisement UART** : `ESP_TX → unité_RX` et `ESP_RX ← unité_TX`.
>    L'association *couleur ↔ GPIO* diffère selon les sources (le README pedobry associe
>    Bleu↔GPIO33 et Blanc↔GPIO32 ; le tableau ci‑dessus suit le sens *fonctionnel*
>    unité‑TX→ESP‑RX). **Si aucune trame n'est reçue au 1er boot, inverser les deux fils
>    de signal** (ou permuter `tx_pin`/`rx_pin`) — c'est l'erreur de câblage la plus
>    fréquente. Repérer GND et +5V **au multimètre** avant tout branchement.

---

## 4. Structure du projet Rust

Emplacement réel : **`firmware/toshiba-suzumi-rs/`** (crate **détaché** du workspace
Pi5). Légende : ✅ fait · ⏳ TODO (attend le matériel).

```
firmware/toshiba-suzumi-rs/
├── Cargo.toml                   # ✅ crate détaché ([workspace] vide), lib pure, 0 dép.
├── .gitignore                   # ✅ ignore /target
├── src/
│   ├── lib.rs                   # ✅ expose protocol / state / framing / machine
│   ├── protocol.rs              # ✅ couche PURE : checksum, enums, trames, validation,
│   │                            #    parsing (+ ODU/IDU), handshake, timings — 11 tests
│   ├── state.rs                 # ✅ ToshibaState (agrégat, applique les Field) — 4 tests
│   ├── framing.rs               # ✅ accumulateur RX incrémental (validate_message_) — 5 tests
│   ├── machine.rs               # ✅ Client : handshake, pacing, file, watchdog
│   │                            #    + simulateur d'unité (test end-to-end) — 5 tests
│   ├── mqtt_payload.rs          # ✅ état→JSON ; JSON→Command→apply_command — 8 tests
│   ├── config.rs                # ⏳ Configuration (zones/topics/pins ; NVS)
│   ├── uart.rs                  # ⏳ UART ESP-IDF (9600 8E1) → on_rx_byte / poll_tx
│   ├── wifi.rs                  # ⏳ WiFi station (connexion, reconnexion)
│   ├── mqtt.rs                  # ⏳ Transport MQTT (santuario/toshiba/<zone>)
│   └── main.rs                  # ⏳ Point d'entrée ESP32 (feature `esp32`, esp-idf-svc)
├── .cargo/config.toml           # ⏳ cible xtensa-esp32-espidf (ajouté avec la partie ESP32)
└── build.rs / sdkconfig.defaults / partitions.csv   # ⏳ ESP-IDF (OTA + NVS)
```

> **Séparation I/O ↔ logique** (inspirée de o0Zz, §14.3) : `protocol`/`state`/`framing`/
> `machine` sont **purs** (aucune I/O) → testés sur host. Le futur code ESP-IDF
> (`uart`/`wifi`/`mqtt`/`main`) ne fera **que** relayer : lire l'UART → `Client::on_rx_byte`,
> écrire ← `Client::poll_tx`, timer → `Client::on_tick`. Cette partie sera **derrière une
> feature `esp32`** pour garder la couche pure compilable partout.

---

## 5. Dépendances Rust (Cargo.toml)

```toml
[package]
name = "toshiba-suzumi-rs"
version = "0.1.0"
edition = "2021"
resolver = "2"

[dependencies]
# HAL & système ESP-IDF
esp-idf-svc = { version = "0.49", features = ["alloc", "native"] }
esp-idf-hal = "0.44"
esp-idf-sys = "0.35"

# Async runtime (optionnel mais recommandé pour MQTT + UART concurrents)
embassy-sync = "0.6"
embassy-futures = "0.1"

# MQTT client (rust-mqtt ou esp-mqtt)
# Option A : esp-idf-svc intègre déjà un client MQTT via esp-mqtt-dispatcher
# Option B : crate mqtt externe si besoin de QoS 2 avancé

# Serialization JSON pour payloads MQTT
serde = { version = "1.0", default-features = false, features = ["derive"] }
serde-json-core = "0.6"          # Version no-std compatible heap limité

# Logging & diagnostic
log = "0.4"                      # EspLogger fourni par esp-idf-svc (esp_idf_svc::log::EspLogger)

# Utilitaires no-std
heapless = "0.8"                 # Vec/String statiques (pas d'allocateur requis)
nb = "1.1"                       # Non-blocking traits

[dev-dependencies]
# Tests host-side (pas de cross-compilation)
serde_json = "1.0"

[[bin]]
name = "toshiba-suzumi-rs"
harness = false                  # Pas de libtest embarqué (sauvegarde RAM/Flash)

[profile.release]
opt-level = 3
lto = true
```

---

## 6. Spécification du protocole SUZUMI

### 6.1 Paramètres UART

| Paramètre | Valeur | Commentaire |
|-----------|--------|-------------|
| Baudrate | **9600** | Confirmé sur toutes les unités SUZUMI/SHORAI/SEIYA |
| Data bits | 8 | |
| Parité | **EVEN** | ⚠️ Erreur fréquente : ce n'est PAS `None` |
| Stop bits | 1 | |
| Flow control | Aucun | |

### 6.2 Format général d'une trame ✅ source

**Toutes les trames** (émission comme réception) commencent par l'en‑tête **`0x02`**
(STX). Le firmware construit deux gabarits, tous deux avec un **préfixe fixe** puis le
type de commande, une valeur optionnelle et un checksum final.

**Trame de commande (écriture)** — 15 octets, produite par `sendCmd()` :

```
Index :  0    1  2  3   4  5  6  7   8  9 10 11    12    13    14
Octet : 02   00 03 10  00 00 07 01  30 01 00 02  <cmd> <val> <cks>
        └──────── préfixe fixe (12 octets) ────┘   │     │     └ checksum
                        [6]=07 (len)  [11]=02 (write)│     └ valeur (uint8)
                                                     └ cmd (ToshibaCommandType)
```

**Trame de requête (lecture)** — 14 octets, produite par `requestData()` :

```
Index :  0    1  2  3   4  5  6  7   8  9 10 11    12    13
Octet : 02   00 03 10  00 00 06 01  30 01 00 01  <cmd> <cks>
        └──────── préfixe fixe (12 octets) ────┘   │     └ checksum
                        [6]=06 (len)  [11]=01 (read)└ cmd    (pas d'octet valeur)
```

**Sémantique des champs** (déduite de `validate_message_` + `sendCmd`/`requestData`) :

| Octet | Rôle |
|:-----:|------|
| `[0]` | **En‑tête STX = `0x02`** (constant, vérifié à la réception) |
| `[2]` | Toujours `0x03` (vérifié à la réception) |
| `[6]` | **Longueur du payload** : le récepteur en déduit l'**index du checksum** `cks_idx = 6 + data[6] + 1` (donc taille totale = `cks_idx + 1`). Écriture : `[6]=07` → checksum en `[14]`, trame 15 o. Lecture : `[6]=06` → checksum en `[13]`, trame 14 o. |
| `[11]` | **Classe** : `0x01` = lecture/requête, `0x02` = écriture/commande |
| `[12]` | **Type** = `ToshibaCommandType` (§6.5) |
| `[13]` | Valeur (écriture) **ou** checksum (lecture) |
| dernier | **Checksum** (§6.3) |

> Les octets `[1]=00 [3]=10 [4..5]=00 00 [7]=01 [8]=30 [9]=01 [10]=00` sont **constants**
> dans les deux gabarits (adressage/canal interne du bus TCC). Reproduire tels quels.

### 6.3 Checksum ✅ source

Le C++ d'origine :

```cpp
uint8_t checksum(std::vector<uint8_t> data, uint8_t length) {
  uint8_t sum = 0;
  for (size_t i = 1; i < length; i++) sum += data[i];   // exclut data[0]=0x02
  return 256 - sum;                                      // complément à deux (u8)
}
```

Algorithme exact : **somme (mod 256) de tous les octets à partir de l'index 1**
(l'en‑tête `0x02` est exclu ; le checksum lui‑même **n'est pas encore présent** au
moment du calcul), puis **complément à deux** (`256 − somme`, tronqué à 8 bits).

En Rust — `frame` est la trame **sans** l'octet de checksum final :

```rust
/// Checksum SUZUMI. `frame` = tous les octets AVANT le checksum (en-tête 0x02 inclus).
/// Équivaut à `(256 - Σ frame[1..]) mod 256`, soit la négation en complément à deux.
pub fn compute_checksum(frame: &[u8]) -> u8 {
    frame[1..]
        .iter()
        .fold(0u8, |acc, &b| acc.wrapping_add(b))
        .wrapping_neg() // 0u8.wrapping_sub(sum) == 256 - sum (mod 256)
}
```

> ⚠️ À la **réception**, le firmware appelle `checksum(rx, at)` où `at` = index de
> l'octet de checksum → il somme `rx[1..at]` (donc hors en‑tête **et** hors checksum),
> ce qui est cohérent avec le calcul ci‑dessus.

### 6.4 Séquence de handshake (boot / reconnexion) ✅ source

Le climatiseur **ne répond pas** aux requêtes tant que le handshake n'est pas établi.
La séquence est critique. Les **8 trames sont des constantes pré‑calculées** (checksum
déjà inclus) — les émettre **telles quelles**, ne rien recalculer.

**Trames `HANDSHAKE[6]`** (octets décimaux, du firmware) :

```
HANDSHAKE[0] = {2, 255, 255, 0,   0, 0, 0,   2}
HANDSHAKE[1] = {2, 255, 255, 1,   0, 0, 1,   2, 254}
HANDSHAKE[2] = {2, 0,   0,   0,   0, 0, 2, 2, 2, 250}
HANDSHAKE[3] = {2, 0,   1,   129, 1, 0, 2, 0, 0, 123}
HANDSHAKE[4] = {2, 0,   1,   2,   0, 0, 2, 0, 0, 254}  // ⚠️ 254=0xFE (pedobry, éprouvé) ; alt. checksum-correct = 251=0xFB (o0Zz) — cf. note ci-dessous
HANDSHAKE[5] = {2, 0,   2,   0,   0, 0, 0, 254}
```

**Trames `AFTER_HANDSHAKE[2]`** :

```
AFTER_HANDSHAKE[0] = {2, 0, 2, 1, 0, 0, 2, 0, 0, 251}
AFTER_HANDSHAKE[1] = {2, 0, 2, 2, 0, 0, 2, 0, 0, 250}
```

> ⚠️ **Divergence `HANDSHAKE[4]` (dernier octet)** — repérée en recoupant avec
> `o0Zz/climate-uart` (§14). pedobry termine cette trame par **`254` (0xFE)**, mais la
> **règle de checksum donne `251` (0xFB)** (Σ index 1..8 = 5 → 256−5 = 251) — c'est la
> valeur qu'utilise o0Zz (`kSyn5`). Les **7 autres trames sont checksum‑correctes** ; seule
> celle‑ci ne l'est pas côté pedobry. Comme le firmware pedobry **fonctionne sur le terrain
> avec 0xFE**, l'unité **ne valide probablement pas** le checksum des trames de handshake
> (init figées). **Recommandation** : garder les octets pedobry tels quels pour la parité,
> mais si le handshake échoue au banc, **tester `0xFB`**.

**Déroulé exact** (`setup()` → `start_handshake()` puis `getInitData()`) :

```
1. Envoyer HANDSHAKE[0..5]  (6 trames)
2. Attendre 2000 ms          (trame interne DELAY dans la file)
3. Envoyer AFTER_HANDSHAKE[0..1] (2 trames)
4. getInitData() : enfiler 10 requêtes de lecture (§6.4.1)
5. set_wifi_led(...) : LED WiFi de l'unité (ON/OFF, §6.5)
```

**Cadence d'émission** : la file de commandes est vidée par `process_command_queue_()`
avec un **délai minimal de `COMMAND_DELAY = 100 ms` entre deux trames** (et **jamais**
pendant qu'une trame est en cours de réception — `rx_message_` doit être vide). Le
`2000 ms` de l'étape 2 est réalisé par une commande spéciale `DELAY` insérée dans la
file (pas un `sleep` bloquant). → ce n'est **pas** 50 ms comme supposé initialement.

#### 6.4.1 Données initiales `getInitData()`

Après le handshake, le firmware demande **10 valeurs** (une requête de lecture chacune) :

```
POWER_STATE, [SELF_CLEAN si capteur], MODE, TARGET_TEMP, FAN,
POWER_SEL, SWING, ROOM_TEMP, OUTDOOR_TEMP, SPECIAL_MODE
```

> **Note reconnexion** : si l'unité est coupée/rallumée ou le câble débranché, **rejouer
> le handshake complet** puis `getInitData()`. Un watchdog logiciel (aucune trame reçue
> pendant N secondes) doit forcer ce re‑handshake (§5 state-machine).

### 6.5 Types de commandes `ToshibaCommandType` ✅ source

Valeurs **exactes** (`toshiba_climate_mode.h`). Elles servent à la fois de champ `[12]`
en émission (lecture ou écriture) et d'identifiant du capteur/réglage en réception.

```rust
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmdType {
    Handshake    = 0,    // 0x00 — trames de handshake (interne)
    Delay        = 1,    // 0x01 — pseudo-commande d'attente (interne file)
    PowerState   = 128,  // 0x80 — ON/OFF (valeurs STATE, §6.8)
    PowerSel     = 135,  // 0x87 — niveau de puissance 50/75/100 % (PWR_LEVEL)
    ComfortSleep = 148,  // 0x94 — déclaré dans l'enum mais NON utilisé par le firmware
                         //        (valeurs/sémantique inconnues — à caractériser au scan)
    Fan          = 160,  // 0xA0 — ventilation (FAN)
    Swing        = 163,  // 0xA3 — orientation (SWING)
    Mode         = 176,  // 0xB0 — mode CVC (MODE)
    TargetTemp   = 179,  // 0xB3 — consigne °C (offset +16 si mode 8°, §6.8)
    RoomTemp     = 187,  // 0xBB — température ambiante mesurée (lecture)
    OutdoorTemp  = 190,  // 0xBE — température extérieure (lecture ; 127 = invalide)
    WifiLed1     = 222,  // 0xDE — LED WiFi (octet 1)
    WifiLed2     = 223,  // 0xDF — LED WiFi (octet 2)
    SelfClean    = 203,  // 0xCB — auto-nettoyage (SELF_CLEAN_STATE)
    SpecialMode  = 247,  // 0xF7 — préréglage/spécial (SPECIAL_MODE)
    IduStatus    = 228,  // 0xE4 — trame étendue unité intérieure (§6.8)
    OduStatus    = 229,  // 0xE5 — trame étendue unité extérieure (§6.8)
}
```

> **`scan()`** (débogage) : le firmware balaye `requestData(i)` pour `i` de **128 à 254**
> afin de découvrir les capteurs d'un modèle non répertorié. À reproduire pour une
> commande MQTT `santuario/toshiba/<zone>/scan` (cf. §12, point 6).

### 6.6 Modes de fonctionnement `MODE` ✅ source

⚠️ **Il n'existe pas de valeur de mode « OFF »** : l'arrêt passe par `PowerState`
(`STATE::OFF`), pas par le champ MODE. Le « mode auto » = `HEAT_COOL`.

| Valeur (déc / hex) | `MODE` | Mode ESPHome | Description |
|:------------------:|--------|--------------|-------------|
| 65 / 0x41 | `HEAT_COOL` | `heat_cool` | Automatique (chaud/froid) |
| 66 / 0x42 | `COOL` | `cool` | Refroidissement |
| 67 / 0x43 | `HEAT` | `heat` | Chauffage |
| 68 / 0x44 | `DRY` | `dry` | Déshumidification |
| 69 / 0x45 | `FAN_ONLY` | `fan_only` | Ventilation seule |

Marche/arrêt via `PowerState` : `STATE::ON = 48 (0x30)`, `STATE::OFF = 49 (0x31)`.

### 6.7 Préréglages `SPECIAL_MODE` ✅ source

| Valeur (déc) | `SPECIAL_MODE` | Nom (chaîne ESPHome) |
|:------------:|----------------|----------------------|
| 0  | `STANDARD`    | `Standard` |
| 1  | `HI_POWER`    | `Hi POWER` |
| 2  | `SILENT_1`    | `Silent#1` |
| 3  | `ECO`         | `ECO` |
| 4  | `EIGHT_DEG`   | `8 degrees` (garde hors‑gel) |
| 5  | `SLEEP`       | `Sleep` |
| 6  | `FLOOR`       | `Floor` |
| 7  | `COMFORT`     | `Comfort` |
| 10 | `SILENT_2`    | `Silent#2` |
| 32 | `FIREPLACE_1` | `Fireplace 1` |
| 48 | `FIREPLACE_2` | `Fireplace 2` |

> Les **valeurs ne sont pas contiguës** (0,1,2,3,4,5,6,7,10,32,48) → utiliser une table
> explicite, **jamais** un `preset as u8` séquentiel. La liste des presets réellement
> exposés est configurable côté ESPHome (`supported_presets`).

### 6.8 Enums de valeurs `FAN` / `SWING` / `STATE` / `PWR_LEVEL` ✅ source

```rust
#[repr(u8)] enum Fan { Quiet=49, Low=50, Mode2=51, Medium=52, Mode4=53, High=54, Auto=65 }
//                     (Mode2 = « Low-Medium », Mode4 = « Medium-High » : modes custom)
#[repr(u8)] enum Swing {
    Off=49, Vertical=65, Horizontal=66, Both=67,
    VFix1=80, VFix2=81, VFix3=82, VFix4=83, VFix5=84, // positions verticales fixes
}
#[repr(u8)] enum State { On=48, Off=49 }
#[repr(u8)] enum PwrLevel { Pct50=50, Pct75=75, Pct100=100 }
#[repr(u8)] enum SelfClean { Running=0x18, Off=0x10 }
```

Le select « Vertical Air Direction » ESPHome mappe :
`Off/Swing/Top/Middle Top/Middle/Middle Bottom/Bottom` → `Off/Vertical/VFix1..VFix5`.

### 6.9 Parsing des réponses `parseResponse()` ✅ source

Le récepteur **accumule octet par octet** (`handle_rx_byte_`) et valide de façon
incrémentale (`validate_message_` : en‑tête `0x02`, `[2]==0x03`, longueur via `[6]`,
checksum). Une trame complète est ensuite dispatchée **selon sa longueur totale** :

| Longueur | Signification | Type en `[?]` | Valeur en `[?]` |
|:--------:|---------------|:-------------:|:---------------:|
| **15** | réponse à une lecture | `[12]` | `[13]` |
| **16** | **ACK** de commande | — | *(ignorée)* |
| **17** | réponse à une lecture (variante) | `[14]` | `[15]` |
| **22** | trame étendue ODU/IDU | `[12]` | offset 13 |
| **24** | trame étendue ODU/IDU (variante) | `[14]` | offset 15 |

Décodage par type :

| Type | Décodage |
|------|----------|
| `TargetTemp` | consigne = `value` (°C). **Si mode `EIGHT_DEG` actif : `value -= 16`** |
| `RoomTemp` | temp ambiante = `value` (°C, `int8`) ; **127 = invalide** |
| `OutdoorTemp` | temp ext. = `value as i8` ; **127 = invalide** |
| `Mode` | `MODE` (§6.6) ; n'écrase le mode que si l'unité est ON et pas en auto‑nettoyage |
| `Fan` | `FAN` (§6.8) |
| `Swing` | `SWING` (§6.8) |
| `PowerState` | `STATE` : OFF → mode `off` ; ON → redemande `Mode`/`SelfClean` |
| `PowerSel` | `PWR_LEVEL` |
| `SpecialMode` | `SPECIAL_MODE` (§6.7) → preset courant |
| `SelfClean` | `SELF_CLEAN` : `RUNNING`=0x18 (mode affiché `off`) / `OFF`=0x10 |

### 6.10 Capteurs de diagnostic ODU / IDU (optionnels) ✅ source

Trames **étendues** (`OduStatus=0xE5`, `IduStatus=0xE4`), longueur 22 ou 24.
**Offset de base** : `off = 13` si longueur 22, `off = 15` si longueur 24.

**ODU (`0xE5`)** :

| Capteur | Octet | Unité | Décodage |
|---------|:-----:|:-----:|----------|
| `cdu_td_temp` | `off+0` | °C | `int8` (tube de refoulement) |
| `cdu_ts_temp` | `off+1` | °C | `int8` (tube d'aspiration) |
| `cdu_te_temp` | `off+2` | °C | `int8` (évaporateur ODU) |
| `cdu_load` | `off+3` | % | **`value / 1.7`** (charge compresseur) |
| `cdu_iac` | `off+6` | A | `value` (courant compresseur) |

**IDU (`0xE4`)** :

| Capteur | Octet | Unité | Décodage |
|---------|:-----:|:-----:|----------|
| `fcu_tc_temp` | `off+0` | °C | `int8` (échangeur IDU) |
| `fcu_tcj_temp` | `off+1` | °C | `int8` (jonction échangeur IDU) |
| `fcu_fan_rpm` | `off+2` | RPM | `value` (ventilateur IDU) |

> **Comportement d'émission (à corriger vs. supposition initiale)** : le composant est un
> **PollingComponent** — `update()` (par défaut **toutes les 120 s**) redemande activement
> `RoomTemp` (+ `OutdoorTemp` si capteur, + `SelfClean` si actif). Les trames ODU/IDU et
> les changements faits **à la télécommande** arrivent de façon **non sollicitée**. Il ne
> faut donc **pas** compter sur un push « toutes les 1‑2 s » : prévoir un polling
> périodique **et** l'écoute des pushes.

### 6.11 Encodage de la consigne de température ✅ source

- **Mode standard** : consigne = entier °C, **plage 17–30 °C** (`MIN_TEMP_STANDARD=17`,
  `MAX_TEMP=30`). Pas de demi‑degrés.
- **Mode 8° (garde hors‑gel `EIGHT_DEG`)** : plage **5–13 °C** (défaut 8). Bascule
  **automatique** : demander une consigne `< 17` fait passer en `EIGHT_DEG` ; `≥ 17`
  revient en `STANDARD`. **Sur le fil, la valeur est décalée de +16** (ex. 8 °C → octet
  `24`) à l'émission, et `−16` à la réception.
- `set_wifi_led(on)` (LED WiFi de l'unité) : ON = `WifiLed1=0x05` puis `WifiLed2=0x00` ;
  OFF = `WifiLed1=0x00` puis `WifiLed2=0x80`.

---

## 7. Étapes détaillées de développement

### Phase 1 : Environnement et squelettage (1h)

1. Installer `espup` et `cargo-generate`.
2. Générer le projet : `cargo generate esp-rs/esp-idf-template`.
3. Configurer `.cargo/config.toml` pour la cible `xtensa-esp32-espidf`.
4. Ajouter les dépendances dans `Cargo.toml` (voir §5).
5. Configurer `sdkconfig.defaults` :
   ```
   CONFIG_ESP_MAIN_TASK_STACK_SIZE=8192
   CONFIG_ESP_SYSTEM_EVENT_TASK_STACK_SIZE=2048
   CONFIG_FREERTOS_UNICORE=n
   CONFIG_UART_ISR_IN_IRAM=y
   CONFIG_MBEDTLS_DYNAMIC_BUFFER=1
   ```
6. Vérifier la compilation : `cargo build --release`.

### Phase 2 : Transposition du protocole (3h)

1. **Reprendre les constantes** documentées : `HANDSHAKE[6]` + `AFTER_HANDSHAKE[2]`
   (§6.4), préfixes fixes des trames lecture/écriture (§6.2), enums (§6.5–6.8).
2. **Implémenter `compute_checksum`** (`wrapping_neg`, voir §6.3/§8.1).
3. **Implémenter `validate_frame`** : en‑tête `0x02`, `[2]==0x03`, longueur via `[6]`,
   checksum (§8.1) — de préférence en accumulation incrémentale (§6.9).
4. **Définir les structures** (consigne = **entier** °C, pas de demi‑degrés) :
   ```rust
   pub struct ToshibaStatus {
       pub power: bool,               // depuis PowerState (STATE::On/Off)
       pub mode: OperationMode,       // MODE 65..69 (OFF = power=false)
       pub target_temp: u8,           // 17..30 (ou 5..13 en mode 8°, déjà « dé-offseté »)
       pub current_temp: Option<i8>,  // RoomTemp ; None si 127 (invalide)
       pub fan_speed: Fan,
       pub preset: Option<SpecialMode>,
       pub swing: Swing,
       pub pwr_level: Option<PwrLevel>,
       pub self_clean: bool,
       pub outdoor_temp: Option<i8>,  // None si 127
       // Diagnostics ODU/IDU optionnels (§6.10) :
       pub cdu_td_temp: Option<i8>, pub cdu_ts_temp: Option<i8>, pub cdu_te_temp: Option<i8>,
       pub cdu_load_pct: Option<f32>, // = octet / 1.7
       pub cdu_iac_a: Option<u8>,
       pub fcu_tc_temp: Option<i8>, pub fcu_tcj_temp: Option<i8>, pub fcu_fan_rpm: Option<u16>,
   }
   ```
5. **Implémenter `build_read`/`build_write`** : assembler préfixe fixe + `cmd` (+ `value`)
   + checksum (§6.2). Gérer le décalage `+16` de la consigne en mode 8° (§6.11).
6. **Implémenter `parse_response`** : dispatch par **longueur** (15/16/17/22/24, §6.9)
   puis par type → `ToshibaStatus`.
7. **Écrire les tests unitaires host-side** (`tests/test_protocol.rs`) : valider checksum
   et parsing **sur les trames constantes connues** (les 8 trames de handshake ont un
   checksum vérifiable) + trames d'état capturées au banc.

### Phase 3 : Interface UART (1h30)

1. Initialiser UART2 (ou UART1) avec les bons paramètres :
   ```rust
   let config = uart::config::Config::new()
       .baudrate(Hertz(9600))
       .data_bits(uart::config::DataBits::DataBits8)
       .parity(uart::config::Parity::ParityEven)  // ⚠️ EVEN
       .stop_bits(uart::config::StopBits::STOP1);
   ```
2. Configurer les pins **GPIO33 (TX ESP → RX unité)** et **GPIO32 (RX ESP ← TX unité)**
   (défaut du composant ESPHome ; cf. §3.2 pour le croisement et la vérification au banc).
3. Implémenter `send_bytes(data: &[u8])` avec flush après envoi.
4. Implémenter `read_bytes(timeout_ms: u32) -> heapless::Vec<u8, 64>` (buffer circulaire).
5. **Gérer le framing** : détection du start-byte, lecture de la longueur, attente du checksum.

### Phase 4 : WiFi et MQTT (2h)

1. **WiFi** : Utiliser `esp_idf_svc::wifi::EspWifi` en mode Station.
   - Lecture des credentials depuis NVS (flash) ou variables d'environnement au build.
   - Reconnexion automatique avec backoff exponentiel.
2. **MQTT** : Utiliser le client intégré à `esp_idf_svc::mqtt::client::EspMqttClient`.
   - Connexion au broker Mosquitto Pi5 (`192.168.1.141:1883`, `allow_anonymous`).
   - LWT (Last Will Testament) pour signaler la déconnexion.
   - **Préfixe** : rester dans l'espace **local non bridgé** `santuario/toshiba/<zone>/…`
     (cf. `docs/integration-toshiba-shorai-esphome.md` §5 — **ne PAS bridger vers le
     NanoPi** ; valider avec `verify-no-loop.sh`).
   - **Topic d'état** (publish) : `santuario/toshiba/<zone>/state` — JSON structuré.
   - **Topic de commande** (subscribe) : `santuario/toshiba/<zone>/command`.
   - **Disponibilité** : `santuario/toshiba/<zone>/status` — `online` / `offline`.
   > **Cohérence Pi5** : le module `energy-manager logic/toshiba_ac` est prévu pour le
   > schéma **ESPHome/HA‑climate** (sous‑topics `mode/state`, `target_temperature/command`…).
   > Si ce firmware Rust doit s'y brancher **sans modifier le module Pi5**, reproduire ce
   > schéma de sous‑topics plutôt qu'un unique blob JSON — **à figer après le 1er boot**
   > (`mosquitto_sub -t 'santuario/toshiba/#' -v`).
3. **Format JSON d'état** (exemple — valeurs conformes §6) :
   ```json
   {
     "power": true,
     "mode": "heat",
     "target_temp": 22,
     "current_temp": 21,
     "fan_speed": "auto",
     "preset": "Standard",
     "swing": "vertical",
     "pwr_level": 100,
     "outdoor_temp": 8,
     "cdu_load_pct": 45.3,
     "self_clean": false,
     "timestamp": 1720000000
   }
   ```
4. **Format JSON de commande** (exemple) :
   ```json
   {
     "cmd": "set",
     "power": true,
     "mode": "cool",
     "target_temp": 24,
     "preset": "eco"
   }
   ```

### Phase 5 : Machine à états et logique applicative (3h)

```rust
pub enum State {
    Boot,           // Initialisation hardware
    HandshakeInit,  // Envoi des 6 trames INIT
    HandshakeWait,  // Attente 2s
    HandshakeAfter, // Envoi des 2 trames AFTER
    Online,         // Communication normale
    Error,          // Timeout ou checksum invalide
    RetryDelay,     // Attente avant re-handshake (10s)
}
```

1. **Gestion de la file d'attente** : `heapless::spsc::Queue` ou `embassy_sync::channel` pour stocker les commandes MQTT à envoyer.
2. **Boucle principale** (task FreeRTOS) :
   - Si `Online` et file non vide → envoyer commande UART.
   - Lire les données UART disponibles (timeout 200 ms).
   - Valider et parser les trames reçues.
   - Si trame valide → mettre à jour `ToshibaStatus` et publier MQTT si changement détecté (delta).
   - Si timeout persistant (3x) → transition vers `Error` puis `RetryDelay`.
3. **Watchdog logiciel** : Si aucune trame reçue pendant 60s, forcer un re-handshake.
4. **Déduplication** : Ne publier MQTT que si l'état a changé (évite le spam).

### Phase 6 : Configuration et NVS (1h)

1. Stocker en NVS (flash) via `esp_idf_svc::nvs::EspNvs` :
   - `wifi_ssid`, `wifi_pass`
   - `mqtt_broker`, `mqtt_port`, `mqtt_user`, `mqtt_pass`
   - `device_id` (pour les topics MQTT)
   - `uart_tx_pin`, `uart_rx_pin` (optionnel)
2. **Premier boot** : Si NVS vide, démarrer en mode AP (WiFi Access Point) avec un serveur HTTP minimal pour configurer via un formulaire web.
   - Alternative : utiliser le port série (UART0) pour injecter la config en CLI.

### Phase 7 : Tests et validation (4h–8h)

1. **Tests unitaires (host)** :
   - `test_checksum` : valider sur des trames capturées réelles.
   - `test_parse_status` : vérifier le décodage d'une trame d'état brute.
   - `test_build_command` : vérifier que `build_command(CmdType::TargetTemp, 22)` produit la bonne séquence.
2. **Tests sur cible nue (ESP32)** :
   - Vérifier l'envoi du handshake avec un analyseur logique (Saleae, PulseView).
   - Simuler le climatiseur avec un second ESP32 ou un PC (USB-UART) envoyant des trames de réponse pré-enregistrées.
3. **Tests réels avec le climatiseur** :
   - Démarrer le handshake, observer les réponses dans les logs (UART0).
   - Vérifier que la température ambiante et l'état ON/OFF sont corrects.
   - Envoyer des commandes MQTT et vérifier la réaction de l'unité (marche/arrêt, changement de mode, réglage température).
   - Tester les presets avancés (`8 degrees`, `Fireplace`, etc.) si l'unité les supporte.
4. **Tests de robustesse** :
   - Débrancher/rebrancher le câble CN22 → vérifier la reconnexion auto.
   - Couper le WiFi → vérifier la reconnexion MQTT et le LWT.
   - Envoyer une commande invalide → vérifier que le firmware ne panique pas.

---

## 8. Détails techniques critiques

### 8.1 Checksum & validation ✅ source

```rust
/// Checksum SUZUMI : `frame` = trame SANS l'octet de checksum (en-tête 0x02 inclus).
pub fn compute_checksum(frame: &[u8]) -> u8 {
    frame[1..].iter().fold(0u8, |a, &b| a.wrapping_add(b)).wrapping_neg()
}

/// Validation d'une trame reçue complète (checksum au dernier octet).
/// Reproduit `validate_message_` : en-tête 0x02, octet [2]==0x03, longueur via [6].
pub fn validate_frame(data: &[u8]) -> Result<(), ProtocolError> {
    if data.len() < 8   { return Err(ProtocolError::TooShort); }
    if data[0] != 0x02 { return Err(ProtocolError::BadHeader); }
    if data[2] != 0x03 { return Err(ProtocolError::BadFrame); }
    // Le firmware calcule l'INDEX de l'octet de checksum : cks_idx = 6 + data[6] + 1.
    // La taille totale attendue est donc cks_idx + 1 (ex. écriture data[6]=7 → idx 14, 15 o.).
    let cks_idx = 6 + data[6] as usize + 1;
    if data.len() != cks_idx + 1 { return Err(ProtocolError::BadLength); }
    let expected = compute_checksum(&data[..cks_idx]); // somme data[1..cks_idx-1]
    let got = data[cks_idx];
    if got != expected {
        return Err(ProtocolError::ChecksumMismatch { expected, got });
    }
    Ok(())
}
```

> Note : côté réception, mieux vaut **accumuler octet par octet** et valider de façon
> incrémentale (comme `handle_rx_byte_`/`validate_message_`) plutôt que d'attendre un
> buffer figé — l'unité n'envoie pas de délimiteur de fin, la longueur se déduit de `[6]`.

### 8.2 Temporisation ✅ source (constantes du firmware)

| Étape | Délai | Origine / tolérance |
|-------|-------|---------------------|
| **Inter-trame (file de commandes)** | **100 ms** | `COMMAND_DELAY` — min. entre 2 trames émises, **et** seulement si `rx_message_` vide |
| **Attente post-handshake** | **2000 ms** | pseudo-commande `DELAY` dans la file (stricte) |
| **Timeout réception UART** | **200 ms** | `RECEIVE_TIMEOUT` — sans octet reçu pendant 200 ms, purger le buffer partiel |
| **Période de polling** | **120 s** (défaut) | `update()` : redemande `RoomTemp` (+ `OutdoorTemp`/`SelfClean`). Ajustable |
| Re-handshake après échec | 10 s (conseillé) | design firmware Rust : backoff exp. max 60 s |
| Heartbeat MQTT | 60 s (conseillé) | design : publier `availability: online` + état complet |

### 8.3 Gestion des erreurs et reprise

| Erreur | Comportement |
|--------|--------------|
| Checksum invalide | Ignorer la trame, incrémenter un compteur. Au 3e échec consécutif → re-handshake. |
| Timeout UART (pas de réponse) | Attendre 10s, puis rejouer le handshake complet. |
| WiFi déconnecté | Tentative de reconnexion toutes les 5s. MQTT reste en pause. |
| MQTT déconnecté | Tentative de reconnexion toutes les 10s. Bufferiser les commandes reçues (max 10). |
| Mémoire insuffisante | Log d'erreur, redémarrage watchdog (panic handler personnalisé). |
| Commande invalide reçue (MQTT) | Répondre sur `.../command/response` avec JSON d'erreur, ne pas crasher. |

---

## 9. Déploiement et maintenance

### 9.1 Flashage initial

```bash
# Avec espflash (recommandé)
espflash flash --monitor target/xtensa-esp32-espidf/release/toshiba-suzumi-rs

# Ou cargo intégré
cargo run --release
```

### 9.2 Mise à jour OTA (optionnel)

- Prévoir une partition OTA dans `partitions.csv`.
- Utiliser `esp-idf-svc::ota::EspOta` pour télécharger et flasher un nouveau firmware depuis une URL HTTPS.
- **Sécurité** : vérifier la signature ECDSA du binaire avant flash.

### 9.3 Configuration runtime

- **NVS** : stockage persistant des credentials WiFi/MQTT.
- **Mode AP de secours** : si WiFi inconnu au boot, créer un AP `Toshiba-Setup-<chipid>` avec un captive portal HTTP.
- **UART0 CLI** : commandes `wifi-set`, `mqtt-set`, `reboot`, `status` accessibles via le port série de debug.

---

## 10. Ressources et documentation

| Ressource | Lien |
|-----------|------|
| The Rust on ESP Book | https://docs.esp-rs.org/book/ |
| esp-idf-hal documentation | https://docs.esp-rs.org/esp-idf-hal/ |
| esp-idf-svc examples | https://github.com/esp-rs/esp-idf-svc/tree/master/examples |
| Code source C++ référence (pedobry) | https://github.com/pedobry/esphome_toshiba_suzumi |
| 2ᵉ implémentation (validation croisée, §14) | https://github.com/o0Zz/climate-uart — `src/protocols/toshiba.cpp` |
| ESP-IDF UART API | https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/peripherals/uart.html |
| Projet connexe (Shorai) | https://github.com/toremick/shorai-esp32 |
| Projet connexe (TConnect) | https://github.com/Vpowgh/TConnect |
| Discord communautaire | https://discord.gg/wYYFawvqfr |
| **Voie ESPHome du projet** (câblage, YAML, MQTT, module EM) | `docs/integration-toshiba-shorai-esphome.md` |

**Fichiers source vérifiés** (juillet 2026, branche `main`) pour la spec §6/§8 :
`components/toshiba_suzumi/{toshiba_climate.cpp, toshiba_climate.h,
toshiba_climate_mode.cpp, toshiba_climate_mode.h, climate.py}`.

**Modèles couverts par le composant** (donc par ce protocole) : Seiya, Suzumi Plus,
Shorai Premium, Daiseikai 9, **Shorai Edge** (RAS‑B07 … RAS‑B24). MCU testés :
ESP32 (WROOM‑32D) et ESP8266. → **compatible avec les 3 unités intérieures du projet.**

---

## 11. Calendrier prévisionnel

| Phase | Durée (heures) | Cumul (heures) |
|-------|:--------------:|:--------------:|
| Environnement + squelettage | 1 | 1 |
| Protocole (transposition + tests unitaires) | 3 | 4 |
| UART (init, read, write, framing) | 1.5 | 5.5 |
| WiFi + MQTT (connexion, topics, JSON) | 2 | 7.5 |
| Logique applicative (state machine, queue, watchdog) | 3 | 10.5 |
| Configuration NVS + mode AP secours | 1 | 11.5 |
| Tests sur cible + débogage réel | 4–8 | 15.5–19.5 |
| **Total** | | **~2–3 jours intensifs** ou **1 semaine à rythme normal** |

---

## 12. Conseils supplémentaires et pièges à éviter

1. **Démarrage progressif** : Commencez par un MVP qui envoie le handshake et affiche les réponses brutes en hexadécimal sur la console UART0 (sans MQTT). Validez le handshake avant d'ajouter la pile réseau.
2. **Logging** : Utilisez `esp_idf_svc::sys::link_patches()` et `esp_idf_svc::log::EspLogger` pour avoir les traces sur UART0. Activez `CONFIG_LOG_DEFAULT_LEVEL_DEBUG` pendant le développement.
3. **Parité UART** : C'est le piège le plus courant. Le C++ source utilise `parity: EVEN`. Si vous configurez `None`, le climatiseur ignorera silencieusement toutes les trames.
4. **Level-shifter** : Ne pas négliger l'étage de conversion de niveau. Un simple diviseur résistif côté RX peut suffire, mais un TXB0108 est plus sûr et bidirectionnel.
5. **Version de secours** : Avant le premier flash sur le hardware définitif, testez avec un ESP32 de développement et assurez-vous de pouvoir entrer en mode boot (GPIO0 → GND) en cas de brick.
6. **Scan de capteurs inconnus** : Le C++ original propose une fonction `scan()` (balaye `requestData(i)`, `i` = 128→254) pour découvrir les capteurs sur des modèles non répertoriés. Prévoir une commande MQTT `santuario/toshiba/<zone>/scan` qui active un mode debug et logue toutes les trames inconnues.
7. **Mémoire** : L'ESP32 a 520 Ko de SRAM. Utilisez `heapless::Vec` et `heapless::String` pour éviter les allocations dynamiques dans le hot path (UART/MQTT).
8. **Task priorities** : Donnez une priorité FreeRTOS plus élevée à la task UART que à la task MQTT pour ne pas manquer de trames.

---

## 13. Checklist de validation avant mise en production

- [ ] Handshake réussi sur 10 démarrages consécutifs sans intervention.
- [ ] Commandes ON/OFF, mode, température testées et réactives (< 2s).
- [ ] Reconnexion WiFi testée (coupure 30s, reprise automatique).
- [ ] Reconnexion MQTT testée (broker redémarré, LWT correct).
- [ ] Déconnexion/reconnexion CN22 testée (re-handshake auto).
- [ ] Capteurs ODU/IDU parsés correctement (si présents sur l'unité).
- [ ] Mémoire stable : pas de fuite détectée sur 24h de fonctionnement (`esp_get_free_heap_size()`).
- [ ] Watchdog hardware alimenté (`CONFIG_ESP_TASK_WDT_EN=y`).
- [ ] Mode AP de secours fonctionnel (si WiFi invalide).
- [ ] Binaire signé pour OTA (si fonctionnalité activée).

---

## 14. Validation croisée avec `o0Zz/climate-uart`

> Source : [`o0Zz/climate-uart`](https://github.com/o0Zz/climate-uart),
> `src/protocols/toshiba.cpp` — une **2ᵉ implémentation C++ indépendante** du protocole
> Toshiba UART, au sein d'une bibliothèque multi‑marques (Mitsubishi, **Toshiba**, Daikin
> S21, Sharp, LG, Hitachi H‑Link, Fujitsu). Modèles Toshiba cités : **Seiya, Shorai, Yukai**.

### 14.1 Verdict : **même protocole** (confirmation)

Recoupement octet par octet : STX `0x02`, structure de trame (`[6]=dataSize+5`,
`[11]=dataSize`, données = `{fonction, valeur}`), **checksum identique**
(`−Σ octets[1..]`), et **valeurs identiques** pour les codes fonction et les mappings :

| | pedobry (notre réf.) | o0Zz | |
|---|---|---|---|
| Power / Mode / Setpoint | `0x80 / 0xB0 / 0xB3` | `kFunctionPowerState/UnitMode/Setpoint` idem | ✅ |
| Fan / Swing / RoomTemp | `0xA0 / 0xA3 / 0xBB` | idem | ✅ |
| Power ON/OFF | `0x30 / 0x31` | `kPowerStateOn/Off` idem | ✅ |
| Modes (Auto/Cool/Heat/Dry/Fan) | `0x41..0x45` | idem | ✅ |
| Fan (Quiet…Auto) | `0x31..0x36, 0x41` | `kFanQuiet/Lvl1..5/Auto` idem | ✅ |
| Swing (Fix/V/H/Both/Pos1‑5) | `0x31,0x41‑43,0x50‑54` | idem | ✅ |
| Handshake | 8 trames | **7/8 identiques** | ⚠️ voir 14.2 |

→ Deux reverse‑engineering indépendants convergent : **notre spécification §6 est fiable.**

### 14.2 Un écart à connaître : `HANDSHAKE[4]` / `kSyn5`

Détaillé en §6.4 : dernier octet **`0xFE` (pedobry) vs `0xFB` (o0Zz, checksum‑correct)**.
L'unité ignore vraisemblablement le checksum des trames d'init → garder `0xFE`, tester
`0xFB` seulement si le handshake échoue au banc.

### 14.3 Apports utiles de o0Zz (à considérer)

1. **Codes fonction supplémentaires nommés** par o0Zz, **absents** de pedobry — non
   caractérisés, à sonder au `scan()` : `kFunctionStatus = 0x88`, `kFunctionGroup1 = 0xF8`.
   (pedobry expose plutôt `SPECIAL_MODE = 0xF7`, absent de o0Zz.)
2. **Indice de parsing des réponses** : o0Zz marque les réponses avec
   `type = commande | masque_réponse = 0x10 | 0x80 = 0x90` (octet `[3]`). Utile pour
   distinguer une **réponse** (`[3]=0x90`) d'un **ACK/commande** — complément à notre
   dispatch par longueur (§6.9).
3. **Timeout** : `kPacketReadTimeoutMs = 250 ms` (vs `RECEIVE_TIMEOUT = 200 ms` pedobry) —
   même ordre de grandeur ; 200–250 ms est la bonne fenêtre.
4. **Architecture logicielle** (inspiration directe pour notre `protocol.rs` / `uart.rs`) :
   o0Zz sépare un **transport UART abstrait** (portable ESP32/Arduino) d'une **classe
   protocole par marque** exposant une API unifiée (`query/sendCommand/setState/getState/
   getRoomTemperature`). En Rust : un `trait UartTransport` + un `struct ToshibaProtocol`
   pur (sans I/O) testable sur host → aligne bien avec les règles projet (#16 supervision,
   décisions/parsing purs et testés).

### 14.4 Ce que o0Zz **ne** couvre pas (→ on garde pedobry comme référence primaire)

o0Zz/Toshiba est un **sous‑ensemble** « contrôle de base ». Il **n'implémente pas** ce que
nous avons déjà documenté et qui a de la valeur pour le projet :

- **Diagnostics ODU/IDU** (`0xE5`/`0xE4` : `cdu_*`, `fcu_*`, charge compresseur, RPM) — §6.10.
- **Température extérieure** (`0xBE`) — §6.5.
- **Auto‑nettoyage** (`0xCB`) et **LED WiFi** (`0xDE`/`0xDF`).
- **Presets / modes spéciaux** (`0xF7` : 8°/garde hors‑gel, ECO, Hi‑Power, Sleep…) — §6.7/§6.11.

### 14.5 Conclusion pour le projet

**Aucun ajout protocolaire indispensable** : notre spec §6 est déjà un **sur‑ensemble** de
o0Zz/Toshiba. Actions retenues : (a) **note `HANDSHAKE[4]`** intégrée (§6.4) ; (b) **sonder
`0x88`/`0xF8`** lors du `scan()` de mise au point ; (c) **s'inspirer de l'architecture
transport/protocole** de o0Zz pour le découpage Rust ; (d) fenêtre timeout **200–250 ms**.
pedobry reste la **référence primaire** (couverture complète : diagnostics + presets).

---

## 15. Paysage des protocoles Toshiba — NE PAS CONFONDRE ⚠️

Il existe **deux familles de protocoles série Toshiba, incompatibles**. Un contrôleur
prévu pour l'un **ne fonctionnera pas** sur l'autre. Nos 3 unités **Shorai Edge**
(splits résidentiels) relèvent **exclusivement** de la voie **CN22 / SUZUMI**.

| Critère | **Voie CN22 / SUZUMI (LA NÔTRE)** | Voie A/B « TCC‑Link » |
|---|---|---|
| Bus physique | connecteur **CN22** (port de l'adaptateur WiFi d'origine) | **bus filaire A/B** (2 fils de la télécommande murale) |
| Familles d'unités | **splits résidentiels** : Seiya, Suzumi Plus, **Shorai (Edge/Premium)**, Daiseikai, Yukai | **central / VRF / light‑commercial** (ex. RAV‑SM406BTP‑E) via télécommande **RBC‑AMT32E** |
| UART | **9600** 8E1 | **2400** 8E1 |
| Début de trame | **STX `0x02`** + préfixe fixe | **octet d'adresse source** (`0x00` master / `0x40` remote / `0xFE` broadcast) |
| Adressage | non (liaison point‑à‑point) | **oui** (`FROM|TO|OPCODE1|COUNT|MODE|OPCODE2|…`) |
| Checksum | **`256 − Σ(octets[1..])`** (complément à deux) | **XOR** de tous les octets (ex. `00 FE 10 02 80 8A` → CRC `E6`) |
| Modes (octet) | `0x41..0x45` | `1=Heat, 2=Cool, 3=Fan, 4=Dry, 6=Auto` |
| Fan (octet) | `0x31..0x36, 0x41` | `2=Auto, 3=High, 4=Med, 5=Low` |
| Implémentations | **pedobry** (réf.), **o0Zz** (§14) | **issalig/toshiba_air_cond**, makusets/esphome‑toshiba‑ab, muxa/esphome‑tcc‑link |

**Conclusion** : `issalig/toshiba_air_cond` (`air/ac_protocol.h/.cpp`) implémente le
**protocole A/B TCC‑Link à 2400 bauds avec CRC XOR** — **inapplicable** à nos Shorai Edge.
Ne **pas** câbler sur A/B ni régler 2400/XOR pour nos unités. Ce projet reste utile
**uniquement** comme culture du paysage Toshiba (et comme référence si, un jour, on devait
intégrer une unité **centrale/VRF** — cas non présent dans le projet).

### 15.1 Documentation officielle Toshiba

**Aucune spécification publique officielle** du protocole série (ni CN22/SUZUMI ni A/B)
n'a été trouvée : **toutes** les implémentations connues sont **reverse‑engineered**
(analyseur logique + captures). Les documents Toshiba officiels *connexes* mais **non
pertinents** pour le protocole série applicatif : guides de **codes défaut TCC‑Link**, et
**passerelles** LonWorks/Modbus/BACnet (ex. interface **TCB‑IFLN642TLE**) — ces passerelles
parlent Modbus/LON côté client, pas le protocole CN22. → **Notre spec §6 (issue de pedobry,
recoupée o0Zz) reste la meilleure source disponible.**

---

*Bon courage ! Si vous bloquez sur un point précis (handshake, checksum, parsing d'une trame spécifique), n'hésitez pas à demander avec un extrait de log hexadécimal.*
