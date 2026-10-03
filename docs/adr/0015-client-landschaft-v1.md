# ADR-0015: Client-Landschaft v1

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Sessions sollen von jedem Gerät aus nutzbar sein. Omnigent hat Web-UI/PWA, tmux-basiertes Terminal, Electron-Desktop, iOS-/Android-WebView-Shells, VS-Code-Extension, Slack-Bot sowie REST/OpenAPI und ein Python-SDK. Für einen Solo-Entwickler ist jede zusätzliche Client-Plattform dauerhafte Pflege. Die Desktop-Technologie ist festgelegt (Tauri, ADR-0004).

## Betrachtete Optionen
1. **A — Nur Web-UI + CLI** — minimal; keine native Desktop-Integration, kein Terminal-Workflow.
2. **B — Desktop, Web/PWA, CLI, TUI, API + TS/Rust-SDK; native Mobile, Slack, VS Code in v2** — deckt Desktop-, Terminal- und Mobil-Nutzung (via PWA) ab.
3. **C — Omnigent-Parität inkl. nativer Mobile-Apps, Slack, VS Code** — maximale Reichweite; zu viel Pflege für v1.

## Entscheidung
Option **B**. v1-Clients:
- **Desktop** (Tauri 2, ADR-0004).
- **Web-UI/PWA** – dieselbe Frontend-Codebasis; Mobile in v1 = **PWA mit Web-Push**.
- **CLI** (`beton run/serve/host/…`).
- **TUI** mit **ratatui**, eigenes PTY-Multiplexing, **kein tmux**, läuft auch unter Windows.
- **REST-API**: OpenAPI 3.1, aus Rust generiert (`utoipa`); **SDKs TypeScript + Rust** (Python v2).

v2: native Mobile-Apps (Tauri Mobile), Slack-Bot, VS-Code-Extension.

## Konsequenzen
- Positiv: Alle Kernnutzungsszenarien ohne native Mobile-Apps abgedeckt; kein tmux-Zwang (Windows-tauglich).
- Positiv: SDKs entstehen aus denselben generierten Typen (ADR-0019).
- Negativ / Risiken: PWA-Web-Push unter iOS mit Einschränkungen; mobile Nutzung braucht erreichbaren Server (Tailscale/LAN + Device-Pairing).
- Negativ / Risiken: TUI mit eigenem PTY-Multiplexing ist aufwendig (Terminal-Emulation, Resize, Scrollback).
- Folgearbeiten: PTY-Crate `beton-pty`, SDK-Paket `@ifahrentholz/beton-sdk`, Crate `beton-sdk`.
- Verschärft durch ADR-0033: Web-Push (Push-Dienste der Browser-Hersteller) ist optional und standardmäßig aus; lokale Desktop-Notifications sind der Standard.

## Bezug
- Spec: docs/spec/08-clients.md (Prefix DESK/WEB/CLI/TUI/API)
