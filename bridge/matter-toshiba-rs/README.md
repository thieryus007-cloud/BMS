# matter-toshiba-rs — bridge Matter (Pi5) pour les clim Toshiba, **en Rust, sans Node.js**

Expose les climatiseurs Toshiba comme **endpoints Thermostat Matter** (device-type
*Bridge/Aggregator*) à partir du dorsal MQTT `santuario/toshiba/<zone>/{state,command}`,
pour une passerelle smart-home **multi-fabric** (Homey Pro / Apple Home / Google Home
simultanément). C'est la voie **tout-Rust** de `docs/toshiba-suzumi-rs-plan.md` **§18.12**
(l'alternative clé-en-main y est Matterbridge — **Node.js**, écartée ici par choix).

```
ESP32 (Rust+MQTT) ──► santuario/toshiba/<zone>/state ──►┐
                                                        │  matter-toshiba-rs (Pi5, systemd)
gateway (Homey/Apple/Google) ◄──Matter Thermostat──────┤   MQTT state → attributs cluster
                            ──Matter write──► command ──►┘   Matter write → …/<zone>/command
```

Le firmware ESP32 et energy-manager **ne changent pas** : le dorsal reste MQTT, ce bridge
n'est qu'un **traducteur au bord** (même patron que `bridge/mqtt-homekit-occupancy`).

## État

- ✅ **Cœur pur testé** (`cargo test`, **15 tests**, aucune dépendance réseau/crypto) :
  - `mapping.rs` — état Toshiba (JSON) ↔ attributs cluster **Thermostat** ; écriture
    Matter → commande MQTT.
  - `config.rs` — `NodeConfig` (zones, broker, **commissioning Matter**) + validation
    (discriminateur 0..4095, passcode non-interdit par la spec) + dérivation des topics.
  - `bridge.rs` — **orchestration** transport-agnostique (cache par zone).
- ⏳ **Couche transport** (feature `matter`, **à câbler**) : `matter.rs` (rs-matter),
  `mqtt.rs` (rumqttc), `main.rs`. Ne fait que **relayer** vers `bridge::Bridge` — aucune
  logique nouvelle. Voir ci-dessous.

Tester le cœur : `cargo test --manifest-path bridge/matter-toshiba-rs/Cargo.toml`

## Mapping cluster **Thermostat** Matter

| Attribut Matter | ← état Toshiba | Note |
|---|---|---|
| `LocalTemperature` | `current_temp` ×100 | centi-°C ; `null` si 127 (invalide) |
| `OccupiedHeatingSetpoint` | `target_temp` ×100 | consigne **unique** Toshiba miroitée… |
| `OccupiedCoolingSetpoint` | `target_temp` ×100 | …sur les deux setpoints |
| `SystemMode` | `power`+`mode` | Off / Auto / Cool / Heat / Dry / FanOnly |

| Écriture Matter | → commande MQTT (`…/<zone>/command`) |
|---|---|
| `SystemMode = Off` | `{"power":false}` |
| `SystemMode = Cool/Heat/…` | `{"power":true,"mode":"cool\|heat\|…"}` |
| setpoint (centi-°C) | `{"target_temp":<°C entier>}` |

**Non représentable** dans le cluster Thermostat (reste sur MQTT/Grafana) : presets
(8°/Fireplace/ECO), `pwr_level`, `self_clean`, diagnostics ODU/IDU. La ventilation
Toshiba se mappera sur un cluster **FanControl** séparé (évolution).

## Couche transport à câbler (feature `matter`)

Activer la feature et ajouter les dépendances (versions à figer au moment du câblage —
rs-matter est pré-1.0, API mouvante) :

```toml
[features]
matter = ["dep:rs-matter", "dep:rumqttc", "dep:tokio", "dep:log"]

[dependencies]
rs-matter = { version = "*", optional = true }   # figer la version testée
rumqttc   = { version = "0.24", optional = true }
tokio     = { version = "1", features = ["rt-multi-thread","macros","time","sync"], optional = true }
log       = { version = "0.4", optional = true }
```

Puis 3 fichiers (relais pur vers `bridge::Bridge`) :

1. **`mqtt.rs`** — client rumqttc : souscrit `santuario/toshiba/+/state`, extrait la zone
   (`config::zone_from_state_topic`), appelle `Bridge::on_state_json` → transmet les
   `ThermostatAttrs` au cluster ; publie `Bridge::on_matter_write(...)` sur
   `config::command_topic(zone)` (QoS1, non-retained) sur écriture Matter.
2. **`matter.rs`** — nœud rs-matter : un **Aggregator** (device-type Bridge) portant **un
   endpoint Thermostat par zone** (+ Descriptor/Bridged Device Basic Information). Câbler
   les *read/write callbacks* des attributs Thermostat (§« Mapping ») ↔ `Bridge`.
   Commissioning depuis `NodeConfig` (discriminator/passcode/VID/PID). Le **multi-fabric**
   est natif (≥ 5 fabrics) → commissionner dans Homey **puis** partager à Apple/Google.
3. **`main.rs`** — `#[tokio::main]` : charge la config, démarre MQTT + le nœud Matter,
   supervise (fail-fast, redémarrage systemd).

> **Base de référence** : les exemples `std` de `github.com/project-chip/rs-matter`
> (notamment l'exemple de commissioning + un cluster applicatif) — le port *std* (Linux)
> est le côté le plus mûr de rs-matter, adapté au Pi5.

## Déploiement (Pi5)

```bash
cargo build --release --features matter        # une fois la couche transport câblée
sudo cp target/release/matter-toshiba /usr/local/bin/
sudo cp contrib/matter-toshiba.service /etc/systemd/system/ && sudo systemctl enable --now matter-toshiba
```

## Sécurité (règle projet #12)

- **Passcode / discriminateur Matter** = secrets d'appairage → fournir par
  `config.toml`/env **non commité** (défauts = valeurs **de test** `0xFFF1`/`0x8000`,
  passcode `20202021` — à changer avant tout usage réel).
- `vendor_id/product_id` de production nécessitent une allocation CSA pour un appareil
  **certifié** ; la plage test suffit pour un usage privé non certifié.
- L'état d'appairage (fabric keys) persistera hors du dépôt (cf. `.gitignore`).
