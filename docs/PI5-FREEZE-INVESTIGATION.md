# Pi5 — Guide d'investigation : freezes et instabilités

> Ce document décrit comment exploiter les trois couches de monitoring installées
> pour diagnostiquer les redémarrages non planifiés ou les freezes du Pi5.

---

## 1. Architecture du monitoring installé

```
┌─────────────────────────────────────────────────────────────────┐
│  Layer 1 — systemd Watchdog (sd-notify)                        │
│    daly-bms-server  → READY=1 au démarrage, WATCHDOG=1 /30s   │
│    energy-manager   → READY=1 au démarrage, WATCHDOG=1 /30s   │
│    Effet : systemd redémarre le service si silence > 60s       │
├─────────────────────────────────────────────────────────────────┤
│  Layer 2 — node_exporter → VictoriaMetrics (:9100 scrape)      │
│    Métriques OS complètes : CPU/core, RAM, I/O disque,         │
│    réseau, filesystem, température, services systemd           │
├─────────────────────────────────────────────────────────────────┤
│  Layer 3 — Métriques applicatives → VictoriaMetrics (:8428)    │
│    daly-bms-server : pi5_cpu_percent, pi5_mem_used_mb,         │
│      pi5_cpu_temp_c, pi5_process_cpu_percent{process},         │
│      tokio_task_polls_total, tokio_task_mean_poll_us           │
│    energy-manager  : em_cpu_percent, em_mem_used_mb,           │
│      em_process_cpu_percent, em_tokio_task_polls_total …       │
└─────────────────────────────────────────────────────────────────┘
```

---

## 2. Premier réflexe : la machine répond-elle ?

### Le Pi5 répond encore en SSH

→ Ce n'est pas un freeze OS, c'est un service bloqué ou surchargé.

```bash
# Quelle est la charge ?
uptime
top -b -n 1 | head -20

# Quel service consomme ?
ps aux --sort=-%cpu | head -15
ps aux --sort=-%mem | head -10

# État des services
systemctl status daly-bms energy-manager mosquitto node-exporter

# Logs récents
journalctl -u daly-bms -n 50 --no-pager
journalctl -u energy-manager -n 50 --no-pager

# Mémoire
free -h
cat /proc/meminfo | grep -E "MemTotal|MemAvailable|SwapTotal|SwapFree"

# Température (throttling ?)
cat /sys/class/thermal/thermal_zone0/temp
# Diviser par 1000 → °C. > 80°C = throttling probable

# I/O disque (carte SD saturée ?)
iostat -x 1 5   # si sysstat installé
# OU :
cat /proc/diskstats
```

### Le Pi5 ne répond plus du tout (SSH timeout)

→ Freeze OS, OOM killer, ou panique noyau. Redémarrer et analyser les logs post-mortem (voir §5).

---

## 3. Requêtes VictoriaMetrics pour analyse historique

> Endpoint : `http://192.168.1.141:8428`
> Interface web : `http://192.168.1.141:8428/vmui`
>
> Voir aussi `docs/victoriametrics-queries.md` pour la syntaxe PromQL complète.

### 3.1 Métriques Layer 3 (applicatives — toutes les 30s)

```promql
# CPU Pi5 — tendance avant le freeze
pi5_cpu_percent[6h]

# Température CPU — throttling ?
pi5_cpu_temp_c[6h]

# Mémoire utilisée
pi5_mem_used_mb[6h]

# Swap (si swap > 0 → OOM imminent)
pi5_swap_used_mb[6h]

# Charge système (load average 1 min)
pi5_load_avg{window="1m"}[6h]

# I/O réseau
pi5_net_rx_bps[6h]
pi5_net_tx_bps[6h]

# Top processus — lequel consommait du CPU ?
pi5_process_cpu_percent[6h]

# Même chose côté energy-manager
em_cpu_percent[6h]
em_mem_used_mb[6h]
em_process_cpu_percent[6h]
```

### 3.2 Santé tokio (détecte deadlocks async)

```promql
# Nombre de polls — si flat → la tâche est bloquée
tokio_task_polls_total{task="daly_bms_monitor"}[6h]
tokio_task_polls_total{task="daly_bms_watchdog"}[6h]
em_tokio_task_polls_total[6h]

# Latence moyenne de poll — si spike → tâche lente à rendre la main
tokio_task_mean_poll_us{task="daly_bms_monitor"}[6h]

# Latence de scheduling — si spike → runtime tokio surchargé
tokio_task_mean_scheduled_us{task="daly_bms_monitor"}[6h]
```

### 3.3 Métriques Layer 2 (node_exporter — si installé)

```promql
# CPU par core (identifie si un seul core sature)
rate(node_cpu_seconds_total{mode!="idle"}[5m])

# I/O disque — % de temps en I/O (carte SD !)
rate(node_disk_io_time_seconds_total[5m]) * 100

# Mémoire disponible
node_memory_MemAvailable_bytes / 1024 / 1024

# OOM kills
node_vmstat_oom_kill[6h]

# Température via hwmon
node_thermal_zone_temp[6h]

# Services systemd down
node_systemd_unit_state{state="failed"}

# Interruptions (anomalie IRQ)
rate(node_intr_total[1m])[6h]
```

---

## 4. Arbre de décision — identifier la cause

```
Le Pi5 était-il accessible en SSH au moment du problème ?
│
├── NON → Freeze OS complet
│   ├── Vérifier OOM killer (§5.1)
│   ├── Vérifier kernel panic (§5.2)
│   └── Vérifier surchauffe (§5.3)
│
└── OUI → Service bloqué ou surchargé
    │
    ├── CPU > 90% avant le freeze ?
    │   ├── OUI → Quel processus ? pi5_process_cpu_percent dans VM
    │   │         → Leak CPU dans daly-bms-server ou energy-manager ?
    │   │         → Voir tokio_task_mean_poll_us (tâche bloquante ?)
    │   └── NON → Ce n'est pas du CPU
    │
    ├── RAM available → 0 avant le freeze ?
    │   ├── OUI → OOM (pi5_mem_used_mb ou node_memory_MemAvailable_bytes)
    │   │         → energy-manager a MemoryMax=100M → qui consomme ?
    │   └── NON → Ce n'est pas la RAM
    │
    ├── I/O disque élevé ?
    │   ├── OUI → Carte SD saturée ou en train de mourir
    │   │         → node_disk_io_time_seconds_total > 80%
    │   │         → Vérifier dmesg pour erreurs MMC
    │   └── NON → Ce n'est pas le disque
    │
    ├── Température > 80°C ?
    │   ├── OUI → Throttling CPU → performances dégradées → timeout RS485
    │   └── NON → Ce n'est pas la chaleur
    │
    └── tokio_task_polls_total flat (ne progresse plus) ?
        ├── OUI → Deadlock async (§6)
        └── NON → Problème réseau ou MQTT (§7)
```

---

## 5. Analyse post-mortem (après redémarrage)

### 5.1 OOM Killer

```bash
# Le kernel a-t-il tué un processus par manque de mémoire ?
journalctl -k --since "2 hours ago" | grep -i "oom\|out of memory\|killed process"
dmesg | grep -i "oom\|out of memory\|killed"
```

**Signes** : `oom_kill_process`, `Memory cgroup out of memory`, `Killed process XXXX`.

**Action** : Identifier le processus tué. Si c'est `daly-bms-server` ou `energy-manager`,
chercher un memory leak via `pi5_mem_used_mb` dans VM sur les 24h précédentes.

### 5.2 Kernel panic / crash

```bash
# Crash kernel enregistré ?
journalctl -k -b -1 | head -50       # boot précédent
journalctl --list-boots               # liste des boots
last reboot                           # historique des redémarrages

# Raison du dernier arrêt
who -b
```

**Signes** : boot précédent se termine abruptement sans "shutdown" dans les logs.

### 5.3 Surchauffe

```bash
# Température actuelle
vcgencmd measure_temp 2>/dev/null || cat /sys/class/thermal/thermal_zone0/temp

# Throttling actuel (Pi5)
vcgencmd get_throttled
# 0x0 = OK, 0x50005 = undervoltage + throttled
```

**Requête VM** :
```promql
# Pic de température avant le freeze
pi5_cpu_temp_c[24h]
```

**Action** : Si temp > 80°C → améliorer refroidissement ou réduire charge.

### 5.4 Watchdog systemd

```bash
# Le service a-t-il été tué par le watchdog ?
journalctl -u daly-bms --since "2 hours ago" | grep -i "watchdog\|timeout\|killed"
journalctl -u energy-manager --since "2 hours ago" | grep -i "watchdog\|timeout\|killed"

# Message typique en cas de watchdog kill :
# "Watchdog timeout (limit 60s), killing"
```

Si le watchdog a tué le service → la tâche tokio ne rendait plus la main (deadlock async).
Chercher `tokio_task_mean_poll_us` élevé juste avant dans VM.

---

## 6. Deadlock async (tokio bloqué)

Le service répond encore mais ne traite plus les données RS485 ou MQTT.

**Détection** :
```promql
# Si poll_count ne monte plus → tâche gelée
increase(tokio_task_polls_total{task="daly_bms_monitor"}[5m])
# Résultat attendu : ~10 (1 poll/30s × 5min)
# Résultat anormal : 0
```

**Causes fréquentes** :
1. `tokio::time::sleep` avec durée très longue dans une tâche critique
2. `Mutex::lock()` bloquant dans une tâche async (utiliser `tokio::sync::Mutex`)
3. Requête HTTP (reqwest) sans timeout → bloque le worker thread
4. Port série RS485 bloqué en lecture sans timeout

**Investigation** :
```bash
# Threads bloqués du processus (Linux)
strace -p $(pgrep daly-bms-server) -e trace=futex 2>&1 | head -30

# Ou plus simple : regarder l'état des threads
ls /proc/$(pgrep daly-bms-server)/task/
cat /proc/$(pgrep daly-bms-server)/status | grep -E "Threads|VmRSS"
```

---

## 7. Problèmes réseau / MQTT

Le service tourne mais ne communique plus.

```bash
# Mosquitto tourne ?
systemctl status mosquitto
nc -zv 127.0.0.1 1883

# NanoPi (Venus) accessible ?
ping -c 3 192.168.1.120
nc -zv 192.168.1.120 1883

# Connexions MQTT actives
ss -tnp | grep 1883

# Logs MQTT energy-manager
journalctl -u energy-manager -n 100 | grep -i "mqtt\|connect\|disconnect\|error"
```

---

## 8. Vérification complète post-déploiement

Après chaque `make build-arm` / déploiement binaire, vérifier :

```bash
# Services démarrés et watchdog actif
systemctl status daly-bms energy-manager node-exporter
journalctl -u daly-bms -n 20 --no-pager
journalctl -u energy-manager -n 20 --no-pager

# Chercher dans les logs :
# daly-bms-server  → "DalyBMS Server démarrage" + "Agent de monitoring Pi5 démarré"
# energy-manager   → "energy-manager fully started"

# VictoriaMetrics reçoit les métriques ? (attendre 60s après démarrage)
curl -s "http://localhost:8428/api/v1/query?query=pi5_cpu_percent" | python3 -m json.tool | head -20
curl -s "http://localhost:8428/api/v1/query?query=em_cpu_percent" | python3 -m json.tool | head -20

# node_exporter répond ?
curl -s http://localhost:9100/metrics | grep "node_cpu_seconds_total" | head -3
```

---

## 9. Tableau des métriques clés

| Métrique | Source | Seuil d'alerte | Signification |
|----------|--------|---------------|---------------|
| `pi5_cpu_percent` | Layer 3 | > 85% soutenu | CPU saturé |
| `pi5_cpu_temp_c` | Layer 3 | > 80°C | Throttling imminent |
| `pi5_mem_used_mb` | Layer 3 | croissance continue | Memory leak |
| `pi5_swap_used_mb` | Layer 3 | > 0 | OOM imminent |
| `pi5_disk_percent` | Layer 3 | > 90% | Disque plein |
| `pi5_load_avg{window="1m"}` | Layer 3 | > nb_cores (4) | Surcharge |
| `tokio_task_mean_poll_us` | Layer 3 | > 100 000 µs | Tâche bloquante (100ms) |
| `tokio_task_mean_scheduled_us` | Layer 3 | > 50 000 µs | Runtime surchargé |
| `node_disk_io_time_seconds_total` | Layer 2 | rate > 0.8 | Carte SD saturée |
| `node_memory_MemAvailable_bytes` | Layer 2 | < 50MB | OOM imminent |
| `node_vmstat_oom_kill` | Layer 2 | > 0 | OOM killer déclenché |
| `node_thermal_zone_temp` | Layer 2 | > 80000 (milli°C) | Surchauffe |

---

## 10. Maintenance préventive

```bash
# Vérifier l'état de la carte SD (erreurs MMC)
dmesg | grep -i "mmc\|mmcblk\|error" | tail -20

# Espace disque
df -h /

# Logs journald (taille)
journalctl --disk-usage

# Limiter la taille si nécessaire (dans /etc/systemd/journald.conf)
# SystemMaxUse=200M

# Surveiller la croissance mémoire sur 24h
# (requête VM)
# increase(pi5_mem_used_mb[24h])   → doit être proche de 0
```

---

## 11. Fichiers et commandes de référence

| Besoin | Commande / Fichier |
|--------|-------------------|
| Logs daly-bms live | `journalctl -u daly-bms -f` |
| Logs energy-manager live | `journalctl -u energy-manager -f` |
| Métriques OS brutes | `http://192.168.1.141:9100/metrics` |
| Interface VM | `http://192.168.1.141:8428/vmui` |
| Relancer watchdog | `sudo systemctl restart daly-bms` |
| Forcer reload services | `sudo systemctl daemon-reload` |
| Config scrape VM | `/etc/victoriametrics/scrape.yml` |
| Service node_exporter | `contrib/node-exporter.service` |
| Ce guide | `docs/PI5-FREEZE-INVESTIGATION.md` |
| Requêtes PromQL | `docs/victoriametrics-queries.md` |
| Procédures générales | `PROCEDURES.md` |
