"""Tests du cœur pur (aucune dépendance tierce). Lancer :

    python3 -m unittest discover -s bridge/aqara-fp2-mqtt -p 'test_*.py'
"""

import json
import os
import sys
import unittest

# Rendre le paquet importable quel que soit le CWD.
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from fp2_bridge import core  # noqa: E402


class TestTopicsPayloads(unittest.TestCase):
    def test_topic(self):
        self.assertEqual(
            core.presence_topic("Shorai-31"),
            "santuario/toshiba/presence/Shorai-31",
        )

    def test_payload(self):
        js = core.presence_payload(True, now=1720000000)
        self.assertEqual(json.loads(js), {"present": True, "ts": 1720000000})
        # ts entier même si now est un float.
        self.assertEqual(json.loads(core.presence_payload(False, now=1.9))["ts"], 1)
        self.assertIs(json.loads(core.presence_payload(False, now=0))["present"], False)


class TestAggregate(unittest.TestCase):
    def test_or_logic(self):
        self.assertFalse(core.aggregate_presence([]))
        self.assertFalse(core.aggregate_presence([False, False]))
        self.assertTrue(core.aggregate_presence([False, True, False]))
        self.assertTrue(core.aggregate_presence([1]))  # truthy


class TestTracker(unittest.TestCase):
    def test_change_and_heartbeat(self):
        t = core.PresenceTracker(heartbeat_secs=60)
        # 1er échantillon → publie.
        self.assertEqual(t.update("z", False, now=0.0), False)
        # Même valeur avant heartbeat → rien.
        self.assertIsNone(t.update("z", False, now=10.0))
        # Changement → publie.
        self.assertEqual(t.update("z", True, now=11.0), True)
        # Même valeur, mais heartbeat écoulé → republie.
        self.assertEqual(t.update("z", True, now=11.0 + 60), True)

    def test_zones_are_independent(self):
        t = core.PresenceTracker(heartbeat_secs=60)
        self.assertEqual(t.update("a", True, 0.0), True)
        self.assertEqual(t.update("b", False, 0.0), False)
        self.assertIsNone(t.update("a", True, 1.0))


class TestParseConfig(unittest.TestCase):
    def _valid(self):
        return {
            "mqtt": {"host": "192.168.1.141", "port": 1883},
            "units": [
                {"zone": "Shorai-31", "device_id": "AA:BB:CC:DD:EE:01"},
                {"zone": "Shorai-32", "device_id": "AA:BB:CC:DD:EE:02"},
            ],
        }

    def test_valid(self):
        cfg = core.parse_config(self._valid())
        self.assertEqual(cfg.mqtt_host, "192.168.1.141")
        self.assertEqual(len(cfg.units), 2)
        self.assertEqual(cfg.units[0].zone, "Shorai-31")

    def test_rejects_bad(self):
        d = self._valid()
        d["units"][0]["zone"] = ""  # vide
        self.assertRaises(ValueError, core.parse_config, d)

        d = self._valid()
        d["units"][0]["zone"] = "a/b"  # caractère de topic interdit
        self.assertRaises(ValueError, core.parse_config, d)

        d = self._valid()
        d["units"][1]["zone"] = "Shorai-31"  # doublon
        self.assertRaises(ValueError, core.parse_config, d)

        d = self._valid()
        d["units"][0].pop("device_id")  # device_id manquant
        self.assertRaises(ValueError, core.parse_config, d)

        self.assertRaises(ValueError, core.parse_config, {"units": []})  # aucune unité


if __name__ == "__main__":
    unittest.main()
