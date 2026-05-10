**fichiers de configuration** et **répertoires principaux**  VictoriaMetrics sur le Raspberry Pi 5.

---

## 📁 Fichiers de configuration

| Fichier | Chemin | Rôle |
|---------|--------|------|
| **Service systemd** | `/etc/systemd/system/victoriametrics.service` | Définit comment démarrer/stopper le service, l'utilisateur, les arguments de démarrage |
| *(Ancien fichier supprimé)* | ~~`/etc/victoriametrics/victoriametrics.conf`~~ | ~~Fichier de variables d'environnement (bug, supprimé)~~ |

**Contenu actuel du service** (`/etc/systemd/system/victoriametrics.service`) :
```ini
[Unit]
Description=VictoriaMetrics Time Series Database
After=network-online.target

[Service]
Type=simple
User=victoriametrics
Group=victoriametrics
ExecStart=/usr/local/bin/victoria-metrics-prod \
  -storageDataPath=/var/lib/victoria-metrics \
  -retentionPeriod=30d \
  -httpListenAddr=:8428 \
  -selfScrapeInterval=0 \
  -maxLabelsPerTimeseries=30 \
  -search.maxQueryDuration=30s \
  -search.maxConcurrentRequests=4
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

> **Point clé** : VictoriaMetrics **n'a pas de fichier de configuration YAML/JSON**. Toute la config se fait par **arguments en ligne de commande** dans le service systemd.

---

## 📂 Répertoires principaux

| Répertoire | Chemin | Contenu |
|------------|--------|---------|
| **Binaire** | `/usr/local/bin/victoria-metrics-prod` | Exécutable principal (lien symbolique `victoria-metrics` → `victoria-metrics-prod`) |
| **Données TSDB** | `/var/lib/victoria-metrics/` | Toutes les données temporelles (chunks, index, métadonnées) |
| **Logs systemd** | `journalctl -u victoriametrics` | Logs en mémoire/journal systemd (pas de fichier log dédié) |

---

## 🔍 Exploration des données

```bash
# Voir la structure du stockage
sudo ls -la /var/lib/victoria-metrics/

# Taille totale des données
sudo du -sh /var/lib/victoria-metrics/

# Détail par sous-répertoire
sudo du -h /var/lib/victoria-metrics/ | sort -h

# Voir les logs
sudo journalctl -u victoriametrics -n 50

# Voir la config du service
sudo systemctl cat victoriametrics
```

---

## 📊 Structure interne de `/var/lib/victoria-metrics/`

Après quelques heures/d'jours d'utilisation, vous verrez :

```
/var/lib/victoria-metrics/
├── data/
│   ├── small/
│   │   ├── 2026_04/          → Parties récentes (données fraîches)
│   │   └── 2026_05/
│   └── big/
│       ├── 2026_04/          → Parties compactées (données anciennes)
│       └── 2026_05/
├── indexdb/
│   └── ...                   → Index pour recherche rapide
├── metadata/
│   └── ...                   → Métadonnées des séries temporelles
└── cache/
    └── ...                   → Cache interne
```

---

## 🛠️ Modifier la configuration

Pour changer un paramètre (ex: passer à 90 jours de retention) :

```bash
# 1. Éditer le service
sudo systemctl edit --full victoriametrics

# 2. Modifier la ligne -retentionPeriod=30d → -retentionPeriod=90d

# 3. Recharger et redémarrer
sudo systemctl daemon-reload
sudo systemctl restart victoriametrics
```

---

## 📝 Récap visuel

```
/etc/systemd/system/victoriametrics.service  ←  CONFIGURATION (arguments)
/usr/local/bin/victoria-metrics-prod          ←  BINAIRE
/var/lib/victoria-metrics/                    ←  DONNÉES TSDB
journalctl -u victoriametrics                 ←  LOGS
```

VictoriaMetrics suit une philosophie **"binaire unique + flags CLI"** — pas de fichiers de config complexes, ce qui le rend très adapté à un environnement léger comme votre Pi 5.
