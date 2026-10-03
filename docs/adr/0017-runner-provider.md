# ADR-0017: Runner-Provider – lokal, Remote-Host, Docker/Podman, Kubernetes; Runner-Image; SaaS-Provider v2

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Ein Runner führt genau eine Session aus und kapselt den Harness-Prozess in der Sandbox. Die Frage ist, *wo* Runner laufen können. Omnigent trennt OS-Sandbox ("was darf der Agent") von Cloud-Sandbox-Hosts ("wo läuft der Runner") und unterstützt rund 13 Provider (Modal, Daytona, E2B, K8s, microsandbox …), meist über Python-SDKs. Subscriptions funktionieren nur über die offizielle Vendor-CLI (ADR-0005) – auch im Container.

## Betrachtete Optionen
1. **A — Nur lokal + Remote-Host (`beton host`)** — einfach; keine reproduzierbaren, isolierten Umgebungen für Teams.
2. **B — Lokal, Remote-Host, Docker/Podman, Kubernetes über `RunnerProvider`-Interface; SaaS-Provider v2** — selbst betreibbar, Open-Source-freundlich.
3. **C — Zusätzlich SaaS-Sandbox-Provider (E2B, Daytona, Modal, Fly) in v1** — Cloud-Komfort; viele proprietäre APIs, Pflegeaufwand.

## Entscheidung
Option **B**:
- `RunnerProvider`-Interface: `provision` / `start` / `exec` / `terminate` / optional `snapshot` + deklarierte Capabilities.
- v1-Provider: **lokal**, **Remote-Host** (`beton host`), **Docker/Podman** (lokal oder Remote-Docker-Host; zugleich starke Sandbox-Option unter Windows), **Kubernetes** (Pod/Job, PVC für Workspace).
- Offizielles **Runner-Image** (`ghcr.io/ifahrentholz/beton-runner`) mit vorinstallierten Harness-CLIs.
- **Subscription im Container:** Der Nutzer führt dort einmal `claude auth login` (Device-Flow) aus; das Token liegt in einem persistenten, verschlüsselten Volume. Login erfolgt durch die CLI, nicht durch beton.
- v2: SaaS-Provider; MicroVMs als Community-Provider (Plugin, ADR-0018).

## Konsequenzen
- Positiv: Self-Hosting ohne Fremd-SaaS; Teams bekommen reproduzierbare Umgebungen.
- Positiv: Community kann weitere Provider als Out-of-Process-Plugins ergänzen.
- Negativ / Risiken: Subscription-Login pro Container/Volume ist manuell; Volume-Verschlüsselung und -Lebenszyklus müssen sauber gelöst werden.
- Negativ / Risiken: K8s-Betrieb (RBAC, PVC, Netzwerk-Policies) erhöht Doku- und Testaufwand.
- Folgearbeiten: Label-basiertes Dispatching (ADR-0013), Image-Build-Pipeline multi-arch (ADR-0026), Reaper für verwaiste Runner.
- Verschärft durch ADR-0033: Das Runner-Image ist auch lokal baubar bzw. per Datei ladbar; kein Registry-Zwang.

## Bezug
- Spec: docs/spec/10-runners-extensibility.md (Prefix RUN)
