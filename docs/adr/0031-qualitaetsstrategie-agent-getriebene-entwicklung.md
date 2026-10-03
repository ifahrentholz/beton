# ADR-0031: Qualitätsstrategie für agent-getriebene Entwicklung

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
beton wird von einer Person mit Coding-Agents entwickelt (ADR-0030). Agents produzieren schnell viel Code, brauchen aber eindeutige Ziele und automatische Leitplanken. Gleichzeitig enthält beton sicherheitskritische Komponenten (Policy, Sandbox, Proxy, Auth), deren Fehler Nutzer gefährden. Vendor-Protokolle (stream-json, app-server) ändern sich ohne Vorankündigung; echte Subscriptions stehen in CI nicht zur Verfügung.

## Betrachtete Optionen
1. **A — Klassisch: Unit-Tests + Code-Review nach Gefühl** — wenig Overhead; bei Agent-Volumen nicht skalierbar, Sicherheitslücken wahrscheinlich.
2. **B — Spec-getrieben mit Feature-IDs, ADRs, Agent-Leitdateien, spezialisierten Testarten und harten CI-Gates** — reproduzierbare Qualität, agent-tauglich; initialer Aufwand.
3. **C — Maximale formale Verifikation sicherheitskritischer Teile** — höchste Sicherheit; unverhältnismäßig für v1.

## Entscheidung
Option **B**:
- **Spec agent-tauglich:** Feature-IDs (z. B. `POL-003`), testbare Akzeptanzkriterien, Meilenstein-Zuordnung.
- **ADRs** für alle Entscheidungen; `AGENTS.md`/`CLAUDE.md` als Leitdateien für Agents.
- **Golden-Transcript-Tests:** aufgezeichnete echte claude/codex-Ausgaben → normalisiertes Event-Modell.
- **Sandbox-Escape-Tests** pro OS in CI (macOS/Linux/Windows-Runner): `~/.ssh` lesen, `curl evil.com`, `/etc` schreiben → muss scheitern.
- **Deklarative Policy-Tests** (YAML: Event rein, Entscheidung raus).
- **Snapshot-Tests** der generierten JSON-Schemas/OpenAPI.
- **E2E mit Playwright** gegen Web-UI/Desktop mit **Fake-Harness** (deterministisch, keine Subscription in CI).
- **CI-Gates:** `cargo clippy -D warnings`, `cargo deny`, `cargo nextest`, Coverage-Floor für `beton-policy` und `beton-sandbox`, Frontend `tsc` + Lint + Vitest.
- **TDD für sicherheitskritische Crates** (Policy, Sandbox, Proxy, Auth); dort ist **menschliches Review Pflicht**.

## Konsequenzen
- Positiv: Agents arbeiten gegen klare, prüfbare Ziele; Regressionen bei Vendor-Protokolländerungen fallen früh auf.
- Positiv: Sicherheitsversprechen (YOLO-Mode) werden automatisiert belegt.
- Negativ / Risiken: CI-Laufzeit und -Kosten (macOS-/Windows-Runner); Golden-Transcripts müssen bei Vendor-Updates neu aufgezeichnet werden.
- Folgearbeiten: Fake-Harness implementieren, Aufnahme-Werkzeug für Golden-Transcripts, Coverage-Schwellen festlegen, ADR-Prozess im README dokumentieren.
- Verschärft durch ADR-0033 (Offline-E2E-Test im Netz-Namespace ohne Netzwerk, QA-018) und ADR-0034 (manuelle Subscription-Verifikations-Checkliste pro Release, QA-019).

## Bezug
- Spec: docs/spec/12-distribution-quality.md (Prefix QA)
