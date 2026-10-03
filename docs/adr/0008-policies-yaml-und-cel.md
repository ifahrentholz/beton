# ADR-0008: Policies – YAML + CEL, eingebaute Regeltypen, hierarchisch, strengere gewinnt

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Policies entscheiden in Echtzeit über Aktionen von Agents (allow/deny/ask) und sind zustandsbehaftet (Kosten, Tool-Historie). Omnigent kombiniert Python-Funktionen, eingebaute Python-Policies und `cel_policy`; die Kombinationssemantik über Ebenen ist widersprüchlich dokumentiert ("first decision wins" vs. "DENY short-circuits") – eine Session-Policy könnte eine Server-Policy aushebeln. beton ist in Rust; Python-Policies scheiden aus.

## Betrachtete Optionen
1. **A — Reines YAML (deklarative Regeltypen)** — einfach, sicher; nicht ausdrucksstark genug für Bedingungen.
2. **B — YAML + CEL-Ausdrücke** — sicher (nicht Turing-vollständig, seiteneffektfrei), schnell, in Rust verfügbar (`cel-rust`); bekannte Sprache (Kubernetes, Envoy).
3. **C — Skriptsprache (Rhai/Lua) oder WASM** — maximal flexibel; schwer zu prüfen, Sandbox-/Laufzeitrisiken.
4. **D — OPA/Rego** — mächtig, etabliert; zusätzliche Runtime, steile Lernkurve, schwergewichtig.

## Entscheidung
Option **B**, mit WASM als späterem Erweiterungspunkt:
- **YAML + CEL** (`cel-rust`) plus eingebaute Regeltypen als Zucker (`spend_cap`, `model_route`, `require_approval`, …).
- **Hook-Punkte:** Session-Start, vor Model-Request, vor Tool-Call, nach Tool-Result, Browser-Aktionen/Navigation.
- **Aktionen:** `allow` / `deny` / `ask` (Approval-Card) / `modify` (z. B. Modell umrouten) / `notify`.
- **Zustand als Variablen:** `session.cost_usd`, `user.daily_cost_usd`, `tool.name`, `tool.args`, …
- **Hierarchie:** Org → Team → User → Projekt → Agent; **strengere Regel gewinnt** (deny > ask > allow), unabhängig von der Ebene. Lokal-only: User → Projekt → Agent.
- Beispiel: `when: tool.name == "bash" && tool.args.command.matches("rm -rf|git push")` → `action: ask`.
- **WASM** (wasmtime + WIT) später für komplexe Erweiterungen (z. B. LLM-Risiko-Klassifizierer), siehe ADR-0018.

## Konsequenzen
- Positiv: Eindeutige, sichere Kombinationssemantik; niedrigere Ebenen können nur verschärfen.
- Positiv: Deklarative Policy-Tests (Event rein, Entscheidung raus) leicht möglich.
- Negativ / Risiken: Kein Ersatz für Omnigents LLM-basierte Policies in v1; `cel-rust`-Reife (Regex, Makros) prüfen.
- Folgearbeiten: Policy-Event-Schema, Approval-Flow (ADR-0013), Budget-Leases (ADR-0010), Coverage-Floor für `beton-policy`.
- Verschärft durch ADR-0034: LLM-basierte Policies (v2) müssen über eine eingeloggte Vendor-CLI bzw. den Harness der Session laufen, nicht über einen API-Key.

## Bezug
- Spec: docs/spec/03-policies.md (Prefix POL)
