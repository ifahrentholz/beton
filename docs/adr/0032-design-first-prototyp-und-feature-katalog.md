# ADR-0032: Design-first – klickbarer Prototyp aller Screens und Feature-Katalog vor den UI-Arbeitspaketen

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
beton hat viele Oberflächen (Web/PWA, Desktop, TUI) und über 390 Features, von denen ein großer Teil sichtbar ist (Chat-Stream, Approval-Bar, Workspace-Rail, Inbox, Usage, Browser-Panel, Onboarding …). Die UI wird überwiegend von Coding-Agents gebaut (ADR-0030, ADR-0031). Ohne verbindliche Vorlage entscheiden Agents Layout, Zustände und Interaktion pro Arbeitspaket neu; das Ergebnis wird inkonsistent, Leer-, Fehler- und Ladezustände fehlen, und Abweichungen fallen erst im Review auf. Gleichzeitig ist für viele Features (Policies, Sandbox, Proxy) nicht offensichtlich, *wo* ihre Wirkung für den Nutzer sichtbar wird.

## Betrachtete Optionen
1. **A — UI direkt in den Arbeitspaketen entwerfen** — kein Vorlauf; inkonsistente UI, Zustände werden vergessen, viel Nacharbeit.
2. **B — Statische Mockups (Figma o. Ä.)** — schnell; nicht klickbar, Zustände und Abläufe bleiben unscharf, zweites Werkzeug neben dem Code-Stack.
3. **C — Klickbarer Prototyp im Ziel-Stack mit Mock-Daten plus Feature-Katalog** — Abläufe und Zustände sind erlebbar, Komponenten und Tokens wiederverwendbar, jede Feature-ID hat einen Ort; Vorlaufaufwand vor den UI-Paketen.

## Entscheidung
Option **C**. **Für jedes Feature gibt es ein Design.**
- **Prototyp:** unter `design/prototype/` ein klickbarer Prototyp **aller Screens und Zustände** (inkl. leer, lädt, Fehler, offline, ohne Berechtigung, Beta/Capability nicht verfügbar) im Stack von ADR-0020: React 19 + TypeScript + Vite + Tailwind + shadcn/ui, ausschließlich mit **Mock-Daten, ohne Backend**.
- **Feature-Katalog:** Jede Feature-ID der Spec ist entweder **einem Screen bzw. Zustand des Prototyps zugeordnet** oder als **„kein UI“** markiert, mit Verweis, wo ihre Wirkung sichtbar wird (z. B. „SBX-004: kein UI → Tool-Card mit Fehler, `sandbox.violation`-Hinweis“). Der Katalog liegt beim Prototyp und wird bei neuen Feature-IDs fortgeschrieben.
- **Reihenfolge:** Der Prototyp entsteht **vor den UI-Arbeitspaketen (WP-14 ff.)**. Nach **Abnahme durch den Maintainer** ist er die **verbindliche UI-Vorlage**.
- **Abweichungen:** Weicht die spätere UI vom abgenommenen Prototyp ab, braucht der PR eine **Begründung**; dauerhafte Abweichungen werden im Prototyp nachgezogen.
- Der Prototyp ist kein Produktcode: keine Anbindung an Server, SDK oder echte Daten; er wird nicht ausgeliefert.

## Konsequenzen
- Positiv: Konsistente UI über Web, Desktop und PWA; Zustände (leer, Fehler, offline) sind vor der Umsetzung entschieden.
- Positiv: Coding-Agents bekommen pro UI-Feature eine konkrete Vorlage; Reviews prüfen gegen den Prototyp statt gegen Geschmack.
- Positiv: Der Katalog macht Lücken sichtbar (Feature ohne Ort, Screen ohne Feature).
- Negativ / Risiken: Vorlauf vor WP-14 verzögert die erste Web-UI; Prototyp und Produkt können auseinanderlaufen, wenn Abweichungen nicht nachgezogen werden.
- Folgearbeiten: Arbeitspaket für Prototyp und Katalog vor WP-14 schneiden; Abnahme-Issue für den Maintainer; Katalog-Format (z. B. Tabelle Feature-ID → Screen/Zustand bzw. „kein UI“ + Verweis) und ein CI-Check, dass jede Feature-ID im Katalog vorkommt.

## Bezug
- Spec: docs/spec/08-clients.md (Prefix DESK, WEB, TUI), docs/spec/11-platform-features.md (Prefix UX, USE, VOI); Feature-Katalog über alle Prefixe
- ADR-0020 (Frontend-Stack), ADR-0030 (Roadmap, Arbeitspakete), ADR-0031 (Qualitätsstrategie)
