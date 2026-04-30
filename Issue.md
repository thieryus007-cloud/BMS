# 🔍 Debug tsink "IO error, no such file" — Guide simple Pi5

C'est un problème classique : le dossier existe, mais **tsink ne trouve pas ce qu'il cherche à l'intérieur** (ou ne peut pas y écrire).

Voici les **5 vérifications rapides**, dans l'ordre :

---

## 1️⃣ Voir l'erreur exacte dans les logs
```bash
journalctl -u daly-bms -n 20 --no-pager | grep -i "tsink\|io\|error"
```
🔎 **Cherche** : le chemin complet que tsink essaie d'ouvrir (ex: `/var/lib/daly-bms/tsink/wal/000001.log`).

---

## 2️⃣ Vérifier les permissions réelles (pas juste le owner)
```bash
ls -la /var/lib/daly-bms/
ls -la /var/lib/daly-bms/tsink/
```
✅ **Doit afficher** :
```
drwxr-xr-x 3 dalybms dalybms 4096 ... tsink/
```
❌ **Problème si** : `dalybms` n'a pas `rwx` sur **tous les parents** (`/var`, `/var/lib`, `/var/lib/daly-bms`).

🔧 **Fix rapide** :
```bash
sudo chown -R dalybms:dalybms /var/lib/daly-bms/tsink
sudo chmod 755 /var/lib/daly-bms /var/lib/daly-bms/tsink
```

---

## 3️⃣ Tester l'écriture manuellement (en tant que dalybms)
```bash
sudo -u dalybms touch /var/lib/daly-bms/tsink/test_write.tmp && \
  sudo -u dalybms rm /var/lib/daly-bms/tsink/test_write.tmp && \
  echo "✓ Écriture OK" || echo "❌ Écriture échouée"
```
❌ **Si échec** : problème de permissions ou de filesystem (SD card en read-only ?).

---

## 4️⃣ Vérifier le chemin dans Config.toml
```bash
grep -i "data_path\|tsink" /etc/daly-bms/Config.toml
```
✅ **Doit correspondre exactement** à : `/var/lib/daly-bms/tsink`  
⚠️ **Attention** : pas de `/` final, pas de variable non résolue, pas de chemin relatif.

---

## 5️⃣ Lancer le binaire "à la main" pour voir la vraie erreur
```bash
# Stopper le service d'abord
sudo systemctl stop daly-bms

# Lancer manuellement en tant que dalybms avec logs détaillés
sudo -u dalybms RUST_LOG=debug /usr/local/bin/daly-bms-server --config /etc/daly-bms/Config.toml
```
🔎 **Regarde la première erreur** : souvent plus précise que via systemd.

---

## 🚨 Cas fréquents et solutions

| Symptôme | Cause probable | Solution |
|----------|---------------|----------|
| `No such file or directory` sur `wal/` | tsink veut créer un sous-dossier mais ne peut pas | `sudo mkdir -p /var/lib/daly-bms/tsink/wal && sudo chown -R dalybms:dalybms /var/lib/daly-bms/tsink` |
| `Permission denied` | Parent directory non accessible | `sudo chmod 755 /var/lib/daly-bms` |
| `Read-only file system` | SD card corrompue ou mount en ro | `mount | grep " / "` → vérifier `rw` ; sinon `fsck` ou changer de SD |
| `IO error: Is a directory` | Config pointe vers un fichier au lieu d'un dossier | Vérifier `data_path` dans Config.toml |
| `No space left on device` | SD card pleine | `df -h /` → libérer de l'espace |

---

## 🧹 Reset propre (si rien ne marche)

⚠️ **Attention** : ça efface l'historique tsink local.

```bash
sudo systemctl stop daly-bms
sudo rm -rf /var/lib/daly-bms/tsink/*
sudo -u dalybms mkdir -p /var/lib/daly-bms/tsink
sudo systemctl start daly-bms
journalctl -u daly-bms -f --no-pager
```

---

## 📤 Si tu veux que je regarde plus précisément

Copie-colle ici la sortie de :
```bash
# 1. L'erreur complète
journalctl -u daly-bms -n 30 --no-pager

# 2. Les permissions
ls -laR /var/lib/daly-bms/tsink/ | head -20

# 3. Le chemin dans la config
grep -A2 -B2 "tsink\|data_path" /etc/daly-bms/Config.toml
```

pi5compute@pi5compute:~/Daly-BMS-Rust $ ls -la /var/lib/daly-bms/
ls -la /var/lib/daly-bms/tsink/
total 12
drwxr-xr-x  3 dalybms dalybms 4096 Apr 28 19:23 .
drwxr-xr-x 30 root    root    4096 Apr 28 19:23 ..
drwxr-xr-x  4 dalybms dalybms 4096 Apr 30 09:39 tsink
total 8588
drwxr-xr-x 4 dalybms dalybms    4096 Apr 30 09:39 .
drwxr-xr-x 3 dalybms dalybms    4096 Apr 28 19:23 ..
drwxr-xr-x 4 dalybms dalybms    4096 Apr 28 21:24 lane_numeric
-rw-r--r-- 1 dalybms dalybms    1038 Apr 30 09:18 series_index.bin
-rw-r--r-- 1 dalybms dalybms 8773323 Apr 30 09:39 series_index.catalog.json
-rw-r--r-- 1 dalybms dalybms       0 Apr 28 21:23 .tsink.lock
drwxr-xr-x 2 dalybms dalybms    4096 Apr 30 09:17 wal

pi5compute@pi5compute:~/Daly-BMS-Rust $ ls -laR /var/lib/daly-bms/tsink/ | head -20
/var/lib/daly-bms/tsink/:
total 8588
drwxr-xr-x 4 dalybms dalybms    4096 Apr 30 09:39 .
drwxr-xr-x 3 dalybms dalybms    4096 Apr 28 19:23 ..
drwxr-xr-x 4 dalybms dalybms    4096 Apr 28 21:24 lane_numeric
-rw-r--r-- 1 dalybms dalybms    1038 Apr 30 09:18 series_index.bin
-rw-r--r-- 1 dalybms dalybms 8773323 Apr 30 09:39 series_index.catalog.json
-rw-r--r-- 1 dalybms dalybms       0 Apr 28 21:23 .tsink.lock
drwxr-xr-x 2 dalybms dalybms    4096 Apr 30 09:17 wal

/var/lib/daly-bms/tsink/lane_numeric:
total 16
drwxr-xr-x 4 dalybms dalybms 4096 Apr 28 21:24 .
drwxr-xr-x 4 dalybms dalybms 4096 Apr 30 09:39 ..
drwxr-xr-x 2 dalybms dalybms 4096 Apr 30 09:39 .compaction-replacements
drwxr-xr-x 5 dalybms dalybms 4096 Apr 29 15:34 segments

/var/lib/daly-bms/tsink/lane_numeric/.compaction-replacements:
total 8
drwxr-xr-x 2 dalybms dalybms 4096 Apr 30 09:39 .

pi5compute@pi5compute:~/Daly-BMS-Rust $ grep -A2 -B2 "tsink\|data_path" /etc/daly-bms/Config.toml
# =============================================================================
# Tsink est embarqué dans le binaire : aucune dépendance externe, aucun Docker.
# Les données sont stockées dans data_path en format compressé (Gorilla + zstd).
#
# Migration progressive :
#   Phase 1 : tsink.enabled = true + influxdb.enabled = true  (dual-write)
#   Phase 2 : valider les données Tsink via /api/v1/query
#   Phase 3 : influxdb.enabled = false  (Tsink seul)
[tsink]
enabled = true

# Répertoire de stockage local (relatif au répertoire courant ou chemin absolu)
data_path = "/var/lib/daly-bms/tsink"

# Rétention des données en jours (purge automatique des données anciennes)
pi5compute@pi5compute:~/Daly-BMS-Rust $ sudo -u dalybms touch /var/lib/daly-bms/tsink/test_write.tmp && \
  sudo -u dalybms rm /var/lib/daly-bms/tsink/test_write.tmp && \
  echo "✓ Écriture OK" || echo "❌ Écriture échouée"
✓ Écriture OK
pi5compute@pi5compute:~/Daly-BMS-Rust $

pi5compute@pi5compute:~/Daly-BMS-Rust $ sudo -u dalybms RUST_LOG=debug /usr/local/bin/daly-bms-server --config /etc/daly-bms/Config.toml
Warning: cannot create log dir /var/log/daly-bms: Permission denied (os error 13) — file logging disabled
2026-04-30T07:59:01.164970Z  INFO daly_bms_server: DalyBMS Server démarrage version="0.1.0" mode="HARDWARE" api=0.0.0.0:8080
