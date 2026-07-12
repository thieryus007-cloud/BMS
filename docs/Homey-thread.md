PLAN PROJET : Infrastructure Thread redondante avec 3 XIAO et Homey en Matter over Wi-Fi

1. Objectif et Architecture

L'objectif est de créer un réseau Thread robuste et redondant avec 3 XIAO, chacun étant un routeur Thread autonome et identique, contrôlant son propre Toshiba Shorei Edge via UART. Le Homey Pro (ou Self-Hosted), quant à lui, ne se connecte pas en Thread. Il communique avec chaque climatiseur via le protocole Matter sur votre réseau Wi-Fi.

· Avantage clé : Homey n'a plus besoin d'être physiquement connecté à un XIAO spécifique. Si un XIAO tombe en panne, le réseau Thread continue de fonctionner avec les deux autres, et Homey peut toujours joindre chaque appareil Matter via le Wi-Fi, rendant l'ensemble du système totalement résilient.

2. Inventaire du Matériel

· 3x XIAO nRF54LM20A Sense
· 3x Toshiba Shorei Edge avec port CN22
· 3x Level Shifter Bi-directionnel 4MOS IIC I2C (3.3V ↔ 5V)
· 1x Homey Pro (Serveur Self-Hosted) sur Mac-Mini
· 1x Réseau local (Wi-Fi et/ou Ethernet) stable

3. Schéma de l'Architecture

```
   ┌─────────────────────────────────────────────────────────────┐
   │                     RÉSEAU LOCAL (LAN)                     │
   │                    (Wi-Fi / Ethernet)                      │
   └───────────────┬───────────────────────┬─────────────────────┘
                   │ (Matter over Wi-Fi)   │ (Matter over Wi-Fi)
                   │                       │
     ┌─────────────▼───────────┐ ┌─────────▼─────────────────────┐
     │    Homey Pro (Controlleur │ │    Autre Appareil Matter     │
     │    Matter & Orchestrateur)│ │    (Ex: Téléphone, etc.)    │
     └───────────────────────────┘ └─────────────────────────────┘
                   │
                   │ (Communication Wi-Fi / Ethernet)
                   │
   ┌───────────────┼───────────────────────┬─────────────────────┐
   │               │                       │                     │
   ▼               ▼                       ▼                     ▼
┌──────────┐  ┌──────────┐           ┌──────────┐
│ XIAO #1  │  │ XIAO #2  │   ...     │ XIAO #3  │
│ (Routeur │  │ (Routeur │           │ (Routeur │
│  Thread) │  │  Thread) │           │  Thread) │
└────┬─────┘  └────┬─────┘           └────┬─────┘
     │ (UART)       │ (UART)              │ (UART)
     ▼              ▼                     ▼
┌──────────┐  ┌──────────┐           ┌──────────┐
│ Toshiba  │  │ Toshiba  │   ...     │ Toshiba  │
│   #1     │  │   #2     │           │   #3     │
└──────────┘  └──────────┘           └──────────┘
```

Fonctionnement :

· Les XIAO forment un réseau Thread maillé redondant.
· Chaque XIAO expose son climatiseur comme un appareil Matter-over-Wi-Fi.
· Homey, en tant que contrôleur Matter, découvre et contrôle ces appareils via le Wi-Fi, sans avoir à se soucier du réseau Thread sous-jacent.

4. Phase 1 : Préparation de l'Environnement

1. Installer VS Code et l'extension nRF Connect for VS Code.
2. Installer le nRF Connect SDK (NCS).
3. Connecter chaque XIAO en USB-C et vérifier la détection du port série.

5. Phase 2 : Flashage des XIAO (Configuration Identique)

Cette étape est cruciale : les 3 XIAO doivent avoir le même firmware.

1. Compiler le firmware : Utilisez un exemple Matter pour XIAO nRF54LM20A depuis le NCS.
   ```bash
   cd nrf/samples/matter/template  # Ou un exemple Matter approprié
   west build -b xiao_nrf54lm20a_sense
   ```
2. Flasher le firmware : west flash --runner jlink (à répéter pour chaque XIAO).

6. Phase 3 : Configuration du Réseau Thread

Un seul des trois XIAO doit créer le réseau. Les deux autres le rejoindront.

Étape 3.1 : Créer le réseau avec le XIAO #1

· Connectez-vous à la console série du XIAO #1.
· Utilisez les commandes OpenThread pour démarrer un nouveau réseau :
  ```bash
  > dataset init new
  > dataset commit active
  > ifconfig up
  > thread start
  ```
· Récupérez le Active Dataset (la clé du réseau) :
  ```bash
  > dataset active -x
  ```
  Notez précieusement cette chaîne hexadécimale.

Étape 3.2 : Rejoindre le réseau avec les XIAO #2 et #3

· Connectez-vous à la console de chaque XIAO.
· Injectez le Active Dataset récupéré précédemment pour qu'ils rejoignent le même réseau :
  ```bash
  > dataset set active <VOTRE_ACTIVE_DATASET_HEX>
  > dataset commit active
  > ifconfig up
  > thread start
  ```
· Vérifiez l'état : la commande state doit afficher router ou child.

7. Phase 4 : Interface UART et Firmware Matter

Cette partie est identique pour les 3 XIAO. Le firmware doit :

· Lire les données du Toshiba via UART.
· Exposer ces données en tant qu'appareil Matter-over-Wi-Fi.
· Recevoir les commandes Matter (via Wi-Fi) et les traduire en commandes UART pour le Toshiba.

Câblage (Level Shifter) : Il est impératif d'utiliser un level shifter entre le XIAO (3.3V) et le Toshiba (5V).

Borne Level Shifter Connexion
HV (5V) Alimentation du Toshiba (CN22)
LV (3.3V) Alimentation du XIAO (pin 3.3V)
HV1 / LV1 Ligne TX entre Toshiba et XIAO
HV2 / LV2 Ligne RX entre Toshiba et XIAO
GND Masse commun

8. Phase 5 : Intégration dans Homey (Matter over Wi-Fi)

1. Ajouter un appareil Matter dans Homey :
   · Dans l'application Homey, allez dans Appareils → Ajouter un appareil → Matter.
2. Suivre les instructions : Homey va scanner le réseau Wi-Fi et détecter les appareils Matter (vos XIAO).
3. Commissionner : Suivez les instructions à l'écran (code QR ou code PIN). La communication se fera alors via le Wi-Fi.
4. Répéter pour les 3 XIAO.

9. Phase 6 : Tests de Résilience

· Scénario 1 : Panne d'un XIAO.
  · Débranchez le XIAO #1.
  · Résultat attendu : Les XIAO #2 et #3 maintiennent le réseau Thread. Homey peut toujours contrôler les Toshiba #2 et #3 via le Wi-Fi. Le Toshiba #1 devient injoignable.
· Scénario 2 : Panne du réseau Thread.
  · Débranchez les 3 XIAO.
  · Résultat attendu : Tous les appareils deviennent injoignables. En rebranchant les XIAO, le réseau Thread se reforme automatiquement.
· Scénario 3 : Panne du Wi-Fi.
  · Débranchez votre box internet/Wi-Fi.
  · Résultat attendu : Homey ne peut plus communiquer avec les appareils Matter. Le réseau Thread interne continue de fonctionner entre les XIAO.

10. En Résumé

Cette architecture est plus simple et plus robuste :

· Homey n'a pas besoin de radio Thread.
· La redondance est maximale : la perte d'un XIAO n'affecte pas la communication avec les autres.
· Chaque XIAO est une unité autonome, facile à remplacer ou à dupliquer.
