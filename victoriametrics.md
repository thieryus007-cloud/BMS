**fichiers de configuration** et **répertoires principaux**  VictoriaMetrics sur le Raspberry Pi 5.

---

## 📁 Fichiers de configuration

| Fichier | Chemin | Rôle |
|---------|--------|------|
| **Service systemd** | `/etc/systemd/system/victoriametrics.service` | Définit comment démarrer/stopper le service, l'utilisateur, les arguments de démarrage |
| **Fichier source service** | `contrib/victoriametrics.service` | Version de référence dans le dépôt — à déployer via `sudo cp` |

**Contenu actuel du service** (`contrib/victoriametrics.service` → `/etc/systemd/system/victoriametrics.service`) :
```ini
[Unit]
Description=VictoriaMetrics Time Series Database
After=network-online.target mnt-nvme.mount
Requires=mnt-nvme.mount

[Service]
Type=simple
User=victoriametrics
Group=victoriametrics
ExecStart=/usr/local/bin/victoria-metrics-prod \
  -storageDataPath=/mnt/nvme/victoria-metrics \
  -retentionPeriod=5y \
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
| **Données TSDB** | `/mnt/nvme/victoria-metrics/` | Toutes les données temporelles (chunks, index, métadonnées) — sur NVMe 256 Go |
| **Logs systemd** | `journalctl -u victoriametrics` | Logs en mémoire/journal systemd (pas de fichier log dédié) |

> **Disque NVMe** : `/dev/nvme0n1p1` (238 Go) monté sur `/mnt/nvme`.
> Rétention configurée à **5 ans** — capacité largement suffisante (~1–5 Mo/jour pour ~200 séries).

---

## 🚀 Première installation / migration depuis la SD

Si VictoriaMetrics tournait sur la carte SD (`/var/lib/victoria-metrics`), utiliser le script de migration :

```bash
# Depuis ~/Daly-BMS-Rust sur le Pi5
sudo bash scripts/migrate-vm-to-nvme.sh
```

Le script :
1. Vérifie que `/mnt/nvme` est monté
2. Arrête le service VictoriaMetrics
3. Copie les données existantes (`rsync`) vers `/mnt/nvme/victoria-metrics/`
4. Vérifie l'intégrité (comptage fichiers)
5. Déploie `contrib/victoriametrics.service` vers `/etc/systemd/system/`
6. Redémarre le service et vérifie l'API

Après migration, supprimer l'ancienne donnée SD si tout est OK :
```bash
sudo rm -rf /var/lib/victoria-metrics
```

---

## 🔍 Exploration des données

```bash
# Voir la structure du stockage
sudo ls -la /mnt/nvme/victoria-metrics/

# Taille totale des données
sudo du -sh /mnt/nvme/victoria-metrics/

# Détail par sous-répertoire
sudo du -h /mnt/nvme/victoria-metrics/ | sort -h

# Voir les logs
sudo journalctl -u victoriametrics -n 50

# Voir la config du service
sudo systemctl cat victoriametrics

# Vérification santé API
curl -sf http://localhost:8428/-/healthy
```

---

## 📊 Structure interne de `/mnt/nvme/victoria-metrics/`

Après quelques heures/jours d'utilisation, vous verrez :

```
/mnt/nvme/victoria-metrics/
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

Pour changer un paramètre (ex: passer à 5 ans de retention) :

```bash
# 1. Éditer le fichier source dans le dépôt
#    contrib/victoriametrics.service → -retentionPeriod=5y

# 2. Déployer sur le Pi5
sudo cp contrib/victoriametrics.service /etc/systemd/system/victoriametrics.service

# 3. Recharger et redémarrer
sudo systemctl daemon-reload
sudo systemctl restart victoriametrics
```

---

## 💾 Vérification du montage NVMe au démarrage

Le service systemd dépend de `mnt-nvme.mount`. Vérifier que le NVMe est bien dans `/etc/fstab` :

```bash
# Vérifier le montage automatique
grep nvme /etc/fstab

# Format attendu (UUID recommandé)
# UUID=<uuid>  /mnt/nvme  ext4  defaults,noatime  0  2

# Obtenir l'UUID du NVMe
sudo blkid /dev/nvme0n1p1

# Tester le montage fstab sans reboot
sudo mount -a
```

---

## 📝 Récap visuel

```
/etc/systemd/system/victoriametrics.service  ←  CONFIGURATION (arguments)
  (source: contrib/victoriametrics.service dans le dépôt)
/usr/local/bin/victoria-metrics-prod          ←  BINAIRE
/mnt/nvme/victoria-metrics/                   ←  DONNÉES TSDB (NVMe 256 Go, rétention 2 ans)
journalctl -u victoriametrics                 ←  LOGS
```

VictoriaMetrics suit une philosophie **"binaire unique + flags CLI"** — pas de fichiers de config complexes, ce qui le rend très adapté à un environnement léger comme Pi 5.
