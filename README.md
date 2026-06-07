# Daly-BMS — Rust Edition

Supervision et automatisation d'un système de stockage d'énergie (ESS) domestique :
BMS Daly, compteurs ET112, irradiance PRALRAN, ATS CHINT, intégration **Venus OS** (Victron).
Stack **100 % Rust** — remplacement total de l'ancienne stack Python/FastAPI **et** des flows Node-RED.

> Dashboard **SSR Rust** (Askama + ECharts, sans npm) · Broker **MQTT natif systemd** ·
> TSDB **redb** embarquée (shim PromQL compatible Grafana) · Déploiement = **binaires statiques**, sans Docker.
> Cibles : Raspberry Pi 5 / CM5 (aarch64) + NanoPi Venus OS (armv7). Windows et Linux x86_64 supportés pour le dev.

---

## ✨ En bref

- **Workspace multi-crates** : `rs485-bus`, `daly-bms-core`, `daly-bms-server`, `energy-manager`, `metrics-store`, `dbus-mqtt-venus`.
- **daly-bms-server** (Pi5, :8080) : polling RS485 (BMS Daly + ET112 + PRALRAN + ATS), API REST/WebSocket, dashboard SSR, AlertEngine, metrics-store redb.
- **energy-manager** (Pi5, :8081) : automatisation énergie (solaire, DEYE, chauffe-eau LG, charge, météo) via `rust-rule-engine`.
- **dbus-mqtt-venus** (NanoPi) : bridge MQTT → D-Bus Venus OS en zbus pur Rust (~5–8 Mo RAM).
- **Observabilité** : redb (tiering raw 30 j / hourly 365 j / daily 5 ans) + 20 dashboards Grafana.

---

## 📚 Documentation

**Point d'entrée : [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md)** — vue d'ensemble du système
et **index complet** de la documentation.

### Applications (binaires)

- [docs/app-daly-bms-server.md](./docs/app-daly-bms-server.md) — serveur principal : RS485, protocole Daly, API REST/WS, dashboard SSR.
- [docs/app-energy-manager.md](./docs/app-energy-manager.md) — automatisation énergie : modules `logic/`, règles `.grl`, clients HTTP.
- [docs/app-dbus-mqtt-venus.md](./docs/app-dbus-mqtt-venus.md) — bridge MQTT → D-Bus Venus OS, déploiement armv7.

### Domaines transverses

- [docs/deploiement-exploitation.md](./docs/deploiement-exploitation.md) — build, déploiement Pi5 + NanoPi, systemd, logs, restauration.
- [docs/metriques-redb-architecture.md](./docs/metriques-redb-architecture.md) — moteur TSDB redb + historique de migration.
- [docs/metriques-promql-reference.md](./docs/metriques-promql-reference.md) — catalogue des métriques + référence PromQL.
- [docs/grafana-dashboards.md](./docs/grafana-dashboards.md) — Grafana : datasource, provisioning, 20 dashboards.
- [docs/mqtt-mosquitto.md](./docs/mqtt-mosquitto.md) — architecture MQTT, topics, bridge, anti-boucle.
- [docs/alertes.md](./docs/alertes.md) — AlertEngine natif (règles, hysteresis, notifications).
- [docs/integration-materiel.md](./docs/integration-materiel.md) — inventaire RS485/D-Bus, ajout d'un BMS, ATS, ET112.
- [docs/diagnostic-depannage.md](./docs/diagnostic-depannage.md) — dépannage, netdiag, debug onduleur/SmartShunt.

> [`CLAUDE.md`](./CLAUDE.md) (racine) reste la **mémoire projet** : commandes rapides, inventaire de production, règles de travail.

---

## 🚀 Démarrage rapide

```bash
# Configuration (le service lit /etc/daly-bms/config.toml, PAS le Config.toml du dépôt)
sudo mkdir -p /etc/daly-bms
sudo cp Config.toml /etc/daly-bms/config.toml
sudo nano /etc/daly-bms/config.toml      # port série + adresses BMS

# Développement local
make run-debug

# Production Pi5 (aarch64)
make build-arm
make install                              # binaire + service systemd daly-bms
journalctl -u daly-bms -f
```

Détails (toutes les cibles Makefile, workflow Pi5 + NanoPi, services systemd) :
**[docs/deploiement-exploitation.md](./docs/deploiement-exploitation.md)**.

---

## 🖥️ Interfaces

| Accès | URL |
|-------|-----|
| Dashboard SSR (synthèse BMS) | `http://<pi5>:8080/dashboard` |
| API REST / WebSocket | `http://<pi5>:8080/api/v1/…` · `ws://<pi5>:8080/ws/…` |
| API PromQL (compat Grafana) | `http://<pi5>:8080/api/v1/query`, `/query_range`, `/labels` |
| Healthcheck | `http://<pi5>:8080/-/healthy` |
| Grafana | `http://<pi5>:3000` |

---

## 🔧 Matériel

Raspberry Pi Compute Module 5 (4 Go RAM) + adaptateur USB/RS485 ; NanoPi sous Venus OS pour le bridge D-Bus.
Inventaire complet du bus RS485 et des services Victron : [docs/integration-materiel.md](./docs/integration-materiel.md)
et [docs/ARCHITECTURE.md §7](./docs/ARCHITECTURE.md#7-inventaire-matériel-résumé).

---

*Référence protocole : Daly UART/485 Communications Protocol V1.21.
Runtime : tokio-serial · Axum · rumqttc · zbus · redb.*
