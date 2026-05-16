# Tuning VictoriaMetrics — Pi5 Daly-BMS

Réduire l'empreinte mémoire de VictoriaMetrics **sans toucher au code Rust** ni au stockage existant.

> **Baseline actuelle (à confirmer avant tuning)** : ~120 Mo RSS, légère dérive
> à la hausse. Rétention 5 ans sur NVMe, ~50 séries actives.
> Objectif après tuning : 60–90 Mo stable + garde-fou OOM via cgroup systemd.

---

## 0. Contexte projet

| Paramètre | Valeur prod actuelle |
|---|---|
| Binaire | `/usr/local/bin/victoria-metrics-prod` |
| Storage path | `/mnt/nvme/victoria-metrics` (NVMe, monté via `mnt-nvme.mount`) |
| User / Group | `victoriametrics` / `victoriametrics` |
| Port HTTP | `8428` |
| Rétention | `5y` |
| Service systemd | `victoriametrics.service` (fichier `contrib/victoriametrics.service`) |
| Producteurs | `daly-bms-server` (push direct) + `energy-manager` (push direct). Pas de `promscrape` (le fichier `victoriametrics-scrape.yml` a `scrape_configs: []`) |

Le service existant contient déjà :
`-retentionPeriod=5y`, `-selfScrapeInterval=0`, `-maxLabelsPerTimeseries=30`,
`-search.maxQueryDuration=30s`, `-search.maxConcurrentRequests=4`.

Ce tuning **ajoute** les flags mémoire manquants et le durcissement systemd.

---

## 1. Fichier service proposé

**Fichier :** `contrib/victoriametrics.service` (remplacement, pas un nouveau service)

```ini
[Unit]
Description=VictoriaMetrics Time Series Database (tuned)
After=network-online.target mnt-nvme.mount
Requires=mnt-nvme.mount

[Service]
Type=simple
User=victoriametrics
Group=victoriametrics
WorkingDirectory=/mnt/nvme/victoria-metrics

ExecStart=/usr/local/bin/victoria-metrics-prod \
    -storageDataPath=/mnt/nvme/victoria-metrics \
    -retentionPeriod=5y \
    -httpListenAddr=:8428 \
    -selfScrapeInterval=0 \
    -maxLabelsPerTimeseries=30 \
    \
    -memory.allowedPercent=10 \
    -memory.allowedBytes=80MB \
    \
    -search.maxQueryDuration=30s \
    -search.maxConcurrentRequests=4 \
    -search.maxMemoryPerQuery=40MB \
    -search.maxUniqueTimeseries=2000 \
    -search.maxSamplesPerSeries=50000 \
    \
    -storage.maxDailySeries=5000 \
    -storage.maxHourlySeries=500

# Durcissement
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/mnt/nvme/victoria-metrics
LimitNOFILE=65536
LimitNPROC=4096

# Garde-fou mémoire (systemd kill si dépassé)
MemoryMax=150M
MemorySwapMax=0

Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

### Différences vs service actuel

| Ajout | But |
|---|---|
| `-memory.allowedPercent=10` + `-memory.allowedBytes=80MB` | Plafonne les caches internes VM (gain RAM principal) |
| `-search.maxMemoryPerQuery=40MB` | Une requête `/dashboard/history` "5 ans" ne peut plus saturer la RAM |
| `-search.maxUniqueTimeseries=2000` | Marge x40 vs ~50 séries actuelles |
| `-search.maxSamplesPerSeries=50000` | Limite par série (60s × 50000 ≈ 34 jours par requête) |
| `-storage.maxDailySeries=5000` / `maxHourlySeries=500` | Anti-explosion accidentelle (typo de label, etc.) |
| `MemoryMax=150M` / `MemorySwapMax=0` | systemd OOM-kill avant que ça déborde sur Rust |
| `ProtectSystem=strict` + `ReadWritePaths=...` | Le process ne peut écrire QUE sur le NVMe |
| `WorkingDirectory=...` | Évite les surprises de cwd |

> **À noter** : `-promscrape.config=...` du draft initial est **retiré** —
> notre `scrape_configs` est vide (push direct depuis Rust).
> Si on ajoute un jour un exporter externe (Mosquitto, node_exporter…),
> rajouter `-promscrape.config=/etc/victoriametrics/scrape.yml -promscrape.streamParse=true`.

---

## 2. Pourquoi les valeurs sont moins agressives que le draft initial

Le draft proposait `-memory.allowedBytes=80MB` + `MemoryMax=120M`.
Vu qu'on est **déjà à 120 Mo en régime nominal**, plafonner à 120 Mo
garantit des kills systemd réguliers.

Choix prudents :
- `MemoryMax=150M` laisse une marge de sécurité de ~25 % au-dessus du baseline.
- `-memory.allowedPercent=10` (au lieu de 5 %) : sur 8 Go RAM = 800 Mo théoriques,
  donc le plafond effectif sera `-memory.allowedBytes=80MB`.
- `maxMemoryPerQuery=40MB` (au lieu de 30) : nos requêtes 5 ans agrègent
  beaucoup de samples, 30 Mo serait probablement trop juste.

Si après 1 semaine la RSS reste < 90 Mo, on pourra durcir à `MemoryMax=120M`.

---

## 3. Pré-validation (à faire AVANT déploiement)

```bash
# 1. Mesurer le baseline actuel sur 24h
watch -n 60 'ps -o rss= -p $(pgrep -f victoria-metrics-prod) | awk "{print \$1/1024 \" Mo\"}"'

# 2. Compter les séries réellement actives
curl -s 'http://localhost:8428/api/v1/status/tsdb' | jq '.data.totalSeries'

# 3. Lister les top requêtes lourdes (si présentes)
curl -s 'http://localhost:8428/api/v1/status/top_queries' | jq

# 4. Vérifier qu'aucun job promscrape n'a été ajouté entre-temps
grep -A2 scrape_configs /etc/victoriametrics/scrape.yml 2>/dev/null || echo "pas de fichier scrape (OK)"
```

Si `totalSeries > 1500` ou pic mémoire > 130 Mo observé → augmenter
`maxUniqueTimeseries` et `MemoryMax` en conséquence avant déploiement.

---

## 4. Déploiement

Procédure **identique au workflow standard du projet** (pas de service séparé,
on remplace `victoriametrics.service`) :

```bash
# Depuis Pi5, après git pull
sudo cp contrib/victoriametrics.service /etc/systemd/system/victoriametrics.service
sudo systemctl daemon-reload
sudo systemctl restart victoriametrics

# Vérifications immédiates
sudo systemctl status victoriametrics --no-pager
sudo journalctl -u victoriametrics -n 30 --no-pager
curl -sf http://localhost:8428/health && echo "OK"

# RSS après stabilisation (~2 min)
sleep 120
ps -o pid,comm,rss -p $(pgrep -f victoria-metrics-prod) | awk 'NR>1 {print $3/1024 " Mo"}'
```

> **Pas de migration de données** : `-storageDataPath` reste
> `/mnt/nvme/victoria-metrics`, l'historique 5 ans est conservé tel quel.

---

## 5. Validation 24–72 h

```bash
# RAM en continu
watch -n 30 'ps -o pid,comm,rss,vsz -p $(pgrep -f victoria-metrics-prod)'

# Logs systemd (chercher des "OOMKill" ou "out of memory")
journalctl -u victoriametrics -f

# Vérifier que daly-bms et energy-manager n'ont pas perdu de samples
journalctl -u daly-bms -n 200 | grep -iE 'victoria|metric|push' | tail
journalctl -u energy-manager -n 200 | grep -iE 'victoria|metric|push' | tail

# Tester un range query "5 ans" depuis le dashboard
curl -s "http://localhost:8428/api/v1/query_range?query=bms_pack_voltage&start=$(date -d '5 years ago' +%s)&end=$(date +%s)&step=1d" \
  -o /dev/null -w "HTTP %{http_code} en %{time_total}s\n"
```

### Seuils d'alerte

| Symptôme | Action |
|---|---|
| systemd kill VM (OOM) une fois | Augmenter `MemoryMax` à 180M temporairement, analyser les top_queries |
| HTTP 503 sur `/api/v1/query_range` | Augmenter `search.maxMemoryPerQuery` à 60MB |
| Requête dashboard > 10 s | Pré-agréger via recording rules ou downsampling |
| RSS croît linéairement après 7 j | Bug VM — repasser au service actuel et ouvrir un ticket upstream |

---

## 6. Rollback

Le service est versionné dans git, donc rollback trivial :

```bash
git -C ~/Daly-BMS-Rust checkout HEAD~1 -- contrib/victoriametrics.service
sudo cp contrib/victoriametrics.service /etc/systemd/system/victoriametrics.service
sudo systemctl daemon-reload
sudo systemctl restart victoriametrics
```

---

## 7. Résultats attendus

| Métrique | Baseline mesurée | Cible après tuning |
|---|---|---|
| **RSS au démarrage** | ~120 Mo | 50–70 Mo |
| **RSS sous charge (5 ans range query)** | inconnu | < 110 Mo |
| **OOM possible** | Oui (pas de cap) | Non (`MemoryMax=150M`) |
| **Requêtes longues** | Cap 30 s (déjà actif) | Cap 30 s + 40 Mo/query |
| **Données / rétention** | 5 ans NVMe | **Inchangé** |

---

## 8. Si insuffisant — pistes complémentaires

1. **Recording rules** (`-rule.configPath`) pour pré-agréger les séries
   les plus coûteuses du dashboard `/dashboard/history`.
2. **Downsampling natif** (`-downsampling.period=30d:5m,1y:1h`) — réduit
   la taille disque ET l'usage RAM des requêtes longues.
3. **Désactiver dedup côté push** si déjà fait côté Rust
   (`-dedup.minScrapeInterval=60s` aligné sur `scrape_interval`).
4. Migration éventuelle vers SQLite/Parquet — hors scope, voir
   `PROCEDURES.md` si décidé un jour.

---

## 9. Checklist de décision

- [ ] Baseline RSS confirmée sur 24 h (cf. §3)
- [ ] `totalSeries` < 1500 confirmé
- [ ] Sauvegarde NVMe à jour (`/mnt/nvme/victoria-metrics`)
- [ ] Fenêtre de maintenance prévue (restart VM = ~30 s de coupure ingestion ; les pushers Rust bufferisent)
- [ ] Plan de rollback validé (§6)

> Tant que ces 5 cases ne sont pas cochées, **ne pas déployer**.
