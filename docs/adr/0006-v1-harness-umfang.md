# ADR-0006: v1-Harness-Umfang

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Omnigent unterstützt rund 20 Harnesses (Claude Code, Codex, Cursor, OpenCode, Pi, Copilot, Devin, Kiro, Hermes, Kimi, Qwen, Goose, Antigravity, generisches ACP …), viele mit eigenen, fragilen Native-Integrationen. Für eine Solo-Entwicklung muss der v1-Umfang so gewählt werden, dass maximale Abdeckung bei minimalem Pflegeaufwand entsteht. Eigene Agents (YAML, ADR-0012) sollen Subscriptions nutzen können und brauchen deshalb einen Harness als Ausführungsumgebung.

## Betrachtete Optionen
1. **A — Nur Claude Code** — minimal; verfehlt den Meta-Harness-Kern (Harness-Wechsel, Cross-Vendor-Review).
2. **B — Claude Code + Codex** — die zwei Top-Harnesses mit Subscription; keine Breite.
3. **C — Claude Code + Codex + generisches ACP + Direkt-API** — Top-Harnesses tief, Breite via ACP, API-Keys/Gateways via eigenem Loop.
4. **D — Breite Parität mit Omnigent** — unrealistischer Pflegeaufwand.

## Entscheidung
Option **C**. v1-Harnesses:
- **Claude Code** – nativ (stream-json) **und** PTY/TUI-Modus.
- **Codex** – nativ (`codex app-server`) **und** PTY/TUI-Modus.
- **Generisches ACP** – deckt Gemini CLI, Goose, Qwen u. a. ab.
- **Direkt-API-/Gateway-Harness** – eigener Agent-Loop in Rust, Anthropic- und OpenAI-kompatibel; deckt Ollama, LiteLLM, OpenRouter, vLLM ab (API-Keys).

Eigene YAML-Agents rufen keine APIs direkt, sondern laufen als Orchestratoren **auf** einem Harness (`executor.harness: claude`) und nutzen so die Subscription. Pi, Copilot, Cursor, OpenCode u. a. folgen später bzw. durch die Community – bevorzugt via ACP oder Out-of-Process-Plugin (ADR-0018).

## Konsequenzen
- Positiv: Kernversprechen (Claude ⇄ Codex im Wechsel, Cross-Vendor-Review) in M1 erreichbar.
- Positiv: ACP liefert Breite ohne Pflege pro Vendor.
- Negativ / Risiken: Nutzer von Cursor/Copilot o. Ä. ohne ACP-Unterstützung müssen auf Plugins warten.
- Negativ / Risiken: Direkt-API-Loop ist ein eigener Agent (Tool-Loop, Compaction) – nicht trivial.
- Folgearbeiten: Fork über Harness-Grenzen (History-Rebuild/Präambel), Import von Claude-/Codex-Chats (M1).

## Bezug
- Spec: docs/spec/01-harnesses.md (Prefix HAR), docs/roadmap.md
