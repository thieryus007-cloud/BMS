"""CLI du pont MQTT → HomeKit (capteurs multi-types).

    python -m mqtt_hk_sensors check-config --config config.toml   # valider (sans dépendance)
    python -m mqtt_hk_sensors run          --config config.toml   # boucle principale (bloquant)

`run` : démarre le serveur HomeKit (Bridge + 1 accessoire/capteur), souscrit au broker,
pilote les caractéristiques depuis MQTT. Appairer ensuite le Bridge dans l'app **Maison**
avec le **PIN** de la config (`hap.pincode`).
"""

from __future__ import annotations

import argparse
import os
import signal
import sys
import tomllib

from . import core


def _load_config(path: str) -> core.BridgeConfig:
    with open(path, "rb") as fh:
        return core.parse_config(tomllib.load(fh))


def _cmd_check_config(args) -> int:
    cfg = _load_config(args.config)
    print(f"OK — {len(cfg.sensors)} capteur(s), broker {cfg.mqtt_host}:{cfg.mqtt_port}")
    print(f"HomeKit : bridge {cfg.hap_bridge_name!r}, port {cfg.hap_port}, PIN {cfg.hap_pincode}")
    for s in cfg.sensors:
        svc, char = core.service_and_char(s.kind)
        if s.kind == "switch":
            src = f"{s.topic} ↔ {s.command_topic}"
        elif s.json_field:
            src = f"{s.topic} [{s.json_field}]"
        else:
            src = f"{s.topic} [nombre brut]"
        print(f"  - {s.name!r}  ({svc}.{char})  ←  {src}")
    return 0


def _cmd_run(args) -> int:
    from pyhap.accessory_driver import AccessoryDriver

    from . import accessory, mqtt_in

    cfg = _load_config(args.config)
    os.makedirs(cfg.hap_state_dir, exist_ok=True)

    driver = AccessoryDriver(
        port=cfg.hap_port,
        persist_file=os.path.join(cfg.hap_state_dir, "accessory.state"),
        pincode=cfg.hap_pincode.encode(),
    )

    # Le subscriber est créé d'abord (pour fournir `publish` aux switches) ; son
    # callback route vers `accessories`, peuplé juste après par build_bridge.
    accessories: dict = {}

    def on_value(name: str, value: float) -> None:
        acc = accessories.get(name)
        if acc is not None:
            acc.set_from_value(value)

    sub = mqtt_in.MqttSubscriber(cfg, on_value)
    bridge, built = accessory.build_bridge(driver, cfg, sub.publish)
    accessories.update(built)
    driver.add_accessory(bridge)
    sub.connect()

    signal.signal(signal.SIGTERM, lambda *_: driver.stop())
    try:
        driver.start()
    finally:
        sub.close()
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="mqtt_hk_sensors", description="Pont MQTT → HomeKit (capteurs)")
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--config", default="config.toml", help="chemin du fichier de config")
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check-config", parents=[common], help="valider la config sans rien démarrer")
    sub.add_parser("run", parents=[common], help="boucle principale (bloquant)")

    args = p.parse_args(argv)
    try:
        if args.cmd == "check-config":
            return _cmd_check_config(args)
        return _cmd_run(args)
    except (ValueError, OSError, tomllib.TOMLDecodeError) as e:
        print(f"erreur de config ({args.config}) : {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
