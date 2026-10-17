# ADR-0041: Projekt- und Agent-Ebene dürfen Sandbox und Modus nur verschärfen

- **Status:** Akzeptiert
- **Datum:** 2026-10-16
- **Entscheider:** Ingo Fahrentholz (Entscheidung vom 2026-10-14 in #200); festgehalten von einem Coding-Agent, umgesetzt in WP-49 (#172, PR #327)

## Kontext
Laut Spec (04-sandbox.md, „Sandbox-Konfiguration“) kommt `sandbox:` aus Agent-YAML, Projekt-`.beton/config.yaml` und User-`~/.beton/config.yaml`; Grants gelten von der spezifischsten Ebene. Projekt-Konfiguration und Projekt-Agents (`<repo>/.beton/agents/`) sind aber Repo-Inhalt, also Angreifer A2 des Bedrohungsmodells. Ein bösartiges Repo könnte sich `read_paths: ["~"]` und `egress_rules: ["* evil.example/**"]` geben und Daten abfließen lassen. Dasselbe gilt für Modus und Policies: Ein Projekt-Agent mit `executor.permission_mode: yolo` oder eine Projekt-Regel mit `modify.args` würde die Wahl des Users lockern (#234, #258).

## Betrachtete Optionen
1. **A — Alle Ebenen dürfen Grants geben** — flexibel, aber jedes Repo kann die Sandbox öffnen.
2. **B — Unterhalb der User-Ebene nur verschärfen** — Lockern nur in der User- bzw. Server-Konfiguration.
3. **C — Grants aus dem Repo nach ausdrücklichem Vertrauen pro Repo** (wie `direnv allow`) — bequem für Teams; ein weiterer Vertrauenszustand mit eigener Angriffsfläche (Änderungen nach dem Vertrauen).

## Entscheidung
Option **B**:
- **Projekt-Ebene:** lehnt `sandbox:` weiter ab (fail closed). Einschränkende Angaben werden erst zugelassen, wenn ein Anwendungsfall sie braucht, und dann nur verschärfend.
- **Agent-Ebene:** darf nur verschärfen: ein Backend verlangen (nur das, das die Session ohnehin nutzt), `allow_network: false`, `workspace: ro`, `preset: readonly`, `require_tool_isolation: true`, zusätzliche `masks` (vereinigt mit den Standard-Masken), niedrigere `limits`. Grants, `backend: none` und lockernde Werte sind ein Startfehler. Presets unterhalb der User-Ebene sind nur verschärfend.
- **Modus und Policies:** Ein Projekt-Agent setzt `permission_mode: yolo` nicht selbst; Agent-Policies dürfen `modify.permission_mode`, `modify.args` und `on_timeout: allow` nicht verwenden und nur eigenen Zustand (`agent/<set>/…`) schreiben (AGT-014). `modify.args` gilt erst ab der User-Ebene (POL-003).
- Option **C** wird nur über ein neues ADR wieder aufgegriffen.

## Konsequenzen
- Positiv: Repo-Inhalt kann die Sandbox nicht öffnen; die Entscheidung des Users bleibt die lockerste Stufe.
- Negativ / Risiken: Teams können projektspezifische Freigaben nicht im Repo mitliefern; jeder Nutzer pflegt sie in seiner User-Konfiguration. Offen bleibt `accept_edits` aus Projekt-Agents (#258) und die Effects der Projekt-Ebene (#268).
- Folgearbeiten: `beton agent validate` macht dieselben Prüfungen wie der Start (#324); #258, #268.

## Bezug
- Spec: docs/spec/04-sandbox.md (Sandbox-Konfiguration, SBX-003), docs/spec/02-agents.md (AGT-014), docs/spec/03-policies.md (POL-003, POL-007)
- ADR-0007, ADR-0008
- Issues: #200, #234, #258, #268, #324
