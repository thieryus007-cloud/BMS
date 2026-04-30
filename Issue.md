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

Je te dirai exactement quoi corriger. 🛠️
