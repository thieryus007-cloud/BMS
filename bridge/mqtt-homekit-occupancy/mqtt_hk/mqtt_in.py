"""Souscription MQTT (paho‑mqtt) — mince couche I/O au‑dessus de `core`.

Souscrit aux topics de présence configurés et appelle `on_presence(name, present)` pour
chaque message décodable. Publie aussi une disponibilité LWT du **pont HomeKit** (distincte
de celle du pont FP2). Compatible paho‑mqtt 1.x **et** 2.x.
"""

from __future__ import annotations

from typing import Callable

import paho.mqtt.client as mqtt

from . import core

#: Disponibilité de CE pont (≠ `.../bridge/availability` publié par le pont FP2).
AVAILABILITY_TOPIC = f"{core.TOPIC_ROOT}/homekit-bridge/availability"


def _make_client(client_id: str) -> mqtt.Client:
    """Compatible paho‑mqtt >= 2.0 **et** < 2.0."""
    try:  # paho-mqtt >= 2.0
        return mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, client_id=client_id)
    except (AttributeError, TypeError):  # paho-mqtt < 2.0
        return mqtt.Client(client_id=client_id)


class MqttSubscriber:
    def __init__(
        self,
        cfg: core.BridgeConfig,
        on_presence: Callable[[str, bool], None],
        client_id: str = "mqtt-homekit-occupancy",
    ) -> None:
        self._cfg = cfg
        self._on_presence = on_presence
        self._route = core.build_route(cfg)
        self._client = _make_client(client_id)
        if cfg.mqtt_user:
            self._client.username_pw_set(cfg.mqtt_user, cfg.mqtt_pass or "")
        self._client.will_set(AVAILABILITY_TOPIC, "offline", qos=1, retain=True)
        self._client.on_connect = self._on_connect
        self._client.on_message = self._on_message

    def _on_connect(self, client, userdata, flags, reason_code, properties=None) -> None:
        # (Ré)abonnement à chaque (re)connexion → survit à une coupure broker.
        for topic in self._route:
            client.subscribe(topic, qos=1)
        client.publish(AVAILABILITY_TOPIC, "online", qos=1, retain=True)

    def _on_message(self, client, userdata, msg) -> None:
        name = self._route.get(msg.topic)
        if name is None:
            return
        present = core.parse_presence_payload(msg.payload)
        if present is None:
            return
        self._on_presence(name, present)

    def connect(self) -> None:
        self._client.connect(self._cfg.mqtt_host, self._cfg.mqtt_port, keepalive=30)
        self._client.loop_start()

    def close(self) -> None:
        try:
            # Poser explicitement "offline" (un disconnect() propre ne déclenche PAS le LWT).
            info = self._client.publish(AVAILABILITY_TOPIC, "offline", qos=1, retain=True)
            try:
                info.wait_for_publish(timeout=2.0)
            except Exception:  # noqa: BLE001 — publication best-effort
                pass
            self._client.loop_stop()
            self._client.disconnect()
        except Exception:  # noqa: BLE001 — best-effort à l'arrêt
            pass
