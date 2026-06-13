# Diagnostic et Dépannage — Daly-BMS-Rust

> Document de référence transverse pour le dépannage du système Daly-BMS-Rust.
> Couvre : table maîtresse des problèmes courants, diagnostic port série et permissions,
> diagnostic services systemd, test API/WebSocket, diagnostic réseau (`netdiag`),
> procédure d'investigation Onduleur & SmartShunt, investigation memory-leak (§1–§16),
> nettoyage disque builds cumulés, et récupération Venus/NanoPi.
> Fait partie de l'[architecture documentaire](./ARCHITECTURE.md).
> Dernière consolidation : 2026-06-07.

## Table des matières

- [1. Table maîtresse des problèmes courants](#1-table-maitresse-des-problemes-courants)
- [2. Diagnostic port série et permissions dialout](#2-diagnostic-port-serie-et-permissions-dialout)
- [3. Diagnostic services systemd](#3-diagnostic-services-systemd)
  - [3.1 Healthcheck et statut rapide](#31-healthcheck-et-statut-rapide)
  - [3.2 Logs par service](#32-logs-par-service)
- [4. Test API REST et WebSocket](#4-test-api-rest-et-websocket)
- [5. Diagnostic réseau — netdiag](#5-diagnostic-reseau--netdiag)
  - [5.1 Présentation du script](#51-presentation-du-script)
  - [5.2 Capture immédiate](#52-capture-immediate)
  - [5.3 Mode veille (--watch)](#53-mode-veille---watch)
  - [5.4 Interprétation du rapport](#54-interpretation-du-rapport)
  - [5.5 Exemple de baseline (2026-05-31)](#55-exemple-de-baseline-2026-05-31)
- [6. Procédure debug Onduleur et SmartShunt](#6-procedure-debug-onduleur-et-smartshunt)
  - [6.1 Étape 1 — Vérifier que les services tournent](#61-etape-1--verifier-que-les-services-tournent)
  - [6.2 Étape 2 — Vérifier les logs BMS (erreurs MQTT)](#62-etape-2--verifier-les-logs-bms-erreurs-mqtt)
  - [6.3 Étape 3 — Vérifier le broker MQTT](#63-etape-3--verifier-le-broker-mqtt)
  - [6.4 Étape 4 — Vérifier les topics MQTT (energy-manager publie ?)](#64-etape-4--verifier-les-topics-mqtt-energy-manager-publie-)
  - [6.5 Étape 5 — Vérifier les API endpoints](#65-etape-5--verifier-les-api-endpoints)
  - [6.6 Étape 6 — Vérifier les MQTT handlers (logs debug)](#66-etape-6--verifier-les-mqtt-handlers-logs-debug)
  - [6.7 Étape 7 — Vérifier que le dashboard fetch les endpoints](#67-etape-7--verifier-que-le-dashboard-fetch-les-endpoints)
  - [6.8 Arbre de décision](#68-arbre-de-decision)
  - [6.9 Fixes rapides](#69-fixes-rapides)
  - [6.10 Checklist de vérification](#610-checklist-de-verification)
  - [6.11 Commandes de dépannage rapide](#611-commandes-de-depannage-rapide)
  - [6.12 Rapport à fournir si toujours bloqué](#612-rapport-a-fournir-si-toujours-bloque)
- [7. Nettoyage disque — builds cumulés target/](#7-nettoyage-disque--builds-cumules-target)
- [8. Récupération Venus / NanoPi](#8-recuperation-venus--nanopi)
- [9. Investigation memory-leak daly-bms-server (EN COURS → TERMINÉE)](#9-investigation-memory-leak-daly-bms-server-en-cours--terminee)
  - [§1 Symptôme](#1-symptome)
  - [§2 Hypothèses initiales testées et écartées](#2-hypotheses-initiales-testees-et-ecartees)
  - [§3 État confirmé par les mesures](#3-etat-confirme-par-les-mesures)
  - [§4 Pistes investiguées (non concluantes)](#4-pistes-investigues-non-concluantes)
  - [§5 Pistes restantes à investiguer](#5-pistes-restantes-a-investiguer)
  - [§6 Plan d'action immédiat](#6-plan-daction-immediat)
  - [§7 Outils diagnostiques](#7-outils-diagnostiques)
  - [§8 Apprentissages](#8-apprentissages)
  - [§9 Investigation finale (2026-05-19 après-midi)](#9-investigation-finale-2026-05-19-apres-midi)
  - [§10 Workaround appliqué — RuntimeMaxSec=86400](#10-workaround-applique--runtimemaxsec86400)
  - [§11 Code d'investigation conservé](#11-code-dinvestigation-conserve)
  - [§12 Si on veut REPRENDRE l'investigation](#12-si-on-veut-reprendre-linvestigation)
  - [§13 Phase 3 (2026-05-20) — Cause identifiée : tower-http stack clone](#13-phase-3-2026-05-20--cause-identifiee--tower-http-stack-clone)
  - [§14 Phase B livré (commit b73024f) — tower-http 0.5 → 0.6](#14-phase-b-livre-commit-b73024f--tower-http-05--06)
  - [§15 Status final — investigation close](#15-status-final--investigation-close)
  - [§16 Phase C (commit 018e363) — axum 0.7 → 0.8](#16-phase-c-commit-018e363--axum-07--08)
  - [§17 Phase D (2026-06) — root cause du résiduel : trafic HTTP passif 1 Hz](#17-phase-d-2026-06--root-cause-du-residuel--trafic-http-passif-1-hz)
  - [§18 Phase E (2026-06-13) — la fuite est une VRAIE fuite heap](#18-phase-e-2026-06-13--la-fuite-est-une-vraie-fuite-heap-mesures-terrain)
- [Voir aussi](#voir-aussi)
- [Sources consolidées](#sources-consolidees)

---

## 1. Table maîtresse des problèmes courants

La table ci-dessous est exhaustive — elle reprend l'ensemble des entrées connues pour le projet.
Pour le détail des procédures matérielles spécifiques (registres ATS, mbpoll ET112 adresses Modbus),
voir [`./integration-materiel.md`](./integration-materiel.md).

| Symptôme | Cause probable | Solution |
|----------|----------------|----------|
| `make sync` → "Permission denied" | Propriété du répertoire modifiée (sudo précédent) | `sudo chown -R pi5compute:pi5compute ~/Daly-BMS-Rust/ && git reset --hard origin/<branch>` |
| `deploy-pi5.sh` → `rustup: not found` | PATH root ≠ PATH user sous `sudo` | Corrigé : le script build via `as_user` (sous `$SUDO_USER`). Sinon : builder sans sudo (`make build-arm && make build-energy-arm`) puis `sudo bash scripts/deploy-pi5.sh --no-build`. Dashboards seuls : `sudo bash scripts/fix-grafana.sh`. |
| Service BMS ne démarre pas | Binaire absent, config manquante, port série indisponible | `journalctl -u daly-bms -n 50` — vérifier le message d'erreur exact |
| Config ignorée | Le service lit `/etc/daly-bms/config.toml`, pas `~/Daly-BMS-Rust/Config.toml` | `sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart daly-bms` |
| `scp: dest open Failure` (déploiement NanoPi) | `dbus-mqtt-venus` monopolise le binaire lors de la copie | `ssh root@192.168.1.120 "svc -d /service/dbus-mqtt-venus"` puis redéployer |
| Venus symlink disparu (après mise à jour firmware) | Firmware Venus recrée `/service` à la mise à jour | `ssh root@192.168.1.120 "ln -sf /data/etc/sv/dbus-mqtt-venus /service/dbus-mqtt-venus"` |
| Disque racine Pi5 se remplit (`df -h /` > 45 %) | Builds Rust cumulés dans `target/` (aarch64 + armv7 + natif debug/release) | Voir §7 "Nettoyage disque". Ne **jamais** supprimer `~/.cargo/bin`, `/usr/local/bin/*`, ni `/mnt/nvme/.../metrics.redb`. |
| `dbus-mqtt-venus` crash-loop sur NanoPi (`svstat` uptime=0, tous les services D-Bus absents) | Binaire armv7 mal compilé → **SIGILL** (exit 132). Cause : `target-cpu=native` dans le build armv7 (hôte aarch64 ≠ cible armv7). | Corrigé dans le Makefile. Diag : lancer le binaire à la main sur le NanoPi. Indice au build : warnings `'+lse' is not a recognized feature`. Le build armv7 n'utilise que `-C link-arg=-Wl,--as-needed` (jamais `target-cpu=native`). |
| `install-venus.sh: Permission denied` (`make install-venus-v7`) | Bit `+x` manquant sur le script | `chmod +x nanoPi/install-venus.sh` ou `ARCH=armv7 bash nanoPi/install-venus.sh 192.168.1.120` |
| Compteur grid/acload affiche L2/L3 fantômes à 0 W dans VRM | ET112 monophasé : `grid_service` n'expose que les phases présentes via `/Ac/NumberOfPhases` (dérivé du payload) | Si L2/L3 persistent → rafraîchir VRM (cache console). Comportement normal, pas un bug. |
| ET112 "en attente de données" | Mauvaise adresse Modbus configurée | `sudo systemctl stop daly-bms && mbpoll -m rtu -a 1:15 -b 9600 -t 3:float -r 1 -c 1 /dev/ttyUSB0` (détail → [`./integration-materiel.md`](./integration-materiel.md)) |
| Dashboard Grafana ET112 vide alors que les données existent | **Format du label `address`** : le backend écrit `address="0x07/0x08/0x09"` (hex, `redb_writes.rs::write_et112`). Les requêtes PromQL doivent utiliser `address="0x07"`, **jamais** `address="7"` (décimal → 0 série). | Vérif : `curl -s 'localhost:8080/api/v1/query?query=et112_power_w' \| jq '.data.result[].metric'` |
| Widget météo "Température: -" | Limitation Venus OS — inévitable, non fixable | Aucune ; comportement attendu. |
| `mbpoll` sans réponse | `daly-bms` monopolise le port série | `sudo systemctl stop daly-bms` d'abord, puis relancer mbpoll |
| Dashboard affiche cumul brut (sans baseline) | `pvinv_baseline` retained MQTT absent | Vérifier `santuario/persist/pvinv_baseline` retained dans Mosquitto |
| energy-manager ne démarre pas | TOML manquant, section `[energy_manager]` absente, ou `.env` absent | `journalctl -u energy-manager -n 50` — souvent TOML manquant ou `.env` absent |
| `missing field energy_manager` au démarrage | Section `[energy_manager]` absente de `/etc/daly-bms/config.toml` | `sudo cp Config.toml /etc/daly-bms/config.toml` |
| energy-manager ne reçoit pas MQTT | `portal_id` incorrect dans Config.toml, ou Mosquitto inaccessible | Vérifier `portal_id` dans Config.toml et que Mosquitto est accessible sur `mqtt.host` |
| LG ThinQ ne répond pas | Token ou clé API invalide/expiré | Vérifier `LG_BEARER_TOKEN` et `LG_API_KEY` dans `/etc/daly-bms/.env` |
| Grafana dossier vide "No items" | Dashboards au mauvais format (export vs provisioning) | Les JSON ne doivent pas contenir `__inputs`/`__requires`. Le datasource UID doit être `daly-metrics`. Voir [`./grafana-dashboards.md`](./grafana-dashboards.md). |
| Grafana ne démarre pas | YAML de provisioning invalide | `journalctl -u grafana-server -n 50` — souvent YAML provisioning invalide |
| Grafana "datasource not found" | Fichier datasource absent ou UID incorrect | Vérifier `/etc/grafana/provisioning/datasources/daly-metrics.yaml` présent. Supprimer `victoriametrics.yaml` si résiduel. |
| Grafana ancien dossier "PV Solaire" vide | Dashboards déplacés, dossier fantôme | Supprimer manuellement via UI Grafana : Dashboards → PV Solaire → Delete |
| Port série `/dev/ttyUSB*` absent ou permission refusée | Utilisateur non membre du groupe `dialout` | `ls -l /dev/ttyUSB* && groups $USER` ; si absent : `sudo usermod -aG dialout $USER` (déconnexion/reconnexion nécessaire) |
| Onduleur ou SmartShunt affiche "—" dans le dashboard | Données MQTT non reçues par daly-bms-server | Suivre la procédure §6 "Debug Onduleur & SmartShunt" |
| daly-bms-server mémoire croissante (fuite) | Fuite passive confirmée (voir §9 investigation) | Workaround appliqué : `RuntimeMaxSec=86400` + upgrade tower-http 0.6 + axum 0.8. Pente résiduelle : ~1.18 MB/h. |
| Débit réseau anormal / pic réseau | Client externe multiplexé (Grafana, scraper) ou boucle MQTT | Utiliser `sudo bash scripts/netdiag.sh` (voir §5). |
| `heaptrack -p $PID` crashe le service | Injection GDB → signaux → auto-restart systemd | Ne pas utiliser sur ce binaire ; voir §9.7 pour les alternatives. |

---

## 2. Diagnostic port série et permissions dialout

```bash
# Vérifier la présence du port série et les permissions
ls -l /dev/ttyUSB*

# Vérifier que l'utilisateur courant est dans le groupe dialout
groups $USER

# Ajouter l'utilisateur au groupe (nécessite déconnexion/reconnexion)
sudo usermod -aG dialout $USER
```

Si le port `/dev/ttyUSB0` est absent alors que l'adaptateur USB/RS485 est branché :

```bash
# Vérifier la détection USB
dmesg | tail -20
lsusb
```

Si `daly-bms` monopolise le port et empêche d'autres outils (mbpoll, etc.) d'y accéder :

```bash
# Arrêter le service avant toute opération manuelle sur le bus
sudo systemctl stop daly-bms
# Opération manuelle ...
sudo systemctl start daly-bms
```

Réouverture automatique après débranchement USB : `SharedBus::reopen()` est intégré dans
le service — il rouvre `/dev/ttyUSB0` après déconnexion USB / ré-énumération. `poll_loop`
déclenche la réouverture sur `DalyError::Serial` **et** `DalyError::Io` (backoff + reopen).
Bus partagé → ET112/ATS/PRALRAN repartent aussi. **Plus besoin de redémarrer le service
manuellement après un débranchement USB.**

---

## 3. Diagnostic services systemd

### 3.1 Healthcheck et statut rapide

```bash
# État de tous les services BMS/énergie
systemctl status daly-bms mosquitto-broker energy-manager

# Healthcheck backend daly-bms-server
curl -s http://localhost:8080/-/healthy

# Healthcheck Grafana
curl -s http://localhost:3000/api/health

# Nombre de séries en base redb
curl -s http://localhost:8080/api/v1/redb/series | jq '.data | length'

# Taille de la base redb
du -sh /mnt/nvme/daly-bms/metrics.redb
```

### 3.2 Logs par service

```bash
# daly-bms-server (service principal RS485 + API)
journalctl -u daly-bms -f
journalctl -u daly-bms -n 50 --no-pager

# energy-manager (automatisation énergie)
journalctl -u energy-manager -f
journalctl -u energy-manager -n 50 --no-pager

# Mosquitto (broker MQTT)
journalctl -u mosquitto-broker -f
systemctl status mosquitto-broker

# Grafana
journalctl -u grafana-server -n 50 --no-pager

# Logs depuis une date précise
journalctl -u daly-bms --since "2026-03-17 00:00:00"

# Taille du journal systemd
journalctl --disk-usage

# Limiter la rétention (dans /etc/systemd/journald.conf)
# SystemMaxUse=200M
# MaxRetentionSec=7day
sudo systemctl restart systemd-journald

# Purger manuellement les anciens logs
sudo journalctl --vacuum-time=7d
sudo journalctl --vacuum-size=100M
```

Niveau de logs augmenté (debug) :

```bash
RUST_LOG=debug daly-bms-server
# Ou pour redémarrer le service avec debug :
RUST_LOG=debug systemctl restart daly-bms
```

---

## 4. Test API REST et WebSocket

```bash
# Statut global
curl http://localhost:8080/api/v1/system/status | jq

# Statut BMS (remplacer 1 par l'id souhaité)
curl http://localhost:8080/api/v1/bms/1/status | jq

# Venus — onduleur
curl -s http://localhost:8080/api/v1/venus/inverter | jq '.'

# Venus — smartshunt
curl -s http://localhost:8080/api/v1/venus/smartshunt | jq '.'

# Venus — MPPT
curl -s http://localhost:8080/api/v1/venus/mppt | jq '.'

# Venus — températures
curl -s http://localhost:8080/api/v1/venus/temperatures | jq '.'

# ET112 (vérifier les labels address en hex)
curl -s 'localhost:8080/api/v1/query?query=et112_power_w' | jq '.data.result[].metric'

# Test WebSocket (nécessite wscat : npm install -g wscat)
wscat -c ws://localhost:8080/ws/bms/stream

# Test tous les endpoints Venus en boucle
for ep in inverter smartshunt mppt temperatures; do
  echo "=== $ep ==="
  curl -s http://localhost:8080/api/v1/venus/$ep | jq '.connected'
done
```

Résultat attendu pour un endpoint Venus opérationnel :

```json
{
  "connected": true,
  "inverter": {
    "voltage_v": 48.2,
    "ac_output_power_w": 1286.0
  }
}
```

Si `"connected": false` → les données MQTT n'ont jamais été reçues par l'AppState.
Suivre la procédure §6.

---

## 5. Diagnostic réseau — netdiag

### 5.1 Présentation du script

`scripts/netdiag.sh` capture le responsable d'un pic de trafic réseau sortant sur le Pi5.
Le débit mesuré reproduit **exactement** celui de la page monitoring du serveur
(`pi5_net_tx_bps`) : somme des octets/s TX de toutes les interfaces non-loopback
(`/proc/net/dev`). Aucun outil externe requis (`ss`/`iproute2` + `coreutils` suffisent ;
`iftop`/`nethogs` facultatifs).

Le script mesure en 5 sections :
1. Débit TX global + par interface
2. Top connexions par octets envoyés (socket local|pair|process)
3. Nombre de connexions établies par port local (détecte un afflux de clients WS)
4. Top process CPU (un pic réseau s'accompagne souvent d'un pic CPU)
5. Métriques `$SYS` du broker MQTT Mosquitto (si `mosquitto_sub` disponible)
6. 20 dernières lignes de logs `daly-bms`

### 5.2 Capture immédiate

```bash
# Capture immédiate sur une fenêtre de 10 secondes (défaut)
sudo bash scripts/netdiag.sh

# Capture sur une fenêtre personnalisée (ici 15 secondes)
sudo bash scripts/netdiag.sh --once 15
```

### 5.3 Mode veille (--watch)

```bash
# Veille automatique : capture dès que TX dépasse 150 Ko/s (seuil par défaut), poll 5 s
sudo bash scripts/netdiag.sh --watch

# Paramètres personnalisés : seuil 150 Ko/s, poll toutes les 5 secondes
sudo bash scripts/netdiag.sh --watch 150000 5
```

Comportement du mode `--watch` :
- Lit le débit TX toutes les `poll` secondes (défaut : 5 s)
- Dès que TX ≥ seuil (défaut : 150 000 B/s ≈ 150 Ko/s), déclenche une capture complète
- Écrit le rapport horodaté dans `/tmp/netdiag-<date>.txt`
- Cooldown de 60 secondes avant la prochaine capture possible
- Affiche en continu : `HH:MM:SS  TX=NNNN B/s`

```bash
# Consulter les rapports générés par --watch
ls /tmp/netdiag-*.txt
cat /tmp/netdiag-20260601-143022.txt
```

Constantes du script (dans `scripts/netdiag.sh`) :

| Variable | Valeur | Signification |
|----------|--------|---------------|
| `INTERVAL_DEFAULT` | 10 | Durée de la fenêtre de capture (secondes) |
| `THRESHOLD_DEFAULT` | 150000 | Seuil de déclenchement en mode --watch (B/s) |
| `POLL_DEFAULT` | 5 | Fréquence de poll en mode --watch (secondes) |
| `COOLDOWN` | 60 | Délai minimum entre deux captures consécutives (secondes) |

### 5.4 Interprétation du rapport

Exemple de structure d'un rapport `netdiag` :

```
════════════════════════════════════════════════════════════
 netdiag — capture du 2026-05-31 11:51:40  (fenêtre 10s)
════════════════════════════════════════════════════════════

▶ TX total (toutes ifaces non-lo) : 11010 B/s  (~10 Ko/s, 0.09 Mbit/s)

▶ Débit TX par interface :
  eth0       0 B/s
  wlan0  11206 B/s

▶ Top connexions par octets ENVOYÉS sur la fenêtre :
         B/s  local                  pair                   process
        9516  192.168.1.120:1883     ...                    mosquitto [mosquitto]
        8939  127.0.0.1:1883         ...                    daly-bms-server → mosquitto
        2978  127.0.0.1:45060        ...                    mosquitto
        2640  127.0.0.1:1883         ...                    energy-manager → mosquitto
        1474  127.0.0.1:45080        ...                    mosquitto
         405  192.168.1.141:54878    ...                    daly-bms-server
         314  192.168.1.141:8080     ...                    energy-manager → daly-bms (API/WS)
           3  192.168.1.116:12451    ...                    sshd-session

▶ Connexions établies par port local (count) :
     6  :1883   mosquitto
     4  :8080   daly-bms (API/WS)
     ...

▶ Top process CPU :
  PID   Commande             %CPU  %MEM
  1217  daly-bms-server       1.2   1.9
  1193  grafana               0.4   6.8
  ...

▶ MQTT broker $SYS (1 min) :
  load/bytes/sent/1min       874924.87
  load/messages/sent/1min    4551.56
  clients/connected          6

▶ daly-bms — 20 dernières lignes :
  [logs systemd récents]
════════════════════════════════════════════════════════════
```

**Points d'attention lors de l'interprétation :**

- Un débit élevé sur `192.168.1.120:1883` (NanoPi) = le bridge MQTT Pi5→NanoPi envoie
  beaucoup. Normal si de nombreux capteurs publient. Anormal si la connexion NanoPi est tombée
  (buffer qui se remplit).
- Un grand nombre de connexions sur `:8080` = clients WebSocket multiples ou Grafana
  qui poll intensément. Normal si Grafana est ouvert sur plusieurs onglets.
- Un pic CPU daly-bms-server > 5 % = burst PromQL ou burst RS485 (lecture de nombreuses trames).
- `$SYS load/bytes/sent/1min` > 1 Mo = débit MQTT très élevé ; examiner les topics publiés.

### 5.5 Exemple de baseline (2026-05-31)

Valeurs de référence mesurées en production normale :

| Métrique | Valeur baseline |
|----------|----------------|
| TX total (wlan0) | ~11 Ko/s (0.09 Mbit/s) |
| Connexions Mosquitto (`:1883`) | 6 clients |
| CPU daly-bms-server | ~1.2 % |
| CPU grafana | ~0.4 % |
| CPU energy-manager | ~0.1 % |
| MQTT messages/min | ~4 552 |
| MQTT octets/min | ~875 Ko |

---

## 6. Procédure debug Onduleur et SmartShunt

> Exécuter chaque étape dans l'**ordre exact** sur le Pi5, noter les résultats.
> La procédure s'applique à tout device Venus qui affiche "—" (onduleur Victron,
> SmartShunt, MPPT) — c'est-à-dire que daly-bms-server n'a pas reçu les données
> MQTT attendues.

### 6.1 Étape 1 — Vérifier que les services tournent

```bash
# Sur Pi5 — État des services
systemctl status daly-bms | head -20
systemctl is-active mosquitto-broker
```

**Résultat attendu :**
- `daly-bms` : `active (running)`
- `mosquitto-broker` : `active`

**Si NOK :**
```bash
sudo systemctl restart daly-bms && sudo systemctl restart mosquitto-broker
```

### 6.2 Étape 2 — Vérifier les logs BMS (erreurs MQTT)

```bash
# Sur Pi5 — Logs du serveur BMS (30 dernières lignes)
journalctl -u daly-bms -n 30 --no-pager
```

**Chercher :**
- Erreurs MQTT : `"MQTT connection error"`, `"Failed to subscribe"`
- Erreurs parsing : `"Failed to parse JSON"`
- Messages reçus : `"Updated inverter"`, `"Updated smartshunt"`

**Si erreurs MQTT :** → Vérifier MQTT broker (étape 3)

**Si PAS de messages "Updated" :** → Vérifier les topics MQTT avec `mosquitto_sub` (étape 4)

### 6.3 Étape 3 — Vérifier le broker MQTT

```bash
# Sur Pi5 — Vérifier que Mosquitto écoute
ss -tlnp | grep 1883
mosquitto_sub -h 127.0.0.1 -p 1883 -t '$SYS/#' -C 1
```

**Résultat attendu :**
- Mosquitto écoute sur `0.0.0.0:1883`
- Les topics `$SYS` sont publiés = broker actif

**Si broker down :**
```bash
sudo systemctl restart mosquitto-broker
# Attendre 5 secondes
sleep 5
# Tester à nouveau
mosquitto_sub -h 127.0.0.1 -p 1883 -t '$SYS/#' -C 1
```

### 6.4 Étape 4 — Vérifier les topics MQTT (energy-manager publie ?)

```bash
# Sur Pi5 — Watch tous les topics santuario pendant 30 secondes
echo "Watching MQTT for 30 seconds..."
timeout 30 mosquitto_sub -h 127.0.0.1 -p 1883 -t 'santuario/#' -v 2>&1 | head -100
```

**Résultat attendu :**
```
santuario/inverter/venus {"Voltage": ..., "AcPower": ...}
santuario/system/venus {"Voltage": ..., "Current": ...}
santuario/meteo/venus {"MpptPower": ...}
santuario/bms/1/venus {...}
santuario/bms/2/venus {...}
```

**Si `santuario/inverter/venus` est ABSENT :**
- energy-manager ne publie pas ce topic → vérifier `journalctl -u energy-manager -n 30`

**Si `santuario/system/venus` est ABSENT :**
- Même diagnostic → vérifier energy-manager

### 6.5 Étape 5 — Vérifier les API endpoints

```bash
# Sur Pi5 — Tester les endpoints directement
echo "=== INVERTER ENDPOINT ==="
curl -s http://localhost:8080/api/v1/venus/inverter | jq '.'

echo "=== SMARTSHUNT ENDPOINT ==="
curl -s http://localhost:8080/api/v1/venus/smartshunt | jq '.'

echo "=== MPPT ENDPOINT ==="
curl -s http://localhost:8080/api/v1/venus/mppt | jq '.'
```

**Résultat attendu :**
```json
{
  "connected": true,
  "inverter": {
    "voltage_v": 48.2,
    "ac_output_power_w": 1286.0
  }
}
```

**Si `"connected": false` :**
- Cause : AppState n'a jamais reçu de données MQTT
- Vérifier étape 4 (MQTT topics publiés ?)
- Vérifier logs BMS (étape 2) pour messages "Updated"

**Si API endpoint retourne 404 :**
- Cause : Route non enregistrée dans le router axum
- Vérifier `crates/daly-bms-server/src/api/mod.rs` contient `.route("/api/v1/venus/inverter", ...)`
- Vérifier que la compilation était sans erreurs

### 6.6 Étape 6 — Vérifier les MQTT handlers (logs debug)

```bash
# Activer les logs debug et redémarrer
RUST_LOG=debug systemctl restart daly-bms
sleep 2
journalctl -u daly-bms -f &  # Laisser tourner en arrière-plan

# Dans un autre terminal, déclencher un message MQTT de test :
mosquitto_pub -h 127.0.0.1 -p 1883 -t 'santuario/inverter/venus' \
  -m '{"Voltage": 48.2, "Current": 3.5, "Power": 168.7, "AcVoltage": 229.8, "AcCurrent": 5.6, "AcPower": 1286.0, "State": "on", "Mode": "inverter"}'

# Vérifier dans les logs : doit apparaître "Updated inverter"
```

### 6.7 Étape 7 — Vérifier que le dashboard fetch les endpoints

```bash
# Ouvrir le dashboard dans un navigateur
# URL : http://192.168.1.141:8080/dashboard/visualization
# Ouvrir DevTools (F12) → Console
# Exécuter dans la console JavaScript :
fetch('/api/v1/venus/inverter').then(r => r.json()).then(console.log)
fetch('/api/v1/venus/smartshunt').then(r => r.json()).then(console.log)
```

**Résultat attendu :** affiche les objets JSON avec `connected: true` et les données.

### 6.8 Arbre de décision

```
Onduleur affiche "—" ?
    │
    ├─→ Vérifier étape 4 : Topic 'santuario/inverter/venus' publié ?
    │   │
    │   ├─ NON → energy-manager ne publie pas ce topic
    │   │        ACTION : vérifier journalctl -u energy-manager
    │   │
    │   └─ OUI → Aller à étape 5
    │
    └─→ Vérifier étape 5 : API /venus/inverter retourne connected: true ?
        │
        ├─ NON (connected: false) → MQTT reçu par handler ?
        │   │
        │   └─→ Vérifier logs étape 2 pour "Updated inverter"
        │       │
        │       ├─ NON → MQTT handler pas appelé
        │       │        ACTION : Vérifier bridges/mqtt.rs a le subscribe correct
        │       │
        │       └─ OUI → AppState pas mise à jour
        │                ACTION : Vérifier on_venus_inverter() est appelé
        │
        ├─ OUI (connected: true) → Dashboard ne fetch pas ?
        │                          ACTION : Vérifier étape 7
        │
        └─ API 404 → Route non enregistrée
                     ACTION : Vérifier api/mod.rs
```

### 6.9 Fixes rapides

**FIX — MQTT handler pas enregistré**

Vérifier `crates/daly-bms-server/src/bridges/mqtt.rs` (vers lignes 50–60 et 100–120) :

```rust
// Dans async fn connect_mqtt() :
mqtt_client.subscribe("santuario/inverter/venus", QoS::AtLeastOnce).await?;
mqtt_client.subscribe("santuario/system/venus", QoS::AtLeastOnce).await?;

// Dans le match pattern de réception :
"santuario/inverter/venus" => handle_inverter_topic(&state, &json).await,
"santuario/system/venus" => handle_system_topic(&state, &json).await,
```

Si le code est absent ou incorrect :
```bash
git pull origin main
make build-arm
sudo systemctl restart daly-bms
```

**FIX — API route non enregistrée**

Vérifier `crates/daly-bms-server/src/api/mod.rs` (vers lignes 50–60) :

```rust
.route("/api/v1/venus/inverter", get(system::get_venus_inverter))
.route("/api/v1/venus/smartshunt", get(system::get_venus_smartshunt))
```

Si absent → même procédure que le fix précédent.

**FIX — Serveur BMS pas recompilé avec les changements récents**

```bash
cd ~/Daly-BMS-Rust
git pull origin main
make build-arm
sudo systemctl stop daly-bms
sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/
sudo systemctl start daly-bms
sleep 3
journalctl -u daly-bms -n 10
```

### 6.10 Checklist de vérification

```
□ Services tournent (BMS, Mosquitto, energy-manager)
  Commande : systemctl status daly-bms mosquitto-broker energy-manager

□ MQTT topics publiés
  Commande : mosquitto_sub -t 'santuario/#' -v

□ API endpoints répondent
  Commande : curl http://localhost:8080/api/v1/venus/inverter

□ Dashboard peut fetch l'API
  Console navigateur : fetch('/api/v1/venus/inverter').then(r => r.json()).then(console.log)

□ Logs BMS sans erreurs
  Commande : journalctl -u daly-bms -n 50
```

### 6.11 Commandes de dépannage rapide

```bash
# Redémarrer tout
sudo systemctl restart daly-bms
sudo systemctl restart mosquitto-broker
sleep 5

# Vérifier MQTT
timeout 10 mosquitto_sub -h 127.0.0.1 -t 'santuario/#' -v

# Vérifier API
for ep in inverter smartshunt mppt temperatures; do
  echo "=== $ep ==="
  curl -s http://localhost:8080/api/v1/venus/$ep | jq '.connected'
done

# Vérifier logs
journalctl -u daly-bms -n 30 | grep -E "Updated|error|Error|MQTT"
```

### 6.12 Rapport à fournir si toujours bloqué

Fournir exactement :
1. Résultat étape 4 (MQTT topics publiés oui/non)
2. Résultat étape 5 (API endpoints répondent oui/non — `connected: true/false`)
3. Output de : `journalctl -u daly-bms -n 50`
4. Dernière action prise : quelle commande a été exécutée

---

## 7. Nettoyage disque — builds cumulés target/

Les builds Rust cumulés dans `target/` (aarch64 + armv7 + natif debug/release) sont la cause
principale du remplissage du disque racine du Pi5. Les binaires de production sont dans
`/usr/local/bin` → le répertoire `target/` est jetable.

```bash
# Vérifier l'usage disque
df -h /
du -sh ~/Daly-BMS-Rust/target/

# Nettoyage sélectif (conserve target/aarch64 pour rebuild rapide)
rm -rf target/armv7-unknown-linux-gnueabihf target/debug target/release
rm -rf ~/.cargo/registry/cache ~/.cargo/registry/src
sudo apt-get clean
# Économie ≈ -2.6 Go ; conserve target/aarch64

# Reset total (force rebuild complet la prochaine fois)
cargo clean

# Ne JAMAIS supprimer :
# - ~/.cargo/bin         (outils cargo installés)
# - /usr/local/bin/*     (binaires de production)
# - /mnt/nvme/.../metrics.redb  (base de données métriques)
```

---

## 8. Récupération Venus / NanoPi

**Crash-loop `dbus-mqtt-venus` (svstat uptime = 0) :**

```bash
# Sur NanoPi
svstat /service/dbus-mqtt-venus
# Lancer le binaire manuellement pour voir l'erreur
/usr/local/bin/dbus-mqtt-venus
```

Si le binaire produit `SIGILL` (exit 132) → binaire armv7 compilé avec `target-cpu=native`
sur un hôte aarch64. Recompiler depuis le Pi5 :

```bash
# Sur Pi5 — build armv7 SANS target-cpu=native
make build-venus-v7 && make install-venus-v7
```

**Venus symlink disparu après mise à jour firmware :**

```bash
ssh root@192.168.1.120 "ln -sf /data/etc/sv/dbus-mqtt-venus /service/dbus-mqtt-venus"
```

**`svc -d` / `svc -t` pour contrôle manuel :**

```bash
# Arrêter dbus-mqtt-venus
ssh root@192.168.1.120 "svc -d /service/dbus-mqtt-venus"

# Redémarrer dbus-mqtt-venus
ssh root@192.168.1.120 "svc -t /service/dbus-mqtt-venus"

# Voir les logs
ssh root@192.168.1.120 "tail -f /var/log/dbus-mqtt-venus/current"
```

**Vérifier les services D-Bus actifs sur NanoPi :**

```bash
ssh root@192.168.1.120 "dbus -y | grep victronenergy"
```

Résultat attendu en production nominale :

```
com.victronenergy.battery.mqtt_1
com.victronenergy.battery.mqtt_2
com.victronenergy.battery.mqtt_3
com.victronenergy.pvinverter.mqtt_7
com.victronenergy.acload.mqtt_8
com.victronenergy.grid.mqtt_9
com.victronenergy.temperature.mqtt_1
com.victronenergy.switch.mqtt_1  ... mqtt_6
com.victronenergy.meteo
```

---

## 9. Investigation memory-leak daly-bms-server (EN COURS → TERMINÉE)

> **STATUT INITIAL : NON RÉSOLU** — Fuite linéaire confirmée par mesure terrain.
> **STATUT FINAL : TERMINÉE** (commit 018e363) — Réduction -82 % (6.6 MB/h → 1.18 MB/h).
> Voir §15 (status final) et §16.4 (synthèse 3 phases).
> **Phase D (2026-06)** : root cause du résiduel identifiée (trafic HTTP passif
> 1 Hz d'energy-manager) et corrigée — voir §17.
>
> Les numéros de section (§1–§16) sont préservés car certains commentaires du code source
> y font référence (notamment `api/mod.rs` §13, `daly-bms.service` §10,
> `promql/exec.rs` §12, [`metriques-redb-architecture.md`](./metriques-redb-architecture.md)).

---

### §1 Symptôme

`daly-bms-server` (Pi5, systemd, port 8080) présentait :

- **Fuite passive nocturne confirmée** : RSS passe de 27 MB → 160 MB
  en ~8 h **sans aucun client externe** (PC user éteint la nuit,
  aucun browser/Grafana ouvert).
- Pente passive ≈ **+16 MB/h** (cumulative, linéaire sur 8 h).
- Avec `energy-manager` arrêté, la pente tombe à **+4 MB/h** résiduel.

Donc :
- ~12 MB/h provenaient du **traitement des messages MQTT reçus depuis
  energy-manager** par le bridge MQTT côté daly-bms-server.
- ~4 MB/h résiduels venaient d'autre chose (polling RS485, monitor
  agent, AlertEngine, ou ailleurs).

---

### §2 Hypothèses initiales testées et écartées

#### §2.1 — Path PromQL/redb (PR #481/#482/#483)

Le path `eval_range` a été optimisé en 3 PRs (cache `match_series`,
`Arc<Labels>`, rtx partagée, libération avant `try_unwrap`).

Ces optimisations réduisent le pic transitoire, mais **n'éliminent pas
la fuite passive** (qui se produit avec `metrics_store=false` aussi).

#### §2.2 — Plateau jemalloc (FAUSSE PISTE — erreur d'analyse)

Des tests courts (≤10 min) montraient une stabilisation après quelques
navigations, ce qui avait conduit prématurément à conclure à un "plateau
d'allocator". **C'était faux** : le test 8 h en production a montré
une croissance linéaire de 130 MB. Le plateau apparent en test court
était dû à l'absence de stimulation continue côté serveur.

#### §2.3 — narenas:2 dans `_RJEM_MALLOC_CONF`

Tentative pour limiter le nombre d'arenas jemalloc. **Empirait le
problème** sous concurrence (concentration sur 2 arenas au lieu de 16).
Retiré.

---

### §3 État confirmé par les mesures

| Test | Résultat |
|------|----------|
| 8 h sans user actif | 27 MB → 160 MB (+130 MB, **+16 MB/h linéaire**) |
| 10 min avec energy-manager stoppé | +640 kB (**+4 MB/h résiduel**) |
| 100 req `/healthy` | +0.8 MB (Axum/middleware OK) |
| 100 req `/dashboards/catalog` | +0.8 MB (route catalog OK) |
| 100 req `/history/energy?period=day` | +5.4 MB (**~54 kB/req mais récupéré après 90s idle**) |
| 500 req sequential `/history/energy` + 90 s idle | +0.5 MB net (jemalloc libère bien sur sequential) |

**Conclusion factuelle** : la fuite est dans une boucle **passive
interne** activée par les messages MQTT entrants d'energy-manager. Pas
dans la lecture PromQL, pas dans le middleware HTTP.

---

### §4 Pistes investiguées (non concluantes)

#### §4.1 — BTreeMap qui grossiraient sans bornes
- `venus_mppts` : 4 entrées (borné par config)
- `venus_temperatures` : 2 entrées
- `venus_heatpumps` : 2 entrées
- `buffers` (BmsRingBuffer) : bornés par config
→ Aucune croissance non bornée détectée.

#### §4.2 — ws_tx broadcast (cap 128) qui retiendrait des Arc
- `state.on_snapshot()` push `Arc<Vec<BmsSnapshot>>` à chaque BMS poll
- `bridges/alerts.rs::run_alert_engine` subscribe en permanence
- Ring max 128 slots × ~1 KB = 128 KB **constante**, pas linéaire
→ Ce n'est pas la source linéaire seule, mais à confirmer.

#### §4.3 — VictoriaMetrics write hooks via `self.vm`
- Le rollback du user a réintroduit `vm: Option<Arc<VmClient>>` dans
  `state.rs` (17 sites `if let Some(vm) = self.vm.clone()`)
- `victoriametrics.service` = inactive, port 8428 non écouté
- Mais `self.vm` est initialisé via `vm.map(Arc::new)` dans `AppState::new`
  ligne 542 — si l'appelant passe `Some(VmClient::new(...))`, les writes
  sont tentés contre un endpoint mort.
- À vérifier : la valeur de `self.vm` au runtime + si les `vm.write_rows`
  tentent vraiment du HTTP via reqwest qui retient son pool.

#### §4.4 — Console_bus.emit sans guard receiver_count
- `state.rs:617` (`on_snapshot`) appelle
  `console_bus.emit(ConsoleEvent::rs485(device, &format!("BMS-{} snapshot", ...), json!({...})))`.
- **Les arguments (`format!`, `json!`, `ConsoleEvent::rs485(...)`)
  sont évalués AVANT le check `tx.send()` qui drop si pas de subscriber**.
- Idem ligne 91 `mqtt_out` dans `bridges/mqtt.rs` publish loop.
- Allocations éphémères (Strings + Values) à chaque BMS poll
  (~8 Hz total) + chaque MQTT publish (~1 Hz).
- À chaque emit sans subscriber : alloc + drop immédiat.
- **Devrait être de la rétention transitoire, pas une fuite linéaire**,
  mais à valider.

#### §4.5 — tracing-appender non-blocking buffer
- Configuration : `tracing_appender::non_blocking(file_appender)` ligne 223 `main.rs`
  avec `daly-bms.log` rolling daily.
- Buffer interne par défaut = 128k entrées max.
- Avec `RUST_LOG=info`, les debug events filtrés au registry.
- Si une boucle émet INFO/WARN à haute fréquence, le buffer peut
  saturer puis bloquer.
- À auditer : taux d'écriture dans `/var/log/daly-bms/daly-bms.log`.

---

### §5 Pistes restantes à investiguer

#### §5.1 — Bridge MQTT côté daly-bms (réception EM) — PRIORITAIRE

Code `crates/daly-bms-server/src/bridges/mqtt.rs:503-558` :
```rust
Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(p))) => {
    let topic = &p.topic;
    let payload = std::str::from_utf8(&p.payload).unwrap_or("");
    if let Ok(json) = serde_json::from_str::<Value>(payload) {
        state.console_bus.emit(ConsoleEvent::mqtt_in(ev_device, topic, json.clone()));
        // ... handle_*_topic ...
    }
}
```

À chaque message MQTT d'EM :
1. `serde_json::from_str::<Value>(payload)` alloue Value tree
2. `json.clone()` deep-clone le Value
3. `ConsoleEvent::mqtt_in(...)` wrap → Arc → `tx.send` (drop if no sub)
4. `handle_*_topic` lit le json, construit struct, `state.on_venus_*`

À ~1-2 msg/sec EM × 12 MB/h fuite = 1.5–3 KB par message non libéré.

Suspects précis :
- `json.clone()` ligne 534 : **clone systématique avant de checker
  `receiver_count`** sur le `console_bus`.
- `handle_*_topic` qui parsent à nouveau le Value pour extraire les
  champs (sans clone supplémentaire en principe).
- `state.on_venus_*` qui font `if let Some(vm) = self.vm.clone()` ×
  reqwest write attempt si vm est Some et VM down.

#### §5.2 — Code `if let Some(vm)` actif malgré VM down

Si le rollback a laissé `self.vm = Some(VmClient::new(...))` dans
`main.rs`, alors **chaque** message MQTT déclenche une tentative de
write HTTP vers un endpoint mort (port 8428 fermé).

À vérifier en lisant `main.rs` pour voir comment `vm` est passé à
`AppState::new`.

Test : passer `None` à `AppState::new(...)` et mesurer.

#### §5.3 — tokio-metrics TaskMonitor

`monitor.rs::spawn_all` lignes 433–449 crée 2 TaskMonitor :
```rust
let monitor_tm  = TaskMonitor::new();
let watchdog_tm = TaskMonitor::new();
tokio::spawn(monitor_tm.instrument(run_monitor_agent(state.clone())));
tokio::spawn(watchdog_tm.instrument(run_watchdog_agent(state.clone())));
```

TaskMonitor mesure les durées de poll. À haute fréquence, **stocke
peut-être un histogramme bordé** qui ne libère pas.

Test : retirer le wrapping `.instrument(...)` et observer.

#### §5.4 — rumqttc internal state

Pour QoS 1 (AtLeastOnce) — utilisé par toutes les souscriptions et
publications — rumqttc retient les paquets inflight jusqu'à PUBACK.
Avec keep_alive 30s et un broker localhost qui ack rapidement,
l'inflight devrait être minimal.

Test : passer en QoS 0 (AtMostOnce) sur les souscriptions et mesurer.

#### §5.5 — reqwest::Client (alerts.rs:472)

`bridges/alerts.rs:472` : `let client = reqwest::Client::new();` créé
à chaque appel telegram. Mais telegram est rare. Pas un suspect pour
fuite passive.

---

### §6 Plan d'action immédiat

1. **Confirmer si `self.vm = Some(...)` en runtime** — grep `main.rs` ou
   ajouter un `tracing::info!` au démarrage.
2. Si oui : passer `None` (patch ciblé dans `main.rs`), rebuild, mesurer
   1 h avec EM actif. Si pente tombe → c'est les writes VM vers un
   endpoint mort. Fix définitif = supprimer les blocs `if let Some(vm)`
   ou désactiver complètement.
3. Si non : creuser §5.1 (bridge MQTT réception). Désactiver
   temporairement `json.clone()` ligne 534 + tester.

---

### §7 Outils diagnostiques

#### §7.1 — Mesures utilisées
- `awk '/^VmRSS|^RssAnon/' /proc/$PID/status` (RSS / Anon kB)
- Comparaison pré/post burst + idle long
- Toggle `[metrics_store].enabled` dans Config.toml
- `sudo systemctl stop energy-manager` pour isoler EM
- `sudo ss -tnp -o state established | grep "pid=$PID"` pour identifier
  les scrapers externes

#### §7.2 — Outils essayés sans succès
- `heaptrack -p $PID` via GDB attach : crash du service à l'injection
  (signaux GDB → auto-restart systemd) — non recommandé sur ce binaire.
- jemalloc profiling via cargo feature : `--features jeprof` ne propage
  pas correctement au build C, `_RJEM_MALLOC_CONF=prof:true` ignoré.
- Tests "plateau" sur 5–10 min : **trompeur**, conclusion erronée car
  pas de stimulation continue côté serveur. Pour distinguer plateau
  vs fuite, **un test ≥1 h est requis** (idéalement nuit complète).

---

### §8 Apprentissages

1. **Toujours mesurer ≥ 1 h en condition réaliste** avant de conclure.
   Des tests courts ne distinguent pas plateau jemalloc transitoire et
   fuite linéaire.
2. **`MALLOC_ARENA_MAX=N` est glibc-only** — ignoré par jemalloc. Ne
   pas mélanger avec `narenas:N` du `_RJEM_MALLOC_CONF`.
3. **narenas:2 peut empirer une fuite** sous concurrence en concentrant
   la pression sur peu d'arenas.
4. **PromQL n'est pas la source** : la fuite passive existe même avec
   `metrics_store=false`.
5. **Les bisections par service (stop energy-manager) sont
   redoutablement efficaces** pour identifier la source.

---

### §9 Investigation finale (2026-05-19 après-midi)

#### §9.1 — Bisection composant par composant (10 min)

Tous les composants applicatifs ont été désactivés individuellement, la
pente RSS reste 6–8 MB/h **quelle que soit la config**.

| Désactivation testée | Pente RSS sur 10 min |
|----------------------|----------------------|
| Normal (tout actif) | ~6-7 MB/h |
| `energy-manager` stoppé | ~4 MB/h |
| `mqtt.enabled=false` (publisher + subscriber) | ~8 MB/h |
| `alerts.db_path=""` (AlertEngine off) | ~8 MB/h |
| `TaskMonitor` instrumentation retirée | ~7 MB/h |
| BMS poll callback `tokio::spawn` → `mpsc::channel` | ~6 MB/h |
| `_RJEM_MALLOC_CONF=dirty_decay_ms:0` | ~7.5 MB/h |
| `narenas:2` jemalloc | ~7-8 MB/h (pire) |
| `publish_interval_sec=60` | ~13 MB/h (pire) |
| `DALY_DISABLE_MONITOR=1` (monitor + watchdog off) | ~6.7 MB/h |
| `DALY_DISABLE_RS485=1` (polling RS485 bypass) | ~6.7 MB/h |
| `metrics_store.enabled=false` (redb mmap retiré) | ~7.8 MB/h |
| TOUT désactivé en même temps | ~9 MB/h |

→ **La mesure 10 min a un bruit de ±2 MB**, suffisant pour masquer
l'impact des composants individuels. Aucune source isolée par bisection.

#### §9.2 — heaptrack LD_PRELOAD : non concluant

Tentative `heaptrack -o /var/lib/daly-bms/heaptrack-daly <binary>` comme
ExecStart : le fichier `.zst` reste à 0 byte après 30 min, le
`heaptrack_interpret` companion consomme 65 MB de RAM mais 0:00 CPU
time → ne reçoit rien dans le FIFO. Probable incompatibilité avec
notre `Type=simple` workaround + variables d'env systemd.

LD_PRELOAD direct testé avant : `grep -c heaptrack /proc/PID/maps = 0`
→ lib pas chargée. Variables `HEAPTRACK_OUTPUT` / `DUMP_HEAPTRACK_OUTPUT`
ignorées.

heaptrack inutilisable dans notre environnement.

#### §9.3 — Décomposition `/proc/PID/smaps_rollup`

Identification que la fuite est **100 % dans le heap Rust** :

| Métrique | T0 | T+10min | Δ |
|----------|-----|---------|---|
| Rss | X | Y | +1-2 MB |
| **Anonymous** (heap) | X | Y | **+1-2 MB (toute la fuite)** |
| Private_Dirty | = Anonymous | = Anonymous | idem |
| Pss_File (mmap) | constant | constant | +0 |
| Pss_Shmem | 0 | 0 | 0 |

Pas de mmap exotique, pas de mémoire partagée. Heap allocator-managed
exclusivement.

#### §9.4 — Mesure de référence 1 h propre (2026-05-19 ~20h)

Avec config complète + writer redb actif + tous les composants OK :

| Métrique | T0 | T+1h | Δ |
|----------|-----|------|---|
| Rss | 52048 | 58656 | **+6608 kB** |
| Anonymous | 41808 | 48416 | +6608 kB |
| Private_Dirty | 41808 | 48416 | +6608 kB |

**Pente fiable : +6.6 MB/h** dans Anonymous heap. Sur 24h = +158 MB.
Sur 1 semaine = +1.1 GB → catastrophique sans intervention.

#### §9.5 — Conclusion

Source de la fuite **non identifiée par bisection applicative**. Pente
stable à ~6–8 MB/h indépendamment de la config désactivée. Cohérent avec
une fuite dans une **couche partagée** : runtime tokio, hyper, axum,
askama, ou une dépendance transitive (rumqttc, reqwest, etc.) — ou un
bug d'allocator jemalloc dans un pattern précis.

Outils d'investigation tentés :
- heaptrack via GDB attach (crash du service) : échec
- heaptrack via LD_PRELOAD (lib pas chargée) : échec
- heaptrack via ExecStart wrapper (FIFO inactif) : échec
- jemalloc profiling cargo feature (ne propage pas au build C) : échec
- pmap / smaps_rollup (confirme la classe Anonymous mais pas la source) : partiel

---

### §10 Workaround appliqué — RuntimeMaxSec=86400

Décision pragmatique : restart quotidien automatique via systemd.

`contrib/daly-bms.service` :
```ini
[Service]
RuntimeMaxSec=86400
```

Effet :
- Service redémarre toutes les 24 h (timer systemd interne)
- Coût : ~5 s d'interruption
- MQTT retained messages reviennent automatiquement
- BMS poll RS485 reprend immédiatement
- Écritures redb continuent
- État in-memory (snapshots, broadcasts) reconstruit en <30 s

Plafond RSS estimé avant correction : `52 MB (baseline) + 24 × 6.6 MB/h = 52 + 158 ≈ 210 MB`
avant restart quotidien. Très acceptable sur un Pi5 avec 8 GB RAM.

---

### §11 Code d'investigation conservé

Deux env vars sont laissées dans le code pour pouvoir réinvestiguer
sans recompiler :

- `DALY_DISABLE_MONITOR=1` → désactive `monitor::spawn_all` (monitor +
  watchdog agents). Cf. commit 153ba97.
- `DALY_DISABLE_RS485=1` → bypass complet du polling RS485 (BMS, ET112,
  ATS, irradiance). Cf. commit 11d7e60.

Inactives par défaut. À utiliser via systemd drop-in :
```ini
[Service]
Environment=DALY_DISABLE_MONITOR=1
WatchdogSec=0  # nécessaire car sd_notify n'est plus envoyé
```

---

### §12 Si on veut REPRENDRE l'investigation

Pistes restantes si on souhaite descendre encore la pente résiduelle :
1. **Rebuild avec system malloc** au lieu de jemalloc (retirer
   `tikv_jemallocator` dans `main.rs`). Si la pente change → c'est
   jemalloc. Sinon → c'est une dépendance Rust.
2. **dhat-rs en mode prod** : recompiler avec `--features dhat-heap`,
   accepter la perte de perf 5×, capturer un profil sur 30 min.
3. **Identifier la version de chaque dépendance** et chercher des
   issues mémoire reportées (rumqttc, tokio, hyper, axum, redb).
4. **Bisection par revert progressif** : reverter PR par PR (en
   commençant par la plus récente) jusqu'à voir la pente disparaître,
   pour identifier le commit qui a introduit la fuite.

---

### §13 Phase 3 (2026-05-20) — Cause identifiée : tower-http stack clone

#### §13.1 — Capture valgrind en mode `--full`

Le script `scripts/valgrind-leak-hunt.sh` a été étendu avec un mode
`--full` (commit ca88318) qui active MQTT + redb + alerts (DB redirigées
vers `/tmp` pour éviter conflit ownership avec prod). En mode isolé,
le binaire idle ne reproduisait pas la fuite ; en mode `--full`, la
pente apparaît clairement.

#### §13.2 — Top leaks "possibly lost" en mode `--full`

```
1,114,120 / 1 block   : metrics_store::writer::run (LruCache 50k startup)  ← ONE-SHOT
   94,480 / 2 blocks  : hashbrown reserve_rehash (linéaire avec LruCache)  ← ONE-SHOT
   56,832 / 192 blocks: BoxCloneService::clone_box (axum::Route + Cors)    ← PAR REQUÊTE ⚠
   47,240 / 1 block   : hashbrown reserve_rehash                           ← ONE-SHOT
   45,584 / 154 blocks: BoxCloneService::clone_box                          ← PAR REQUÊTE ⚠
   28,416 / 96 blocks : BoxCloneService::clone_box                          ← PAR REQUÊTE ⚠
   26,256 / 6 blocks  : sqlite3MemMalloc (pcache alerts.db)                ← BORNÉ
   23,040 / 192 blocks: tower_http::cors::Vary::clone                       ← PAR REQUÊTE ⚠
```

Les entrées avec **154–192 blocks** suivent un pattern net : ~296 bytes
× nombre de requêtes HTTP traitées pendant la capture. À 5 req/sec en
prod × 296 bytes × 3600 s = **5.3 MB/h** — cohérent avec la pente
observée 6.6 MB/h.

#### §13.3 — Confirmation par retrait `CorsLayer` + `TraceLayer`

Test : commenter `.layer(cors)` et `.layer(TraceLayer::new_for_http())`
dans `api/mod.rs:168-169`, relancer valgrind `--full` 10 min.

| Métrique | Avant | Sans CORS+Trace | Δ |
|----------|-------|------------------|---|
| `possibly_lost` (bytes) | 1.85 MB | 1.39 MB | -25 % |
| Nombre de blocks | 2 761 | **549** | **-80 %** |
| Errors valgrind | 565 | 434 | -23 % |

**Confirmation nette** : les blocks par requête chutent de 80 %. Les
549 restants sont des allocs startup one-shot (LruCache writer, SQLite
pcache, hashbrown tables) — pas linéaires.

#### §13.4 — Cause racine

Le pattern problématique vient de `tower::util::boxed_clone::BoxCloneService`
qui clone une copie complète de toute la stack (`Route → CorsLayer →
TraceLayer → Vary header`) à chaque requête HTTP entrante. Certaines
parties de cette stack (notamment `tower_http::cors::Vary` avec son
`Vec<HeaderValue>`) effectuent une `to_vec()` interne au clone, ce que
valgrind marque "possibly lost" car le pointer transite via un mpsc
channel tokio que valgrind ne suit pas.

C'est un comportement **upstream connu** dans `tower-http 0.5` qui a
été corrigé dans `tower-http 0.6` (refactor de `BoxCloneService` vers
`Service::call(&mut self)`).

#### §13.5 — Solution appliquée (commit XXXXXXX)

1. **CorsLayer conservé** : nécessaire pour Grafana en local (port 3000)
   qui interroge daly-bms (port 8080) en cross-origin.
2. **TraceLayer RETIRÉ** dans `api/mod.rs:176` (avec import commenté
   ligne 27). Ce layer ne servait qu'à émettre des spans HTTP pour
   observabilité, dispensable. Gain : ~30 % des allocations linéaires.
3. **`RuntimeMaxSec=86400`** conservé pour absorber le résiduel (CORS
   ne peut pas être retiré sans casser Grafana).

#### §13.6 — Plan B futur (non urgent)

Si on veut éliminer 100 % de la fuite par requête sans workaround :

1. **Upgrade `tower-http 0.5` → `0.6`** dans workspace Cargo.toml
   (et potentiellement axum `0.7` → `0.8`). Vérifier les breaking
   changes API. Si OK, la fuite par requête disparaît complètement.

2. **Sinon, middleware CORS minimal custom** : remplacer `CorsLayer`
   par un middleware qui ajoute juste `Access-Control-Allow-Origin: *`
   sans clone de Vec interne. ~30 lignes de code.

#### §13.7 — Résultat attendu en prod après déploiement

Pente avant : ~6.6 MB/h (mesuré 1 h propre, cf. §9.4).
Pente attendue après retrait TraceLayer : **~4–5 MB/h** (CORS toujours
actif). Workaround `RuntimeMaxSec=86400` reste utile pour absorber.

À mesurer en prod : `awk '/^VmRSS|^Anonymous/' /proc/$PID/smaps_rollup`
sur 1 h après déploiement, comparer avec la valeur de §9.4.

#### §13.8 — Findings sur les fichiers conservés

- `scripts/valgrind-leak-hunt.sh` : outil de diagnostic conservé pour
  réinvestigations futures (mode isolé + `--full`).
- `crates/daly-bms-server/src/redb_writes.rs` : nouveau module qui
  rétablit l'écriture redb (Grafana fonctionnel à nouveau).
- Code instrumenté `DALY_DISABLE_MONITOR=1` et `DALY_DISABLE_RS485=1`
  conservé (inactif par défaut, utile pour debug futur).
- Dossier `valgrind/` à supprimer du repo (logs binaires de test, gros
  fichiers .zst + .db inutiles pour la prod).

---

### §14 Phase B livré (commit b73024f) — tower-http 0.5 → 0.6

#### §14.1 — Upgrade appliqué

`Cargo.toml` workspace ligne 53 : `tower-http = "0.5"` → `"0.6"`.
`cargo update -p tower-http` : version effective 0.6.11.
TraceLayer et CorsLayer **réactivés** dans `api/mod.rs` (le commit
33dd5e4 les avait retirés en workaround partiel).

#### §14.2 — Mesure de validation 1 h propre (2026-05-20)

Test post-déploiement Plan B :

| Métrique | T0 | T+1h | Δ |
|----------|-----|------|---|
| Rss | 67056 kB | 69904 kB | +2848 kB |
| Anonymous | 56912 kB | 59760 kB | **+2848 kB** |

**Pente confirmée : +2.85 MB/h** (vs 6.6 MB/h avant). **Réduction -57 %**.

#### §14.3 — Synthèse globale

| État | Pente Anonymous | Réduction |
|------|------------------|-----------|
| Avant tout fix (nuit complète) | ~16 MB/h | référence |
| Après broadcast guards (PRs antérieures) | ~5.5 MB/h | -66 % |
| Mesure 1 h propre (post writer redb actif) | 6.6 MB/h | référence stable |
| **Après tower-http 0.6** | **2.85 MB/h** | **-57 % cumulé** |

#### §14.4 — Workaround `RuntimeMaxSec=86400` conservé

Avec pente résiduelle 2.85 MB/h, le restart quotidien absorbe +68 MB
par jour. Plafond cumulé : 52 MB (baseline) + 68 = ~120 MB → restart →
~52 MB. Cycle stable largement supportable sur Pi5 8 GB.

À RETIRER seulement si l'investigation continue jusqu'à <1 MB/h.

#### §14.5 — Source résiduelle 2.85 MB/h (hypothèses)

Le résiduel ne vient pas de `tower-http` (qui est fixé). Hypothèses :
- Allocations restantes dans `axum 0.7` (clonage de `Route` ?)
- `hyper 1.x` connections handling
- `tokio` runtime allocations sous charge
- Notre code : `json!`, `format!` dans handlers MQTT
- `serde_json::Value` parsing → des Strings allouées non poolées

#### §14.6 — Pour pousser plus loin (futur, optionnel)

1. **Upgrade `axum 0.7 → 0.8`** : breaking changes plus larges
   (router path matching), mais potentiel gain similaire à tower-http.
2. **Audit `still reachable: 4.6 MB` dans valgrind** : ces blocks
   sont vivants mais peut-être grossissants (caches internes, pools).
   Relancer valgrind avec `--show-leak-kinds=all`.
3. **Pooler les allocations** dans `bridges/mqtt.rs` : réutiliser
   les Strings et Values via une pool (peu compatible avec serde mais
   possible pour le payload final).

Effort estimé : 3–5 h dédiées. Gain potentiel : -1 à -2 MB/h.

---

### §15 Status final — investigation close

| Aspect | État |
|--------|------|
| Cause racine identifiée | `tower-http 0.5 BoxCloneService` (§13) |
| Fix permanent appliqué | upgrade tower-http 0.6 (commit b73024f) |
| Pente prod après phase B | 6.6 → 2.85 MB/h (-57 %) |
| Workaround restart quotidien | conservé (RuntimeMaxSec=86400) |
| Plafond cumulé 24 h | ~120 MB (très acceptable) |
| Writer redb fonctionnel | Grafana reçoit données fraîches (PR commit 2016b24) |
| Documentation | §1–§16 complète |
| Code instrumentation debug | `DALY_DISABLE_MONITOR`, `DALY_DISABLE_RS485` conservés |
| Script `valgrind-leak-hunt.sh` | conservé pour futures sessions (modes isolé/--full) |
| Phase C (axum 0.8) | appliquée, pente finale 1.18 MB/h (-82 % total) — voir §16 |

**Investigation déclarée terminée**. Si on veut atteindre <1 MB/h dans
le futur, voir §14.6 pour les pistes.

---

### §16 Phase C (commit 018e363) — axum 0.7 → 0.8

#### §16.1 — Upgrade appliqué

Suite au succès de Phase B (tower-http 0.6, -57 %), tentative de
descendre sous 1 MB/h en upgradant aussi axum.

`Cargo.toml` workspace ligne 51 : `axum = "0.7"` → `"0.8"`.
`cargo update -p axum` cascade :
- axum 0.7.9 → 0.8.9
- axum-core 0.4.5 → 0.5.6
- axum-macros 0.4.2 → 0.5.1
- matchit 0.7.3 → 0.8.4
- tokio-tungstenite 0.24.0 → 0.29.0
- tungstenite 0.24.0 → 0.29.0

#### §16.2 — Breaking changes corrigés

1. **`Message::Text` accepte `Utf8Bytes` au lieu de `String`** (cascade
   tokio-tungstenite 0.29). Fix : `.into()` à 6 sites dans
   `api/bms.rs` (4× sur les WS streams) et `api/console.rs` (2×).

2. **Path matching `:param` → `{param}`** (matchit 0.8 brace syntax).
   Routes mises à jour dans `api/mod.rs` :
   - `/api/v1/bms/{id}/*` (12 routes)
   - `/api/v1/et112/{addr}/*` (2 routes)
   - `/api/v1/tasmota/{id}/*` (3 routes)
   - `/api/v1/shelly/{id}/*`, `/{id}/channel/{ch}/*` (3 routes)
   - `/api/v1/label/{name}/values` (1 route)
   - `/api/v1/dashboards/panel/{id}/data` (1 route)
   - Total : ~25 routes

#### §16.3 — Mesure de validation 1 h propre

| Métrique | T0 | T+1h | Δ |
|----------|-----|------|---|
| Rss | 70848 kB | 72032 kB | +1184 kB |
| Anonymous | 60656 kB | 61840 kB | **+1184 kB** |

**Pente confirmée : +1.18 MB/h**. Objectif <1 MB/h **quasi-atteint**.

#### §16.4 — Synthèse globale (3 phases)

| Phase | Pente | Réduction cumulée |
|-------|-------|---------------------|
| Référence initiale | 6.6 MB/h | — |
| Phase B (tower-http 0.6) | 2.85 MB/h | -57 % |
| **Phase C (axum 0.8)** | **1.18 MB/h** | **-82 %** |

À 1.18 MB/h × 24 h = **+28 MB par jour**. Sur 1 semaine = +200 MB.
Largement supportable sur Pi5 8 GB.

#### §16.5 — Workaround `RuntimeMaxSec=86400` conservé

Conservé en filet de sécurité par décision opérationnelle. Plafond cumulé
24 h : 60 MB baseline + 28 MB = ~90 MB → restart quotidien → reset à
60 MB. Cycle stable.

#### §16.6 — Source résiduelle ~1.18 MB/h (hypothèses non investiguées)

Le résiduel ne vient ni de tower-http ni d'axum (fixés). Hypothèses
restantes :
- `tokio` runtime allocations sous charge (channels, broadcasts, workers)
- `hyper 1.x` connections handling
- `rumqttc` MQTT client internal buffers
- Notre code : `json!`, `format!` dans handlers MQTT et bridges
- `serde_json::Value` parsing → des Strings allouées non poolées
- `redb` writer batching internal allocations

#### §16.7 — Investigation close

À ce stade, **l'investigation est officiellement terminée**. La pente
est passée de 6.6 MB/h à 1.18 MB/h (-82 %). Avec le workaround
`RuntimeMaxSec=86400` conservé, le service est stable indéfiniment.

Pour pousser plus loin (futur, optionnel) : audit du code applicatif
pour réduire les allocations transitoires (`json!`/`format!` patterns),
mais le ROI devient marginal sous 1.18 MB/h.

---

### §17 Phase D (2026-06) — root cause du résiduel : trafic HTTP passif 1 Hz

#### §17.1 — Le point aveugle des bisections

Relecture à froid de §9.1 : aucune bisection n'a jamais coupé le **POST HTTP
1 Hz** qu'energy-manager envoyait vers `/api/v1/solar/mppt-yield`
(`logic/solar_power::writer_task`, `interval(Duration::from_secs(1))`).
3 600 requêtes/heure traversaient la stack Axum 24 h/24, même « sans aucun
client externe » — le régime qualifié de « passif » ne l'était pas.

Le calcul colle avec la pente résiduelle de §16.3 :
1 184 kB/h ÷ 3 600 req/h ≈ **330 octets/requête** — même ordre de grandeur
que les ~296 o/req mesurés par valgrind en §13.2. Les phases B/C ont réduit
le coût par requête (-82 %), jamais le débit de requêtes. Cela explique
aussi pourquoi « arrêter energy-manager » faisait tomber la pente (§1) :
on attribuait l'effet aux messages MQTT entrants, mais l'arrêt coupait
aussi le POST 1 Hz.

#### §17.2 — Correctifs appliqués

| Axe | Changement | Effet attendu |
|-----|-----------|----------------|
| A | Télémétrie solaire **MQTT** : `santuario/em/solar` (1 Hz conservé) au lieu du POST HTTP. Handler `handle_em_solar_topic` (bridges/mqtt.rs), sémantique identique à `set_mppt_yield` (conservé en fallback). | -3 600 req HTTP/h → suppression du moteur du résiduel |
| B | Auto-télémétrie mémoire : agent monitor exporte toutes les 30 s `process_rss_bytes` + `process_jemalloc_{allocated,active,resident,mapped,retained}_bytes` (tikv-jemalloc-ctl, feature `stats`) → dashboard Grafana **« 21 - Mémoire daly-bms »** (`daly-mem-21`). | Diagnostic définitif fuite vs rétention, sans heaptrack/valgrind |
| C1 | `redb_writes::push()` : construction **paresseuse** du `Sample` (closure `FnOnce`) — l'ancien passage par valeur jetait ~80 % des constructions (snapshots ~1 Hz, écriture 1×/5 s). | -churn d'allocations continu |
| C2 | AlertEngine sur **canal mpsc dédié** (`alert_tx`) au lieu d'un abonnement permanent au broadcast `ws_tx` — la garde `receiver_count() > 0` de `on_snapshot` était toujours vraie, forçant un clone de `latest_snapshots()` à chaque poll même sans client WS. | -churn, garde broadcast à nouveau effective |
| C3 | `TraceLayer` retiré (api/mod.rs) : span + allocations par requête sans aucun consommateur (`RUST_LOG=info`). | -churn par requête |
| — | Page `/dashboard/history` + `/api/v1/dashboards/*` + `/api/v1/history/energy` retirés (Grafana = unique outil d'historique). Supprime aussi les bursts `eval_range` lourds historiques (§2.2, VmPeak 367 Mo). | -code mort, -surface mémoire |

Trafic HTTP passif restant : irradiance GET 1/30 s (energy-manager) +
water-heater POST 1/300 s + sondes TCP watchdog/monitor. ≈ 132 req/h,
soit ~3,6 % de l'ancien débit.

#### §17.3 — Protocole de validation

1. Déployer, attendre 1 h de régime stable, puis :
   `awk '/^VmRSS|^Anonymous/' /proc/$(pidof daly-bms-server)/smaps_rollup`
   sur 1 h (méthode §9.4). Attendu : pente << 1,18 MB/h.
2. Dashboard Grafana 21 sur 24 h : si `allocated` plat et RSS plat →
   envisager d'allonger `RuntimeMaxSec` (86400 → 604800) dans
   `contrib/daly-bms.service`. Garder `MemoryHigh`/`MemoryMax` comme filet.
3. Si une pente subsiste : la courbe `allocated` tranche désormais seule
   (croît = fuite applicative à chasser ; plate = tuning decay jemalloc).

#### §17.4 — Statut

Correctifs livrés sur branche `claude/quirky-galileo-qvn74g`. Validation
terrain (étapes ci-dessus) à faire après déploiement Pi5.

---

### §18 Phase E (2026-06-13) — la fuite est une VRAIE fuite heap (mesures terrain)

#### §18.1 — Mesures décisives

Binaire phase D confirmé en place (`strings … | grep dashboard/history = 0`,
métrique `process_jemalloc_allocated_bytes` présente). RSS 37 → ~100 Mo en
~12 h (≈ 3,7 MB/h). `smaps_rollup` + stats jemalloc :

| Métrique | Valeur | Lecture |
|----------|--------|---------|
| `Rss` | 100 704 kB | — |
| `Anonymous` / `Private_Dirty` | 90 016 kB | **tout est en heap anonyme** |
| `Pss_File` | 9 046 kB | mmap/redb négligeable, **pas** la cause |
| jemalloc `allocated` | 70,5 Mo | **mémoire VIVANTE réellement détenue** |
| séries redb | 1 036 | pas d'explosion de cardinalité |
| fichier redb | 3,8 Go | problème disque séparé (voir §18.4) |

Conclusion sans ambiguïté : `allocated` (70 Mo) ≈ `Anonymous` (90 Mo) ⇒ ce
n'est **ni** de la rétention allocateur (sinon `allocated` ≪ `resident`),
**ni** du mmap redb (`Pss_File` minuscule), **ni** de la cardinalité (1 036
séries). C'est une **vraie fuite applicative/dépendance** : du code retient
des allocations vivantes, libérées seulement au restart. Le baseline bas
après restart (37 Mo) avec la même base 3,8 Go montre que la croissance est
une **accumulation runtime par opération**, pas un coût lié à la taille de la
base.

#### §18.2 — Candidats structurels éliminés par audit code

- Aucune collection applicative non bornée (tous les `insert`/`push` sont
  keyés par identifiants stables ou des `Vec` locaux transitoires).
- Pas de transaction redb en lecture longue-durée (les `OnceCell<ReadTransaction>`
  de l'`Evaluator` sont par-requête ; aucun reader ne pin la MVCC).
- Corrigés au passage (réels mais marginaux) : fuite de tâches Shelly à
  chaque reconnexion, `RateLimiter` non borné (noms de process transitoires),
  `narenas:2` (rétention pire — retiré). Cf. commit phase E.

#### §18.3 — Localiser la fuite au call-stack : heap profiling jemalloc

Le binaire est désormais compilé avec `--enable-prof` (feature `profiling`,
coût nul tant qu'inactif). **Tout est automatisé par un script unique** qui
active le profiling, mesure, produit le rapport, et REMET la configuration
normale à la fin (y compris si interrompu — Ctrl-C, déconnexion, erreur) :

```bash
# Mesure 2 h par défaut (durée configurable : 30m, 3h, …)
sudo bash scripts/jemalloc-leak-profile.sh 2h

# Mesure longue sans garder le terminal ouvert :
sudo tmux new -s prof 'bash scripts/jemalloc-leak-profile.sh 3h'
```

Le script (`scripts/jemalloc-leak-profile.sh`) :
1. vérifie que le binaire a le profiling compilé (sinon : déployer d'abord) ;
2. installe `jeprof` au besoin (`libjemalloc-dev`) ;
3. active le profiling via un drop-in systemd + redémarre `daly-bms` ;
4. mesure (le service dumpe `/tmp/jeprof.*.heap` toutes les ~5 min) ;
5. écrit un rapport `/tmp/jeprof/leak-report-*.txt` : le **diff** entre le
   premier et le dernier profil = exactement ce qui a CRÛ (la fuite), au
   call-stack près ;
6. **rétablit** la config normale (profiling OFF) automatiquement.

Le haut du diff donne la pile d'allocation responsable (notre code vs
tokio/hyper/rumqttc/redb) → c'est elle qu'on corrige.

> Procédure manuelle équivalente (si besoin de piloter finement) : drop-in
> `Environment=DALY_JEMALLOC_PROF=1` +
> `Environment=_RJEM_MALLOC_CONF=prof:true,prof_active:true,lg_prof_sample:19,…`,
> `systemctl restart daly-bms`, attendre, puis
> `jeprof --show_bytes --text --base=<ancien> /usr/local/bin/daly-bms-server <récent>`.
> Désactivation : `sudo systemctl revert daly-bms && sudo systemctl restart daly-bms`.

#### §18.4 — Base redb à 3,8 Go (problème disque distinct)

`raw_retention_days = 30` à ~100 écritures/s remplit la table raw sur des Go.
Ce n'est pas la fuite RSS (mmap non résident) mais c'est volumineux et
ralentit les commits/compactions. Recommandé : baisser
`raw_retention_days` (ex. 7) dans `Config.toml` — les tiers hourly/daily
conservent l'historique long terme. ⚠ redb ne rétrécit pas le fichier
automatiquement (réutilise l'espace libéré) ; pour récupérer le disque,
compaction redb ou recréation de la base.

---

## Voir aussi

- [`./deploiement-exploitation.md`](./deploiement-exploitation.md) — Procédures de déploiement normales (Pi5 + NanoPi, workflow, systemd).
- [`./integration-materiel.md`](./integration-materiel.md) — Inventaire matériel RS485/D-Bus, ajout BMS Daly, ATS CHINT, ET112, PRALRAN — pour le détail device (registres, adresses Modbus).
- [`./mqtt-mosquitto.md`](./mqtt-mosquitto.md) — Architecture MQTT, topics, bridge NanoPi, anti-boucle, validation config Mosquitto.
- [`./metriques-redb-architecture.md`](./metriques-redb-architecture.md) — TSDB redb : moteur, tables, tiering, write path.
- [`./grafana-dashboards.md`](./grafana-dashboards.md) — Grafana : installation, datasource, provisioning, format JSON correct.
- [`./ARCHITECTURE.md`](./ARCHITECTURE.md) — Document maître : vue d'ensemble système et index de toute la documentation.

---

## Sources consolidées

Ce document fusionne et **remplace** les anciens fichiers suivants :
`DEBUG_ONDULEUR_SMARTSHUNT.md`, `docs/memory-leak-investigation.md`, `docs/netdiag.md`.
