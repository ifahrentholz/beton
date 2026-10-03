# ADR-0021: Umfang weiterer Plattform-Features in v1

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Neben den Kernbereichen (Harnesses, Policies, Sandbox, Collaboration) bietet Omnigent zahlreiche Plattform-Features: Usage-Seite, Compaction mit Kontextanzeige, automatische Titel, MCP-Verwaltung, Skills, Inbox, Command-Palette, Themes, Export/Import, Feature-Flags, Branding, Canvas u. v. m. Für v1 muss festgelegt werden, welche davon zum vollständigen Produkt gehören.

## Betrachtete Optionen
1. **A — Minimal (nur Kern)** — schneller; Produkt fühlt sich unfertig an (keine Kostenübersicht, keine Inbox).
2. **B — Kuratierter Satz alltagsrelevanter Features** — vollständiges Nutzungserlebnis ohne Nischen.
3. **C — Omnigent-Parität inkl. Branding/White-Label, Canvas, Theme-Editor** — maximaler Umfang, hoher Pflegeaufwand.

## Entscheidung
Option **B**. v1 umfasst:
- **Usage-/Kostenseite** (nach Session/Tag/Harness/Modell; Preis-Katalog + eigene Preise für Gateway-Modelle).
- **Subscription-Usage** als Token-/Rate-Limit-Nutzung statt €.
- **Compaction + Kontextanzeige** (meist an den Harness durchgereicht).
- **Automatische Session-Titel**.
- **MCP-Server-Verwaltung** auf User-/Projekt-/Agent-Ebene, an jeden Harness durchgereicht.
- **Skills** (ADR-0012).
- **Inbox** (offene Approvals, Fragen, fertige Async-Agents).
- **Command-Palette** (⌘K), Session-Switcher, Shortcuts, Light/Dark + 2–3 Themes.
- **Session-Export/-Import** (JSONL).
- **Interne Feature-Flags**.

Nicht v1: Branding/White-Label, Canvas (ADR-0014), Smart Routing (ADR-0022), Prompt-Cleanup nach Diktat (ADR-0023).

## Konsequenzen
- Positiv: Single-User-Produkt ist zu M3 alltagstauglich; Kostenkontrolle sichtbar.
- Negativ / Risiken: Preis-Katalog muss gepflegt werden (Vendor-Preise ändern sich); Subscription-Usage hängt davon ab, was die CLIs melden.
- Folgearbeiten: Quelle und Update-Mechanismus des Preis-Katalogs, Inbox-Datenmodell, Feature-Flag-Konvention.
- Verschärft durch ADR-0033 (Preis-Katalog nur mitgeliefert, kein Online-Abruf) und ADR-0034 (Titel, Compaction und Zusammenfassungen ohne API-Key über die Vendor-CLI bzw. den Harness der Session).

## Bezug
- Spec: docs/spec/11-platform-features.md (Prefix USE/UX)
