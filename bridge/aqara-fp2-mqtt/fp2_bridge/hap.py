"""Contrôleur **HomeKit local** (`aiohomekit`) pour les FP2 — couche I/O.

⚠️ **Scaffold à finaliser sur un FP2 réel.** La logique de flux (découverte →
appairage → abonnement occupation → agrégation → callback) est en place, mais les
**appels exacts d'`aiohomekit` varient selon la version** installée. Les points à
confirmer sur l'appareil sont marqués `# VERIFY`. Utiliser `python -m fp2_bridge
discover` puis `... pair` pour caler les identifiants, et `... dump` pour lister les
caractéristiques d'occupation (une par zone du FP2).

Aucune de ces fonctions n'est testée hors matériel ; le cœur pur (`core.py`, testé)
reste la référence pour le contrat MQTT.
"""

from __future__ import annotations

import json
import os
from typing import Awaitable, Callable

from aiohomekit import Controller  # type: ignore
from aiohomekit.model.characteristics import CharacteristicsTypes  # type: ignore

from . import core

#: Caractéristique HAP standard d'un capteur d'occupation (0 = libre, 1 = présence).
OCCUPANCY = CharacteristicsTypes.OCCUPANCY_DETECTED


def _pairing_path(pairings_dir: str, zone: str) -> str:
    return os.path.join(pairings_dir, f"{zone}.pairing.json")


async def discover(timeout: int = 10) -> list[dict]:
    """Liste les accessoires HomeKit visibles sur le LAN (mDNS)."""
    found: list[dict] = []
    async with Controller() as controller:
        async for disc in controller.async_discover(timeout):  # VERIFY: signature
            info = disc.description
            found.append(
                {
                    "device_id": info.id,  # VERIFY: champ id (device_id HAP)
                    "name": getattr(info, "name", ""),
                    "paired": not getattr(info, "status_flags", 1) & 0x01,
                }
            )
    return found


async def pair(device_id: str, setup_code: str, zone: str, pairings_dir: str) -> None:
    """Appaire un FP2 (code d'appairage 8 chiffres) et **persiste** la clé.

    ⚠️ La donnée d'appairage contient des **secrets** → dossier `pairings/` **jamais
    commité** (cf. `.gitignore`).
    """
    os.makedirs(pairings_dir, exist_ok=True)
    async with Controller() as controller:
        disc = await controller.async_find(device_id)  # VERIFY
        finish = await disc.async_start_pairing(zone)  # VERIFY
        pairing = await finish(setup_code)  # VERIFY
        data = pairing.pairing_data  # VERIFY: dict sérialisable
    with open(_pairing_path(pairings_dir, zone), "w", encoding="utf-8") as fh:
        json.dump(data, fh)


async def watch(
    unit: core.UnitConfig,
    pairings_dir: str,
    on_room_presence: Callable[[str, bool], Awaitable[None]],
) -> None:
    """Se connecte au FP2 de `unit`, suit l'occupation de toutes ses zones et
    appelle `on_room_presence(zone, present)` (présence pièce = OU des régions).
    """
    path = _pairing_path(pairings_dir, unit.zone)
    with open(path, encoding="utf-8") as fh:
        pairing_data = json.load(fh)

    async with Controller() as controller:
        controller.load_pairing(unit.zone, pairing_data)  # VERIFY
        pairing = controller.pairings[unit.zone]

        accessories = await pairing.list_accessories_and_characteristics()  # VERIFY
        # Repérer toutes les caractéristiques d'occupation (une par zone du FP2).
        occ: list[tuple[int, int]] = []  # (aid, iid)
        for acc in accessories:
            aid = acc["aid"]
            for svc in acc.get("services", []):
                for ch in svc.get("characteristics", []):
                    if ch.get("type") == OCCUPANCY:
                        occ.append((aid, ch["iid"]))
        if not occ:
            raise RuntimeError(f"{unit.zone}: aucune caractéristique d'occupation trouvée")

        region_state: dict[tuple[int, int], bool] = {c: False for c in occ}

        async def emit() -> None:
            await on_room_presence(unit.zone, core.aggregate_presence(region_state.values()))

        # État initial.
        current = await pairing.get_characteristics(occ)  # VERIFY
        for (aid, iid), v in current.items():
            region_state[(aid, iid)] = bool(v.get("value"))
        await emit()

        # Abonnement aux changements.
        def _on_event(data):  # VERIFY: forme de l'event
            for (aid, iid), v in data.items():
                region_state[(aid, iid)] = bool(v.get("value"))

        pairing.dispatcher_connect(_on_event)  # VERIFY
        await pairing.subscribe(occ)  # VERIFY

        # La connexion est maintenue par le contexte ; les events déclenchent emit().
        # (Boucle de maintien + ré-émission périodique gérées par l'appelant.)
        import asyncio

        while True:
            await asyncio.sleep(1)
            await emit()
