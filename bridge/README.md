# `bridge/` — ponts d'intégration (services Pi5 auxiliaires)

Ce dossier regroupe les **ponts** : des services **auxiliaires** hébergés sur le Pi5 qui
traduisent un protocole externe ↔ notre **dorsal MQTT** `santuario/…`. Chacun est une
**unité de déploiement autonome** (son propre service systemd), **hors** du workspace Rust
principal.

> ℹ️ **Le regroupement se fait par *fonction* (pont d'intégration), PAS par langage.**
> C'est pourquoi on trouve ici du **Rust et du Python** côte à côte — c'est voulu.

## Pourquoi le Rust ici n'est **pas** dans `crates/`

| Dossier | Signification | Build |
|---------|---------------|-------|
| `crates/` | **membres du workspace** (`members = [...]` racine) | compilés ensemble, **cross-buildés par la CI** (aarch64/armv7, toolchain 1.94.1), 1 seul `Cargo.lock` |
| `bridge/`, `firmware/` | crates/services **détachés** (`[workspace]` vide) | isolés → **zéro impact** sur le build/CI de daly-bms |

Un crate détaché (`matter-toshiba-rs`, comme `firmware/toshiba-suzumi-rs`) reste **dehors**
pour garder ses **dépendances lourdes / expérimentales** (rs-matter : crypto, mDNS, pré-1.0)
**hors** du workspace et de ses cross-builds. Le mettre dans `crates/` casserait cette
isolation. → L'axe est **l'unité de déploiement**, pas le langage.

## Les ponts

| | Pont | Langage | Rôle | README |
|---|------|---------|------|--------|
| **C** | `aqara-fp2-mqtt/` | Python (aiohomekit) | lit la présence FP2 (HomeKit local) → publie `santuario/toshiba/presence/<zone>` | [→](./aqara-fp2-mqtt/README.md) |
| **D** | `mqtt-homekit-occupancy/` | Python (HAP-python) | ré-expose la présence MQTT → capteurs d'occupation **Apple Home** — *généralisé par `mqtt-homekit-sensors`* | [→](./mqtt-homekit-occupancy/README.md) |
| **D′** | `mqtt-homekit-sensors/` | Python (HAP-python) | **généralise D** : MQTT → capteurs HomeKit **multi-types** (température, luminosité, occupation). Sert à **tester la chaîne** avec les capteurs déjà en place | [→](./mqtt-homekit-sensors/README.md) |
| **E** | `matter-toshiba-rs/` | **Rust** (détaché) | expose les clim (Thermostat) + présence (Occupancy) en **Matter** multi-fabric | [→](./matter-toshiba-rs/README.md) |

> Référence opérationnelle complète (contrat MQTT, carte des composants A–E, pipeline
> présence, décision **E supersède D**) → [`docs/toshiba-bridges.md`](../docs/toshiba-bridges.md).
