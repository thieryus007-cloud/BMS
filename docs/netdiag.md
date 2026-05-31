# 🔍 netdiag — Diagnostic Réseau

| | |
|:---|:---|
| **Date** | `2026-05-31 11:51:40` |
| **Fenêtre** | 10 secondes |
| **Commande** | `sudo bash scripts/netdiag.sh` |

---

## 📊 Débit Total (TX)

| Métrique | Valeur |
|:---|:---|
| **TX total (toutes interfaces non-loopback)** | **11 010 B/s** (~10 Ko/s, 0.09 Mbit/s) |

---

## 🌐 Débit TX par Interface

| Interface | Débit |
|:---|---:|
| `eth0` | 0 B/s |
| `wlan0` | **11 206 B/s** |

---

## 🔗 Top Connexions par Octets Envoyés

| B/s | Local | Pair | Processus |
|---:|:---|:---|:---|
| 9 516 | `192.168.1.120:1883` | `mosquitto` (pid 1194, fd 11) | `mosquitto` |
| 8 939 | `127.0.0.1:1883` | `daly-bms-server` (pid 1217, fd 14) | `daly-bms-server` → `mosquitto` |
| 2 978 | `127.0.0.1:45060` | `mosquitto` (pid 1194, fd 15) | `mosquitto` |
| 2 640 | `127.0.0.1:1883` | `energy-manager` (pid 1310, fd 9) | `energy-manager` → `mosquitto` |
| 1 474 | `127.0.0.1:45080` | `mosquitto` (pid 1194, fd 16) | `mosquitto` |
| 405 | `192.168.1.141:54878` | `daly-bms-server` (pid 1217, fd 26) | `daly-bms-server` |
| 314 | `192.168.1.141:8080` | `energy-manager` (pid 1310, fd 13) | `energy-manager` → `daly-bms (API/WS)` |
| 3 | `192.168.1.116:12451` | `sshd-session` (pid 1253/1244, fd 7) | `sshd-session` |

---

## 🔌 Connexions Établies par Port Local

| Count | Port | Service |
|---:|:---|:---|
| 6 | `:1883` | `mosquitto` |
| 4 | `:8080` | `daly-bms` (API/WS) |
| 1 | `:54890` | — |
| 1 | `:54878` | — |
| 1 | `:45080` | — |
| 1 | `:45060` | — |
| 1 | `:45032` | — |
| 1 | `:45030` | — |
| 1 | `:45014` | — |
| 1 | `:38842` | — |

---

## ⚡ Top Process CPU

| PID | Commande | %CPU | %MEM |
|---:|:---|---:|---:|
| 1217 | `daly-bms-server` | 1.2 | 1.9 |
| 1193 | `grafana` | 0.4 | 6.8 |
| 606 | `kworker/u17:2-b` | 0.3 | 0.0 |
| 1194 | `mosquitto` | 0.2 | 0.3 |
| 1310 | `energy-manager` | 0.1 | 0.2 |
| 299 | `systemd-journal` | 0.0 | 0.5 |
| 4359 | `kworker/0:2-eve` | 0.0 | 0.0 |

---

## 📡 MQTT Broker — Métriques $SYS (1 min)

| Métrique | Valeur |
|:---|---:|
| `load/bytes/sent/1min` | 874 924.87 |
| `load/messages/sent/1min` | 4 551.56 |
| `clients/connected` | 6 |

---

&gt; 💡 **Observations** : `mosquitto` domine le trafic sortant (~11 Ko/s via wlan0). Le broker MQTT gère 6 clients connectés avec un débit moyen de ~875 Ko/min.
