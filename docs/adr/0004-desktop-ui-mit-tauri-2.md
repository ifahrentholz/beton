# ADR-0004: Desktop-UI mit Tauri 2

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
beton braucht eine Desktop-App (macOS, Linux; Windows Beta) mit nativem Fenster, Menü, Tray/Dock-Badge, Notifications, Deep-Links `beton://`, Keychain-Zugriff, Auto-Updater und Multi-Server-Profilen. Dieselbe UI soll auch als Web-UI/PWA vom Server ausgeliefert werden. Omnigent nutzt eine dünne **Electron**-Shell um die Server-SPA (Chromium überall, aber großer Footprint). Der Core ist in Rust.

## Betrachtete Optionen
1. **A — Tauri 2 (System-WebView + Rust-Core im selben Prozess)** — kleine Binaries, Rust nativ integriert, Web-Frontend wiederverwendbar für Web/PWA; aber unterschiedliche Web-Engines je OS (WKWebView/WebView2/WebKitGTK).
2. **B — Pure-Rust-GUI (Slint, Iced, egui, GPUI)** — alles in Rust, hohe Performance; aber keine Web-/Mobile-Parität, Monaco/xterm/Diff-Viewer müssten nachgebaut werden, Ökosystem unreif.
3. **C — Pro Plattform nativ (SwiftUI / WinUI / GTK) über Rust-Core (FFI)** — beste Plattform-Integration; dreifacher UI-Aufwand, für Solo-Entwicklung unrealistisch.
4. **D — Electron** — Chromium überall identisch; großer Footprint, Node-Runtime zusätzlich zum Rust-Core.

## Entscheidung
Option **A – Tauri 2**. Die React-UI (ADR-0020) läuft im System-WebView; der Rust-Core läuft im selben Prozess und startet bei Bedarf den lokalen Daemon. Plattformen: macOS und Linux voll, Windows Beta. Native Features: Fenster, Menü, Tray/Dock-Badge, Notifications, Deep-Links `beton://`, Keychain, Auto-Updater, Multi-Server-Profile. Kein Electron, kein Pure-Rust-GUI.

## Konsequenzen
- Positiv: Eine Frontend-Codebasis für Desktop, Web und PWA; kleiner Download; direkte Rust-Integration.
- Positiv: Tauri-Updater mit signierten Manifesten (ADR-0026).
- Negativ / Risiken: WebView-Unterschiede (insb. WebKitGTK unter Linux) erfordern Cross-Engine-Tests.
- Negativ / Risiken: Ein agent-steuerbarer Browser ist mit System-WebViews nicht per CDP machbar → separate Lösung über Chromium/CDP (ADR-0016).
- Folgearbeiten: Mikrofonzugriff für Push-to-Talk (ADR-0023), Signing/Notarisierung, E2E-Tests der Desktop-App mit Playwright (ADR-0031).

## Bezug
- Spec: docs/spec/08-clients.md (Prefix DESK)
