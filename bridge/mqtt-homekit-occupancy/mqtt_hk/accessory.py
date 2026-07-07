"""Serveur d'accessoires **HomeKit** (HAP‑python / `pyhap`) — couche I/O.

⚠️ **Scaffold à finaliser sur l'appareil** (appairage réel à l'app Maison). La logique
(un Bridge portant un capteur d'occupation par unité, valeur pilotée par MQTT) est en
place, mais les **appels exacts de `pyhap` varient selon la version** installée. Les
points à confirmer sont marqués `# VERIFY`.

Sens du flux : ici on est un **accessoire** que l'iPhone (contrôleur) appaire — c'est
**l'inverse** de l'appairage du FP2 (lu par `aiohomekit`). Les deux relations HAP sont
indépendantes → aucun conflit avec l'appairage mono‑contrôleur du FP2 (cf. §18.9).

Aucune de ces fonctions n'est testée hors matériel ; le cœur pur (`core.py`, testé)
reste la référence pour le décodage et le routage.
"""

from __future__ import annotations

from pyhap.accessory import Accessory, Bridge  # type: ignore  # VERIFY
from pyhap.const import CATEGORY_SENSOR  # type: ignore  # VERIFY

from . import core


class OccupancyAccessory(Accessory):
    """Un capteur d'occupation HomeKit dont l'état suit les messages MQTT."""

    category = CATEGORY_SENSOR  # VERIFY

    def __init__(self, driver, display_name: str) -> None:
        super().__init__(driver, display_name)
        serv = self.add_preload_service("OccupancySensor")  # VERIFY: nom du service
        self._char = serv.configure_char(  # VERIFY: nom de la caractéristique
            "OccupancyDetected", value=core.OCCUPANCY_NOT_DETECTED
        )

    def set_present(self, present: bool) -> None:
        """Met à jour *OccupancyDetected* (notifie l'app Maison)."""
        self._char.set_value(core.occupancy_value(present))  # VERIFY


def build_bridge(driver, cfg: core.BridgeConfig):
    """Construit le Bridge HomeKit + un `OccupancyAccessory` par unité.

    Retourne `(bridge, {nom: accessoire})` pour que la couche MQTT pousse l'état.
    """
    bridge = Bridge(driver, cfg.hap_bridge_name)  # VERIFY
    accessories: dict[str, OccupancyAccessory] = {}
    for unit in cfg.units:
        acc = OccupancyAccessory(driver, unit.name)
        bridge.add_accessory(acc)  # VERIFY
        accessories[unit.name] = acc
    return bridge, accessories
