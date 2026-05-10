# ATS CHINT NXZB/NXZBN — Maintenance Opérationnelle

> Document de référence pour l'exploitation, la surveillance et le dépannage
> du commutateur automatique de sources CHINT intégré dans le système ESS Santuario.
>
> Dernière mise à jour : 2026-04-06

---

## 1. ARCHITECTURE D'INTÉGRATION

```
┌─────────────────────────────────────────────────────────────────────┐
│  Bus RS485 unifié  /dev/ttyUSB0  (9600-8N1)                        │
│                                                                     │
│  Addr 0x01 → BMS-360Ah (Daly)                                      │
│  Addr 0x02 → BMS-320Ah (Daly)                                      │
│  Addr 0x05 → Irradiance PRALRAN                                     │
│  Addr 0x06 → ATS CHINT NXZB  ◄── ici                              │
│  Addr 0x07 → ET112 Micro-Onduleurs                                  │
│  Addr 0x08 → ET112 PAC Chauffe-eau                                  │
│  Addr 0x09 → ET112 PAC Climatisation                                │
└──────────────────┬──────────────────────────────────────────────────┘
                   │ Modbus RTU FC=03 (polling 5s)
                   │ Modbus RTU FC=06 (commandes à la demande)
                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  daly-bms-server (Pi5 — systemd)                                    │
│                                                                     │
│  ATS polling loop (5s)                                              │
│    ├── API REST  GET  /api/v1/ats/status                            │
│    ├── API REST  POST /api/v1/ats/remote_on|off|force_*             │
│    ├── Dashboard SSR  /dashboard/ats  (schéma unifilaire SVG)       │
│    └── MQTT publish   santuario/switch/1/venus  (retain=true)       │
└──────────────────┬──────────────────────────────────────────────────┘
                   │ MQTT → broker NanoPi 192.168.1.120:1883
                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  dbus-mqtt-venus (NanoPi)                                           │
│                                                                     │
│  com.victronenergy.switch.mqtt_1  (device_instance = 60)           │
│    ├── /Position   : 0 = AC1/Réseau, 1 = AC2/Onduleur              │
│    ├── /State      : 0 = inactif, 1 = actif, 2 = alerte            │
│    ├── /Connected  : 0 ou 1                                         │
│    ├── /CustomName : "ATS CHINT NXZB"                               │
│    └── /ProductName: "ATS CHINT"                                    │
└─────────────────────────────────────────────────────────────────────┘
                   │ D-Bus Venus OS
                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  Victron GX (NanoPi) → Venus OS GUI + VRM Portal                   │
│  Affichage : "ATS CHINT NXZB" dans liste des équipements           │
│  Position source visible dans l'énergie (AC1 / AC2)                │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 2. CONFIGURATION MATÉRIELLE

### Câblage RS485

| Fil | Signal | Connecteur ATS |
|-----|--------|----------------|
| A+  | Data + | Borne A (RS485) |
| B-  | Data − | Borne B (RS485) |
| GND | Masse  | Borne GND (si présente) |

> Câbler sur le même bus que les autres appareils RS485.
> Terminaison 120 Ω en bout de ligne si l'ATS est le dernier appareil.

### Configuration de l'ATS (bouton Setup physique)

| Paramètre | Valeur requise | Défaut usine |
|-----------|----------------|--------------|
| Adresse Modbus | 6 | 1 |
| Baud rate | 9600 | 9600 |
### | **Parité** | **None (8N1)** | **Even (8E1)** |      >>>> **None (8N1)** Déja modifié

> ⚠ **CRITIQUE** : La parité DOIT être configurée à **None** sur l'ATS.
> Le bus unifié Pi5 tourne en 8N1. Si l'ATS reste en Even, il ne répondra pas.

### Configurer la parité via Modbus (si accès temporaire en Even)

Si l'ATS est encore en parité Even, le reconfigurer via un PC Windows
avec le logiciel Carlo Gavazzi UCS ou via `mbpoll` sur un port dédié temporaire :

```bash
# Sur Pi5 — arrêter daly-bms avant d'utiliser mbpoll
sudo systemctl stop daly-bms

# Écrire parité = None (0x0000) dans le registre 0x000E
mbpoll -m rtu -a 6 -b 9600 -P E -t 4 -r 0x000E /dev/ttyUSB0 0

# Redémarrer
sudo systemctl start daly-bms
```

---

## 3. REGISTRES MODBUS

### Lecture FC=03

#### Bloc A — Tensions et identification (0x0006, 13 registres)

| Registre | Nom | Unité | Description |
|----------|-----|-------|-------------|
| 0x0006 | v1a | V | Tension Source 1 phase A |
| 0x0007 | v1b | V | Tension Source 1 phase B |
| 0x0008 | v1c | V | Tension Source 1 phase C |
| 0x0009 | v2a | V | Tension Source 2 phase A |
| 0x000A | v2b | V | Tension Source 2 phase B |
| 0x000B | v2c | V | Tension Source 2 phase C |
| 0x000C | sw_version | — | Version logicielle (÷100) |
| 0x000D | freq | — | Fréquences hi=f1/lo=f2 (MN only) |
| 0x000E | parity_code | — | 0=None, 1=Odd, 2=Even |
| 0x000F | max1_v | V | Tension max enregistrée Source 1 |
| 0x0010 | — | — | (réservé) |
| 0x0011 | — | — | (réservé) |
| 0x0012 | max2_v | V | Tension max enregistrée Source 2 |

#### Bloc C — Compteurs (0x0015, 3 registres)

| Registre | Nom | Description |
|----------|-----|-------------|
| 0x0015 | cnt1 | Compteur commutations Source1→Source2 |
| 0x0016 | cnt2 | Compteur commutations Source2→Source1 |
| 0x0017 | runtime_h | Durée de fonctionnement totale (heures) |

#### Bloc D — Statut (0x004F, 2 registres)

| Registre | Nom | Description |
|----------|-----|-------------|
| 0x004F | pwr_status | Bitfield statut tensions par phase |
| 0x0050 | sw_status | Bitfield statut commutation |

**Décodage pwr_status (0x004F)** — 2 bits par phase, 3 phases × 2 sources :

| Bits | Phase | Source | Valeur 0 | Valeur 1 | Valeur 2 | Valeur 3 |
|------|-------|--------|----------|----------|----------|----------|
| 1:0 | A | 1 | Normal | Sous-tension | Sur-tension | Erreur |
| 3:2 | B | 1 | Normal | Sous-tension | Sur-tension | Erreur |
| 5:4 | C | 1 | Normal | Sous-tension | Sur-tension | Erreur |
| 7:6 | A | 2 | Normal | Sous-tension | Sur-tension | Erreur |
| 9:8 | B | 2 | Normal | Sous-tension | Sur-tension | Erreur |
| 11:10 | C | 2 | Normal | Sous-tension | Sur-tension | Erreur |

**Décodage sw_status (0x0050)** :

| Bit | Nom | 0 | 1 |
|-----|-----|---|---|
| 0 | sw_mode | Manuel | Auto |
| 3 | sw1_bit | SW1 fermé | SW1 ouvert |
| 4 | sw2_bit | SW2 fermé | SW2 ouvert |
| 7:5 | fault | 0=Aucun, 1=Incendie, 2=Surcharge moteur, 3=Disj. I, 4=Disj. II, 5=Fermeture anormale, 6=Phase anormale I, 7=Phase anormale II |
| 8 | remote | Télécommande Off | Télécommande On |

**Position de commutation** (déduite des bits 3 et 4) :

| sw1_bit | sw2_bit | État |
|---------|---------|------|
| 0 | 0 | Position centrale / neutre (middle-off) |
| 0 | 1 | **SW1 fermé → Source 1 (Onduleur) active** |
| 1 | 0 | **SW2 fermé → Source 2 (Réseau) active** |
| 1 | 1 | Les deux ouverts (transition en cours) |

#### Bloc E — Config Modbus (0x0100, 2 registres)

| Registre | Nom | Description |
|----------|-----|-------------|
| 0x0100 | modbus_addr | Adresse Modbus configurée sur l'ATS |
| 0x0101 | modbus_baud | Code baud (0=4800, 1=9600, 2=19200, 3=38400) |

#### Bloc F — Paramètres MN uniquement (0x2065, 9 registres)

> Lire ce bloc en premier pour détecter le modèle : succès = MN, timeout = BN.

| Registre | Nom | Unité | Description |
|----------|-----|-------|-------------|
| 0x2065 | uv1 | V | Seuil sous-tension Source 1 |
| 0x2066 | uv2 | V | Seuil sous-tension Source 2 |
| 0x2067 | ov1 | V | Seuil sur-tension Source 1 |
| 0x2068 | ov2 | V | Seuil sur-tension Source 2 |
| 0x2069 | t1 | s | Délai commutation Source1→Source2 |
| 0x206A | t2 | s | Délai retour Source1 |
| 0x206B | t3 | s | Délai commutation Source2→Source1 |
| 0x206C | t4 | s | Délai retour Source2 |
| 0x206D | op_mode | — | Mode : 0=Auto-réarm, 1=Auto-no-réarm, 2=Secours, 3=Générateur, 4=Gén-no-réarm, 5=Gén-secours |

### Écriture FC=06

| Registre | Valeur | Commande | Prérequis |
|----------|--------|----------|-----------|
| 0x2800 | 0x0004 | Activer télécommande | — |
| 0x2800 | 0x0000 | Désactiver télécommande | — |
| 0x2700 | 0x0000 | Forcer Source 1 (Onduleur) | Télécommande active |
| 0x2700 | 0x00AA | Forcer Source 2 (Réseau) | Télécommande active |
| 0x2700 | 0x00FF | Forcer double déclenché | Télécommande active |

> **Ordre obligatoire pour forçage** :
> 1. `POST /api/v1/ats/remote_on` (activer télécommande)
> 2. `POST /api/v1/ats/force_source1` ou `force_source2` ou `force_double`
> 3. `POST /api/v1/ats/remote_off` (rendre l'ATS en Auto)

---

## 4. INTERFACES DE CONTRÔLE

### 4a. Dashboard Web (Pi5)

URL : `http://192.168.1.141:8080/dashboard/ats`

**Panneau gauche — Schéma unifilaire SVG** :
- Source 1 (Onduleur) avec tension phase A
- Source 2 (Réseau) avec tension phase A
- SW1 et SW2 : FERMÉ (vert) / OUVERT (rouge)
- Mode AUTO / MANUEL
- Source active alimentant la charge
- Compteurs de commutations et runtime

**Panneau droit — État détaillé** :
- Toutes les tensions par phase (3 phases × 2 sources)
- Code de défaut et statut
- Paramètres MN : seuils UV/OV, délais T1-T4
- Fréquences, version logicielle, adresse Modbus

**Commandes disponibles** (boutons) :
- Télécommande ON / OFF
- Forcer Onduleur (Source 1)
- Forcer Réseau (Source 2)
- Forcer Double Déclenché

> La page se rafraîchit automatiquement toutes les **3 secondes** via polling JS.
> Les boutons de commande envoient immédiatement la commande Modbus FC=06.

### 4b. API REST (Pi5)

```bash
# Lecture état ATS
curl http://192.168.1.141:8080/api/v1/ats/status

# Activer télécommande
curl -X POST http://192.168.1.141:8080/api/v1/ats/remote_on

# Désactiver télécommande (retour en Auto)
curl -X POST http://192.168.1.141:8080/api/v1/ats/remote_off

# Forcer sur Source 1 (Onduleur)
curl -X POST http://192.168.1.141:8080/api/v1/ats/force_source1

# Forcer sur Source 2 (Réseau)
curl -X POST http://192.168.1.141:8080/api/v1/ats/force_source2

# Forcer double déclenché
curl -X POST http://192.168.1.141:8080/api/v1/ats/force_double
```

### 4c. Venus OS GUI (NanoPi)

L'ATS apparaît sous le nom **"ATS CHINT NXZB"** dans la liste des équipements.

Chemin VRM : `Device list → Switches → ATS CHINT NXZB (instance 60)`

Valeurs visibles dans Venus OS :
- `/Position` : `AC Input 1` (Réseau) ou `AC Input 2` (Onduleur)
- `/State` : `Inactive` (0) / `Active` (1) / `Alerted` (2)
- `/Connected` : `Connected` (1) / `Disconnected` (0)
- `/CustomName` : "ATS CHINT NXZB"

---

## 5. DIAGNOSTIC ET SURVEILLANCE

### 5a. Vérification état (Pi5)

```bash
# Logs du service en direct
journalctl -u daly-bms -f | grep -i ats

# Vérifier que le polling tourne
journalctl -u daly-bms --since "5 minutes ago" | grep -i "ATS\|0x06"

# Appel API direct
curl -s http://192.168.1.141:8080/api/v1/ats/status | python3 -m json.tool
```

### 5b. Vérification MQTT (Pi5 ou NanoPi)

```bash
# Voir le payload ATS publié
mosquitto_sub -h 192.168.1.120 -p 1883 -t 'santuario/switch/1/venus' -v

# Résultat attendu :
# santuario/switch/1/venus {"Position":0,"State":1,"ProductName":"ATS CHINT","CustomName":"ATS CHINT NXZB"}
```

### 5c. Vérification D-Bus Venus OS (NanoPi)

```bash
ssh root@192.168.1.120

# Service présent ?
dbus -y | grep switch

# Valeurs D-Bus
dbus -y com.victronenergy.switch.mqtt_1 /Position GetValue
dbus -y com.victronenergy.switch.mqtt_1 /State GetValue
dbus -y com.victronenergy.switch.mqtt_1 /Connected GetValue
dbus -y com.victronenergy.switch.mqtt_1 / GetItems
```

### 5d. Test Modbus direct (Pi5 — STOP service avant)

```bash
# Arrêter le service pour libérer le port
sudo systemctl stop daly-bms

# Lire les tensions (registre 0x0006, 6 regs) depuis adresse 6
mbpoll -m rtu -a 6 -b 9600 -P N -t 3 -r 6 -c 6 /dev/ttyUSB0

# Lire le statut de commutation (0x0050)
mbpoll -m rtu -a 6 -b 9600 -P N -t 3 -r 0x0050 -c 1 /dev/ttyUSB0

# Redémarrer
sudo systemctl start daly-bms
```

---

## 6. DÉPANNAGE

### Problème : "Aucune donnée ATS" dans le dashboard

**Symptôme** : `/api/v1/ats/status` retourne 404, logs montrent des timeouts.

**Causes possibles** :

| Cause | Diagnostic | Solution |
|-------|------------|----------|
| ATS encore en parité Even | `mbpoll ... -P E` répond, `... -P N` ne répond pas | Configurer parité None sur l'ATS |
| Adresse Modbus incorrecte | Scanner toutes les adresses | Voir §6.1 |
| Câble RS485 débranché | Aucun appareil ne répond | Vérifier câble A/B |
| ATS hors tension | LED verte éteinte | Alimenter l'ATS |
| Terminaison 120Ω manquante | Réponses intermittentes | Ajouter résistance en bout de ligne |

#### 6.1 Scanner l'adresse réelle de l'ATS

```bash
sudo systemctl stop daly-bms

# Scanner toutes les adresses 1-15
mbpoll -m rtu -a 1:15 -b 9600 -P N -t 3 -r 6 -c 1 /dev/ttyUSB0
# → L'adresse qui retourne ~230 V (tension réseau) est l'adresse de l'ATS

sudo systemctl start daly-bms
```

Si une adresse répond, mettre à jour `/etc/daly-bms/config.toml` :
```toml
[ats]
address = <nouvelle_adresse>
```
Puis `sudo systemctl restart daly-bms`.

### Problème : Commandes ignorées (pas d'effet)

**Cause la plus fréquente** : Télécommande non activée avant le forçage.

**Solution** :
```bash
# Toujours dans cet ordre :
curl -X POST http://192.168.1.141:8080/api/v1/ats/remote_on
# Attendre 1 seconde
curl -X POST http://192.168.1.141:8080/api/v1/ats/force_source1
# Quand terminé, repasser en auto :
curl -X POST http://192.168.1.141:8080/api/v1/ats/remote_off
```

### Problème : ATS absent du Venus OS GUI

**Causes** :

1. **MQTT non publié** — vérifier `santuario/switch/1/venus` sur le broker
2. **dbus-mqtt-venus ne tourne pas** — `svstat /service/dbus-mqtt-venus` sur NanoPi
3. **`[[switches]]` absent de config-nanopi.toml** — vérifier le fichier sur NanoPi

**Vérification config NanoPi** :
```bash
ssh root@192.168.1.120 "cat /data/daly-bms/config.toml" | grep -A 5 switches
```

Doit contenir :
```toml
[[switches]]
mqtt_index      = 1
name            = "ATS CHINT"
custom_name     = "ATS CHINT NXZB"
device_instance = 60
```

Si absent, ajouter et redémarrer :
```bash
scp nanoPi/config-nanopi.toml root@192.168.1.120:/data/daly-bms/config.toml
ssh root@192.168.1.120 "svc -t /service/dbus-mqtt-venus"
```

### Problème : Code de défaut persistant

| Code défaut | Signification | Action |
|-------------|---------------|--------|
| Interconnexion incendie | Entrée incendie déclenchée | Vérifier/réinitialiser détecteur incendie |
| Surcharge moteur | Moteur ATS en surcharge | Inspection mécanique — contacter maintenance |
| Disjonction I (Onduleur) | Disjoncteur côté Onduleur déclenché | Réarmer disjoncteur aval onduleur |
| Disjonction II (Réseau) | Disjoncteur côté Réseau déclenché | Réarmer disjoncteur aval réseau |
| Fermeture anormale | Fermeture non commandée | Inspection électrique urgente |
| Phase anormale I/II | Anomalie séquence phases | Vérifier rotation phases source |

---

## 7. PROCÉDURES D'EXPLOITATION

### Basculement manuel d'urgence (Réseau → Onduleur)

```bash
# Via API
curl -X POST http://192.168.1.141:8080/api/v1/ats/remote_on
curl -X POST http://192.168.1.141:8080/api/v1/ats/force_source1

# Vérifier position
curl -s http://192.168.1.141:8080/api/v1/ats/status | python3 -c \
  "import sys,json; d=json.load(sys.stdin)['data']; print(d['active_source'], d['sw1_closed'], d['sw2_closed'])"
```

### Retour en mode automatique

```bash
curl -X POST http://192.168.1.141:8080/api/v1/ats/remote_off
```

### Test hebdomadaire de commutation

```bash
echo "=== Test commutation ATS ===" 
echo "Source initiale :"
curl -s http://192.168.1.141:8080/api/v1/ats/status | python3 -c \
  "import sys,json; d=json.load(sys.stdin)['data']; print(f\"  Source active : {d['active_source']}, SW1={d['sw1_closed']}, SW2={d['sw2_closed']}\")"

echo "Activation télécommande + forçage Source 1..."
curl -sX POST http://192.168.1.141:8080/api/v1/ats/remote_on
sleep 2
curl -sX POST http://192.168.1.141:8080/api/v1/ats/force_source1
sleep 3

echo "Après forçage Source 1 :"
curl -s http://192.168.1.141:8080/api/v1/ats/status | python3 -c \
  "import sys,json; d=json.load(sys.stdin)['data']; print(f\"  Source active : {d['active_source']}, SW1={d['sw1_closed']}, SW2={d['sw2_closed']}\")"

echo "Retour Auto..."
curl -sX POST http://192.168.1.141:8080/api/v1/ats/remote_off
sleep 3
echo "Fin test."
curl -s http://192.168.1.141:8080/api/v1/ats/status | python3 -c \
  "import sys,json; d=json.load(sys.stdin)['data']; print(f\"  Source finale : {d['active_source']}, Défaut : {d['fault']}\")"
```

---

## 8. CHECKLIST DE DÉPLOIEMENT INITIAL

### Sur Pi5

- [ ] ATS câblé sur le bus RS485 `/dev/ttyUSB0` (même bus que BMS/ET112)
- [ ] Parité ATS configurée à **None** (8N1) — registre 0x000E = 0
- [ ] Adresse Modbus ATS = 6 (ou adapter `[ats].address` dans config.toml)
- [ ] `Config.toml` → `[ats]` avec `enabled = true`
- [ ] Copier vers production : `sudo cp Config.toml /etc/daly-bms/config.toml`
- [ ] Recompiler si besoin : `make build-arm`
- [ ] Déployer : `sudo cp target/aarch64.../daly-bms-server /usr/local/bin/`
- [ ] Redémarrer : `sudo systemctl restart daly-bms`
- [ ] Vérifier logs : `journalctl -u daly-bms -f | grep -i ats`
- [ ] Tester API : `curl http://192.168.1.141:8080/api/v1/ats/status`
- [ ] Tester dashboard : `http://192.168.1.141:8080/dashboard/ats`

### Sur NanoPi

- [ ] `[[switches]]` présent dans `/data/daly-bms/config.toml` (mqtt_index=1, device_instance=60)
- [ ] `svc -t /service/dbus-mqtt-venus` pour redémarrer le bridge
- [ ] Vérifier D-Bus : `dbus -y | grep switch`
- [ ] Vérifier dans Venus OS GUI : Device list → Switches → "ATS CHINT NXZB"

---

## 9. ÉTAT NOMINAL (LOGS ATTENDUS)

```
INFO daly_bms_server::ats::poll: ATS CHINT polling démarré (bus RS485 unifié) addr=0x06 name=ATS CHINT NXZB
INFO daly_bms_server::ats::poll: Modèle ATS détecté addr=0x06 model=MN
INFO daly_bms_server::bridges::mqtt: ATS CHINT publié → santuario/switch/1/venus
```

**Payload MQTT nominal** :
```json
{
  "Position": 0,
  "State": 1,
  "ProductName": "ATS CHINT",
  "CustomName": "ATS CHINT NXZB"
}
```
(`Position=0` = Source 2/Réseau active, `State=1` = actif)

---

*Document généré automatiquement — maintenir à jour après chaque modification de l'intégration ATS.*
