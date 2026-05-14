# Perses + VictoriaMetrics — Monitoring PV Solaire

Guide d'installation de **Perses** (alternative légère et GitOps à Grafana)
sur Raspberry Pi 5, en **coexistence avec Grafana** pendant une phase d'essai.

Testé et validé avec **Perses 0.53.1** sur Pi5 aarch64.

---

## Pourquoi Perses en parallèle ?

| Critère | Grafana | Perses |
|---------|---------|--------|
| RAM au repos | ~150 MB | ~20 MB |
| Dashboards en Git | JSON peu lisible | YAML natif versionnables |
| Dépendances | PostgreSQL ou SQLite | Fichiers JSON uniquement |
| Maturité | Très mature | Jeune (CNCF sandbox) |
| Export PDF | Oui (plugin) | Non (prévu) |

**Stratégie** : Perses sur port 8090, Grafana reste sur 3000.
Les deux lisent VictoriaMetrics (port 8428) sans interférence.

---

## Installation rapide

```bash
# Depuis ~/Daly-BMS-Rust (après make sync)
bash scripts/setup-perses.sh --nvme

# Sans NVMe
bash scripts/setup-perses.sh

# Désinstaller
sudo bash scripts/setup-perses.sh --uninstall
```

Le script est **idempotent** : relancer = mise à jour.

---

## Points techniques importants (Perses 0.50+)

### Archive plate

L'archive de release `perses_X.Y.Z_linux_arm64.tar.gz` extrait ses fichiers
**à la racine** (pas de sous-dossier). Les binaires `perses` et `percli` sont
directement dans le répertoire d'extraction.

### Plugins

Perses 0.50+ utilise un système de plugins. L'archive contient un dossier
`plugins-archive/` avec des archives `.tar.gz` par plugin. Le script extrait
chaque plugin dans `/etc/perses/plugins/<NomPlugin>/`.

**Auto-découverte** : Perses charge `./plugins/` relatif au `WorkingDirectory`
du service systemd (`/etc/perses`). **Aucune clé de config n'est nécessaire.**

> ⚠️ Ne pas ajouter `plugins.archive_path` ni `plugins.path` dans `config.yaml` :
> ces clés font crasher Perses 0.53.

### Kind du plugin Prometheus

Le schéma CUE du plugin déclare `#kind: "PrometheusDatasource"` — c'est le nom
à utiliser partout, **pas** `Prometheus` :

```yaml
# GlobalDatasource
spec:
  plugin:
    kind: PrometheusDatasource   # ✓ correct
    # kind: Prometheus           # ✗ → "schema not found for plugin Prometheus"
```

### config.yaml minimal

```yaml
database:
  file:
    folder: /mnt/nvme/perses/db   # ou /var/lib/perses/db
    extension: json

provisioning:
  interval: 1m
  folders:
    - /etc/perses/provisioning
```

> ⚠️ Pas de bloc `server:` dans le config — Perses 0.50+ l'ignore ou crashe.
> Le port est passé via le flag CLI `--web.listen-address=:8090`.

### Service systemd

```ini
[Service]
User=pi5compute
WorkingDirectory=/etc/perses          # ← plugin auto-discovery ./plugins/
ExecStart=/usr/local/bin/perses \
    --config /etc/perses/config.yaml \
    --web.listen-address=:8090
```

---

## Structure des fichiers

```
/etc/perses/
├── config.yaml                       ← config minimale
├── plugins/                          ← plugins extraits (auto-découverte)
│   ├── Prometheus-0.57.1/
│   ├── StatChart-0.12.1/
│   └── ...
└── provisioning/                     ← ressources auto-chargées (interval: 1m)
    ├── project-default.yaml          ← kind: Project
    ├── victoriametrics-datasource.yaml  ← kind: GlobalDatasource
    └── pv-solar-5y.yaml              ← kind: Dashboard
```

Sources versionées dans le repo :

```
contrib/perses/
├── project-default.yaml
├── victoriametrics-datasource.yaml
└── dashboards/
    └── pv-solar-5y.yaml
```

---

## Datasource VictoriaMetrics

`contrib/perses/victoriametrics-datasource.yaml` :

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
> côté serveur. Nécessaire car le navigateur (LAN) ne peut pas atteindre
> `127.0.0.1:8428` directement.

---

## Import / application des ressources

Le provisioning par fichier est automatique (délai jusqu'à 1 min).
Pour un import immédiat via `percli` :

```bash
# Se connecter
percli login http://localhost:8090

# Appliquer dans l'ordre (projet → datasource → dashboard)
percli apply -f /etc/perses/provisioning/project-default.yaml
percli apply -f /etc/perses/provisioning/victoriametrics-datasource.yaml
percli apply -f /etc/perses/provisioning/pv-solar-5y.yaml

# Vérifier
percli get project
percli get globaldatasource
percli get dashboard -p default
```

---

## Dashboard PV Solaire

`contrib/perses/dashboards/pv-solar-5y.yaml` — panels inclus :

| Section | Panels |
|---------|--------|
| Puissance instantanée | Total PV, MPPT DC, Micro-onduleurs AC, irradiance |
| Batteries | SOC BMS 1 (360Ah), SOC BMS 2 (320Ah) |
| Courbes | Puissance temps réel, tensions, courants, irradiance |
| Journalier | Aujourd'hui, hier, cette semaine, variation J/J-1 |
| Historique | Production journalière 30j (barres) |
| Comparaison | 5 ans superposés, variation annuelle N vs N-1 |

Chaque panel référence explicitement la datasource :
```yaml
datasource:
  kind: PrometheusDatasource
  name: victoriametrics
```

### Modifier le dashboard (GitOps)

```bash
# 1. Éditer sur le poste de dev
vim contrib/perses/dashboards/pv-solar-5y.yaml

# 2. Commit + push
git add contrib/perses/dashboards/pv-solar-5y.yaml
git commit -m "feat(perses): ..."
git push

# 3. Sur Pi5
make sync
sudo cp ~/Daly-BMS-Rust/contrib/perses/dashboards/pv-solar-5y.yaml \
    /etc/perses/provisioning/pv-solar-5y.yaml
# Le provisioning recharge automatiquement toutes les minutes
# OU forcer :
percli apply -f /etc/perses/provisioning/pv-solar-5y.yaml
```

---

## Commandes utiles

```bash
# Logs
journalctl -u perses -f

# Status
systemctl status perses

# Healthcheck
curl http://127.0.0.1:8090/api/v1/health

# Relancer
sudo systemctl restart perses

# Lister les ressources
percli get project
percli get globaldatasource
percli get dashboard -p default

# Diagnostics plugins
ls /etc/perses/plugins/
ls /etc/perses/plugins/Prometheus-0.57.1/schemas/

# Vérifier config générée
cat /etc/perses/config.yaml
```

---

## Architecture — Coexistence Grafana + Perses

```
Victron GX → MQTT → Pi5 (daly-bms) → VictoriaMetrics :8428
                                             |
                              ┌──────────────┴──────────────┐
                              ▼                             ▼
                    Grafana :3000                  Perses :8090
                    (dashboards existants)          (GitOps, YAML versionné)
```

| Service | Port |
|---------|------|
| daly-bms-server | 8080 |
| energy-manager | 8081 |
| VictoriaMetrics | 8428 |
| Grafana | 3000 |
| Perses | 8090 |

---

## Décision de migration

**Option A — Garder Perses uniquement**
```bash
sudo systemctl disable --now grafana-server
# Libère ~130 MB RAM
```

**Option B — Coexistence permanente**
```bash
# Rien à faire — les deux coexistent sans interférence
# Perses pour GitOps, Grafana pour exports PDF
```

**Option C — Revenir sur Grafana seul**
```bash
sudo bash scripts/setup-perses.sh --uninstall
```

---

## Troubleshooting

| Symptôme | Cause | Solution |
|----------|-------|----------|
| `connection refused` au démarrage | Clé inconnue dans config.yaml | Vérifier qu'il n'y a pas de bloc `server:` ni `plugins.archive_path` |
| `schema not found for plugin Prometheus` | Mauvais kind dans la datasource | Utiliser `kind: PrometheusDatasource` (pas `Prometheus`) |
| `No datasource found for kind 'PrometheusDatasource' and name 'undefined'` | Dashboard importé depuis Grafana sans datasource | `percli apply -f /etc/perses/provisioning/victoriametrics-datasource.yaml` |
| Dashboard vide après provisioning | Délai jusqu'à 1 min | Attendre ou `percli apply -f ...` pour forcer |
| `install: cannot stat 'perses_X.Y.Z_linux_arm64/perses'` | Archive plate (pas de sous-dossier) | Utiliser `EXTRACT_DIR="$TMPDIR_PERSES"` directement |
| `percli: unknown flag --server` | percli utilise un fichier de config | `percli login http://localhost:8090` d'abord |
