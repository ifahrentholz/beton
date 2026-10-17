#!/usr/bin/env python3
"""QA-005 AC3: Ein Linux-Job der Escape-Matrix prüft die für sein Image erwartete Landlock-ABI.

Liest die Ausgabe von `beton sandbox probe --json` (SBX-010 AC1: `landlock_abi`, `kernel` je
Linux-Backend) und scheitert, wenn die ABI des Backends `landlock` nicht passt, statt die Fälle
der Suite zu überspringen. Mit `--unavailable` müssen die genannten Backends `available: false`
melden (Ubuntu 22.04, ABI 1: Landlock und bwrap sind dort nicht nutzbar, SBX-008).

  python3 scripts/check_landlock_abi.py probe.json --abi 2
  python3 scripts/check_landlock_abi.py probe.json --min-abi 4
  python3 scripts/check_landlock_abi.py probe.json --abi 1 --unavailable landlock,bwrap
"""
import argparse
import json
import sys


def check(probe, abi=None, min_abi=None, unavailable=()):
    """Liste der Abweichungen (leer = passt)."""
    providers = {p.get("id"): p for p in probe.get("providers", [])}
    landlock = providers.get("landlock")
    if landlock is None:
        return ["Probe ohne Backend `landlock`"]
    errors = []
    found = landlock.get("landlock_abi")
    kernel = landlock.get("kernel") or "?"
    if abi is not None and found != abi:
        errors.append(f"Landlock-ABI {found} statt {abi} (Kernel {kernel})")
    if min_abi is not None and (not isinstance(found, int) or found < min_abi):
        errors.append(f"Landlock-ABI {found} statt mindestens {min_abi} (Kernel {kernel})")
    for name in unavailable:
        p = providers.get(name)
        if p is None:
            errors.append(f"Probe ohne Backend `{name}`")
        elif p.get("available") is not False:
            errors.append(f"`{name}` meldet sich als verfügbar, erwartet: nicht verfügbar")
        elif not p.get("unavailable_reason"):
            errors.append(f"`{name}` nicht verfügbar, aber ohne Grund")
    if not unavailable and landlock.get("available") is not True:
        errors.append(f"`landlock` nicht verfügbar: {landlock.get('unavailable_reason')}")
    return errors


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("probe")
    ap.add_argument("--abi", type=int)
    ap.add_argument("--min-abi", type=int)
    ap.add_argument("--unavailable", default="")
    args = ap.parse_args(argv)
    if args.abi is None and args.min_abi is None:
        ap.error("--abi oder --min-abi angeben")
    try:
        with open(args.probe, encoding="utf-8") as f:
            probe = json.load(f)
    except (OSError, ValueError) as e:
        print(f"::error title=Landlock-ABI::{args.probe}: {e}")
        return 1
    unavailable = [n for n in args.unavailable.split(",") if n]
    errors = check(probe, args.abi, args.min_abi, unavailable)
    landlock = next((p for p in probe.get("providers", []) if p.get("id") == "landlock"), {})
    print(f"Kernel {landlock.get('kernel')}, Landlock-ABI {landlock.get('landlock_abi')}")
    for e in errors:
        print(f"::error title=Landlock-ABI (QA-005 AC3)::{e}")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
