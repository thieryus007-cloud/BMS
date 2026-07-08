"""Pont MQTT → HomeKit (capteurs multi-types : température, luminosité, occupation).

`core` est **pur** (testable sans dépendance) ; `accessory`/`mqtt_in` sont la couche I/O
(pyhap / paho). Sert de test de la chaîne MQTT→HomeKit et de successeur de
`mqtt-homekit-occupancy`. Voir `docs/toshiba-bridges.md`.
"""
