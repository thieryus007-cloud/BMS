# PLAN PROJET : Infrastructure Thread redondante avec XIAO nRF54LM20A et Homey Pro Self-Hosted

## 1. Contexte et Problématique

Le **XIAO nRF54LM20A Sense** est un SoC **Bluetooth LE + 802.15.4 (Thread/Zigbee)**. Il ne possède **aucune radio Wi-Fi**. Il est donc impossible de réaliser du *Matter over Wi-Fi* avec ce hardware.

Pour connecter des appareils Matter over Thread à Homey Pro Self-Hosted (sur Mac Mini), il faut un **Border Router Thread** qui fasse le pont entre le réseau IP local (LAN) et le réseau maillé Thread (802.15.4).

Nous avons identifié deux architectures pour implémenter ce Border Router.

---

## 2. Option A : RCP + OTBR Docker sur Mac Mini *(recommandée)*

### 2.1. Principe
Le **XIAO#1** est flashé en mode **RCP (Radio Co-Processor)** : il n'est qu'une radio 802.15.4 "muette" contrôlée par le Mac Mini via USB. Toute l'intelligence réseau (routage IPv6, NAT64, DNS-SD, mDNS proxying, firewall) est assurée par le **OpenThread Border Router (OTBR)** qui tourne dans un conteneur **Docker** sur le Mac Mini.

### 2.2. Architecture
```
Mac Mini ──USB-C──► XIAO#1 (RCP : radio uniquement)
       │
       └─► Docker OTBR (OpenThread Border Router officiel)
              ├─► Interface IP vers Homey (localhost)
              ├─► NAT64 (IPv6 Thread ↔ IPv4 LAN)
              ├─► DNS-SD / mDNS proxying
              └─► Commissioning Matter
```

### 2.3. Détails techniques
| Composant | Rôle | Localisation |
|-----------|------|--------------|
| **XIAO#1** | RCP (Spinel over UART/USB) | Branché en USB-C au Mac Mini |
| **Mac Mini** | Hôte Docker + Homey Pro | Exécute l'OTBR + Homey |
| **Conteneur OTBR** | Border Router complet | `openthread/otbr:latest` |
| **XIAO#2, #3, #4** | FTD (Full Thread Device) + Matter | Routeurs Thread + contrôleurs Toshiba |

### 2.4. Avantages
- **Stack validée** : L'image Docker `openthread/otbr` est maintenue par Google/OpenThread. Compatible testée avec HomeKit, Google Home, Home Assistant, et Homey.
- **Tout est géré** : NAT64, DNS64, mDNS proxying, firewall, commissioning — tout est dans le conteneur.
- **Debug facile** : `docker logs otbr`, `docker exec otbr ot-ctl`, interface web sur `http://localhost:8080`.
- **Mise à jour simple** : `docker pull openthread/otbr:latest`.
- **Le XIAO#1 est "jetable"** : s'il tombe en panne, on le remplace, on reflash le RCP, on redémarre le conteneur. Aucune config réseau perdue.

### 2.5. Inconvénients
- Nécessite **Docker** sur le Mac Mini (Docker Desktop ou OrbStack).
- Le XIAO#1 est dédié au Border Router et ne peut **pas** contrôler de Toshiba en parallèle.
- **Nécessite 4 XIAO** pour 3 Toshiba (1 RCP + 3 FTD).

---

## 3. Option B : Border Router natif sur le XIAO

### 3.1. Principe
Le **XIAO#1** est flashé en mode **FTD + Border Router natif OpenThread**. Il crée et gère le réseau Thread, expose une **interface réseau IP virtuelle** (USB CDC-ECM ou RNDIS) au Mac Mini via USB, et fait lui-même le pont IPv6 entre Thread et le LAN.

### 3.2. Architecture
```
Mac Mini ──USB-C──► XIAO#1 (FTD + Border Router + USB CDC-ECM)
       │
       └─► Interface réseau virtuelle "usb0" sur le Mac
              ├─► Routage IPv6 manuel
              ├─► mDNS / DNS-SD manuel (avahi/dns-sd)
              └─► NAT64 manuel (ou préfixe IPv6 global)
```

### 3.3. Détails techniques
| Composant | Rôle | Localisation |
|-----------|------|--------------|
| **XIAO#1** | FTD + BR natif + USB ECM | Branché en USB-C au Mac Mini |
| **Mac Mini** | Hôte + Homey Pro | Doit router IPv6, proxy mDNS, NAT64 |
| **XIAO#2, #3** | FTD + Matter | Routeurs Thread + contrôleurs Toshiba |

### 3.4. Avantages
- **Pas de Docker** : tout tourne sur le XIAO#1 et le Mac Mini en natif.
- **Un XIAO en moins** : avec 3 XIAO, on peut avoir 1 BR natif + 2 FTD/Toshiba (2 climatiseurs).
- **Latence potentiellement plus faible** : pas de conteneur, pas de virtualisation réseau.

### 3.5. Inconvénients
- **Firmware complexe** : il faut compiler Zephyr avec `CONFIG_OPENTHREAD_BORDER_ROUTER`, `CONFIG_NET_IPV6`, `CONFIG_USB_DEVICE_NETWORK`, et un DeviceTree overlay USB ECM spécifique au XIAO nRF54.
- **Config Mac Mini complexe** : macOS ne route pas l'IPv6 par défaut. Il faut activer `net.inet6.ip6.forwarding=1`, configurer le forwarding entre l'interface USB et le LAN, gérer les routes statiques IPv6.
- **mDNS manuel** : il faut installer et configurer **avahi** ou utiliser `dns-sd` pour que Homey découvre les appareils Thread et le service `_meshcop._udp`.
- **NAT64 manuel** : si vous n'avez pas de préfixe IPv6 global, il faut configurer un NAT64 (ex: ` tayga`) sur le Mac Mini.
- **Debug difficile** : les logs du BR sont sur UART à 115200 bauds. Le debug réseau se fait via `tcpdump`/`wireshark` sur le Mac.
- **RAM limitée** : le nRF54L15 a 256 KB de RAM. Faire tourner le BR + NAT64 + DNS + USB stack + Matter est très juste.
- **Compatibilité Homey non garantie** : Homey teste principalement contre l'OTBR Docker officiel. Un BR natif sur Zephyr peut fonctionner, mais n'est pas un chemin validé.

---

## 4. Décision : Option A choisie

**Nous choisissons l'Option A (RCP + OTBR Docker) pour les raisons suivantes :**

1. **Fiabilité** : L'OTBR Docker est la stack de référence, testée et maintenue par le consortium OpenThread.
2. **Simplicité opérationnelle** : `docker run` et l'interface web gèrent 90% de la configuration réseau.
3. **Debug** : Les logs Docker et l'interface web facilitent énormément le diagnostic.
4. **Compatibilité Homey** : Homey Pro communique avec l'OTBR via mDNS standard (`_meshcop._udp`), ce qui est garanti avec l'image officielle.
5. **Scalabilité** : Si le projet s'étend (plus de XIAO, plus de Toshiba), l'OTBR Docker gère le mesh sans modification du firmware.

> **⚠️ Conséquence matérielle** : Avec l'Option A, il faut **1 XIAO dédié au Border Router**. Si vous n'avez que 3 XIAO, vous ne pourrez contrôler que **2 Toshiba** (1 RCP + 2 FTD). Pour 3 Toshiba, il faut **4 XIAO**.
>
> **Alternative** : Si vous ne disposez que de 3 XIAO, vous pouvez soit n'en connecter que 2 aux Toshiba, soit acquérir un 4ème XIAO.

---

## 5. Architecture finale (Option A)

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

## 6. Phase 1 : Préparation de l'Environnement

### 6.1. Mac Mini
1. Installer **Docker Desktop** (ou OrbStack) pour faire tourner l'OTBR.
2. Vérifier que le Mac Mini est accessible sur le réseau local avec une **IP fixe** (ex: `192.168.1.100`).

### 6.2. VS Code + nRF Connect SDK
1. Installer VS Code et l'extension **nRF Connect for VS Code Extension Pack**.
2. Installer le **nRF Connect SDK (NCS)** complet (RTOS-based, pas Bare Metal).
3. Vérifier la détection des ports série USB des XIAO.

---

## 7. Phase 2 : XIAO #1 — Flashage en RCP (Radio Co-Processor)

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

## 8. Phase 3 : OpenThread Border Router (OTBR) sur Mac Mini

### 8.1. Lancer l'OTBR via Docker

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

### 8.2. Vérifier l'OTBR
1. Accéder à l'interface web de l'OTBR : `http://localhost:8080`
2. Vérifier que le **XIAO#1 (RCP)** est détecté comme "Radio" et que le status est "Active".
3. Dans l'interface web, cliquer sur **"Form"** pour créer le réseau Thread.
4. Noter le **Active Dataset** généré (chaîne hexadécimale) ou utiliser la fonction "Commission" pour générer un code PIN.

### 8.3. Configuration réseau
- L'OTBR publie le réseau Thread sur le LAN via **mDNS** (service `_meshcop._udp`).
- Homey (sur le même Mac Mini) verra ce service mDNS automatiquement.
- Le conteneur gère le **NAT64** (accès IPv4 depuis le réseau Thread) et le **DNS64** automatiquement.

---

## 9. Phase 4 : XIAO #2, #3, #4 — Firmware Routeur Thread + Matter

Ces 3 XIAO ont le même firmware. Chacun :
- Rejoint le réseau Thread créé par l'OTBR.
- Contrôle un Toshiba via UART.
- Expose le climatiseur comme un appareil Matter (clusters Thermostat, On/Off, Fan Control).

### 9.1. Compiler le firmware Matter
```bash
cd nrf/samples/matter/template  # Ou matter-thermostat si disponible
west build -b xiao_nrf54lm20a_sense
```

### 9.2. Flasher (répéter pour les 3 XIAO)
```bash
west flash --runner jlink
```

---

## 10. Phase 5 : Configuration du Réseau Thread

### 10.1. Méthode recommandée : Commissioning via OTBR

1. Sur l'interface web de l'OTBR (`http://localhost:8080`), cliquer sur **"Commission"**.
2. L'OTBR génère un **Joiner Credential** (code PIN alphanumérique, ex: `J01NME`).
3. Sur chaque XIAO (#2, #3, #4), via la console série, activer le mode Joiner :
   ```bash
   > ifconfig up
   > thread joiner start <PIN_DE_LOTBR>
   ```
4. Le XIAO rejoint automatiquement le réseau Thread, reçoit le dataset actif, et devient un routeur.
5. Vérifier sur l'interface web de l'OTBR que les nouveaux routeurs apparaissent.

### 10.2. Méthode manuelle (fallback)

Si le commissioning automatique échoue :

1. **Récupérer le dataset** depuis l'OTBR :
   ```bash
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
   # Doit lister les autres XIAO du mesh
   ```

---

## 11. Phase 6 : Interface UART et Firmware Matter

### 11.1. Câblage (Level Shifter obligatoire)

| Level Shifter | Connexion |
|---------------|-----------|
| HV (5V) | Alimentation Toshiba (CN22) |
| LV (3.3V) | Pin 3.3V du XIAO |
| HV1 / LV1 | Ligne TX (Toshiba → XIAO) |
| HV2 / LV2 | Ligne RX (XIAO → Toshiba) |
| GND | Masse commune |

### 11.2. Protocole Toshiba Shorei Edge (CN22)

**⚠️ À documenter impérativement avant de coder :**
- **Baud rate** (probablement 9600 ou 2400)
- **Format** (8N1, 7E1...)
- **Structure des trames** (header, commande, données, checksum)
- **Mapping des commandes** : ON/OFF, température consigne, mode (chaud/froid), vitesse ventilateur

**Recommandation** : Sniffer le bus UART avec un analyseur logique entre le contrôleur d'origine et le Toshiba pour documenter le protocole.

### 11.3. Intégration dans le firmware Matter

Dans le firmware NCS (`matter-template` ou adapté) :
1. **Ajouter le driver UART** dans le DeviceTree overlay du XIAO.
2. **Créer un module C dédié** (`toshiba_uart.c`) qui parse/envoie les trames.
3. **Mapper vers les clusters Matter** :
   - `On/Off` : Marche/Arrêt du climatiseur
   - `Thermostat` : Température consigne, mode (chaud/froid)
   - `Fan Control` : Vitesse de ventilation
   - `Temperature Measurement` : Température ambiante (retour du Toshiba)

---

## 12. Phase 7 : Intégration dans Homey (Matter over Thread)

1. **Vérifier la connectivité** : Homey (sur le Mac Mini) doit voir l'OTBR comme un service mDNS local (`_meshcop._udp`).
2. **Ajouter un appareil Matter** dans Homey :
   - Appareils → Ajouter un appareil → Matter.
3. **Commissionner** :
   - Homey scanne le réseau Thread via l'OTBR.
   - Chaque XIAO (#2, #3, #4) émet un **code QR** ou un **PIN** via sa console série (ou un LED pattern).
   - Scanner le code QR dans Homey pour commissionner l'appareil.
4. **Répéter** pour les 3 climatiseurs.

> **Chemin de la donnée** : Homey → IP local → OTBR Docker → Thread → XIAO#2/#3/#4 → UART → Toshiba.

---

## 13. Phase 8 : Tests de Résilience

| Scénario | Action | Résultat attendu |
|----------|--------|------------------|
| **Panne XIAO#2** | Débrancher le XIAO#2 | XIAO#3 et #4 maintiennent le mesh. Toshiba#1 injoignable. Homey contrôle toujours #2 et #3 via l'OTBR. |
| **Panne XIAO#1 (RCP)** | Débrancher le XIAO#1 du Mac Mini | Le réseau Thread continue entre #2, #3, #4, mais **Homey perd tout accès** (pas de Border Router). |
| **Redémarrage Mac Mini** | Redémarrer le Mac Mini | L'OTBR redémarre, le XIAO#1 est réinitialisé. Le mesh se reforme. Homey retrouve les appareils. |
| **Panne Wi-Fi** | Couper le Wi-Fi du Mac Mini | Homey (local) ne peut plus être atteint depuis l'extérieur, mais le LAN interne et le Thread continuent de fonctionner. |
| **Portée radio** | Tester la distance XIAO#1 ↔ XIAO#2 | Si le signal est faible, le mesh doit passer par XIAO#3 ou #4 comme relais. |

---

## 14. En Résumé

| Aspect | Plan initial (incorrect) | Plan corrigé (Option A) |
|--------|------------------------|------------------------|
| **Protocole** | Matter over Wi-Fi (impossible) | **Matter over Thread** |
| **Border Router** | Inexistant | **XIAO#1 en RCP + OTBR Docker sur Mac Mini** |
| **Radio Homey** | Thread intégrée requise | **Plus nécessaire** (OTBR local) |
| **Redondance** | 3 XIAO en mesh | **4 XIAO** (1 RCP + 3 FTD) en mesh |
| **Stack réseau** | Aucune | **OTBR Docker officiel (Google/OpenThread)** |

---

## 15. Checklist de démarrage

1. [ ] **Vérifier le nombre de XIAO** : 4 XIAO nécessaires pour 3 Toshiba avec l'Option A. Sinon, n'en connecter que 2 aux Toshiba.
2. [ ] **Valider le port USB** du XIAO#1 sur le Mac Mini (`/dev/tty.usbmodem*` ou `/dev/ttyACM*`).
3. [ ] **Tester Docker** sur le Mac Mini (`docker run hello-world`).
4. [ ] **Documenter le protocole UART** du Toshiba Shorei Edge (CN22) : baud rate, format, trames.
5. [ ] **Vérifier le board name NCS** : `west boards \| grep xiao` ou `nrf54lm20a`.
6. [ ] **Tester un exemple Matter simple** (`matter-template`) sur un seul XIAO#2 pour valider le commissioning avec Homey avant d'ajouter la couche UART.
7. [ ] **Positionner le XIAO#1** : il doit être à portée radio Thread (10-15m, moins avec murs) d'au moins un XIAO#2-4. Utiliser une rallonge USB-C active si nécessaire.
```

---
