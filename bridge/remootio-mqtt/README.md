# remootio-mqtt — pont Remootio (garage) → MQTT

Petit service Node.js (à héberger **sur le Pi5 existant**, à côté du broker Mosquitto)
qui garde une session WebSocket locale chiffrée ouverte vers un ou plusieurs
appareils **Remootio** et republie l'état + accepte des commandes via **MQTT**.

## Pourquoi

L'app Homey Remootio maintient elle-même la connexion WebSocket vers l'appareil
et se déconnecte souvent. Ce pont tourne en continu à côté du broker (reconnexion
auto de la librairie `remootio-api-client`), et Homey n'a plus qu'à s'abonner à un
topic MQTT via un app générique (bien plus fiable).

Prototype Node.js d'abord (librairie officielle `remootio-api-client`, qui gère
déjà le protocole chiffré AES-CBC + HMAC-SHA256) ; portage Rust prévu ensuite
pour rester cohérent avec le reste de la stack.

## Contrat MQTT

```text
santuario/remootio/<name>/state              {"state": "open"|"closed"|"no sensor", "ts": <epoch>}   (retained)
santuario/remootio/<name>/availability       online | offline                                          (retained)
santuario/remootio/<name>/set                open | close | trigger | query   (commande, à publier)
santuario/remootio/<name>/secondary/set      trigger                          (commande sortie 2, à publier)
santuario/remootio/bridge/availability       online | offline   (LWT du pont lui-même)
```

`<name>` = nom configuré dans `[[devices]]` (ex. `garage`).

L'API Remootio ne remonte l'**état** (`state`, événement `StateChange`) que pour la
sortie **principale** (Output 1, `.../set`) — jamais pour la sortie **secondaire**
(Output 2, `.../secondary/set`), qui n'accepte qu'une commande, sans retour d'état
possible via cette API, même si un capteur y est câblé physiquement. `open`/`close`
restent supportés par le pont sur la sortie principale en plus de `trigger`, pour
d'éventuels boîtiers Remootio configurés en mode open/close (toggle "Use open/close
config" activé dans l'app) — sur ce genre de config `open`/`close` sont les actions
pertinentes ; sur un boîtier en pur mode impulsion (voir ci-dessous) c'est `trigger`
qu'il faut utiliser.

Côté Homey : app MQTT générique — carte "quand un message arrive sur `santuario/remootio/garage/state`"
pour le trigger, et carte "publier `trigger` (ou `open`/`close` selon le boîtier) sur
`santuario/remootio/garage/set`" pour l'action (idem `trigger` sur
`santuario/remootio/garage/secondary/set` pour la sortie 2).

## Cette installation (192.168.1.228, `name = "garage"`)

| Sortie | Usage physique | Config app Remootio | Capteur position | Commande MQTT correcte |
| --- | --- | --- | --- | --- |
| **1 (principale)** | Porte de garage | impulse ctrl, "Use open/close config" **désactivé** | aucun (`state` = toujours `"no sensor"`) | `trigger` sur `.../set` |
| **2 (secondaire)** | Gâche électrique | **free output** (⚠ pas "impulse ctrl", voir plus bas) | aucun retour possible (API) | `trigger` sur `.../secondary/set` |

Validé en conditions réelles le 2026-07-15 :

- `trigger` sur `.../set` → reproduit exactement le bouton de l'app (2 impulsions de
  2 s, réglage "Give two impulses" de l'app respecté).
- `open` sur `.../set` → ne déclenche qu'**une seule** impulsion (le réglage "Give two
  impulses" n'est pas appliqué à cette action quand "Use open/close config" est
  désactivé) → **ne pas utiliser `open`/`close` sur ce boîtier**, seulement `trigger`.
- La gâche électrique (sortie 2) doit être pilotée en **impulsion courte** (déverrouillage
  bref, comme un bouton d'interphone) — pas en maintien temporisé — donc `trigger` sur
  `.../secondary/set` (→ `sendTriggerSecondary()`), jamais `holdTriggerSecondaryOutputActive()`.
- **`TRIGGER_SECONDARY` exige Output 2 = "free output"** dans l'app Remootio (Settings >
  Output configuration). Avec Output 2 en "impulse ctrl" (config initiale, cf. capture
  d'écran de l'app), le boîtier **rejette** la commande :
  `{"type":"TRIGGER_SECONDARY","success":false,"errorCode":"ERR_INVALID_REQUEST"}` — la
  frame est bien reçue/authentifiée, mais l'action est invalide pour ce mode de sortie.
  Après bascule sur "free output" dans l'app : `{"success":true,"relayTriggered":true}`,
  gâche déclenchée avec succès. Ce n'est pas un bug du pont — le boîtier lui-même
  distingue "impulse ctrl" (sortie 1, contrôle principal) de "free output" (seul mode
  où la sortie 2 est pilotable via l'API smart-home/automatisations).
- Ni l'app Remootio ni l'API n'ont de capteur de position calibré sur aucune des deux
  sorties : `state` reste `"no sensor"` en permanence, sur les deux sorties. C'est un
  état de configuration/câblage, pas un bug du pont — Homey ne peut donc suivre que la
  *disponibilité* du pont, pas la position réelle de la porte/gâche.

## Installation

```bash
cd bridge/remootio-mqtt
npm install
cp config.example.toml config.toml   # puis éditer : IP + API Secret Key + API Auth Key
node index.js
```

Les clés API s'obtiennent dans l'app Remootio : appareil > Settings > API Secret Key / API Auth Key.

## Déploiement systemd (Pi5)

Unité fournie : `contrib/remootio-mqtt.service` (démarre après `mosquitto-broker`).
Config déployée en `/etc/daly-bms/remootio-mqtt.toml`.

```bash
sudo cp config.toml /etc/daly-bms/remootio-mqtt.toml
sudo cp contrib/remootio-mqtt.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now remootio-mqtt
```

## Sécurité (règle projet #12)

- `config.toml` (API Secret Key + API Auth Key = contrôle physique du garage) →
  **jamais** commité (seul `config.example.toml` l'est, `.gitignore` local).
- Ouvrir/fermer un portail est une action sensible : si une ACL Mosquitto est mise
  en place un jour, restreindre `santuario/remootio/#` aux clients qui en ont besoin.
