"""Serveur d'accessoires **HomeKit** (HAP-python / `pyhap`) — capteurs + switch.

Un accessoire par entrée : *TemperatureSensor* / *HumiditySensor* / *LightSensor* /
*OccupancySensor* (lecture seule), ou *Switch* (**contrôlable** : l'app Maison bascule
l'état → publication `cmnd/<id>/POWER`, et l'état réel `stat/<id>/POWER` remonte la valeur).

Le nœud s'appaire à l'app **Maison** avec son **propre PIN** (accessoire logiciel — Pi5 +
appareil Apple sur le même LAN). Services/caractéristiques vérifiés contre pyhap (`Loader`).
"""

from __future__ import annotations

from typing import Callable

from pyhap.accessory import Accessory, Bridge
from pyhap.const import CATEGORY_SENSOR, CATEGORY_SWITCH

from . import core

#: Signature du publieur de commande injecté (topic, payload) → publie en MQTT.
PublishCommand = Callable[[str, str], None]


class SensorAccessory(Accessory):
    """Capteur en lecture seule dont la caractéristique suit les messages MQTT."""

    category = CATEGORY_SENSOR

    def __init__(self, driver, spec: core.SensorSpec) -> None:
        super().__init__(driver, spec.name)
        service_name, char_name = core.service_and_char(spec.kind)
        serv = self.add_preload_service(service_name)
        initial = core.to_char_value(spec.kind, 0.0)
        self._char = serv.configure_char(char_name, value=initial)
        self._kind = spec.kind

    def set_from_value(self, value: float) -> None:
        self._char.set_value(core.to_char_value(self._kind, value))


class SwitchAccessory(Accessory):
    """Switch **contrôlable** : HomeKit → commande MQTT ; état MQTT → HomeKit."""

    category = CATEGORY_SWITCH

    def __init__(self, driver, spec: core.SensorSpec, publish: PublishCommand) -> None:
        super().__init__(driver, spec.name)
        serv = self.add_preload_service("Switch")
        # `setter_callback` = appelé UNIQUEMENT sur écriture HomeKit (pas par set_value)
        # → pas de boucle quand on reflète l'état physique via `set_from_value`.
        self._char = serv.configure_char("On", value=False, setter_callback=self._on_write)
        self._spec = spec
        self._publish = publish

    def _on_write(self, value: bool) -> None:
        if self._spec.command_topic:
            self._publish(self._spec.command_topic, core.switch_command_payload(bool(value)))

    def set_from_value(self, value: float) -> None:
        self._char.set_value(value >= 0.5)


def build_bridge(driver, cfg: core.BridgeConfig, publish: PublishCommand):
    """Construit le Bridge HomeKit + un accessoire par entrée.

    `publish(topic, payload)` est utilisé par les accessoires **contrôlables** (switch).
    Retourne `(bridge, {nom: accessoire})`.
    """
    bridge = Bridge(driver, cfg.hap_bridge_name)
    accessories: dict[str, Accessory] = {}
    for spec in cfg.sensors:
        if spec.kind in core.CONTROLLABLE_KINDS:
            acc: Accessory = SwitchAccessory(driver, spec, publish)
        else:
            acc = SensorAccessory(driver, spec)
        bridge.add_accessory(acc)
        accessories[spec.name] = acc
    return bridge, accessories
