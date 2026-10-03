# ADR-0025: Telemetrie Opt-in, volle Betreiber-Observability

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Zwei unterschiedliche Bedürfnisse: (1) Produkt-Telemetrie für den Projektinhaber (welche Versionen/Features werden genutzt), (2) Observability für Betreiber eines beton-Servers (Fehlersuche, Kosten- und Policy-Nachvollziehbarkeit). Omnigent hat Server-Telemetrie **default an** (Opt-out per Env), was in der Open-Source-Community oft kritisch gesehen wird. beton verarbeitet sensible Inhalte (Code, Prompts).

## Betrachtete Optionen
1. **A — Keine Produkt-Telemetrie** — maximales Vertrauen; keinerlei Nutzungsdaten.
2. **B — Opt-in-Telemetrie (Default aus)** — Vertrauen bleibt erhalten; wenig Daten.
3. **C — Opt-out-Telemetrie (Default an, wie Omnigent)** — aussagekräftige Daten; Vertrauensverlust, Datenschutzfragen (DSGVO).

## Entscheidung
Option **B** für Produkt-Telemetrie, volle Observability für Betreiber:
- **Produkt-Telemetrie Opt-in** (Default nein): nur anonyme Version, OS und Feature-Zähler, **nie Inhalte**. **Crash-Reports Opt-in**, nie Prompts.
- **Betreiber-Observability:** `tracing`-Logs, **OpenTelemetry** (Traces/Metriken), Prometheus `/metrics`, **ein Trace pro Session** (Prompt → Policy-Entscheidungen → Tool-Calls → Kosten).
- `beton doctor` prüft Sandbox-Fähigkeiten, CLIs, Logins, Ports; `beton diagnose` erzeugt ein **secret-freies** Diagnose-Bundle.

## Konsequenzen
- Positiv: Datenschutzfreundlicher Default; Betreiber haben volle Einsicht in eigene Systeme.
- Positiv: `doctor`/`diagnose` beschleunigen Support bei Issues.
- Negativ / Risiken: Wenig Nutzungsdaten für Priorisierung; Issues/Umfragen müssen das ersetzen.
- Negativ / Risiken: Traces können sensible Inhalte enthalten → Redaktionsregeln für Spans/Attribute nötig.
- Folgearbeiten: Telemetrie-Endpoint und Event-Schema öffentlich dokumentieren; Redaction in `beton diagnose` testen.

## Bezug
- Spec: docs/spec/11-platform-features.md (Prefix OBS)
