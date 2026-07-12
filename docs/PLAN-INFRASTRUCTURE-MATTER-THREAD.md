# Plan de mise en œuvre — Infrastructure Matter over Thread (Santuario)

> **Objectif :** permettre à Homey Self-Hosted Server (Mac Mini) de découvrir et commander automatiquement des appareils Matter-over-Thread construits sur des XIAO nRF54LM20A, via un OpenThread Border Router (OTBR) hébergé sur le Raspberry Pi 5.
>
> **Date de rédaction :** 12 juillet 2026
> **Statut :** plan validé ; **outillage Mac Mini + Pi5 préparé** (2026-07-12, sans le
> matériel radio) ; en attente de réception matériel (SMHUB Nano MG24, XIAO nRF54LM20A)
> pour la suite (Phases 2, 4-7).

---

## 0. Journal de préparation — 2026-07-12 (lire en premier pour reprendre le travail)

> Section à mettre à jour à chaque session de travail sur ce projet. Objectif : qu'une
> nouvelle session (sans mémoire des échanges précédents) puisse repartir sur une base
> documentaire fiable sans avoir à tout redécouvrir.

### ✅ Fait

#### Accès & outillage (préalable, réutilisable pour tout le projet)

- SSH Claude Code → Pi5 : clé dédiée `~/.ssh/id_pi5_claude` (sans passphrase, hors trousseau
  macOS — la clé perso `id_ed25519` bloque tout usage non-interactif). Config dans
  `~/.ssh/config` du Mac Mini (`Host pi5`). Voir CLAUDE.md §2.
- SSH Claude Code → GitHub (push) : clé dédiée `~/.ssh/id_github_claude`, ajoutée en
  **Deploy Key** (write access) sur `github.com/thieryus007-cloud/Daly-BMS-Rust/settings/keys`.
  Remote `origin` basculé de HTTPS à SSH (`git@github.com:...`).

#### Mac Mini (confirmé être le "Mac Mini" du plan — Apple M4, hostname `Mac-Mini.local`)

- `Homey.app` + `Homey Self-Hosted Server.app` : déjà installés, app macOS native (pas Docker).
- nRF Connect for Desktop, VS Code (extension nRF Connect **déjà présente** avant la session).
- SDK NCS v3.2.1 installé (`/opt/nordic/ncs/v3.2.1`, ~11 Go) via `~/bin/nrfutil`
  (téléchargé directement depuis `files.nordicsemi.com` — **le cask Homebrew `nrfutil` est
  cassé par Gatekeeper**, ne pas l'utiliser).
- **Découverte technique importante** : `xiao_nrf54lm20a` n'est **pas** un board NCS officiel
  (contrairement à ce que ce document affirmait initialement en §3.4/§4.1). Nécessite le repo
  tiers `Seeed-Studio/platform-seeedboards` en `BOARD_ROOT`. Son devicetree référence un nom
  de fichier HAL Nordic incorrect pour NCS v3.2.1 (corrigé localement : voir §4.1). Après ce
  correctif, un **problème Kconfig non résolu** bloque encore la compilation (warnings
  traités en erreur) — **premier point à reprendre** dès que le XIAO est en main (essayer une
  version NCS plus récente, ou une révision plus récente de `platform-seeedboards`).

#### Pi5

- `sudo apt update && upgrade` puis **reboot appliqué** — kernel actif `6.18.34+rpt-rpi-2712`.
  Tout vérifié fonctionnel après coup (`daly-bms`, `energy-manager`, `mosquitto-broker`,
  `grafana-server` actifs, healthcheck 200, RS485 opérationnel).
- OTBR natif (`ot-br-posix`) installé via `scripts/setup-otbr-pi5.sh` (script commité,
  idempotent) : backbone `wlan0` (pas `eth0`, DOWN sur ce Pi5), interface web sur le port
  **8083** (évite 8080=daly-bms / 8081=energy-manager / 80=nginx). Services `otbr-agent` et
  `otbr-web` **désactivés + arrêtés volontairement** (pas de XIAO RCP branché — évite un
  crash-loop, cf. incident CLAUDE.md §8).
- **Effet de bord découvert et corrigé** : le bootstrap OTBR installe `bind9` (requis DNS64),
  qui entre en conflit port 53 avec `dnsmasq` (déjà présent mais non configuré). `dnsmasq`
  désactivé, aucun impact fonctionnel (DNS vérifié OK via `named`). Le correctif est dans le
  script, donc automatique lors d'une restauration.
- Pont FP2 (`bridge/aqara-fp2-mqtt/`) préparé : venv Python + `aiohomekit` installés,
  `config.toml` + service systemd déployés en `disabled/inactive` (prêt pour `discover`/`pair`
  dès réception des capteurs FP2).
- **Sauvegarde image disque complète** ajoutée (`scripts/backup-sdcard-pi5.sh` +
  `pi5-sdcard-backup.timer`, hebdomadaire) : la carte microSD du Pi5 (OS, `~/ot-br-posix`,
  tout ce qui est « local seulement ») est maintenant imagée sur le NVMe. Premier run manuel
  validé : 29 Go → 4,3 Go compressé (zstd), 3 min, aucun service perturbé. Détails CLAUDE.md §0.
- **Piège découvert** : le Pi5 était resté checkout sur une ancienne branche Claude déjà
  fusionnée (`claude/homey-mqtt-device-setup-3ljz66`) → `make sync` « réussissait » sans jamais
  rapatrier les commits poussés sur `main` (6 commits de cette session étaient invisibles côté
  Pi5 jusqu'à correction). Corrigé (`git checkout main`). **Toujours vérifier la branche du Pi5
  après un push notable** — procédure dans CLAUDE.md §3.

#### Documentation / Git

- Ce document, `CLAUDE.md` et `docs/Infrastructure-Thread.md` mis à jour et **poussés sur
  GitHub** (`origin/main`, commits `3dff342`..`943be54`, voir `git log`).
- Clarification actée : `docs/Infrastructure-Thread.md` n'est **pas** un brouillon obsolète —
  c'est un **second projet parallèle** (pilotage des Toshiba par XIAO en Matter-over-Thread
  **direct**, sans MQTT), à mener en parallèle du firmware ESP32+MQTT existant
  (`firmware/toshiba-suzumi-rs/`). Les deux partagent la même infra OTBR/Pi5.
- CLAUDE.md contient une section dédiée « ⚠️ État Git — ce qui est poussé vs ce qui reste
  local » qui liste, machine par machine, chaque artefact non commité (builds, venv, secrets,
  clés SSH) et sa commande de reconstruction exacte. **Toujours vérifier là en premier** avant
  de supposer qu'un fichier existe sur une machine.

### ⏳ Reste à faire

1. **Bloqué sur matériel** : réception du SMHUB Nano MG24 et des XIAO nRF54LM20A.
2. **Dès le XIAO en main** : résoudre le blocage Kconfig de `platform-seeedboards` (voir
   ci-dessus) avant de pouvoir compiler/flasher quoi que ce soit dessus.
3. Phase 2 du plan (§5) : flasher le XIAO#1 en firmware RCP, le brancher sur le Pi5, vérifier
   `/dev/ttyACM0`, puis **activer** `otbr-agent`/`otbr-web` (`sudo systemctl enable --now
   otbr-agent otbr-web` — actuellement désactivés, cf. ci-dessus).
4. Phases 4-7 (§7-§10) : former le réseau Thread, vérifier la découverte par Homey, flasher
   les XIAO endpoints, commissioning **depuis un téléphone Android** (contrainte iOS, §3.3).
5. FP2 : une fois les capteurs reçus, `discover`/`pair` via `bridge/aqara-fp2-mqtt`, puis
   `[energy_manager.toshiba_ac] control_enabled = true` dans `Config.toml` (cf.
   `docs/toshiba-bridges.md` §4).
6. Second projet (`docs/Infrastructure-Thread.md`, XIAO en Matter-over-Thread direct pour
   piloter les Toshiba) : pas encore démarré, à planifier séparément quand décidé.

### ⚠️ Points d'attention pour la prochaine session

- Les identités SSH créées cette session (`id_pi5_claude`, `id_github_claude`) sont **propres
  à ce Mac Mini** — si une session tourne sur une autre machine, il faudra soit régénérer des
  clés dédiées, soit copier les clés privées existantes (jamais les committer).
- `otbr-agent`/`otbr-web`/`aqara-fp2-mqtt` sont **désactivés par conception** — ne pas
  s'étonner qu'ils soient `inactive` : c'est l'état attendu tant que le matériel n'est pas là.
- Avant de relancer une install/compilation, toujours consulter la section « État Git » de
  CLAUDE.md pour savoir ce qui doit être reconstruit localement vs ce qui vient de `git pull`.

---

## Table des matières

1. [Architecture cible](#1-architecture-cible)
2. [Inventaire matériel et logiciel](#2-inventaire-matériel-et-logiciel)
3. [Points techniques vérifiés et contraintes](#3-points-techniques-vérifiés-et-contraintes)
4. [Phase 1 — Préparation de l'environnement de développement nRF Connect SDK](#4-phase-1--préparation-de-lenvironnement-de-développement-nrf-connect-sdk)
5. [Phase 2 — Flasher le XIAO n°1 en firmware RCP (Radio Co-Processor)](#5-phase-2--flasher-le-xiao-n1-en-firmware-rcp-radio-co-processor)
6. [Phase 3 — Installation OTBR native sur le Raspberry Pi 5](#6-phase-3--installation-otbr-native-sur-le-raspberry-pi-5)
7. [Phase 4 — Formation du réseau Thread](#7-phase-4--formation-du-réseau-thread)
8. [Phase 5 — Vérification de la découverte du TBR par Homey](#8-phase-5--vérification-de-la-découverte-du-tbr-par-homey)
9. [Phase 6 — Flasher les XIAO endpoints en firmware Matter](#9-phase-6--flasher-les-xiao-endpoints-en-firmware-matter)
10. [Phase 7 — Commissioning des appareils Matter via Android](#10-phase-7--commissioning-des-appareils-matter-via-android)
11. [Phase 8 — Rôle du SMHUB Nano MG24 (Zigbee)](#11-phase-8--rôle-du-smhub-nano-mg24-zigbee)
12. [Dépannage](#12-dépannage)
13. [Évolutions futures](#13-évolutions-futures)
14. [Références](#14-références)

---

## 1. Architecture cible

```
┌─────────────────────────────────────────────────────────────────┐
│                        LAN Ethernet (Santuario)                  │
│                                                                  │
│  ┌──────────────┐    ┌──────────────────┐   ┌────────────────┐  │
│  │   Mac Mini   │    │  Raspberry Pi 5  │   │ SMHUB Nano MG24│  │
│  │  Homey SHS   │◄──►│  OTBR (natif)    │   │ Zigbee (Z2M +  │  │
│  │ (app macOS   │    │  otbr-agent      │   │  Matterbridge) │  │
│  │  native)     │    │  + apps Rust     │   │                │  │
│  └──────────────┘    └────────┬─────────┘   └───────┬────────┘  │
│         ▲                     │ USB                 │ 802.15.4  │
│         │                     ▼                     ▼  Zigbee   │
│         │            ┌─────────────────┐    ┌──────────────┐    │
│         │            │ XIAO nRF54LM20A │    │  Appareils   │    │
│         │            │  #1 (RCP fw)    │    │  Zigbee      │    │
│         │            └────────┬────────┘    └──────────────┘    │
│         │                     │ Radio 802.15.4 / Thread         │
│         │                     ▼                                 │
│         │        ┌───────────────────────────┐                  │
│         │        │     Réseau mesh Thread    │                  │
│         │        │  XIAO #2  XIAO #3  XIAO #N│                  │
│         │        │  (Matter endpoints)       │                  │
│         │        └───────────────────────────┘                  │
│         │                     ▲                                 │
│         └─────────────────────┘                                 │
│           Matter over Thread (IPv6, via OTBR)                   │
│                                                                  │
│  ┌──────────────┐                                                │
│  │   Android    │  ← Commissioning UNIQUEMENT (scan QR code)    │
│  │  (app Homey) │     puis contrôle depuis n'importe où          │
│  └──────────────┘                                                │
└─────────────────────────────────────────────────────────────────┘
```

**Séparation des rôles :**

| Composant | Rôle | Protocole radio |
|---|---|---|
| Raspberry Pi 5 + XIAO #1 (RCP) | Thread Border Router (OTBR) | Thread (802.15.4) |
| XIAO #2 … #N | Endpoints Matter-over-Thread | Thread (802.15.4) |
| SMHUB Nano MG24 | Coordinateur Zigbee + bridge Matter-over-IP | Zigbee (802.15.4) |
| Mac Mini | Homey SHS (contrôleur Matter) | — (IP uniquement) |
| Téléphone Android | Commissioning Matter (pairing initial) | BLE + IP |
| iPad | Contrôle quotidien via app Homey | — (IP uniquement) |

---

## 2. Inventaire matériel et logiciel

### Matériel

| Élément | État | Détails |
|---|---|---|
| Mac Mini (Apple Silicon) | ✅ en service | Héberge Homey SHS (app macOS native, macOS 26+) + Docker Desktop |
| Raspberry Pi 5 | ✅ en service | < 2 % CPU, 14 % RAM occupés, apps Rust, 1 port USB libre |
| SMHUB Nano MG24 | 🚚 en livraison | SoC SG2000 (RISC-V/ARM), radio EFR32MG24, SMHUB-OS Linux |
| XIAO nRF54LM20A (× N) | 🚚 en livraison | nRF54LM20A : Cortex-M33 128 MHz + RISC-V FLPR, 2 MB NVM, 512 KB RAM, PMIC nPM1300, flash externe PY25Q64 |
| Téléphone Android | ✅ disponible | Pour le commissioning Matter uniquement |
| iPad | ✅ en service | Contrôle quotidien via app Homey |

### Logiciel (versions au moment de la rédaction)

| Logiciel | Version / source |
|---|---|
| Homey Self-Hosted Server | App macOS native (menu bar) |
| nRF Connect SDK | v3.2.x ou supérieur (support XIAO nRF54LM20A confirmé sur v3.2.1) |
| VS Code + extension nRF Connect | Dernière version stable |
| ot-br-posix | branche `main`, GitHub openthread/ot-br-posix |
| Raspberry Pi OS | Bookworm 64-bit (NetworkManager par défaut) |

---

## 3. Points techniques vérifiés et contraintes

Ces points ont été vérifiés par recherche documentaire (juillet 2026). Ils conditionnent les choix d'architecture.

### 3.1 OTBR ne fonctionne pas nativement sur macOS

`ot-br-posix` dépend de composants Linux (systemd, netfilter/ipset, interface TUN `wpan0`, sysctl IPv6). Les tentatives de compilation sur macOS échouent (issue GitHub #1285). Docker Desktop sur macOS n'est pas une alternative fiable : le mode `--network host` s'applique à la VM Linux interne et non au Mac, ce qui casse le mDNS (port 5353 UDP) indispensable à la découverte Thread/Matter. **→ D'où le choix du Raspberry Pi 5.**

### 3.2 Homey SHS ne fournit pas de Thread Border Router

Homey SHS supporte Matter-over-Thread **uniquement si un TBR est déjà présent sur le LAN** (documentation officielle Homey, janvier 2026). Homey découvre les TBR via mDNS (`_meshcop._udp`) et affiche l'état dans l'app mobile : *Plus (…) → Réglages → Thread*.

### 3.3 Contrainte iOS/iPadOS sur les credentials Thread

Lors du commissioning Matter depuis un iPad, Homey délègue l'opération au framework Matter d'Apple (dialogue natif « Ajouter un accessoire »). Pour Matter-over-Thread, iOS utilise **exclusivement** les credentials Thread de son propre credential store, alimenté uniquement par les Apple Border Routers (HomePod mini, Apple TV 4K). Un OTBR tiers (RPi5, SMHUB) **ne peut pas** injecter ses credentials dans ce store. Confirmé par la documentation Homey : « Currently, Homey Pro's Thread network cannot be added to iOS ».

**→ Le commissioning se fera depuis le téléphone Android.** Sur Android, l'app Homey peut configurer le réseau Thread automatiquement (Homey Mobile App ≥ v7.6.1) et le partage de credentials Thread des OTBR tiers est possible via l'API Thread d'Android. Après le pairing, le contrôle quotidien se fait depuis n'importe quel appareil (iPad, web).

### 3.4 Spécificités matérielles du XIAO nRF54LM20A (vs cartes Nordic officielles)

https://nrfconnectdocs.nordicsemi.com/ncs/3.2.1/zephyr/boards/nordic/nrf54lm20dk/doc/index.html#nrf54lm20dk


Vérifié sur le wiki Seeed (`xiao_nrf54lm20a_with_matter`, mis à jour 26 mai 2026) :

- **Flash externe différent :** PY25Q64 (Puya, 64 Mbit SPI NOR) au lieu du MX25R64 des cartes d'évaluation Nordic → overlays et tables de partitions spécifiques obligatoires.
- **PMIC nPM1300 sur I2C :** doit être **désactivé** dans le firmware RCP pour éviter les conflits (`&pmic_i2c { status = "disabled"; }` + `CONFIG_MFD_NPM13XX=n`).
- **UART pour le RCP :** `uart20` à **1 000 000 bauds** avec hardware flow control.
- **Arduino IDE non supporté** : uniquement nRF Connect SDK, PlatformIO, Zephyr RTOS.
- **Antenne externe recommandée** (connecteur IPEX4) pour la qualité radio.
- Board target Zephyr : `xiao_nrf54lm20a/nrf54lm20a/cpuapp`.

### 3.5 Support RCP nRF54L : point de vigilance

Le support RCP de la série nRF54L est plus récent que celui du nRF52840. Des incompatibilités ont été signalées entre le firmware RCP nRF54L15 et l'image Docker `openthread/otbr` générique (Nordic DevZone) ; Nordic recommande son image `nrfconnect/otbr` en Docker. **En installation native** (notre cas), utiliser un `ot-br-posix` récent et vérifier la compatibilité de version d'API Spinel. Le wiki Seeed valide le XIAO nRF54LM20A en RCP avec l'add-on OTBR de Home Assistant, ce qui confirme la faisabilité de la chaîne RCP → otbr-agent.

**Plan B si le RCP nRF54LM20A pose problème :** un dongle nRF52840 (~15 €) est la référence absolue pour ce rôle et fonctionne immédiatement.

### 3.6 Matterbridge n'est PAS un Matter Controller

Le Matterbridge du SMHUB expose des appareils **non-Matter** (Zigbee via Z2M, etc.) **comme** devices Matter-over-IP. Il ne peut pas commissionner ni « relayer » des appareils qui sont déjà Matter. Un appareil Matter-over-Thread se commissionne directement par un contrôleur Matter (Homey) via l'OTBR — pas besoin de bridge intermédiaire.

### 3.7 Exigences réseau (commissioning Matter)

- Le border router, le contrôleur (Homey/Mac Mini) et le téléphone de commissioning doivent être sur le **même sous-réseau/VLAN**.
- **mDNS (UDP 5353)** doit circuler librement — c'est la cause n°1 d'échec de commissioning.
- **IPv6 activé** sur le LAN (link-local suffit ; pas besoin d'IPv6 opérateur).
- Connecter le RPi5 et le Mac Mini en **Ethernet** (pas WiFi) pour la fiabilité.

---

## 4. Phase 1 — Préparation de l'environnement de développement nRF Connect SDK

> **Machine de développement :** le Mac Mini convient parfaitement (nRF Connect SDK supporte macOS Apple Silicon).

### 4.1 Installer les outils

1. Installer **nRF Connect for Desktop** : https://www.nordicsemi.com/Products/Development-tools/nrf-connect-for-desktop
2. Depuis nRF Connect for Desktop, installer le **Toolchain Manager** puis le **nRF Connect SDK v3.2.x** (ou plus récent).
3. Installer **VS Code** + extension **nRF Connect for VS Code**.
4. Vérifier la présence du board target :

```bash
west boards | grep xiao_nrf54lm20a
```

Résultat attendu : `xiao_nrf54lm20a` listé. Si absent, mettre à jour le SDK (le support XIAO nRF54LM20A est présent dans Zephyr upstream et NCS ≥ 3.2).

> **⚠️ Correctif constaté (2026-07-12, préparation sans matériel sur NCS v3.2.1)** : contrairement
> à ce qui précède, `xiao_nrf54lm20a` n'apparaît **pas** dans `west boards` d'une installation
> NCS standard — le board n'est **pas mergé en amont**. Il faut cloner le repo tiers
> **`Seeed-Studio/platform-seeedboards`** et le passer en `-DBOARD_ROOT=<clone>/zephyr` (ou
> l'ajouter aux « Board Roots » de l'extension VS Code). Une fois ajouté, deux frictions
> observées avec NCS v3.2.1 :
>
> 1. Le devicetree Seeed inclut `<nordic/nrf54lm20a_cpuapp.dtsi>`, qui n'existe pas dans le HAL
>    v3.2.1 — le fichier réel s'appelle `nrf54lm20a_enga_cpuapp.dtsi` (suffixe « enga » = silicium
>    ingénierie A). Corriger l'include dans le clone local règle la résolution DTS.
> 2. Après ce correctif, le `defconfig` Seeed déclenche des **warnings Kconfig traités en erreur**
>    (`NULL_POINTER_EXCEPTION_DETECTION_NONE`, dépendance `NRFX_GRTC`) — non résolu à ce stade,
>    probablement un decalage de version entre le `platform-seeedboards` et NCS v3.2.1/Zephyr
>    4.2.99. À creuser une fois le XIAO physique disponible (essayer une version NCS plus
>    récente — v3.2.5 ou v3.4.0 disponibles via `nrfutil sdk-manager search` — ou une révision
>    plus récente de `platform-seeedboards`).
>
> Le SoC nRF54LM20A lui-même **est** bien supporté nativement par NCS v3.2.1 (board officiel
> `nrf54lm20dk` présent dans `west boards`) — c'est uniquement l'empreinte carte XIAO (pinout,
> régulateur, flash externe PY25Q64) qui manque en amont.
>
> Outillage installé (Mac Mini, 2026-07-12) : nRF Connect for Desktop + VS Code (extension nRF
> Connect déjà présente) via Homebrew ; SDK NCS v3.2.1 installé via `nrfutil sdk-manager install
> v3.2.1` dans `/opt/nordic/ncs/v3.2.1`. **Le cask Homebrew `nrfutil` est cassé par Gatekeeper**
> (binaire supprimé après install, cask déprécié) — utiliser `~/bin/nrfutil` téléchargé directement
> depuis `files.nordicsemi.com` (voir CLAUDE.md § nRF Connect SDK).

### 4.2 Test de validation de la chaîne

Compiler et flasher un `hello_world` sur un XIAO pour valider la chaîne complète :

```bash
west build -b xiao_nrf54lm20a/nrf54lm20a/cpuapp samples/hello_world
west flash
```

Le XIAO intègre un SAMD11 avec CMSIS-DAP : le flash et le debug se font directement par le port USB, sans programmateur externe.

**Critère de réussite Phase 1 :** `Hello World! xiao_nrf54lm20a` visible sur le terminal série (USB CDC ACM).

---

## 5. Phase 2 — Flasher le XIAO n°1 en firmware RCP (Radio Co-Processor)

> Le XIAO n°1 devient la radio 802.15.4 du border router. La pile Thread complète tourne sur le RPi5 (otbr-agent) ; le XIAO n'exécute que le sub-MAC et le PHY via le protocole Spinel.

### 5.1 Créer le projet

Dans VS Code / nRF Connect : **Create a new application → Copy a sample →** rechercher **Coprocessor** (`samples/net/openthread/coprocessor`).

### 5.2 Ajouter les fichiers de configuration spécifiques XIAO

Créer le dossier `boards/` à la racine du projet et y ajouter :

**`boards/xiao_nrf54lm20a_nrf54lm20a_cpuapp.overlay`** :

```dts
&uart20 {
	current-speed = <1000000>;
	status = "okay";
	hw-flow-control;
};

/ {
	chosen {
		zephyr,ot-uart = &uart20;
	};
};

&pmic_i2c {
	status = "disabled";
};
```

**`boards/xiao_nrf54lm20a_nrf54lm20a_cpuapp.conf`** :

```conf
CONFIG_SPI_NOR=n

# Increase Main and shell stack sizes to avoid stack overflow
# while using CRACEN
CONFIG_MAIN_STACK_SIZE=2048

CONFIG_I2C_GPIO=n
CONFIG_MFD_NPM13XX=n
CONFIG_NPM13XX_CHARGER=n
```

> **Pourquoi :** désactivation du flash SPI externe et du PMIC nPM1300 (inutiles en RCP, sources de conflits), augmentation de la stack pour le moteur crypto CRACEN du nRF54L.

### 5.3 Compiler en configuration RCP

Le sample coprocessor se décline en NCP (par défaut) ou RCP. **Utiliser la variante RCP** (recommandée par OpenThread) :

```bash
west build -b xiao_nrf54lm20a/nrf54lm20a/cpuapp \
  samples/net/openthread/coprocessor \
  -- -DEXTRA_CONF_FILE=overlay-rcp.conf
```

> Via l'interface graphique VS Code : ajouter `overlay-rcp.conf` dans « Extra Kconfig fragments » lors du Build Configuration.

### 5.4 Flasher

```bash
west flash
```

### 5.5 Vérifier

Brancher le XIAO n°1 sur le RPi5 (port USB libre) et vérifier :

```bash
ls -l /dev/ttyACM*
```

Résultat attendu : `/dev/ttyACM0` présent (ou `ttyACM1` — noter le bon numéro).

```bash
udevadm info /dev/ttyACM0 | grep -i -E "vendor|model"
```

**Critère de réussite Phase 2 :** le XIAO apparaît comme périphérique série USB CDC ACM sur le RPi5.

### 5.6 (Recommandé) Règle udev pour un nom de port stable

Comme pour les FTDI du système Victron, fixer le port par numéro de série pour survivre aux reboots :

```bash
udevadm info -a /dev/ttyACM0 | grep -E "serial|idVendor|idProduct" | head -6
```

Puis créer `/etc/udev/rules.d/99-otbr-rcp.rules` :

```
SUBSYSTEM=="tty", ATTRS{idVendor}=="XXXX", ATTRS{idProduct}=="YYYY", ATTRS{serial}=="ZZZZ", SYMLINK+="ttyOTBR"
```

(Remplacer XXXX/YYYY/ZZZZ par les valeurs relevées.) Recharger :

```bash
sudo udevadm control --reload-rules && sudo udevadm trigger
ls -l /dev/ttyOTBR
```

---

## 6. Phase 3 — Installation OTBR native sur le Raspberry Pi 5

> Installation **native** (sans Docker) : c'est la configuration de référence du projet OpenThread, la plus stable et la plus documentée. Empreinte : < 100 MB RAM, CPU négligeable — sans impact sur les applications Rust existantes.
>
> **✅ Déjà fait (2026-07-12)**, sans le XIAO RCP (matériel pas encore livré) : `ot-br-posix`
> cloné + compilé + installé dans `~/ot-br-posix` sur le Pi5, avec l'interface web (`WEB_GUI=1`).
> **Reproductible/restaurable depuis GitHub** via `sudo bash scripts/setup-otbr-pi5.sh`
> (script idempotent, commité dans ce repo — encode exactement les étapes ci-dessous +
> les deux adaptations suivantes). Deux adaptations par rapport aux étapes manuelles ci-dessous :
>
> - **Backbone = `wlan0`, pas `eth0`** — `eth0` est DOWN/NO-CARRIER sur ce Pi5 (le Pi5 tourne
>   en WiFi, cf. CLAUDE.md §2). `INFRA_IF_NAME=wlan0 ./script/setup`.
> - **`otbr-web` sur le port 8083** (`/etc/default/otbr-web` → `OTBR_WEB_OPTS="-I wpan0 -p
>   8083"`) pour ne pas entrer en conflit avec `daly-bms-server` (8080), `energy-manager`
>   (8081) ou `nginx` (80) déjà en écoute sur ce Pi5.
>
> Les deux services (`otbr-agent`, `otbr-web`) sont **désactivés et arrêtés** volontairement
> (`systemctl disable/stop`) : sans XIAO RCP branché, le chemin radio configuré
> (`/dev/ttyACM0`) n'existe pas → risque de boucle crash-restart sur un Pi5 de production (cf.
> incident CLAUDE.md §8). **Ne pas activer** avant la Phase 2 (flash + branchement du XIAO#1).

### 6.1 Prérequis

```bash
sudo apt update && sudo apt upgrade -y
sudo apt install -y git
```

Identifier l'interface réseau d'infrastructure (Ethernet) :

```bash
ip -br link show
```

Sur RPi OS Bookworm, l'interface Ethernet s'appelle généralement `eth0`. **Noter le nom exact** — il est requis pour le setup.

> ⚠️ **Point Bookworm :** RPi OS Bookworm utilise NetworkManager (et non dhcpcd) comme gestionnaire réseau par défaut. Le script `setup` d'ot-br-posix gère cette configuration, mais si un avertissement lié à dhcpcd apparaît, il est bénin tant que NetworkManager gère `eth0`. Ne pas installer dhcpcd5 (cela casserait des paquets système RPi OS).

### 6.2 Cloner et compiler

```bash
cd ~
git clone --recursive --depth=1 https://github.com/openthread/ot-br-posix
cd ot-br-posix
./script/bootstrap
```

Puis compiler et installer (⚠️ **bien spécifier l'interface Ethernet**, la valeur par défaut est `wlan0`) :

```bash
INFRA_IF_NAME=eth0 ./script/setup
```

Durée : 15–30 minutes de compilation sur RPi5.

### 6.3 Configurer le port série du RCP

Éditer la configuration de l'agent :

```bash
sudo nano /etc/default/otbr-agent
```

Localiser la variable `OTBR_AGENT_OPTS` et vérifier/ajuster la Radio URL :

```
OTBR_AGENT_OPTS="-I wpan0 -B eth0 spinel+hdlc+uart:///dev/ttyACM0?uart-baudrate=1000000"
```

> - `/dev/ttyACM0` : remplacer par `/dev/ttyOTBR` si la règle udev de la Phase 2.6 a été créée.
> - `?uart-baudrate=1000000` : **obligatoire** — correspond au `current-speed = <1000000>` du firmware RCP.
> - `-B eth0` : interface backbone (infrastructure).

### 6.4 Démarrer et activer le service

```bash
sudo systemctl daemon-reload
sudo service otbr-agent restart
sudo service otbr-agent status
```

Résultat attendu :

```
● otbr-agent.service - Border Router Agent
     Loaded: loaded (/lib/systemd/system/otbr-agent.service; enabled; ...)
     Active: active (running) since ...
```

Le service est activé au boot par le script setup (`systemctl enable otbr-agent`).

### 6.5 Vérifier l'interface Thread et l'interface web Attention interface 8080 deja utilisée sur le Pi5.

```bash
ip -br link show wpan0
sudo ot-ctl state
```

`wpan0` doit exister ; `ot-ctl state` répond `disabled` (normal, le réseau n'est pas encore formé).

Interface web OTBR : `http://<IP-du-RPi5>:80` (ou `:8080` selon la version du paquet otbr-web). configurer un autre port.

**Critère de réussite Phase 3 :** service `otbr-agent` actif, interface `wpan0` créée, `ot-ctl` répond.

---

## 7. Phase 4 — Formation du réseau Thread

### 7.1 Former le réseau (en ligne de commande — recommandé)

```bash
sudo ot-ctl dataset init new
sudo ot-ctl dataset networkname Santuario-Thread
sudo ot-ctl dataset channel 25
sudo ot-ctl dataset commit active
sudo ot-ctl ifconfig up
sudo ot-ctl thread start
```

> **Choix du canal :** 15, 20 ou 25 pour minimiser le chevauchement avec les canaux WiFi 1/6/11. Vérifier l'occupation locale (le réseau UniFi U6 Mesh Pro et le Zigbee du SMHUB partagent la bande 2,4 GHz). Si le SMHUB Zigbee utilise le canal Zigbee 11–15, choisir le canal Thread 25 pour éloigner les deux réseaux 802.15.4.

Vérifier :

```bash
sudo ot-ctl state
```

Résultat attendu : `leader` (premier et unique border router du réseau).

### 7.2 Sauvegarder le dataset (CRITIQUE)

```bash
sudo ot-ctl dataset active -x
```

**Copier la chaîne hexadécimale de sortie dans un endroit sûr** (gestionnaire de mots de passe). Ce dataset contient la clé réseau Thread : il permet de reconstruire le border router sans re-commissionner tous les appareils en cas de panne du RPi5 ou de la carte SD.

### 7.3 Vérifier l'annonce mDNS

Depuis le Mac Mini :

```bash
dns-sd -B _meshcop._udp local.
```

Résultat attendu : une instance annoncée par le RPi5 (le service Border Agent).

**Critère de réussite Phase 4 :** état `leader`, dataset sauvegardé, service `_meshcop._udp` visible depuis le Mac Mini.

---

## 8. Phase 5 — Vérification de la découverte du TBR par Homey

1. Ouvrir l'app **Homey mobile** (le téléphone doit être sur le même LAN/WiFi que le Mac Mini et le RPi5).
2. Aller dans **Plus (…) → Réglages → Thread**.
3. Le réseau `Santuario-Thread` (border router du RPi5) doit apparaître comme réseau Thread disponible.

> Si « Homey's Thread network unavailable » : vérifier que le téléphone est sur le même réseau local, que le mDNS n'est pas filtré (paramètre IGMP/mDNS snooping sur l'UniFi), et que RPi5 + Mac Mini sont sur le même sous-réseau.

**Sur Android spécifiquement :** l'app Homey (≥ v7.6.1) peut configurer automatiquement le réseau Thread sur le téléphone **si aucun autre réseau Thread n'a été configuré auparavant** sur cet appareil. Si un ancien réseau Thread (Google, etc.) traîne sur le téléphone, le supprimer dans les paramètres Google/Thread du téléphone avant de commencer.

**Critère de réussite Phase 5 :** le réseau Thread du RPi5 est visible dans l'app Homey sur Android.

---

## 9. Phase 6 — Flasher les XIAO endpoints en firmware Matter

> Chaque XIAO n°2…N devient un appareil Matter-over-Thread. Le wiki Seeed documente le sample **Matter-Template** ; pour un type d'appareil précis, partir du sample correspondant du NCS (`matter/light_bulb`, `matter/light_switch`, `matter/thermostat`, `matter/lock`, `matter/window_covering`…) en appliquant **les mêmes fichiers d'adaptation XIAO**.

### 9.1 Créer le projet

VS Code / nRF Connect : **Create a new application → Copy a sample →** rechercher **Matter-Template** (ou le sample Matter du type d'appareil voulu).

### 9.2 Choisir la variante de partitionnement

Deux variantes possibles (fichiers ci-dessous) :

| Variante | OTA secondary slot | Taille app | Usage recommandé |
|---|---|---|---|
| **External flash** (par défaut) | PY25Q64 (flash SPI externe) | ~1,9 MB (0x1E2800) | **Recommandé** — plus d'espace, OTA sans compression |
| Internal | RRAM interne | ~1,2 MB (0x125800) | Si le PY25Q64 doit être désactivé (économie d'énergie batterie) |

### 9.3 Fichiers à ajouter au projet (variante External flash)

**`boards/xiao_nrf54lm20a_nrf54lm20a_cpuapp.overlay`** :

```dts
/*
 * Copyright (c) 2025 Nordic Semiconductor ASA
 * SPDX-License-Identifier: LicenseRef-Nordic-5-Clause
 */

/ {
	chosen {
		nordic,pm-ext-flash = &py25q64;
	};

	aliases {
		/* Use watchdog wdt31 as the application watchdog */
		watchdog0 = &wdt31;
	};
};

&py25q64 {
	status = "okay";
};

&wdt31 {
	status = "okay";
};
```

**`sysbuild/mcuboot/boards/xiao_nrf54lm20a_nrf54lm20a_cpuapp.conf`** :

```conf
#
# Copyright (c) 2025 Nordic Semiconductor ASA
# SPDX-License-Identifier: LicenseRef-Nordic-5-Clause
#

CONFIG_HW_STACK_PROTECTION=n
CONFIG_BOOT_WATCHDOG_FEED=n

# XIAO nRF54LM20A uses SPI NOR external flash (PY25Q64)
CONFIG_GPIO=y
CONFIG_SPI=y
CONFIG_SPI_NOR=y
CONFIG_SPI_NOR_SFDP_DEVICETREE=y
CONFIG_SPI_NOR_FLASH_LAYOUT_PAGE_SIZE=4096

# Increase the maximum number of sectors to 512 to fit the big image size (> 1024 kB).
CONFIG_BOOT_MAX_IMG_SECTORS=512

# Enable tickless kernel for mcuboot (SYSCOUNTER reboot-time issue workaround).
CONFIG_TICKLESS_KERNEL=y
```

**`sysbuild/mcuboot/boards/xiao_nrf54lm20a_nrf54lm20a_cpuapp.overlay`** :

```dts
/*
 * Copyright (c) 2025 Nordic Semiconductor ASA
 * SPDX-License-Identifier: LicenseRef-Nordic-5-Clause
 */

/ {
	chosen {
		nordic,pm-ext-flash = &py25q64;
	};
};

&py25q64 {
	status = "okay";
};
```

**`pm_static_xiao_nrf54lm20a_nrf54lm20a_cpuapp.yml`** (racine du projet) :

```yml
mcuboot:
  address: 0x0
  region: flash_primary
  size: 0xD000
mcuboot_pad:
  address: 0xD000
  region: flash_primary
  size: 0x800
app:
  address: 0xD800
  region: flash_primary
  size: 0x1E2800
mcuboot_primary:
  address: 0xD000
  orig_span: &id001
  - app
  - mcuboot_pad
  region: flash_primary
  size: 0x1E3000
  span: *id001
mcuboot_primary_app:
  address: 0xD800
  orig_span: &id002
  - app
  region: flash_primary
  size: 0x1E2800
  span: *id002
factory_data:
  address: 0x1F0000
  region: flash_primary
  size: 0x1000
settings_storage:
  address: 0x1F1000
  region: flash_primary
  size: 0xC000
mcuboot_secondary:
  address: 0x0
  orig_span: &id003
  - mcuboot_secondary_pad
  - mcuboot_secondary_app
  region: external_flash
  size: 0x1E3000
  span: *id003
mcuboot_secondary_pad:
  region: external_flash
  address: 0x0
  size: 0x800
mcuboot_secondary_app:
  region: external_flash
  address: 0x800
  size: 0x1E2800
external_flash:
  address: 0x1E3000
  size: 0x5DB000
  device: PY25Q64
  region: external_flash
```

> Pour la variante Internal (fichiers `*_internal.*`), se référer au wiki Seeed (lien en références) — la structure est identique avec MCUboot réduit à 40 KB et le slot secondaire en RRAM.

### 9.4 Compiler et flasher

```bash
west build -b xiao_nrf54lm20a/nrf54lm20a/cpuapp <chemin-du-projet>
west flash
```

### 9.5 Récupérer le QR code de commissioning

1. Ouvrir le **nRF Serial Terminal** (VS Code) et sélectionner le port série du XIAO.
2. Au boot, les logs affichent les informations Matter du device et **le lien du QR code de pairing** :

```
[SVR] SetupQRCode: [MT:XXXXXXXXXXXXXX]
[SVR] https://project-chip.github.io/connectedhomeip/qrcode.html?data=MT%3AXXXX...
[SVR] Manual pairing code: [XXXXXXXXXXX]
```

3. Ouvrir le lien dans un navigateur pour afficher le QR code, **ou** noter le code manuel à 11 chiffres.

> Le device démarre automatiquement l'advertising BLE et attend le commissioning dans un réseau Thread compatible Matter.

**Critère de réussite Phase 6 :** firmware flashé, QR code et code manuel obtenus dans les logs série.

---

## 10. Phase 7 — Commissioning des appareils Matter via Android

> ⚠️ **Utiliser le téléphone Android exclusivement** pour cette phase (contrainte iOS documentée en §3.3). Après le pairing, le contrôle se fait depuis n'importe quel appareil.

### 10.1 Préparation

1. Installer/mettre à jour l'**app Homey** sur le téléphone Android (≥ v7.6.1).
2. Activer le **Bluetooth** (le commissioning Matter initial passe par BLE).
3. Vérifier que le téléphone est sur le **même WiFi/LAN** que le Mac Mini et le RPi5.
4. Vérifier **Plus (…) → Réglages → Thread** : le réseau `Santuario-Thread` doit être configuré/visible (cf. Phase 5).
5. Garder le XIAO **à proximité du téléphone** pendant le pairing (portée BLE).

### 10.2 Commissioning

1. App Homey → onglet **Devices** → **+ New Device** → **Matter**.
2. Suivre les instructions : « Please put your Matter device in 'pair mode' » → le XIAO fraîchement flashé/redémarré est déjà en mode pairing → **Continue**.
3. Scanner le **QR code** généré en Phase 6.5 (ou saisir le code manuel à 11 chiffres).
4. Le flux : connexion BLE → transfert des credentials Thread → le XIAO rejoint le réseau `Santuario-Thread` → vérification de connectivité → ajout à Homey.
5. Nommer l'appareil et l'assigner à une zone.

### 10.3 Vérification

- L'appareil apparaît dans Homey (Devices) et répond aux commandes depuis l'iPad et le web.
- Sur le RPi5, vérifier que le XIAO est un enfant/routeur du réseau :

```bash
sudo ot-ctl child table
sudo ot-ctl router table
sudo ot-ctl neighbor table
```

**Critère de réussite Phase 7 :** l'appareil Matter est contrôlable depuis Homey sur l'iPad ; il apparaît dans les tables Thread de l'OTBR.

### 10.4 Répéter pour chaque XIAO endpoint

Chaque nouveau XIAO : flasher (Phase 6) → commissionner (Phase 7). Compter ~5 minutes par appareil une fois le processus rôdé.

---

## 11. Phase 8 — Rôle du SMHUB Nano MG24 (Zigbee)

Le SMHUB **reste en firmware Zigbee** (configuration usine) et prend un rôle complémentaire, séparé du réseau Thread :

1. **Coordinateur Zigbee** via Zigbee2MQTT (pré-installé, tourne sur le SMHUB).
2. **Matterbridge + plugin matterbridge-z2m** (pré-installés) : expose les appareils Zigbee comme devices **Matter-over-IP** sur le LAN via un QR code.
3. **Commissioning dans Homey** : Devices → New Device → Matter → scanner le QR code affiché dans l'onglet Matterbridge du SMHUB. Comme c'est du Matter-over-IP (pas Thread), **ce pairing fonctionne aussi depuis l'iPad** — pas de contrainte de credentials Thread.

> Bénéfice : séparation propre des réseaux 802.15.4 — Thread (canal 25, RPi5) et Zigbee (canal 11–15, SMHUB) — et un point d'entrée Matter unique dans Homey pour tout le parc Zigbee.

> Option future : le SMHUB peut être reflashé en OTBR pour servir de **second border router Thread** (redondance/couverture), en le joignant au réseau existant avec le dataset sauvegardé en Phase 4.2. ⚠️ Attention : reflasher le SMHUB via Type-C peut effacer les données OTBR — toujours sauvegarder avant.

---

## 12. Dépannage

| Symptôme | Cause probable | Action |
|---|---|---|
| `otbr-agent` en échec `InvalidArguments` | Mauvais chemin RCP | Vérifier `/dev/ttyACM0` vs `ttyACM1` ; utiliser la règle udev (§5.6) |
| `otbr-agent` démarre puis crashe | Baudrate incorrect | Vérifier `?uart-baudrate=1000000` dans `/etc/default/otbr-agent` ET `current-speed = <1000000>` dans le firmware |
| Incompatibilité Spinel / RCP version mismatch dans les logs | ot-br-posix trop ancien ou trop récent vs firmware NCS | Recompiler ot-br-posix à jour ; en dernier recours, plan B nRF52840 (§3.5) |
| Réseau Thread invisible dans Homey | mDNS bloqué | Vérifier mDNS/IGMP snooping sur l'UniFi ; même sous-réseau pour RPi5, Mac Mini, téléphone |
| Commissioning bloqué sur « Searching » | Credentials Thread absents du téléphone | Sur Android : supprimer les anciens réseaux Thread, re-vérifier Réglages → Thread dans Homey ; ne PAS utiliser l'iPad pour Matter-over-Thread |
| Commissioning échoue en fin de process | IPv6 filtré sur le LAN | Vérifier qu'aucune règle firewall ne bloque IPv6 link-local/multicast |
| Device Matter pairé mais injoignable ensuite | Mesh Thread trop clairsemé | Ajouter des XIAO alimentés secteur (routeurs Thread) entre l'OTBR et les devices lointains |
| Le XIAO ne flashe plus | Protection NVM / état corrompu | Utiliser le script de factory reset Seeed (`scripts/factory_reset/`) |
| Interférences 2,4 GHz | WiFi/Zigbee/Thread superposés | Thread canal 25, Zigbee canal 11, WiFi canaux 1/6/11 ; éloigner physiquement RPi5, SMHUB et AP UniFi |
| Perte du RPi5 / carte SD | — | Réinstaller Phases 3–4 puis `sudo ot-ctl dataset set active <hex-sauvegardé>` → les devices re-rejoignent sans re-commissioning |

---

## 13. Évolutions futures

- **Second border router** : reflasher le SMHUB (ou un second RPi + XIAO) en OTBR et le joindre au réseau `Santuario-Thread` avec le dataset sauvegardé → redondance et extension de couverture (Thread 1.4 gère nativement le multi-BR).
- **Types de devices Matter** : décliner les endpoints XIAO selon les besoins Santuario — capteurs de température (cave batteries), contacts de porte, relais, thermostat. Vérifier que le cluster Matter correspondant existe dans la spec avant de choisir le sample NCS.
- **Intégration ESS** : exposer des mesures du système Victron/Daly comme devices Matter virtuels via le Matterbridge du SMHUB (plugin webhooks/MQTT) pour les voir dans Homey Energy.
- **OTA Matter** : la variante external flash (PY25Q64) est prête pour les mises à jour OTA des XIAO via Matter.
- **HomePod mini (optionnel)** : si un jour le commissioning direct depuis l'iPad devient souhaitable, un HomePod mini alimenterait le credential store Apple et pourrait coexister avec l'OTBR RPi5 sur le même réseau Thread.

---

## 14. Références

| Sujet | URL |
|---|---|
| Wiki Seeed — Matter for XIAO nRF54LM20A (overlays, pm_static, RCP) | https://wiki.seeedstudio.com/xiao_nrf54lm20a_with_matter/ |
| Wiki Seeed — Getting Started XIAO nRF54LM20A | https://wiki.seeedstudio.com/xiao_nrf54lm20a_getting_started/ |
| Zephyr — board xiao_nrf54lm20a | https://docs.zephyrproject.org/latest/boards/seeed/xiao_nrf54lm20a/doc/index.html |
| OpenThread — Border Router Native install | https://openthread.io/guides/border-router/build-native |
| OpenThread — coprocessor sample (Zephyr/NCS) | https://docs.zephyrproject.org/latest/samples/net/openthread/coprocessor/README.html |
| ot-br-posix (GitHub) | https://github.com/openthread/ot-br-posix |
| Homey — Matter-over-Thread avec Self-Hosted Server | https://support.homey.app/hc/en-us/articles/24629602740892 |
| Homey — Thread pour Homey Pro (contrainte iOS, Android auto-config) | https://support.homey.app/hc/en-us/articles/12010766903580 |
| Homey — installation SHS sur macOS | https://support.homey.app/hc/en-us/articles/23975134909724 |
| SMLight — SMHUB comme Thread Border Router | https://smlight.tech/support/manuals/books/smhub/page/using-smhub-as-thread-border-router-for-matter-devices |
| SMLight — SMHUB Nano MG24 (produit) | https://smlight.tech/global/smhub-nano-mg24 |
| Matterbridge (plugin manager Matter) | https://github.com/Luligu/matterbridge |
| Nordic DevZone — RCP nRF54L / OTBR Docker (point de vigilance) | https://devzone.nordicsemi.com/f/nordic-q-a/121150 |

---

*Document rédigé pour le projet Santuario — infrastructure domotique Matter/Thread/Zigbee. À placer dans `docs/` du dépôt Daly-BMS-Rust.*
