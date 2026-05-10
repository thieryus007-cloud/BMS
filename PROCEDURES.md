# PROCEDURES.md — Procédures Détaillées

> Lire uniquement quand une procédure spécifique est nécessaire.
> Référence rapide → **CLAUDE.md**.

---

## A. DÉPLOIEMENT NANOPI — PROCÉDURE COMPLÈTE

```bash
# Depuis Pi5
git pull origin <branche>
make build-venus-v7
make install-venus-v7
```

**Ce que fait `install-venus.sh`** :
1. ControlMaster SSH (une seule auth)
2. Crée `/data/daly-bms/` et `/data/etc/sv/dbus-mqtt-venus/`
3. Arrête le service avant copie (évite "dest open Failure")
4. Copie le binaire `dbus-mqtt-venus` (armv7)
5. Copie `config.toml` si absent
6. Copie le script runit `run`
7. Crée symlink `/service/dbus-mqtt-venus`
8. Crée `/data/rc.local` (persistance après reboot/firmware update)
9. Vérifie le démarrage

**Diagnostic NanoPi** :
```bash
ssh root@192.168.1.120
svstat /service/dbus-mqtt-venus
tail -f /var/log/dbus-mqtt-venus/current
dbus -y com.victronenergy.battery.mqtt_1 /Soc GetValue
dbus -y com.victronenergy.battery.mqtt_2 /Soc GetValue
mosquitto_sub -h 127.0.0.1 -p 1883 -t "santuario/#" -v
```

---

## B. DÉPLOIEMENT PI5 — MISE À JOUR BINAIRE

**Cas 1 — Config seule** :
```bash
cd ~/Daly-BMS-Rust
git pull origin <branche>
sudo cp Config.toml /etc/daly-bms/config.toml
sudo systemctl restart daly-bms
journalctl -u daly-bms -f
```

**Cas 2 — Code Rust ou template HTML** :
```bash
cd ~/Daly-BMS-Rust
git pull origin <branche>
make build-arm                    # ~5-10 min
sudo systemctl stop daly-bms
sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/
sudo cp Config.toml /etc/daly-bms/config.toml   # si config aussi modifiée
sudo systemctl start daly-bms
journalctl -u daly-bms -f
```

---

## C. RÉCUPÉRATION APRÈS MISE À JOUR FIRMWARE VENUS OS

Une màj firmware peut supprimer le symlink `/service/dbus-mqtt-venus`.

**Vérification** (NanoPi) :
```bash
svstat /service/dbus-mqtt-venus
ls -la /service/dbus-mqtt-venus
cat /data/rc.local
ls -la /data/daly-bms/dbus-mqtt-venus
```

**Restauration symlink seulement** (binaire présent) :
```bash
ssh root@192.168.1.120
ln -sf /data/etc/sv/dbus-mqtt-venus /service/dbus-mqtt-venus
# Vérifier rc.local :
cat /data/rc.local
# Doit contenir : ln -sf /data/etc/sv/dbus-mqtt-venus /service/dbus-mqtt-venus
# Si absent :
cat > /data/rc.local << 'EOF'
#!/bin/sh
ln -sf /data/etc/sv/dbus-mqtt-venus /service/dbus-mqtt-venus
EOF
chmod +x /data/rc.local
sleep 10
svstat /service/dbus-mqtt-venus
```

**Redéploiement complet** (binaire disparu) :
```bash
cd ~/Daly-BMS-Rust
git pull origin <branche>
make build-venus-v7 && make install-venus-v7
```

---

## D. AJOUTER UN APPAREIL (energy-manager → VENUS OS)

### Étape 1 — Config NanoPi (`/data/daly-bms/config.toml`)

```toml
# Capteur température
[[sensors]]
mqtt_index      = 2
name            = "Eau chaude"
temperature_type = 5   # 0=battery 1=fridge 2=generic 3=room 4=outdoor 5=waterheater
device_instance = 102

# PAC
[[heatpumps]]
mqtt_index      = 2
name            = "PAC LG"
custom_name     = "Chauffe-eau Ballon"   # optionnel, affiché dans Venus GUI/VRM
device_instance = 202

# Switch/ATS
[[switches]]
mqtt_index      = 2
name            = "ATS Groupe"
custom_name     = "Commutation Réseau/Groupe"
device_instance = 302

# Compteur réseau
[[grids]]
mqtt_index      = 2
name            = "Compteur Fronius"
device_instance = 402
service_type    = "grid"   # "grid" ou "acload"
```

Puis : `svc -t /service/dbus-mqtt-venus`

### Étape 2 — energy-manager (Pi5)

Publier via MQTT retained sur broker `192.168.1.120:1883`. Utiliser `bus.publish(MqttOutgoing::retained(...))` dans le module logique correspondant (voir `docs/energy-manager-guide.md` section 9).

Payload JSON capteur température :
```json
{"Temperature": 42.5, "TemperatureType": 5, "Status": 0, "ProductName": "Eau chaude"}
```

Payload JSON switch/ATS :
```json
{"State": 1, "Position": 2, "ProductName": "ATS CHINT"}
```

Payload JSON compteur grid :
```json
{"Ac/L1/Power": 1250.0, "Ac/L1/Voltage": 230.0, "Ac/L1/Current": 5.43}
```

### Étape 3 — Vérification D-Bus
```bash
ssh root@192.168.1.120 "dbus -y | grep victronenergy"
ssh root@192.168.1.120 "dbus -y com.victronenergy.temperature.mqtt_2 / GetItems"
```

---

## E. DIAGNOSTIC ET112 — ADRESSE MODBUS INCORRECTE

Symptôme : dashboard "En attente du premier snapshot..."

```bash
# 1. Vérifier les logs
journalctl -u daly-bms --since "2 minutes ago" | grep -E "modbus|timeout|CRC"

# 2. Scanner les adresses (arrêter daly-bms OBLIGATOIRE)
sudo systemctl stop daly-bms
mbpoll -m rtu -a 1:15 -b 9600 -t 3:float -r 1 -c 1 /dev/ttyUSB0
# Les adresses qui répondent affichent ~230.x V (tension réseau)
sudo systemctl start daly-bms

# 3. Mettre à jour Config.toml + /etc/daly-bms/config.toml avec la bonne adresse
```

> **mbpoll** : `-t 3` = input registers (FC=04). ET112 sort d'usine à l'adresse `0x01`.
> Identifier l'adresse aussi via logiciel Carlo Gavazzi UCS sur PC Windows.

---

## F. PERSISTANCE PRODUCTION SOLAIRE (pvinv_baseline)

**Problème** : après reboot Pi5, `pvinv_baseline` est perdue → TodaysYield repart à 0.

**Solution** : MQTT retained sur `santuario/persist/pvinv_baseline` (Mosquitto persistence=true + volume Docker). energy-manager restaure automatiquement au démarrage via le topic retained.

**Vérification** :
```bash
mosquitto_sub -h localhost -p 1883 -t 'santuario/persist/pvinv_baseline' -C 1
```

**Récupération manuelle si reset mid-journée** :
```bash
# 1. Sur NanoPi — lire cumul actuel PVInverter
dbus -y com.victronenergy.pvinverter.cgwacs_ttyUSB0_mb2 /Ac/Energy/Forward GetValue
# 2. Calculer : pvinv_today = total_victron_gui - mppt_today
# 3. Injecter manuellement le baseline via mosquitto_pub :
```
```bash
# baseline = cumul_total - production_jour_courant_pvinv
mosquitto_pub -h 192.168.1.120 -t 'santuario/persist/pvinv_baseline' -m '587.2' -r
global.set('total_yield_today', mpptToday + pvinvToday);
return null;
```

---

## G. CHECKLIST MAINTENANCE OPÉRATIONNELLE

```bash
# Pi5 — état global
systemctl status daly-bms
journalctl -u daly-bms --since "1 hour ago" | grep -E "ERROR|WARN"
docker compose ps
docker compose logs --since 1h | grep -i error

# NanoPi — état Venus bridge
ssh root@192.168.1.120 "svstat /service/dbus-mqtt-venus"
ssh root@192.168.1.120 "tail -20 /var/log/dbus-mqtt-venus/current"

# Valeurs clés D-Bus
ssh root@192.168.1.120 "dbus -y com.victronenergy.battery.mqtt_1 /Soc GetValue"
ssh root@192.168.1.120 "dbus -y com.victronenergy.battery.mqtt_2 /Soc GetValue"
ssh root@192.168.1.120 "dbus -y com.victronenergy.meteo /TodaysYield GetValue"
ssh root@192.168.1.120 "dbus -y com.victronenergy.pvinverter.mqtt_7 /Ac/Power GetValue"
ssh root@192.168.1.120 "dbus -y com.victronenergy.heatpump.mqtt_8 /Ac/Power GetValue"
```

**Redémarrage propre** :
```bash
make down && make up
sudo systemctl restart daly-bms
ssh root@192.168.1.120 "svc -t /service/dbus-mqtt-venus"
```

---

## H. SAUVEGARDE CONFIG NANOPI

```bash
# Depuis Pi5 — sauvegarder config NanoPi dans le repo
scp root@192.168.1.120:/data/daly-bms/config.toml nanoPi/config-nanopi.toml
git add nanoPi/config-nanopi.toml
git commit -m "chore(nanopi): backup config.toml"
git push -u origin <branche>
```

---

## I. NOTES TECHNIQUES

**Architecture température** : température extérieure via `temperature.mqtt_1` (type 4=Outdoor) uniquement.
`/ExternalTemperature` absent de `meteo` — ne pas réajouter dans `MeteoValues` ni `meteo_service.rs`.

**Menu Setup Venus OS absent pour pvinverter** : nécessite `/AllowedRoles` et `/Role` dans le service D-Bus
(ajoutés dans `pvinverter_service.rs`).

**`OwnedValue` non Clone dans zvariant 4.2.0** : utiliser `DbusValueKind` enum (Clone-able) qui calcule
`OwnedValue` à la demande.

**MPPT Solar VE.CAN** : race condition au boot possible — `svc -t /service/vedirect-interface.ttyS1`.

**Fréquences de polling** :
- Open-Meteo API : 5 min (limite API)
- Keepalive Venus OS : 5 min ("Dernière màj" dans widget Victron)
