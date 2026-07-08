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

- ✅ **Cœur pur testé** (`mqtt_hk_sensors/core.py`, **14 tests**, sans dépendance) :
  extraction de valeur (champ JSON / nombre brut / texte ON-OFF), conversion → caractéristique
  HomeKit (bornes respectées), commande switch, config + validation, **topic partagé** (un
  message → plusieurs accessoires : température **et** humidité).
- ✅ **Couche HAP vérifiée** contre pyhap réel (services/caractéristiques sondés ; **smoke-test** :
  build du Bridge + injection de valeurs → caractéristiques mises à jour ; **switch : write
  HomeKit → commande MQTT publiée** + reflet de l'état physique). Reste le seul geste matériel :
  **appairer l'iPad/iPhone**.

Tester le cœur : `python3 bridge/mqtt-homekit-sensors/tests/test_core.py`

## Types supportés

| `type` | Service HomeKit | Caractéristique | Tuile Apple | Note |
|--------|-----------------|-----------------|:---:|------|
| `temperature` | TemperatureSensor | CurrentTemperature (°C) | ✅ | `json_field` = ex. `"Temperature"` |
| `humidity` | HumiditySensor | CurrentRelativeHumidity (%) | ✅ | `json_field` = ex. `"Humidity"` (borné 0–100) |
| `light` | LightSensor | CurrentAmbientLightLevel (lux) | ⚠️ | affiché comme « lumière » dans la pièce ; 0→`0.0001` ; pas de déclencheur d'automatisation natif |
| `occupancy` | OccupancySensor | OccupancyDetected (0/1) | ✅ | `json_field="present"` (bool) accepté |
| `switch` | Switch | On (bool) | ✅ **contrôlable** | état `stat/<id>/POWER` (`ON`/`OFF`) + **`command_topic`** `cmnd/<id>/POWER` |

`json_field` absent (capteurs) → le payload entier est parsé comme **nombre** (ex. `santuario/irradiance/raw` = `"750"`). Pour `switch`, l'état est le **texte** `ON`/`OFF`.

> **⚠️ `switch` = actionneur réel** (disjoncteur/prise Tongou). N'exposer **que** des switches
> **non** pilotés par energy-manager (ex. `tongou_3BC764` = chauffe-eau piloté par EM → **ne pas**
> exposer : conflit de commande). `tongou_3ACC34` (Switch5) n'est pas piloté par EM → OK.

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
- ✅ **Capteur de lumière (irradiance)** : **apparaît bien** comme une « lumière » dans la
  **pièce assignée** (validé — Séjour). Seule limite Apple : les capteurs lux ne sont pas
  utilisables comme **déclencheur d'automatisation** dans l'app **native** (une app tierce comme
  Eve le permet). Pour un vrai suivi d'irradiance, Grafana (`irradiance_wm2`) reste la référence.
- ✅ **Switch Tongou** : contrôlable depuis l'app Maison (allumer/couper) ; l'état physique
  remonte via `stat/<id>/POWER`. La **puissance W** mesurée par le Tongou ne s'affiche **pas**
  dans Apple Home (pas de type natif) → reste sur la page Tasmota / Grafana.

## Déploiement systemd (service permanent)

Pour que le pont tourne **en continu** (survit aux reboots) — unité fournie :
`contrib/mqtt-homekit-sensors.service` (config **locale** `config.toml`, après `mosquitto-broker`).

```bash
# venv + deps + config déjà faits (cf. « Installation »). Puis :
sudo cp contrib/mqtt-homekit-sensors.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now mqtt-homekit-sensors
systemctl status mqtt-homekit-sensors        # doit être "active (running)"
journalctl -u mqtt-homekit-sensors -f        # logs (dont "Enter this code..." au 1er appairage)
```

> L'appairage HomeKit **persiste** dans `hap-state/accessory.state` → après un redémarrage du
> service ou du Pi5, les accessoires **restent appairés** (pas de ré-appairage).

## Diagnostic « accessoire sans réponse » (No Response)

Deux causes possibles :

**1. Réattribution d'AID après modification d'un bridge déjà appairé.** Historiquement,
HAP-python assignait les *Accessory ID* dans l'**ordre de la config** ; insérer un capteur au
milieu décalait les AID des suivants → l'iPad déjà appairé gardait l'ancienne correspondance →
« sans réponse ». **Corrigé** : ce pont **épingle un AID stable dérivé du nom**
(`core.stable_aid`) → réordonner / insérer / retirer d'autres capteurs **ne décale plus rien**.
**Transition unique** : le passage aux AID épinglés change les AID actuels → il faut
**supprimer le pont dans l'app Maison puis le ré-ajouter UNE fois** ; ensuite les éditions de
config ne cassent plus l'appairage.

- **Correctif** : dans l'app Maison, **supprimer le pont « Santuario Sensors » puis le ré-ajouter**
  (ré‑appairage → l'iPad réapprend tous les AID). Alternative radicale : `rm -rf hap-state/` + relancer + ré‑appairer.
- **Bon réflexe** : garder les `name` **stables** dans `config.toml` (renommer dans l'app Maison,
  pas dans la config — changer un `name` change l'aid de CE capteur → à re-découvrir).

**2. Le capteur ne reçoit jamais de valeur** (champ réellement absent du payload) :

```bash
mosquitto_sub -t 'santuario/heat/1/venus' -v      # le champ attendu est-il présent ?
```

→ Si le champ est absent : retirer ce capteur, ou le pointer sur une source qui le publie.

> Dans tous les cas ce **n'est pas un bug du pont** : les accessoires dont l'AID est stable **et**
> qui reçoivent une valeur (température, switch…) fonctionnent.

## Sécurité (règle #12)

`hap-state/accessory.state` (clé d'appairage) + `config.toml` (PIN) → **jamais** commités.
Changer le PIN par défaut avant la mise en service.
