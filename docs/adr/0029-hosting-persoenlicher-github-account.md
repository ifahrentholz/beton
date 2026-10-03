# ADR-0029: Hosting auf persönlichem GitHub-Account `ifahrentholz`

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Das Repository, Container-Images (ghcr.io) und npm-Pakete brauchen einen Namensraum. Eine GitHub-Organisation bietet Team-Verwaltung und eine neutrale Marke, verursacht aber Verwaltungsaufwand und setzt eine verfügbare, passende Org-Bezeichnung voraus. Aktuell entwickelt eine Person (ADR-0030).

## Betrachtete Optionen
1. **A — GitHub-Organisation "beton-ai"** (ursprünglich erwogen) — neutrale Marke, später mehrere Maintainer einfach; Verwaltungsaufwand, Name/Marke ungeprüft.
2. **B — Persönlicher Account `ifahrentholz`** — sofort verfügbar, kein Zusatzaufwand; Projekt wirkt personengebunden, spätere Umzüge nötig.
3. **C — Andere Org-Namen** — keine überzeugende Alternative gefunden.

## Entscheidung
Option **B**. Die Org "beton-ai" wurde erwogen und **verworfen** zugunsten des persönlichen Accounts:
- Repository: `github.com/ifahrentholz/beton` (**keine Org**).
- npm-Scope vorerst **`@ifahrentholz`** (`@ifahrentholz/beton-*`, z. B. `@ifahrentholz/beton-sdk`).
- Container: `ghcr.io/ifahrentholz/beton-server`, `ghcr.io/ifahrentholz/beton-runner`.

## Konsequenzen
- Positiv: Kein Verwaltungsoverhead; sofort startklar.
- Negativ / Risiken: Bus-Faktor 1 bei Rechten; ein späterer Umzug in eine Org ändert Repo-URL, npm-Scope und Image-Pfade (GitHub-Redirects helfen nur teilweise, npm-Scopes gar nicht).
- Folgearbeiten / offene Punkte: Spätere GitHub-Org bzw. npm-Scope-Umbenennung bewusst offen; Paketnamen in Doku zentral pflegen, um eine Umbenennung zu erleichtern.

## Bezug
- Spec: docs/spec/12-distribution-quality.md (Prefix DIST), docs/spec/00-overview.md
