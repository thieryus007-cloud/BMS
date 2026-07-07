# mqtt-homekit-occupancy — pont MQTT → HomeKit (occupation)

Petit service Python (à héberger **sur le Pi5**, aucun matériel supplémentaire) qui relit la
**présence** publiée par le pont FP2 et la **ré-expose comme capteurs d'occupation HomeKit**,
pour retrouver les **tuiles dans l'app Maison** de l'iPhone.

> Contexte & décisions : `docs/toshiba-suzumi-rs-plan.md` **§18.9 (scénario C)**.
> C'est le rôle **légitime** d'un serveur d'accessoires (MQTT → Apple Home) — à ne pas
> confondre avec « lire le FP2 » (ça, c'est le pont `aqara-fp2-mqtt` via `aiohomekit`).

## Sens du flux (important)

```
FP2 ──aiohomekit──►  santuario/toshiba/presence/<zone>  ──► energy-manager (contrôle clim)
 (aqara-fp2-mqtt)              (MQTT retained)            └─► CE pont ──► app Maison (iPhone)
```

- Le **FP2** est un *accessoire* lu par `aiohomekit` (appairage mono-contrôleur — cf. §18.9).
- **CE pont** est lui-même un *accessoire* que **l'iPhone appaire** (avec son **propre** PIN).
  Les deux relations HomeKit sont **indépendantes** → aucun conflit d'appairage.
- L'occupation exposée est « reconstruite » **par pièce** (OU des zones, déjà agrégé par le
  pont FP2), pas la UI multi-zones native du FP2.

## État

- ✅ **Cœur pur testé** (`mqtt_hk/core.py`) : config + validation, routage topic→accessoire,
  décodage du payload de présence en valeur HAP (0/1), anti-rebattement.
  Tests : `python3 -m unittest discover -s bridge/mqtt-homekit-occupancy -p 'test_*.py'`
  (aucune dépendance).
- ⏳ **Couche HomeKit** (`mqtt_hk/accessory.py`) + **souscripteur** (`mqtt_hk/mqtt_in.py`) :
  scaffold à **finaliser/valider sur l'appareil** (appels `pyhap` marqués `# VERIFY`,
  appairage réel à l'app Maison).

## Installation (sur le Pi5)

```bash
cd bridge/mqtt-homekit-occupancy
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt
cp config.example.toml config.toml   # puis éditer (noms, PIN)
python -m mqtt_hk check-config --config config.toml   # dry-run (valide sans dépendance)
```

## Mise en service

```bash
python -m mqtt_hk run --config config.toml
# Puis dans l'app Maison de l'iPhone : Ajouter un accessoire → « Plus d'options » →
# choisir le Bridge (hap.bridge_name) → saisir le PIN (hap.pincode).
```

## Déploiement systemd

Unité fournie : `contrib/mqtt-homekit-occupancy.service` (démarre après `mosquitto-broker`
et `aqara-fp2-mqtt`). Config déployée en `/etc/daly-bms/mqtt-homekit-occupancy.toml`.

## Sécurité (règle projet #12)

- `hap-state/accessory.state` = clé d'appairage HomeKit → **jamais** commité (`.gitignore`).
- `config.toml` (PIN, éventuel mot de passe MQTT) → **jamais** commité (seul
  `config.example.toml` l'est). **Changer le PIN par défaut** avant la mise en service.
