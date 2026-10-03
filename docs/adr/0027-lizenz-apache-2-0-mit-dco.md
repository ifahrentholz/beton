# ADR-0027: Lizenz Apache-2.0 mit DCO

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
beton ist Open Source (ADR-0002) und soll in Unternehmen (auch intern im eigenen Team) problemlos einsetzbar sein. Das Vorbild Omnigent steht unter Apache-2.0 mit DCO. Das Rust-Ökosystem nutzt überwiegend MIT/Apache-2.0. Beitragsprozesse sollen leichtgewichtig sein (kein CLA-Verwaltungsaufwand für einen Solo-Maintainer).

## Betrachtete Optionen
1. **A — Apache-2.0** — permissiv, explizite Patentlizenz, unternehmensfreundlich, kompatibel zum Vorbild.
2. **B — Dual MIT/Apache-2.0 (Rust-Konvention)** — maximal kompatibel; Dual-Lizenz ohne konkreten Mehrwert für eine Anwendung (keine Library-first-Ausrichtung).
3. **C — AGPL-3.0** — schützt vor proprietären SaaS-Forks; schreckt Unternehmen ab.
4. **D — Open Core (offener Kern + proprietäre Team-Features)** — Geschäftsmodell; widerspricht ADR-0002.
- Beitragsmodell: **DCO** vs. **CLA**.

## Entscheidung
Option **A – Apache License 2.0**. Beiträge mit **DCO** (Developer Certificate of Origin, `Signed-off-by`-Zeile in jedem Commit), **kein CLA**.

## Konsequenzen
- Positiv: Patentschutz für Nutzer und Beitragende; breite Einsetzbarkeit in Unternehmen; leichtgewichtiger Beitragsprozess.
- Negativ / Risiken: Proprietäre Forks/SaaS-Angebote sind erlaubt; spätere Relizenzierung ohne CLA praktisch ausgeschlossen.
- Folgearbeiten: `LICENSE`, `NOTICE`, DCO-Check in CI, `cargo deny` für Lizenz-Kompatibilität der Abhängigkeiten (ADR-0031), Lizenz-Header-Konvention.

## Bezug
- Spec: docs/spec/12-distribution-quality.md (Prefix DIST/QA)
