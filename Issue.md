# 1. Stopper le service
```
sudo systemctl stop daly-bms  <br>
``` 

# 2. Sauvegarder l'existant (au cas où)
```
sudo -u dalybms cp -r /var/lib/daly-bms/tsink /var/lib/daly-bms/tsink.backup.$(date +%Y%m%d%H%M)
```

# 3. Recréer un dossier tsink VIDE avec les bons droits
```
sudo rm -rf /var/lib/daly-bms/tsink
sudo -u dalybms mkdir -p /var/lib/daly-bms/tsink
sudo chmod 755 /var/lib/daly-bms/tsink
```

# 4. Redémarrer
```
sudo systemctl start daly-bms 
journalctl -u daly-bms -f --no-pager 
```

Une augmentation progressive du CPU (memory/CPU leak léger) est fréquente sur les services longue durée avec ingestion continue. Voici comment **diagnostiquer, stabiliser et surveiller** sans toucher au code.

```
wget -O /tmp/netdata-kickstart.sh https://my-netdata.io/kickstart.sh
sh /tmp/netdata-kickstart.sh --stable-channel --dont-wait

sudo apt install sysstat
pidstat -t -p $(pgrep daly-bms-server) 1

```
port 19999

## 🔍 1. Diagnostic immédiat (3 commandes)

```bash
# 1. Quel thread consomme ? (souvent compaction, flush ou tokio runtime)
top -H -p $(pgrep daly-bms-server)

# 2. Fuite de descripteurs de fichiers ? (doit rester < 200)
ls /proc/$(pgrep daly-bms-server)/fd 2>/dev/null | wc -l

# 3. RAM augmente-t-elle aussi ? (signe de cache non vidé)
cat /proc/$(pgrep daly-bms-server)/status | grep -E "VmRSS|VmSize"
```

📌 **Interprétation rapide** :
- Si `VmRSS` monte avec le CPU → cache/mémoire non libérée
- Si FD > 500 → fuite de connexions MQTT ou HTTP
- Si un thread `tokio-runtime` ou `flush` domine → activité background tsink

---

## 📉 2. Causes probables & correctifs rapides

| Cause | Vérification | Solution |
|-------|-------------|----------|
| 📝 **Logs en DEBUG/TRACE** | `grep RUST_LOG /etc/systemd/system/daly-bms.service` | Passer à `INFO` ou `WARN` |
| 🌐 **Dashboard poll trop agressif** | Regarder les accès `/api/v1/query` dans les logs | Espacer les refreshs (>10s) ou ajouter un cache HTTP |
| 🗜️ **Compaction tsink trop active** | `ls -1 /var/lib/daly-bms/tsink/lane_numeric/segments \| wc -l` | Limiter à 1 thread, augmenter l'intervalle |
| 📦 **WAL qui grossit** | `du -sh /var/lib/daly-bms/tsink/wal` | Ajuster `flush_interval` |
|  **Tâches tokio orphelines** | `top -H` montre plusieurs threads bloqués | Redémarrage périodique (cron) en attendant un fix code |

---

## ⚙️ 3. Réglages stabilisateurs (`/etc/daly-bms/Config.toml`)

Ajoute/ajuste ces paramètres dans ta section `[tsink]` :

```toml
[tsink]
enabled = true
data_path = "/var/lib/daly-bms/tsink"
retention_days = 30

# Stabilisation CPU/RAM
flush_interval_secs = 15        # Écriture disque moins fréquente (défaut souvent 5s)
max_compaction_threads = 1      # Évite les pics CPU background
memory_limit_mb = 1024          # Force la libération mémoire si nécessaire
wal_max_size_mb = 64            # Limite la taille du WAL avant flush forcé
```

Puis redémarre :
```bash
sudo systemctl restart daly-bms
```

---

## 📊 4. Surveillance automatique (pour valider la tendance)

Crée un script léger qui loggue le CPU/RAM toutes les 5 minutes :

```bash
# /usr/local/bin/monitor-daly-bms.sh
#!/bin/bash
LOG="/var/log/daly-bms-usage.log"
PID=$(pgrep daly-bms-server)
if [ -n "$PID" ]; then
  CPU=$(ps -p $PID -o %cpu= | xargs)
  MEM=$(ps -p $PID -o %mem= | xargs)
  echo "$(date '+%Y-%m-%d %H:%M') CPU=${CPU}% MEM=${MEM}%" >> $LOG
fi
```

Active-le via cron :
```bash
chmod +x /usr/local/bin/monitor-daly-bms.sh
echo "*/5 * * * * /usr/local/bin/monitor-daly-bms.sh" | sudo crontab -
```

Visualise la tendance :
```bash
tail -20 /var/log/daly-bms-usage.log
```

---

## ✅ 5. Quand s'inquiéter ?

| CPU | Action |
|-----|--------|
| **< 8%** | ✅ Normal pour 500 pts/s + compaction background |
| **8% → 15%** (sur 24h) | ⚠️ Vérifier `flush_interval` et logs dashboard |
| **> 20%** ou **RAM > 60%** | 🔴 Redémarrage auto + investigation `perf` |
| **FD > 500** ou **fuite RAM** | 🛑 Restart toutes les 6h (cron) en attendant un patch |

---

## 🛠️ Fallback : Redémarrage périodique (si fuite connue)

Si le CPU continue de monter doucement sur plusieurs jours, ajoute un restart propre toutes les 24h (sans perte de données MQTT) :

```bash
# Dans crontab root
0 3 * * * systemctl restart daly-bms
```

---
Netdata inclut nativement son interface web dans le même paquet. Il n'existe donc pas de composant "netdata web" à désinstaller séparément : la suppression du paquet ou du service retire à la fois l'agent et le tableau de bord web.

La méthode dépend de la façon dont vous l'avez installé :

### 🔹 1. Script d'installation officiel (recommandé & le plus courant)
```bash
sudo /usr/libexec/netdata/netdata-uninstaller.sh --yes
```
Si le script est introuvable, téléchargez-le directement :
```bash
curl -Ss 'https://raw.githubusercontent.com/netdata/netdata/master/packaging/installer/netdata-uninstaller.sh' -o /tmp/netdata-uninstaller.sh
chmod +x /tmp/netdata-uninstaller.sh
sudo /tmp/netdata-uninstaller.sh --yes
```

### 🔹 2. Gestionnaire de paquets système
- **Debian / Ubuntu** : `sudo apt remove --purge netdata`
- **RHEL / CentOS / Fedora / Alma / Rocky** : `sudo dnf remove netdata` (ou `yum` sur les anciennes versions)
- **Arch Linux / Manjaro** : `sudo pacman -Rns netdata`
- **OpenSUSE** : `sudo zypper remove netdata`

### 🔹 3. Installation via Docker
```bash
docker stop netdata
docker rm netdata
docker rmi netdata/netdata
```

### 🗑️ Nettoyage des fichiers résiduels (configs, données, logs)
```bash
sudo rm -rf /etc/netdata /var/lib/netdata /var/log/netdata /var/cache/netdata
sudo userdel netdata 2>/dev/null || true
sudo groupdel netdata 2>/dev/null || true
```

### ⚠️ Avant de procéder
- La désinstallation **supprime définitivement** toutes vos métriques historiques, configurations personnalisées et alertes.
- Si vous souhaitez les conserver, faites une sauvegarde rapide :
  ```bash
  sudo tar czf ~/netdata-backup.tar.gz /etc/netdata /var/lib/netdata
  ```

📖 Documentation officielle à jour : https://learn.netdata.cloud/docs/agent/packaging/installer#uninstalling-netdata

Précisez votre OS ou votre méthode d'installation si vous souhaitez une commande adaptée à votre environnement.
