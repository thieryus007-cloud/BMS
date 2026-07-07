# CLAUDE.md — Référence Projet Daly-BMS-Rust

> Chargé automatiquement à chaque session. Garder concis.
> Index complet de la doc → **docs/ARCHITECTURE.md** (lire sur demande).
> 
> **ATTENTION**
> Toujours faire un PLAN et decomposer les taches pour ne pas avoir "API Error: Stream idle timeout - partial response received"

---

## 0. COMMANDES RAPIDES

### Pi5 (`~/Daly-BMS-Rust`, user: pi5compute)

| Quand | Commande |
|-------|----------|
| **Déploiement Pi5 (script unique)** | `sudo bash scripts/deploy-pi5.sh` (sync + build + config + unités systemd + binaires ciblés + Grafana/mosquitto + validation). Aperçu : `--dry-run`. Pousser la config repo : `--apply-config`. |
| Récupérer le code | `make sync` |
| Appliquer Config.toml | `sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart daly-bms` |
| Logs BMS | `journalctl -u daly-bms -f` |
| Compiler Pi5 | `make build-arm` |
| Déployer binaire Pi5 | `sudo systemctl stop daly-bms && sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/ && sudo systemctl start daly-bms` |
| Broker MQTT (status/logs) | `systemctl status mosquitto-broker` / `journalctl -u mosquitto-broker -f` |
| Taille base redb | `du -sh /mnt/nvme/daly-bms/metrics.redb` |
| Compacter la base redb (réduit le fichier, garde l'historique ; service arrêté pdt l'op) | `sudo bash scripts/compact-redb.sh 7` (abaisse raw_retention_days à 7 + tiering + compaction physique) |
| Nettoyer disque (build) | `rm -rf target/aarch64-unknown-linux-gnu/release-symbols target/armv7-unknown-linux-gnueabihf target/debug target/release ~/.cargo/registry/cache ~/.cargo/registry/src /tmp/jeprof /tmp/jeprof.*.heap && sudo apt-get clean && sudo journalctl --vacuum-size=200M` (garde `target/aarch64-…/release` = cache prod ; `release-symbols`/jeprof = artefacts de diagnostic régénérables). Reset total : `cargo clean` |
| Nb séries en base | `curl -s http://localhost:8080/api/v1/redb/series \| jq '.data \| length'` |
| Healthcheck backend | `curl -s http://localhost:8080/-/healthy` |
| Valider Config.toml avant déploiement | `DALY_CONFIG=Config.toml daly-bms-server --check-config` et `ENERGY_CONFIG=Config.toml energy-manager --check-config` (dry-run : parse + bornes + typos — audit 2026-06 §12) |
| Diag pic réseau (capture immédiate) | `sudo bash scripts/netdiag.sh` |
| Profiler une fuite RSS (heap jemalloc, auto-restauré) | `sudo bash scripts/jemalloc-leak-profile.sh 2h` → rapport `/tmp/jeprof/leak-report-*.txt` (cf. docs/diagnostic-depannage.md §18) |
| Diag pic réseau (veille auto-capture) | `sudo bash scripts/netdiag.sh --watch` → rapport `/tmp/netdiag-*.txt` |
| Logs energy-manager | `journalctl -u energy-manager -f` |
| Taille du journal systemd | `journalctl --disk-usage` |
| Plafonner le journal (déjà fait par deploy-pi5.sh) | drop-in `contrib/journald/daly-bms.conf` → `/etc/systemd/journal.conf.d/` (SystemMaxUse=200M) ; purge immédiate : `sudo journalctl --vacuum-size=200M` |
| Compiler energy-manager | `make build-energy-arm` |
| Déployer energy-manager | `sudo systemctl stop energy-manager && sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/ && sudo systemctl start energy-manager` |
| Appliquer Config energy-manager | `sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart energy-manager` |

### Perses (Pi5, port 8090)

> ⚠️ Perses a été remplacé par Grafana (`:3000`). La page custom `/dashboard/history`
> et les API `/api/v1/dashboards/*` + `/api/v1/history/energy` ont été retirées
> (2026-06, axe « churn mémoire » — Grafana est l'unique outil d'historique).

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

## ⇒ VOIE UNIQUE Pi5 : sudo bash scripts/deploy-pi5.sh
##   sync → build → config (PRÉSERVÉE par défaut) → unités systemd (daemon-reload)
##   → mosquitto/Grafana → déploiement CIBLÉ (ne redémarre QUE ce qui a changé :
##   binaire OU unité OU config) → validation. Flags : --dry-run (aperçu, aucune
##   écriture), --apply-config (pousser Config.toml du repo, avec backup),
##   --no-sync / --no-build / --no-validate.
##   NanoPi (dbus-mqtt-venus) reste séparé : make build-venus-v7 && make install-venus-v7

# Détail manuel (si besoin de cibler une seule étape) :
3a. Config seule        : sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart daly-bms
3b. Code Rust/HTML      : make build-arm → stop → cp binaire → start
3c. Venus code          : make build-venus-v7 && make install-venus-v7
3d. Config NanoPi       : scp nanoPi/config-nanopi.toml root@192.168.1.120:/data/daly-bms/config.toml && ssh root@192.168.1.120 "svc -t /service/dbus-mqtt-venus"
3e. energy-manager code : make build-energy-arm → sudo systemctl stop energy-manager → sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/ → sudo systemctl start energy-manager
3f. Config seule (energy): sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart energy-manager
3g. Grafana dashboards  : bash scripts/deploy-pi5.sh (ou manuellement : sudo cp contrib/grafana/dashboards/*.json /var/lib/grafana/dashboards/ && sudo systemctl restart grafana-server)
```

> **Script de déploiement unique** : `scripts/deploy-pi5.sh` (l'ancien duo
> `deploy.sh`/`deploy-pi5.sh` a été fusionné — `deploy.sh` oubliait les unités
> systemd, d'où des limites mémoire non appliquées). Il combine déploiement
> ciblé (ne redémarre que ce qui a changé), `--dry-run`, et **préserve**
> `Config.toml` par défaut (`--apply-config` pour pousser le repo avec backup).

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
    ├── 22 dashboards provisionés dans /var/lib/grafana/dashboards/
    └── Données NVMe optionnel (/mnt/nvme/grafana)

NanoPi (192.168.1.120, root)
  dbus-mqtt-venus (runit /service/dbus-mqtt-venus)
    └── MQTT subscribe → D-Bus Victron (com.victronenergy.*)
```

---

## 2. RÉSEAU & SSH

| Machine | IP | User | Lien |
|---------|----|------|------|
| Pi5 | 192.168.1.141 | pi5compute | WiFi `StarTh` (Starlink) — IP **fixe** via profil NetworkManager |
| NanoPi | 192.168.1.120 | root | WiFi `StarTh` |

SSH Pi5 config (`~/.ssh/config`): clé `~/.ssh/id_nanopi` → `Host nanopi` + `Host 192.168.1.120` (les deux entrées nécessaires).

**Tous les appareils sont en WiFi sur la box Starlink (SSID `StarTh`, WPA2, passerelle `192.168.1.1`).** Le Pi5 n'a **aucun service réseau dépendant du WiFi pour booter** : `daly-bms` lit le RS485 (USB) même sans réseau → un Pi5 « vivant mais injoignable » est presque toujours un problème WiFi, pas un crash.

**IP fixe Pi5 (NetworkManager)** : l'adresse `192.168.1.141` est figée dans le profil WiFi `StarTh` (`ipv4.method manual`), pas via bail DHCP. MAC WiFi du Pi5 : `88:a2:9e:37:ed:bc` (pour réservation DHCP côté Starlink). Le profil vit dans `/etc/NetworkManager/system-connections/StarTh.nmconnection`.

**Accès de secours si le WiFi tombe** : brancher un **câble Ethernet** entre le Pi5 (port RJ45 intégré) et la box/un switch → `eth0` prend une IP DHCP automatiquement → `ssh pi5compute@<ip_eth0>`. Procédure complète de récupération/fiabilisation WiFi → **docs/diagnostic-depannage.md §10**.

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
  logic/<module>/rules.rs               ← logique de décision **Rust pur** (fonctions
                                          pures + tests). Remplace l'ancien moteur
                                          `rust-rule-engine`/`.grl` (retiré 2026-06 :
                                          no-loop renvoyait des résultats faux ; cf.
                                          docs/app-energy-manager.md §4)
crates/dbus-mqtt-venus/src/             ← bridge MQTT→D-Bus NanoPi
contrib/daly-bms.service                ← unité systemd daly-bms-server
contrib/energy-manager.service          ← unité systemd energy-manager
contrib/journald/daly-bms.conf          ← drop-in journald (plafond journal : SystemMaxUse=200M)
                                          déployé par deploy-pi5.sh → /etc/systemd/journal.conf.d/
contrib/grafana/                        ← provisioning Grafana complet
  dashboards/01-bms.json … 22-toshiba-clim.json ← 22 dashboards JSON
    (17→20 = dashboards évolués PromQL avancé : flotte/SLO, rendement PV,
     bilan énergie J/J-1, alertes multi-critères — cf. docs/metriques-promql-reference.md §9 ;
     21 = mémoire process RSS/jemalloc — diagnostic fuite, cf. docs/diagnostic-depannage.md §17 ;
     22 = climatiseurs Toshiba, séries toshiba_ac_* labellisées par zone — cf. docs/toshiba-suzumi-rs-plan.md §0.4)
  provisioning/datasources/daly-metrics.yaml        ← datasource PromQL → :8080
  provisioning/dashboards/daly-bms.yaml             ← provider → /var/lib/grafana/dashboards
scripts/setup-grafana.sh                ← installation Grafana (première fois)
scripts/deploy-pi5.sh                   ← SCRIPT DE DÉPLOIEMENT UNIQUE Pi5 : sync + build + config
                                          + unités systemd + mosquitto/Grafana + déploiement CIBLÉ
                                          (ne redémarre que ce qui a changé) + validation.
                                          Flags : --dry-run, --apply-config, --no-sync/--no-build/--no-validate.
                                          ⚠ PRÉSERVE Config.toml par défaut (--apply-config pour pousser le repo + backup).
                                          ⚠ NE déploie PAS le NanoPi → `make install-venus-v7` séparément.
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
| 0x08 | ET112-Maison (SN 119215X) | `acload.mqtt_8` | `grid/8/venus` | 30 |
| 0x09 | ET112-Réseau (SN 061077X) | `grid.mqtt_9` | `grid/9/venus` | 31 |

Services D-Bus actifs nominaux :

```
com.victronenergy.battery.mqtt_1          BMS-360Ah (inst. 151)
com.victronenergy.battery.mqtt_2          BMS-320Ah (inst. 152)
com.victronenergy.battery.mqtt_3          BMS-620Ah (inst. 153)
com.victronenergy.pvinverter.mqtt_7       ET112-Micro-Onduleurs (inst. 32)
com.victronenergy.acload.mqtt_8           ET112-Maison / Consommation AC (inst. 30)
com.victronenergy.grid.mqtt_9             ET112-Réseau / Compteur réseau EDF (inst. 31)
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

> **Switches Tongou** : **tous du MÊME modèle** — disjoncteurs/switchs intelligents
> **flashés Tasmota** qui **mesurent TOUT** (tension, courant, **puissance W**, **énergie
> kWh**, + protections). Visibles sur la **page Tasmota** du dashboard
> (`/dashboard/tasmota`, API `GET /api/v1/tasmota`) ; télémétrie `tele/<id>/SENSOR`
> parsée par `energy-manager` `logic/tasmota` (`power`/`today`). → Pour mesurer la conso
> d'un appareil (ex. clim Toshiba multi‑split sur l'unité extérieure), poser un Tongou
> sur son alim suffit — pas de capteur additionnel. Cf. `docs/integration-toshiba-shorai-esphome.md` §7.

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

Topics internes `santuario/em/*` (energy-manager → daly-bms-server, pas de D-Bus) :
`em/metrics` (système EM), `em/water_heater` (LG ThinQ), `em/solar` (télémétrie
solaire 1 Hz — remplace l'ancien POST HTTP `/api/v1/solar/mppt-yield`, conservé
en fallback ; cf. docs/diagnostic-depannage.md §17).

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
`/dashboard/console`, `/dashboard/visualization`,
`/dashboard/alerts`, `/dashboard/logs`, `/dashboard/settings`.

---

## 8. PROBLÈMES COURANTS

| Symptôme | Solution |
|----------|----------|
| `make sync` → "Permission denied" | `sudo chown -R pi5compute:pi5compute ~/Daly-BMS-Rust/ && git reset --hard origin/<branch>` |
| `deploy-pi5.sh` → `rustup: not found` | PATH root ≠ PATH user sous `sudo`. Corrigé : le script build via `as_user` (sous `$SUDO_USER`). Sinon : builder **sans sudo** (`make build-arm && make build-energy-arm`) puis `sudo bash scripts/deploy-pi5.sh --no-build`. Dashboards seuls : `sudo bash scripts/fix-grafana.sh`. |
| **Pi5 injoignable après reboot** (mais RS485 clignote → OS up) | **WiFi non remonté.** Souvent le profil WiFi NetworkManager a disparu (reset config). Accès de secours : **câble Ethernet** Pi5→box → `ssh pi5compute@<ip_eth0>`. Recréer le profil + IP fixe : voir docs/diagnostic-depannage.md §10. Diag rapide : `nmcli connection show` (profil `StarTh` présent ?), `nmcli device wifi list` (SSID vu ?), `ip -br a` (`wlan0` a-t-il une IP ?). |
| Service BMS ne démarre pas | `journalctl -u daly-bms -n 50` |
| Config ignorée | Copier vers `/etc/daly-bms/config.toml` |
| `scp: dest open Failure` | `ssh root@192.168.1.120 "svc -d /service/dbus-mqtt-venus"` puis redéployer |
| Venus symlink disparu (màj firmware) | `ssh root@192.168.1.120 "ln -sf /data/etc/sv/dbus-mqtt-venus /service/dbus-mqtt-venus"` |
| Journal systemd qui grossit (`journalctl --disk-usage`) | **Comportement normal** (journald grossit jusqu'au plafond puis rotationne). Plafond explicite borné à 200M via `contrib/journald/daly-bms.conf` (déployé par `deploy-pi5.sh`). Bruit réduit à la source : `info!→debug!` sur les boucles irradiance (30 s) et water_heater (5 min) — 2026-06. Détail → docs/diagnostic-depannage.md §11. Purge manuelle : `sudo journalctl --vacuum-size=200M`. |
| Disque racine Pi5 se remplit (`df -h /` > 45 %) | Builds Rust cumulés dans `target/` (aarch64 + armv7 + natif debug/release). Les binaires prod sont dans `/usr/local/bin` → `target/` est jetable. Voir « Nettoyer disque (build) » §0. Ne **jamais** supprimer `~/.cargo/bin`, `/usr/local/bin/*`, ni `/mnt/nvme/.../metrics.redb`. |
| `dbus-mqtt-venus` crash-loop sur NanoPi (`svstat` uptime=0, tous les services D-Bus absents) | Binaire armv7 mal compilé → **SIGILL** (exit 132). Cause : `target-cpu=native` dans le build armv7 (hôte aarch64 ≠ cible armv7). Corrigé dans le Makefile. Diag : lancer le binaire à la main sur le NanoPi. Indice au build : warnings `'+lse' is not a recognized feature`. |
| `install-venus.sh: Permission denied` (`make install-venus-v7`) | Bit +x manquant → `chmod +x nanoPi/install-venus.sh` ou déployer via `ARCH=armv7 bash nanoPi/install-venus.sh 192.168.1.120`. |
| Compteur grid/acload affiche L2/L3 fantômes à 0 W dans VRM | ET112 monophasé : `grid_service` n'expose que les phases présentes via `/Ac/NumberOfPhases` (dérivé du payload). Si L2/L3 persistent → rafraîchir VRM (cache console). |
| ET112 "en attente de données" | Mauvaise adresse Modbus → `sudo systemctl stop daly-bms && mbpoll -m rtu -a 1:15 -b 9600 -t 3:float -r 1 -c 1 /dev/ttyUSB0` |
| Dashboard Grafana ET112 vide alors que les données existent | **Format du label `address`** : le backend écrit `address="0x07/0x08/0x09"` (hex, `redb_writes.rs::write_et112`). Les requêtes PromQL doivent utiliser `address="0x07"`, **jamais** `address="7"` (décimal → 0 série). Vérif : `curl -s 'localhost:8080/api/v1/query?query=et112_power_w' \| jq '.data.result[].metric'`. |
| Widget météo "Température: -" | Limitation Venus OS — inévitable, non fixable |
| `mbpoll` sans réponse | daly-bms monopolise le port — `sudo systemctl stop daly-bms` d'abord |
| `/dev/ttyUSB0` devient `ttyUSB1` après débranchement USB | Chemin udev stable : `ls -l /dev/serial/by-id/` → `[serial] port = "/dev/serial/by-id/usb-…"` (suit le périphérique à la ré-énumération, `reopen()` inchangé). Cf. docs/integration-materiel.md §2.1 |
| RSS de daly-bms-server qui croît (suspicion fuite) | **Avant tout : ce N'EST probablement PAS une fuite.** Le RSS se stabilise seul à un palier fixé par `raw_retention_days` (état interne redb ∝ taille du fichier) — mesures §21. 7/30 j → ~100 Mo, 60 j → ~96 Mo. Pour baisser le palier : réduire `raw_retention_days` + `scripts/compact-redb.sh`. Dashboard « 21 - Mémoire daly-bms » pour confirmer la stabilisation. Cf. docs/diagnostic-depannage.md §21 (qui infirme §18/§20) |
| Une source ne se rafraîchit plus (valeurs figées, aucune erreur) | Requêter `source_last_update_age_seconds{source=...}` (bms_0x01, et112_0x07, venus_mppt, irradiance, ats…) et `em_source_last_update_age_seconds` (open_meteo, lg_thinq). Âge > 5× l'intervalle de polling = source morte (audit 2026-06 §18) |
| metrics-store ne s'ouvre plus après coupure brutale | Quarantaine auto au boot : base corrompue renommée `metrics.redb.corrupt.<ts>` + base vide recréée (audit §15). L'ancien fichier reste sur le NVMe pour autopsie |
| Dashboard affiche cumul brut | Vérifier `pvinv_baseline` retained MQTT (`santuario/persist/pvinv_baseline`) |
| energy-manager ne démarre pas | `journalctl -u energy-manager -n 50` — souvent TOML manquant ou `.env` absent |
| `missing field energy_manager` | `sudo cp Config.toml /etc/daly-bms/config.toml` — section `[energy_manager]` absente |
| energy-manager ne reçoit pas MQTT | Vérifier `portal_id` dans Config.toml et que Mosquitto est accessible sur `mqtt.host` |
| Micro-coupures AC Out au passage 51,5 Hz (auto-coupure DEYE) | Le Shelly doit couper les DEYE **avant** leur auto-trip 51,5 Hz. **Décision = Fréquence AC + état MPPT UNIQUEMENT** (ni réseau, ni SmartShunt). Seuil de fréquence **unique** (pas de zone morte) : `[energy_manager.deye] freq_high_hz=51.0` (coupe ≥51,0 / restaure <51,0) + `freq_hard_hz=51.3` (coupure immédiate). Coupe aussi dès qu'un MPPT passe en 4/5/6 (`mppt_full_states`, `mppt_cut_enabled=true`). **Décision STATELESS en Rust pur** (`logic/deye_command/rules.rs::decide`, plus d'état `Lockout` latchable — 2026-06) : le Rust pré-calcule les flags (`DeyeController::flags()`), `decide()` les mappe vers `desired_on`, ré-évalué **chaque seconde** ; le relais SUIT en continu `OFF si (freq≥51,0 OU mppt_full), ON sinon` → **ne peut jamais rester coincé**. Anti-rebattement = 2 débounces : `cut_delay_secs=3` (coupe douce) + `reenable_delay_secs=45` (restauration) ; coupure immédiate à `freq_hard_hz=51.3`. Canaux : `[energy_manager.victron] shelly_deye_channels=[0,1]` (un par DEYE — **les deux**). Détail → docs/app-energy-manager.md §4.3. |
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
6. NanoPi = **armv7**, Pi5 = **aarch64** — ne pas confondre les binaires. **Jamais** `target-cpu=native` pour le build armv7 (l'hôte est aarch64 → SIGILL sur le NanoPi). Le Makefile build armv7 n'utilise que `-C link-arg=-Wl,--as-needed`.
7. SSH vers NanoPi : `ssh root@192.168.1.120` (pas l'alias `nanopi`).
8. Templates Askama → `make build-arm` + redéploiement après tout changement HTML.
9. **CLAUDE.md = mémoire projet** : toute info découverte → ajouter ici + commit.
10. Nom exact D-Bus onduleur Victron direct : `cgwacs_ttyUSB0_mb2` (pas `rs485`).
11. Broker MQTT = Mosquitto natif systemd (`mosquitto-broker.service`). Plus de Docker — config dans `contrib/mosquitto/mosquitto.conf`, déployée vers `/etc/mosquitto/mosquitto.conf`. Toujours valider avec `sudo /usr/local/bin/verify-no-loop.sh` après modif des topics bridge.
12. Secrets : ne jamais committer `.env`.
13. **Source de vérité métrique** : les valeurs mesurées par Victron (D-Bus/MQTT) et lues sur RS485 sont **prioritaires sur tout calcul dérivé**. Ne jamais remplacer une mesure firmware par un V×I recalculé, ni écraser un champ direct par un agrégat système. Les sommes (`solar_total = mppt+pvinv`) sont OK car ce sont des agrégats explicites, pas des recalculs d'une valeur déjà disponible.
14. **Dashboards Grafana** : les 21 JSON dans `contrib/grafana/dashboards/` doivent être au format **provisioning** (pas export). Ne jamais inclure `__inputs`/`__requires`. Le datasource UID doit être `daly-metrics` (pas `${datasource}`). `scripts/deploy-pi5.sh` déploie automatiquement.
15. **CI** : `.github/workflows/ci.yml` (build natif + tests + `clippy -D warnings` + cross-build aarch64/armv7) garde le code vert. Toolchain épinglée dans `rust-toolchain.toml` (1.94.1). Le cross-build armv7 n'utilise **jamais** `target-cpu=native` (cf. SIGILL §8). Garder clippy propre ; pour faire taire un lint, `#[allow(...)]` ciblé.
16. **Supervision (fail-fast)** : les boucles de service longue durée passent par `spawn_critical` (helper dans chaque binaire ; `supervise.rs` pour energy-manager). Si une boucle retourne (ou panique, via `panic=abort`), le process s'arrête → redémarrage par systemd (`Restart=on-failure`) / runit. **Ne jamais** `spawn_critical` une tâche transitoire (one-shot, timer, traitement par-snapshot) : elle se termine normalement et provoquerait un exit. Conséquence : plus de bridge/poll mort silencieux pendant que le service paraît « up ».
17. **Réouverture port série** : `SharedBus::reopen()` rouvre `/dev/ttyUSB0` après déconnexion USB / ré-énumération. `poll_loop` la déclenche sur `DalyError::Serial` **et** `DalyError::Io` (backoff + reopen). Bus partagé → ET112/ATS/PRALRAN repartent aussi. Plus besoin de redémarrer le service à la main après un débranchement USB.

---

## 10. GUIDES COMPLÉMENTAIRES (lire sur demande)

> 📐 **Point d'entrée : [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)** — vue d'ensemble + index complet.

| Besoin | Document |
|--------|----------|
| **Vue d'ensemble système + index de toute la doc** | `docs/ARCHITECTURE.md` |
| Serveur principal (RS485, protocole Daly, API REST/WS, dashboard SSR) | `docs/app-daly-bms-server.md` |
| energy-manager (modules `logic/`, décisions Rust pur `rules.rs`, modifier/ajouter/retirer) | `docs/app-energy-manager.md` |
| Bridge Venus OS + ajouter un device D-Bus + déploiement armv7 | `docs/app-dbus-mqtt-venus.md` |
| Déployer (Pi5 + NanoPi), procédures détaillées, restauration git | `docs/deploiement-exploitation.md` |
| Architecture redb (schéma, tiering) + historique migration VM→redb | `docs/metriques-redb-architecture.md` |
| Catalogue des métriques + requêtes & conformité PromQL | `docs/metriques-promql-reference.md` |
| Grafana — 22 dashboards (liste, datasource, provisioning) | `docs/grafana-dashboards.md` |
| MQTT / Mosquitto (topics, bridge, anti-boucle, migration Docker→natif) | `docs/mqtt-mosquitto.md` |
| Alertes (AlertEngine natif, règles, hysteresis, notifications) | `docs/alertes.md` |
| Ajouter un appareil / BMS Daly, ATS CHINT, ET112, PRALRAN | `docs/integration-materiel.md` |
| **Clim Toshiba SHORAI EDGE — VOIE RETENUE = firmware RUST natif ESP32** (protocole SUZUMI CN22 vérifié vs pedobry + o0Zz ; PAS ESPHome). **Reprise de session → §0 du doc.** Crate détaché `firmware/toshiba-suzumi-rs/` (couche protocole pure faite + testée host ; ESP32 en attente matériel). Test : `cargo test --manifest-path firmware/toshiba-suzumi-rs/Cargo.toml` | `docs/toshiba-suzumi-rs-plan.md` |
| Clim Toshiba — **référence câblage/MQTT** (BOM, brochage CN22, topics `santuario/toshiba`, module EM `logic/toshiba_ac`, conso Tongou) — ⚠️ voie ESPHome **non retenue**, YAML §4 obsolète | `docs/integration-toshiba-shorai-esphome.md` |
| Dépannage, netdiag réseau, debug onduleur/SmartShunt, memory-leak | `docs/diagnostic-depannage.md` |
| **Audit robustesse 2026-06** — 18 axes (§3-§18 implémentés ; §1-§2 sécurité en attente d'action utilisateur) | `docs/audit-robustesse-2026-06.md` |
