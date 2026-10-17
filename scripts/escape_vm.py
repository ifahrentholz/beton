#!/usr/bin/env python3
"""Escape-Suite in VMs mit echten Kerneln (SBX-011 Details, QA-005 AC3, #301).

Container teilen den Kernel des Runners; Debian 12 (Kernel 6.1, Landlock-ABI 2), Ubuntu 22.04
(5.15, ABI 1) und Fedora (ABI ≥ 6) laufen deshalb als VMs mit KVM auf dem Linux-Runner:

1. `prepare`: Cloud-Image per Prüfsumme laden und einmal mit Netz provisionieren (Pakete,
   Benutzer `runner` mit derselben UID wie auf dem Runner, ein systemd-Dienst, der beim nächsten
   Start das Datenlaufwerk ausführt). Ergebnis im Verzeichnis `--dir` (Actions-Cache).
2. `run`: Datenlaufwerk (ext4, `mkfs.ext4 -d`) mit Quellbaum, nextest-Archiv der Escape-Suite,
   Binaries und Gast-Skript bauen, die VM **ohne Netzwerkkarte** starten, Ergebnisse (Probe,
   `beton sandbox test`, JUnit-Bericht, Logs) zurückholen (Laufwerk per `sudo mount -o ro`).
3. `check`: erwartete Landlock-ABI des Images (`scripts/check_landlock_abi.py`) und Ergebnisse
   prüfen. Eine Abweichung lässt den Job scheitern, statt Fälle zu überspringen.

Der Quellbaum liegt in der VM unter demselben absoluten Pfad wie auf dem Runner, weil die Tests
Pfade wie `CARGO_BIN_EXE_*` und `CARGO_MANIFEST_DIR` beim Kompilieren festhalten.

  python3 scripts/escape_vm.py prepare debian-12 --dir ~/escape-vm/debian-12
  python3 scripts/escape_vm.py run debian-12 --dir … --archive escape.tar.zst --nextest … --out …
  python3 scripts/escape_vm.py check debian-12 --out …
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, ".."))

# Images per Prüfsumme festgelegt. `abi`: erwartete Landlock-ABI (genau) bzw. `min_abi`;
# `suite`: ob die Escape-Suite läuft (Ubuntu 22.04: nur die Prüfung auf `unavailable`).
IMAGES = {
    "debian-12": {
        "url": "https://cloud.debian.org/images/cloud/bookworm/20261006-2623/debian-12-genericcloud-amd64-20261006-2623.qcow2",
        "sha512": "a09170d17e13af43da61774666f520e3bbec0f8bd96b4db37c15f3426fb327ccad950b80dbe78c09aa5ca90ce5015e99334b0ba5188fd4018736bc3fb5f0882c",
        "abi": 2,
        "backends": "landlock,bwrap",
        "suite": True,
    },
    "ubuntu-22.04": {
        "url": "https://cloud-images.ubuntu.com/releases/22.04/release-20261004/ubuntu-22.04-server-cloudimg-amd64.img",
        "sha256": "012d81fade7e8ff4428fc35dfeee711d66d1f47d5ea9c5c8ced96a85b959835c",
        "abi": 1,
        "unavailable": "landlock,bwrap",
        "suite": False,
    },
    "fedora-44": {
        "url": "https://download.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images/Fedora-Cloud-Base-Generic-44-1.7.x86_64.qcow2",
        "sha256": "28680fe5b371a5a82ebf43a31926e086a168e59949d03969c5093e7071f90b7f",
        "min_abi": 6,
        "backends": "landlock,bwrap",
        "suite": True,
    },
}

PROVISIONED = "BETON-PROVISIONED"


def log(msg):
    print(msg, flush=True)


def sh(*cmd, **kw):
    log("+ " + " ".join(str(c) for c in cmd))
    return subprocess.run([str(c) for c in cmd], check=True, **kw)


def digest(path, algo):
    h = hashlib.new(algo)
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def fetch(image, dest):
    """Lädt das Image und prüft die Prüfsumme; eine falsche Datei wird gelöscht."""
    spec = IMAGES[image]
    algo = "sha512" if "sha512" in spec else "sha256"
    if not os.path.exists(dest):
        log(f"Lade {spec['url']}")
        tmp = dest + ".part"
        with urllib.request.urlopen(spec["url"], timeout=120) as r, open(tmp, "wb") as f:
            shutil.copyfileobj(r, f, 1 << 20)
        os.replace(tmp, dest)
    got = digest(dest, algo)
    if got != spec[algo]:
        os.remove(dest)
        raise SystemExit(f"Prüfsumme von {image} passt nicht: {got}")
    log(f"{image}: {algo} geprüft")


def user_data(uid):
    """cloud-init für den Provisionierungs-Start (mit Netz)."""
    runner = "\n".join(
        [
            "#!/bin/sh",
            "# Führt das Datenlaufwerk der CI aus (scripts/escape_vm.py) und schaltet ab.",
            "[ -e /etc/beton-escape-ready ] || exit 0",
            "for i in $(seq 1 30); do [ -e /dev/disk/by-label/BETONDATA ] && break; sleep 1; done",
            "if [ -e /dev/disk/by-label/BETONDATA ]; then",
            "  mkdir -p /mnt/data",
            "  mount /dev/disk/by-label/BETONDATA /mnt/data && sh /mnt/data/run.sh",
            "  sync",
            "fi",
            "systemctl poweroff",
            "",
        ]
    )
    unit = "\n".join(
        [
            "[Unit]",
            "Description=beton Escape-Suite (CI)",
            "After=local-fs.target",
            "",
            "[Service]",
            "Type=oneshot",
            "ExecStart=/usr/local/sbin/beton-escape",
            "StandardOutput=journal+console",
            "StandardError=journal+console",
            "",
            "[Install]",
            "WantedBy=multi-user.target",
            "",
        ]
    )
    config = {
        "package_update": True,
        "packages": ["git", "bubblewrap", "curl"],
        "users": [
            "default",
            {"name": "runner", "uid": uid, "shell": "/bin/bash", "lock_passwd": True},
        ],
        # Keine automatischen Updates: Der Kernel des Images bleibt der geprüfte.
        "bootcmd": [
            ["sh", "-c", "systemctl mask --now apt-daily.timer apt-daily-upgrade.timer unattended-upgrades.service || true"],
        ],
        "write_files": [
            {"path": "/usr/local/sbin/beton-escape", "permissions": "0755", "content": runner},
            {"path": "/etc/systemd/system/beton-escape.service", "content": unit},
        ],
        "runcmd": [
            ["systemctl", "enable", "beton-escape.service"],
            ["touch", "/etc/beton-escape-ready", "/etc/cloud/cloud-init.disabled"],
            ["sh", "-c", f"uname -r; echo {PROVISIONED} > /dev/ttyS0"],
        ],
        "power_state": {"mode": "poweroff", "condition": True},
    }
    return "#cloud-config\n" + json.dumps(config, indent=2) + "\n"


def qemu(disks, net, console, timeout):
    """Startet die VM bis zum Abschalten; die Konsole landet in `console`."""
    cmd = [
        "qemu-system-x86_64", "-machine", "accel=kvm", "-cpu", "host", "-smp", "4", "-m", "8192",
        "-nographic", "-no-reboot",
    ]
    for path, fmt, ro in disks:
        cmd += ["-drive", f"file={path},if=virtio,format={fmt}" + (",readonly=on" if ro else "")]
    cmd += ["-nic", "user,model=virtio-net-pci"] if net else ["-nic", "none"]
    log("+ " + " ".join(cmd))
    with open(console, "wb") as out:
        try:
            r = subprocess.run(cmd, stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT, timeout=timeout)
        except subprocess.TimeoutExpired:
            show(console)
            raise SystemExit(f"VM nach {timeout} s nicht beendet")
    if r.returncode != 0:
        show(console)
        raise SystemExit(f"qemu endete mit {r.returncode}")


def show(path, title="Konsole der VM"):
    log(f"::group::{title}")
    with open(path, "rb") as f:
        sys.stdout.write(f.read().decode("utf-8", "replace"))
    log("::endgroup::")


def cache_key(image):
    """Hängt nur an Image und Provisionierung, nicht am Rest dieses Skripts."""
    spec = json.dumps(IMAGES[image], sort_keys=True) + user_data(os.getuid())
    return f"{image}-{hashlib.sha256(spec.encode()).hexdigest()[:16]}"


def prepare(args):
    os.makedirs(args.dir, exist_ok=True)
    base = os.path.join(args.dir, "base.qcow2")
    provisioned = os.path.join(args.dir, "provisioned.qcow2")
    if os.path.exists(provisioned):
        log(f"{args.image}: provisioniertes Image aus dem Cache")
        return 0
    fetch(args.image, base)
    with tempfile.TemporaryDirectory() as tmp:
        with open(os.path.join(tmp, "user-data"), "w") as f:
            f.write(user_data(os.getuid()))
        with open(os.path.join(tmp, "meta-data"), "w") as f:
            f.write(f"instance-id: beton-escape-{args.image}\nlocal-hostname: beton-escape\n")
        seed = os.path.join(tmp, "seed.iso")
        sh("genisoimage", "-quiet", "-output", seed, "-volid", "cidata", "-joliet", "-rock",
           os.path.join(tmp, "user-data"), os.path.join(tmp, "meta-data"))
        part = provisioned + ".part"
        sh("qemu-img", "create", "-q", "-f", "qcow2", "-F", "qcow2", "-b", "base.qcow2", part, "16G")
        console = os.path.join(args.dir, "provision.log")
        qemu([(part, "qcow2", False), (seed, "raw", True)], net=True, console=console, timeout=args.timeout)
        text = open(console, "rb").read().decode("utf-8", "replace")
        if PROVISIONED not in text:
            show(console)
            raise SystemExit("Provisionierung ohne Abschlussmeldung")
        show(console, "Konsole der Provisionierung")
        os.replace(part, provisioned)
    return 0


GUEST = r"""#!/bin/sh
# Läuft als root in der VM ohne Netzwerkkarte (scripts/escape_vm.py). Ergebnisse nach out/.
D=/mnt/data
. "$D/env"
mkdir -p "$D/out"
exec >"$D/out/guest.log" 2>&1
set -x
uname -a
ip -o link
# Wie im Job „test (linux, ohne Netzwerk)“: Die Sandbox darf sich nicht auf Yama verlassen.
sysctl -w kernel.yama.ptrace_scope=0 || true
mkdir -p "$WS"
mount --bind "$D/ws" "$WS"
as_runner() {
  runuser -u runner -- env -i HOME=/home/runner USER=runner LANG=C.UTF-8 \
    PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin "$@"
}
cd "$WS"
as_runner "$WS/target/debug/beton" sandbox probe --json > "$D/out/probe.json"
echo $? > "$D/out/probe.status"
as_runner "$WS/target/debug/beton" sandbox test --json > "$D/out/sandbox-test.json"
echo $? > "$D/out/sandbox-test.status"
if [ "$SUITE" = 1 ]; then
  as_runner BETON_TEST_LINUX_BACKENDS="$BACKENDS" "$D/cargo-nextest" nextest run \
    --archive-file "$D/escape.tar.zst" --workspace-remap "$WS" \
    --extract-to "$WS" --extract-overwrite --profile escape --no-tests fail \
    > "$D/out/suite.log" 2>&1
  echo $? > "$D/out/suite.status"
  cp "$WS/target/nextest/escape/junit.xml" "$D/out/junit.xml" || true
fi
cd /
umount "$WS"
echo fertig > "$D/out/done"
"""


def run(args):
    spec = IMAGES[args.image]
    provisioned = os.path.join(args.dir, "provisioned.qcow2")
    if not os.path.exists(provisioned):
        raise SystemExit("erst `prepare` ausführen")
    ws = os.path.abspath(args.ws)
    os.makedirs(args.out, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=args.scratch) as tmp:
        data = os.path.join(tmp, "data")
        src = os.path.join(data, "ws")
        os.makedirs(os.path.join(src, "target", "debug"))
        # Quellbaum (Fixtures, Konfiguration) ohne target/.
        archive = subprocess.run(["git", "-C", ws, "archive", "--format=tar", "HEAD"], check=True, capture_output=True).stdout
        subprocess.run(["tar", "-x", "-C", src], input=archive, check=True)
        # Binaries neben `beton` (m2_demo sucht die Fake-CLI dort); die Test-Binaries kommen aus
        # dem nextest-Archiv.
        for name in args.bins.split(","):
            shutil.copy2(os.path.join(ws, "target", "debug", name), os.path.join(src, "target", "debug", name))
        if spec["suite"]:
            shutil.copy2(args.archive, os.path.join(data, "escape.tar.zst"))
            shutil.copy2(args.nextest, os.path.join(data, "cargo-nextest"))
        with open(os.path.join(data, "run.sh"), "w") as f:
            f.write(GUEST)
        with open(os.path.join(data, "env"), "w") as f:
            f.write(f"WS='{ws}'\nSUITE={1 if spec['suite'] else 0}\nBACKENDS='{spec.get('backends', '')}'\n")
        img = os.path.join(tmp, "data.img")
        sh("mkfs.ext4", "-q", "-L", "BETONDATA", "-d", data, img, "12G")
        disk = os.path.join(tmp, "run.qcow2")
        sh("qemu-img", "create", "-q", "-f", "qcow2", "-F", "qcow2", "-b", provisioned, disk)
        console = os.path.join(args.out, "console.log")
        qemu([(disk, "qcow2", False), (img, "raw", False)], net=False, console=console, timeout=args.timeout)
        fetch_results(img, os.path.join(tmp, "mnt"), args.out)
    out = os.path.join(args.out, "out")
    for name in ("guest.log", "suite.log"):
        if os.path.exists(os.path.join(out, name)):
            show(os.path.join(out, name), name)
    return 0


def fetch_results(img, mnt, dest):
    """Holt `out/` vom Datenlaufwerk (schreibgeschützt eingehängt, wie auf dem Runner üblich
    mit sudo); fehlt es, zeigt es den Inhalt des Laufwerks."""
    os.makedirs(mnt)
    sh("sudo", "mount", "-o", "loop,ro", img, mnt)
    try:
        src = os.path.join(mnt, "out")
        if os.path.isdir(src):
            target = os.path.join(dest, "out")
            shutil.rmtree(target, ignore_errors=True)
            shutil.copytree(src, target)
        else:
            log(f"::warning title=escape_vm::Datenlaufwerk ohne out/: {sorted(os.listdir(mnt))}")
    finally:
        sh("sudo", "umount", mnt)


def read(path):
    try:
        with open(path, encoding="utf-8") as f:
            return f.read().strip()
    except OSError:
        return None


def check(args):
    """Ergebnis der VM: ABI, `beton sandbox test`, Suite. Liefert die Zahl der Fehler."""
    spec = IMAGES[args.image]
    out = os.path.join(args.out, "out")
    errors = []
    if read(os.path.join(out, "done")) != "fertig":
        errors.append("Gast-Skript ist nicht bis zum Ende gelaufen")
    abi = ["--abi", str(spec["abi"])] if "abi" in spec else ["--min-abi", str(spec["min_abi"])]
    if spec.get("unavailable"):
        abi += ["--unavailable", spec["unavailable"]]
    r = subprocess.run([sys.executable, os.path.join(HERE, "check_landlock_abi.py"), os.path.join(out, "probe.json"), *abi])
    if r.returncode != 0:
        errors.append("Landlock-ABI bzw. Backends weichen vom Image ab (QA-005 AC3)")
    # `beton sandbox test` (SBX-010 AC3): besteht, wo die Suite läuft; sonst muss es scheitern.
    status = read(os.path.join(out, "sandbox-test.status"))
    want = "0" if spec["suite"] else "1"
    if status != want:
        errors.append(f"`beton sandbox test` endete mit {status}, erwartet {want}")
    if spec["suite"]:
        if read(os.path.join(out, "suite.status")) != "0":
            errors.append("Escape-Suite fehlgeschlagen (suite.log, junit.xml)")
        if not os.path.exists(os.path.join(out, "junit.xml")):
            errors.append("kein JUnit-Bericht")
    for e in errors:
        log(f"::error title=escape (linux-vm, {args.image})::{e}")
    return 1 if errors else 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("prepare")
    p.add_argument("image", choices=sorted(IMAGES))
    p.add_argument("--dir", required=True)
    p.add_argument("--timeout", type=int, default=600)
    p = sub.add_parser("run")
    p.add_argument("image", choices=sorted(IMAGES))
    p.add_argument("--dir", required=True)
    p.add_argument("--ws", default=ROOT)
    p.add_argument("--archive")
    p.add_argument("--nextest")
    p.add_argument("--bins", default="beton,beton-fake-cli")
    p.add_argument("--out", required=True)
    p.add_argument("--scratch")
    p.add_argument("--timeout", type=int, default=900)
    p = sub.add_parser("check")
    p.add_argument("image", choices=sorted(IMAGES))
    p.add_argument("--out", required=True)
    p = sub.add_parser("cache-key", help="Schlüssel des provisionierten Images (Actions-Cache)")
    p.add_argument("image", choices=sorted(IMAGES))
    args = ap.parse_args(argv)
    if args.cmd == "cache-key":
        print(cache_key(args.image))
        return 0
    return {"prepare": prepare, "run": run, "check": check}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
