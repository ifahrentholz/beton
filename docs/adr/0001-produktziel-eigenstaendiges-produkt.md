# ADR-0001: Produktziel – eigenständiges Produkt nach Omnigent-Vorbild, keine Kompatibilität

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Omnigent (Databricks, Python, Apache-2.0, Status *alpha*) ist ein "open-source meta-harness": eine gemeinsame Schicht über Coding-Agent-Harnesses (Claude Code, Codex, ACP-Agents u. v. m.) mit Policies, OS-Sandbox und geteilten Live-Sessions. beton soll diese Produktidee in Rust umsetzen.

Kräfte:
- Omnigent ist funktional reich, aber architektonisch stark gewachsen (Hotspots mit 8–10k Zeilen, viele Doppel-Schreibweisen im Agent-YAML, explizit als "TECH DEBT" markierte Kompatibilitätspfade).
- Omnigents Formate (Agent-YAML, REST-API, Responses-artiges Event-Schema, Python-Policies) sind eng an Python und an die OpenAI-Responses-API gekoppelt.
- Ein Solo-Entwickler mit Coding-Agents kann sich keine doppelte Pflege von Kompatibilitätsschichten leisten.
- Omnigent ändert sich wöchentlich mit Breaking Changes – ein bewegliches Ziel.

## Betrachtete Optionen
1. **A — Drop-in-Ersatz (API-/Format-kompatibel)** — Nutzer könnten wechseln; aber Omnigent-Altlasten würden übernommen, ständiges Hinterherlaufen hinter Breaking Changes.
2. **B — Teilkompatibilität (z. B. Agent-YAML importierbar)** — Migrationspfad; trotzdem dauerhafte Kopplung an fremde Formate.
3. **C — Eigenständiges Produkt nach Omnigent-Vorbild** — freie, saubere Architektur; kein Migrationspfad.

## Entscheidung
Option **C**. beton ist ein eigenständiges Produkt, das Omnigent als inhaltliches Vorbild (Feature-Umfang, Konzepte wie Harness/Runner/Host/Policies/Sandbox) nutzt, aber **keine Kompatibilitätspflicht** zu Omnigent-Formaten, -APIs oder -Events hat. Wo Omnigent Widersprüche oder Doppelformen hat, wählt beton genau eine kanonische Form.

## Konsequenzen
- Positiv: Freie Wahl von Event-Modell, Protokoll, Agent-Format und Policy-Sprache; Rust-idiomatische Architektur; keine Altlasten.
- Positiv: Bewusste Verbesserungen gegenüber dem Vorbild möglich (z. B. WS-Resume ab `seq`, echte Windows-Sandbox-Beta, Landlock).
- Negativ / Risiken: Kein Migrationspfad für Omnigent-Nutzer; Agent-YAMLs müssen manuell portiert werden.
- Negativ / Risiken: Gefahr, unbeabsichtigt Features "1:1" nachzubauen statt bewusst zu entscheiden – jede Abweichung/Übernahme braucht eine ADR.
- Folgearbeiten / offene Punkte: Ein optionaler Import-Konverter für Omnigent-Agent-YAML kann später als Community-Werkzeug entstehen (nicht v1).

## Bezug
- Spec: docs/spec/00-overview.md
