"""Cœur **PUR** du pont MQTT → HomeKit (capteurs multi-types) — aucune dépendance tierce.

Généralise `mqtt-homekit-occupancy` (occupation seule) en un pont **multi-types** :
expose des topics MQTT arbitraires comme capteurs HomeKit **TemperatureSensor**,
**LightSensor** (luminosité) ou **OccupancySensor**. Sert à la fois de test de la chaîne
MQTT→HomeKit (avec les capteurs déjà en place : température, irradiance) et, à terme, de
successeur de `mqtt-homekit-occupancy` (l'occupation n'est qu'un type parmi d'autres).

Ce module ne fait **ni HomeKit ni MQTT réseau** : uniquement config, extraction de valeur
depuis le payload, et conversion → valeur de caractéristique HomeKit. Testable **sans**
`pyhap`/`paho`.
"""

from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass

#: Types supportés → (service HAP, caractéristique HAP). `switch` est **contrôlable**
#: (lecture d'état + commande) ; les autres sont des **capteurs** (lecture seule).
SENSOR_KINDS: dict[str, tuple[str, str]] = {
    "temperature": ("TemperatureSensor", "CurrentTemperature"),
    "humidity": ("HumiditySensor", "CurrentRelativeHumidity"),
    "light": ("LightSensor", "CurrentAmbientLightLevel"),
    "occupancy": ("OccupancySensor", "OccupancyDetected"),
    "switch": ("Switch", "On"),
}

#: Types **contrôlables** (nécessitent un `command_topic`).
CONTROLLABLE_KINDS = frozenset({"switch"})

#: Bornes HomeKit (extraites des définitions pyhap) à respecter côté valeurs.
TEMP_MIN, TEMP_MAX = -273.1, 1000.0
HUM_MIN, HUM_MAX = 0.0, 100.0
LUX_MIN = 0.0001  # CurrentAmbientLightLevel : minValue strictement > 0 → clamp du 0.

#: Format du code d'appairage HomeKit : `XXX-XX-XXX`.
_PINCODE_RE = re.compile(r"^\d{3}-\d{2}-\d{3}$")


@dataclass(frozen=True)
class SensorSpec:
    """Un capteur exposé : un accessoire HomeKit alimenté par un topic MQTT."""

    name: str  # nom affiché dans l'app Maison (unique)
    kind: str  # clé de SENSOR_KINDS
    topic: str  # topic MQTT d'ÉTAT souscrit
    #: Champ JSON à extraire (ex. "Temperature"). `None` → payload = **nombre brut**
    #: (ex. `santuario/irradiance/raw` = `"750"`). Ignoré pour `switch` (état = texte ON/OFF).
    json_field: str | None = None
    #: Topic de **commande** (types contrôlables). Ex. `cmnd/tongou_3ACC34/POWER`.
    command_topic: str | None = None


@dataclass(frozen=True)
class BridgeConfig:
    mqtt_host: str = "127.0.0.1"
    mqtt_port: int = 1883
    mqtt_user: str | None = None
    mqtt_pass: str | None = None
    hap_bridge_name: str = "Santuario Sensors"
    hap_port: int = 51827
    hap_pincode: str = "031-45-155"
    hap_state_dir: str = "hap-state"
    sensors: tuple[SensorSpec, ...] = ()


def service_and_char(kind: str) -> tuple[str, str]:
    """(service HAP, caractéristique HAP) pour un type de capteur."""
    return SENSOR_KINDS[kind]


def parse_value(spec: SensorSpec, raw) -> float | None:
    """Extrait la valeur numérique du payload selon `spec.json_field`.

    - `json_field` défini → `json.loads(payload)[json_field]` (nombre **ou** booléen).
    - sinon → le payload entier est parsé comme un **nombre** (ex. `"750"`).
    Retourne `None` si illisible (message ignoré, dernière valeur conservée).
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
    # `switch` : état soit texte brut "ON"/"OFF" (Tasmota `stat/<id>/POWER`), soit champ
    # d'un JSON si `json_field` est défini (ex. `tele/<id>/STATE` `{"POWER":"ON"}`, ou
    # Zigbee2MQTT `{"state":"ON"}`). Accepte ON/1/true et OFF/0/false.
    if spec.kind == "switch":
        extracted = raw
        if spec.json_field is not None:
            try:
                obj = json.loads(raw)
            except (ValueError, TypeError):
                return None
            if not isinstance(obj, dict) or spec.json_field not in obj:
                return None
            extracted = obj[spec.json_field]
        s = str(extracted).strip().upper()
        if s in ("ON", "1", "TRUE"):
            return 1.0
        if s in ("OFF", "0", "FALSE"):
            return 0.0
        return None
    try:
        if spec.json_field is not None:
            obj = json.loads(raw)
            if not isinstance(obj, dict) or spec.json_field not in obj:
                return None
            return float(obj[spec.json_field])  # bool True/False → 1.0/0.0
        return float(raw)
    except (ValueError, TypeError):
        return None


def to_char_value(kind: str, value: float) -> float | int | bool:
    """Valeur numérique → valeur de caractéristique HomeKit (bornée/typée)."""
    if kind == "temperature":
        return float(min(max(value, TEMP_MIN), TEMP_MAX))
    if kind == "humidity":
        return float(min(max(value, HUM_MIN), HUM_MAX))
    if kind == "light":
        # HomeKit lux : minValue = 0.0001 → 0 W/m² (nuit) devient 0.0001 pour rester valide.
        return float(max(value, LUX_MIN))
    if kind == "occupancy":
        return 1 if value >= 0.5 else 0
    if kind == "switch":
        return value >= 0.5  # caractéristique "On" (bool)
    raise ValueError(f"type de capteur inconnu : {kind!r}")


def switch_command_payload(on: bool) -> str:
    """Valeur HomeKit d'un switch → commande Tasmota (`cmnd/<id>/POWER`)."""
    return "ON" if on else "OFF"


def _valid_topic(topic: str) -> bool:
    return bool(topic) and not any(c in topic for c in "+# ")


def parse_config(data: dict) -> BridgeConfig:
    """Construit et **valide** une `BridgeConfig` depuis un dict (issu de `tomllib`)."""
    mqtt = data.get("mqtt", {}) or {}
    hap = data.get("hap", {}) or {}
    sensors_raw = data.get("sensors", []) or []

    sensors: list[SensorSpec] = []
    seen_names: set[str] = set()
    for s in sensors_raw:
        name = str(s.get("name", "")).strip()
        kind = str(s.get("kind", s.get("type", ""))).strip()
        topic = str(s.get("topic", "")).strip()
        field = s.get("json_field")
        field = str(field).strip() if field is not None else None
        cmd = s.get("command_topic")
        cmd = str(cmd).strip() if cmd is not None else None
        if not name:
            raise ValueError("capteur sans `name`")
        if kind not in SENSOR_KINDS:
            raise ValueError(f"capteur {name!r} : type inconnu {kind!r} (attendu {sorted(SENSOR_KINDS)})")
        if not _valid_topic(topic):
            raise ValueError(f"capteur {name!r} : topic d'état invalide {topic!r}")
        if kind in CONTROLLABLE_KINDS:
            if not cmd or not _valid_topic(cmd):
                raise ValueError(f"capteur {name!r} ({kind}) : `command_topic` requis et valide")
        elif cmd is not None:
            raise ValueError(f"capteur {name!r} ({kind}) : `command_topic` interdit (non contrôlable)")
        if name in seen_names:
            raise ValueError(f"nom d'accessoire dupliqué : {name!r}")
        seen_names.add(name)
        sensors.append(
            SensorSpec(name=name, kind=kind, topic=topic, json_field=field or None, command_topic=cmd or None)
        )

    if not sensors:
        raise ValueError("aucun capteur configuré (section [[sensors]])")

    pincode = str(hap.get("pincode", "031-45-155"))
    if not _PINCODE_RE.match(pincode):
        raise ValueError(f"hap.pincode doit être au format XXX-XX-XXX : {pincode!r}")
    port = int(hap.get("port", 51827))
    if not 1024 <= port <= 65535:
        raise ValueError(f"hap.port hors plage 1024–65535 : {port}")

    return BridgeConfig(
        mqtt_host=str(mqtt.get("host", "127.0.0.1")),
        mqtt_port=int(mqtt.get("port", 1883)),
        mqtt_user=mqtt.get("user"),
        mqtt_pass=mqtt.get("password"),
        hap_bridge_name=str(hap.get("bridge_name", "Santuario Sensors")),
        hap_port=port,
        hap_pincode=pincode,
        hap_state_dir=str(hap.get("state_dir", "hap-state")),
        sensors=tuple(sensors),
    )


def stable_aid(name: str) -> int:
    """AID HomeKit **stable** dérivé du nom (indépendant de l'ordre de la config).

    Avec des AID épinglés par nom, ajouter/retirer/**réordonner** d'autres accessoires ne
    décale plus les AID existants → plus de « sans réponse » après édition d'un bridge déjà
    appairé (seul renommer un accessoire change SON aid). `1` est réservé au Bridge lui-même.
    """
    digest = hashlib.sha256(name.encode("utf-8")).digest()
    return 2 + (int.from_bytes(digest[:6], "big") % (2**31 - 3))


def build_route(cfg: BridgeConfig) -> dict[str, list[SensorSpec]]:
    """Table `topic MQTT → [SensorSpec]` : un même topic peut alimenter **plusieurs**
    accessoires (ex. `santuario/heat/1/venus` → température **et** humidité)."""
    route: dict[str, list[SensorSpec]] = {}
    for s in cfg.sensors:
        route.setdefault(s.topic, []).append(s)
    return route
