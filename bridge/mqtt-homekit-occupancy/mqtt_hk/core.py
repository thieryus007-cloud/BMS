"""Cœur **PUR** du pont MQTT → HomeKit (occupation) — aucune dépendance tierce.

Ré-expose la présence publiée par le pont FP2 comme **capteurs d'occupation HomeKit**,
pour retrouver les tuiles dans l'app Maison de l'iPhone (scénario C, cf.
`docs/toshiba-suzumi-rs-plan.md` §18.9).

Contrat MQTT consommé (messages *retained*, produits par `bridge/aqara-fp2-mqtt/`) :

    santuario/toshiba/presence/<zone>   →   {"present": true|false, "ts": <epoch>}

Ce module ne fait **ni HomeKit ni MQTT réseau** : uniquement config, routage
topic→accessoire, décodage du payload en valeur d'occupation HAP (0/1) et
anti‑rebattement. Il est importable et testable **sans** `pyhap`/`paho`.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass

#: Racine des topics de présence (locale, non bridgée — règle projet #11).
TOPIC_ROOT = "santuario/toshiba/presence"

#: Valeurs de la caractéristique HAP *OccupancyDetected*.
OCCUPANCY_DETECTED = 1
OCCUPANCY_NOT_DETECTED = 0

#: Format du code d'appairage HomeKit imposé par HAP : `XXX-XX-XXX`.
_PINCODE_RE = re.compile(r"^\d{3}-\d{2}-\d{3}$")


@dataclass(frozen=True)
class OccupancyUnit:
    """Une pièce ré‑exposée = un capteur d'occupation HomeKit."""

    zone: str  # segment de topic FP2 (ex. "Shorai-31")
    name: str  # nom affiché dans l'app Maison (défaut = zone)
    topic: str  # topic MQTT souscrit (défaut = santuario/toshiba/presence/<zone>)


@dataclass(frozen=True)
class BridgeConfig:
    mqtt_host: str = "127.0.0.1"
    mqtt_port: int = 1883
    mqtt_user: str | None = None
    mqtt_pass: str | None = None
    #: Nom du Bridge HomeKit tel qu'il apparaît à l'appairage.
    hap_bridge_name: str = "Santuario Presence"
    hap_port: int = 51826
    #: Code d'appairage HomeKit (à taper dans l'app Maison). Format `XXX-XX-XXX`.
    hap_pincode: str = "031-45-154"
    #: Dossier de l'état HAP (`accessory.state` = clé d'appairage → SECRET, jamais commité).
    hap_state_dir: str = "hap-state"
    units: tuple[OccupancyUnit, ...] = ()


def presence_topic(zone: str) -> str:
    """`santuario/toshiba/presence/<zone>`."""
    return f"{TOPIC_ROOT}/{zone}"


def occupancy_value(present: bool) -> int:
    """Booléen de présence → valeur HAP *OccupancyDetected* (1 présent / 0 libre)."""
    return OCCUPANCY_DETECTED if present else OCCUPANCY_NOT_DETECTED


def parse_presence_payload(raw) -> bool | None:
    """Décode un payload de présence en booléen, ou `None` si illisible/non pertinent.

    Accepte le contrat FP2 `{"present": true|false, ...}` (bytes **ou** str). Tout autre
    contenu (JSON sans clé `present`, message de disponibilité `online`/`offline`, JSON
    invalide, vide) → `None` : on **ignore** et on garde le dernier état connu.
    """
    if isinstance(raw, (bytes, bytearray)):
        try:
            raw = raw.decode("utf-8")
        except UnicodeDecodeError:
            return None
    if not isinstance(raw, str):
        return None
    raw = raw.strip()
    if not raw:
        return None
    try:
        obj = json.loads(raw)
    except (ValueError, TypeError):
        return None
    if not isinstance(obj, dict) or "present" not in obj:
        return None
    return bool(obj["present"])


def build_route(cfg: BridgeConfig) -> dict[str, str]:
    """Table `topic MQTT → nom d'accessoire` (pour dispatcher les messages reçus)."""
    return {u.topic: u.name for u in cfg.units}


class OccupancyTracker:
    """Anti‑rebattement : renvoie la valeur **seulement sur changement** (sinon `None`).

    HAP‑python conserve l'état de la caractéristique ; inutile de la ré‑écrire à chaque
    message *retained* identique. Le premier échantillon d'un accessoire est toujours
    émis (pose l'état initial).
    """

    def __init__(self) -> None:
        self._last: dict[str, bool] = {}

    def update(self, name: str, present: bool) -> bool | None:
        prev = self._last.get(name)
        if prev is None or prev != present:
            self._last[name] = present
            return present
        return None


def _valid_topic_segment(seg: str) -> bool:
    return bool(seg) and not any(c in seg for c in "/+# ")


def _valid_topic(topic: str) -> bool:
    return bool(topic) and not any(c in topic for c in "+# ")


def parse_config(data: dict) -> BridgeConfig:
    """Construit et **valide** une `BridgeConfig` depuis un dict (issu de `tomllib`)."""
    mqtt = data.get("mqtt", {}) or {}
    hap = data.get("hap", {}) or {}
    units_raw = data.get("units", []) or []

    units: list[OccupancyUnit] = []
    seen_names: set[str] = set()
    seen_topics: set[str] = set()
    for u in units_raw:
        zone = str(u.get("zone", "")).strip()
        if not _valid_topic_segment(zone):
            raise ValueError(f"zone invalide ou vide (segment de topic) : {zone!r}")
        name = str(u.get("name", "") or zone).strip()
        if not name:
            raise ValueError(f"unité {zone!r} : nom d'accessoire vide")
        topic = str(u.get("topic", "") or presence_topic(zone)).strip()
        if not _valid_topic(topic):
            raise ValueError(f"unité {zone!r} : topic invalide {topic!r}")
        # HomeKit exige des noms d'accessoire **uniques** ; les topics doivent l'être
        # aussi (sinon deux accessoires piloteraient depuis la même source).
        if name in seen_names:
            raise ValueError(f"nom d'accessoire dupliqué : {name!r}")
        if topic in seen_topics:
            raise ValueError(f"topic dupliqué : {topic!r}")
        seen_names.add(name)
        seen_topics.add(topic)
        units.append(OccupancyUnit(zone=zone, name=name, topic=topic))

    if not units:
        raise ValueError("aucune unité configurée (section [[units]])")

    pincode = str(hap.get("pincode", "031-45-154"))
    if not _PINCODE_RE.match(pincode):
        raise ValueError(f"hap.pincode doit être au format XXX-XX-XXX : {pincode!r}")

    port = int(hap.get("port", 51826))
    if not 1024 <= port <= 65535:
        raise ValueError(f"hap.port hors plage 1024–65535 : {port}")

    return BridgeConfig(
        mqtt_host=str(mqtt.get("host", "127.0.0.1")),
        mqtt_port=int(mqtt.get("port", 1883)),
        mqtt_user=mqtt.get("user"),
        mqtt_pass=mqtt.get("password"),
        hap_bridge_name=str(hap.get("bridge_name", "Santuario Presence")),
        hap_port=port,
        hap_pincode=pincode,
        hap_state_dir=str(hap.get("state_dir", "hap-state")),
        units=tuple(units),
    )
