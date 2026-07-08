"""Serveur d'accessoires **HomeKit** (HAP-python / `pyhap`) — capteurs multi-types.

Un accessoire par capteur configuré : *TemperatureSensor* / *LightSensor* /
*OccupancySensor*. Les valeurs sont pilotées par les messages MQTT (`mqtt_in`).

Le nœud s'appaire à l'app **Maison** de l'iPhone/iPad avec son **propre PIN** (accessoire
100 % logiciel — aucun matériel requis hormis le Pi5 et l'appareil Apple sur le même LAN).
Services/caractéristiques vérifiés contre pyhap (`Loader`) → API correcte.
"""

from __future__ import annotations

from pyhap.accessory import Accessory, Bridge
from pyhap.const import CATEGORY_SENSOR

from . import core


class SensorAccessory(Accessory):
    """Un capteur HomeKit dont la caractéristique suit les messages MQTT."""

    category = CATEGORY_SENSOR

    def __init__(self, driver, spec: core.SensorSpec) -> None:
        super().__init__(driver, spec.name)
        service_name, char_name = core.service_and_char(spec.kind)
        serv = self.add_preload_service(service_name)
        # Valeur initiale neutre et valide (respecte les bornes HomeKit).
        initial = core.to_char_value(spec.kind, 0.0)
        self._char = serv.configure_char(char_name, value=initial)
        self._kind = spec.kind

    def set_from_value(self, value: float) -> None:
        """Pousse une valeur numérique brute (convertie/bornée pour la caractéristique)."""
        self._char.set_value(core.to_char_value(self._kind, value))


def build_bridge(driver, cfg: core.BridgeConfig):
    """Construit le Bridge HomeKit + un `SensorAccessory` par capteur configuré.

    Retourne `(bridge, {nom: accessoire})` pour que la couche MQTT pousse les valeurs.
    """
    bridge = Bridge(driver, cfg.hap_bridge_name)
    accessories: dict[str, SensorAccessory] = {}
    for spec in cfg.sensors:
        acc = SensorAccessory(driver, spec)
        bridge.add_accessory(acc)
        accessories[spec.name] = acc
    return bridge, accessories
