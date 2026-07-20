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

> **Important** : L'ESP32-C61 est un **End Device Thread**, pour les test ensuite il passera en Border Router pour redondance avec SMLight Nano MG24. Par la suite, des XIAO nRF54LM20A viendront completer le Mesh Thread comme end-devices et router.

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
