# Grafana — Installation, Datasource, Provisioning et 20 Dashboards — Daly-BMS-Rust

> Guide complet de l'intégration Grafana sur le Raspberry Pi 5 : installation automatique
> ou manuelle, configuration de la datasource PromQL vers `daly-bms-server` (redb,
> UID `daly-metrics`), provisioning complet, catalogue des 20 dashboards, monitoring
> PV solaire et dépannage.
> Fait partie de l'[architecture documentaire](./ARCHITECTURE.md).
> Dernière consolidation : 2026-06-07.

## Table des matières

- [1. Vue d'ensemble et rôle de Grafana](#1-vue-densemble-et-role-de-grafana)
- [2. Installation automatique (recommandée)](#2-installation-automatique-recommandee)
  - [2.1 Script setup-grafana.sh](#21-script-setup-grafanash)
  - [2.2 Options disponibles](#22-options-disponibles)
  - [2.3 Ce que le script déploie](#23-ce-que-le-script-deploie)
  - [2.4 Stockage des données sur NVMe (option --nvme)](#24-stockage-des-donnees-sur-nvme-option---nvme)
- [3. Installation manuelle (étape par étape)](#3-installation-manuelle-etape-par-etape)
  - [3.1 Prérequis système](#31-prerequis-systeme)
  - [3.2 Ajouter le dépôt Grafana](#32-ajouter-le-depot-grafana)
  - [3.3 Installer Grafana](#33-installer-grafana)
  - [3.4 Démarrer et activer le service](#34-demarrer-et-activer-le-service)
  - [3.5 Accéder à Grafana](#35-acceder-a-grafana)
  - [3.6 Changer le port (optionnel)](#36-changer-le-port-optionnel)
  - [3.7 Image renderer (optionnel)](#37-image-renderer-optionnel)
- [4. Datasource PromQL — daly-metrics](#4-datasource-promql--daly-metrics)
  - [4.1 Fichier YAML de provisioning](#41-fichier-yaml-de-provisioning)
  - [4.2 Configuration manuelle via l'interface Grafana](#42-configuration-manuelle-via-linterface-grafana)
  - [4.3 Endpoints PromQL exposés par daly-bms-server](#43-endpoints-promql-exposes-par-daly-bms-server)
- [5. Provisioning complet](#5-provisioning-complet)
  - [5.1 Arborescence des fichiers de provisioning](#51-arborescence-des-fichiers-de-provisioning)
  - [5.2 Provider dashboards (daly-bms.yaml)](#52-provider-dashboards-daly-bmsyaml)
  - [5.3 Règles de format provisioning (OBLIGATOIRE)](#53-regles-de-format-provisioning-obligatoire)
- [6. Déploiement des dashboards](#6-deploiement-des-dashboards)
  - [6.1 Via deploy-pi5.sh (méthode standard)](#61-via-deploy-pi5sh-methode-standard)
  - [6.2 Via fix-grafana.sh (import API — contourne bug Grafana 11+)](#62-via-fix-grafanash-import-api--contourne-bug-grafana-11)
  - [6.3 Copie manuelle](#63-copie-manuelle)
  - [6.4 Commandes Grafana de référence rapide](#64-commandes-grafana-de-reference-rapide)
- [7. Catalogue des 20 dashboards](#7-catalogue-des-20-dashboards)
  - [7.1 Dashboards 01 à 16 — Vue opérationnelle](#71-dashboards-01-a-16--vue-operationnelle)
  - [7.2 Dashboards 17 à 20 — PromQL avancé (Flotte, PV, Bilan, Alertes)](#72-dashboards-17-a-20--promql-avance-flotte-pv-bilan-alertes)
- [8. Monitoring PV solaire — Dashboard comparaison 5 ans](#8-monitoring-pv-solaire--dashboard-comparaison-5-ans)
  - [8.1 Architecture des données PV](#81-architecture-des-donnees-pv)
  - [8.2 Structure du dashboard PV (pv-solar-5y)](#82-structure-du-dashboard-pv-pv-solar-5y)
  - [8.3 Requêtes PromQL pour le monitoring PV](#83-requetes-promql-pour-le-monitoring-pv)
  - [8.4 Limites du shim PromQL redb](#84-limites-du-shim-promql-redb)
  - [8.5 JSON complet du dashboard pv-solar-5y](#85-json-complet-du-dashboard-pv-solar-5y)
- [9. Génération de rapports PDF](#9-generation-de-rapports-pdf)
- [10. Dépannage Grafana](#10-depannage-grafana)
- [11. Désinstallation](#11-desinstallation)
- [Voir aussi](#voir-aussi)
- [Sources consolidées](#sources-consolidees)

---

## 1. Vue d'ensemble et rôle de Grafana

Grafana est déployé sur le **Raspberry Pi 5** (`192.168.1.141`, port **3000**) en tant que
service systemd (`grafana-server`). Il se connecte exclusivement à `daly-bms-server` via
une datasource de type Prometheus (UID `daly-metrics`) pointant sur
`http://127.0.0.1:8080`.

```
┌─────────────────┐     PromQL      ┌──────────────────────────────────────┐
│   Grafana       │ ←────────────── │ daly-bms-server (redb :8080)         │
│   (:3000)       │  /api/v1/query  │   (Stockage metrics-store — redb)    │
│                 │  /query_range   │   Tiering raw 30 j / hourly 365 j /  │
│   20 dashboards │  /labels        │   daily 5 ans                        │
│   provisionnés  │                 │   Alimenté par :                     │
└─────────────────┘                 │    - polling RS485 (BMS, ET112, ATS) │
                                    │    - MQTT (energy-manager, Victron)  │
                                    └──────────────────────────────────────┘
```

**Données NVMe optionnel** : avec l'option `--nvme`, la base SQLite Grafana, les logs et
les plugins sont stockés sur `/mnt/nvme/grafana` au lieu de la SD/eMMC — fortement
recommandé en production.

**20 dashboards provisionés** dans `/var/lib/grafana/dashboards/`, tous au format
provisioning (pas export), tous utilisant l'UID datasource `daly-metrics`.

> ⚠️ **Note architecture** : `daly-bms-server` embarque le metrics-store redb directement
> — il n'y a pas de service TSDB séparé. Grafana interroge daly-bms-server via son shim
> PromQL. Pour l'architecture interne de redb, voir [./metriques-redb-architecture.md].

---

## 2. Installation automatique (recommandée)

### 2.1 Script setup-grafana.sh

Un script d'installation **idempotent** gère l'intégralité de la mise en place : dépôt APT,
paquet Grafana OSS (aarch64), provisioning datasource, provisioning dashboards, ouverture
UFW, healthcheck.

```bash
# Depuis ~/Daly-BMS-Rust sur le Pi5 — commande standard production
sudo bash scripts/setup-grafana.sh --nvme
```

Le script est idempotent : il peut être relancé sans casser une installation existante.

### 2.2 Options disponibles

| Option | Description |
|--------|-------------|
| `--nvme` | Stocke DB/logs/plugins sur `/mnt/nvme/grafana` (recommandé Pi5) |
| `--data-path=PATH` | Chemin custom pour les données Grafana (remplace `--nvme`) |
| `--port=N` | Change le port Grafana (défaut 3000) |
| `--admin-pwd=PASS` | Définit le mot de passe admin initial |
| `--api-url=URL` | URL daly-bms-server (défaut `http://127.0.0.1:8080`) |
| `--no-firewall` | Désactive l'ajout de règle UFW |
| `--renderer` | Installe `grafana-image-renderer` + Chromium (~300 Mo) |
| `--uninstall` | Désinstalle Grafana et nettoie la configuration |

Exemples :

```bash
bash scripts/setup-grafana.sh --nvme
bash scripts/setup-grafana.sh --nvme --admin-pwd='ChangeMe!2026'
bash scripts/setup-grafana.sh --port=3000
bash scripts/setup-grafana.sh --renderer
bash scripts/setup-grafana.sh --api-url=http://10.0.0.5:8080  # daly-bms-server distant
sudo bash scripts/setup-grafana.sh --uninstall
```

### 2.3 Ce que le script déploie

| Étape | Action |
|-------|--------|
| 1 | Mise à jour APT, installation des dépendances (`apt-transport-https`, `wget`, `curl`, `gnupg`, `ca-certificates`) |
| 2 | Ajout clé GPG Grafana (`/usr/share/keyrings/grafana.gpg`) + dépôt APT stable |
| 3 | Installation `grafana` OSS via APT |
| 4 | Configuration stockage NVMe si `--nvme` (chemins `data`, `logs`, `plugins` dans `grafana.ini`) |
| 5 | Vérification de connectivité vers `daly-bms-server` (`/-/healthy`) |
| 6 | Déploiement provisioning datasource (`daly-metrics.yaml`) + provider dashboards (`daly-bms.yaml`) + 20 JSON dashboards dans `/var/lib/grafana/dashboards/` |
| 7 | Configuration `grafana.ini` (port, mot de passe admin initial si `--admin-pwd`) |
| 8 | Installation `grafana-image-renderer` + Chromium si `--renderer` |
| 9 | Ouverture port UFW si UFW actif et `--no-firewall` absent |
| 10 | `systemctl daemon-reload && systemctl enable && systemctl restart grafana-server` |
| 11 | Healthcheck final (`http://127.0.0.1:3000/api/health`) |

Résultat affiché en fin de script :

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Installation Grafana terminée
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  URL          : http://192.168.1.141:3000
  Login        : admin / admin  (changer à la 1ʳᵉ connexion)
  Datasource   : Daly Metrics (redb) → http://127.0.0.1:8080  (provisionnée)
  Dashboards   : 20 dashboards Grafana (dossier Daly-BMS)
  Données NVMe : /mnt/nvme/grafana/

  Logs         : journalctl -u grafana-server -f
  Config       : /etc/grafana/grafana.ini
  Provisioning : /etc/grafana/provisioning/

  Désinstaller : sudo bash scripts/setup-grafana.sh --uninstall
```

### 2.4 Stockage des données sur NVMe (option --nvme)

Avec `--nvme`, le script configure `grafana.ini` (section `[paths]`) pour stocker les
données Grafana sur le NVMe :

| Sous-répertoire | Chemin configuré |
|-----------------|-----------------|
| Base SQLite | `/mnt/nvme/grafana/data/grafana.db` |
| Logs | `/mnt/nvme/grafana/log/` |
| Plugins | `/mnt/nvme/grafana/plugins/` |
| Temp | `/mnt/nvme/grafana/temp/` (durée de vie 24h) |

Le script vérifie que `/mnt/nvme` est monté avant de continuer. Si une base
`/var/lib/grafana/grafana.db` existe, elle est migrée automatiquement vers le NVMe.

> **Note** : Les JSON de dashboards restent dans `/var/lib/grafana/dashboards/` (petits
> fichiers, pas de raison de les déplacer).

---

## 3. Installation manuelle (étape par étape)

> La procédure manuelle n'est nécessaire que si `setup-grafana.sh` ne peut pas être utilisé.
> Préférer le script automatique en production.

### 3.1 Prérequis système

```bash
# Mettre à jour le système
sudo apt update && sudo apt upgrade -y

# Installer les dépendances
sudo apt install -y apt-transport-https software-properties-common wget gnupg ca-certificates
```

### 3.2 Ajouter le dépôt Grafana

```bash
# Télécharger la clé GPG
wget -q -O - https://apt.grafana.com/gpg.key | sudo gpg --dearmor -o /usr/share/keyrings/grafana.gpg

# Ajouter le dépôt (version OSS stable, ARM64)
echo "deb [signed-by=/usr/share/keyrings/grafana.gpg] https://apt.grafana.com stable main" \
    | sudo tee /etc/apt/sources.list.d/grafana.list

# Mettre à jour les paquets
sudo apt update
```

### 3.3 Installer Grafana

```bash
sudo apt install -y grafana
```

### 3.4 Démarrer et activer le service

```bash
# Recharger systemd, activer et démarrer
sudo systemctl daemon-reload
sudo systemctl enable grafana-server
sudo systemctl start grafana-server

# Vérifier le statut
sudo systemctl status grafana-server
```

### 3.5 Accéder à Grafana

- **URL** : `http://192.168.1.141:3000` (ou `http://<ip-du-pi5>:3000`)
- **Login par défaut** : `admin` / `admin`
- **Changer le mot de passe** lors de la première connexion

### 3.6 Changer le port (optionnel)

```bash
sudo nano /etc/grafana/grafana.ini
# Modifier dans [server] :  http_port = 3000  →  http_port = <nouveau_port>
sudo systemctl restart grafana-server
```

### 3.7 Image renderer (optionnel)

Le plugin `grafana-image-renderer` permet l'export PDF de dashboards (nécessite Chromium,
~300 Mo).

```bash
# Via le script (recommandé)
bash scripts/setup-grafana.sh --renderer

# Ou manuellement
sudo apt install -y chromium chromium-sandbox
sudo grafana-cli plugins install grafana-image-renderer
sudo systemctl restart grafana-server
```

---

## 4. Datasource PromQL — daly-metrics

### 4.1 Fichier YAML de provisioning

Fichier source (versionné) : `contrib/grafana/provisioning/datasources/daly-metrics.yaml`
Déployé vers : `/etc/grafana/provisioning/datasources/daly-metrics.yaml`

```yaml
apiVersion: 1

# Datasource Grafana qui interroge le backend metrics-store (redb) via
# daly-bms-server. redb est la seule TSDB de lecture du système.
#
# Les endpoints Prometheus-compat exposés par daly-bms-server (/api/v1/query,
# /api/v1/query_range, /api/v1/labels, /api/v1/series, /api/v1/label/:n/values,
# /-/healthy) lisent redb directement.

datasources:
  - name: Daly Metrics (redb)
    type: prometheus
    access: proxy
    url: http://127.0.0.1:8080
    uid: daly-metrics
    isDefault: true
    editable: true
    jsonData:
      httpMethod: GET
      timeInterval: 60s
      queryTimeout: 30s
      prometheusType: Prometheus
      prometheusVersion: 2.40.0
      manageAlerts: false
```

Paramètres clés :

| Champ | Valeur | Explication |
|-------|--------|-------------|
| `name` | `Daly Metrics (redb)` | Nom affiché dans l'interface Grafana |
| `type` | `prometheus` | Type de datasource (PromQL compatible) |
| `url` | `http://127.0.0.1:8080` | daly-bms-server local |
| `uid` | `daly-metrics` | **UID fixe** — tous les dashboards l'utilisent |
| `isDefault` | `true` | Datasource par défaut |
| `timeInterval` | `60s` | Intervalle de scrape minimal |
| `queryTimeout` | `30s` | Timeout des requêtes |
| `prometheusType` | `Prometheus` | Déclaration du type de backend |
| `prometheusVersion` | `2.40.0` | Version de compatibilité PromQL |
| `manageAlerts` | `false` | Ne pas déléguer les alertes à Grafana |

> **IMPORTANT** : L'UID `daly-metrics` est la valeur de référence utilisée par les
> 20 dashboards. Ne jamais utiliser `${datasource}` (format export) ni changer cet UID.

### 4.2 Configuration manuelle via l'interface Grafana

> Procédure à utiliser uniquement si la datasource n'a pas été provisionnée automatiquement.

1. Dans Grafana : **Configuration → Data Sources → Add data source**
2. Sélectionner **Prometheus**
3. **Name** : `Daly Metrics (redb)`
4. **URL** : `http://127.0.0.1:8080`
5. **UID** : `daly-metrics` (important : fixer manuellement dans les paramètres avancés)
6. **HTTP Method** : `GET`
7. **Scrape interval** : `60s`
8. **Save & Test** → doit afficher "Data source is working"

### 4.3 Endpoints PromQL exposés par daly-bms-server

Les endpoints suivants sont servis par `daly-bms-server` sur le port 8080 et forment le
shim PromQL qu'interroge Grafana :

```
GET  /api/v1/query          ← instant query
GET  /api/v1/query_range    ← range query (utilisée par les graphiques)
GET  /api/v1/labels         ← liste des label names
GET  /api/v1/label/:n/values ← valeurs d'un label
GET  /api/v1/series         ← series matching (curl -s http://localhost:8080/api/v1/redb/series | jq '.data | length')
GET  /-/healthy             ← healthcheck backend redb
```

Vérification rapide :

```bash
# Healthcheck backend
curl -s http://localhost:8080/-/healthy

# Nombre de séries en base
curl -s http://localhost:8080/api/v1/redb/series | jq '.data | length'

# Tester une requête PromQL
curl -s 'http://localhost:8080/api/v1/query?query=solar_total_w' | jq '.data.result'

# Vérifier le format du label address pour ET112
curl -s 'http://localhost:8080/api/v1/query?query=et112_power_w' | jq '.data.result[].metric'
```

> Pour le catalogue complet des métriques disponibles et la syntaxe PromQL, voir
> [./metriques-promql-reference.md].

---

## 5. Provisioning complet

### 5.1 Arborescence des fichiers de provisioning

Source dans le dépôt :

```
contrib/grafana/
├── dashboards/
│   ├── 01-bms.json
│   ├── 02-et112.json
│   ├── 03-mppt.json
│   ├── 04-smartshunt.json
│   ├── 05-onduleur.json
│   ├── 06-temperatures-venus.json
│   ├── 07-heatpumps.json
│   ├── 08-solaire.json
│   ├── 09-irradiance.json
│   ├── 10-ats.json
│   ├── 11-tasmota.json
│   ├── 12-shelly.json
│   ├── 13-chauffe-eau.json
│   ├── 14-pi5.json
│   ├── 15-energy-manager.json
│   ├── 16-bms-detail.json
│   ├── 17-flotte-sante.json
│   ├── 18-rendement-pv.json
│   ├── 19-bilan-energie.json
│   └── 20-alertes-avancees.json
└── provisioning/
    ├── datasources/
    │   └── daly-metrics.yaml        ← datasource PromQL → :8080
    └── dashboards/
        └── daly-bms.yaml            ← provider → /var/lib/grafana/dashboards
```

Déployé vers (sur le Pi5) :

```
/etc/grafana/provisioning/
├── datasources/
│   └── daly-metrics.yaml
└── dashboards/
    └── daly-bms.yaml

/var/lib/grafana/dashboards/
├── 01-bms.json
├── 02-et112.json
... (20 fichiers)
└── 20-alertes-avancees.json
```

### 5.2 Provider dashboards (daly-bms.yaml)

Fichier source : `contrib/grafana/provisioning/dashboards/daly-bms.yaml`
Déployé vers : `/etc/grafana/provisioning/dashboards/daly-bms.yaml`

```yaml
apiVersion: 1

providers:
  - name: 'Daly-BMS'
    orgId: 1
    folder: 'Daly-BMS'
    folderUid: daly-bms-folder
    type: file
    disableDeletion: false
    updateIntervalSeconds: 60
    allowUiUpdates: true
    options:
      path: /var/lib/grafana/dashboards
      foldersFromFilesStructure: false
```

Paramètres clés :

| Champ | Valeur | Explication |
|-------|--------|-------------|
| `folder` | `Daly-BMS` | Nom du dossier Grafana affiché dans l'UI |
| `folderUid` | `daly-bms-folder` | UID stable du dossier |
| `updateIntervalSeconds` | `60` | Rechargement automatique des JSON toutes les 60s |
| `allowUiUpdates` | `true` | Modifications via l'UI autorisées |
| `path` | `/var/lib/grafana/dashboards` | Dossier source des JSON |

### 5.3 Règles de format provisioning (OBLIGATOIRE)

> Règle 14 du projet (CLAUDE.md) — à respecter absolument.

**Format provisioning, pas export** : Les JSON dans `contrib/grafana/dashboards/` doivent
être au format **provisioning** et non au format export Grafana. La différence principale :

| Critère | Format provisioning (correct) | Format export (incorrect) |
|---------|-------------------------------|---------------------------|
| `__inputs` | ABSENT | présent |
| `__requires` | ABSENT | présent |
| UID datasource | `"uid": "daly-metrics"` (valeur fixe) | `"uid": "${datasource}"` (variable) |
| `id` | `null` ou absent | entier de la DB locale |

Vérification d'un JSON :

```bash
# Vérifier l'absence de __inputs/__requires et la présence de l'UID daly-metrics
python3 -c "
import json
with open('contrib/grafana/dashboards/01-bms.json') as f:
    d = json.load(f)
assert '__inputs' not in d, 'ERREUR: __inputs présent (format export)'
assert '__requires' not in d, 'ERREUR: __requires présent (format export)'
import re
txt = json.dumps(d)
assert 'daly-metrics' in txt, 'ERREUR: UID daly-metrics absent'
print('OK — format provisioning correct')
"
```

> **Conséquence pratique** : si un dashboard vide "No items" s'affiche dans Grafana, c'est
> souvent dû à un mauvais format (présence de `__inputs`/`__requires` ou UID `${datasource}`).

---

## 6. Déploiement des dashboards

### 6.1 Via deploy-pi5.sh (méthode standard)

`scripts/deploy-pi5.sh` déploie l'ensemble (binaires + mosquitto.conf + Grafana) en une
seule commande. Il inclut automatiquement le déploiement des 20 dashboards Grafana.

```bash
# Déploiement complet (binaires + dashboards)
bash scripts/deploy-pi5.sh

# Déploiement sans rebuild (si binaires déjà compilés)
sudo bash scripts/deploy-pi5.sh --no-build
```

> ⚠️ `deploy-pi5.sh` NE déploie PAS `Config.toml` si `/etc/daly-bms/config.toml` existe
> (préserve la config de production). Copie manuelle si besoin :
> `sudo cp Config.toml /etc/daly-bms/config.toml`
>
> ⚠️ `deploy-pi5.sh` NE déploie PAS le NanoPi → `make install-venus-v7` séparément.

### 6.2 Via fix-grafana.sh (import API — contourne bug Grafana 11+)

`scripts/fix-grafana.sh` est la méthode de réparation lorsque le provisioning par fichier
échoue (bug "restricted database access" de Grafana 11+). Il importe les dashboards via
l'API HTTP Grafana plutôt que par le système de fichiers.

```bash
sudo bash scripts/fix-grafana.sh
sudo bash scripts/fix-grafana.sh --password=MonMotDePasse
```

Ce que fait `fix-grafana.sh` :

1. Vérifie que `grafana-server` tourne (démarre si nécessaire)
2. Réinitialise le mot de passe admin (`grafana-cli admin reset-admin-password`)
3. Attend que l'API Grafana soit prête (15 tentatives, 2s entre chaque)
4. Déploie la datasource `daly-metrics.yaml` (par fichier — fonctionne toujours)
5. Supprime le provisioning fichier des dashboards (pour éviter les conflits)
6. Crée ou retrouve le dossier `Daly-BMS` via l'API (`uid: daly-bms-folder`)
7. Importe chaque JSON via l'API (`POST /api/dashboards/import`, `id: null`, `overwrite: true`)

> La datasource reste provisionnée par fichier (non affectée par le bug Grafana 11+).
> Seuls les dashboards passent par l'API dans cette méthode.

### 6.3 Copie manuelle

Pour ne déployer que les dashboards sans passer par les scripts :

```bash
# Copier les JSON vers le dossier Grafana
sudo cp contrib/grafana/dashboards/*.json /var/lib/grafana/dashboards/
sudo systemctl restart grafana-server
```

Déploiement complet provisioning manuel :

```bash
# Datasource
sudo install -m 0644 -o root -g grafana \
    contrib/grafana/provisioning/datasources/daly-metrics.yaml \
    /etc/grafana/provisioning/datasources/daly-metrics.yaml

# Provider dashboards
sudo install -m 0644 -o root -g grafana \
    contrib/grafana/provisioning/dashboards/daly-bms.yaml \
    /etc/grafana/provisioning/dashboards/daly-bms.yaml

# Dashboards JSON
sudo install -d -o grafana -g grafana -m 0755 /var/lib/grafana/dashboards
for dash in contrib/grafana/dashboards/*.json; do
    sudo install -m 0644 -o grafana -g grafana "$dash" /var/lib/grafana/dashboards/
done

sudo systemctl restart grafana-server
```

> **Attention** : supprimer d'éventuels fichiers résiduels d'une ancienne datasource
> VictoriaMetrics avant de continuer :
> `sudo rm -f /etc/grafana/provisioning/datasources/victoriametrics.yaml`

### 6.4 Commandes Grafana de référence rapide

| Action | Commande |
|--------|----------|
| Installer Grafana | `sudo bash scripts/setup-grafana.sh --nvme` |
| Déployer dashboards | Inclus dans `bash scripts/deploy-pi5.sh` |
| Réparer dashboards seuls | `sudo bash scripts/fix-grafana.sh` |
| Redémarrer Grafana | `sudo systemctl restart grafana-server` |
| Logs Grafana | `journalctl -u grafana-server -f` |
| Healthcheck | `curl -s http://localhost:3000/api/health` |
| Taille base SQLite Grafana | `du -sh /mnt/nvme/grafana/data/grafana.db` (si NVMe) ou `/var/lib/grafana/grafana.db` |
| Supprimer dossier vide | Via UI Grafana : Dashboards → dossier → Delete |

---

## 7. Catalogue des 20 dashboards

Tous les dashboards sont dans le dossier Grafana **Daly-BMS**, provisionnés depuis
`/var/lib/grafana/dashboards/`. Chaque JSON respecte les règles de format provisioning
(UID `daly-metrics`, pas de `__inputs`/`__requires`).

### 7.1 Dashboards 01 à 16 — Vue opérationnelle

#### Dashboard 01 — BMS Batteries Daly
**Fichier** : `01-bms.json` | **UID** : `daly-bms-01` | **Tags** : `daly-bms`, `battery`

Vue d'ensemble de toutes les batteries Daly (3 BMS en production : BMS-360Ah `0x01`,
BMS-320Ah `0x02`, BMS-620Ah `0x03`).

Panels principaux (27 panels) :
- SOC par BMS (BMS-360Ah, BMS-320Ah, BMS-620Ah)
- SOH (State of Health) par BMS
- Capacité restante par BMS
- Tension pack, courant, puissance par BMS
- Températures, delta cellules (déséquilibre)
- État MOS charge/décharge
- Alarmes actives

#### Dashboard 02 — ET112 Compteurs Énergie AC
**Fichier** : `02-et112.json` | **UID** : `daly-et112-02` | **Tags** : `daly-bms`, `et112`, `energy`

Visualisation des 3 compteurs ET112 sur le bus RS485 :
- `0x07` ET112-Micro-Onduleurs (SN 119253X)
- `0x08` ET112-Maison (SN 119215X)
- `0x09` ET112-Réseau (SN 061077X)

Panels (12 panels) :
- Puissance active par compteur
- Tension, courant
- Énergie importée / exportée (cumul)
- Facteur de puissance (Maison, Réseau)
- Fréquence

> **IMPORTANT** : Le backend écrit le label `address` en hexadécimal :
> `address="0x07"`, `address="0x08"`, `address="0x09"`. Les requêtes PromQL doivent
> utiliser ces valeurs hexadécimales — jamais le format décimal (`address="7"`). Vérif :
> `curl -s 'localhost:8080/api/v1/query?query=et112_power_w' | jq '.data.result[].metric'`

#### Dashboard 03 — Venus MPPT Chargeurs Solaires
**Fichier** : `03-mppt.json` | **UID** : `daly-mppt-03` | **Tags** : `daly-bms`, `mppt`, `solar`, `victron`

Monitoring des chargeurs MPPT Victron (données reçues via MQTT Victron → energy-manager).

Panels (10 panels) :
- Puissance MPPT (tous chargeurs agrégés)
- Rendement journalier total
- Rendement par MPPT
- Puissance max aujourd'hui
- Tension PV (panneaux)
- Courant DC (charge batterie)
- État MPPT (0=Off, 3=Bulk, 4=Absorption, 5=Float)

#### Dashboard 04 — SmartShunt Moniteur Batterie Victron
**Fichier** : `04-smartshunt.json` | **UID** : `daly-shunt-04` | **Tags** : `daly-bms`, `smartshunt`, `victron`, `battery`

Monitoring du SmartShunt Victron (données MQTT Victron).

Panels (15 panels) :
- SOC SmartShunt
- Tension, temps restant
- État (charge/décharge/idle)
- Courant, puissance
- Ah chargés aujourd'hui
- Courbes courant & puissance

#### Dashboard 05 — Onduleur / Chargeur Victron
**Fichier** : `05-onduleur.json` | **UID** : `daly-inv-05` | **Tags** : `daly-bms`, `inverter`, `victron`

Monitoring de l'onduleur/chargeur Victron (Multiplus ou équivalent).

Panels (14 panels) :
- État onduleur, mode
- AC Input ignoré
- Puissance AC sortie
- Tension DC entrée, courant DC, puissance DC
- Tension AC sortie
- Puissance AC entrée

#### Dashboard 06 — Températures Venus Capteurs Victron
**Fichier** : `06-temperatures-venus.json` | **UID** : `daly-temp-06` | **Tags** : `daly-bms`, `temperature`, `victron`

Capteurs de température déclarés sur le bus Venus OS (via `com.victronenergy.temperature.*`).

Panels (7 panels) :
- Températures par capteur (courbes)
- Humidité relative
- Pression barométrique
- Nombre de capteurs connectés

#### Dashboard 07 — Heatpumps ET112 via Venus (Maison & Réseau)
**Fichier** : `07-heatpumps.json` | **UID** : `daly-heat-07` | **Tags** : `daly-bms`, `heatpump`, `victron`, `et112`

Vue des charges AC mesurées par les ET112 Maison (0x08) et Réseau (0x09) côté Venus.

Panels (9 panels) :
- Puissance AC (Maison et Réseau)
- Énergie cumulative
- Puissance Maison actuelle, Puissance Réseau actuelle
- État de connexion Maison, Réseau
- État opérationnel

#### Dashboard 08 — Production Solaire Vue Globale
**Fichier** : `08-solaire.json` | **UID** : `daly-solar-08` | **Tags** : `daly-bms`, `solar`, `pv`

Vue agrégée de toute la production PV (MPPT + micro-onduleurs ET112).

Panels (10 panels) :
- Puissance Solaire Totale (`solar_total_w = dc_pv_power_w + pvinv_power_w`)
- Puissance totale actuelle
- Rendement journalier (kWh)
- Rendement journalier (Wh)
- Puissance MPPT seul
- Puissance Micro-onduleurs (ET112 `0x07`)
- Irradiance solaire (capteur PRALRAN `0x05`)

#### Dashboard 09 — Irradiance Solaire Capteur PRALRAN
**Fichier** : `09-irradiance.json` | **UID** : `daly-irr-09` | **Tags** : `daly-bms`, `irradiance`, `solar`, `meteo`

Données du capteur d'irradiance PRALRAN (adresse RS485 `0x05`).

Panels (4 panels) :
- Irradiance actuelle (W/m²)
- Courbe irradiance temps réel
- Historique irradiance

#### Dashboard 10 — ATS CHINT Commutateur de Source
**Fichier** : `10-ats.json` | **UID** : `daly-ats-10` | **Tags** : `daly-bms`, `ats`, `grid`

Monitoring de l'ATS CHINT (commutateur automatique de source entre réseau EDF et onduleur
Victron), lu via RS485 par `daly-bms-server`.

Panels (22 panels) :
- Source active (Onduleur / Réseau)
- Contacteur S1 (Onduleur), Contacteur S2 (Réseau)
- Défaut détecté
- Télécommande activée
- Position milieu (coupure transitoire)
- Mode commutation
- Tension active, fréquence
- Historique des basculements

Correspond aux services D-Bus `com.victronenergy.switch.mqtt_1` (ATS CHINT, instance 60).

#### Dashboard 11 — Tasmota Prises Intelligentes
**Fichier** : `11-tasmota.json` | **UID** : `daly-tas-11` | **Tags** : `daly-bms`, `tasmota`, `smartplug`

Monitoring des prises Tasmota (WiFi MQTT) gérées par `daly-bms-server`.

Panels (12 panels) :
- Puissance par prise
- Tension, courant
- Énergie totale (cumul), énergie aujourd'hui, énergie hier
- État relais (ON/OFF)
- Signal WiFi (RSSI)

#### Dashboard 12 — Shelly EM Compteurs Énergie WiFi
**Fichier** : `12-shelly.json` | **UID** : `daly-shelly-12` | **Tags** : `daly-bms`, `shelly`, `energy`

Monitoring des compteurs Shelly EM (WiFi).

Panels (13 panels) :
- Puissance totale par Shelly
- Puissance par canal
- Courant par canal
- Tension
- Facteur de puissance
- Énergie par canal (Wh), énergie retournée (Wh)
- Signal WiFi (RSSI)

#### Dashboard 13 — Chauffe-eau LG ThinQ
**Fichier** : `13-chauffe-eau.json` | **UID** : `daly-wh-13` | **Tags** : `daly-bms`, `water-heater`, `lg`

Monitoring de la pompe à chaleur chauffe-eau LG (via API LG ThinQ, gérée par
`energy-manager`).

Panels (6 panels) :
- Température eau actuelle
- Température cible
- Mode opérationnel
- Historique des températures
- Mode de fonctionnement (Eco, Boost, Normal…)

#### Dashboard 14 — Pi5 Monitoring Système
**Fichier** : `14-pi5.json` | **UID** : `daly-pi5-14` | **Tags** : `daly-bms`, `system`, `pi5`

Métriques système du Raspberry Pi 5 (CPU, mémoire, disque, réseau, température).

Panels (16 panels) :
- CPU (utilisation %)
- Mémoire (utilisation %)
- Disque (utilisation %)
- Température CPU
- Charge système (load average)
- Réseau (bps entrant/sortant)

#### Dashboard 15 — Energy Manager Monitoring Système
**Fichier** : `15-energy-manager.json` | **UID** : `daly-em-15` | **Tags** : `daly-bms`, `energy-manager`, `system`

Métriques système spécifiques au processus `energy-manager` (port 8081).

Panels (13 panels) :
- CPU Energy Manager
- Mémoire (MB utilisée, `LimitMemoryMax=100M`)
- Disque, température CPU
- Charge système
- Réseau (bps)

#### Dashboard 16 — BMS Détail Batterie
**Fichier** : `16-bms-detail.json` | **UID** : `bms-detail-battery` | **Tags** : `daly-bms`, `battery`, `detail`, `echarts`

Vue détaillée par BMS individuel — le tableau le plus complet pour diagnostiquer l'état
d'une batterie.

Panels (35 panels) :
- Tension pack, courant, puissance, SOC, SOH
- Température max, delta cellules (mV)
- Capacité SOC
- Tensions individuelles de chaque cellule
- Historique tension pack sur plusieurs plages de temps
- Historique courant et puissance

---

### 7.2 Dashboards 17 à 20 — PromQL avancé (Flotte, PV, Bilan, Alertes)

Ces 4 dashboards utilisent des fonctionnalités PromQL avancées (agrégations, fonctions
statistiques, `offset`, `start()`, `count_values`). Ils sont décrits dans
`docs/Evolution-compliance-PromQL.md §9`.

#### Dashboard 17 — Flotte Santé & SLO Batterie
**Fichier** : `17-flotte-sante.json` | **UID** : `daly-fleet-17` | **Tags** : `daly-bms`, `battery`, `slo`, `fleet`

Vue agrégée de la flotte de batteries — indicateurs de santé et SLO (Service Level
Objectives).

Panels (12 panels) :
- **BMS actifs** — nombre de packs en ligne
- **SOC médian parc (P50)** — percentile 50 du SOC sur tous les BMS
- **C-rate par pack** — `|I| / capacité nominale` (indicateur de sollicitation)
- **SoH min parc** — State of Health minimum de la flotte
- **SOC P05 / P50 / P95** — distribution statistique du SOC
- **Tension cellule — bande P05↔P95 (SLO 2.8–4.2 V)** — surveillance SLO tension
- **Dispersion cellules (σ) par pack** — écart-type des tensions intra-pack
- **Distribution SoH (count_values)** — histogramme des valeurs SoH

Requêtes PromQL caractéristiques :
```promql
# SOC médian flotte
quantile(0.5, bms_soc_pct)

# C-rate
abs(bms_current_a) / bms_capacity_ah

# Dispersion cellules
stddev by (bms_id) (bms_cell_voltage_v)
```

#### Dashboard 18 — PV Rendements & Lissage
**Fichier** : `18-rendement-pv.json` | **UID** : `daly-pv-18` | **Tags** : `daly-bms`, `solar`, `pv`, `efficiency`

Analyse approfondie du rendement PV avec lissage temporel.

Panels (7 panels) :
- **Production solaire totale (MPPT + micro-onduleurs)** — `solar_total_w`
- **Pic puissance 24 h (glissant)** — `max_over_time(solar_total_w[24h])`
- **Rendement onduleur (AC/DC)** — ratio puissance AC sur puissance DC
- **Puissance MPPT lissée 1 h (sous-requête [1h:5m])** — moyenne glissante horaire
- **Yield aujourd'hui vs hier (offset 24h)** — comparaison avec `solar_yield_kwh`

#### Dashboard 19 — Énergie Bilan J / J-1 / 7 j
**Fichier** : `19-bilan-energie.json` | **UID** : `daly-energy-19` | **Tags** : `daly-bms`, `energy`, `balance`

Bilan énergétique quotidien et sur 7 jours glissants.

Panels (8 panels) :
- **Import réseau aujourd'hui**
- **Import : aujourd'hui − hier** (variation J/J-1)
- **Ratio Export / Import (jour)**
- **SOC au début de fenêtre (`@ start()`)** — valeur SOC en début de période Grafana
- **Décharge cumulée 7 j** — énergie déchargée sur 7 jours
- **Dérive SOC sur 24 h (offset 24h)** — delta SOC entre maintenant et hier à la même heure

Requêtes PromQL caractéristiques :
```promql
# SOC au début de la fenêtre sélectionnée dans Grafana
bms_soc_pct @ start()

# Variation J/J-1 import réseau
max_over_time(grid_import_kwh[24h]) - max_over_time(grid_import_kwh[24h] offset 24h)

# Dérive SOC 24h
bms_soc_pct - bms_soc_pct offset 24h
```

#### Dashboard 20 — Alertes Centre Multi-critères
**Fichier** : `20-alertes-avancees.json` | **UID** : `daly-alerts-20` | **Tags** : `daly-bms`, `alerts`, `monitoring`

Tableau de bord d'alertes multi-conditions — détecte les anomalies complexes combinant
plusieurs métriques simultanément.

Panels (8 panels) :
- **Surcharge thermique** : `bms_soc_pct < 20 ET bms_temp_max_c > 45`
- **Déséquilibre + cellule basse** : `bms_cell_delta_mv > 50 ET bms_cell_min_v < 3.0`
- **Sur-courant prolongé** : C-rate > 0.5C
- **Onduleur OFF mais PV présent** : `inverter_state = 0 ET solar_total_w > 100`
- **Alarmes BMS actives** : flags température / tension (données RS485 directes)
- **BMS muet (heartbeat)** : absence de données depuis plus de N minutes

Requêtes PromQL caractéristiques :
```promql
# Surcharge thermique combinée
(bms_soc_pct < 20) * on(bms_id) (bms_temp_max_c > 45)

# Onduleur OFF mais production active
(victron_inverter_state == 0) and (solar_total_w > 100)
```

---

## 8. Monitoring PV solaire — Dashboard comparaison 5 ans

### 8.1 Architecture des données PV

```
┌─────────────────┐     MQTT      ┌─────────────────┐
│  Victron GX     │──────────────→│   Pi5 compute   │
│  (MPPT, Venus)  │               │   (MQTT broker) │
└─────────────────┘               └────────┬────────┘
                                           │
                                           ↓
┌─────────────────┐               ┌──────────────────────────┐
│   Grafana       │←──────────────│ daly-bms-server (redb    │
│   (:3000)       │    PromQL     │   :8080)                  │
│   + export PDF  │               │   Tiering :               │
└─────────────────┘               │   raw 30 j / hourly 365 j │
                                  │   / daily 5 ans            │
                                  └──────────────────────────┘
```

**Rétention redb (tiering automatique)** :
- Raw : 30 jours (toutes les mesures)
- Hourly : 365 jours (agrégats horaires)
- Daily : 5 ans (agrégats journaliers)

La maintenance de tiering tourne 4×/jour dans `daly-bms-server`. Aucun flag externe —
la base redb est self-contained.

**Métriques PV réelles écrites par le projet** (voir
`crates/daly-bms-server/src/redb_writes.rs`) :

| Métrique | Type | Description |
|----------|------|-------------|
| `solar_total_w` | gauge | Puissance PV totale instantanée (MPPT + micro-onduleurs) en W |
| `dc_pv_power_w` | gauge | Puissance MPPT côté DC en W |
| `pvinv_power_w` | gauge | Puissance micro-onduleurs ET112 côté AC en W |
| `solar_yield_kwh` | compteur | Yield journalier remis à 0 chaque jour (kWh) |

> `increase()` ne s'applique **pas** aux gauges. Pour l'énergie, utiliser le compteur
> `solar_yield_kwh` ou l'approximation `avg_over_time(<puissance>[durée]) * heures` (Wh).

**Espace disque** : surveiller avec `du -sh /mnt/nvme/daly-bms/metrics.redb`. Prévoir
~200–400 Mo à 30 j, au maximum ~2 Go à l'horizon 5 ans grâce au tiering. Sauvegarde :
copier le fichier `metrics.redb` pour archivage externe.

### 8.2 Structure du dashboard PV (pv-solar-5y)

Le dashboard `pv-solar-5y` (titre : *PV Solaire - Monitoring & Comparaison 5 Ans*) est
un dashboard autonome disponible dans `docs/Solar_PV.json` et `docs/grafana-pv_dashboard.json`.
Il est distinct des 20 dashboards provisionnés (il est orienté import manuel ou intégration
dans une présentation PV).

**Structure en sections (rows)** :

| Section | Panels | Description |
|---------|--------|-------------|
| Puissance Instantanée | Stat + Timeseries | `total_solar_power` (puissance totale temps réel) |
| Comparaison Quotidienne | 3 Stats | Aujourd'hui (Wh), Hier (Wh), Variation J/J-1 (%) |
| Comparaison Mensuelle | Timeseries barres | Mois en cours vs mois précédent |
| Comparaison Annuelle 5 Ans | Timeseries lignes | N, N-1, N-2, N-3, N-4 avec `offset` |
| Dégradation Annuelle | Stat | N vs N-1 (%) |
| Production Mensuelle Cumulée | Timeseries barres | Cumul mensuel `sum(increase(...[30d]))` |
| Profil Journalier Saisonnier | Timeseries | `avg_over_time(total_solar_power[1d])` |

**Paramètres du dashboard** :

| Paramètre | Valeur |
|-----------|--------|
| UID | `pv-solar-5y` |
| Refresh | `30s` |
| Période par défaut | `now-30d` à `now` |
| Timezone | `browser` |
| Style | `dark` |
| Tags | `pv`, `solaire`, `victron`, `redb` |
| Variable template | `$datasource` (type datasource Prometheus) |

> ⚠️ Ce dashboard utilise `${datasource}` comme variable de template (format import),
> contrairement aux 20 dashboards provisionnés qui utilisent directement l'UID
> `daly-metrics`. Lors d'un import manuel, sélectionner la datasource "Daly Metrics (redb)".

### 8.3 Requêtes PromQL pour le monitoring PV

> Pour le catalogue complet des métriques, voir [./metriques-promql-reference.md].

| Objectif | Requête PromQL |
|----------|----------------|
| **Puissance totale instantanée** | `solar_total_w` |
| **Production aujourd'hui** | `max_over_time(solar_yield_kwh[24h])` |
| **Production hier** | `max_over_time(solar_yield_kwh[24h] offset 24h)` |
| **Variation J / J-1 (%)** | `((max_over_time(solar_yield_kwh[24h]) - max_over_time(solar_yield_kwh[24h] offset 24h)) / max_over_time(solar_yield_kwh[24h] offset 24h)) * 100` |
| **Production cumulée 30 j** ⚠️ | `query_range` de `max_over_time(solar_yield_kwh[1d])` sur 30 j, puis somme côté client |
| **Production cumulée 1 an** ⚠️ | idem sur 365 j, somme côté client |
| **Comparaison année N vs N-1** | `max_over_time(solar_yield_kwh[1d] offset 365d)` |
| **Puissance moyenne 1 h** | `avg_over_time(solar_total_w[1h])` |
| **Pic de puissance du jour** | `max_over_time(solar_total_w[24h])` |
| **Énergie via intégration (Wh)** | `avg_over_time(solar_total_w[1d]) * 24` |

Les requêtes du dashboard `pv-solar-5y` utilisent `total_solar_power` (nom ancien) et
`increase()` sur une gauge — à migrer vers `solar_total_w` et les patterns ci-dessus si
adaptation du dashboard.

### 8.4 Limites du shim PromQL redb

> Référence : `docs/architecture-redb.md §5` et `docs/redb-queries.md`.

Le shim PromQL de `daly-bms-server` n'est **pas** un PromQL complet. Fonctionnalités
**supportées** et **non supportées** :

| Fonctionnalité | Supportée | Alternative |
|----------------|-----------|-------------|
| `offset` modifier (`[1d] offset 1d`) | Oui | — |
| `avg_over_time`, `max_over_time`, `min_over_time` | Oui | — |
| `increase()` sur compteurs | Oui | — |
| `sum()`, `avg()`, `min()`, `max()` (agrégations) | Oui | — |
| `quantile()` | Oui | — |
| `@ start()` | Oui | — |
| Subqueries `[range:step]` | **Non** | `query_range` puis agrégation côté client |
| `integrate()` (MetricsQL) | **Non** | `avg_over_time(p[w]) * heures` |
| Fonctions MetricsQL / VictoriaMetrics exclusives | **Non** | Équivalent PromQL standard |

> ⚠️ Les panels de cumul à base de subquery (ex. `increase(total_solar_power[1y:1d])`)
> **ne fonctionnent pas** avec le shim redb. Utiliser `query_range` sur la période puis
> agréger côté client (JavaScript/Python). Le modificateur `offset` fonctionne lui
> correctement.

### 8.5 JSON complet du dashboard pv-solar-5y

Le JSON ci-dessous est le dashboard de comparaison PV 5 ans. Il utilise `${datasource}`
(variable template) — à importer manuellement via Grafana UI ou adapter pour le
provisioning (remplacer `${datasource}` par `daly-metrics`).

```json
{
  "annotations": {
    "list": [
      {
        "builtIn": 1,
        "datasource": {
          "type": "grafana",
          "uid": "-- Grafana --"
        },
        "enable": true,
        "hide": true,
        "iconColor": "rgba(0, 211, 255, 1)",
        "name": "Annotations & Alerts",
        "type": "dashboard"
      }
    ]
  },
  "editable": true,
  "fiscalYearStartMonth": 0,
  "graphTooltip": 0,
  "id": null,
  "links": [],
  "liveNow": false,
  "panels": [
    {
      "collapsed": false,
      "gridPos": { "h": 1, "w": 24, "x": 0, "y": 0 },
      "id": 20,
      "panels": [],
      "title": "Puissance Instantanée",
      "type": "row"
    },
    {
      "datasource": { "type": "prometheus", "uid": "${datasource}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "thresholds" },
          "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              { "color": "red", "value": null },
              { "color": "yellow", "value": 500 },
              { "color": "green", "value": 1500 }
            ]
          },
          "unit": "watt"
        },
        "overrides": []
      },
      "gridPos": { "h": 4, "w": 8, "x": 0, "y": 1 },
      "id": 2,
      "options": {
        "colorMode": "value",
        "graphMode": "area",
        "justifyMode": "auto",
        "orientation": "auto",
        "reduceOptions": { "calcs": ["lastNotNull"], "fields": "", "values": false },
        "textMode": "auto"
      },
      "pluginVersion": "10.0.0",
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "total_solar_power",
          "refId": "A"
        }
      ],
      "title": "Puissance PV Totale",
      "type": "stat"
    },
    {
      "datasource": { "type": "prometheus", "uid": "${datasource}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "palette-classic" },
          "custom": {
            "axisCenteredZero": false, "axisColorMode": "text", "axisLabel": "",
            "axisPlacement": "auto", "barAlignment": 0, "drawStyle": "line",
            "fillOpacity": 10, "gradientMode": "none",
            "hideFrom": { "legend": false, "tooltip": false, "viz": false },
            "insertNulls": false, "lineInterpolation": "linear", "lineWidth": 1,
            "pointSize": 5, "scaleDistribution": { "type": "linear" },
            "showPoints": "never", "spanNulls": false,
            "stacking": { "group": "A", "mode": "none" },
            "thresholdsStyle": { "mode": "off" }
          },
          "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [{ "color": "green", "value": null }]
          },
          "unit": "watt"
        },
        "overrides": []
      },
      "gridPos": { "h": 8, "w": 16, "x": 8, "y": 1 },
      "id": 3,
      "options": {
        "legend": { "calcs": [], "displayMode": "list", "placement": "bottom", "showLegend": true },
        "tooltip": { "mode": "multi", "sort": "none" }
      },
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "total_solar_power",
          "legendFormat": "Total",
          "refId": "A"
        }
      ],
      "title": "Courbe Puissance (Temps Réel)",
      "type": "timeseries"
    },
    {
      "collapsed": false,
      "gridPos": { "h": 1, "w": 24, "x": 0, "y": 9 },
      "id": 21, "panels": [],
      "title": "Comparaison Quotidienne",
      "type": "row"
    },
    {
      "datasource": { "type": "prometheus", "uid": "${datasource}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "thresholds" }, "mappings": [],
          "thresholds": { "mode": "absolute", "steps": [{ "color": "blue", "value": null }] },
          "unit": "watth"
        },
        "overrides": []
      },
      "gridPos": { "h": 4, "w": 6, "x": 0, "y": 10 },
      "id": 4,
      "options": {
        "colorMode": "value", "graphMode": "area", "justifyMode": "auto",
        "orientation": "auto",
        "reduceOptions": { "calcs": ["lastNotNull"], "fields": "", "values": false },
        "textMode": "auto"
      },
      "pluginVersion": "10.0.0",
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "increase(total_solar_power[1d])",
          "refId": "A"
        }
      ],
      "title": "Aujourd'hui (Wh)",
      "type": "stat"
    },
    {
      "datasource": { "type": "prometheus", "uid": "${datasource}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "thresholds" }, "mappings": [],
          "thresholds": { "mode": "absolute", "steps": [{ "color": "purple", "value": null }] },
          "unit": "watth"
        },
        "overrides": []
      },
      "gridPos": { "h": 4, "w": 6, "x": 6, "y": 10 },
      "id": 5,
      "options": {
        "colorMode": "value", "graphMode": "area", "justifyMode": "auto",
        "orientation": "auto",
        "reduceOptions": { "calcs": ["lastNotNull"], "fields": "", "values": false },
        "textMode": "auto"
      },
      "pluginVersion": "10.0.0",
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "increase(total_solar_power[1d] offset 1d)",
          "refId": "A"
        }
      ],
      "title": "Hier (Wh)",
      "type": "stat"
    },
    {
      "datasource": { "type": "prometheus", "uid": "${datasource}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "thresholds" }, "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              { "color": "red", "value": null },
              { "color": "green", "value": 0 }
            ]
          },
          "unit": "percent"
        },
        "overrides": []
      },
      "gridPos": { "h": 4, "w": 6, "x": 12, "y": 10 },
      "id": 6,
      "options": {
        "colorMode": "value", "graphMode": "none", "justifyMode": "auto",
        "orientation": "auto",
        "reduceOptions": { "calcs": ["lastNotNull"], "fields": "", "values": false },
        "textMode": "auto"
      },
      "pluginVersion": "10.0.0",
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "((increase(total_solar_power[1d]) - increase(total_solar_power[1d] offset 1d)) / increase(total_solar_power[1d] offset 1d)) * 100",
          "refId": "A"
        }
      ],
      "title": "Variation J/J-1 (%)",
      "type": "stat"
    },
    {
      "collapsed": false,
      "gridPos": { "h": 1, "w": 24, "x": 0, "y": 14 },
      "id": 22, "panels": [],
      "title": "Comparaison Mensuelle",
      "type": "row"
    },
    {
      "datasource": { "type": "prometheus", "uid": "${datasource}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "palette-classic" },
          "custom": {
            "axisCenteredZero": false, "axisColorMode": "text", "axisLabel": "",
            "axisPlacement": "auto", "barAlignment": 0, "drawStyle": "bars",
            "fillOpacity": 80, "gradientMode": "none",
            "hideFrom": { "legend": false, "tooltip": false, "viz": false },
            "insertNulls": false, "lineInterpolation": "linear", "lineWidth": 1,
            "pointSize": 5, "scaleDistribution": { "type": "linear" },
            "showPoints": "never", "spanNulls": false,
            "stacking": { "group": "A", "mode": "none" },
            "thresholdsStyle": { "mode": "off" }
          },
          "mappings": [],
          "thresholds": { "mode": "absolute", "steps": [{ "color": "green", "value": null }] },
          "unit": "watth"
        },
        "overrides": []
      },
      "gridPos": { "h": 8, "w": 24, "x": 0, "y": 15 },
      "id": 7,
      "options": {
        "legend": {
          "calcs": ["mean", "max"], "displayMode": "table",
          "placement": "right", "showLegend": true
        },
        "tooltip": { "mode": "multi", "sort": "none" }
      },
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "increase(total_solar_power[30d])",
          "legendFormat": "Mois en cours", "refId": "A"
        },
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "increase(total_solar_power[30d] offset 30d)",
          "legendFormat": "Mois précédent", "refId": "B"
        }
      ],
      "title": "Production Mensuelle (Wh)",
      "type": "timeseries"
    },
    {
      "collapsed": false,
      "gridPos": { "h": 1, "w": 24, "x": 0, "y": 23 },
      "id": 23, "panels": [],
      "title": "Comparaison Annuelle - 5 Ans",
      "type": "row"
    },
    {
      "datasource": { "type": "prometheus", "uid": "${datasource}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "palette-classic" },
          "custom": {
            "axisCenteredZero": false, "axisColorMode": "text", "axisLabel": "",
            "axisPlacement": "auto", "barAlignment": 0, "drawStyle": "line",
            "fillOpacity": 0, "gradientMode": "none",
            "hideFrom": { "legend": false, "tooltip": false, "viz": false },
            "insertNulls": false, "lineInterpolation": "smooth", "lineWidth": 2,
            "pointSize": 3, "scaleDistribution": { "type": "linear" },
            "showPoints": "auto", "spanNulls": false,
            "stacking": { "group": "A", "mode": "none" },
            "thresholdsStyle": { "mode": "off" }
          },
          "mappings": [],
          "thresholds": { "mode": "absolute", "steps": [{ "color": "green", "value": null }] },
          "unit": "watth"
        },
        "overrides": []
      },
      "gridPos": { "h": 10, "w": 24, "x": 0, "y": 24 },
      "id": 8,
      "options": {
        "legend": {
          "calcs": ["mean", "max", "min"], "displayMode": "table",
          "placement": "right", "showLegend": true
        },
        "tooltip": { "mode": "multi", "sort": "none" }
      },
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "increase(total_solar_power[1y])",
          "legendFormat": "Année N", "refId": "A"
        },
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "increase(total_solar_power[1y] offset 1y)",
          "legendFormat": "Année N-1", "refId": "B"
        },
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "increase(total_solar_power[1y] offset 2y)",
          "legendFormat": "Année N-2", "refId": "C"
        },
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "increase(total_solar_power[1y] offset 3y)",
          "legendFormat": "Année N-3", "refId": "D"
        },
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "increase(total_solar_power[1y] offset 4y)",
          "legendFormat": "Année N-4", "refId": "E"
        }
      ],
      "title": "Production Annuelle sur 5 Ans (Comparaison)",
      "type": "timeseries"
    },
    {
      "datasource": { "type": "prometheus", "uid": "${datasource}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "thresholds" }, "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              { "color": "red", "value": null },
              { "color": "yellow", "value": -5 },
              { "color": "green", "value": 0 }
            ]
          },
          "unit": "percent"
        },
        "overrides": []
      },
      "gridPos": { "h": 4, "w": 12, "x": 0, "y": 34 },
      "id": 9,
      "options": {
        "colorMode": "value", "graphMode": "area", "justifyMode": "auto",
        "orientation": "auto",
        "reduceOptions": { "calcs": ["lastNotNull"], "fields": "", "values": false },
        "textMode": "auto"
      },
      "pluginVersion": "10.0.0",
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "((increase(total_solar_power[1y]) - increase(total_solar_power[1y] offset 1y)) / increase(total_solar_power[1y] offset 1y)) * 100",
          "refId": "A"
        }
      ],
      "title": "Dégradation Annuelle N vs N-1 (%)",
      "type": "stat"
    },
    {
      "datasource": { "type": "prometheus", "uid": "${datasource}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "palette-classic" },
          "custom": {
            "axisCenteredZero": false, "axisColorMode": "text", "axisLabel": "",
            "axisPlacement": "auto", "barAlignment": 0, "drawStyle": "bars",
            "fillOpacity": 80, "gradientMode": "none",
            "hideFrom": { "legend": false, "tooltip": false, "viz": false },
            "insertNulls": false, "lineInterpolation": "linear", "lineWidth": 1,
            "pointSize": 5, "scaleDistribution": { "type": "linear" },
            "showPoints": "never", "spanNulls": false,
            "stacking": { "group": "A", "mode": "none" },
            "thresholdsStyle": { "mode": "off" }
          },
          "mappings": [],
          "thresholds": { "mode": "absolute", "steps": [{ "color": "green", "value": null }] },
          "unit": "watth"
        },
        "overrides": []
      },
      "gridPos": { "h": 8, "w": 24, "x": 0, "y": 38 },
      "id": 10,
      "options": {
        "legend": {
          "calcs": ["sum"], "displayMode": "table",
          "placement": "right", "showLegend": true
        },
        "tooltip": { "mode": "multi", "sort": "none" }
      },
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "sum(increase(total_solar_power[30d]))",
          "legendFormat": "Total Mensuel", "refId": "A"
        }
      ],
      "title": "Production Mensuelle Cumulée (Wh)",
      "type": "timeseries"
    },
    {
      "collapsed": false,
      "gridPos": { "h": 1, "w": 24, "x": 0, "y": 46 },
      "id": 24, "panels": [],
      "title": "Profil Journalier (Comparaison Saisonnière)",
      "type": "row"
    },
    {
      "datasource": { "type": "prometheus", "uid": "${datasource}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "palette-classic" },
          "custom": {
            "axisCenteredZero": false, "axisColorMode": "text", "axisLabel": "",
            "axisPlacement": "auto", "barAlignment": 0, "drawStyle": "line",
            "fillOpacity": 10, "gradientMode": "none",
            "hideFrom": { "legend": false, "tooltip": false, "viz": false },
            "insertNulls": false, "lineInterpolation": "smooth", "lineWidth": 2,
            "pointSize": 3, "scaleDistribution": { "type": "linear" },
            "showPoints": "auto", "spanNulls": false,
            "stacking": { "group": "A", "mode": "none" },
            "thresholdsStyle": { "mode": "off" }
          },
          "mappings": [],
          "thresholds": { "mode": "absolute", "steps": [{ "color": "green", "value": null }] },
          "unit": "watt"
        },
        "overrides": []
      },
      "gridPos": { "h": 8, "w": 24, "x": 0, "y": 47 },
      "id": 11,
      "options": {
        "legend": {
          "calcs": ["mean"], "displayMode": "table",
          "placement": "right", "showLegend": true
        },
        "tooltip": { "mode": "multi", "sort": "none" }
      },
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${datasource}" },
          "expr": "avg_over_time(total_solar_power[1d])",
          "legendFormat": "Moyenne Journalière", "refId": "A"
        }
      ],
      "title": "Profil de Production Moyen (W)",
      "type": "timeseries"
    }
  ],
  "refresh": "30s",
  "schemaVersion": 38,
  "style": "dark",
  "tags": ["pv", "solaire", "victron", "redb"],
  "templating": {
    "list": [
      {
        "current": { "selected": false, "text": "Prometheus", "value": "prometheus" },
        "hide": 0,
        "includeAll": false,
        "label": "Data Source",
        "multi": false,
        "name": "datasource",
        "options": [],
        "query": "prometheus",
        "refresh": 1,
        "regex": "",
        "skipUrlSync": false,
        "type": "datasource"
      }
    ]
  },
  "time": { "from": "now-30d", "to": "now" },
  "timepicker": {},
  "timezone": "browser",
  "title": "PV Solaire - Monitoring & Comparaison 5 Ans",
  "uid": "pv-solar-5y",
  "version": 1,
  "weekStart": ""
}
```

---

## 9. Génération de rapports PDF

### Méthode 1 : Export natif Grafana (v9+)

1. Ouvrir le dashboard concerné
2. Cliquer sur **Share** (icône en haut à droite) → **Export** → **PDF**
3. Choisir la période (ex : "Last 30 days", "Last 1 year")
4. Télécharger le PDF

### Méthode 2 : Plugin Image Renderer (recommandé — via setup-grafana.sh)

```bash
# Installation via le script (recommandé, ~300 Mo incluant Chromium)
bash scripts/setup-grafana.sh --renderer

# Ou installation manuelle
sudo apt install -y chromium chromium-sandbox
sudo grafana-cli plugins install grafana-image-renderer
sudo systemctl restart grafana-server

# Utilisation depuis l'interface Grafana
# Dashboard → Share → Direct link rendered image → Format PDF
```

### Méthode 3 : API Grafana + script automatique

```bash
#!/bin/bash
# grafana_pdf_export.sh

GRAFANA_URL="http://localhost:3000"
API_KEY="votre-api-key"
DASHBOARD_UID="pv-solar-5y"
OUTPUT_DIR="/home/pi/reports"
DATE=$(date +%Y-%m-%d)

# Générer PDF via API Grafana render
curl -H "Authorization: Bearer ${API_KEY}" \
    "${GRAFANA_URL}/render/d/${DASHBOARD_UID}?width=1920&height=4000&from=now-30d&to=now" \
    -o "${OUTPUT_DIR}/rapport_pv_${DATE}.pdf"

# Pour planifier une exécution mensuelle (cron) :
# 0 1 1 * * /home/pi/grafana_pdf_export.sh
```

---

## 10. Dépannage Grafana

| Symptôme | Cause | Solution |
|----------|-------|----------|
| Grafana ne démarre pas | YAML provisioning invalide / erreur de config | `journalctl -u grafana-server -n 50` — examiner les erreurs YAML |
| Dossier "No items" — dashboards vides | Dashboards au mauvais format (export vs provisioning) | Vérifier : `__inputs` et `__requires` doivent être ABSENTS des JSON, UID datasource = `daly-metrics` (pas `${datasource}`) |
| "datasource not found" | Fichier `daly-metrics.yaml` absent ou UID incorrect | Vérifier `/etc/grafana/provisioning/datasources/daly-metrics.yaml`. Supprimer `victoriametrics.yaml` si résiduel : `sudo rm -f /etc/grafana/provisioning/datasources/victoriametrics.yaml` |
| Grafana ancien dossier "PV Solaire" vide | Dossier créé lors d'une ancienne configuration | Supprimer manuellement via UI Grafana : Dashboards → PV Solaire → Delete |
| Dashboard ET112 vide alors que les données existent | Format du label `address` incorrect | Le backend écrit `address="0x07"` (hex). Vérifier : `curl -s 'localhost:8080/api/v1/query?query=et112_power_w' \| jq '.data.result[].metric'`. Utiliser `address="0x07"` (pas `address="7"`) |
| Dashboard affiche cumul brut (non delta) | Baseline `pvinv_baseline` absent ou incorrect | Vérifier le topic MQTT retained : `santuario/persist/pvinv_baseline` |
| `deploy-pi5.sh` → `rustup: not found` | PATH root ≠ PATH user sous `sudo` | Builder sans sudo : `make build-arm && make build-energy-arm`, puis `sudo bash scripts/deploy-pi5.sh --no-build`. Pour dashboards seuls : `sudo bash scripts/fix-grafana.sh` |
| Grafana 11+ — "restricted database access" au provisioning fichier | Bug connu Grafana 11 | Utiliser `sudo bash scripts/fix-grafana.sh` (import via API HTTP à la place) |
| API healthcheck échoue | daly-bms-server non démarré | `systemctl status daly-bms` — démarrer et vérifier `curl -s http://localhost:8080/-/healthy` |
| Panels PV vides / "No data" | Métriques PV non disponibles (energy-manager non démarré) | `journalctl -u energy-manager -n 50`, vérifier MQTT (`santuario/energy/solar_total`) |
| Subqueries PromQL `[range:step]` sans résultat | Non supporté par le shim redb | Utiliser `query_range` + agrégation côté client (voir §8.4) |

Commandes de diagnostic :

```bash
# Logs Grafana en temps réel
journalctl -u grafana-server -f

# Healthcheck Grafana
curl -s http://localhost:3000/api/health

# Vérifier que la datasource répond
curl -s 'http://admin:admin@localhost:3000/api/datasources' | python3 -c "
import json, sys
for ds in json.load(sys.stdin):
    print(ds.get('uid'), ds.get('name'), ds.get('url'))
"

# Lister les dashboards provisionnés
curl -s 'http://admin:admin@localhost:3000/api/search?type=dash-db' | python3 -c "
import json, sys
for d in json.load(sys.stdin):
    print(d.get('uid'), d.get('title'), d.get('folderTitle',''))
"

# Vérifier les séries disponibles dans redb
curl -s 'http://localhost:8080/api/v1/redb/series' | jq '.data | length'

# Test requête PromQL directe
curl -s 'http://localhost:8080/api/v1/query?query=solar_total_w' | jq '.data.result'

# Taille base SQLite Grafana
du -sh /mnt/nvme/grafana/data/grafana.db 2>/dev/null || du -sh /var/lib/grafana/grafana.db
```

---

## 11. Désinstallation

```bash
# Via le script (supprime Grafana + provisioning, conserve les données NVMe)
sudo bash scripts/setup-grafana.sh --uninstall

# Ce que fait le script --uninstall :
# - systemctl stop grafana-server && systemctl disable grafana-server
# - apt-get remove -y grafana
# - rm -f /etc/grafana/provisioning/datasources/victoriametrics.yaml
# - rm -f /etc/grafana/provisioning/datasources/daly-metrics.yaml
# - rm -f /etc/grafana/provisioning/dashboards/daly-bms.yaml
# - rm -rf /var/lib/grafana/dashboards
# Les données NVMe (/mnt/nvme/grafana/) sont conservées (suppression manuelle si nécessaire)
```

Pour une suppression complète incluant les données NVMe :

```bash
sudo bash scripts/setup-grafana.sh --uninstall
sudo rm -rf /mnt/nvme/grafana/
```

---

## Voir aussi

- [./metriques-promql-reference.md](./metriques-promql-reference.md) — Catalogue des métriques disponibles et syntaxe PromQL complète, requêtes des panels Grafana.
- [./metriques-redb-architecture.md](./metriques-redb-architecture.md) — Architecture interne redb : moteur, tables, tiering, write path. À lire pour comprendre pourquoi certaines requêtes PromQL ne fonctionnent pas.
- [./deploiement-exploitation.md](./deploiement-exploitation.md) — Workflow de déploiement complet Pi5 + NanoPi, scripts `deploy-pi5.sh`, maintenance.
- [./app-daly-bms-server.md](./app-daly-bms-server.md) — Serveur principal : shim PromQL, écriture métriques (`redb_writes.rs`), endpoints `/api/v1/query`.
- [./app-energy-manager.md](./app-energy-manager.md) — energy-manager : publication des métriques solaires/énergie consommées par Grafana.
- [./alertes.md](./alertes.md) — AlertEngine natif Rust : règles, hysteresis, API alertes (complémentaire aux dashboards 17–20).
- [./ARCHITECTURE.md](./ARCHITECTURE.md) — Document maître : vue d'ensemble système et index de toute la documentation.

---

## Sources consolidées

Ce document fusionne et **remplace** l'ancien fichier suivant :
`docs/grafana-README_pv.md`
