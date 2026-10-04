# ADR-0035: Feature-Flag-Konvention

- **Status:** Akzeptiert
- **Datum:** 2026-10-04
- **Entscheider:** Coding-Agent (delegiert, AGENTS.md § 5)

## Kontext
ADR-0021 nimmt interne Feature-Flags in v1 auf und nennt die „Feature-Flag-Konvention“ als Folgearbeit. UX-007 legt Enum, Reifegrade und Aktivierung fest; offen war, wo der Katalog liegt, wer Flags setzen darf, wie die API reagiert und wie unfertige Funktionen aus M1-Arbeitspaketen dahinter landen. Omnigent nutzt eine Env-Variable `OMNIGENT_FEATURES` ohne Reifegrade.

## Betrachtete Optionen
1. **A — Freie Strings in der Konfiguration** — kein Code nötig; Tippfehler bleiben unbemerkt, kein Reifegrad, kein Überblick.
2. **B — Enum mit Katalog in `beton-core`, Aktivierung über User-Config und Env, server-autoritativ** — Tippfehler und entfernte Flags werden gemeldet, die API kann gezielt ablehnen; jedes Flag braucht einen Code-Eintrag.
3. **C — Laufzeit-Flags aus einer Datenbank** — umschaltbar ohne Neustart; braucht UI, Rechte und Migration, für interne Schalter zu schwer.

## Entscheidung
Option **B**.
- Katalog: `beton_core::feature` – Makro `feature_flags!` erzeugt `FeatureFlag`, `CATALOG` und Namen (`snake_case`). Jedes Flag hat Reifegrad (`experimental | beta | stable | removed`) und Kurzbeschreibung.
- Aktivierung: `features: [..]` in der **User**-Konfiguration und `BETON_FEATURES=a,b`; das Projekt (Repository) darf keine Flags setzen. `stable` ist immer an, `removed` wird ignoriert und als „entfernt“ gemeldet, Unbekanntes erzeugt eine Warnung (Log, `beton doctor`) mit Vorschlag bei Tippfehlern, nie einen Abbruch.
- Server-autoritativ: Der Daemon löst die Flags einmal beim Start auf; `GET /v1/info` liefert `features` (aktive Namen); Clients richten sich danach. Eine Funktion hinter einem nicht aktiven Flag antwortet mit `404 feature_disabled`.
- Arbeitspakete legen unfertige Funktionen als `experimental` an, heben sie bei Abnahme auf `stable` und entfernen das Flag erst, wenn Konfigurationen es nicht mehr nennen (vorher `removed`).
- Erstes Flag: `fake_harness` (experimentell) schaltet den Fake-Harness ohne `--dev` frei (HAR-026 AC3); `--dev` aktiviert es ebenfalls.

## Konsequenzen
- Positiv: Unfertiges bleibt ohne Verzweigungen im Build erreichbar für Tests und Demos, aber standardmäßig unsichtbar.
- Positiv: Tippfehler und Altlasten in Konfigurationen werden sichtbar statt still ignoriert.
- Negativ / Risiken: Flags kosten Code-Pfade; ohne Aufräumen wächst der Katalog → Reifegrad `removed` als Zwischenschritt, Abschluss-Pakete prüfen den Katalog.
- Folgearbeiten: Einstellungsseite „Experimentelle Funktionen“ (Screen `settings-flags`) und Diagnose-Ansicht (`diag-doctor`) in der Web-UI; im zentralen Betrieb (M4) Flags pro Server statt pro Daemon.

## Bezug
- Spec: docs/spec/11-platform-features.md (UX-007), docs/spec/01-harnesses.md (HAR-026)
- ADR-0021
