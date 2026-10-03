# ADR-0005: Harness-Integration – ein Adapter-Interface mit drei Transporten; Subscription nur via offizielle CLI

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Kern von beton ist die Anbindung fremder Coding-Agents ("Harnesses"). Omnigent nutzt fünf Integrationsmodi (SDK in-process, CLI-Subprozess, ACP-Subprozess, Native-TUI in tmux, Native-Server), viele davon über Python-/TS-SDKs, die es in Rust nicht gibt. Harte Anforderung: **Claude Pro/Max und ChatGPT Plus/Pro müssen nutzbar sein**, nicht nur API-Keys. Gleichzeitig darf beton keine Subscription-Credentials anfassen.

## Betrachtete Optionen
1. **A — Nur PTY (Vendor-TUI fernsteuern)** — funktioniert mit jedem CLI; aber keine strukturierten Events, fragil, Policies nur über Hooks.
2. **B — Strukturierte native Protokolle pro Harness** — volle Kontrolle (Tool-Calls, Usage, Modellwechsel); hoher Aufwand pro Vendor.
3. **C — Nur ACP (Agent Client Protocol, Zed)** — ein Protokoll für viele Agents; Claude Code/Codex nicht in voller Tiefe abgedeckt.
4. **D — Nur eigener Agent-Loop gegen Vendor-APIs** — volle Kontrolle; Subscriptions nicht nutzbar.

## Entscheidung
**Kombination B + C + A** über **ein Adapter-Interface** (Crate `beton-harness`) mit drei Transporten:
1. **Natives Protokoll** für Top-Harnesses: Claude Code via `claude --input-format stream-json --output-format stream-json` (Rust spricht das Protokoll direkt, kein SDK), Codex via `codex app-server` (JSON-RPC).
2. **ACP** für alle anderen (Gemini CLI, Goose, Qwen, eigene). ACP-Permission-Requests sind Policy-Hooks; beton exponiert System-Tools per MCP an ACP-Agents (abschaltbar).
3. **PTY / Native-TUI** als Zusatzmodus: Original-TUI in einem PTY, gespiegelt; Policies über Vendor-Hooks (Claude-Code-Hooks, Codex-Approval). Eigenes PTY-Multiplexing in Rust, **kein tmux**.

Jeder Adapter deklariert Capabilities (Elicitation/Approval, Resume, Fork-History, Modellwechsel, Usage-Reporting, Auth-Herkunft). Der Direkt-API-Harness (eigener Loop, ADR-0006) ist ein weiterer Adapter.

**Subscription-Regel:** Subscriptions funktionieren nur, wenn die **offizielle Vendor-CLI** läuft und sich selbst authentifiziert (`claude auth login`, `codex login`). beton fasst OAuth-Tokens nie an, bietet **keinen eigenen OAuth-Login**, speichert keine Subscription-Credentials.

## Konsequenzen
- Positiv: Tiefe Integration für Claude Code/Codex, Breite via ACP, Fallback via PTY für Vendor-TUI-Nutzer.
- Positiv: Kein Python/Node-SDK-Zwang im Rust-Core.
- Negativ / Risiken: stream-json- und app-server-Protokolle sind Vendor-intern und können sich ändern → Golden-Transcript-Tests (ADR-0031).
- Negativ / Risiken (rechtlich, **offener Punkt**): Anthropic weist in der Agent-SDK-Doku darauf hin, dass Drittprodukte keinen claude.ai-Login (Subscription) für ihre Nutzer anbieten sollen. Ob das Starten der offiziellen `claude`-CLI mit deren eigener Anmeldung durch beton zulässig ist, muss anhand des **aktuellen Wortlauts** der Anthropic- (und OpenAI-) Nutzungsbedingungen geprüft werden. Bis zur Klärung: keine Werbung mit "Subscription-Nutzung", klare Doku, dass Login/Auth ausschließlich bei der Vendor-CLI liegt.
- Folgearbeiten: Capability-Matrix pro Adapter, Hook-Bridge für PTY-Modus, MCP-Relay für System-Tools.
- Verschärft durch ADR-0034: Kein Feature darf einen API-Key voraussetzen; Modell-Hilfsfunktionen laufen über die eingeloggte Vendor-CLI bzw. den Harness der Session.

## Bezug
- Spec: docs/spec/01-harnesses.md (Prefix HAR)
