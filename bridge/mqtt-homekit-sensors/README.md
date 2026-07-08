# mqtt-homekit-sensors — pont MQTT → HomeKit (capteurs multi-types)

Service Python (sur le Pi5) qui expose des topics MQTT arbitraires comme **capteurs
HomeKit** : **TemperatureSensor**, **LightSensor** (luminosité), **OccupancySensor**.

Deux usages :
1. **Tester la chaîne MQTT → HomeKit *maintenant*** avec les capteurs **déjà en place**
   (température, irradiance) — **sans** matériel Toshiba/FP2. HAP-python est un accessoire
   100 % logiciel : il suffit du Pi5 + un iPhone/iPad **sur le même WiFi** pour appairer.
2. **Généralise / supersède `mqtt-homekit-occupancy`** : l'occupation n'est qu'un type
   parmi d'autres. (Le pont Matter **E** supersède l'ensemble à terme — `docs/toshiba-bridges.md` §5.)

## État

- ✅ **Cœur pur testé** (`mqtt_hk_sensors/core.py`, **9 tests**, sans dépendance) :
  extraction de valeur (champ JSON **ou** nombre brut), conversion → caractéristique HomeKit
  (bornes respectées), config + validation.
- ✅ **Couche HAP vérifiée** contre pyhap réel (services/caractéristiques sondés ;
  **smoke-test** : build du Bridge + injection de valeurs simulées → les caractéristiques se
  mettent à jour). Reste le seul geste matériel : **appairer l'iPad/iPhone**.

Tester le cœur : `python3 bridge/mqtt-homekit-sensors/tests/test_core.py`

## Types supportés

| `type` | Service HomeKit | Caractéristique | Note |
|--------|-----------------|-----------------|------|
| `temperature` | TemperatureSensor | CurrentTemperature (°C) | `json_field` = ex. `"Temperature"` |
| `light` | LightSensor | CurrentAmbientLightLevel (**lux**) | 0 → `0.0001` (min HomeKit) ; l'irradiance W/m² est portée telle quelle |
| `occupancy` | OccupancySensor | OccupancyDetected (0/1) | `json_field="present"` (bool) accepté |

`json_field` absent → le payload entier est parsé comme **nombre** (ex. `santuario/irradiance/raw` = `"750"`).

## Installation & mise en service (Pi5)

```bash
cd bridge/mqtt-homekit-sensors
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt
cp config.example.toml config.toml     # déjà pointé sur température + irradiance
python -m mqtt_hk_sensors check-config --config config.toml   # dry-run (sans dépendance)
python -m mqtt_hk_sensors run          --config config.toml
# Puis app Maison (iPad/iPhone) : Ajouter un accessoire → « Plus d'options » →
# choisir le Bridge (hap.bridge_name) → saisir le PIN (hap.pincode).
```

> **Réseau** : l'iPad et le Pi5 doivent être sur le **même sous-réseau L2** (StarTh) pour la
> découverte mDNS/Bonjour `_hap._tcp`. Pas de hub requis pour l'**appairage local** (un
> HomePod/Apple TV n'est nécessaire que pour l'accès distant / les automatisations).

## Retours de mise en service (validé sur iPad, 2026-07-07)

- ✅ **Chaîne validée** : le capteur **température** apparaît et se met à jour dans l'app Maison
  → toute la plomberie MQTT → HAP → HomeKit fonctionne.
- ⚠️ **PIN** : dans l'app Maison, on entre les **8 chiffres sans tirets** (`03145155`). La config
  **doit** garder le format à tirets `XXX-XX-XXX` (imposé par HAP-python) ; l'app les ignore/ajoute.
- ⚠️ **Capteur de lumière (irradiance)** : **limitation de l'app Maison d'Apple**, PAS un bug du
  pont. Apple **n'affiche pas** les capteurs de lumière (lux) comme **tuile de pièce** (contrairement
  à la température) et ne les propose pas comme déclencheur d'automatisation. L'accessoire **est
  bien appairé et présent** — le voir : **barre d'état** en haut de l'app Maison, ou détail de
  l'accessoire, ou une **app tierce** (Eve — gratuite — l'affiche pleinement). Pour un vrai suivi
  d'irradiance, Grafana (série `irradiance_wm2`) reste la référence.

## Déploiement systemd

Unité : `contrib/mqtt-homekit-sensors.service` (après `mosquitto-broker`). Config déployée en
`/etc/daly-bms/mqtt-homekit-sensors.toml`.

## Sécurité (règle #12)

`hap-state/accessory.state` (clé d'appairage) + `config.toml` (PIN) → **jamais** commités.
Changer le PIN par défaut avant la mise en service.
