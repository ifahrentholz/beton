#!/usr/bin/env python3
"""DNS-Stub für den Sinkhole-Lauf (QA-018 AC2).

Lauscht auf 127.0.0.1:53 und [::1]:53 (UDP und TCP), beantwortet jede Anfrage mit NXDOMAIN
und schreibt Zeitpunkt, Name und Typ in die Logdatei. Eine leere Logdatei heißt: keine Anfrage.
"""

import socket
import struct
import sys
import threading
import time

log = open(sys.argv[1], "a", buffering=1, encoding="utf-8")
lock = threading.Lock()
PORT = int(__import__("os").environ.get("DNS_PORT", "53"))


def record(text: str) -> None:
    with lock:
        log.write(f"{time.strftime('%Y-%m-%dT%H:%M:%S')} {text}\n")


def nxdomain(query: bytes) -> bytes:
    i, labels = 12, []
    while i < len(query) and query[i]:
        n = query[i]
        labels.append(query[i + 1 : i + 1 + n].decode(errors="replace"))
        i += n + 1
    qtype = struct.unpack(">H", query[i + 1 : i + 3])[0] if i + 3 <= len(query) else 0
    record(f"{'.'.join(labels) or '.'} type={qtype}")
    rd = query[2] & 0x01
    flags = 0x8000 | (rd << 8) | 0x0080 | 0x0003  # QR, RD gespiegelt, RA, NXDOMAIN
    return query[:2] + struct.pack(">HHHHH", flags, 1, 0, 0, 0) + query[12 : i + 5]


def udp(family: int, host: str) -> None:
    s = socket.socket(family, socket.SOCK_DGRAM)
    s.bind((host, PORT))
    while True:
        q, addr = s.recvfrom(4096)
        if len(q) > 12:
            s.sendto(nxdomain(q), addr)


def tcp(family: int, host: str) -> None:
    s = socket.socket(family, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind((host, PORT))
    s.listen()
    while True:
        c, _ = s.accept()
        with c:
            data = c.recv(4096)
            if len(data) > 14:
                r = nxdomain(data[2:])
                c.sendall(struct.pack(">H", len(r)) + r)


threads = []
for fam, host in ((socket.AF_INET, "127.0.0.1"), (socket.AF_INET6, "::1")):
    for fn in (udp, tcp):
        try:
            probe = socket.socket(fam, socket.SOCK_DGRAM)
            probe.close()
        except OSError:
            continue
        t = threading.Thread(target=fn, args=(fam, host), daemon=True)
        t.start()
        threads.append(t)
for t in threads:
    t.join()
