# Ajout d'un BMS Daly (ex : adresse RS485 `0x03`)

Procédure complète **Pi5** (daly-bms-server) **+ NanoPi/Venus OS** (dbus-mqtt-venus)
pour intégrer un nouveau BMS Daly sur le bus RS485 unifié `/dev/ttyUSB0`.

> **Bonne nouvelle : aucune recompilation nécessaire.** Le support BMS est
> générique (piloté par la config). C'est une opération **config-only** des
> deux côtés. (Contrairement à un nouveau *type* de device qui demande du code.)

Exemple fil rouge : `0x03`, batterie « BMS-628Ah », `mqtt_index = 3`,
`device_instance = 153`.

---

## 0. Choisir les identifiants (à faire une fois)

| Paramètre | Valeur exemple | Règle |
|---|---|---|
| Adresse RS485 (Modbus) | `0x03` | Unique sur le bus `/dev/ttyUSB0` (déjà pris : 0x01,0x02 BMS · 0x05 PRALRAN · 0x07/08/09 ET112) |
| `mqtt_index` | `3` | Topic → `santuario/bms/3/venus` |
| `device_instance` | `153` | **Unique** sur le D-Bus Victron. Suite logique 151→152→**153**. ⚠ NE PAS réutiliser 141/142 (legacy dbus-mqtt-battery) ni 143 (réservé à l'exemple batterie virtuelle agrégée 628Ah) |
| Service D-Bus résultant | `com.victronenergy.battery.mqtt_3` | dérivé de `mqtt_index` |

---

## 1. Matériel — régler l'adresse du BMS

1. Régler l'**adresse Modbus du BMS à 3** (via l'app Daly Bluetooth/Smart BMS,
   ou l'outil série constructeur). Deux BMS ne doivent **jamais** partager la
   même adresse sur le bus.
2. Câbler le BMS sur le bus RS485 partagé `/dev/ttyUSB0` (A/B en parallèle des
   autres BMS, masse commune).
3. (Optionnel, à froid) Vérifier la réponse Modbus **avant** d'intégrer :
   ```bash
   sudo systemctl stop daly-bms          # libère le port
   mbpoll -m rtu -a 3 -b 9600 -t 3:float -r 1 -c 1 /dev/ttyUSB0
   sudo systemctl start daly-bms
   ```

---

## 2. Pi5 — `daly-bms-server`

### 2.1 Éditer `Config.toml` (dépôt)

**a) Ajouter l'adresse à la liste de scan** `[serial].addresses` — ⚠ **étape la
plus souvent oubliée** : sans elle, le BMS n'est jamais interrogé.
```toml
[serial]
addresses = ["0x01", "0x02", "0x03"]   # ← ajouter "0x03"
```

**b) Ajouter le bloc `[[bms]]`** (décommenter celui déjà présent) :
```toml
[[bms]]
address         = "0x03"
name            = "BMS-628Ah"
capacity_ah     = 628.0
max_charge_a    = 200.0
max_discharge_a = 120.0
mqtt_index      = 3
device_instance = 153
```

### 2.2 Déployer la config sur le Pi5

> `deploy-pi5.sh` **n'écrase pas** `/etc/daly-bms/config.toml` s'il existe
> (protège la prod). La copie est donc **manuelle**.

```bash
cd ~/Daly-BMS-Rust
# (commit/push les modifs de Config.toml d'abord, puis sur le Pi5 :)
make sync
sudo cp Config.toml /etc/daly-bms/config.toml
sudo systemctl restart daly-bms
```

### 2.3 Vérifier côté Pi5
```bash
# Le BMS est lu (status RS485)
curl -s http://localhost:8080/api/v1/bms/3/status | jq '.data.soc, .data.dc'
# Métriques en base (clé = adresse)
curl -s 'http://localhost:8080/api/v1/query?query=bms_power' | jq '.data.result[].metric'
# Topic MQTT publié
timeout 5 mosquitto_sub -h 127.0.0.1 -t 'santuario/bms/3/venus' -v
# Santé RS485 (le nouveau doit apparaître, sans timeout)
curl -s http://localhost:8080/api/v1/monitor/rs485-health | jq
```

---

## 3. NanoPi / Venus OS — `dbus-mqtt-venus`

### 3.1 Éditer `nanoPi/config-nanopi.toml` (dépôt)

Ajouter un bloc `[[bms]]` **identique** (mêmes `mqtt_index`/`device_instance`) :
```toml
[[bms]]
address         = "0x03"
name            = "BMS-628Ah"
mqtt_index      = 3
device_instance = 153
capacity_ah     = 628.0
max_charge_a    = 200.0
max_discharge_a = 120.0
```

### 3.2 Déployer la config sur le NanoPi

> **Pas de recompilation** : seul le fichier de config change. (Si tu modifies
> du code Rust un jour → `make install-venus-v7`, et **jamais**
> `target-cpu=native` pour l'armv7, cf. CLAUDE.md §8 SIGILL.)

```bash
# Depuis le Pi5, après make sync :
scp nanoPi/config-nanopi.toml root@192.168.1.120:/data/daly-bms/config.toml
ssh root@192.168.1.120 'svc -t /service/dbus-mqtt-venus'
```

### 3.3 Vérifier côté NanoPi
```bash
# Le nouveau service D-Bus batterie doit apparaître
ssh root@192.168.1.120 'dbus -y | grep battery'
#   attendu : com.victronenergy.battery.mqtt_1 / mqtt_2 / mqtt_3
# Valeurs exposées
ssh root@192.168.1.120 'dbus -y com.victronenergy.battery.mqtt_3 / GetItems | grep -E "Soc|Dc/0/Voltage|DeviceInstance"'
```

Puis dans **VRM / console GX** : la nouvelle batterie apparaît dans la liste des
appareils (instance 153). Rafraîchir si cache.

---

## 4. Bridge Mosquitto — RIEN à faire ✅

La règle egress couvre déjà **tous** les index BMS :
```
topic santuario/bms/# out 1 "" ""
```
`santuario/bms/3/venus` est donc automatiquement bridgé Pi5 → NanoPi. Aucune
modification de `contrib/mosquitto/mosquitto.conf`.

---

## 5. Récapitulatif des fichiers

| Fichier | Modif | Déploiement |
|---|---|---|
| `Config.toml` (Pi5) | `[serial].addresses` + bloc `[[bms]]` | `sudo cp … /etc/daly-bms/config.toml` + restart daly-bms |
| `nanoPi/config-nanopi.toml` | bloc `[[bms]]` | `scp … :/data/daly-bms/config.toml` + `svc -t` |
| `contrib/mosquitto/mosquitto.conf` | — (déjà `bms/# out`) | — |
| Code Rust | — | — (pas de build) |

---

## 6. Checklist finale

- [ ] Adresse Modbus du BMS réglée à `3`, câblé sur `/dev/ttyUSB0`
- [ ] `Config.toml` : `"0x03"` dans `[serial].addresses` **et** bloc `[[bms]]`
- [ ] `device_instance = 153` (unique — pas 141/142/143)
- [ ] `Config.toml` copié vers `/etc/daly-bms/config.toml` + `daly-bms` redémarré
- [ ] `config-nanopi.toml` : bloc `[[bms]]` ajouté, scp + `svc -t`
- [ ] `curl /api/v1/bms/3/status` renvoie des données
- [ ] `dbus -y | grep battery` montre `battery.mqtt_3`
- [ ] Batterie visible dans VRM (instance 153)
- [ ] `make sync` à jour, commit/push de `Config.toml` + `config-nanopi.toml`

---

## 7. Rollback

```bash
# Pi5
sudo cp /etc/daly-bms/config.toml.bak-<date> /etc/daly-bms/config.toml   # si backup
#   ou retirer "0x03" de [serial].addresses + le bloc [[bms]], puis :
sudo systemctl restart daly-bms
# NanoPi (retirer le bloc [[bms]] 0x03, redéployer)
scp nanoPi/config-nanopi.toml root@192.168.1.120:/data/daly-bms/config.toml
ssh root@192.168.1.120 'svc -t /service/dbus-mqtt-venus'
```
Le service D-Bus `battery.mqtt_3` disparaît au redémarrage de `dbus-mqtt-venus`
(un D-Bus ne survit pas au restart). Penser à purger le retained si besoin :
```bash
mosquitto_pub -h 127.0.0.1 -t santuario/bms/3/venus -r -n
ssh root@192.168.1.120 'mosquitto_pub -h localhost -t santuario/bms/3/venus -r -n'
```

---

## 8. Pièges à éviter

| Piège | Conséquence | Solution |
|---|---|---|
| Oublier `"0x03"` dans `[serial].addresses` | BMS jamais interrogé (aucune donnée, mais pas d'erreur visible) | Ajouter à la liste de scan |
| `device_instance` dupliqué (141/142/143) | Conflit D-Bus / batterie fantôme dans VRM | Utiliser **153** |
| Copier `Config.toml` via `deploy-pi5.sh` | Ne s'applique PAS (préservation prod) | `sudo cp` manuel |
| Recompiler « par précaution » | Inutile + risque (armv7 SIGILL si `target-cpu=native`) | Config-only, pas de build |
| Adresse Modbus du BMS pas réglée à 3 | Pas de réponse / collision avec un autre device | Régler via app Daly avant câblage |
| `mqtt_index`/`device_instance` différents entre Pi5 et NanoPi | Topic publié ≠ topic attendu → batterie absente de VRM | Garder **identiques** des deux côtés |
