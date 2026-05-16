**tuning VictoriaMetrics** pour diviser sa consommation RAM par 2 à 3, sans toucher à une ligne de code Rust.

---

## Fichier de service systemd allégé

**Fichier :** `contrib/victoriametrics.service`

```ini
[Unit]
Description=VictoriaMetrics (allégé)
After=network.target
Wants=network.target

[Service]
Type=simple
User=victoriametrics
Group=victoriametrics
WorkingDirectory=/var/lib/victoria-metrics-data

# Flags critiques pour réduire l'empreinte mémoire
ExecStart=/usr/local/bin/victoria-metrics \
    -storageDataPath=/var/lib/victoria-metrics-data \
    -retentionPeriod=5y \
    -memory.allowedPercent=5 \
    -memory.allowedBytes=80MB \
    -search.maxMemoryPerQuery=30MB \
    -search.maxQueryDuration=30s \
    -search.maxConcurrentRequests=4 \
    -search.maxUniqueTimeseries=1000 \
    -search.maxSamplesPerSeries=30000 \
    -storage.maxDailySeries=5000 \
    -storage.maxHourlySeries=500 \
    -promscrape.config=/etc/victoriametrics/scrape.yml \
    -promscrape.maxScrapeSize=16MB \
    -promscrape.streamParse=true \
    -httpListenAddr=:8428

# Sécurité et limites
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/victoria-metrics-data
LimitNOFILE=65536
LimitNPROC=4096

# Limite mémoire dure (systemd cgroup)
MemoryMax=120M
MemorySwapMax=0

Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

---

## Explication des flags

| Flag | Valeur | Effet |
|---|---|---|
| `-memory.allowedPercent=5` | 5% | VM limite ses caches internes à 5% de la RAM totale |
| `-memory.allowedBytes=80MB` | 80 Mo | Plafond absolu, plus fort que le pourcentage |
| `-search.maxMemoryPerQuery=30MB` | 30 Mo | Empêche une requête Grafana "5 ans" de saturer la RAM |
| `-search.maxQueryDuration=30s` | 30s | Kill les requêtes trop lentes (évite les fuites) |
| `-search.maxConcurrentRequests=4` | 4 | Limite les requêtes parallèles (moins de buffers simultanés) |
| `-search.maxUniqueTimeseries=1000` | 1000 | Vous avez ~50 séries, donc 1000 est très large |
| `-storage.maxDailySeries=5000` | 5000 | Limite l'index mémoire des séries par jour |
| `-storage.maxHourlySeries=500` | 500 | Réduit les buffers horaires |
| `MemoryMax=120M` | 120 Mo | **Systemd tue le processus s'il dépasse** (sécurité) |

---

## Script de déploiement

**Fichier :** `contrib/install-vm-light.sh`

```bash
#!/bin/bash
set -e

echo "=== Tuning VictoriaMetrics (mode allégé) ==="

# 1. Créer user si absent
if ! id -u victoriametrics &>/dev/null; then
    sudo useradd -r -s /bin/false victoriametrics
fi

# 2. Permissions données
sudo mkdir -p /var/lib/victoria-metrics-data
sudo chown -R victoriametrics:victoriametrics /var/lib/victoria-metrics-data

# 3. Copier le service allégé
sudo cp contrib/victoriametrics.service /etc/systemd/system/
sudo systemctl daemon-reload

# 4. Arrêter l'ancien service si existant
sudo systemctl stop victoriametrics 2>/dev/null || true

# 5. Démarrer le service allégé
sudo systemctl enable --now victoriametrics-light

# 6. Vérification
sleep 3
echo ""
echo "=== Statut ==="
systemctl status victoriametrics-light --no-pager
echo ""
echo "=== RAM utilisée ==="
ps -o pid,comm,rss -p $(pgrep -f victoria-metrics) | awk '{print $3/1024 " Mo"}'
echo ""
echo "=== Test endpoint ==="
curl -s http://localhost:8428/api/v1/status/tsdb | head -c 200
echo ""
echo "OK. VictoriaMetrics allégé démarré."
```

---

## Résultats attendus

| Métrique | Avant (défaut) | Après (tuning) |
|---|---|---|
| **RAM au démarrage** | ~150 Mo | ~40–60 Mo |
| **RAM sous charge** | ~350 Mo | ~80–100 Mo |
| **Requêtes lentes** | Possible | Limitées à 30s |
| **Stabilité** | Standard | + sécurisée (OOM impossible) |

---

## Validation en production

Après déploiement, surveillez 24h :

```bash
# RAM toutes les 10s
watch -n 10 'ps -o pid,comm,rss,vsz -p $(pgrep -f victoria-metrics)'

# Requêtes Grafana : vérifier que les dashboards "5 ans" répondent en < 10s
# Si une requête timeout → augmenter -search.maxMemoryPerQuery à 50MB

# Logs OOM
journalctl -u victoriametrics-light -f

# Si systemd tue VM (MemoryMax atteint) :
# - Vérifier quelle requête Grafana cause le pic
# - Augmenter MemoryMax à 150M temporairement
# - Ou ajouter un recording rule pour pré-agréger les données lourdes
```

---

## Prochaine étape

Si après 1 semaine ce tuning vous satisfait (RAM < 100 Mo, dashboards réactifs), vous pouvez **rester sur VM allégé** — c'est une solution pérenne.

Si vous voulez quand même aller vers SQLite plus tard, ce tuning vous donne le **temps de développer la migration sans pression**.
