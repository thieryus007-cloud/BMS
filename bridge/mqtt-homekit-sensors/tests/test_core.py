"""Tests du cœur pur (aucune dépendance tierce). Lancer :

    python3 bridge/mqtt-homekit-sensors/tests/test_core.py
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from mqtt_hk_sensors import core  # noqa: E402


class TestParseValue(unittest.TestCase):
    def temp(self):
        return core.SensorSpec("T", "temperature", "santuario/heat/1/venus", "Temperature")

    def light(self):
        return core.SensorSpec("L", "light", "santuario/irradiance/raw", None)

    def occ(self):
        return core.SensorSpec("O", "occupancy", "p/x", "present")

    def test_json_field(self):
        v = core.parse_value(self.temp(), b'{"Temperature":15.3,"Humidity":65}')
        self.assertAlmostEqual(v, 15.3)

    def test_raw_number(self):
        self.assertEqual(core.parse_value(self.light(), b"750"), 750.0)
        self.assertEqual(core.parse_value(self.light(), "0"), 0.0)

    def test_bool_field_for_occupancy(self):
        self.assertEqual(core.parse_value(self.occ(), b'{"present":true}'), 1.0)
        self.assertEqual(core.parse_value(self.occ(), b'{"present":false}'), 0.0)

    def test_ignored(self):
        for raw in (b"", "  ", b"\xff", "pas nombre", b'{"Humidity":65}', None):
            self.assertIsNone(core.parse_value(self.temp(), raw), repr(raw))


class TestSwitch(unittest.TestCase):
    def spec(self):
        return core.SensorSpec(
            "Sw", "switch", "stat/tongou_3ACC34/POWER", command_topic="cmnd/tongou_3ACC34/POWER"
        )

    def test_state_text_parsing(self):
        s = self.spec()
        self.assertEqual(core.parse_value(s, b"ON"), 1.0)
        self.assertEqual(core.parse_value(s, b"OFF"), 0.0)
        self.assertEqual(core.parse_value(s, "on"), 1.0)  # casse insensible
        self.assertIsNone(core.parse_value(s, b"blink"))

    def test_state_json_parsing(self):
        # État de switch niché dans un JSON (Tasmota tele/STATE, Zigbee2MQTT…).
        s = core.SensorSpec(
            "Sw", "switch", "tele/tongou_3ACC34/STATE",
            json_field="POWER", command_topic="cmnd/tongou_3ACC34/POWER",
        )
        self.assertEqual(core.parse_value(s, b'{"POWER":"ON"}'), 1.0)
        self.assertEqual(core.parse_value(s, b'{"POWER":false}'), 0.0)
        self.assertIsNone(core.parse_value(s, b'{"POWER":"UNKNOWN"}'))
        self.assertIsNone(core.parse_value(s, b'{"other":"ON"}'))  # champ absent
        self.assertIsNone(core.parse_value(s, b"pas du json"))

    def test_to_char_bool_and_command(self):
        self.assertIs(core.to_char_value("switch", 1.0), True)
        self.assertIs(core.to_char_value("switch", 0.0), False)
        self.assertEqual(core.switch_command_payload(True), "ON")
        self.assertEqual(core.switch_command_payload(False), "OFF")


class TestToCharValue(unittest.TestCase):
    def test_temperature_clamped(self):
        self.assertEqual(core.to_char_value("temperature", 15.3), 15.3)
        self.assertEqual(core.to_char_value("temperature", -999), core.TEMP_MIN)

    def test_humidity_clamped(self):
        self.assertEqual(core.to_char_value("humidity", 65.0), 65.0)
        self.assertEqual(core.to_char_value("humidity", 150.0), core.HUM_MAX)

    def test_light_zero_becomes_min_lux(self):
        # 0 W/m² (nuit) → lux minimal valide (HomeKit refuse 0).
        self.assertEqual(core.to_char_value("light", 0.0), core.LUX_MIN)
        self.assertEqual(core.to_char_value("light", 750.0), 750.0)

    def test_occupancy_is_binary(self):
        self.assertEqual(core.to_char_value("occupancy", 1.0), 1)
        self.assertEqual(core.to_char_value("occupancy", 0.0), 0)


class TestConfig(unittest.TestCase):
    def _valid(self):
        return {
            "mqtt": {"host": "127.0.0.1"},
            "hap": {"pincode": "031-45-155"},
            "sensors": [
                {"name": "Temp ext", "type": "temperature", "topic": "santuario/heat/1/venus", "json_field": "Temperature"},
                {"name": "Irradiance", "type": "light", "topic": "santuario/irradiance/raw"},
            ],
        }

    def test_valid(self):
        cfg = core.parse_config(self._valid())
        self.assertEqual(len(cfg.sensors), 2)
        self.assertEqual(cfg.sensors[0].json_field, "Temperature")
        self.assertIsNone(cfg.sensors[1].json_field)
        route = core.build_route(cfg)
        self.assertEqual(route["santuario/irradiance/raw"][0].kind, "light")

    def test_shared_topic_feeds_multiple(self):
        # Un même topic (heat/1/venus) → température + humidité.
        d = self._valid()
        d["sensors"].append({"name": "Hum", "type": "humidity", "topic": "santuario/heat/1/venus", "json_field": "Humidity"})
        cfg = core.parse_config(d)
        route = core.build_route(cfg)
        kinds = sorted(s.kind for s in route["santuario/heat/1/venus"])
        self.assertEqual(kinds, ["humidity", "temperature"])

    def test_switch_needs_command_topic(self):
        base = self._valid()
        # Switch valide.
        base["sensors"].append({
            "name": "Prise séjour", "type": "switch",
            "topic": "stat/tongou_3ACC34/POWER", "command_topic": "cmnd/tongou_3ACC34/POWER",
        })
        cfg = core.parse_config(base)
        sw = cfg.sensors[-1]
        self.assertEqual(sw.kind, "switch")
        self.assertEqual(sw.command_topic, "cmnd/tongou_3ACC34/POWER")

        # Switch sans command_topic → rejeté.
        bad = self._valid()
        bad["sensors"].append({"name": "P", "type": "switch", "topic": "stat/x/POWER"})
        self.assertRaises(ValueError, core.parse_config, bad)

        # command_topic sur un capteur non contrôlable → rejeté.
        bad = self._valid()
        bad["sensors"][0]["command_topic"] = "cmnd/x/POWER"
        self.assertRaises(ValueError, core.parse_config, bad)

    def test_rejects_bad(self):
        d = self._valid()
        d["sensors"][0]["type"] = "pressure"  # type inconnu
        self.assertRaises(ValueError, core.parse_config, d)

        d = self._valid()
        d["sensors"][1]["name"] = "Temp ext"  # nom dupliqué
        self.assertRaises(ValueError, core.parse_config, d)

        d = self._valid()
        d["hap"]["pincode"] = "12345678"  # mauvais format
        self.assertRaises(ValueError, core.parse_config, d)

        self.assertRaises(ValueError, core.parse_config, {"sensors": []})  # aucun capteur


if __name__ == "__main__":
    unittest.main()
