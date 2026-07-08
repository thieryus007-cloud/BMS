# Ponts & crates d'intégration Toshiba / FP2 — **référence opérationnelle**

> Cette doc porte le **quoi/comment** des composants (crates, ponts, contrat MQTT, tests).
> Le **pourquoi** (décisions, comparatifs, sources) reste dans
> [`toshiba-suzumi-rs-plan.md`](./toshiba-suzumi-rs-plan.md) **§18** (et la reprise de session
> en **§0**). But : alléger le plan, devenu lourd, en sortant le détail opérationnel ici.

---

## 1. Vue d'ensemble

```
                         ┌──────────────── Pi5 (broker Mosquitto = dorsal) ─────────────────┐
Clim Toshiba ×3→7        │                                                                  │
  ↕ CN22 (UART SUZUMI)   │   santuario/toshiba/<zone>/state ─► energy-manager (B, lecture)  │
[A] ESP32 (Rust+MQTT) ───┼──►                               ─► daly-bms-server (redb + Grafana 22)
                         │                                  ─► [E] bridge Matter (Thermostat)│
                         │   santuario/toshiba/<zone>/command ◄─ energy-manager (B, présence)│
                         │                                    ◄─ [E] bridge Matter (écriture) │
FP2 ×7 (WiFi/HomeKit) ───┼──[C] aiohomekit→MQTT─► presence/<zone> ─► energy-manager (B)      │
                         │                                        ─► [D] MQTT→HomeKit (Apple)│
                         │                                        ─► [E] Matter Occupancy (futur)
Passerelle (Homey/Apple/Google) ◄──── Matter multi-fabric ──── [E] bridge Matter (Pi5)       │
                         └──────────────────────────────────────────────────────────────────┘
```

**Principe directeur** (cf. plan §18.11/§18.12) : **un seul dorsal = MQTT**, source de vérité.
Les protocoles smart-home (HomeKit, Matter) vivent **au bord**, sur le **Pi5** (traducteurs),
jamais sur les ESP32 contraints. Le firmware ESP32 reste **Rust + MQTT**, léger.

---

## 2. Contrat MQTT (figé)

Préfixe **local** `santuario/toshiba/…` — **non** relayé par le bridge NanoPi (règle #11).

| Topic | Sens | Payload | Producteur → Consommateur(s) |
|-------|------|---------|------------------------------|
| `santuario/toshiba/<zone>/state` | télémétrie | `{power,mode,target_temp,current_temp,outdoor_temp,fan,swing,preset,pwr_level,self_clean}` | firmware **A** → EM **B** (lecture), daly-bms-server (redb `toshiba_ac_*`), bridge Matter **E** |
| `santuario/toshiba/<zone>/command` | commande | `{power,mode,target_temp,preset,…}` (partiel) | EM **B** (présence) + bridge Matter **E** (écriture) → firmware **A** |
| `santuario/toshiba/<zone>/availability` | dispo | `online`/`offline` (LWT) | firmware **A** |
| `santuario/toshiba/presence/<zone>` | présence | `{"present":bool,"ts":epoch}` (retained) | pont FP2 **C** → EM **B** (contrôle), HomeKit **D**, Matter **E** (futur) |
| `santuario/toshiba/presence/bridge/availability` | dispo | LWT du pont **C** | pont FP2 **C** |
| `santuario/toshiba/presence/homekit-bridge/availability` | dispo | LWT du pont **D** | pont **D** |

`<zone>` = nom du nœud Toshiba (`Shorai-31/32/33`, → 7). La présence est publiée **sous le
même nom de zone** que la clim → **pas de table de mapping** côté EM/bridges.

---

## 3. Carte des composants

| | Composant | Emplacement | Techno | Tests | README | Statut |
|---|-----------|-------------|--------|-------|--------|--------|
| **A** | Firmware ESP32 (protocole SUZUMI) | `firmware/toshiba-suzumi-rs/` | Rust (détaché) | `cargo test --manifest-path firmware/toshiba-suzumi-rs/Cargo.toml` (**39**) | [lien](../firmware/toshiba-suzumi-rs/README.md) | ✅ protocole pur ; ⏳ I/O ESP-IDF (matériel) |
| **B** | Module EM `logic/toshiba_ac` | `crates/energy-manager/src/logic/toshiba_ac/` | Rust (workspace) | `cargo test -p energy-manager toshiba` (**14**) | plan §18.5 | ✅ lecture + **contrôle présence câblé** ; inerte tant que `control_enabled=false` |
| **C** | Pont FP2 → MQTT présence | `bridge/aqara-fp2-mqtt/` | Python (aiohomekit) | `python3 bridge/aqara-fp2-mqtt/tests/test_core.py` (**8**) | [lien](../bridge/aqara-fp2-mqtt/README.md) | ✅ cœur pur ; ⏳ `hap.py` (FP2 réel) |
| **D** | Pont MQTT → HomeKit (occupation) | `bridge/mqtt-homekit-occupancy/` | Python (HAP-python) | `python3 bridge/mqtt-homekit-occupancy/tests/test_core.py` (**10**) | [lien](../bridge/mqtt-homekit-occupancy/README.md) | ✅ cœur pur ; ⏳ `accessory.py`/`mqtt_in.py` (appairage) — **voir §5 : superseded par E** |
| **E** | Bridge Matter (passerelle) | `bridge/matter-toshiba-rs/` | Rust (détaché, **sans Node**) | `cargo test --manifest-path bridge/matter-toshiba-rs/Cargo.toml` (**15**) | [lien](../bridge/matter-toshiba-rs/README.md) | ✅ cœur pur ; ⏳ transport rs-matter (passerelle) |

> **Patron commun** à C/D/E (et A) : **cœur pur testé sur host** (logique/mapping, zéro I/O) +
> **couche transport** à finaliser (marquée TODO/`# VERIFY`). Les décisions ayant conduit à
> chaque brique sont dans le plan §18.

---

## 4. Pipeline présence FP2 → contrôle clim (C → B)

1. **C** (`aiohomekit`, Pi5) s'appaire à chaque FP2 en **HomeKit local** (le FP2 est WiFi+HomeKit,
   pas Zigbee), agrège ses zones (OU) et publie `presence/<zone>` *retained*.
2. **B** (`logic/toshiba_ac`) souscrit `santuario/toshiba/presence/+`, alimente `PresenceControl`
   (pur), et au tick 1 Hz applique `decide_presence` (**ECO dès absence, OFF après
   `off_after_secs`**) → publie `presence_command_json` sur `.../<zone>/command` **sur changement**.
3. **Inerte** tant que `[energy_manager.toshiba_ac].control_enabled=false`. À la pose des FP2 :
   régler le **délai d'absence du FP2 court** (sinon ses 10 min masquent nos 5 min — plan §18.9),
   puis `control_enabled=true`.

Détail décision → plan §18 (§18.2/§18.3/§18.5).

---

## 5. Les deux bords smart-home — **D (HomeKit) vs E (Matter)** et la décision

Les ponts **D** et **E** ré-exposent nos données vers un écosystème. Ils ne sont **pas
complémentaires** : ce sont **deux barreaux de la même échelle**.

| | **D. `mqtt-homekit-occupancy`** | **E. `matter-toshiba-rs`** |
|---|---|---|
| Sortie | **HomeKit seul** (app Maison iPhone) | **Matter multi-fabric** : Apple **+** Homey **+** Google simultanés |
| Couvre l'app Maison ? | ✅ | ✅ (Apple Home **est** un contrôleur Matter) |
| Portée | présence (Occupancy) | clim (Thermostat) **+** présence (Occupancy — mapping pur **fait**) |
| Disponibilité | **maintenant**, sans passerelle | **futur** (transport rs-matter + passerelle) |
| Techno | Python (HAP-python) | Rust (sans Node) |

**Décision (2026-07-07) : E supersède D.** Puisque **Apple Home consomme Matter**, un endpoint
**Occupancy** exposé par **E** apparaît dans l'app Maison **comme** le fait **D**, **plus** Homey
et Google. Conséquences :

- **D = stopgap** : à n'utiliser que pour obtenir les **tuiles présence dans Apple Home**
  **avant** d'avoir câblé le transport Matter (D est prêt et ne dépend d'aucune passerelle).
  D est **généralisé** par `bridge/mqtt-homekit-sensors/` (**D′**) : MQTT → capteurs HomeKit
  **multi-types** (température / luminosité / occupation). D′ permet de **tester la chaîne
  MQTT→HomeKit dès maintenant** avec les capteurs déjà en place (température `santuario/heat/1/venus`,
  irradiance `santuario/irradiance/raw`) — **sans** matériel Toshiba/FP2 : HAP-python est un
  accessoire logiciel, il suffit du Pi5 + un iPad/iPhone sur le même WiFi pour appairer. Couche
  HAP **vérifiée par smoke-test** (build + injection de valeurs). Cf. `bridge/mqtt-homekit-sensors/README.md`.
- **E = cible** : le mapping **présence→Occupancy** est **déjà fait et testé** dans E
  (`mapping::occupancy_bitmap` + `Bridge::on_presence_json`). Quand le transport rs-matter de E
  sera câblé (endpoints Occupancy Sensor), **on retire D**. Un seul pont pour **clim + présence**,
  multi-écosystème.

Rationale complet → plan **§18.9** (scénario C) + **§18.12** (Matter).

---

## 6. Commandes rapides

```bash
# Tests (cœurs purs — sans matériel)
cargo test --manifest-path firmware/toshiba-suzumi-rs/Cargo.toml     # A (39)
cargo test -p energy-manager toshiba                                 # B (14)
python3 bridge/aqara-fp2-mqtt/tests/test_core.py                     # C (8)
python3 bridge/mqtt-homekit-occupancy/tests/test_core.py            # D (10)
cargo test --manifest-path bridge/matter-toshiba-rs/Cargo.toml       # E (15)

# Valider un pont Python sans dépendance
python3 -m mqtt_hk check-config --config config.example.toml         # D (dry-run)

# Activer le contrôle présence (une fois les FP2 posés)
#   [energy_manager.toshiba_ac] control_enabled = true   → Config.toml → redeploy
```

Déploiement systemd de chaque pont : voir le README du composant (unités dans `contrib/`).

---

## 7. Sécurité (règle #12)

Aucun secret d'appairage commité : `bridge/*/pairings/`, `*.pairing.json` (C), `hap-state/`
+ PIN dans `config.toml` (D), passcode/discriminateur + fabric keys `matter-state/` (E). Seuls
les `config.example.*` sont versionnés.
