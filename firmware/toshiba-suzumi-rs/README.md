# toshiba-suzumi-rs

Firmware Rust natif (ESP32) pour piloter les climatiseurs **Toshiba Shorai Edge**
via le connecteur **CN22** (protocole série SUZUMI, 9600 8E1), en remplacement de
l'adaptateur WiFi Toshiba — **sans ESPHome ni Home Assistant**, MQTT comme seul
point de contact.

> 📄 **Référence complète et reprise de session** :
> [`../../docs/toshiba-suzumi-rs-plan.md`](../../docs/toshiba-suzumi-rs-plan.md) —
> **lire son §0** (état d'avancement, décisions, journal, prochaines étapes).

## Statut

⚠️ **Matériel ESP32 pas encore disponible** → progression **sans aucun test
matériel**. Les **couches logiques pures** sont faites et **testées sur host** ;
la partie ESP-IDF (UART/WiFi/MQTT) viendra ensuite (feature `esp32`).

Crate **détaché** du workspace Pi5 (`[workspace]` vide) → aucun impact sur le
build/CI de `daly-bms-server`/`energy-manager`.

## Modules (couches pures, host-testables ✅)

| Module | Rôle |
|--------|------|
| `protocol` | checksum, enums, construction/validation de trames, parsing (+ ODU/IDU), handshake, timings |
| `state` | `ToshibaState` : image courante, applique les `Field` parsés |
| `framing` | `FrameAccumulator` : assemblage RX octet par octet (façon `validate_message_`) |
| `machine` | `Client` : séquence handshake, pacing 100 ms, file de commandes, watchdog, commandes haut niveau — **+ simulateur d'unité** (test end-to-end) |
| `mqtt_payload` | état ↔ JSON télémétrie ; JSON de commande ↔ `Command` → `Client::apply_command` |
| `config` | `NodeConfig` + dérivation des topics `santuario/toshiba/<zone>/…` + validation |

À venir (⏳ nécessite le matériel) : `uart` (ESP-IDF), `wifi`, `mqtt` (transport),
`main` — se contenteront de relayer vers `Client::{on_rx_byte, poll_tx, on_tick}`.

**Toolchain ESP32 (Xtensa) + flash** : voir `docs/toshiba-suzumi-rs-plan.md` **§16**
(carte ESP32‑WROOM‑32U « KIT A » IPEX ; `espup`, pas `rustup target add`).

## Tester (sur host, sans matériel)

```bash
cargo test   --manifest-path firmware/toshiba-suzumi-rs/Cargo.toml
cargo clippy --manifest-path firmware/toshiba-suzumi-rs/Cargo.toml --all-targets -- -D warnings
cargo fmt    --manifest-path firmware/toshiba-suzumi-rs/Cargo.toml --check
```
