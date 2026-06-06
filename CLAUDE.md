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
| Broker MQTT (status/logs) | `systemctl status mosquitto-broker` / `journalctl -u mosquitto-broker -f` |
| Taille base redb | `du -sh /mnt/nvme/daly-bms/metrics.redb` |
| Nb séries en base | `curl -s http://localhost:8080/api/v1/redb/series \| jq '.data \| length'` |
| Healthcheck backend | `curl -s http://localhost:8080/-/healthy` |
| Diag pic réseau (capture immédiate) | `sudo bash scripts/netdiag.sh` |
| Diag pic réseau (veille auto-capture) | `sudo bash scripts/netdiag.sh --watch` → rapport `/tmp/netdiag-*.txt` |
| Logs energy-manager | `journalctl -u energy-manager -f` |
| Compiler energy-manager | `make build-energy-arm` |
| Déployer energy-manager | `sudo systemctl stop energy-manager && sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/ && sudo systemctl start energy-manager` |
| Appliquer Config energy-manager | `sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart energy-manager` |

### Perses (Pi5, port 8090)

> ⚠️ Perses a été remplacé par le dashboard custom interne (`/dashboard/history`).
> Tous les fichiers et scripts d'installation ont été retirés.

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

### Grafana (Pi5, port 3000)

| Quand | Commande |
|-------|----------|
| Installer Grafana | `sudo bash scripts/setup-grafana.sh --nvme` |
| Déployer dashboards | Inclus dans `bash scripts/deploy-pi5.sh` |
| Redémarrer Grafana | `sudo systemctl restart grafana-server` |
| Logs Grafana | `journalctl -u grafana-server -f` |
| Healthcheck | `curl -s http://localhost:3000/api/health` |
| Supprimer dossier vide | Via UI Grafana : Dashboards → dossier → Delete |

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
3g. Grafana dashboards  : bash scripts/deploy-pi5.sh (ou manuellement : sudo cp contrib/grafana/dashboards/*.json /var/lib/grafana/dashboards/ && sudo systemctl restart grafana-server)
```

---

## 1. ARCHITECTURE

```
Pi5 (192.168.1.141, pi5compute)
  mosquitto-broker (systemd, :1883 + :9001 WS)
    └── bridge unique pi5-nanopi → 192.168.1.120:1883
  daly-bms-server (systemd, :8080)
    ├── RS485 /dev/ttyUSB0 → 2 BMS + 3 ET112 + 1 PRALRAN
    ├── REST API + WebSocket :8080
    ├── PromQL compat (/api/v1/query, /api/v1/query_range) ← Grafana datasource
    ├── MQTT subscribe/publish → 127.0.0.1:1883 (broker local)
    └── metrics-store (redb à /mnt/nvme/daly-bms/metrics.redb)
  energy-manager (systemd, :8081)
    ├── MQTT subscribe/publish → 127.0.0.1:1883 (broker local)
    ├── Logique solaire, DEYE, chauffe-eau, charge, météo
    ├── WebSocket live events :8081/live
    └── publication MQTT → consommée par daly-bms-server (writes metrics-store)
  grafana-server (systemd, :3000)
    ├── Datasource : "Daly Metrics (redb)" → http://127.0.0.1:8080 (UID: daly-metrics)
    ├── 20 dashboards provisionés dans /var/lib/grafana/dashboards/
    └── Données NVMe optionnel (/mnt/nvme/grafana)

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
  monitoring.rs                         ← métriques système + tokio (TODO : republier dans metrics-store)
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
crates/dbus-mqtt-venus/src/             ← bridge MQTT→D-Bus NanoPi
contrib/daly-bms.service                ← unité systemd daly-bms-server
contrib/energy-manager.service          ← unité systemd energy-manager
contrib/grafana/                        ← provisioning Grafana complet
  dashboards/01-bms.json … 20-alertes-avancees.json ← 20 dashboards JSON
    (17→20 = dashboards évolués PromQL avancé : flotte/SLO, rendement PV,
     bilan énergie J/J-1, alertes multi-critères — cf. docs/Evolution-compliance-PromQL.md §9)
  provisioning/datasources/daly-metrics.yaml        ← datasource PromQL → :8080
  provisioning/dashboards/daly-bms.yaml             ← provider → /var/lib/grafana/dashboards
scripts/setup-grafana.sh                ← installation Grafana (première fois)
scripts/deploy-pi5.sh                   ← déploiement complet (binaires + Grafana + validation)
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
| 0x03 | BMS-620Ah | `battery.mqtt_3` | `bms/3/venus` | 153 |
| 0x05 | PRALRAN irradiance | `meteo` | `irradiance/raw` | 40 |
| 0x07 | ET112-Micro-Onduleurs (SN 119253X) | `pvinverter.mqtt_7` | `pvinverter/7/venus` | 32 |
| 0x08 | ET112-Maison (SN 119215X) | `heatpump.mqtt_8` | `heatpump/8/venus` | 30 |
| 0x09 | ET112-Réseau (SN 061077X) | `heatpump.mqtt_9` | `heatpump/9/venus` | 31 |

Services D-Bus actifs nominaux :

```
com.victronenergy.battery.mqtt_1          BMS-360Ah (inst. 151)
com.victronenergy.battery.mqtt_2          BMS-320Ah (inst. 152)
com.victronenergy.battery.mqtt_3          BMS-620Ah (inst. 153)
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

| Topic | Service D-Bus |
|-------|---------------|
| `bms/{n}/venus` | `battery.mqtt_{n}` |
| `heat/{n}/venus` | `temperature.mqtt_{n}` |
| `heatpump/{n}/venus` | `heatpump.mqtt_{n}` |
| `switch/{n}/venus` | `switch.mqtt_{n}` |
| `grid/{n}/venus` | `grid.mqtt_{n}` |
| `pvinverter/{n}/venus` | `pvinverter.mqtt_{n}` |
| `meteo/venus` | `meteo` (singleton) |

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
| `deploy-pi5.sh` → `rustup: not found` | PATH root ≠ PATH user sous `sudo`. Corrigé : le script build via `as_user` (sous `$SUDO_USER`). Sinon : builder **sans sudo** (`make build-arm && make build-energy-arm`) puis `sudo bash scripts/deploy-pi5.sh --no-build`. Dashboards seuls : `sudo bash scripts/fix-grafana.sh`. |
| Service BMS ne démarre pas | `journalctl -u daly-bms -n 50` |
| Config ignorée | Copier vers `/etc/daly-bms/config.toml` |
| `scp: dest open Failure` | `ssh root@192.168.1.120 "svc -d /service/dbus-mqtt-venus"` puis redéployer |
| Venus symlink disparu (màj firmware) | `ssh root@192.168.1.120 "ln -sf /data/etc/sv/dbus-mqtt-venus /service/dbus-mqtt-venus"` |
| ET112 "en attente de données" | Mauvaise adresse Modbus → `sudo systemctl stop daly-bms && mbpoll -m rtu -a 1:15 -b 9600 -t 3:float -r 1 -c 1 /dev/ttyUSB0` |
| Dashboard Grafana ET112 vide alors que les données existent | **Format du label `address`** : le backend écrit `address="0x07/0x08/0x09"` (hex, `redb_writes.rs::write_et112`). Les requêtes PromQL doivent utiliser `address="0x07"`, **jamais** `address="7"` (décimal → 0 série). Vérif : `curl -s 'localhost:8080/api/v1/query?query=et112_power_w' \| jq '.data.result[].metric'`. |
| Widget météo "Température: -" | Limitation Venus OS — inévitable, non fixable |
| `mbpoll` sans réponse | daly-bms monopolise le port — `sudo systemctl stop daly-bms` d'abord |
| Dashboard affiche cumul brut | Vérifier `pvinv_baseline` retained MQTT (`santuario/persist/pvinv_baseline`) |
| energy-manager ne démarre pas | `journalctl -u energy-manager -n 50` — souvent TOML manquant ou `.env` absent |
| `missing field energy_manager` | `sudo cp Config.toml /etc/daly-bms/config.toml` — section `[energy_manager]` absente |
| energy-manager ne reçoit pas MQTT | Vérifier `portal_id` dans Config.toml et que Mosquitto est accessible sur `mqtt.host` |
| LG ThinQ ne répond pas | Vérifier `LG_BEARER_TOKEN` et `LG_API_KEY` dans `/etc/daly-bms/.env` |
| Grafana dossier vide "No items" | Dashboards au mauvais format (export vs provisioning) — `__inputs`/`__requires` doivent être absents, datasource UID = `daly-metrics` |
| Grafana ne démarre pas | `journalctl -u grafana-server -n 50` — souvent YAML provisioning invalide |
| Grafana "datasource not found" | Vérifier `/etc/grafana/provisioning/datasources/daly-metrics.yaml` présent, supprimer `victoriametrics.yaml` si résiduel |
| Grafana ancien dossier "PV Solaire" vide | Supprimer manuellement via UI Grafana (Dashboards → PV Solaire → Delete) |

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
11. Broker MQTT = Mosquitto natif systemd (`mosquitto-broker.service`). Plus de Docker — config dans `contrib/mosquitto/mosquitto.conf`, déployée vers `/etc/mosquitto/mosquitto.conf`. Toujours valider avec `sudo /usr/local/bin/verify-no-loop.sh` après modif des topics bridge.
12. Secrets : ne jamais committer `.env`.
13. **Source de vérité métrique** : les valeurs mesurées par Victron (D-Bus/MQTT) et lues sur RS485 sont **prioritaires sur tout calcul dérivé**. Ne jamais remplacer une mesure firmware par un V×I recalculé, ni écraser un champ direct par un agrégat système. Les sommes (`solar_total = mppt+pvinv`) sont OK car ce sont des agrégats explicites, pas des recalculs d'une valeur déjà disponible.
14. **Dashboards Grafana** : les 15 JSON dans `contrib/grafana/dashboards/` doivent être au format **provisioning** (pas export). Ne jamais inclure `__inputs`/`__requires`. Le datasource UID doit être `daly-metrics` (pas `${datasource}`). `scripts/deploy-pi5.sh` déploie automatiquement.

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
| Grafana — 20 dashboards (liste, métriques, provisioning) | `contrib/grafana/` + `scripts/setup-grafana.sh` |
