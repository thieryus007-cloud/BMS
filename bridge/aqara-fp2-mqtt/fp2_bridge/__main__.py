"""CLI du pont Aqara FP2 → MQTT.

    python -m fp2_bridge discover                       # lister les accessoires HomeKit
    python -m fp2_bridge pair --device-id ID --code 12345678 --zone Shorai-31
    python -m fp2_bridge dump  --zone Shorai-31         # lister les caractéristiques
    python -m fp2_bridge run   --config config.toml     # boucle principale

Le sous-commande `run` : charge la config, se connecte au broker, et pour chaque FP2
suit l'occupation → publie la présence *retained* sur `santuario/toshiba/presence/<zone>`.
"""

from __future__ import annotations

import argparse
import asyncio
import sys
import tomllib

from . import core


def _load_config(path: str) -> core.BridgeConfig:
    with open(path, "rb") as fh:
        return core.parse_config(tomllib.load(fh))


async def _cmd_discover(_args) -> int:
    from . import hap

    for d in await hap.discover():
        flag = "appairé" if d.get("paired") else "libre"
        print(f"{d['device_id']}  {d.get('name', '')}  [{flag}]")
    return 0


async def _cmd_pair(args) -> int:
    from . import hap

    cfg = _load_config(args.config)
    await hap.pair(args.device_id, args.code, args.zone, cfg.pairings_dir)
    print(f"Appairé : {args.zone} → {cfg.pairings_dir}/{args.zone}.pairing.json")
    return 0


async def _cmd_dump(args) -> int:
    from . import hap

    cfg = _load_config(args.config)
    unit = next((u for u in cfg.units if u.zone == args.zone), None)
    if unit is None:
        print(f"zone inconnue : {args.zone}", file=sys.stderr)
        return 2

    async def show(zone: str, present: bool) -> None:
        print(f"{zone}: présence = {present}")

    # `watch` imprime l'état initial via le callback, puis boucle : on coupe vite.
    task = asyncio.create_task(hap.watch(unit, cfg.pairings_dir, show))
    await asyncio.sleep(3)
    task.cancel()
    return 0


async def _cmd_run(args) -> int:
    from . import hap, mqtt_out

    cfg = _load_config(args.config)
    pub = mqtt_out.MqttPublisher(cfg)
    pub.connect()
    tracker = core.PresenceTracker(heartbeat_secs=cfg.heartbeat_secs)
    loop = asyncio.get_running_loop()

    async def on_presence(zone: str, present: bool) -> None:
        decision = tracker.update(zone, present, loop.time())
        if decision is not None:
            pub.publish_presence(zone, decision)

    try:
        await asyncio.gather(
            *(hap.watch(u, cfg.pairings_dir, on_presence) for u in cfg.units)
        )
    finally:
        pub.close()
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="fp2_bridge", description="Pont Aqara FP2 → MQTT")
    p.add_argument("--config", default="config.toml", help="chemin du fichier de config")
    sub = p.add_subparsers(dest="cmd", required=True)

    sub.add_parser("discover", help="lister les accessoires HomeKit du LAN")

    sp = sub.add_parser("pair", help="appairer un FP2")
    sp.add_argument("--device-id", required=True)
    sp.add_argument("--code", required=True, help="code d'appairage 8 chiffres")
    sp.add_argument("--zone", required=True, help="ex. Shorai-31")

    sd = sub.add_parser("dump", help="lister l'occupation d'un FP2 appairé")
    sd.add_argument("--zone", required=True)

    sub.add_parser("run", help="boucle principale (présence → MQTT)")

    args = p.parse_args(argv)
    handler = {
        "discover": _cmd_discover,
        "pair": _cmd_pair,
        "dump": _cmd_dump,
        "run": _cmd_run,
    }[args.cmd]
    return asyncio.run(handler(args))


if __name__ == "__main__":
    raise SystemExit(main())
