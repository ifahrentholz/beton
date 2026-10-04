#!/usr/bin/env bash
# QA-018: M0-Demo-Szenario in einem Netz-Namespace nur mit Loopback (ADR-0033).
#
#   scripts/offline-e2e.sh isolated   Namespace ohne Netz: nur `lo`, keine Route, kein DNS.
#   scripts/offline-e2e.sh sinkhole   Dummy-Interface mit Default-Route und Mitschnitt, DNS-Stub
#                                     auf Loopback (NXDOMAIN). Jede DNS-Anfrage und jedes Paket
#                                     nach außen lässt den Lauf mit Ziel und Zeitpunkt scheitern.
#
# Linux, braucht sudo, tcpdump und python3. Vorher bauen: `cargo build -p beton-cli -p beton-fake-cli`,
# `pnpm install`, `pnpm build` und `playwright install chromium` in apps/web.
set -euo pipefail

mode=${1:?Aufruf: offline-e2e.sh isolated|sinkhole}
repo=$(cd "$(dirname "$0")/.." && pwd)
out=${OFFLINE_E2E_OUT:-$repo/target/offline-e2e/$mode}

if [[ -z ${OFFLINE_E2E_INNER:-} ]]; then
  mkdir -p "$out"
  exec sudo env "PATH=$PATH" "HOME=$HOME" "OFFLINE_E2E_USER=$(id -un)" "OFFLINE_E2E_OUT=$out" \
    "BETON_BIN=${BETON_BIN:-$repo/target/debug/beton}" "CI=${CI:-}" OFFLINE_E2E_INNER=1 \
    unshare --net --mount --propagation private -- "$0" "$mode"
fi

# Ab hier: root im neuen Netz- und Mount-Namespace.
ip link set lo up
resolv=$(mktemp)
echo 'nameserver 127.0.0.1' >"$resolv"
chmod 644 "$resolv"
mount --bind "$resolv" /etc/resolv.conf

pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT

if [[ $mode == sinkhole ]]; then
  ip link add sink0 type dummy
  # Keine IPv6-Autokonfiguration: sonst sendet der Kernel selbst Router-Solicitations.
  ip link set sink0 addrgenmode none 2>/dev/null || true
  sysctl -qw net.ipv6.conf.sink0.router_solicitations=0 net.ipv6.conf.sink0.accept_ra=0 2>/dev/null || true
  ip addr add 192.0.2.1/24 dev sink0
  ip -6 addr add 2001:db8::1/64 dev sink0 nodad 2>/dev/null || true
  ip link set sink0 up
  ip route add default via 192.0.2.254 dev sink0
  ip -6 route add default via 2001:db8::fe dev sink0 2>/dev/null || true
  # ICMPv6 (Neighbor Discovery, MLD) erzeugt der Kernel für das Interface selbst;
  # Verbindungsversuche sind TCP und UDP (inkl. DNS an externe Server).
  tcpdump -i sink0 -n -U -w "$out/sink0.pcap" 'not icmp6' 2>"$out/tcpdump.log" &
  pids+=($!)
  python3 "$repo/scripts/dns-sinkhole.py" "$out/dns.log" &
  pids+=($!)
  sleep 1
elif [[ $mode == isolated ]]; then
  links=$(ip -o link show | awk -F': ' '{print $2}' | tr '\n' ' ')
  routes=$(ip route show; ip -6 route show | grep -v '^::1 ' || true)
  if [[ $links != 'lo ' || -n $routes ]]; then
    echo "Namespace ist nicht isoliert: Interfaces [$links], Routen [$routes]" >&2
    exit 1
  fi
else
  echo "unbekannter Modus: $mode" >&2
  exit 2
fi

echo "== QA-018 ($mode): Interfaces $(ip -o link show | awk -F': ' '{print $2}' | tr '\n' ' ')"
status=0
sudo -u "$OFFLINE_E2E_USER" env "PATH=$PATH" "HOME=$HOME" "BETON_BIN=$BETON_BIN" "CI=$CI" \
  PLAYWRIGHT_HTML_REPORT="$out/report" \
  pnpm --dir "$repo/apps/web" exec playwright test --project chromium --output "$out/test-results" e2e/m0.spec.ts \
  || status=$?

if [[ $mode == sinkhole ]]; then
  cleanup
  pids=()
  sleep 1
  leaks=0
  if [[ -s $out/dns.log ]]; then
    echo "DNS-Anfragen aufgezeichnet (Zeit, Name, Typ):" >&2
    cat "$out/dns.log" >&2
    leaks=1
  fi
  packets=$(tcpdump -r "$out/sink0.pcap" -n -tttt 2>/dev/null || true)
  if [[ -n $packets ]]; then
    echo "Verbindungsversuche nach außen (Zeitpunkt, Ziel):" >&2
    echo "$packets" >&2
    leaks=1
  fi
  if [[ $leaks == 1 ]]; then
    echo "QA-018 AC2 verletzt: beton hat Netz außerhalb von Loopback angefragt." >&2
    exit 1
  fi
  echo "== QA-018 (sinkhole): keine DNS-Anfrage, kein Paket nach außen"
fi
exit "$status"
