# ADR-0012: Agent-Definition – YAML + JSON-Schema, nur MCP für Tools, Skills, Cross-Harness-Sub-Agents, Built-ins als YAML

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Eigene Agents sollen deklarativ, git-versionierbar und harness-neutral beschrieben werden. Omnigents Agent-YAML hat mehrere Doppel-Schreibweisen (`executor.harness` vs. `executor.config.harness`, `policies` vs. `guardrails`), erlaubt Python-Funktionen als Tools und liefert Built-in-Agents (Polly, Debby) als "just YAML". beton ist Rust; Python-Tools scheiden aus. Skills im `SKILL.md`-Format sind de-facto-Standard bei Claude Code/Codex.

## Betrachtete Optionen
1. **A — YAML + veröffentlichtes JSON-Schema** — lesbar, validierbar, Editor-Unterstützung; begrenzte Logik.
2. **B — TOML** — Rust-üblich; tiefe Verschachtelung unhandlich, im Agent-Ökosystem unüblich.
3. **C — Code-basierte Definition (Rust/TS-DSL)** — maximal flexibel; nicht portabel, Build-Schritt nötig.
4. Tools: **MCP-only** vs. zusätzlich native Funktions-Tools (Python/JS) — MCP ist harness-übergreifend nutzbar.

## Entscheidung
Option **A** mit MCP-only-Tools:
- Format **YAML + veröffentlichtes JSON-Schema**; Agent = Verzeichnis (`agent.yaml` + Prompts + Skills), git-versionierbar. Genau eine kanonische Schreibweise.
- Felder (Anlehnung an Omnigent): `executor` (`harness`, `model`, `reasoning_effort`), `instructions`, `tools`, `policies`, `skills`, `spawn`, `timers`, `os_env`/`sandbox`.
- **Tools nur via MCP** (eigene und externe). Eingebaute System-Tools (Sub-Session starten, Nachricht senden, auf Ergebnis warten, Policy abfragen, Timer setzen, Schedule verwalten) werden per MCP an **jeden** Harness exponiert. Keine Python-Funktionen als Tools.
- **Skills** im `SKILL.md`-Format (kompatibel zu Claude Code/Codex).
- **Multi-Agent:** Sub-Agents dürfen auf **anderen Harnesses** laufen (Claude implementiert, Codex reviewt).
- **Built-ins v1 als mitgelieferte YAML** (kein Sondercode): ein Orchestrator (planen → implementieren → Cross-Vendor-Review, à la Polly) und ein Debatten-Modus (à la Debby). Namen frei, orchester-thematisch *(Annahme)*.

## Konsequenzen
- Positiv: Agents sind portabel und reviewbar; Built-ins beweisen die Ausdrucksstärke des Formats.
- Positiv: MCP-only vermeidet Sprach-Runtimes im Core.
- Negativ / Risiken: Einfache Tools erfordern einen MCP-Server statt einer Funktion.
- Folgearbeiten: JSON-Schema-Veröffentlichung + Snapshot-Tests, Skill-Discovery-Regeln, System-Tool-MCP-Server in `beton-mcp`.

## Bezug
- Spec: docs/spec/02-agents.md (Prefix AGT)
