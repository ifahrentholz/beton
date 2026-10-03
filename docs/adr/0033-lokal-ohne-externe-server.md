# ADR-0033: Lokal ohne externe Server – harte, getestete Garantie mit abschließender Ausnahmenliste

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
ADR-0003 macht Lokal-only zum Standardfall, sagt aber nicht, welche Netzverbindungen beton im lokalen Betrieb selbst aufbauen darf. Mehrere Spec-Features kontaktierten standardmäßig externe Dienste oder setzten sie voraus: Update-Prüfung beim Start und alle 6 h (ADR-0026), Web-Push über die Push-Dienste der Browser-Hersteller (ADR-0015), Runner-Image aus einer Container-Registry (ADR-0017), Plugin-Registry (ADR-0018), Downloads von Whisper-Modellen (ADR-0023) und Chrome for Testing (ADR-0016) ohne Offline-Alternative. beton verarbeitet vertrauliche Inhalte und soll auch in abgeschotteten Netzen, im Zug oder bei Ausfall eines Drittanbieters voll funktionieren. Ohne automatischen Nachweis schleichen sich Netzabhängigkeiten (CDN-Schriften, Update-Checks, Telemetrie-SDKs) unbemerkt ein.

## Betrachtete Optionen
1. **A — Best effort, dokumentiert** — wenig Aufwand; Netzabhängigkeiten werden erst bei Nutzern sichtbar.
2. **B — Harte Garantie mit abschließender Ausnahmenliste und E2E-Nachweis ohne Netzwerk** — vertrauenswürdig und prüfbar; jedes netzwirksame Feature braucht einen Schalter und eine Offline-Alternative.
3. **C — Vollständig netzfrei (auch ohne optionale Online-Funktionen)** — maximal einfach zu prüfen; Updates, Plugins, Web-Push und Modell-Downloads entfielen ganz.

## Entscheidung
Option **B**.
- **Garantie:** Server (`beton serve`), Host/Runner und alle Clients (Desktop, Web/PWA, CLI, TUI) laufen **vollständig lokal auf ein und demselben Rechner**, ohne Abhängigkeit von irgendeinem externen Server.
- **Einzige Ausnahme:** die **Modell-Anbieter**, mit denen die Vendor-CLIs bzw. der Direkt-API-Harness sprechen (Anthropic, OpenAI, Google …), inklusive deren Login-/Auth-Hosts. Ohne sie gibt es keine Modelle; lokale Modelle (z. B. Ollama auf Loopback) sind davon nicht betroffen.
- **Alles andere ist optional**, standardmäßig **aus** bzw. läuft **nur auf ausdrückliche Nutzeraktion**, und hat eine **Offline-Alternative**:

  | Funktion | Regel | Offline-Alternative |
  |---|---|---|
  | Update-Prüfung / Auto-Update (CLI, Desktop) | nur auf Klick bzw. nach Opt-in | Update aus lokaler, signierter Datei |
  | Produkt-Telemetrie, Crash-Reports | opt-in (ADR-0025, unverändert) | lokale Crash-Datei, `beton diagnose` |
  | Whisper-Modelle, Chrome for Testing | Download nur auf Klick bzw. Bestätigung | Import einer lokalen Datei (Prüfsumme gegen mitgelieferte Liste); installiertes Chrome/Chromium |
  | Preis-Katalog | nur mitgeliefert, kein Online-Abruf | — (Updates mit dem Release) |
  | Plugin-Registry | nur bei ausdrücklichem `install`/`search`/`update` | Installation aus lokalem Pfad, immer möglich |
  | Schriften, Icons, Editor-/Highlighter-Assets im Frontend | gebündelt, kein CDN | — |
  | Web-Push (Push-Dienste der Browser-Hersteller) | opt-in | lokale Desktop-Notifications (Standard), Inbox |
  | Git-Provider GitHub/GitLab, OIDC, Tailscale | nur wenn konfiguriert; nie für den lokalen Betrieb nötig | lokales Git ohne Provider, lokaler Modus ohne IdP, LAN mit eigenem Zertifikat |
  | Runner-Image aus Container-Registry | Pull nur bei Konfiguration des Docker-/K8s-Providers | lokaler Build bzw. `docker load` aus Datei |
  | Installation fehlender Vendor-CLIs | nur nach Bestätigung (ADR-0026) | vorinstallierte CLI, `BETON_<NAME>_PATH` |
- **Fail closed bleibt:** Fehlt eine optionale Online-Funktion, wird nichts stillschweigend gelockert (Sandbox, Proxy, Policies unverändert).
- **„Offline“ ist ein vollwertiger Zustand:** kein Fehlerdialog, keine Wartezeiten auf Timeouts; netzabhängige Aktionen sind sichtbar deaktiviert und nennen ihre Offline-Alternative.
- **Nachweis:** Ein E2E-Test (QA-018) lässt Server, Client und Fake-Harness in einem **Netz-Namespace ohne Netzwerk (nur Loopback)** laufen und beweist die Demo-Szenarien M0 und M3; ein zweiter Lauf mit Sinkhole weist nach, dass kein Verbindungsversuch nach außen stattfindet.
- Jede Komponente, die Nicht-Loopback-Ziele kontaktieren kann, ist in einer Registry netzwirksamer Funktionen (Name, Zweck, Default, Offline-Alternative) eingetragen; neue Einträge brauchen Default „aus“ oder „nur auf Nutzeraktion“, außer Modell-Anbieter.

## Konsequenzen
- Positiv: beton ist in abgeschotteten Netzen und offline nutzbar; Datenschutz- und Vertrauensversprechen sind automatisiert belegt.
- Positiv: Netzabhängigkeiten werden beim Entstehen sichtbar (Registry, Offline-E2E), nicht erst beim Nutzer.
- Negativ / Risiken: Nutzer erfahren von Updates nur, wenn sie aktiv prüfen oder die Prüfung einschalten → Sicherheitsupdates erreichen Nutzer langsamer; Release-Notes/Kanäle müssen das ausgleichen.
- Negativ / Risiken: Zusätzlicher Aufwand für Offline-Pfade (Update aus Datei, Modell-Import, lokaler Image-Build) und für den Netz-Namespace-Test (nur Linux-CI).
- Folgearbeiten: Registry netzwirksamer Funktionen in `beton-core`; Offline-E2E-Job; Update-aus-Datei für CLI und Desktop; Modell-/CfT-Import; Bundling-Prüfung im Frontend.

## Bezug
- Spec: docs/spec/08-clients.md (DESK, WEB), docs/spec/09-browser.md (BRW), docs/spec/10-runners-extensibility.md (RUN, PLG), docs/spec/11-platform-features.md (VOI, UX, OBS), docs/spec/12-distribution-quality.md (DIST, QA-018)
- Verschärft ADR-0003, ADR-0004, ADR-0013, ADR-0015, ADR-0016, ADR-0017, ADR-0018, ADR-0020, ADR-0021, ADR-0023, ADR-0026, ADR-0031; bestätigt ADR-0025 (Telemetrie und Crash-Reports opt-in)
