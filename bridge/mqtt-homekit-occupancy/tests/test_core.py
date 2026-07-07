"""Tests du cœur pur (aucune dépendance tierce). Lancer :

    python3 -m unittest discover -s bridge/mqtt-homekit-occupancy -p 'test_*.py'
"""

import os
import sys
import unittest

# Rendre le paquet importable quel que soit le CWD.
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from mqtt_hk import core  # noqa: E402


class TestTopicAndValue(unittest.TestCase):
    def test_topic(self):
        self.assertEqual(
            core.presence_topic("Shorai-31"),
            "santuario/toshiba/presence/Shorai-31",
        )

    def test_occupancy_value(self):
        self.assertEqual(core.occupancy_value(True), core.OCCUPANCY_DETECTED)
        self.assertEqual(core.occupancy_value(False), core.OCCUPANCY_NOT_DETECTED)
        self.assertEqual((core.OCCUPANCY_DETECTED, core.OCCUPANCY_NOT_DETECTED), (1, 0))


class TestParsePayload(unittest.TestCase):
    def test_valid(self):
        self.assertIs(core.parse_presence_payload('{"present": true, "ts": 1}'), True)
        self.assertIs(core.parse_presence_payload('{"present": false}'), False)
        # bytes (paho remet des bytes).
        self.assertIs(core.parse_presence_payload(b'{"present": true}'), True)

    def test_ignored(self):
        for raw in (
            b"online",  # message de disponibilité
            "offline",
            "",  # vide
            "   ",
            b"\xff\xfe",  # non‑UTF‑8
            "{ pas du json",  # JSON invalide
            '{"ts": 123}',  # clé `present` absente
            "true",  # JSON valide mais pas un objet
            "42",
            None,
        ):
            self.assertIsNone(core.parse_presence_payload(raw), repr(raw))


class TestTracker(unittest.TestCase):
    def test_change_only(self):
        t = core.OccupancyTracker()
        # 1er échantillon → pose l'état initial.
        self.assertIs(t.update("Shorai-31", False), False)
        # Même valeur → rien.
        self.assertIsNone(t.update("Shorai-31", False))
        # Changement → émet.
        self.assertIs(t.update("Shorai-31", True), True)
        self.assertIsNone(t.update("Shorai-31", True))

    def test_names_independent(self):
        t = core.OccupancyTracker()
        self.assertIs(t.update("a", True), True)
        self.assertIs(t.update("b", False), False)
        self.assertIsNone(t.update("a", True))


class TestParseConfig(unittest.TestCase):
    def _valid(self):
        return {
            "mqtt": {"host": "192.168.1.141", "port": 1883},
            "hap": {"bridge_name": "Santuario Presence", "pincode": "031-45-154"},
            "units": [
                {"zone": "Shorai-31", "name": "Clim Chambre"},
                {"zone": "Shorai-32"},  # name par défaut = zone
            ],
        }

    def test_valid_and_defaults(self):
        cfg = core.parse_config(self._valid())
        self.assertEqual(cfg.mqtt_host, "192.168.1.141")
        self.assertEqual(len(cfg.units), 2)
        # name explicite conservé, topic dérivé de la zone.
        self.assertEqual(cfg.units[0].name, "Clim Chambre")
        self.assertEqual(cfg.units[0].topic, "santuario/toshiba/presence/Shorai-31")
        # name par défaut = zone.
        self.assertEqual(cfg.units[1].name, "Shorai-32")
        # routage topic → nom.
        route = core.build_route(cfg)
        self.assertEqual(route["santuario/toshiba/presence/Shorai-32"], "Shorai-32")

    def test_topic_override(self):
        d = self._valid()
        d["units"][0]["topic"] = "santuario/toshiba/presence/salon"
        cfg = core.parse_config(d)
        self.assertEqual(cfg.units[0].topic, "santuario/toshiba/presence/salon")

    def test_rejects_bad_units(self):
        d = self._valid()
        d["units"][0]["zone"] = ""  # vide
        self.assertRaises(ValueError, core.parse_config, d)

        d = self._valid()
        d["units"][0]["zone"] = "a/b"  # caractère de topic interdit
        self.assertRaises(ValueError, core.parse_config, d)

        d = self._valid()
        d["units"][1]["name"] = "Clim Chambre"  # nom d'accessoire dupliqué
        self.assertRaises(ValueError, core.parse_config, d)

        d = self._valid()
        d["units"][1]["topic"] = "santuario/toshiba/presence/Shorai-31"  # topic dupliqué
        self.assertRaises(ValueError, core.parse_config, d)

        self.assertRaises(ValueError, core.parse_config, {"units": []})  # aucune unité

    def test_rejects_bad_hap(self):
        d = self._valid()
        d["hap"]["pincode"] = "12345678"  # mauvais format (pas XXX-XX-XXX)
        self.assertRaises(ValueError, core.parse_config, d)

        d = self._valid()
        d["hap"]["port"] = 80  # hors plage
        self.assertRaises(ValueError, core.parse_config, d)


if __name__ == "__main__":
    unittest.main()
