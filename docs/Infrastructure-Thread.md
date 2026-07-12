# PLAN PROJET : Infrastructure Thread redondante avec XIAO nRF54LM20A et Homey Self-Hosted Server

## 1. Contexte et Problématique

Le **XIAO nRF54LM20A Sense** est un SoC **Bluetooth LE + 802.15.4 (Thread/Zigbee)**. Il ne possède **aucune radio Wi-Fi**. Il est donc impossible de réaliser du *Matter over Wi-Fi* avec ce hardware.

Pour connecter des appareils **Matter over Thread** au **Homey Self-Hosted Server (SHS)** (tournant dans un conteneur Docker sur Mac Mini), il faut un **Thread Border Router (OTBR)** qui fasse le pont entre le réseau IP local (LAN) et le réseau maillé Thread (802.15.4).

**Le Homey SHS ne possède pas de radio Thread.** Il peut utiliser Matter-over-Thread uniquement si un Border Router Thread existe déjà sur le réseau local.

---

## 2. Option A : Matter over Thread (choisie)

C'est la seule architecture viable avec des XIAO nRF54LM20A.

Deux sous-options sont possibles pour fournir le Border Router Thread :

### 2.1. Option A1 : OTBR externe déjà présent sur le réseau *(recommandée si disponible)*

Si vous possédez déjà un appareil **Thread Border Router certifié** sur votre réseau local, vous pouvez l'utiliser directement. Le Homey SHS le découvrira automatiquement via mDNS (`_meshcop._udp`).

**Appareils compatibles (OTBR certifiés) :**
| Appareil | Marque | Type | Notes |
|----------|--------|------|-------|
| **Amazon Echo (4th gen, Echo Studio, Echo Show 15/8 2nd gen)** | Amazon | Smart Speaker | Border Router Thread natif |
| **Apple HomePod / HomePod mini / Apple TV 4K (2nd gen+)** | Apple | Speaker / TV | Border Router Thread natif |
| **Google Nest Hub (2nd gen) / Nest Hub Max / Nest Wifi Pro** | Google | Smart Display / Router | Border Router Thread natif |
| **SMHUB Nano MG24** | Silicon Labs / tiers | Dongle USB | Border Router Thread dédié |
| **Eve Extend** | Eve | Bridge | Border Router Thread natif |
| **Aqara Hub M3** | Aqara | Hub | Border Router Thread natif |

**Architecture :**
```
┌─────────────────────────────────────────┐
│         Homey SHS (Docker/Mac Mini)    │
│         + Votre OTBR existant           │
│         (Amazon Echo / HomePod / etc.)  │
└─────────────────┬───────────────────────┘
                  │ Wi-Fi / Ethernet
                  │ (mDNS discovery)
                  ▼
           ┌──────────────┐
           │  OTBR déjà   │
           │  sur le LAN  │
           │  (HomePod,   │
           │  Echo, etc.) │
           └──────┬───────┘
                  │ Thread 802.15.4
       ┌──────────┼──────────┐
       ▼          ▼          ▼
   ┌────────┐ ┌────────┐ ┌────────┐
   │ XIAO#1 │ │ XIAO#2 │ │ XIAO#3 │
   │Router  │ │Router  │ │Router  │
   │+Matter │ │+Matter │ │+Matter │
   └───┬────┘ └───┬────┘ └───┬────┘
       │ UART     │ UART     │ UART
       ▼          ▼          ▼
   ┌────────┐ ┌────────┐ ┌────────┐
   │Toshiba │ │Toshiba │ │Toshiba │
   │  #1    │ │  #2    │ │  #3    │
   └────────┘ └────────┘ └────────┘
```

**Avantages :**
- **Aucun XIAO supplémentaire** : 3 XIAO suffisent pour 3 Toshiba.
- **Aucun Docker OTBR** à configurer et maintenir sur le Mac Mini.
- **Stack validée** : Ces appareils sont certifiés par le Thread Group et testés avec Matter.
- **Mise à jour automatique** : Le firmware OTBR est maintenu par le fabricant.

**Inconvénients :**
- **Dépendance** : Si l'appareil OTBR tombe en panne ou est débranché, tout le réseau Thread devient invisible pour Homey.
- **Moins de contrôle** : Vous ne pouvez pas accéder aux logs OTBR, ni forcer un canal Thread spécifique, ni debuguer facilement.
- **Portée radio** : L'OTBR doit être physiquement proche (10-15m, moins avec murs) d'au moins un XIAO.

---

### 2.2. Option A2 : XIAO#1 en RCP + OTBR Docker sur Mac Mini *(si pas d'OTBR existant)*

Si vous **n'avez aucun** appareil OTBR sur votre réseau, vous devez en créer un avec un XIAO dédié.

**Architecture :**
```
┌─────────────────────────────────────────────────────────────┐
│                      MAC MINI (Local)                       │
│  ┌─────────────────────┐    ┌───────────────────────────┐ │
│  │  Homey SHS          │◄──►│  OpenThread Border Router │ │
│  │  (Docker)           │ IP │  (Docker / Service local) │ │
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

**Avantages :**
- **Contrôle total** : Accès aux logs, interface web, commandes `ot-ctl`, choix du canal.
- **Indépendance** : Pas de dépendance à un appareil tiers (Echo, HomePod, etc.).
- **Debug facilité** : `docker logs`, interface web sur `localhost:8080`.

**Inconvénients :**
- **Nécessite 1 XIAO supplémentaire** : 4 XIAO pour 3 Toshiba (1 RCP + 3 FTD).
- **Nécessite Docker** sur le Mac Mini (en plus du conteneur Homey SHS).
- **Maintenance** : Vous gérez vous-même les mises à jour de l'image OTBR.

---

## 3. Décision

**Nous choisissons l'Option A (Matter over Thread) avec deux sous-options selon le matériel déjà disponible :**

| Situation | Sous-option choisie | XIAO nécessaires |
|-----------|-------------------|------------------|
| J'ai déjà un OTBR (Echo, HomePod, Google Nest, SMHUB, etc.) | **A1** | **3 XIAO** (tous pour les Toshiba) |
| Je n'ai aucun OTBR sur mon réseau | **A2** | **4 XIAO** (1 RCP + 3 Toshiba) |

> **Note :** Si vous choisissez l'Option A1, vous pouvez ignorer les Phases 2 et 3 du plan ci-dessous et passer directement à la Phase 4.

---

## 4. Inventaire du Matériel

### Option A1 (OTBR existant)
| Qté | Matériel | Rôle |
|-----|----------|------|
| 3x | XIAO nRF54LM20A Sense | Routeurs Thread + Matter + UART Toshiba |
| 3x | Toshiba Shorei Edge (port CN22) | Appareils contrôlés |
| 3x | Level Shifter 3.3V ↔ 5V | Adaptation UART |
| 1x | Mac Mini | Homey SHS (Docker) |
| 1x | OTBR existant (Echo/HomePod/Google/SMHUB) | Border Router Thread |

### Option A2 (OTBR créé avec XIAO)
| Qté | Matériel | Rôle |
|-----|----------|------|
| 4x | XIAO nRF54LM20A Sense | #1 = RCP ; #2-4 = Routeurs Thread + Matter + UART |
| 3x | Toshiba Shorei Edge (port CN22) | Appareils contrôlés |
| 3x | Level Shifter 3.3V ↔ 5V | Adaptation UART |
| 1x | Mac Mini | Homey SHS (Docker) + OTBR Docker |
| 1x | Câble USB-C | Pour XIAO#1 |

---

## 5. Schéma de l'Architecture (Option A1 — OTBR existant)

```
┌─────────────────────────────────────────┐
│         Homey SHS (Docker/Mac Mini)    │
│         + Votre OTBR existant           │
│         (Amazon Echo / HomePod / etc.)  │
└─────────────────┬───────────────────────┘
                  │ Wi-Fi / Ethernet
                  │ (mDNS discovery _meshcop._udp)
                  ▼
           ┌──────────────┐
           │  OTBR déjà   │
           │  sur le LAN  │
           └──────┬───────┘
                  │ Thread 802.15.4
       ┌──────────┼──────────┐
       ▼          ▼          ▼
   ┌────────┐ ┌────────┐ ┌────────┐
   │ XIAO#1 │ │ XIAO#2 │ │ XIAO#3 │
   │Router  │ │Router  │ │Router  │
   │+Matter │ │+Matter │ │+Matter │
   └───┬────┘ └───┬────┘ └───┬────┘
       │ UART     │ UART     │ UART
       ▼          ▼          ▼
   ┌────────┐ ┌────────┐ ┌────────┐
   │Toshiba │ │Toshiba │ │Toshiba │
   │  #1    │ │  #2    │ │  #3    │
   └────────┘ └────────┘ └────────┘
```

---

## 6. Schéma de l'Architecture (Option A2 — OTBR créé avec XIAO)

```
┌─────────────────────────────────────────────────────────────┐
│                      MAC MINI (Local)                       │
│  ┌─────────────────────┐    ┌───────────────────────────┐ │
│  │  Homey SHS          │◄──►│  OpenThread Border Router │ │
│  │  (Docker)           │ IP │  (Docker / Service local) │ │
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

## 7. Phase 1 : Préparation de l'Environnement

### 7.1. Mac Mini
1. Installer **Docker Desktop** (ou OrbStack) pour faire tourner le Homey SHS.
2. Vérifier que le Mac Mini est accessible sur le réseau local avec une **IP fixe** (ex: `192.168.1.100`).
3. **Option A2 uniquement** : Vérifier que Docker peut accéder aux ports USB (`/dev/tty.usbmodem*`).

### 7.2. VS Code + nRF Connect SDK
1. Installer VS Code et l'extension **nRF Connect for VS Code Extension Pack**.
2. Installer le **nRF Connect SDK (NCS)** complet (RTOS-based, pas Bare Metal).
3. Vérifier la détection des ports série USB des XIAO.

---

## 8. Phase 2 : XIAO #1 — Flashage en RCP (Option A2 uniquement)

> **Si vous choisissez l'Option A1 (OTBR existant), sautez cette phase.**

Le XIAO#1 est dédié au Border Router. Il ne contrôle pas de Toshiba.

1. **Ouvrir l'exemple Coprocessor** dans le NCS :
   ```bash
   cd nrf/samples/openthread/coprocessor
   west build -b xiao_nrf54lm20a_sense -- -DCONFIG_OPENTHREAD_COPROCESSOR_RCP=y
   ```
2. **Flasher** :
   ```bash
   west flash
   ```
3. **Vérifier** : Connectez le XIAO#1 en USB-C au Mac Mini. Il doit apparaître comme un port série :
   ```bash
   ls /dev/tty.usbmodem*
   # ou
   ls /dev/ttyACM*
   ```
   Notez le chemin exact (ex: `/dev/tty.usbmodem1234561`).

> **Note :** Si le board `xiao_nrf54lm20a_sense` n'est pas reconnu par `west boards`, utilisez le board Nordic générique `nrf54lm20a` avec un overlay DeviceTree pour le XIAO.

---

## 9. Phase 3 : OpenThread Border Router (Option A2 uniquement)

> **Si vous choisissez l'Option A1 (OTBR existant), sautez cette phase.**

### 9.1. Lancer l'OTBR via Docker

```bash
# Identifier le port USB du XIAO#1
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

### 9.2. Vérifier l'OTBR
1. Accéder à l'interface web de l'OTBR : `http://localhost:8080`
2. Vérifier que le **XIAO#1 (RCP)** est détecté comme "Radio" et que le status est "Active".
3. Dans l'interface web, cliquer sur **"Form"** pour créer le réseau Thread.
4. Noter le **Active Dataset** généré (chaîne hexadécimale) ou utiliser la fonction "Commission" pour générer un code PIN.

### 9.3. Configuration réseau
- L'OTBR publie le réseau Thread sur le LAN via **mDNS** (service `_meshcop._udp`).
- Homey SHS (dans son conteneur Docker) verra ce service mDNS automatiquement.
- Le conteneur gère le **NAT64** (accès IPv4 depuis le réseau Thread) et le **DNS64** automatiquement.

---

## 10. Phase 4 : XIAO — Firmware Routeur Thread + Matter

Ces XIAO ont le même firmware. Chacun :
- Rejoint le réseau Thread créé par l'OTBR (Option A1 ou A2).
- Contrôle un Toshiba via UART.
- Expose le climatiseur comme un appareil Matter (clusters Thermostat, On/Off, Fan Control).

### 10.1. Compiler le firmware Matter
```bash
cd nrf/samples/matter/template  # Ou matter-thermostat si disponible
west build -b xiao_nrf54lm20a_sense
```

### 10.2. Flasher (répéter pour chaque XIAO)
```bash
west flash --runner jlink
```

---

## 11. Phase 5 : Configuration du Réseau Thread

### 11.1. Méthode recommandée : Commissioning via OTBR

**Option A1 (OTBR existant) :**
1. Sur l'app de l'appareil OTBR (ex: Apple Home, Google Home, Alexa), vérifier que le réseau Thread est actif.
2. Récupérer le **Joiner Credential** ou le **Active Dataset** depuis l'interface de l'OTBR.
3. Sur chaque XIAO, via la console série, activer le mode Joiner :
   ```bash
   > ifconfig up
   > thread joiner start <PIN>
   ```

**Option A2 (OTBR Docker) :**
1. Sur l'interface web de l'OTBR (`http://localhost:8080`), cliquer sur **"Commission"**.
2. L'OTBR génère un **Joiner Credential** (code PIN alphanumérique, ex: `J01NME`).
3. Sur chaque XIAO, via la console série, activer le mode Joiner :
   ```bash
   > ifconfig up
   > thread joiner start <PIN_DE_LOTBR>
   ```
4. Le XIAO rejoint automatiquement le réseau Thread, reçoit le dataset actif, et devient un routeur.
5. Vérifier sur l'interface web de l'OTBR (ou via `ot-ctl`) que les nouveaux routeurs apparaissent.

### 11.2. Méthode manuelle (fallback)

Si le commissioning automatique échoue :

**Option A1 :**
- Récupérer le dataset depuis l'interface de l'OTBR existant (Apple Home, Google Home, etc.).
- Injecter sur chaque XIAO :
  ```bash
  > dataset set active <DATASET_HEX>
  > dataset commit active
  > ifconfig up
  > thread start
  ```

**Option A2 :**
- Récupérer le dataset depuis l'OTBR Docker :
  ```bash
  docker exec -it otbr ot-ctl dataset active -x
  ```
- Injecter sur chaque XIAO :
  ```bash
  > dataset set active <DATASET_HEX>
  > dataset commit active
  > ifconfig up
  > thread start
  ```

**Vérification commune :**
```bash
> state
# Doit afficher : router (ou child temporairement)
> router table
# Doit lister les autres XIAO du mesh
```

---

## 12. Phase 6 : Interface UART et Firmware Matter

### 12.1. Câblage (Level Shifter obligatoire)

| Level Shifter | Connexion |
|---------------|-----------|
| HV (5V) | Alimentation Toshiba (CN22) |
| LV (3.3V) | Pin 3.3V du XIAO |
| HV1 / LV1 | Ligne TX (Toshiba → XIAO) |
| HV2 / LV2 | Ligne RX (XIAO → Toshiba) |
| GND | Masse commune |

### 12.2. Protocole Toshiba Shorei Edge (CN22)

**⚠️ À documenter impérativement avant de coder :**
- **Baud rate** (probablement 9600 ou 2400)
- **Format** (8N1, 7E1...)
- **Structure des trames** (header, commande, données, checksum)
- **Mapping des commandes** : ON/OFF, température consigne, mode (chaud/froid), vitesse ventilateur

**Recommandation** : Sniffer le bus UART avec un analyseur logique entre le contrôleur d'origine et le Toshiba pour documenter le protocole.

### 12.3. Intégration dans le firmware Matter

Dans le firmware NCS (`matter-template` ou adapté) :
1. **Ajouter le driver UART** dans le DeviceTree overlay du XIAO.
2. **Créer un module C dédié** (`toshiba_uart.c`) qui parse/envoie les trames.
3. **Mapper vers les clusters Matter** :
   - `On/Off` : Marche/Arrêt du climatiseur
   - `Thermostat` : Température consigne, mode (chaud/froid)
   - `Fan Control` : Vitesse de ventilation
   - `Temperature Measurement` : Température ambiante (retour du Toshiba)

---

## 13. Phase 7 : Intégration dans Homey SHS (Matter over Thread)

1. **Vérifier la connectivité** : Homey SHS doit voir l'OTBR comme un service mDNS local (`_meshcop._udp`).
   - **Option A1** : Vérifiez que l'appareil OTBR (Echo, HomePod, etc.) est bien sur le même LAN.
   - **Option A2** : Vérifiez que le conteneur OTBR est bien en `--network host` et que le service mDNS est actif.
2. **Ajouter un appareil Matter** dans Homey SHS :
   - Appareils → Ajouter un appareil → Matter.
3. **Commissionner** :
   - Homey SHS scanne le réseau Thread via l'OTBR.
   - Chaque XIAO émet un **code QR** ou un **PIN** via sa console série (ou un LED pattern).
   - Scanner le code QR dans Homey pour commissionner l'appareil.
4. **Répéter** pour les 3 climatiseurs.

> **Chemin de la donnée :** Homey SHS → IP local → OTBR → Thread → XIAO → UART → Toshiba.

---

## 14. Phase 8 : Tests de Résilience

### Option A1 (OTBR existant)

| Scénario | Action | Résultat attendu |
|----------|--------|------------------|
| **Panne XIAO#1** | Débrancher le XIAO#1 | XIAO#2 et #3 maintiennent le mesh. Toshiba#1 injoignable. Homey contrôle toujours #2 et #3 via l'OTBR existant. |
| **Panne OTBR existant** | Débrancher l'OTBR (Echo, HomePod, etc.) | Le réseau Thread continue entre les XIAO, mais **Homey perd tout accès**. |
| **Redémarrage Mac Mini** | Redémarrer le Mac Mini | Homey SHS redémarre. L'OTBR existant reste actif. Les appareils Thread sont immédiatement visibles. |
| **Panne Wi-Fi** | Couper le Wi-Fi du Mac Mini | Homey SHS local ne peut plus être atteint depuis l'extérieur, mais le LAN interne et le Thread continuent de fonctionner. |

### Option A2 (OTBR créé avec XIAO)

| Scénario | Action | Résultat attendu |
|----------|--------|------------------|
| **Panne XIAO#2** | Débrancher le XIAO#2 | XIAO#3 et #4 maintiennent le mesh. Toshiba#1 injoignable. Homey contrôle toujours #2 et #3 via l'OTBR. |
| **Panne XIAO#1 (RCP)** | Débrancher le XIAO#1 du Mac Mini | Le réseau Thread continue entre #2, #3, #4, mais **Homey perd tout accès** (pas de Border Router). |
| **Redémarrage Mac Mini** | Redémarrer le Mac Mini | L'OTBR redémarre, le XIAO#1 est réinitialisé. Le mesh se reforme. Homey retrouve les appareils. |
| **Panne Wi-Fi** | Couper le Wi-Fi du Mac Mini | Homey SHS local ne peut plus être atteint depuis l'extérieur, mais le LAN interne et le Thread continuent de fonctionner. |
| **Portée radio** | Tester la distance XIAO#1 ↔ XIAO#2 | Si le signal est faible, le mesh doit passer par XIAO#3 ou #4 comme relais. |

---

## 15. En Résumé

| Aspect | Plan initial (incorrect) | Plan corrigé (Option A) |
|--------|------------------------|------------------------|
| **Protocole** | Matter over Wi-Fi (impossible) | **Matter over Thread** |
| **Border Router** | Inexistant | **OTBR existant (A1) ou XIAO RCP + Docker (A2)** |
| **Radio Homey SHS** | Thread intégrée requise | **Plus nécessaire** (OTBR externe) |
| **Redondance** | 3 XIAO en mesh | **3 XIAO (A1) ou 4 XIAO (A2)** en mesh |
| **Stack réseau** | Aucune | **OTBR certifié (A1) ou OTBR Docker (A2)** |

---

## 16. Checklist de démarrage

### Option A1 (OTBR existant)
1. [ ] **Vérifier que l'OTBR est bien actif** : Vérifier dans l'app correspondante (Apple Home, Google Home, Alexa) que le réseau Thread est formé.
2. [ ] **Récupérer le dataset** ou le code PIN de commissioning depuis l'interface de l'OTBR.
3. [ ] **Documenter le protocole UART** du Toshiba Shorei Edge (CN22) : baud rate, format, trames.
4. [ ] **Vérifier le board name NCS** : `west boards | grep xiao` ou `nrf54lm20a`.
5. [ ] **Tester un exemple Matter simple** (`matter-template`) sur un seul XIAO pour valider le commissioning avec Homey SHS avant d'ajouter la couche UART.
6. [ ] **Vérifier la portée radio** : L'OTBR doit être à portée (10-15m) d'au moins un XIAO.

### Option A2 (OTBR créé avec XIAO)
1. [ ] **Valider le port USB** du XIAO#1 sur le Mac Mini (`/dev/tty.usbmodem*` ou `/dev/ttyACM*`).
2. [ ] **Tester Docker** sur le Mac Mini (`docker run hello-world`).
3. [ ] **Documenter le protocole UART** du Toshiba Shorei Edge (CN22) : baud rate, format, trames.
4. [ ] **Vérifier le board name NCS** : `west boards | grep xiao` ou `nrf54lm20a`.
5. [ ] **Tester un exemple Matter simple** (`matter-template`) sur un seul XIAO#2 pour valider le commissioning avec Homey SHS avant d'ajouter la couche UART.
6. [ ] **Positionner le XIAO#1** : il doit être à portée radio Thread (10-15m, moins avec murs) d'au moins un XIAO#2-4. Utiliser une rallonge USB-C active si nécessaire.
```

---
