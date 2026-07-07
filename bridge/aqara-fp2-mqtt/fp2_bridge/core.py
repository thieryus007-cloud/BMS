"""Cœur **PUR** du pont Aqara FP2 → MQTT — aucune dépendance tierce, testable.

Contrat MQTT (cf. `docs/toshiba-suzumi-rs-plan.md` §18) — messages *retained* :

    santuario/toshiba/presence/<zone>   →   {"present": true|false, "ts": <epoch>}

Ce module ne fait **ni HomeKit ni MQTT réseau** : uniquement config, agrégation de
présence (zones du FP2 → présence « pièce »), construction des payloads et
anti‑rebattement. Il est importable et testable **sans** `aiohomekit`/`paho`.
"""

from __future__ import annotations

import json
import time
from dataclasses import dataclass

#: Racine des topics de présence (locale, non bridgée — règle projet #11).
TOPIC_ROOT = "santuario/toshiba/presence"


@dataclass(frozen=True)
class UnitConfig:
    """Un FP2 = une pièce = une zone Shorai."""

    zone: str  # ex. "Shorai-31" (= segment de topic + nom du nœud Toshiba)
    device_id: str  # identifiant HomeKit de l'accessoire (ex. "AA:BB:CC:DD:EE:FF")


@dataclass(frozen=True)
class BridgeConfig:
    mqtt_host: str = "127.0.0.1"
    mqtt_port: int = 1883
    mqtt_user: str | None = None
    mqtt_pass: str | None = None
    #: Dossier des données d'appairage HomeKit (SECRET — jamais commité).
    pairings_dir: str = "pairings"
    #: Republication périodique même sans changement (rafraîchit le retained).
    heartbeat_secs: int = 60
    units: tuple[UnitConfig, ...] = ()


def presence_topic(zone: str) -> str:
    """`santuario/toshiba/presence/<zone>`."""
    return f"{TOPIC_ROOT}/{zone}"


def presence_payload(present: bool, now: float | None = None) -> str:
    """JSON compact `{"present": bool, "ts": epoch}`."""
    ts = int(now if now is not None else time.time())
    return json.dumps({"present": bool(present), "ts": ts}, separators=(",", ":"))


def aggregate_presence(region_states) -> bool:
    """Présence « pièce » = **OU** logique sur les régions occupées du FP2.

    Le FP2 expose plusieurs capteurs d'occupation (une par zone/région) ; la pièce
    est considérée occupée dès qu'**au moins une** région l'est.
    """
    return any(bool(x) for x in region_states)


class PresenceTracker:
    """Anti‑rebattement : ne publie que sur **changement**, plus un **heartbeat**.

    `update(zone, present, now)` renvoie la valeur à publier (bool) si l'état a
    changé **ou** si le heartbeat est écoulé, sinon `None`.
    """

    def __init__(self, heartbeat_secs: int = 60) -> None:
        self.heartbeat_secs = heartbeat_secs
        self._last: dict[str, tuple[bool, float]] = {}

    def update(self, zone: str, present: bool, now: float) -> bool | None:
        prev = self._last.get(zone)
        changed = prev is None or prev[0] != present
        stale = prev is not None and (now - prev[1]) >= self.heartbeat_secs
        if changed or stale:
            self._last[zone] = (present, now)
            return present
        return None


def _valid_zone(zone: str) -> bool:
    return bool(zone) and not any(c in zone for c in "/+# ")


def parse_config(data: dict) -> BridgeConfig:
    """Construit et **valide** une `BridgeConfig` depuis un dict (issu de `tomllib`)."""
    mqtt = data.get("mqtt", {}) or {}
    units_raw = data.get("units", []) or []

    units: list[UnitConfig] = []
    seen: set[str] = set()
    for u in units_raw:
        zone = str(u.get("zone", "")).strip()
        dev = str(u.get("device_id", "")).strip()
        if not _valid_zone(zone):
            raise ValueError(f"zone invalide ou vide (segment de topic) : {zone!r}")
        if not dev:
            raise ValueError(f"unité {zone!r} sans `device_id` HomeKit")
        if zone in seen:
            raise ValueError(f"zone dupliquée : {zone!r}")
        seen.add(zone)
        units.append(UnitConfig(zone=zone, device_id=dev))

    if not units:
        raise ValueError("aucune unité configurée (section [[units]])")

    # `heartbeat_secs <= 0` rendrait la condition `stale` toujours vraie dans
    # PresenceTracker → publications MQTT en boucle continue (surcharge CPU).
    heartbeat = int(data.get("heartbeat_secs", 60))
    if heartbeat <= 0:
        raise ValueError("heartbeat_secs doit être strictement supérieur à 0")

    return BridgeConfig(
        mqtt_host=str(mqtt.get("host", "127.0.0.1")),
        mqtt_port=int(mqtt.get("port", 1883)),
        mqtt_user=mqtt.get("user"),
        mqtt_pass=mqtt.get("password"),
        pairings_dir=str(data.get("pairings_dir", "pairings")),
        heartbeat_secs=heartbeat,
        units=tuple(units),
    )
