# Intégration de devices Matter-over-Thread avec Homey Self-Hosted Server (SHS)

> **Version** : 2.0 — Juillet 2026  
> **Auteur** : Documentation communautaire  
> **Matériel concerné** : SMLight Nano MG24 (OTBR), Mac-Mini M4 (Homey SHS), Raspberry Pi 5 (Commissioner Matter), ESP32-C61 / IKEA KAJPLATS / GRILLPLATS

---

## Architecture du réseau

```
+-----------------------------------------------------------------------------+
|                           RÉSEAU LOCAL (LAN)                                |
|                                                                             |
|  +---------------------+    +---------------------+    +----------------+   |
|  |  SMLight Nano MG24  |    |   Mac-Mini M4       |    |  Raspberry Pi5 |   |
|  |  (OTBR / TBR)       |<-->|   Homey SHS         |    |  (Commissioner |   |
|  |  - Thread mesh      |IPv6|   - Contrôleur      |    |   Matter)      |   |
|  |  - mDNS proxy       |    |     Matter          |    |  - BLE + IP    |   |
|  |  - IPv6 routing     |    |   - IPv6 actif      |    |  - chip-tool   |   |
|  +----------+----------+    +---------------------+    |  - matterjs-   |   |
|             |                                          |    server      |   |
|             |         Thread mesh                      +----------------+   |
|             |    +-------------+  +-------------+                           |
|             +--->|  ESP32-C61  |  | IKEA        |                           |
|                  |  (End Dev)  |  | KAJPLATS    |                           |
|                  |             |  | (End Dev)   |                           |
|                  +-------------+  +-------------+                           |
|                                                                             |
+-----------------------------------------------------------------------------+
```

| Rôle | Matériel | Fonction |
|------|----------|----------|
| **Thread Border Router (TBR)** | SMLight Nano MG24 | Gère le mesh Thread, routage IPv6 vers LAN, proxy mDNS |
| **Contrôleur Matter** | Homey SHS sur Mac-Mini M4 | Pilote logique des devices via Matter sur IPv6 |
| **Commissioner Matter** | Raspberry Pi 5 | Commissioning BLE -> Thread des devices (chip-tool / matterjs-server) |
| **Devices Thread** | ESP32-C61 / IKEA KAJPLATS / GRILLPLATS | End Devices sur le mesh Thread |

> **Important** : L'ESP32-C61 est un **End Device Thread**, pas un Border Router. Seul le SMLight fait office de TBR.

---

## Problème identifié

Le menu **Paramètres -> Thread** n'existe **pas** dans l'app Homey mobile lorsqu'on utilise Homey SHS sans TBR partenaire certifié (Apple, Google, Amazon). Homey SHS ne peut donc **pas récupérer automatiquement les credentials Thread** depuis le téléphone.

**Conséquence** : Le commissioning Matter-over-Thread standard (via QR code dans l'app Homey) **échouera** car le téléphone ne connaît pas le réseau Thread du SMLight.

**Solution** : Utiliser le **Raspberry Pi 5** comme commissioner Matter indépendant, puis partager le device avec Homey SHS.

---

## Prérequis réseau

### 1. IPv6 actif sur le Mac-Mini (Homey SHS)

Le Mac-Mini et le SMLight communiquent en **IPv6 natif**.

```bash
# Sur le Mac-Mini
ifconfig | grep inet6
# Doit afficher des adresses IPv6 :
#   fe80::...  (link-local)
#   fd00::...  ou 2001:... (globales / ULA)
```

### 2. IPv6 actif sur le SMLight (OTBR)

```bash
# Sur le SMLight (SSH)
ot-ctl ipaddr
# Doit afficher des adresses IPv6 dans le préfixe Thread
```

### 3. mDNS fonctionnel entre SMLight et LAN

**Sur le SMLight (SSH) :**
```bash
ps aux | grep -E 'avahi-daemon|openthread' | grep -v grep
# Doit afficher avahi-daemon ET otbr-agent actifs
```

**Sur le Mac-Mini ou un autre poste du LAN :**
```bash
# macOS - vérifier la découverte Matter
dns-sd -B _matter._tcp local

# Linux - vérifier la découverte Matter
avahi-browse -d local _matter._tcp --resolve

# Doit lister les annonces Matter du réseau
# Si vide = problème mDNS entre SMLight et LAN
```

> **Note** : `_meshcop._udp` annonce le réseau Thread lui-même. `_matter._tcp` annonce les services Matter. Les deux doivent être visibles.

### 4. Thread Dataset connu

Récupérez le **Thread Operational Dataset** actif sur le SMLight :
```bash
# Sur le SMLight (SSH)
ot-ctl dataset active -x
# Copiez la chaîne hexadécimale (TLV) affichée - c'est votre Dataset
# Exemple : 0e080000000000010000000300001335060004001fffe00208fe... 
```

---

## Étape 0 : Préparer le Raspberry Pi 5 comme Commissioner Matter

### 0.1 Vérifier le Bluetooth (BLE) sur le Pi5

Le Pi5 embarque un contrôleur Bluetooth **CYW43455** (Wi-Fi + BT combo). Vérifiez qu'il est fonctionnel :

```bash
# Vérifier le contrôleur Bluetooth
bluetoothctl list
# Résultat attendu : Controller XX:XX:XX:XX:XX:XX pi5compute [default]

# Vérifier le service
sudo systemctl status bluetooth
# Résultat attendu : active (running)

# Vérifier la version BlueZ
bluetoothctl --version
# Minimum requis : 5.55+ (idéalement 5.66+)
# Votre version devrait être 5.82+ sur Pi5 récent

# Vérifier les capacités
bluetoothctl show
# Powered: yes
# Roles: central, peripheral
```

> **Remarque** : Si `Pairable: no`, activez temporairement pour le commissioning :
> ```bash
> bluetoothctl
> [bluetooth]# pairable on
> [bluetooth]# scan on
> ```

---

## Option A : chip-tool (RECOMMANDÉ - CLI officielle CSA)

### A.1 Qu'est-ce que chip-tool ?

`chip-tool` est l'**outil CLI officiel** du projet [connectedhomeip](https://github.com/project-chip/connectedhomeip), maintenu par la **Connectivity Standards Alliance (CSA)**. C'est la référence pour le commissioning et le contrôle de devices Matter.

| Caractéristique | Détail |
|-----------------|--------|
| **Projet source** | `project-chip/connectedhomeip` |
| **Langage** | C++ (compilé) |
| **Statut** | Activement maintenu, certifié CSA |
| **Fonctions** | Commissioning BLE/Wi-Fi/Thread, contrôle de devices, multi-admin, diagnostics |
| **Avantage clé** | Pas de serveur à faire tourner, outil CLI autonome |

### A.2 Installation sur le Pi5

#### Méthode 1 : Via snap (RECOMMANDÉ - plus simple)

```bash
# 1. Installer snapd si ce n'est pas déjà fait
sudo apt update
sudo apt install snapd

# 2. Installer chip-tool via snap
sudo snap install chip-tool

# 3. Vérifier l'installation
chip-tool --version
# Doit afficher la version (ex: chip-tool 1.4.0)
```

#### Méthode 2 : Compilation depuis les sources

```bash
# 1. Cloner le dépôt officiel
git clone https://github.com/project-chip/connectedhomeip.git
cd connectedhomeip

# 2. Initialiser les sous-modules
git submodule update --init --recursive

# 3. Installer les dépendances de build
sudo apt install git gcc g++ pkg-config libssl-dev libdbus-1-dev   libglib2.0-dev libavahi-client-dev ninja-build python3-venv   python3-dev python3-pip unzip libgirepository1.0-dev   libcairo2-dev libreadline-dev

# 4. Compiler chip-tool
./scripts/examples/gn_build_example.sh   examples/chip-tool out/chip-tool

# 5. Vérifier
./out/chip-tool/chip-tool --version
```

> **Note** : La compilation prend 30-60 minutes sur un Pi5. Préférez la méthode snap si disponible.

### A.3 Commissioning BLE -> Thread

```bash
# Récupérer le Thread Dataset du SMLight (étape préalable)
# Sur le SMLight : ot-ctl dataset active -x
# Copiez la valeur hexadécimale complète

# Lancer chip-tool avec accès Bluetooth
chip-tool pairing ble-thread <NODE_ID>   hex:<DATASET_TLV>   <PIN_CODE_MATTER>   <DISCRIMINATOR>
```

| Paramètre | Description | Où le trouver |
|-----------|-------------|---------------|
| `<NODE_ID>` | ID unique du node Matter (ex: 1, 2, 3...) | Vous le choisissez |
| `<DATASET_TLV>` | Thread Operational Dataset en hex | `ot-ctl dataset active -x` sur SMLight |
| `<PIN_CODE_MATTER>` | Code PIN Matter du device (11 chiffres) | QR code / étiquette du device |
| `<DISCRIMINATOR>` | Discriminateur BLE (4 chiffres, 0-4095) | QR code / documentation device |

> **Le code PIN Matter** est différent du code PIN Thread. Le code PIN Matter est sur le QR code du device (format XXX-XXX-XXX-X).

**Exemple concret :**
```bash
chip-tool pairing ble-thread 1   hex:0e080000000000010000000300001335060004001fffe00208fe...   12345678   3840
```

**Sortie attendue :**
```
[TOO] SetupPINCode: 12345678
[TOO] Discriminator: 3840
[TOO] Device completed Rendezvous over BLE
[TOO] Device completed Thread provisioning
[TOO] Device successfully paired. Node Id: 1
```

### A.4 Vérifier le commissioning

```bash
# Lire les attributs du device commissionné
# Exemple pour une prise/ampoule (cluster On/Off)
chip-tool onoff read on-off <NODE_ID> 1

# Si le device répond, le commissioning est réussi
# Sortie attendue :
# [TOO] Response Failure: IM Error 0x0000054F: General error: 0x54f (UNSUPPORTED_ACCESS)
#   ou une valeur 0x00 (OFF) / 0x01 (ON)
```

### A.5 Partager le device avec Homey SHS (Multi-Admin)

```bash
# Générer un code de partage (setup code) pour Homey SHS
chip-tool admin open-commissioning-window <NODE_ID> 1   --option 1 --window-timeout 300

# Résultat : un QR code et un code PIN de partage
# Exemple :
# [TOO] SetupQRCode: MT:Y.K90... 
# [TOO] SetupManualCode: 1234-567-8901
```

Dans **Homey SHS** :
1. **Appareils** -> **+** -> **Matter**
2. Saisissez le **code de partage** généré par chip-tool (QR ou manuel)
3. Homey SHS découvre le device en IPv6 via mDNS et finalise l'appairage

### A.6 Commandes chip-tool utiles

```bash
# Lister les fabrics (administrateurs) d'un device
chip-tool operationalcredentials read fabrics <NODE_ID> 1

# Retirer un fabric (si besoin de révoquer un accès)
chip-tool operationalcredentials remove-fabric <FABRIC_INDEX> <NODE_ID> 1

# Lire les informations du device
chip-tool basicinformation read product-name <NODE_ID> 1
chip-tool basicinformation read software-version <NODE_ID> 1

# Contrôler une prise/ampoule
chip-tool onoff on <NODE_ID> 1
chip-tool onoff off <NODE_ID> 1
chip-tool onoff toggle <NODE_ID> 1

# Lire un capteur de température
chip-tool temperaturemeasurement read measured-value <NODE_ID> 1
```

### A.7 Désinstallation / mise à jour

```bash
# Via snap
sudo snap remove chip-tool
sudo snap refresh chip-tool  # mise à jour

# Compilation manuelle
rm -rf connectedhomeip/
# Re-cloner et re-compiler pour mettre à jour
```

---

## Option B : matterjs-server (successeur de python-matter-server)

### B.1 Contexte

Le projet `python-matter-server` a été **archivé en juin 2026** et n'est plus maintenu. Son successeur officiel est **matterjs-server**, réécrit en JavaScript/Node.js par l'Open Home Foundation.

| Caractéristique | Détail |
|-----------------|--------|
| **Projet source** | `matter-js/matterjs-server` |
| **Langage** | JavaScript / Node.js |
| **Statut** | Activement maintenu, successeur officiel |
| **Fonctions** | Serveur Matter complet avec API WebSocket |
| **Avantage clé** | API haut niveau, intégration facile avec Home Assistant |

### B.2 Installation sur le Pi5

#### Méthode 1 : Via npm

```bash
# 1. Installer Node.js 18+ (si ce n'est pas déjà fait)
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt install -y nodejs

# 2. Vérifier Node.js
node --version  # v20.x.x
npm --version   # 10.x.x

# 3. Installer matterjs-server
npm install -g matterjs-server

# 4. Vérifier
matterjs-server --version
```

#### Méthode 2 : Via Docker

```bash
# 1. Installer Docker
sudo apt install docker.io
sudo usermod -aG docker $USER
# Déconnectez-vous et reconnectez-vous

# 2. Télécharger l'image
# Note : remplacez par l'image officielle quand disponible
docker pull matterjs/matter-server:latest

# 3. Lancer le serveur
docker run -d --name matter-server \
  --net=host \
  -v ~/matter-data:/data \
  matterjs/matter-server:latest
```

### B.3 Démarrer le serveur

```bash
# Mode interactif (pour test)
matterjs-server --storage-path ~/matter-data

# Mode daemon (production)
nohup matterjs-server --storage-path ~/matter-data > ~/matter-server.log 2>&1 &

# Ou service systemd
sudo tee /etc/systemd/system/matterjs-server.service > /dev/null <<EOF
[Unit]
Description=MatterJS Server
After=network.target bluetooth.target

[Service]
Type=simple
User=pi5compute
WorkingDirectory=/home/pi5compute
ExecStart=/usr/bin/matterjs-server --storage-path /home/pi5compute/matter-data
Restart=always

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable matterjs-server
sudo systemctl start matterjs-server
```

### B.4 Commissioning via l'API WebSocket

Le serveur expose une API WebSocket sur le port par défaut (généralement 5580).

**Exemple avec curl / websocat :**
```bash
# Installer websocat
sudo apt install websocat

# Se connecter au serveur
websocat ws://127.0.0.1:5580/ws

# Envoyer une commande de commissioning (JSON)
# Le format exact dépend de la version de matterjs-server
# Référez-vous à la documentation officielle
```

**Exemple avec Node.js client :**
```javascript
const WebSocket = require('ws');

const ws = new WebSocket('ws://127.0.0.1:5580/ws');

ws.on('open', () => {
  // Fournir le Thread Dataset
  ws.send(JSON.stringify({
    command: 'set_thread_dataset',
    dataset: '0e080000000000010000000300001335...'
  }));

  // Commissionner un device
  ws.send(JSON.stringify({
    command: 'commission_with_code',
    code: '12345678'
  }));
});

ws.on('message', (data) => {
  console.log('Réponse:', JSON.parse(data));
});
```

> **Note** : L'API exacte de matterjs-server peut différer. Référez-vous à la [documentation officielle](https://github.com/matter-js/matterjs-server) pour les commandes précises.

### B.5 Intégration avec Homey SHS

1. Le serveur génère un **code de partage** après le commissioning
2. Dans **Homey SHS** : **Appareils** -> **+** -> **Matter**
3. Saisissez le code de partage
4. Homey SHS découvre le device en IPv6 via mDNS

---

## Comparaison des deux options

| Critère | chip-tool (Option A) | matterjs-server (Option B) |
|---------|----------------------|----------------------------|
| **Maintenance** | ✅ Active (CSA) | ✅ Active (Open Home Foundation) |
| **Complexité** | CLI simple | Serveur avec API |
| **Installation** | `snap install` (30 sec) | `npm install` (2-5 min) |
| **Besoin de compiler** | Non (snap) | Non |
| **Serveur permanent** | ❌ Non (CLI autonome) | ✅ Oui (doit tourner en fond) |
| **Multi-admin** | ✅ Natif | ✅ Via API |
| **BLE commissioning** | ✅ Direct | ✅ Via API |
| **Thread Dataset manuel** | ✅ | ✅ |
| **Intégration Homey SHS** | ✅ (code de partage) | ✅ (code de partage) |
| **Documentation** | ✅ Excellente (CSA) | ✅ Bonne |
| **Recommandation** | **🥇 RECOMMANDÉ** | Alternative viable |

---

## Procédure complète d'onboarding (résumé)

### Étape 1 : Préparer le device

- **ESP32-C61** : Appui long sur le bouton BOOT (6-10 secondes) jusqu'à ce que la LED clignote
- **IKEA KAJPLATS / GRILLPLATS** : Cycle d'alimentation 6x (allumer/éteindre) ou appui sur le bouton de reset selon la doc IKEA
- Le device entre en mode **commissioning BLE** (émet un signal Bluetooth Low Energy)

### Étape 2 : Récupérer le Thread Dataset

```bash
# Sur le SMLight (SSH)
ot-ctl dataset active -x
# Copiez la chaîne hexadécimale complète
```

### Étape 3 : Commissionner via le Pi5

**Avec chip-tool (RECOMMANDÉ) :**
```bash
# Installation (une fois)
sudo snap install chip-tool

# Commissioning
chip-tool pairing ble-thread 1 \
  hex:<DATASET_TLV> \
  <PIN_CODE_MATTER> \
  <DISCRIMINATOR>

# Générer le code de partage pour Homey SHS
chip-tool admin open-commissioning-window 1 1 \
  --option 1 --window-timeout 300
```

**OU avec matterjs-server :**
```bash
# Installation (une fois)
npm install -g matterjs-server

# Démarrer le serveur
matterjs-server --storage-path ~/matter-data

# Commissionner via l'API WebSocket
# (voir la documentation officielle pour les commandes exactes)
```

### Étape 4 : Vérifier que le device est sur le réseau

```bash
# Sur le SMLight
ot-ctl router table
# Le device doit apparaître

ot-ctl srp server service
# Le service Matter du device doit être enregistré

# Sur le Mac-Mini
dns-sd -B _matter._tcp local
# Le device doit apparaître dans la liste mDNS
```

### Étape 5 : Ajouter à Homey SHS

1. Dans **Homey SHS** : **Appareils** -> **+** -> **Matter**
2. Saisissez le **code de partage** généré par chip-tool ou matterjs-server
3. Homey SHS découvre le device en IPv6 et finalise l'appairage

### Étape 6 : Exposition vers Apple HomeKit (optionnel)

**Via Homey SHS (HomeKit expérimental) :**
1. **Paramètres** -> **Expérimental** -> **HomeKit**
2. Activez l'intégration
3. Un **QR code de pont HomeKit** est généré pour Homey SHS
4. Dans l'app **Maison** iOS : **+** -> **Ajouter un accessoire** -> scanner le QR code de Homey
5. Tous les devices Homey apparaissent dans Maison

---

## Dépannage

| Symptôme | Cause probable | Solution |
|----------|---------------|----------|
| `chip-tool` ne trouve pas le device en BLE | Device trop loin ou pas en mode commissioning | Rapprocher (< 1m), vérifier LED clignotante |
| `pairing ble-thread` échoue | Dataset TLV incorrect | Vérifier `ot-ctl dataset active -x` sur SMLight |
| "Thread dataset not found" | Dataset mal formaté | Vérifier le préfixe `hex:` et la chaîne complète |
| Device commissionné mais pas visible mDNS | SRP non enregistré sur SMLight | Vérifier `ot-ctl srp server service` |
| Homey SHS ne découvre pas le device | mDNS bloqué entre SMLight et Mac-Mini | Vérifier `dns-sd -B _matter._tcp` sur Mac-Mini |
| "No compatible Thread network" | Homey tente de récupérer credentials du téléphone | Ignorer, utiliser le code de partage multi-admin |
| Commissioning échoue à "Network setup" | Routage IPv6 cassé | Vérifier RA IPv6, multicast, firewall inter-VLAN |
| `bluetoothctl scan on` ne trouve rien | Bluetooth non détecté | Vérifier `hciconfig -a`, `rfkill unblock all` |
| `snap install chip-tool` échoue | Snapd non installé | `sudo apt install snapd` |
| Compilation connectedhomeip échoue | Dépendances manquantes | Vérifier `libssl-dev`, `libdbus-1-dev`, etc. |

---

## Résumé des commandes clés

### SMLight (OTBR)

```bash
# État du mesh Thread
ot-ctl state

# Dataset actif (à copier pour le Pi5)
ot-ctl dataset active -x

# Démarrer le commissioner Thread (optionnel, si besoin)
ot-ctl commissioner start

# Voir les devices sur le mesh
ot-ctl router table
ot-ctl child table

# Services SRP/mDNS
ot-ctl srp server service

# Adresses IPv6
ot-ctl ipaddr

# Logs
sudo tail -f /var/log/otbr-agent.log
```

### Raspberry Pi 5 (Commissioner)

```bash
# === chip-tool (RECOMMANDÉ) ===

# Installation
sudo snap install chip-tool

# Commissioning
chip-tool pairing ble-thread <NODE_ID> hex:<DATASET> <PIN> <DISCRIMINATOR>

# Multi-admin pour Homey SHS
chip-tool admin open-commissioning-window <NODE_ID> 1 --option 1 --window-timeout 300

# Contrôle device
chip-tool onoff on <NODE_ID> 1
chip-tool onoff off <NODE_ID> 1

# === matterjs-server (Alternative) ===

# Installation
npm install -g matterjs-server

# Démarrer
matterjs-server --storage-path ~/matter-data

# Commissionner via WebSocket (voir doc officielle)
```

### Mac-Mini (Homey SHS)

```bash
# Vérifier IPv6
ifconfig | grep inet6

# Vérifier mDNS Matter
dns-sd -B _matter._tcp local

# Vérifier connectivité vers SMLight
ping6 <adresse_ipv6_smlight>
```

---

## Points critiques à retenir

1. **Homey SHS n'a pas de menu Thread** - il ne peut pas récupérer les credentials du téléphone. Le Pi5 sert d'intermédiaire.
2. **Le téléphone n'est PAS utilisé pour le commissioning Thread** - tout se fait via le Pi5 (BLE) + SMLight (Thread).
3. **Deux codes PIN distincts** :
   - **Code PIN Thread** -> pour rejoindre le mesh (rarement utilisé ici, chip-tool gère tout)
   - **Code PIN Matter** -> pour l'authentification CASE (sur le QR code du device)
4. **Le device doit être routable en IPv6** avant que Homey SHS ne puisse communiquer avec lui - le SMLight assure ce routage.
5. **Le mDNS est critique** - sans annonces `_matter._tcp`, Homey ne découvrira jamais le device. Vérifiez avec `dns-sd`.
6. **IPv6 est obligatoire** - Homey SHS (Mac-Mini) et SMLight doivent communiquer en IPv6. Vérifiez les adresses `fd00::` ou `fe80::`.
7. **Le Pi5 n'est pas nécessaire en permanence** - une fois le device partagé avec Homey SHS, le Pi5 peut être éteint. Il ne sert que pour le commissioning initial.
8. **python-matter-server est obsolète** - ne l'utilisez plus. Préférez chip-tool ou matterjs-server.

---

## Ressources

| Projet | Lien | Statut | Usage |
|--------|------|--------|-------|
| connectedhomeip (chip-tool) | https://github.com/project-chip/connectedhomeip | ✅ Actif | CLI commissioner officiel CSA |
| matterjs-server | https://github.com/matter-js/matterjs-server | ✅ Actif | Serveur Matter JS (successeur) |
| python-matter-server (archivé) | https://github.com/matter-js/python-matter-server | ❌ Archivé juin 2026 | Ne plus utiliser |
| Documentation Homey SHS Matter | https://support.homey.app/hc/en-us/articles/24629602740892 | ✅ Actif | Guide officiel Homey |
| OpenThread Border Router | https://openthread.io/guides/border-router | ✅ Actif | Guide OTBR |
| Matter Specification (CSA) | https://csa-iot.org/all-solutions/matter/ | ✅ Actif | Spécification officielle |

---

> **Licence** : Cette documentation est fournie à titre indicatif. Les marques Matter, Thread, Homey, Apple HomeKit, IKEA, ESP32 sont la propriété de leurs détenteurs respectifs.


---

## Annexe A : Intégration des XIAO nRF54LM20A (Thread Router + Bridge UART Toshiba CN22)

### A.1 Spécifications du XIAO nRF54LM20A

| Caractéristique | Valeur |
|-----------------|--------|
| **Processeur** | ARM Cortex-M33 128 MHz + RISC-V 128 MHz coprocessor |
| **Mémoire** | 2 MB NVM (RRAM) + 512 KB RAM |
| **Radio** | Bluetooth LE 6.0, Thread, Zigbee, Matter, 802.15.4 |
| **Interfaces** | 1x UART, 1x I2C, 1x SPI, 1x NFC, 28x GPIO |
| **USB** | High-speed USB-C |
| **Alimentation** | 5V (USB-C) ou 3.7V batterie LiPo |
| **SDK** | nRF Connect SDK (Zephyr RTOS) |

### A.2 Architecture avec XIAO

```
+-----------------------------------------------------------------------------+
|                           RÉSEAU LOCAL (LAN)                                |
|                                                                             |
|  +---------------------+    +---------------------+    +----------------+   |
|  |  SMLight Nano MG24  |    |   Mac-Mini M4       |    |  Raspberry Pi5 |   |
|  |  (OTBR / TBR)       |<-->|   Homey SHS         |    |  (Commissioner |   |
|  |  - Thread mesh      |IPv6|   - Contrôleur      |    |   Matter)      |   |
|  |  - mDNS proxy       |    |     Matter          |    |  - BLE + IP    |   |
|  |  - IPv6 routing     |    |   - IPv6 actif      |    |  - chip-tool   |   |
|  +----------+----------+    +---------------------+    +----------------+   |
|             |                                                               |
|             |         Thread mesh                                           |
|             |    +-------------+  +-------------+  +--------------------+ |
|             +--->|  ESP32-C61  |  | IKEA        |  |  XIAO nRF54LM20A   | |
|             |    |  (End Dev)  |  | KAJPLATS    |  |  - Thread Router   | |
|             |    |             |  | (End Dev)   |  |  - UART Bridge     | |
|             |    +-------------+  +-------------+  |  - Toshiba CN22    | |
|             |                                      |    (via UART)      | |
|             |                                      +----------+---------+ |
|             |                                                 |           |
|             |                                      +----------v----------+ |
|             |                                      |  Toshiba AC         | |
|             |                                      |  (CN22 UART port)   | |
|             |                                      +---------------------+ |
|             |                                                               |
|             |    +--------------------+                                       |
|             +--->|  XIAO nRF54LM20A   |  (autre instance, End Device        |
|                  |  - Thread End Device |   ou Router selon besoin)         |
|                  |  - Capteur / Acteur  |                                   |
|                  +--------------------+                                       |
|                                                                             |
+-----------------------------------------------------------------------------+
```

### A.3 Phase 1 : Flasher le XIAO comme Thread Router (FTD)

#### A.3.1 Prérequis développement

```bash
# Sur votre PC de développement (Linux/macOS/Windows avec WSL)
# Installer nRF Connect SDK et toolchain

# 1. Installer nrfutil
pip3 install --user -U west
curl https://files.nordicsemi.com/artifactory/swtools/external/nrfutil/executables/x86_64-unknown-linux-gnu/nrfutil -o nrfutil
chmod +x nrfutil
sudo mv nrfutil /usr/local/bin/

# 2. Installer device tools
nrfutil install device
nrfutil install nrf5sdk-tools
nrfutil install toolchain-manager

# 3. Installer toolchain nRF Connect SDK v2.9.1
nrfutil toolchain-manager install --ncs-version v2.9.1

# 4. Initialiser le SDK
mkdir -p ~/nrfconnect && cd ~/nrfconnect
west init -m https://github.com/nrfconnect/sdk-nrf --mr v2.9.1
west update
west zephyr-export
```

#### A.3.2 Créer le projet OpenThread Router (FTD)

```bash
# Lancer l'environnement toolchain
nrfutil toolchain-manager launch --shell

# Aller dans le dossier des exemples OpenThread
cd ~/nrfconnect/nrf/samples/openthread/cli

# Créer une copie pour le XIAO nRF54LM20A Router
cp -r ~/nrfconnect/nrf/samples/openthread/cli ~/nrfconnect/nrf/samples/openthread/xiao-router
cd ~/nrfconnect/nrf/samples/openthread/xiao-router
```

#### A.3.3 Configuration du projet (prj.conf)

Créez/modifiez le fichier `prj.conf` :

```conf
# OpenThread Full Thread Device (Router)
CONFIG_OPENTHREAD_FTD=y
CONFIG_OPENTHREAD_THREAD_VERSION_1_3=y

# Commissioning support
CONFIG_OPENTHREAD_COMMISSIONER=y
CONFIG_OPENTHREAD_JOINER=y

# UART pour CLI OpenThread
CONFIG_OPENTHREAD_CLI_UART=y

# UART pour le bridge Toshiba (UART20 sur XIAO)
CONFIG_UART_ASYNC_API=y
CONFIG_NRFX_UARTE20=y

# Stack sizes
CONFIG_MAIN_STACK_SIZE=2048
CONFIG_OPENTHREAD_THREAD_STACK_SIZE=8192

# Logging
CONFIG_LOG=y
CONFIG_OPENTHREAD_LOG_LEVEL_INFO=y

# Réseau
CONFIG_NET_IPV6=y
CONFIG_NET_IPV6_NBR_CACHE=y
CONFIG_NET_IPV6_MLD=y
```

#### A.3.4 Device Tree Overlay (xiao_nrf54lm20a.overlay)

Créez le fichier `boards/xiao_nrf54lm20a_nrf54lm20a_cpuapp.overlay` :

```dts
&uart20 {
    current-speed = <9600>;
    status = "okay";
    hw-flow-control;
};

/ {
    chosen {
        zephyr,ot-uart = &uart20;
        zephyr,shell-uart = &uart20;
    };
};

&pmic_i2c {
    status = "disabled";
};

&pmic {
    status = "disabled";
    charger { status = "disabled"; };
    regulators { status = "disabled"; };
};
```

#### A.3.5 Compiler et flasher

```bash
# Configurer et compiler
west build -b xiao_nrf54lm20a/nrf54lm20a/cpuapp

# Flasher sur le XIAO (connecté en USB-C)
west flash
```

### A.4 Phase 2 : Intégration au réseau Thread existant

#### A.4.1 Récupérer le Dataset du SMLight

```bash
# Sur le SMLight (SSH)
ot-ctl dataset active -x
# Copiez la chaîne hexadécimale complète
```

#### A.4.2 Joindre le XIAO au réseau Thread

Connectez-vous au CLI OpenThread du XIAO via le port série USB :

```bash
# Sur votre PC, trouver le port série du XIAO
ls /dev/ttyACM*  # Linux
# ou
ls /dev/cu.usbmodem*  # macOS

# Se connecter avec minicom ou screen
minicom -D /dev/ttyACM0 -b 115200
```

Dans le CLI OpenThread du XIAO :

```bash
# Vérifier l'état
> state
disabled

# Configurer le dataset du réseau existant
> dataset set active 0e080000000000010000000300001335060004001fffe00208fe...

# Vérifier le dataset
> dataset
Active Timestamp: 1
Channel: 15
Channel Mask: 07fff800
Ext PAN ID: e68d05794bf13052
Mesh Local Prefix: fd7d:ddf7:877b:8756/64
Network Key: a77fe1d03b0e8028a4e13213de38080e
Network Name: OpenThread-8f37
PAN ID: 0x8f37
PSKc: f9debbc1532487984b17f92cd55b21fc
Done

# Activer l'interface radio
> ifconfig up
Done

# Démarrer Thread
> thread start
Done

# Vérifier l'état (devient router après quelques minutes)
> state
child
...
> state
router
Done

# Vérifier les adresses IPv6
> ipaddr
fd7d:ddf7:877b:8756:0:ff:fe00:fc00    # ALOC (Anycast Locator)
fd7d:ddf7:877b:8756:0:ff:fe00:fc10    # ALOC
fd7d:ddf7:877b:8756:0:ff:fe00:fc11    # ALOC
fd7d:ddf7:877b:8756:0:ff:fe00:fc38    # ALOC
fd7d:ddf7:877b:8756:0:ff:fe00:4001    # RLOC (Routing Locator)
fe80:0:0:0:ec0b:dcff:fe5c:fcba        # Link-Local
Done
```

### A.5 Phase 3 : Bridge UART Toshiba CN22

#### A.5.1 Schéma de connexion CN22 -> XIAO

```
Toshiba AC (CN22)                    XIAO nRF54LM20A
+-------------+                     +-------------+
| Pin 1 (TX)  |------->| RX (P0.xx)  |
| Pin 2 (GND) |-------| GND          |
| Pin 3 (+5V) |-------| Vin (5V)     |
| Pin 4 (RX)  |<------| TX (P0.yy)  |
| Pin 5 (NC)  |       |              |
+-------------+       +-------------+

⚠️ ATTENTION : Ne PAS connecter le Pin 5 (rose extérieur) !
⚠️ Débrancher l'AC du secteur avant de connecter/déconnecter !
```

#### A.5.2 Paramètres UART Toshiba CN22

| Paramètre | Valeur |
|-----------|--------|
| **Baud rate** | 9600 |
| **Parité** | EVEN |
| **Bits de données** | 8 |
| **Bit d'arrêt** | 1 |
| **Niveau logique** | 5V TTL (XIAO = 3.3V -> nécessite level shifter) |

#### A.5.3 Code d'application UART Bridge

Ajoutez à votre projet Zephyr une tâche UART bridge. Créez `src/toshiba_uart_bridge.c` :

```c
#include <zephyr/kernel.h>
#include <zephyr/drivers/uart.h>
#include <zephyr/logging/log.h>

LOG_MODULE_REGISTER(toshiba_bridge, LOG_LEVEL_INF);

#define UART_DEVICE_NODE DT_NODELABEL(uart20)
static const struct device *uart_dev = DEVICE_DT_GET(UART_DEVICE_NODE);

static uint8_t rx_buf[64];
static uint8_t tx_buf[64];

// Frame Toshiba CN22 protocol
// Format: [Header][Command][Data][Checksum]
#define TOSHIBA_HEADER 0xF2
#define TOSHIBA_CMD_STATUS 0x01
#define TOSHIBA_CMD_CONTROL 0x02

static void uart_callback(const struct device *dev, struct uart_event *evt, void *user_data)
{
    switch (evt->type) {
    case UART_RX_RDY:
        // Données reçues du Toshiba AC
        LOG_INF("RX %d bytes from AC", evt->data.rx.len);
        // Traiter la trame et publier via Thread/CoAP
        break;
    case UART_RX_DISABLED:
        uart_rx_enable(dev, rx_buf, sizeof(rx_buf), 100);
        break;
    default:
        break;
    }
}

int toshiba_bridge_init(void)
{
    if (!device_is_ready(uart_dev)) {
        LOG_ERR("UART device not ready");
        return -ENODEV;
    }

    // Configurer UART 9600, EVEN parity
    struct uart_config uart_cfg = {
        .baudrate = 9600,
        .parity = UART_CFG_PARITY_EVEN,
        .stop_bits = UART_CFG_STOP_BITS_1,
        .data_bits = UART_CFG_DATA_BITS_8,
        .flow_ctrl = UART_CFG_FLOW_CTRL_NONE,
    };

    uart_configure(uart_dev, &uart_cfg);
    uart_callback_set(uart_dev, uart_callback, NULL);
    uart_rx_enable(uart_dev, rx_buf, sizeof(rx_buf), 100);

    LOG_INF("Toshiba UART bridge initialized");
    return 0;
}

// Envoyer une commande au Toshiba AC
int toshiba_send_command(uint8_t cmd, uint8_t *data, uint8_t len)
{
    uint8_t frame[32];
    uint8_t idx = 0;

    frame[idx++] = TOSHIBA_HEADER;
    frame[idx++] = cmd;
    frame[idx++] = len;

    for (int i = 0; i < len; i++) {
        frame[idx++] = data[i];
    }

    // Calculer checksum
    uint8_t checksum = 0;
    for (int i = 0; i < idx; i++) {
        checksum += frame[i];
    }
    frame[idx++] = checksum;

    return uart_tx(uart_dev, frame, idx, SYS_FOREVER_MS);
}

SYS_INIT(toshiba_bridge_init, APPLICATION, CONFIG_KERNEL_INIT_PRIORITY_DEVICE);
```

### A.6 Phase 4 : Exposition vers Homey SHS

#### A.6.1 Option A : Matter-over-Thread natif

Le XIAO nRF54LM20A supporte Matter. Vous pouvez créer un **device Matter personnalisé** (ex: thermostat/climate) qui :
- Communique en UART avec le Toshiba AC
- Expose les commandes via le cluster Matter Thermostat
- Est commissionné dans Homey SHS

**Problème** : Nécessite de développer un device Matter complet (firmware + clusters).

#### A.6.2 Option C : MQTT/CoAP via Thread (intermédiaire)

Le XIAO publie les données du Toshiba AC sur le réseau Thread via **CoAP** ou **MQTT-SN** :
- Homey SHS ou Home Assistant souscrit aux topics
- Pas besoin de device Matter complet

### A.7 Résumé du processus XIAO

| Phase | Action | Où ? |
|-------|--------|------|
| 1 | Installer nRF Connect SDK + toolchain | PC de développement |
| 2 | Créer projet OpenThread FTD Router | PC de développement |
| 3 | Configurer UART20 pour Toshiba CN22 | PC de développement |
| 4 | Compiler et flasher le XIAO | PC -> XIAO (USB-C) |
| 5 | Récupérer Thread Dataset du SMLight | SMLight (SSH) |
| 6 | Joindre le XIAO au réseau Thread | XIAO (CLI via USB) |
| 7 | Connecter CN22 Toshiba au XIAO | Hardware |
| 8 | Développer l'application UART bridge | PC de développement |
| 9 | Exposer vers Homey SHS | Homey SHS (Matter ou MQTT) |

### A.8 Points critiques

| Point | Détail |
|-------|--------|
| **Level shifter 5V -> 3.3V** | Obligatoire entre CN22 (5V) et XIAO (3.3V) |
| **Pin 5 CN22** | Ne JAMAIS le connecter (risque de dommage) |
| **Débrancher AC** | Toujours débrancher l'AC du secteur avant branchement |
| **Baud rate** | 9600, parité EVEN (pas négociable) |
| **Alimentation XIAO** | 5V via Vin (pas 3.3V) pour le level shifter |
