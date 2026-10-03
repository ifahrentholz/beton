# ADR-0002: Open Source als Vertriebsmodell

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
beton braucht ein Vertriebs- und Zielgruppenmodell, bevor Lizenz, Hosting, Release-Kanäle und Telemetrie entschieden werden können. Das Vorbild Omnigent ist Open Source (Apache-2.0) und hat in kurzer Zeit eine große Community aufgebaut. Gleichzeitig gibt es einen konkreten ersten Einsatzzweck: die Nutzung im eigenen Team.

Kräfte:
- Meta-Harnesses leben von Breite (viele Harnesses, Runner, Git-Provider) – das skaliert nur mit Community-Beiträgen.
- Sicherheitskritische Komponenten (Sandbox, Proxy, Policies) profitieren von öffentlicher Prüfbarkeit.
- Entwicklung erfolgt solo mit Coding-Agents; kein Vertriebs- oder Support-Team.

## Betrachtete Optionen
1. **A — Internes Werkzeug (closed)** — schnell, keine Öffentlichkeitsarbeit; aber keine Community, kein externer Review.
2. **B — Kommerzielles Produkt / SaaS** — Einnahmen möglich; erfordert Vertrieb, Support, Betrieb – passt nicht zur Solo-Situation.
3. **C — Open Source mit Community als Zielgruppe** — Breite durch Beiträge, Vertrauen durch Transparenz; kein direktes Geschäftsmodell.
4. **D — Open Core** — Basis offen, Team-Features kommerziell; erzeugt Abgrenzungsaufwand und Misstrauen.

## Entscheidung
Option **C**. beton wird als **Open-Source-Projekt** entwickelt. Zielgruppe ist die **Community** (Entwickler und Teams, die Coding-Agents einsetzen). Der **interne Teameinsatz** ist der erste reale Use-Case und dient als Validierung (Dogfooding), bestimmt aber nicht das Produktdesign exklusiv. Alle Features (inkl. Team-/Server-Features) sind Teil des offenen Produkts – kein Open Core.

## Konsequenzen
- Positiv: Community-Beiträge für Harness-/Runner-/Git-Provider-Plugins möglich; öffentliche Sicherheitsprüfung.
- Positiv: Interner Einsatz liefert frühes, realistisches Feedback.
- Negativ / Risiken: Öffentliche Erwartungen (Issues, Support) bei Solo-Entwicklung; Release erst, wenn das Produkt Single-User-tauglich ist (siehe ADR-0030).
- Folgearbeiten: Lizenz (ADR-0027), Hosting (ADR-0029), Telemetrie Opt-in (ADR-0025), Contribution-Guide, Code of Conduct, Security-Policy (`SECURITY.md`).

## Bezug
- Spec: docs/spec/00-overview.md, docs/spec/12-distribution-quality.md (Prefix DIST/QA)
