"""Publication MQTT (paho-mqtt) — mince couche I/O au-dessus de `core`.

Publie la présence *retained* sur `santuario/toshiba/presence/<zone>` et une
disponibilité LWT `santuario/toshiba/presence/bridge/availability`.
"""

from __future__ import annotations

import paho.mqtt.client as mqtt

from . import core

AVAILABILITY_TOPIC = f"{core.TOPIC_ROOT}/bridge/availability"


def _make_client(client_id: str) -> mqtt.Client:
    """Compatible paho-mqtt 1.x **et** 2.x."""
    try:  # paho-mqtt >= 2.0
        return mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, client_id=client_id)
    except (AttributeError, TypeError):  # paho-mqtt < 2.0
        return mqtt.Client(client_id=client_id)


class MqttPublisher:
    def __init__(self, cfg: core.BridgeConfig, client_id: str = "aqara-fp2-bridge") -> None:
        self._cfg = cfg
        self._client = _make_client(client_id)
        if cfg.mqtt_user:
            self._client.username_pw_set(cfg.mqtt_user, cfg.mqtt_pass or "")
        # LWT : signale l'arrêt du pont si la connexion tombe.
        self._client.will_set(AVAILABILITY_TOPIC, "offline", qos=1, retain=True)

    def connect(self) -> None:
        self._client.connect(self._cfg.mqtt_host, self._cfg.mqtt_port, keepalive=30)
        self._client.loop_start()
        self._client.publish(AVAILABILITY_TOPIC, "online", qos=1, retain=True)

    def publish_presence(self, zone: str, present: bool, now: float | None = None) -> None:
        self._client.publish(
            core.presence_topic(zone),
            core.presence_payload(present, now),
            qos=1,
            retain=True,
        )

    def close(self) -> None:
        try:
            self._client.publish(AVAILABILITY_TOPIC, "offline", qos=1, retain=True)
            self._client.loop_stop()
            self._client.disconnect()
        except Exception:  # noqa: BLE001 — best-effort à l'arrêt
            pass
