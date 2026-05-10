# CLAUDE.md — Référence Projet Daly-BMS-Rust

> Chargé automatiquement à chaque session. Garder concis.
> Procédures détaillées → **PROCEDURES.md** (lire sur demande).
> 
> **ATTENTION**
> Toujours faire un PLAN et decomposer les taches pour ne pas avoir "API Error: Stream idle timeout - partial response received"

---

## 0. COMMANDES RAPIDES

### Pi5 (`~/Daly-BMS-Rust`, user: pi5compute)

| Quand | Commande |
|-------|----------|
| Récupérer le code | `make sync` |
| Appliquer Config.toml | `sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart daly-bms` |
| Logs BMS | `journalctl -u daly-bms -f` |
| Compiler Pi5 | `make build-arm` |
| Déployer binaire Pi5 | `sudo systemctl stop daly-bms && sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/ && sudo systemctl start daly-bms` |
| MQTT start/stop/logs | `make mqtt-start` / `make mqtt-stop` / `make mqtt-logs` |
| Compiler MQTT (broker+bridge) | `make build-mqtt-arm` |
| Déployer mqtt-broker | `sudo systemctl stop mqtt-broker && sudo cp target/aarch64-unknown-linux-gnu/release/mqtt-broker /usr/local/bin/ && sudo systemctl start mqtt-broker` |
| Déployer mqtt-bridge | `sudo systemctl stop mqtt-bridge && sudo cp target/aarch64-unknown-linux-gnu/release/mqtt-bridge /usr/local/bin/ && sudo systemctl start mqtt-bridge` |
| Métriques broker | `curl http://localhost:8082/metrics` |
| Métriques bridge | `curl http://localhost:8084/metrics` |
| Logs energy-manager | `journalctl -u energy-manager -f` |
| Compiler energy-manager | `make build-energy-arm` |
| Déployer energy-manager | `sudo systemctl stop energy-manager && sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/ && sudo systemctl start energy-manager` |
| Appliquer Config energy-manager | `sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart energy-manager` |

### NanoPi (`root@192.168.1.120`)

| Quand | Commande |
|-------|----------|
| État service Venus | `svstat /service/dbus-mqtt-venus` |
| Redémarrer Venus | `svc -t /service/dbus-mqtt-venus` |
| Logs Venus | `tail -f /var/log/dbus-mqtt-venus/current` |
| Lister services Victron | `dbus -y \| grep victronenergy` |

### Build + déploiement Venus (depuis Pi5)
```bash
make build-venus-v7 && make install-venus-v7
```

### Workflow complet
```
1. Claude Code → git add + commit + push
2. Pi5 → make sync
3a. Config seule        : sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart daly-bms
3b. Code Rust/HTML      : make build-arm → stop → cp binaire → start
3c. Venus code          : make build-venus-v7 && make install-venus-v7
3d. Config NanoPi       : scp nanoPi/config-nanopi.toml root@192.168.1.120:/data/daly-bms/config.toml && ssh root@192.168.1.120 "svc -t /service/dbus-mqtt-venus"
3e. energy-manager code : make build-energy-arm → sudo systemctl stop energy-manager → sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/ → sudo systemctl start energy-manager
3f. Config seule (energy): sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart energy-manager
3g. mqtt-broker code    : make build-mqtt-arm → sudo systemctl restart mqtt-broker mqtt-bridge
3h. Config bridge seule : sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart mqtt-bridge
3i. Config broker seule : sudo cp crates/mqtt-broker/mqtt-broker.toml /etc/daly-bms/ && sudo systemctl restart mqtt-broker
```

---

## 1. ARCHITECTURE

```
Pi5 (192.168.1.141, pi5compute)
  mqtt-broker (systemd, :1883/:9001) ← rumqttd — remplace Mosquitto/Docker
    ├── TCP  :1883  ← tous les clients MQTT locaux
    ├── WS   :9001  ← explorateur dashboard JS
    ├── HTTP :8082  ← /metrics pour monitor agent
    └── Persistence : /var/lib/mqtt-broker
  mqtt-bridge (systemd) ← bridge Pi5 ↔ NanoPi
    ├── SUB localhost:1883 → republish NanoPi (W/ R/ santuario/ shellypro2pm/)
    ├── SUB NanoPi:1883   → republish local  (N/ santuario/ shellypro2pm/)
    └── HTTP :8084  ← /metrics (compteurs, état connexion)
  daly-bms-server (systemd, :8080)
    ├── RS485 /dev/ttyUSB0 → 2 BMS + 3 ET112 + 1 PRALRAN
    ├── REST API + WebSocket :8080
    ├── MQTT publish → localhost:1883 (broker local rumqttd)
    └── VictoriaMetrics → localhost:8428
  energy-manager (systemd, :8081)
    ├── MQTT subscribe/publish → localhost:1883 (broker local rumqttd)
    ├── Logique solaire, DEYE, chauffe-eau, charge, météo
    ├── WebSocket live events :8081/live
    └── VictoriaMetrics → localhost:8428

NanoPi (192.168.1.120, root)
  dbus-mqtt-venus (runit /service/dbus-mqtt-venus)
    └── MQTT subscribe → D-Bus Victron (com.victronenergy.*)
```

---

## 2. RÉSEAU & SSH

| Machine | IP | User |
|---------|----|------|
| Pi5 | 192.168.1.141 | pi5compute |
| NanoPi | 192.168.1.120 | root |

SSH Pi5 config (`~/.ssh/config`): clé `~/.ssh/id_nanopi` → `Host nanopi` + `Host 192.168.1.120` (les deux entrées nécessaires).

---

## 3. GIT

- **Repo** : `thieryus007-cloud/Daly-BMS-Rust`
- **Branche active** : voir `git branch` — toujours vérifier avant push
- **Pi5** : `make sync` uniquement — jamais de commit local
- **Push** : `git push -u origin <branch>`
- **Convention** : `feat(scope):` `fix(scope):` `chore(scope):` `docs(scope):` `refactor(scope):`
- **Règle** : 2 branches max (`main` + 1 branche Claude active)

---

## 4. STRUCTURE PROJET (fichiers clés)

```
Config.toml                              ← config Pi5 production (daly-bms + energy-manager)
nanoPi/config-nanopi.toml               ← config NanoPi production
crates/daly-bms-server/src/             ← serveur principal RS485/API
crates/energy-manager/src/              ← gestionnaire énergie (remplace Node-RED)
  config.rs                             ← chargement [energy_manager] depuis Config.toml
  types.rs                              ← types partagés (EnergyState, MqttIncoming, ...)
  bus.rs                                ← AppBus (broadcast MQTT)
  main.rs                               ← démarrage séquentiel de tous les modules
  monitoring.rs                         ← métriques système + tokio → VictoriaMetrics
  logic/                                ← modules logiques métier (charge_current,
                                          deye_command, inverter, irradiance, meteo,
                                          platform, smartshunt, solar_power,
                                          switch_ats, tasmota, victron_keepalive,
                                          water_heater)
  mqtt/                                 ← client MQTT rumqttc + topics
  http_clients/                         ← Open-Meteo + LG ThinQ
  live_ws/                              ← WebSocket live events
  persist/                              ← restauration baselines au démarrage
crates/energy-manager/rules/            ← règles `.grl` (rust-rule-engine) :
                                          charge_current, deye_command, inverter,
                                          irradiance, smartshunt, solar_power,
                                          water_heater
crates/mqtt-broker/src/                 ← broker MQTT Rust (rumqttd) — remplace Mosquitto
  main.rs                               ← wrapper rumqttd + HTTP /metrics :8082
  mqtt-broker.toml                      ← config déployée dans /etc/daly-bms/
crates/mqtt-bridge/src/                 ← bridge bidirectionnel Pi5 ↔ NanoPi (rumqttc)
  main.rs                               ← entrée, métriques HTTP :8084
  bridge.rs                             ← demi-bridges nanopi→local et local→nanopi
  config.rs                             ← lecture [mqtt_bridge] depuis Config.toml
  metrics.rs                            ← compteurs atomiques + snapshot JSON
crates/dbus-mqtt-venus/src/             ← bridge MQTT→D-Bus NanoPi
contrib/daly-bms.service                ← unité systemd daly-bms-server
contrib/energy-manager.service          ← unité systemd energy-manager
contrib/node-exporter.service           ← unité systemd Prometheus node_exporter
```

**IMPORTANT** : Le service lit `/etc/daly-bms/config.toml`, PAS `~/Daly-BMS-Rust/Config.toml`.
Après toute modif → `sudo cp Config.toml /etc/daly-bms/config.toml`.

**IMPORTANT** : Les templates Askama (`templates/*.html`) sont compilés dans le binaire.
Tout changement HTML → `make build-arm` + redéploiement binaire obligatoire.

---

## 5. INVENTAIRE RS485 & D-BUS PRODUCTION

Bus `/dev/ttyUSB0` :

| Addr | Appareil | Type D-Bus | Topic MQTT | Instance |
|------|----------|-----------|------------|----------|
| 0x01 | BMS-360Ah | `battery.mqtt_1` | `bms/1/venus` | 151 |
| 0x02 | BMS-320Ah | `battery.mqtt_2` | `bms/2/venus` | 152 |
| 0x05 | PRALRAN irradiance | `meteo` | `irradiance/raw` | 40 |
| 0x07 | ET112-Micro-Onduleurs (SN 119253X) | `pvinverter.mqtt_7` | `pvinverter/7/venus` | 32 |
| 0x08 | ET112-Maison (SN 119215X) | `heatpump.mqtt_8` | `heatpump/8/venus` | 30 |
| 0x09 | ET112-Réseau (SN 061077X) | `heatpump.mqtt_9` | `heatpump/9/venus` | 31 |

Services D-Bus actifs nominaux :

```
com.victronenergy.battery.mqtt_1          BMS-360Ah (inst. 151)
com.victronenergy.battery.mqtt_2          BMS-320Ah (inst. 152)
com.victronenergy.pvinverter.mqtt_7       ET112-Micro-Onduleurs (inst. 32)
com.victronenergy.heatpump.mqtt_8         ET112-Maison / Consommation (inst. 30)
com.victronenergy.heatpump.mqtt_9         ET112-Réseau / Grid (inst. 31)
com.victronenergy.temperature.mqtt_1      Capteur ext. (type 4, inst. 20)
com.victronenergy.switch.mqtt_1           ATS CHINT (inst. 60)
com.victronenergy.switch.mqtt_2           Tongou Switch1 (inst. 61)
com.victronenergy.switch.mqtt_3           Tongou Switch2 (inst. 62)
com.victronenergy.switch.mqtt_4           Tongou Switch3 (inst. 63)
com.victronenergy.switch.mqtt_5           Tongou Switch4 (inst. 64)
com.victronenergy.switch.mqtt_6           Tongou Switch5 / tongou_3ACC34 (inst. 65)
com.victronenergy.meteo                   Irradiance PRALRAN + TodaysYield (inst. 40)
com.victronenergy.pvinverter.cgwacs_ttyUSB0_mb2   Onduleur PV Victron direct
```

Diagnostic rapide (NanoPi) :
```bash
ssh root@192.168.1.120 "dbus -y | grep victronenergy"
```

---

## 6. TOPICS MQTT (préfixe `santuario/`)

| Topic | Publié par | Service D-Bus NanoPi |
|-------|-----------|----------------------|
| `bms/{n}/venus` | daly-bms-server (Pi5) | `battery.mqtt_{n}` |
| `heat/{n}/venus` | daly-bms-server (Pi5) | `temperature.mqtt_{n}` |
| `heatpump/{n}/venus` | daly-bms-server (Pi5) | `heatpump.mqtt_{n}` |
| `switch/{n}/venus` | daly-bms-server (Pi5, ATS+Tongou) | `switch.mqtt_{n}` |
| `grid/{n}/venus` | daly-bms-server (Pi5) | `grid.mqtt_{n}` |
| `pvinverter/{n}/venus` | daly-bms-server (Pi5) | `pvinverter.mqtt_{n}` |
| `meteo/venus` | daly-bms-server (Pi5) | `meteo` (singleton) |

**TOUS les topics `santuario/*` sont publiés par Pi5 → bridgés vers NanoPi.**
**NanoPi ne publie RIEN en `santuario/*`.** → Ne jamais mettre `santuario/#` dans la direction NanoPi→Pi5 du bridge (boucle infinie).

### Règles bridge mqtt-bridge (CRITIQUE — anti-boucle)

| Direction | Topics autorisés | Raison |
|-----------|-----------------|--------|
| **local→nanopi** | `santuario/#`, `W/{portal}/#`, `R/{portal}/#`, `cmnd/#`, `shellypro2pm-.../rpc` | Pi5 → NanoPi uniquement |
| **nanopi→local** | `N/{portal}/#`, `tele/#`, `stat/#`, `shellypro2pm-.../#`, `daly-bms-shelly/rpc` | NanoPi → Pi5 uniquement |

⚠ **Un topic dans les deux sens = boucle infinie = flood MQTT = devices qui flashent dans VRM.**

### Commandes Tasmota/Tongou

| Action | Topic | Direction bridge |
|--------|-------|-----------------|
| Mesure énergie | `tele/{id}/SENSOR` | NanoPi→Pi5 |
| État relais | `stat/{id}/POWER` | NanoPi→Pi5 |
| Commande ON/OFF depuis Pi5 web | `cmnd/{id}/POWER` | Pi5→NanoPi |
| Commande depuis Venus console | `cmnd/{id}/Power` (dbus-mqtt-venus) | Direct NanoPi |

### ATS CHINT vs Tongou — mqtt_index

| mqtt_index | Device | Topic |
|-----------|--------|-------|
| 1 | ATS CHINT NXZB (RS485) | `santuario/switch/1/venus` |
| 2 | Tongou Switch1 (tongou_3BC764) | `santuario/switch/2/venus` |
| 3 | Tongou Switch2 (tongou_0A3FA0) | `santuario/switch/3/venus` |
| 4 | Tongou Switch3 (tongou_0A3C14) | `santuario/switch/4/venus` |
| 5 | Tongou Switch4 (tongou_0A4040) | `santuario/switch/5/venus` |
| 6 | Tongou Switch5 (tongou_3ACC34) | `santuario/switch/6/venus` |

---

## 7. API ENDPOINTS (extraits — voir `crates/daly-bms-server/src/api/mod.rs`)

```
# Système
GET  /api/v1/system/status            GET  /api/v1/system/totals
GET  /api/v1/system/logs              GET  /api/v1/config
GET  /api/v1/discover                 GET  /api/v1/irradiance/status
POST /api/v1/solar/mppt-yield

# Venus (lecture cache D-Bus / MQTT)
GET  /api/v1/venus/mppt               GET  /api/v1/venus/smartshunt
GET  /api/v1/venus/inverter           GET  /api/v1/venus/temperatures
GET  /api/v1/venus/heatpumps

# Monitor (RS485 health, logs)
GET  /api/v1/monitor/status           GET  /api/v1/monitor/rs485-health
GET  /api/v1/monitor/logs             GET  /api/v1/monitor/logs/content

# BMS — lecture
GET  /api/v1/bms/:id/status           GET  /api/v1/bms/:id/cells
GET  /api/v1/bms/:id/temperatures     GET  /api/v1/bms/:id/alarms
GET  /api/v1/bms/:id/mos              GET  /api/v1/bms/:id/history
GET  /api/v1/bms/:id/history/summary  GET  /api/v1/bms/:id/export/csv
GET  /api/v1/bms/compare              GET  /api/v1/bms/:id/settings

# BMS — écriture (api_key requis si configurée)
POST /api/v1/bms/:id/mos              POST /api/v1/bms/:id/soc
POST /api/v1/bms/:id/soc/full         POST /api/v1/bms/:id/soc/empty
POST /api/v1/bms/:id/reset
POST /api/v1/bms/:id/settings/cell-voltage-alarms
POST /api/v1/bms/:id/settings/pack-voltage-alarms
POST /api/v1/bms/:id/settings/current-alarms
POST /api/v1/bms/:id/settings/delta-alarms
POST /api/v1/bms/:id/settings/balancing

# ATS CHINT
GET  /api/v1/ats/status
POST /api/v1/ats/remote_on            POST /api/v1/ats/remote_off
POST /api/v1/ats/force_source1        POST /api/v1/ats/force_source2
POST /api/v1/ats/force_double         POST /api/v1/ats/send_raw
GET  /api/v1/ats/debug_on             GET  /api/v1/ats/debug_off

# ET112
GET  /api/v1/et112                    GET  /api/v1/et112/:addr/status
GET  /api/v1/et112/:addr/history

# Charts / History
GET  /api/v1/chart/history            GET  /api/v1/chart/edge-history
GET  /api/v1/history/energy

# Tasmota / Shelly
GET  /api/v1/tasmota                  GET  /api/v1/tasmota/:id/status
GET  /api/v1/tasmota/:id/history      POST /api/v1/tasmota/:id/control
GET  /api/v1/shelly                   GET  /api/v1/shelly/:id/status
POST /api/v1/shelly/:id/channel/:ch/control

# PromQL (compat Grafana)
GET  /api/v1/query                    GET  /api/v1/query_range
GET  /api/v1/labels

# Alertes
GET  /api/v1/alerts/list              GET  /api/v1/alerts/stats
POST /api/v1/alerts/:id/acknowledge

# Health + WebSocket
GET  /health
WS   /ws/bms/stream                   WS   /ws/bms/:id/stream
WS   /ws/venus/stream                 WS   /ws/console
```

Dashboard SSR (Askama) : `/dashboard`, `/dashboard/bms/:id`,
`/dashboard/et112`, `/dashboard/et112/:addr`, `/dashboard/tasmota`,
`/dashboard/tasmota/:id`, `/dashboard/ats`, `/dashboard/monitor`,
`/dashboard/console`, `/dashboard/visualization`, `/dashboard/history`,
`/dashboard/alerts`, `/dashboard/logs`, `/dashboard/settings`.

---

## 8. PROBLÈMES COURANTS

| Symptôme | Solution |
|----------|----------|
| `make sync` → "Permission denied" | `sudo chown -R pi5compute:pi5compute ~/Daly-BMS-Rust/ && git reset --hard origin/<branch>` |
| Service BMS ne démarre pas | `journalctl -u daly-bms -n 50` |
| Config ignorée | Copier vers `/etc/daly-bms/config.toml` |
| `scp: dest open Failure` | `ssh root@192.168.1.120 "svc -d /service/dbus-mqtt-venus"` puis redéployer |
| Venus symlink disparu (màj firmware) | `ssh root@192.168.1.120 "ln -sf /data/etc/sv/dbus-mqtt-venus /service/dbus-mqtt-venus"` |
| ET112 "en attente de données" | Mauvaise adresse Modbus → `sudo systemctl stop daly-bms && mbpoll -m rtu -a 1:15 -b 9600 -t 3:float -r 1 -c 1 /dev/ttyUSB0` |
| Widget météo "Température: -" | Limitation Venus OS — inévitable, non fixable |
| `mbpoll` sans réponse | daly-bms monopolise le port — `sudo systemctl stop daly-bms` d'abord |
| Dashboard affiche cumul brut | Vérifier `pvinv_baseline` retained MQTT (`santuario/persist/pvinv_baseline`) |
| energy-manager ne démarre pas | `journalctl -u energy-manager -n 50` — souvent TOML manquant ou `.env` absent |
| `missing field energy_manager` | `sudo cp Config.toml /etc/daly-bms/config.toml` — section `[energy_manager]` absente |
| energy-manager ne reçoit pas MQTT | Vérifier `portal_id` dans Config.toml et que mqtt-broker est actif (`systemctl status mqtt-broker`) |
| mqtt-broker ne démarre pas | `journalctl -u mqtt-broker -n 30` — souvent `/var/lib/mqtt-broker` owner incorrect |
| mqtt-bridge déconnecté NanoPi | `journalctl -u mqtt-bridge -n 30` — NanoPi inaccessible? `ping 192.168.1.120` |
| Retained messages perdus après migration | Normaux — recréés au prochain publish retain (ex: `pvinv_baseline` par energy-manager) |
| **BMS/pvinverter flashent dans VRM (apparaissent/disparaissent)** | **Boucle bridge — vérifier que `santuario/#` N'EST PAS dans nanopi→local (voir bridge.rs)** |
| **Flood MQTT (des centaines de messages/sec dans l'explorateur)** | **Boucle bridge — un topic est dans les deux directions simultanement** |
| **Commandes Tongou depuis web Pi5 ignorées** | **`cmnd/#` absent de local→nanopi dans bridge.rs** |
| Double messages venus (boucle bridge) | Vérifier que client_id des bridges sont uniques (déjà le cas dans le code) |
| LG ThinQ ne répond pas | Vérifier `LG_BEARER_TOKEN` et `LG_API_KEY` dans `/etc/daly-bms/.env` |

---

## 9. RÈGLES DE TRAVAIL

1. Lire ce fichier avant toute action.
2. `git branch` avant tout push — vérifier la branche.
3. Ne jamais déployer `daly-bms-server` sur NanoPi (uniquement `dbus-mqtt-venus`).
4. `sudo cp Config.toml /etc/daly-bms/config.toml` après toute modif config.
5. Arrêter `dbus-mqtt-venus` avant copie du binaire.
6. NanoPi = **armv7**, Pi5 = **aarch64** — ne pas confondre les binaires.
7. SSH vers NanoPi : `ssh root@192.168.1.120` (pas l'alias `nanopi`).
8. Templates Askama → `make build-arm` + redéploiement après tout changement HTML.
9. **CLAUDE.md = mémoire projet** : toute info découverte → ajouter ici + commit.
10. Nom exact D-Bus onduleur Victron direct : `cgwacs_ttyUSB0_mb2` (pas `rs485`).
11. **Docker supprimé** — plus de `docker-compose.infra.yml` ni de répertoire `docker/`. Si Mosquitto Docker tourne encore : `docker stop dalybms-mosquitto && docker rm dalybms-mosquitto`.
12. Secrets : ne jamais committer `.env`.
13. **MQTT** : broker = `mqtt-broker` (rumqttd systemd), plus de Docker. `make mqtt-start/stop/logs` remplace `make up/down/logs`.
14. Config bridge MQTT : `[mqtt_bridge]` dans Config.toml (portal_id + remote_host NanoPi).
15. **BRIDGE ANTI-BOUCLE** : `santuario/#` = direction Pi5→NanoPi SEULEMENT. Ne JAMAIS mettre `santuario/#` dans nanopi→local. Voir section 6 pour les règles complètes.
16. **Tasmota commandes** : Pi5 web publie `cmnd/{id}/POWER` sur broker local → bridge local→nanopi → switch sur NanoPi. `cmnd/#` doit être dans local→nanopi.

---

## 10. GUIDES COMPLÉMENTAIRES (lire sur demande)

| Besoin | Fichier |
|--------|---------|
| Ajouter un appareil / nouvelle métrique | `DASHBOARD_EXTENSION_GUIDE.md` |
| Procédures détaillées (NanoPi, maintenance, récupération firmware, production solaire) | `PROCEDURES.md` |
| Validation déploiement / checklist | `IMPLEMENTATION_VERIFICATION.md` |
| Debug MQTT | `MQTT_DEBUGGING_GUIDE.md` |
| Debug onduleur / SmartShunt | `DEBUG_ONDULEUR_SMARTSHUNT.md` |
| Guide energy-manager — modifier/ajouter/retirer une fonctionnalité | `docs/energy-manager-guide.md` |
| Migration MQTT / broker rumqttd / bridge / dashboard | `docs/mqtt-broker.md` |
| Architecture bus RS485 unifié (`rs485-bus` vs `daly-bms-core`) | `docs/architecture-rs485-bus.md` |
