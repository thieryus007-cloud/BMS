"""Pont MQTT → HomeKit (occupation) — ré‑expose la présence FP2 dans l'app Maison.

`core` est **pur** (testable sans dépendance) ; `accessory`/`mqtt_in` sont la couche I/O
(pyhap / paho). Voir `docs/toshiba-suzumi-rs-plan.md` §18.9 (scénario C).
"""
