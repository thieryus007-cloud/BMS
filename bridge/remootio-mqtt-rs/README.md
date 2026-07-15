# remootio-mqtt-rs — pont Remootio (garage) → MQTT (Rust)

Portage Rust du prototype Node.js `bridge/remootio-mqtt/` — même contrat MQTT,
mêmes topics, même comportement validé en conditions réelles. Implémentation
1:1 du protocole `remootio-api-client` (WebSocket local chiffré AES-256-CBC +
HMAC-SHA256), sans dépendre du runtime Node.js sur le Pi5.

## Pourquoi un portage Rust

Le prototype Node.js (`bridge/remootio-mqtt/`) a servi à valider le protocole et
le comportement réel du boîtier (voir son README pour l'historique des tests).
Ce crate reprend exactement le même contrat MQTT pour rester cohérent avec le
reste de la stack (`daly-bms-server`, `energy-manager`, `dbus-mqtt-venus` sont
tous en Rust) — un seul runtime à maintenir sur le Pi5.

Crate **détaché** du workspace principal (`[workspace]` vide dans `Cargo.toml`,
même choix que `bridge/matter-toshiba-rs`) : zéro impact sur le build/CI de
`daly-bms`.

## Architecture

- `src/protocol.rs` — couche crypto **pure**, testable sur host sans réseau
  (`cargo test`) : construction/déchiffrement des frames `ENCRYPTED`
  (AES-256-CBC + HMAC-SHA256), calcul du prochain `action id`.
- `src/device.rs` — session WebSocket par appareil : connexion, authentification
  (frame `AUTH` → `CHALLENGE` → `sessionKey`), keepalive PING/PONG, réception
  des `StateChange`/réponses `QUERY`, envoi des commandes. Reconnexion
  automatique en boucle si la session tombe.
- `src/main.rs` — config TOML, client MQTT (`rumqttc`), routage des topics
  `.../set` et `.../secondary/set` vers le bon appareil, republication de
  l'état/disponibilité.

## Contrat MQTT (identique au prototype Node.js)

```text
santuario/remootio/<name>/state              {"state": "open"|"closed"|"no sensor", "ts": <epoch>}   (retained)
santuario/remootio/<name>/availability       online | offline                                          (retained)
santuario/remootio/<name>/set                open | close | trigger | query   (commande, à publier)
santuario/remootio/<name>/secondary/set      trigger                          (commande sortie 2, à publier)
santuario/remootio/bridge/availability       online | offline   (LWT du pont lui-même)
```

Voir `bridge/remootio-mqtt/README.md` §"Cette installation" pour la cartographie
réelle sortie 1 (garage, `trigger`) / sortie 2 (gâche électrique, `trigger` sur
`.../secondary/set`) validée en conditions réelles sur le boîtier 192.168.1.228.

> **Diagnostic intégré** : toute réponse `{"success":false,...}` du boîtier (ex.
> `ERR_INVALID_REQUEST` quand `TRIGGER_SECONDARY` est envoyé à une sortie configurée
> en "impulse ctrl" plutôt qu'en "free output" — cas vécu le 2026-07-15) est
> journalisée en `WARN` par `device.rs::handle_incoming` avec le payload complet du
> boîtier, sans rien à activer. Les réponses réussies restent en `DEBUG`
> (`RUST_LOG=debug` pour les voir).

## Tests

```bash
cargo test
```

Couvre uniquement `protocol.rs` (chiffrement/déchiffrement/MAC/rebouclage
d'`action id`) — aucune dépendance réseau, s'exécute sur n'importe quelle machine.

## Installation / build

```bash
cd bridge/remootio-mqtt-rs
cp config.example.toml config.toml   # puis éditer : IP + API Secret Key + API Auth Key
cargo build --release
RUST_LOG=info ./target/release/remootio-mqtt
```

## Déploiement systemd (Pi5)

Unité fournie : `contrib/remootio-mqtt-rs.service` (démarre après `mosquitto-broker`).
Config déployée en `/etc/daly-bms/remootio-mqtt.toml` (même fichier que la version
Node.js si vous migrez — format identique).

```bash
cargo build --release
sudo cp target/release/remootio-mqtt /usr/local/bin/
sudo cp config.toml /etc/daly-bms/remootio-mqtt.toml
sudo cp contrib/remootio-mqtt-rs.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now remootio-mqtt-rs
```

Si le prototype Node.js (`remootio-mqtt.service`) tourne déjà, l'arrêter/désactiver
avant de démarrer la version Rust pour éviter deux sessions WebSocket simultanées
vers le même boîtier :

```bash
sudo systemctl disable --now remootio-mqtt   # ancien service Node.js
```

## Sécurité (règle projet #12)

- `config.toml` (API Secret Key + API Auth Key = contrôle physique du garage
  et de la gâche électrique) → **jamais** commité (seul `config.example.toml`
  l'est, `.gitignore` local).
- Ouvrir/fermer un portail ou une gâche électrique est une action sensible :
  si une ACL Mosquitto est mise en place un jour, restreindre
  `santuario/remootio/#` aux clients qui en ont besoin.

## Limite connue (capteurs de position)

À ce jour (2026-07-15), aucun capteur de position n'est calibré sur le
boîtier de test : `state` reste `"no sensor"` en permanence sur les deux
sorties. Des capteurs magnétiques sont prévus prochainement sur les deux
sorties (confirmé : l'app Remootio propose bien deux entrées Sensor
indépendantes). **À revalider en conditions réelles une fois câblés** : les
réponses `QUERY`/`TRIGGER*` sont déjà journalisées en clair (`DEBUG`, ou `WARN`
si `success:false` — voir "Diagnostic intégré" ci-dessus), donc il suffira de
relancer avec `RUST_LOG=debug` pour voir si un second champ d'état apparaît
pour la sortie 2, ou si l'API Remootio ne remonte toujours qu'un seul `state`
global. Le contrat MQTT (`secondary/state` notamment) sera étendu à ce
moment-là si nécessaire — pas de topic spéculatif ajouté avant d'avoir vu le
payload réel.
