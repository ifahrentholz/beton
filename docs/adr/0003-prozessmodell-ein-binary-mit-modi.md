# ADR-0003: Prozessmodell – ein Binary mit Modi, lokal = zentral nur Konfiguration

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
beton besteht aus Server (Policies, History, APIs, Sharing, Scheduler), Host-Daemon, Runner (eine Session, kapselt Harness in Sandbox), Harness-Adaptern, Egress-/Credential-Proxy und Clients. Omnigent nutzt ein CLI, das automatisch einen lokalen Server (Port 6767) und einen Host-Daemon startet; Hosts und Runner wählen sich per ausgehendem WebSocket-Tunnel beim Server ein. Der Großteil der Nutzung ist Single-User auf dem eigenen Rechner, Team-Betrieb kommt später (M4).

## Betrachtete Optionen
1. **A — Getrennte Binaries (Server, Host, Runner, CLI)** — klare Trennung; aber Installations-, Versions- und Distributionsaufwand vervielfacht.
2. **B — Ein Binary mit Modi, lokale und zentrale Variante unterschiedlich implementiert** — einfach zu installieren; doppelte Codepfade, Divergenzrisiko.
3. **C — Ein Binary mit Modi, lokal und zentral identischer Code, nur Konfiguration** — eine Codebasis, ein Testpfad; Server-Abstraktion auch lokal nötig.
4. **D — Reiner Cloud-Server, lokal nur Thin-Client** — kein Lokal-only-Betrieb, Abhängigkeit vom Netz.

## Entscheidung
Option **C**. Ein Binary `beton` (Crate `beton-cli`) mit Modi:
- `beton serve` – Server; `beton host` – Host-Daemon (startet Runner, meldet sich per **ausgehendem** WebSocket-Tunnel am Server an, keine offenen Ports); `beton run` – Session starten/attachen; Client-Kommandos (`beton setup`, `beton tui`, `beton doctor`, …).
- Default: alles auf einer Maschine, Server als Daemon auf `localhost`. **Lokal und zentral sind derselbe Code; der Unterschied ist nur Konfiguration** (SQLite vs. Postgres, Token vs. OIDC, …).
- **Lokal-only ist der Standardfall und voll funktionsfähig**: alle Harnesses inkl. Subscription, Sandbox, Policies (User/Projekt/Agent), lokal gezählte Budgets, Persistenz, Desktop/CLI/TUI/Web gegen localhost, mehrere Clients auf dieselbe Session.
- Nicht lokal: Collaboration mit anderen Personen; Handy nur, wenn der lokale Server per Tailscale/LAN + Device-Pairing freigegeben wird.
- Die Desktop-App (Tauri) bündelt den Rust-Core und startet bei Bedarf den lokalen Daemon.

## Konsequenzen
- Positiv: Eine Installation, eine Version, ein Testpfad; Upgrade lokal → Team ist reine Konfiguration.
- Positiv: Keine eingehenden Ports auf Hosts/Runnern.
- Negativ / Risiken: Binary wird groß (Server, TUI, Whisper, Browser-Steuerung) – ggf. Cargo-Features für schlanke Server-/Runner-Builds.
- Negativ / Risiken: Server-Abstraktionen (Auth, Repository-Schicht) müssen auch lokal laufen – leichter Overhead.
- Folgearbeiten: Daemon-Lifecycle (Autostart, PID/Lock-Datei, Port-Wahl), Tunnel-Protokoll Host↔Server (ADR-0019), Feature-Flags im Build.

## Bezug
- Spec: docs/spec/00-overview.md, docs/spec/10-runners-extensibility.md (Prefix RUN), docs/spec/08-clients.md (Prefix CLI)
