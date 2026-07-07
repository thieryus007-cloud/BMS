"""CLI du pont MQTT → HomeKit (occupation).

    python -m mqtt_hk check-config --config config.toml   # valider la config (dry-run, sans dépendance)
    python -m mqtt_hk run          --config config.toml   # boucle principale (bloquant)

`run` : charge la config, démarre le serveur HomeKit (Bridge + 1 capteur d'occupation par
unité), souscrit au broker, puis pilote *OccupancyDetected* depuis la présence MQTT.
Appairer ensuite le Bridge dans l'app **Maison** de l'iPhone avec le **PIN** de la config
(`hap.pincode`). Voir `docs/toshiba-suzumi-rs-plan.md` §18.9.
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
    print(f"OK — {len(cfg.units)} accessoire(s), broker {cfg.mqtt_host}:{cfg.mqtt_port}")
    print(
        f"HomeKit : bridge {cfg.hap_bridge_name!r}, port {cfg.hap_port}, "
        f"PIN {cfg.hap_pincode}, état {cfg.hap_state_dir}/"
    )
    for u in cfg.units:
        print(f"  - {u.name!r}  ←  {u.topic}")
    return 0


def _cmd_run(args) -> int:
    from pyhap.accessory_driver import AccessoryDriver  # type: ignore  # VERIFY

    from . import accessory, mqtt_in

    cfg = _load_config(args.config)
    os.makedirs(cfg.hap_state_dir, exist_ok=True)

    driver = AccessoryDriver(  # VERIFY: signature (port / persist_file / pincode bytes)
        port=cfg.hap_port,
        persist_file=os.path.join(cfg.hap_state_dir, "accessory.state"),
        pincode=cfg.hap_pincode.encode(),
    )
    bridge, accessories = accessory.build_bridge(driver, cfg)
    driver.add_accessory(bridge)  # VERIFY

    tracker = core.OccupancyTracker()

    def on_presence(name: str, present: bool) -> None:
        decided = tracker.update(name, present)
        if decided is None:
            return
        acc = accessories.get(name)
        if acc is not None:
            acc.set_present(decided)

    sub = mqtt_in.MqttSubscriber(cfg, on_presence)
    sub.connect()

    # Arrêt propre : SIGTERM (systemd) → stop du driver HAP, puis fermeture MQTT.
    signal.signal(signal.SIGTERM, lambda *_: driver.stop())  # VERIFY
    try:
        driver.start()  # bloquant jusqu'à stop() (VERIFY)
    finally:
        sub.close()
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="mqtt_hk", description="Pont MQTT → HomeKit (occupation)")
    # `--config` accepté par chaque sous-commande (donc après elle) via un parent commun.
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
