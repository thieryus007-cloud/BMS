# NanoPi — Configuration Venus OS (Cerbo GX)

Fichiers à copier sur le NanoPi Venus OS pour les deux instances du driver
[dbus-mqtt-battery](https://github.com/mr-manuel/venus-os_dbus-mqtt-battery).

---

## Architecture

```
PC Windows (Rust RS485 natif)
       │
       ▼ MQTT publish (toutes les 1s, retain=true)
FlashMQ — 192.168.1.120:1883 (broker intégré Venus OS)
  santuario/bms/1/venus
  santuario/bms/2/venus
       │
       ▼ subscribe (dbus-mqtt-battery)
dbus-mqtt-battery-41  →  com.victronenergy.battery.mqtt_battery_141
dbus-mqtt-battery-42  →  com.victronenergy.battery.mqtt_battery_142
       │
       ▼ D-Bus Venus OS
  GUI / VRM Portal / systemcalc / hub4control
```

> `dbus-mqtt-battery` fonctionne en mode **MQTT → D-Bus** (abonnement MQTT,
> création d'un service batterie virtuel sur le bus système Venus OS).

---

## Déploiement des configs (à faire depuis le NanoPi)

```bash
# BMS 1 (360Ah — adresse RS485 0x01)
cat > /data/etc/dbus-mqtt-battery-41/config.ini << 'EOF'
[DEFAULT]
logging = WARNING
device_name = MQTT Battery 360Ah
device_instance = 141
timeout = 0

[MQTT]
broker_address = 127.0.0.1
broker_port = 1883
topic = santuario/bms/1/venus
EOF

# BMS 2 (320Ah — adresse RS485 0x02)
cat > /data/etc/dbus-mqtt-battery-42/config.ini << 'EOF'
[DEFAULT]
logging = WARNING
device_name = MQTT Battery 320Ah
device_instance = 142
timeout = 0

[MQTT]
broker_address = 127.0.0.1
broker_port = 1883
topic = santuario/bms/2/venus
EOF

# Redémarrer les deux services
svc -t /service/dbus-mqtt-battery-41
svc -t /service/dbus-mqtt-battery-42

# Vérifier qu'ils sont actifs (doit afficher "up X seconds")
sleep 5
svstat /service/dbus-mqtt-battery-41
svstat /service/dbus-mqtt-battery-42
```

### Pourquoi `broker_address = 127.0.0.1` ?

Le broker FlashMQ tourne **sur le NanoPi lui-même**. Utiliser `127.0.0.1` (loopback)
est plus fiable que l'IP `192.168.1.120` au démarrage, car l'interface réseau
peut ne pas être encore configurée quand les services démarrent.

### Pourquoi `timeout = 0` ?

Désactive la déconnexion automatique si aucune donnée reçue. Le service reste
actif même si le PC Windows n'a pas encore envoyé de données (après un reboot).

---

## Diagnostic — Étapes dans l'ordre

### Étape 1 : Vérifier les services

```bash
svstat /service/dbus-mqtt-battery-41
svstat /service/dbus-mqtt-battery-42
# Résultat attendu : "up X seconds, normally up"
# Si "down" ou redémarre souvent → voir Étape 2
```

### Étape 2 : Voir les logs du service

```bash
tail -50 /var/log/dbus-mqtt-battery-41/current
# Messages attendus après connexion OK :
#   "Connected to MQTT broker"
#   "Subscribed to santuario/bms/1/venus"
#   "Received data for battery 141"
```

### Étape 3 : Vérifier que FlashMQ reçoit les données

```bash
# Souscrire et attendre un message (doit arriver en < 2 secondes si Rust tourne)
mosquitto_sub -h 127.0.0.1 -p 1883 -t "santuario/bms/1/venus" -C 1 -v
# Si rien n'arrive → le PC Windows n'envoie pas encore
# Si message reçu → FlashMQ OK, problème dans dbus-mqtt-battery
```

### Étape 4 : Vérifier la config déployée

```bash
cat /data/etc/dbus-mqtt-battery-41/config.ini
# Doit afficher broker_address = 127.0.0.1 et timeout = 0
```

### Étape 5 : Vérifier le D-Bus (après que le service soit "up")

```bash
# Si le service est bien UP et a reçu des données :
dbus -y com.victronenergy.battery.mqtt_battery_141 / GetValue
# Doit retourner les données batterie, PAS une erreur NameHasNoOwner
```

### Étape 6 : Tester manuellement le script Python

```bash
cd /data/etc/dbus-mqtt-battery-41
python dbus-mqtt-battery.py
# Observer les messages : Connected / Subscribed / Received
# Ctrl+C pour arrêter
```

### Erreur "NameHasNoOwner"

```
dbus.exceptions.DBusException: org.freedesktop.DBus.Error.NameHasNoOwner:
  Could not get owner of name 'com.victronenergy.battery.mqtt_battery_141'
```

**Cause** : Le service dbus-mqtt-battery n'est pas enregistré sur D-Bus.
**Solution** :
1. Vérifier que le service est "up" : `svstat /service/dbus-mqtt-battery-41`
2. Vérifier les logs : `tail -30 /var/log/dbus-mqtt-battery-41/current`
3. Déployer la config correcte (voir section Déploiement ci-dessus)
4. Redémarrer : `svc -t /service/dbus-mqtt-battery-41`

---

## Migration depuis CAN

Lors du passage de `dbus-serialbattery.py can0` vers le flux MQTT du PC Windows,
arrêter le service CAN sur le NanoPi :

```bash
# Arrêter sans désinstaller (récupère ~60 MB RAM + 6% CPU)
svc -d /service/dbus-canbattery.can0

# Vérifier l'état (doit afficher "down")
svstat /service/dbus-canbattery.can0
```

---

## Structure sur le NanoPi

```
/data/etc/
  dbus-mqtt-battery-41/
    config.ini          ← broker=127.0.0.1, topic=santuario/bms/1/venus
    dbus-mqtt-battery.py
  dbus-mqtt-battery-42/
    config.ini          ← broker=127.0.0.1, topic=santuario/bms/2/venus
    dbus-mqtt-battery.py
```

---

## Topics MQTT publiés par le PC Windows

```
santuario/bms/1/venus   → dbus-mqtt-battery-41  (BMS-360Ah, adresse 0x01)
santuario/bms/2/venus   → dbus-mqtt-battery-42  (BMS-320Ah, adresse 0x02)
```

Intervalle de publication : **1 seconde** (`publish_interval_sec = 1` dans Config.toml).
Messages publiés avec **retain=true** → disponibles immédiatement après connexion.

---

## Surveillance ressources NanoPi (BusyBox compatible)

```bash
# Vue CPU + RAM temps réel
top -b -n 1 | head -n 20

# RAM disponible
cat /proc/meminfo | grep -E "MemTotal|MemFree|MemAvailable"
```

### État du système

| Process | RAM | CPU | Notes |
|---|---|---|---|
| `flashmq` | ~49 MB | 0% | Broker MQTT local (port 1883) |
| `dbus-mqtt-battery-41/42` | ~42 MB x2 | 1% | Bridge MQTT → D-Bus |
| `dbus-canbattery.can0` | ~60 MB | 0% | **STOPPÉ** — remplacé par MQTT |
