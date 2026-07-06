# aqara-fp2-mqtt — pont présence FP2 → MQTT (HomeKit **local**, sans cloud)

Petit service Python (à héberger **sur le Pi5 existant**, aucun matériel supplémentaire)
qui lit la **présence** des capteurs **Aqara FP2** via **HomeKit en local** (`aiohomekit`,
sans Home Assistant ni cloud Aqara) et la publie en **MQTT** pour l'`energy-manager`.

> Contexte & décisions : `docs/toshiba-suzumi-rs-plan.md` **§18**.
> Le FP2 est en **WiFi + HomeKit** (pas Zigbee). HomeKit est un protocole **local/offline**.
> La liaison **cloud Aqara reste possible en parallèle** (l'appairage HomeKit est exclusif
> à un seul contrôleur : ce pont **ou** Apple Home — pas les deux).

## Contrat MQTT (retained)

```
santuario/toshiba/presence/<zone>              {"present": true|false, "ts": <epoch>}
santuario/toshiba/presence/bridge/availability online | offline   (LWT)
```
`<zone>` = nom du nœud Toshiba correspondant (`Shorai-31`, `Shorai-32`, …). Présence
« pièce » = **OU** des zones/régions détectées par le FP2.

## État

- ✅ **Cœur pur testé** (`fp2_bridge/core.py`) : config, topics/payloads, agrégation,
  anti‑rebattement. Tests : `python3 tests/test_core.py -v` (aucune dépendance).
- ⏳ **Couche HomeKit** (`fp2_bridge/hap.py`) : **scaffold à finaliser sur un FP2 réel**
  (les appels `aiohomekit` marqués `# VERIFY` dépendent de la version + de l'appareil).

## Installation (sur le Pi5)

```bash
cd bridge/aqara-fp2-mqtt
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt
cp config.example.toml config.toml   # puis éditer (device_id via `discover`)
```

## Mise en service (une fois un FP2 dispo)

```bash
# 1) Découvrir les accessoires HomeKit sur le LAN
python -m fp2_bridge --config config.toml discover
# 2) Appairer chaque FP2 (code d'appairage 8 chiffres au dos / dans l'app Aqara)
python -m fp2_bridge --config config.toml pair --device-id AA:BB:.. --code 12345678 --zone Shorai-31
# 3) Vérifier l'occupation (repérer les zones du FP2)
python -m fp2_bridge --config config.toml dump --zone Shorai-31
# 4) Lancer la boucle (présence → MQTT)
python -m fp2_bridge --config config.toml run
```

## Déploiement systemd

Unité fournie : `contrib/aqara-fp2-mqtt.service` (démarre après `mosquitto-broker`).
Config déployée en `/etc/daly-bms/fp2-bridge.toml`. Les **données d'appairage**
(`pairings/`) sont des **secrets** → jamais commitées (`.gitignore`).

## Sécurité (règle projet #12)

- `pairings/*.pairing.json` = clés HomeKit → **jamais** commitées.
- `config.toml` (éventuel mot de passe MQTT) → **jamais** commité (seul `config.example.toml` l'est).
