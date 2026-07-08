"""Souscription MQTT (paho-mqtt) — mince couche I/O au-dessus de `core`.

Souscrit les topics des capteurs configurés et met à jour la caractéristique HomeKit
correspondante. Compatible paho-mqtt 1.x **et** 2.x.
"""

from __future__ import annotations

from typing import Callable

import paho.mqtt.client as mqtt

from . import core

AVAILABILITY_TOPIC = "santuario/homekit-sensors/availability"


def _make_client(client_id: str) -> mqtt.Client:
    try:  # paho-mqtt >= 2.0
        return mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, client_id=client_id)
    except (AttributeError, TypeError):  # paho-mqtt < 2.0
        return mqtt.Client(client_id=client_id)


class MqttSubscriber:
    def __init__(
        self,
        cfg: core.BridgeConfig,
        on_value: Callable[[str, float], None],
        client_id: str = "mqtt-homekit-sensors",
    ) -> None:
        self._cfg = cfg
        self._on_value = on_value
        self._route = core.build_route(cfg)
        self._client = _make_client(client_id)
        if cfg.mqtt_user:
            self._client.username_pw_set(cfg.mqtt_user, cfg.mqtt_pass or "")
        self._client.will_set(AVAILABILITY_TOPIC, "offline", qos=1, retain=True)
        self._client.on_connect = self._on_connect
        self._client.on_message = self._on_message

    def _on_connect(self, client, userdata, flags, reason_code, properties=None) -> None:
        for topic in self._route:
            client.subscribe(topic, qos=0)
        client.publish(AVAILABILITY_TOPIC, "online", qos=1, retain=True)

    def _on_message(self, client, userdata, msg) -> None:
        spec = self._route.get(msg.topic)
        if spec is None:
            return
        value = core.parse_value(spec, msg.payload)
        if value is None:
            return
        self._on_value(spec.name, value)

    def connect(self) -> None:
        self._client.connect(self._cfg.mqtt_host, self._cfg.mqtt_port, keepalive=30)
        self._client.loop_start()

    def close(self) -> None:
        try:
            info = self._client.publish(AVAILABILITY_TOPIC, "offline", qos=1, retain=True)
            try:
                info.wait_for_publish(timeout=2.0)
            except Exception:  # noqa: BLE001
                pass
            self._client.loop_stop()
            self._client.disconnect()
        except Exception:  # noqa: BLE001
            pass
