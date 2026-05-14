# Perses + VictoriaMetrics — Monitoring PV Solaire

Guide d'installation de **Perses** (alternative légère et GitOps à Grafana)
sur Raspberry Pi 5, en **coexistence avec Grafana** pendant une phase d'essai.

---

## Pourquoi Perses en parallèle ?

| Critère | Grafana | Perses |
|---------|---------|--------|
| RAM au repos | ~150 MB | ~20 MB |
| Dashboards en Git | JSON peu lisible | YAML natif versionnables |
| Dépendances | PostgreSQL ou SQLite | Fichiers JSON uniquement |
| Maturité | Très mature | Jeune (CNCF sandbox) |
| Export PDF | Oui (plugin) | Non (prévu) |

**Stratégie** : installer Perses sur le port 8090, laisser Grafana sur 3000.
Les deux lisent VictoriaMetrics (port 8428) sans interférence.
Migration définitive uniquement si Perses couvre 100 % des besoins.

---

## Installation rapide

```bash
# Depuis ~/Daly-BMS-Rust
bash scripts/setup-perses.sh

# Avec données sur NVMe
bash scripts/setup-perses.sh --nvme

# Version spécifique
bash scripts/setup-perses.sh --version=0.49.0 --nvme
```

Le script est idempotent : relancer = mise à jour.

```bash
# Désinstaller
sudo bash scripts/setup-perses.sh --uninstall
```

---

## Installation manuelle (référence)

### 1. Téléchargement du binaire ARM64

```bash
mkdir -p ~/perses && cd ~/perses

VERSION=$(curl -sf https://api.github.com/repos/perses/perses/releases/latest \
    | grep tag_name | cut -d '"' -f 4)

wget "https://github.com/perses/perses/releases/download/${VERSION}/perses_${VERSION#v}_linux_arm64.tar.gz" \
    -O perses.tar.gz
tar xzf perses.tar.gz

sudo install -m 755 perses /usr/local/bin/
sudo install -m 755 percli /usr/local/bin/
```

### 2. Configuration minimale

`/etc/perses/config.yaml` :

```yaml
server:
  port: 8090
  enableUI: true

database:
  file:
    folder: /var/lib/perses/db
    extension: json

provisioning:
  dashboards:
    - folder: /etc/perses/dashboards
  datasources:
    - folder: /etc/perses/datasources
```

Créer les dossiers :

```bash
sudo mkdir -p /etc/perses/{dashboards,datasources} /var/lib/perses/db
sudo chown -R pi5compute:pi5compute /etc/perses /var/lib/perses
```

### 3. Service systemd

```bash
sudo tee /etc/systemd/system/perses.service > /dev/null <<'EOF'
[Unit]
Description=Perses Monitoring Dashboard
After=network.target

[Service]
Type=simple
User=pi5compute
ExecStart=/usr/local/bin/perses --config /etc/perses/config.yaml
Restart=always
RestartSec=5
LimitNOFILE=65535
WorkingDirectory=/var/lib/perses
StandardOutput=journal
StandardError=journal
SyslogIdentifier=perses

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable --now perses
```

### 4. Accès

- URL : `http://192.168.1.141:8090`
- Pas de login par défaut (configurable via OIDC ou basic auth si besoin)

---

## Configuration Datasource VictoriaMetrics

Fichier `/etc/perses/datasources/victoriametrics.yaml` :

```yaml
kind: GlobalDatasource
metadata:
  name: victoriametrics
spec:
  default: true
  plugin:
    kind: PrometheusDatasource
    spec:
      proxy:
        kind: HTTPProxy
        spec:
          url: "http://127.0.0.1:8428"
          allowedEndpoints:
            - endpointPattern: "/api/v1/.*"
              method: GET
            - endpointPattern: "/api/v1/.*"
              method: POST
```

> **Mode proxy** : Perses relaie les requêtes PromQL vers VictoriaMetrics
> côté serveur. Nécessaire pour l'accès depuis le LAN (le navigateur ne
> peut pas atteindre `127.0.0.1:8428` directement).

Redémarrer après modification :

```bash
sudo systemctl restart perses
```

---

## Dashboard PV Solaire

Le dashboard `contrib/perses/dashboards/pv-solar-5y.yaml` est automatiquement
copié dans `/etc/perses/dashboards/` par le script d'installation.

Il contient :
- Puissance instantanée (total, MPPT DC, micro-onduleurs)
- Production journalière et hebdomadaire
- Comparaison J / J-1
- Comparaison annuelle sur 5 ans superposée
- État des batteries BMS (SOC, tension, courant)
- Irradiance solaire (capteur PRALRAN)

### Migration depuis Grafana (optionnel)

Si tu as un dashboard Grafana existant à migrer :

```bash
# Exporter le JSON Grafana puis migrer
percli migrate -f pv-solar-5y.json -o yaml > pv-solar-5y.yaml

# Vérifier et appliquer
percli apply -f pv-solar-5y.yaml --project default

# Copier pour provisioning automatique
sudo cp pv-solar-5y.yaml /etc/perses/dashboards/
```

> La migration est best-effort. Les panels Prometheus/PromQL fonctionnent
> très bien. Certains types de panels Grafana avancés peuvent nécessiter
> des ajustements manuels.

---

## Commandes utiles

```bash
# Logs
journalctl -u perses -f

# Status
systemctl status perses

# Relancer après modification config
sudo systemctl restart perses

# Vérifier health
curl http://127.0.0.1:8090/api/v1/health

# Lister les ressources provisionnées
percli get datasource
percli get dashboard

# Mise à jour (idempotent)
bash scripts/setup-perses.sh
```

---

## Architecture — Coexistence Grafana + Perses

```
Victron GX → MQTT → Pi5 (daly-bms) → VictoriaMetrics :8428
                                             |
                              ┌──────────────┼──────────────┐
                              ▼                             ▼
                    Grafana :3000                  Perses :8090
                    (ancien)                       (essai)
```

Portes utilisées sur le Pi5 :

| Service | Port |
|---------|------|
| daly-bms-server | 8080 |
| VictoriaMetrics | 8428 |
| Grafana | 3000 |
| Perses | 8090 |

---

## Décision de migration

Après la phase d'essai, choisir l'une de ces options :

**Option A — Garder Perses uniquement**
```bash
sudo systemctl disable --now grafana-server
# Libère ~130 MB RAM
```

**Option B — Garder les deux**
```bash
# Ne rien faire — coexistence stable
# Perses pour GitOps/alerting, Grafana pour exports PDF
```

**Option C — Revenir sur Grafana seul**
```bash
sudo bash scripts/setup-perses.sh --uninstall
```

---

## Notes

- **Rétention** : gérée côté VictoriaMetrics (`-retentionPeriod=5y`)
- **Mise à jour Perses** : relancer `bash scripts/setup-perses.sh` (détecte
  automatiquement la dernière version)
- **Dashboards versionés** : modifier `contrib/perses/dashboards/*.yaml` dans
  Git, puis `make sync` sur le Pi5 + `sudo systemctl restart perses`
- **Export PDF** : non disponible dans Perses pour l'instant — garder Grafana
  en parallèle si tu en as besoin
