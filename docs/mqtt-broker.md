# Migration MQTT : Mosquitto/Docker → rumqttd + mqtt-bridge (Rust natif)

## Contexte

Mosquitto fonctionnait sous Docker sur le Pi5 avec un bridge MQTT vers le NanoPi.
Cette migration remplace ce setup par deux services systemd Rust natifs :

| Ancien | Nouveau |
|--------|---------|
| Docker + Mosquitto | `mqtt-broker` (rumqttd) |
| `mosquitto.conf` section `bridge` | `mqtt-bridge` (rumqttc) |
| `make up` / `make down` | `make mqtt-start` / `make mqtt-stop` |
| port 1883 TCP | port 1883 TCP (identique) |
| port 9001 WebSocket | port 9001 WebSocket (identique) |

---

## Architecture

```
Pi5 (127.0.0.1)
  mqtt-broker (rumqttd)
    ├── TCP  :1883    ← daly-bms-server, energy-manager, Tasmota, Shelly
    ├── WS   :9001    ← explorateur MQTT dashboard JS
    ├── HTTP :8082    ← /metrics (pour monitor agent + dashboard)
    └── Persistence /var/lib/mqtt-broker (retained + QoS 1/2)

  mqtt-bridge (rumqttc)
    ├── HTTP :8084    ← /metrics
    ├── SUB  localhost:1883   topics W/R/santuario/shellypro2pm → NanoPi
    └── SUB  192.168.1.120:1883  topics N/santuario → Pi5 local

NanoPi (192.168.1.120)
  Mosquitto :1883  ← broker Venus OS natif (inchangé)
```

---

## Fonctionnalités préservées

| Fonctionnalité Mosquitto | Équivalent rumqttd |
|------------------------|-------------------|
| `listener 1883` | `[v4.1] listen = "0.0.0.0:1883"` |
| `listener 9001 websockets` | `[ws.1] listen = "0.0.0.0:9001"` |
| `allow_anonymous true` | pas d'auth configurée |
| `persistence true` | `router.path = "/var/lib/mqtt-broker"` |
| `max_qos 2` | QoS 0/1/2 supporté nativement |
| `message_size_limit 1048576` | `max_payload_size = 1048576` |
| Bridge `N/c0619ab9929a/#` in | `mqtt-bridge` subscribe NanoPi → republish local |
| Bridge `W/c0619ab9929a/#` out QoS 1 | `mqtt-bridge` subscribe local → republish NanoPi QoS 1 |
| Bridge `R/c0619ab9929a/#` out QoS 1 | idem |
| Bridge `santuario/#` in | `mqtt-bridge` subscribe NanoPi → republish local |
| Bridge `santuario/{heat,heatpump,...}/#` out | `mqtt-bridge` subscribe local → republish NanoPi |
| Bridge `shellypro2pm-.../#` both | `mqtt-bridge` bidirectionnel |

---

## Installation initiale (premier déploiement sur Pi5)

```bash
# 1. Compiler
make build-mqtt-arm

# 2. Déployer (script automatique)
bash scripts/deploy-pi5.sh --mqtt-only

# OU déploiement manuel :

# Créer l'utilisateur système
sudo useradd --system --no-create-home --shell /usr/sbin/nologin mqtt-broker

# Répertoire persistence
sudo mkdir -p /var/lib/mqtt-broker
sudo chown mqtt-broker:mqtt-broker /var/lib/mqtt-broker

# Copier les binaires
sudo cp target/aarch64-unknown-linux-gnu/release/mqtt-broker /usr/local/bin/
sudo cp target/aarch64-unknown-linux-gnu/release/mqtt-bridge /usr/local/bin/

# Copier les configs
sudo cp crates/mqtt-broker/mqtt-broker.toml /etc/daly-bms/
sudo cp Config.toml /etc/daly-bms/config.toml  # contient [mqtt_bridge]

# Enregistrer les services
sudo cp contrib/mqtt-broker.service /etc/systemd/system/
sudo cp contrib/mqtt-bridge.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable mqtt-broker mqtt-bridge
sudo systemctl start mqtt-broker mqtt-bridge
```

---

## Bascule depuis Mosquitto (migration live)

Séquence recommandée (coupure ~2 minutes) :

```bash
# 1. Arrêter les services applicatifs
sudo systemctl stop daly-bms energy-manager

# 2. Arrêter Mosquitto Docker
make down

# 3. Démarrer rumqttd + bridge
sudo systemctl start mqtt-broker
sleep 3
sudo systemctl start mqtt-bridge

# 4. Vérifier
journalctl -u mqtt-broker -n 20
journalctl -u mqtt-bridge -n 20
mosquitto_sub -h localhost -t "santuario/#" -v   # test

# 5. Redémarrer les services
sudo systemctl start daly-bms energy-manager
```

**Rollback :** `sudo systemctl stop mqtt-broker mqtt-bridge && make up`

---

## Commandes courantes

| Action | Commande |
|--------|----------|
| Démarrer | `make mqtt-start` ou `sudo systemctl start mqtt-broker mqtt-bridge` |
| Arrêter | `make mqtt-stop` |
| Redémarrer | `make mqtt-restart` |
| Logs temps réel | `make mqtt-logs` |
| Statut | `make mqtt-status` |
| Métriques broker | `curl http://localhost:8082/metrics` |
| Métriques bridge | `curl http://localhost:8084/metrics` |

---

## Configuration

### Broker (`/etc/daly-bms/mqtt-broker.toml`)

Copié depuis `crates/mqtt-broker/mqtt-broker.toml`. Modifier si besoin et redémarrer :

```bash
sudo systemctl restart mqtt-broker
```

### Bridge (`/etc/daly-bms/config.toml` section `[mqtt_bridge]`)

```toml
[mqtt_bridge]
local_host     = "127.0.0.1"
local_port     = 1883
remote_host    = "192.168.1.120"   # NanoPi
remote_port    = 1883
portal_id      = "c0619ab9929a"    # ID Victron GX
reconnect_secs = 30
keepalive_secs = 60
```

Après modification : `sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart mqtt-bridge`

---

## Dashboard

Page `/dashboard/mqtt` dans l'interface ESS Monitor :

- **Broker** : statut (running/arrêté), uptime, messages reçus
- **Bridge** : connexions local + NanoPi, compteurs messages bidirectionnels, reconnexions
- **Topics bridgés** : tableau récapitulatif des règles (QoS, direction)
- **Explorateur MQTT WebSocket** : connexion live sur `:9001`, abonnement à un topic, affichage temps réel

L'API `/api/v1/mqtt/status` retourne le JSON complet mis à jour toutes les 30s par le monitor agent.

---

## Monitoring systemd

Les deux services reportent leur statut à systemd via `sd_notify` :

```bash
# Vérifier que les deux services sont actifs
systemctl is-active mqtt-broker mqtt-bridge

# Surveiller les redémarrages
journalctl -u mqtt-broker -u mqtt-bridge -f

# Diagnostics bridge (connexion NanoPi)
journalctl -u mqtt-bridge -n 50 | grep -E "Connexion|Déconnexion|Reconnexion"
```

---

## Points de vigilance

1. **Retained messages** : rumqttd persiste en mémoire ET sur disque (`/var/lib/mqtt-broker`). Les retained de Mosquitto ne migrent pas automatiquement — ils seront recréés par les services à leur prochain publish retained (ex: `pvinv_baseline` par energy-manager).

2. **Portal ID** : la valeur `portal_id = "c0619ab9929a"` dans `[mqtt_bridge]` doit correspondre exactement à l'ID Victron GX (visible avec `dbus -y | grep victronenergy` sur NanoPi).

3. **Double abonnement Shelly** : le bridge subscribe `shellypro2pm-ec62608840a4/#` dans les deux sens. Si Shelly est reconfigured pour pointer sur Pi5 local (recommandé), les deux legs du bridge ne créeront pas de boucle car rumqttd détecte les boucles via client_id.

4. **QoS et clean session** : le bridge utilise `clean_session = false` pour les clients persistants, assurant la livraison des messages QoS 1 en cas de reconnexion.
