# PLAN PROJET : Infrastructure Thread redondante avec XIAO nRF54LM20A et Homey Pro Self-Hosted

## 1. Objectif et Architecture

L'objectif est de créer un réseau Thread robuste et redondant avec des XIAO nRF54LM20A, contrôlant chacun un Toshiba Shorei Edge via UART, et intégré à Homey Pro Self-Hosted sur Mac Mini via **Matter over Thread**.

**Architecture corrigée :**
- **XIAO #1** : Branché en USB-C au Mac Mini. Il est flashé en **RCP (Radio Co-Processor)** et sert de radio physique à l'**OpenThread Border Router (OTBR)** qui tourne sur le Mac Mini.
- **XIAO #2, #3, #4** : Routeurs Thread autonomes (FTD), chacun contrôlant un Toshiba Shorei Edge via UART. Ils exposent les climatiseurs comme des appareils **Matter over Thread**.
- **Mac Mini** : Héberge Homey Pro Self-Hosted **et** l'OTBR (via Docker). L'OTBR fait le pont entre le réseau IP local et le réseau Thread maillé.
- **Homey** : Découvre et contrôle les appareils Matter via l'OTBR local (IP → Thread).

**Avantage clé :** Homey n'a pas besoin de radio Thread intégrée. Le XIAO#1 en RCP + l'OTBR sur le Mac Mini assurent le pont. Le mesh Thread entre les 4 XIAO garantit la redondance : si un XIAO de climatiseur tombe, les autres maintiennent le réseau.

---

## 2. Inventaire du Matériel

| Qté | Matériel | Rôle |
|-----|----------|------|
| 4x | XIAO nRF54LM20A Sense | #1 = RCP Border Router ; #2-4 = Routeurs Thread + Matter |
| 3x | Toshiba Shorei Edge (port CN22) | Appareils contrôlés |
| 3x | Level Shifter 3.3V ↔ 5V | Adaptation UART |
| 1x | Mac Mini | Homey Pro Self-Hosted + OTBR (Docker) |
| 1x | Câble USB-C actif (si longueur > 2m) | Pour positionner le XIAO#1 stratégiquement |
| 1x | Réseau local stable | Wi-Fi / Ethernet |

> **Note :** Si vous n'avez que 3 XIAO, vous ne pourrez contrôler que 2 Toshiba (1 XIAO étant requis pour le Border Router).

---

## 3. Schéma de l'Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      MAC MINI (Local)                       │
│  ┌─────────────────────┐    ┌───────────────────────────┐ │
│  │  Homey Pro          │◄──►│  OpenThread Border Router │ │
│  │  (Self-Hosted)      │ IP │  (Docker / Service local) │ │
│  └─────────────────────┘    └─────────────┬─────────────┘ │
│                                           │ USB-C (tty)   │
└───────────────────────────────────────────┼───────────────┘
                                            │
                                            ▼
                                    ┌──────────────┐
                                    │  XIAO #1     │
                                    │  (RCP Mode)  │
                                    │  Border Router│
                                    │  Radio       │
                                    └──────┬───────┘
                                           │ Thread 802.15.4
                              ┌────────────┼────────────┐
                              ▼            ▼            ▼
                        ┌────────┐   ┌────────┐   ┌────────┐
                        │ XIAO #2│   │ XIAO #3│   │ XIAO #4│
                        │Router  │   │Router  │   │Router  │
                        │ +Matter│   │ +Matter│   │ +Matter│
                        └───┬────┘   └───┬────┘   └───┬────┘
                            │ UART       │ UART       │ UART
                            ▼            ▼            ▼
                        ┌────────┐   ┌────────┐   ┌────────┐
                        │Toshiba │   │Toshiba │   │Toshiba │
                        │  #1    │   │  #2    │   │  #3    │
                        └────────┘   └────────┘   └────────┘
```

---

## 4. Phase 1 : Préparation de l'Environnement

### 4.1. Mac Mini
1. Installer **Docker Desktop** (ou OrbStack) pour faire tourner l'OTBR.
2. Vérifier que le Mac Mini est accessible sur le réseau local (IP fixe recommandée).

### 4.2. VS Code + nRF Connect SDK
1. Installer VS Code et l'extension **nRF Connect for VS Code Extension Pack**.
2. Installer le **nRF Connect SDK (NCS)** complet (pas le Bare Metal).
3. Vérifier la détection des ports série USB des XIAO.

---

## 5. Phase 2 : XIAO #1 — Flashage en RCP (Radio Co-Processor)

Le XIAO#1 ne contrôle pas de Toshiba. Il est dédié à la radio du Border Router.

1. **Ouvrir l'exemple Coprocessor** dans le NCS :
   ```bash
   cd nrf/samples/openthread/coprocessor
   west build -b xiao_nrf54lm20a_sense -- -DCONFIG_OPENTHREAD_COPROCESSOR_RCP=y
   ```
2. **Flasher** :
   ```bash
   west flash
   ```
3. **Vérifier** : Connectez le XIAO#1 en USB-C au Mac Mini. Il doit apparaître comme un port série (`/dev/tty.usbmodem*` ou `/dev/ttyACM0`).

> **Note :** Si le board `xiao_nrf54lm20a_sense` n'est pas reconnu par `west boards`, utilisez le board Nordic générique avec un overlay DeviceTree.

---

## 6. Phase 3 : OpenThread Border Router (OTBR) sur Mac Mini

L'OTBR fait le pont entre le réseau IP (Homey) et le réseau Thread (XIAO).

### 6.1. Lancer l'OTBR via Docker

```bash
# Identifier le port USB du XIAO#1 (ex: /dev/tty.usbmodem1234561)
ls /dev/tty.usbmodem*

# Lancer le conteneur OTBR avec accès au port USB
docker run -d --name otbr \
  --privileged \
  --network host \
  -v /dev/tty.usbmodem1234561:/dev/radio \
  -e OTBR_RADIO_URL=spinel+hdlc+uart:///dev/radio?uart-baudrate=1000000 \
  -e OTBR_WEB_GUI=1 \
  openthread/otbr:latest
```

### 6.2. Vérifier l'OTBR
1. Accéder à l'interface web de l'OTBR : `http://localhost:8080`
2. Vérifier que le **XIAO#1 (RCP)** est détecté comme "Radio" et que le status est "Active".
3. Noter le **Active Dataset** généré par l'OTBR (menu "Form" puis copier la chaîne hex).

### 6.3. Configuration réseau
- L'OTBR publie le réseau Thread sur le LAN via **mDNS** (service `_meshcop._udp`).
- Homey (sur le même Mac Mini) verra ce service mDNS automatiquement.

---

## 7. Phase 4 : XIAO #2, #3, #4 — Firmware Routeur Thread + Matter

Ces 3 XIAO ont le même firmware. Chacun :
- Rejoint le réseau Thread créé par l'OTBR.
- Contrôle un Toshiba via UART.
- Expose le climatiseur comme un appareil Matter (clusters Thermostat, On/Off, Fan Control).

### 7.1. Compiler le firmware Matter
```bash
cd nrf/samples/matter/template  # Ou matter-thermostat si disponible
west build -b xiao_nrf54lm20a_sense
```

### 7.2. Flasher (répéter pour les 3 XIAO)
```bash
west flash --runner jlink
```

---

## 8. Phase 5 : Configuration du Réseau Thread

### 8.1. Méthode recommandée : Commissioning via OTBR (le plus sûr)

Au lieu de copier le dataset manuellement, utilisez le **Thread Commissioning** intégré à l'OTBR :

1. Sur l'interface web de l'OTBR (`http://localhost:8080`), cliquez sur **"Join"** ou **"Commission"**.
2. L'OTBR génère un **Joiner Credential** (code PIN).
3. Sur chaque XIAO (#2, #3, #4), activez le mode Joiner :
   ```bash
   > ifconfig up
   > thread joiner start <PIN_DE_LOTBR>
   ```
4. Le XIAO rejoint automatiquement le réseau Thread et reçoit le dataset actif.

### 8.2. Méthode manuelle (fallback)

Si le commissioning automatique échoue :

1. **Récupérer le dataset** depuis l'OTBR :
   ```bash
   # Sur le conteneur OTBR
   docker exec -it otbr ot-ctl dataset active -x
   ```
2. **Injecter sur chaque XIAO** :
   ```bash
   > dataset set active <DATASET_HEX>
   > dataset commit active
   > ifconfig up
   > thread start
   ```
3. **Vérifier** :
   ```bash
   > state
   # Doit afficher : router (ou child temporairement)
   > router table
   # Doit lister les autres XIAO
   ```

---

## 9. Phase 6 : Interface UART et Firmware Matter

Cette partie est identique pour les XIAO #2, #3, #4.

### 9.1. Câblage (Level Shifter obligatoire)

| Level Shifter | Connexion |
|---------------|-----------|
| HV (5V) | Alimentation Toshiba (CN22) |
| LV (3.3V) | Pin 3.3V du XIAO |
| HV1 / LV1 | Ligne TX (Toshiba → XIAO) |
| HV2 / LV2 | Ligne RX (XIAO → Toshiba) |
| GND | Masse commune |

### 9.2. Protocole Toshiba Shorei Edge (CN22)

**⚠️ À documenter impérativement avant de coder :**
- Baud rate (probablement 9600 ou 2400)
- Format (8N1, 7E1...)
- Structure des trames (header, commande, données, checksum)
- Mapping des commandes : ON/OFF, température consigne, mode, vitesse ventilateur

**Recommandation** : Sniffer le bus UART avec un analyseur logique entre le contrôleur d'origine et le Toshiba pour documenter le protocole.

### 9.3. Intégration dans le firmware Matter

Dans le firmware NCS (`matter-template` ou adapté) :
1. **Ajouter le driver UART** dans le DeviceTree overlay du XIAO.
2. **Créer un module C dédié** (`toshiba_uart.c`) qui parse/envoie les trames.
3. **Mapper vers les clusters Matter** :
   - `On/Off` : Marche/Arrêt du climatiseur
   - `Thermostat` : Température consigne, mode (chaud/froid)
   - `Fan Control` : Vitesse de ventilation
   - `Temperature Measurement` : Température ambiante (retour du Toshiba)

---

## 10. Phase 7 : Intégration dans Homey (Matter over Thread)

1. **Vérifier la connectivité** : Homey (sur le Mac Mini) doit voir l'OTBR comme un service mDNS local (`_meshcop._udp`).
2. **Ajouter un appareil Matter** dans Homey :
   - Appareils → Ajouter un appareil → Matter.
3. **Commissionner** :
   - Homey scanne le réseau Thread via l'OTBR.
   - Chaque XIAO (#2, #3, #4) émet un code QR ou un PIN via sa console série (ou un LED pattern).
   - Scanner le code QR dans Homey pour commissionner l'appareil.
4. **Répéter** pour les 3 climatiseurs.

> **Note :** La communication opérationnelle passe par : Homey → IP local → OTBR → Thread → XIAO#2/#3/#4 → UART → Toshiba.

---

## 11. Phase 8 : Tests de Résilience

| Scénario | Action | Résultat attendu |
|----------|--------|------------------|
| **Panne XIAO#2** | Débrancher le XIAO#2 | XIAO#3 et #4 maintiennent le mesh. Toshiba#1 injoignable. Homey contrôle toujours #2 et #3 via l'OTBR. |
| **Panne XIAO#1 (RCP)** | Débrancher le XIAO#1 du Mac Mini | Le réseau Thread continue entre #2, #3, #4, mais **Homey perd tout accès** (pas de Border Router). |
| **Redémarrage Mac Mini** | Redémarrer le Mac Mini | L'OTBR redémarre, le XIAO#1 est réinitialisé. Le mesh se reforme. Homey retrouve les appareils. |
| **Panne Wi-Fi** | Couper le Wi-Fi du Mac Mini | Homey (local) ne peut plus être atteint depuis l'extérieur, mais le LAN interne et le Thread continuent de fonctionner. |
| **Portée radio** | Tester la distance XIAO#1 ↔ XIAO#2 | Si le signal est faible, le mesh doit passer par XIAO#3 ou #4 comme relais. |

---

## 12. En Résumé

Cette architecture corrigée résout le problème fondamental du plan initial :

| Aspect | Plan initial (incorrect) | Plan corrigé |
|--------|-------------------------|--------------|
| **Protocole** | Matter over Wi-Fi (impossible) | **Matter over Thread** |
| **Border Router** | Inexistant | **XIAO#1 en RCP + OTBR sur Mac Mini** |
| **Radio Homey** | Thread intégrée requise | **Plus nécessaire** (OTBR local) |
| **Redondance** | 3 XIAO en mesh | **4 XIAO** (1 RCP + 3 FTD) en mesh |

**Prochaines étapes immédiates :**
1. [ ] Valider le port USB du XIAO#1 sur le Mac Mini (`/dev/tty.usbmodem*`).
2. [ ] Tester le conteneur OTBR avec Docker.
3. [ ] Documenter le protocole UART du Toshiba Shorei Edge (CN22).
4. [ ] Flasher un exemple `matter-template` sur un XIAO#2 pour valider le commissioning avec Homey.
```

---

**Note importante sur le XIAO#1 :** Il doit être positionné de manière à avoir une bonne portée radio Thread avec au moins un des XIAO#2-4. Si le Mac Mini est dans un bureau fermé et les climatiseurs dans des pièces éloignées, envisagez une **rallonge USB-C active** (jusqu'à 5-10m) pour placer le XIAO#1 stratégiquement, ou ajoutez un **XIAO#5** supplémentaire comme simple routeur Thread relais (sans Toshiba) dans un couloir central.
