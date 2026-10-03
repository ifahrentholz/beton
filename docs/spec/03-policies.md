# 03 — Policies

Dieses Kapitel spezifiziert die Policy-Engine von beton: Policy-Dokumente in YAML mit CEL-Ausdrücken, die Hook-Phasen, an denen Policies entscheiden, die Aktionen `allow`/`deny`/`ask`/`modify`/`notify`, die vollständigen Kontext-Variablen je Phase, eingebaute Regeltypen, die Hierarchie Org → Team → User → Projekt → Agent mit präzisem Kombinationsalgorithmus ("strengere Regel gewinnt"), Zustand und Zähler, Approvals, deklarative Policy-Tests, Dry-Run/Explain, die Durchsetzung je Harness-Transport und das Audit jeder Entscheidung.

**Scope:** Crate `beton-policy` (Engine, Hierarchie, Regeltypen, Tests, Explain) und die Gate-Anbindung in `beton-runner`. Nicht hier: Budget-Lease-Protokoll (SYNC in 06-data-sync-protocol.md), Approval-/Inbox-UI (UX in 11-platform-features.md, WEB/DESK in 08-clients.md), Rollen und Freigaben (AUTH/COL), Sandbox als zweite Verteidigungslinie (SBX/PRX in 04-sandbox.md).

**Bezug:** ADR-0008 (YAML + CEL, hierarchisch, strengere gewinnt), ADR-0005 (Durchsetzung über Vendor-Hooks und ACP-Permission-Requests), ADR-0010 (Budget-Leases, server-autoritative Policies), ADR-0013 (Approvals ohne Zuschauer), ADR-0018 (WASM-Erweiterungen), ADR-0031 (deklarative Policy-Tests, Coverage-Floor).

## Konzepte & Begriffe

| Begriff | Bedeutung |
|---|---|
| **Policy-Set** | Eine YAML-Datei bzw. ein Server-Dokument mit `name`, optionalen `defaults` und einer Liste von Regeln. Versioniert über Inhalts-Hash. |
| **Regel** | `id`, Phase (`on`), Bedingung (`when`, CEL) und Aktion – oder ein eingebauter Regeltyp (`type` + `params`), der zu CEL-Regeln entzuckert wird. |
| **Ebene** (`scope`) | Herkunft eines Policy-Sets: `org` > `team` > `user` > `project` > `agent` (links = autoritativer). Lokal-only: `user`, `project`, `agent`. |
| **Effektives Policy-Set** | Vereinigung aller Sets aller Ebenen, die für eine Session gelten; Hash `policy_set_hash`. |
| **Phase** | Hook-Punkt, an dem entschieden wird: `session_start`, `model_request`, `tool_call`, `tool_result`, `browser_navigate`, `browser_action`. |
| **Verdikt** | Ergebnis einer einzelnen Regel: `deny`, `ask`, `modify`, `allow`, `notify` oder `error`. |
| **Entscheidung** | Kombiniertes Ergebnis aller Verdikte einer Phase: `deny`, `ask` oder `allow` (ggf. mit Modifikationen) plus Benachrichtigungen. |
| **Default** | Pro Ebene und Phase konfigurierbares Ergebnis, wenn keine Regel derselben Ebene ausdrücklich erlaubt (`allow`/`ask`/`deny`, Standard `allow`). |
| **Approval** | Menschliche Entscheidung zu einer `ask`-Entscheidung (Approval-Card), mit Timeout und `on_timeout`. |
| **Grant** | Gemerkte Freigabe ("für diese Session erlauben"), die eine bestimmte `ask`-Regel für einen Fingerprint erfüllt. |
| **Enforcement-Level** | Wie stark ein Harness eine Phase durchsetzen kann: `full`, `approval_only`, `observe_only`. |

**Prinzipien:** (1) Die strengere Entscheidung gewinnt über alle Ebenen: `deny` > `ask` > `allow`. Niedrigere Ebenen können nur verschärfen. (2) Fail closed: Engine-Fehler, Timeouts und nicht durchsetzbare Pflichtregeln führen zu Ablehnung. (3) Jede Entscheidung wird als `policy.decision` auditiert. (4) Policies sind deterministisch und seiteneffektfrei; Zustand ändert sich nur über deklarierte `effects`.

## Design

### Policy-Dokument (Beispiel)

```yaml
# .beton/policies/git-and-budget.yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/ifahrentholz/beton/main/schemas/policy.v1.json
spec_version: 1
name: git-and-budget
description: Projekt-Regeln für Git und Kosten
mode: enforce                        # enforce | dry_run (Shadow, POL-027)
defaults:
  browser_navigate: deny             # Default dieser Ebene für diese Phase
rules:
  - id: confirm-destructive-shell
    on: tool_call
    when: tool.kind == "shell" && tool.command.matches("rm -rf|git push")
    action: ask
    severity: high                   # low | medium | high (nur Darstellung)
    reason: "Destruktives oder veröffentlichendes Shell-Kommando"
    approval: { timeout: 30m, on_timeout: deny, remember: session }

  - id: route-when-expensive
    on: model_request
    when: session.cost_usd > 20.0 && request.model.startsWith("claude-opus")
    action: modify
    modify: { model: claude-sonnet-4-5 }
    on_unsupported: deny             # deny | ask | skip, wenn der Harness modify nicht kann
    reason: "Session > 20 USD: auf Sonnet umrouten"

  - id: count-pushes
    on: tool_call
    when: tool.kind == "shell" && shell.has_command(tool.argv, "git", "push")
    action: notify
    notify: { channels: [inbox], message: "Push Nr. {{ state.session.pushes + 1 }}" }
    effects:
      - { op: increment, scope: session, key: pushes }

  - id: allow-docs
    on: browser_navigate
    when: glob(browser.host, "*.rust-lang.org") || browser.host == "localhost"
    action: allow                    # hebt den Default `deny` dieser Ebene auf

  - id: budget
    type: spend_cap
    params: { scope: session, limit_usd: 25, ask_at_usd: [10, 20], on_exceed: deny }
```

### Engine-Schnittstelle (Skizze)

```rust
pub trait PolicyGate: Send + Sync {
    /// Wird von Adaptern/Runner an jedem Hook-Punkt aufgerufen. Blockiert bei `ask` bis zur Auflösung
    /// oder liefert `Pending` (asynchrone Transporte), nie länger als `deadline`.
    async fn evaluate(&self, req: PolicyRequest, deadline: Instant) -> Decision;
    fn explain(&self, req: &PolicyRequest) -> Explanation;      // ohne Seiteneffekte, ohne Approval
}

pub struct PolicyRequest { pub phase: Phase, pub subject: Subject, pub ctx: EvalContext, pub enforcement: Enforcement }
pub enum Outcome { Allow, Deny { reasons: Vec<Reason> }, Ask { approval: ApprovalCard } }
pub struct Decision { pub id: DecisionId, pub outcome: Outcome, pub modified: Option<Subject>,
                      pub notifications: Vec<Notify>, pub trace: Trace }
```

### Kombinationsalgorithmus (verbindlich)

Eingabe: effektives Policy-Set, `PolicyRequest`. Regeln werden in kanonischer Reihenfolge betrachtet: Ebene `org → team → user → project → agent`, innerhalb einer Ebene Set-Name alphabetisch, innerhalb eines Sets in Deklarationsreihenfolge.

1. **Pass 1 – Modifikation.** Alle `modify`-Regeln der Phase auswerten. Treffer werden in kanonischer Reihenfolge angewendet:
   - `model`, `reasoning_effort`, `permission_mode`: **erster Schreiber gewinnt** (also die autoritativste Ebene, darin die früheste Regel). Spätere abweichende Werte werden verworfen und als `conflicts[]` protokolliert.
   - `args` (JSON-Merge-Patch auf Tool-Argumente): Patches werden zusammengeführt; berühren zwei Patches denselben JSON-Pointer mit unterschiedlichen Werten, gewinnt der autoritativere, der andere wird als Konflikt protokolliert.
   - `redact`: kumulativ (Vereinigung aller Muster), wird zuletzt angewendet.
   - Kann der Harness eine Modifikation in dieser Phase nicht anwenden (Enforcement-Matrix), greift `on_unsupported` der Regel (Default `deny`): `deny` bzw. `ask` werden als Verdikt in Pass 2 übernommen, `skip` verwirft die Modifikation mit Warnung.
2. **Pass 2 – Entscheidung.** Alle Nicht-`modify`-Regeln auf dem **modifizierten** Subjekt auswerten (`modify`-Regeln werden nicht erneut ausgewertet; kein Fixpunkt). Verdikte: `deny`, `ask`, `allow`, `notify`, `error`.
3. **Defaults.** Für jede Ebene E mit `defaults.<phase> = D ≠ allow`: D ist *aufgehoben*, wenn in Pass 2 eine `allow`-Regel **derselben Ebene E** gegriffen hat. Nicht aufgehobene Defaults zählen als Verdikt D. (Eine höhere Ebene kann den Default einer niedrigeren nicht aufheben und umgekehrt; Allowlists verschiedener Ebenen wirken dadurch als Schnittmenge.)
4. **Fehler.** Ein `error`-Verdikt einer Regel mit Aktion `deny`/`ask` zählt als `deny` (fail closed); bei `allow` hebt es nichts auf; bei `notify` wird nur protokolliert. CELs fehlerabsorbierende Logik (`false && error == false`) gilt.
5. **Grants.** Ein `ask`-Verdikt einer Regel, für die ein gültiger Grant mit passendem Fingerprint existiert, wird zu `allow`.
6. **Ergebnis.** Irgendein `deny` → `deny` (alle Gründe, primärer Grund = autoritativste Ebene). Sonst irgendein `ask` → **eine** Approval-Card mit allen `ask`-Gründen, zeigt das modifizierte Subjekt. Sonst `allow` mit Modifikationen. `notify`-Verdikte werden immer ausgeführt (auch bei `deny`).
7. **Effects.** `effects` einer Regel werden angewendet, wenn die Regel gegriffen hat und die Endentscheidung in `effects_on` liegt (Default `[allow, deny]`); bei `ask` erst nach Auflösung (`approval.resolved.decision = allow` → wie `allow`).
8. **Dry-Run.** Regeln aus Sets mit `mode: dry_run` (bzw. Regeln mit `dry_run: true`) werden ausgewertet und protokolliert (`would_deny`, `would_ask`, `would_modify`), fließen aber nicht in Schritte 1–7 ein.

## Features

### POL-001 — Policy-Dokumentformat, Ladeorte & Hot-Reload
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Policy-Sets sind YAML-Dateien gemäß veröffentlichtem JSON-Schema `schemas/policy.v1.json` (Struktur siehe Design). Ladeorte je Ebene: User `~/.beton/policies/*.yaml`, Projekt `.beton/policies/*.yaml` im Repo plus Projekt-Policies der Project-Entität (SES-014, ab M3, siehe 07-sessions-collaboration.md), Agent `policies:` im `agent.yaml` (AGT-014 in [02](02-agents.md)), Org/Team serverseitig (POL-008). Änderungen an Dateien werden per Watcher erkannt und gelten ab der nächsten Auswertung.
- **Details:** `beton setup` legt `~/.beton/policies/defaults.yaml` mit sicheren, editierbaren Defaults an (`git_guard` mit `force_push: deny`, `loop_detection` mit `id: loop`) *(Annahme: Defaults auf User-Ebene, weil höhere Ebenen nicht gelockert werden können)*. Ein Reload mit Syntax- oder Typfehler behält die letzte gültige Version und erzeugt eine Fehlermeldung (UI-Toast + `policy.set_invalid`). Erfolgreicher Reload: `policy.set_changed { scope, set, version }` in allen betroffenen aktiven Sessions. Regel-IDs müssen pro Set eindeutig sein (`[a-z0-9-]`, max. 64 Zeichen).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Das Beispiel-Set dieses Kapitels lädt fehlerfrei; Snapshot-Test des generierten Schemas läuft in CI.
  - [ ] AC2 — Wird eine Projekt-Policy-Datei während einer laufenden Session geändert, nutzt die nächste Tool-Call-Auswertung die neue Version (`policy_set_hash` ändert sich) ohne Session-Neustart.
  - [ ] AC3 — Eine fehlerhafte Änderung lässt die alte Version aktiv und erzeugt `policy.set_invalid` mit Datei, Zeile und Fehler.
  - [ ] AC4 — Doppelte Regel-IDs in einem Set sind ein Ladefehler.
- **Abhängigkeiten:** POL-002, AGT-014 ([02](02-agents.md)), SES-014 (ab M3, siehe 07-sessions-collaboration.md)
- **Referenz:** ADR-0008

### POL-002 — CEL-Engine & Erweiterungsfunktionen
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** `when`-Ausdrücke, `modify`-Werte, `effects.value` und `remember_key` sind CEL (`cel-rust`). Ausdrücke werden beim Laden kompiliert und typgeprüft; zur Laufzeit gilt ein Kostenlimit pro Ausdruck.
- **Details:** Standard-CEL inkl. `has()`, `exists`, `all`, `map`, `filter`, `matches` (Regex-Dialekt des `regex`-Crates, RE2-kompatible Teilmenge), `startsWith`, `endsWith`, `contains`, `size`, `duration()`, `timestamp()`. beton-Erweiterungen: `glob(s, pattern)`, `under(path, root)` (normalisiert, symlink-aufgelöst), `shell.has_command(argv, cmd, sub?)`, `shell.has_flag(argv, flag)`, `url.host(u)`, `url.path(u)`, `domain_matches(host, "*.example.com")`, `bytes_human(n)`. Limits: Ausdruck max. 4 KiB, Laufzeit-Kostenbudget je Auswertung (≈ 10 000 Operationen), Gesamtbudget pro `evaluate` 10 ms; Überschreitung = `error`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Ausdruck mit Typfehler (`session.cost_usd > "5"`) wird beim Laden mit Position abgelehnt.
  - [ ] AC2 — `tool.name == "bash" && tool.args.command.matches("rm -rf")` liefert für ein Tool ohne `command` `false` statt eines Fehlers (Error-Absorption).
  - [ ] AC3 — `under("/repo/../etc/passwd", "/repo")` ist `false`; `glob("a.b.example.com", "*.example.com")` ist `true` (Unit-Tests aller Erweiterungsfunktionen).
  - [ ] AC4 — Performance: 200 Regeln, Phase `tool_call`, p99 < 2 ms auf einem CI-Runner (Benchmark-Test mit Schwelle).
- **Abhängigkeiten:** —

### POL-003 — Hook-Phasen & Evaluationspunkte
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Policies entscheiden an sechs Phasen. Jede Phase hat definierte erlaubte Aktionen und modifizierbare Felder.
- **Details:**

  | Phase | Zeitpunkt | Aktionen | modifizierbar |
  |---|---|---|---|
  | `session_start` | vor Start des Harness (auch Fork, Resume, Async-Run) | alle | `model`, `reasoning_effort`, `permission_mode` (nur strenger) |
  | `model_request` | vor jedem Model-Request (Direkt-API) bzw. vor jedem Turn (`request.granularity = "turn"`) | alle | `model`, `reasoning_effort`, `redact` (nur Direkt-API) |
  | `tool_call` | vor Ausführung eines Tool-Calls | alle | `args`, `redact` |
  | `tool_result` | nach Tool-Ausführung, bevor das Result zum Modell geht | `allow`, `deny` (Result wird durch Fehlertext ersetzt bzw. Turn abgebrochen), `modify`, `notify` | `redact` |
  | `browser_navigate` | vor Navigation im beton-Browser | alle | — |
  | `browser_action` | vor click/type/submit/upload/download/eval im beton-Browser | alle | — |

  `session_start` mit `ask` hält den Start bis zur Freigabe an; `deny` erzeugt `session.start_denied { reasons }`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Für jede Phase existiert ein Integrationstest mit dem Fake-Harness, der `deny` durchsetzt (Aktion findet nicht statt) und `policy.decision` mit korrekter `phase` loggt.
  - [ ] AC2 — Eine `modify`-Regel mit einem für die Phase nicht erlaubten Feld (z. B. `args` in `model_request`) ist ein Ladefehler.
  - [ ] AC3 — `session_start` mit `ask` startet den Harness erst nach Freigabe; bei `deny` wird kein Harness-Prozess gestartet.
- **Abhängigkeiten:** POL-001, HAR-001 ([01](01-harnesses.md))

### POL-004 — Kontext-Variablen je Phase
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** CEL-Ausdrücke sehen einen typisierten, read-only Kontext. Die gemeinsamen Variablen gelten in allen Phasen; phasenspezifische nur in ihrer Phase (Zugriff außerhalb ist ein Ladefehler).
- **Details:**
  - **Gemeinsam:** `now` (timestamp); `session.{id, title, harness, mode (native|tui), model, reasoning_effort, permission_mode, cost_usd, tokens_in, tokens_out, turns, tool_calls, started_at, duration_s, is_async, attached_clients, depth, parent_id, project, worktree, branch, labels}`; `run.{id, trigger (interactive|spawn|timer|schedule|webhook|fork|import), cost_usd, subtree_cost_usd, duration_s, turns, budget_usd}` (bei interaktiven Sessions mit `trigger = "interactive"`); `user.{id, email, roles, teams, timezone, daily_cost_usd, monthly_cost_usd}`; `org.id`; `team.ids`; `agent.{name, version, source, hash}` (leer ohne Agent); `host.{id, os, arch, labels, local}`; `actor.{kind (user|agent|system), id}`; `budget.{online, lease_remaining_usd, lease_expires_at}`; `state.{session, run, user_day, user}` (Maps, POL-009); `approvals.{granted_in_session, denied_in_session}`.
  - **`session_start`:** `start.{trigger, harness, model, reasoning_effort, permission_mode, sandbox.enabled, sandbox.backend, sandbox.profile, network.proxy_enabled, worktree, fork_of, agent_ref}`.
  - **`model_request`:** `request.{model, provider, reasoning_effort, granularity (call|turn), prompt (letzte User-Nachricht, max. 64 KiB), messages (nur Direkt-API), tools (Namen), estimated_input_tokens, context_used_pct}`.
  - **`tool_call`:** `tool.{name, native_name, server (MCP-Server oder "builtin"), kind, args, command (Shell-String), argv (geparste Liste), cwd, paths (normalisierte Pfade), urls, call_index, repeat_count, consecutive_errors, is_subagent_spawn}`.
  - **`tool_result`:** alle `tool.*` plus `result.{ok, exit_code, error, text (max. 64 KiB), bytes, duration_ms, truncated}`.
  - **`browser_navigate`:** `browser.{url, scheme, host, path, initiator (agent|user|page), tab_id, profile}`.
  - **`browser_action`:** `browser.{action (click|type|submit|upload|download|eval|select), url, host, selector, element_role, is_form_submit, is_password_field, text_len, file_name}`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Die Variablen sind als Rust-Typen definiert; daraus wird die CEL-Typdeklaration und eine Referenztabelle für die Doku generiert (Snapshot-Test).
  - [ ] AC2 — `result.text` in einer `tool_call`-Regel ist ein Ladefehler mit Hinweis auf die gültige Phase.
  - [ ] AC3 — `session.cost_usd` spiegelt alle bis zum Auswertungszeitpunkt geloggten `cost.delta` wider (Test: Delta → sofortige Auswertung).
  - [ ] AC4 — `user.daily_cost_usd` rechnet in `user.timezone` (Tagesgrenze 00:00 Ortszeit; Default UTC).
- **Abhängigkeiten:** POL-002, POL-005, USE-001 (siehe 11-platform-features.md)

### POL-005 — Tool-Normalisierung
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Damit eine Regel harness-übergreifend gilt, normalisiert beton jeden Tool-Call: kanonische Klasse `tool.kind`, Shell-Parsing (`command`, `argv`, verkettete Kommandos), Pfad-Extraktion (`paths`, absolut, normalisiert) und URL-Extraktion.
- **Details:** Abbildung (Auszug): Claude `Bash` → `shell`; `Read` → `file_read`; `Write` → `file_write`; `Edit`/`MultiEdit` → `file_edit`; `Glob`/`Grep` → `search`; `WebFetch` → `web_fetch`; `Task` → `system` (`is_subagent_spawn`). Codex `exec_command`/`shell` → `shell`; `apply_patch` → `file_edit` (Pfade aus dem Patch). ACP-`kind`: `execute` → `shell`, `read` → `file_read`, `edit`/`delete`/`move` → `file_edit`, `search` → `search`, `fetch` → `web_fetch`, sonst `other`. MCP-Tools → `mcp`, beton-System-Tools → `system`, Browser-Tools → `browser`. Shell-Parsing: Aufteilen an `;`, `&&`, `||`, `|`, Erkennung von `$(…)`/Backticks; `argv` ist die Liste aller Teilkommandos. Ist ein Kommando nicht sicher parsebar (z. B. `eval`, verschachtelte Quotes, Here-Docs mit Kommandos), setzt beton `tool.parse_uncertain = true`; eingebaute Regeltypen werten das als `ask`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `cd repo && git -C . push -f origin main` ergibt `argv` mit zwei Teilkommandos; `shell.has_command(tool.argv, "git", "push")` und `shell.has_flag(tool.argv, "-f")` sind `true`.
  - [ ] AC2 — Ein Codex-`apply_patch` auf zwei Dateien liefert beide Pfade absolut in `tool.paths`.
  - [ ] AC3 — Abbildungstabelle ist als Daten (nicht Code) gepflegt; ein Test stellt sicher, dass jeder in Golden-Transcripts vorkommende `native_name` eine Abbildung hat.
  - [ ] AC4 — `bash -c "$(curl x | sh)"` ergibt `parse_uncertain = true`.
- **Abhängigkeiten:** HAR-004, HAR-006, HAR-007 ([01](01-harnesses.md))

### POL-006 — Aktionen & Entscheidungsmodell
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Regeln haben genau eine Aktion: `allow` (ausdrückliche Erlaubnis; hebt Defaults derselben Ebene auf), `deny` (mit `reason`, die das Modell erhält), `ask` (Approval-Card, POL-010), `modify` (Felder laut Phase, POL-003), `notify` (Kanäle `inbox`, `push`, `log`; blockiert nie). Die Kombination erfolgt nach dem verbindlichen Algorithmus im Design.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Bei einem `deny` erhält das Modell den `reason`-Text als Tool-Fehler bzw. Ablehnungsbegründung (Fake-Harness prüft den Text).
  - [ ] AC2 — `notify` mit Kanal `push` erzeugt eine Notification, ohne die Aktion zu verzögern (Latenz der Entscheidung unverändert ±1 ms).
  - [ ] AC3 — `modify` auf `args` führt dazu, dass der Harness die modifizierten Argumente ausführt; `tool.call.requested` behält die Originalargumente.
- **Abhängigkeiten:** POL-003

### POL-007 — Hierarchie & Kombination (lokale Ebenen)
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Die Ebenen User → Projekt → Agent werden gemäß Kombinationsalgorithmus zusammengeführt: strengere Entscheidung gewinnt, Defaults werden nur durch `allow` derselben Ebene aufgehoben, Modifikations-Konflikte nach "erster Schreiber gewinnt" bzw. JSON-Pointer-Priorität. Ein Repo kann so nur verschärfen; ein `allow` im Agent kann kein `deny`/`ask` aus Projekt oder User aufheben.
- **Akzeptanzkriterien:**
  - [ ] AC1 — User `deny` + Agent `allow` auf dieselbe Aktion → `deny` (Policy-Test).
  - [ ] AC2 — Projekt-Default `browser_navigate: deny` + Agent-`allow` für `github.com` → `deny`; zusätzliches Projekt-`allow` für `github.com` → `allow`.
  - [ ] AC3 — User-Regel `modify model: A` und Agent-Regel `modify model: B` → Modell A; `policy.decision.conflicts` enthält den verworfenen Wert B mit Regel-ID.
  - [ ] AC4 — Zwei `args`-Patches auf denselben Pointer mit unterschiedlichen Werten → Wert der autoritativeren Ebene; disjunkte Pointer werden beide angewendet.
  - [ ] AC5 — Die Auswertung ist unabhängig von der Datei-Lade-Reihenfolge (Property-Test mit permutierten Sets).
- **Abhängigkeiten:** POL-001, POL-006

### POL-008 — Org-/Team-Ebene & Server-Autorität
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Im Team-Betrieb verwaltet der Server Org- und Team-Policy-Sets (API-002 + Admin-UI, siehe 08-clients.md) und ist autoritativ. Lokale Knoten erhalten das für sie gültige Set, cachen es und dürfen nur verschärfen. Änderungen sind versioniert und auditiert (wer, wann, Diff).
- **Details:** Endpunkte `GET/PUT/DELETE /v1/orgs/{org}/policies/{name}`, `…/teams/{team}/policies/{name}`; Rollen Owner/Admin dürfen schreiben (AUTH-014). Verteilung über das signierte Policy-Bundle (SYNC-006 in 06-data-sync-protocol.md); jede Session protokolliert die verwendete Server-Version. Ist ein lokaler Knoten offline, gilt das zuletzt gecachte Set; ein Cache älter als `max_age` des Policy-Bundles (SYNC-006, Default 7 Tage) führt zu `deny` für `session_start` neuer Sessions *(Annahme)*; laufende Sessions verhalten sich gemäß Modus „stale“ (SYNC-006). Ein User in mehreren Teams erhält alle Team-Sets (alle gelten, strengere gewinnt).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Org-Regel `deny` auf Force-Push greift in einer lokal laufenden, synchronisierten Session, auch wenn User- und Projekt-Ebene nichts dazu enthalten.
  - [ ] AC2 — Eine lokale Datei kann eine Org-Regel weder entfernen noch überschreiben (Test: lokale Datei mit gleicher Regel-ID und `allow`).
  - [ ] AC3 — Jede Änderung erzeugt einen Audit-Eintrag mit Diff; `GET …/policies/{name}/history` listet Versionen.
  - [ ] AC4 — Offline mit gültigem Cache laufen Sessions weiter; `policy.decision.policy_set_hash` referenziert die gecachte Version.
- **Abhängigkeiten:** POL-007, SYNC-006 (siehe 06-data-sync-protocol.md), AUTH-014 (siehe 05-security-identity.md), API-002 (siehe 08-clients.md)

### POL-009 — Zustand & Zähler
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Neben systemgepflegten Zählern (`session.cost_usd`, `session.tool_calls`, `user.daily_cost_usd`, …) können Regeln eigenen Zustand über `effects` pflegen: `set`, `increment`, `append` (Liste, max. 100 Elemente), `delete`. Scopes: `session`, `run`, `user_day`, `user`.
- **Details:** `session`/`run`-Zustand wird als `policy.state_changed { scope, key, op, value }` im Session-Log persistiert und ist damit replay- und fork-fähig (ein Fork übernimmt den Zustand bis `up_to_seq`). `user_day`/`user` liegen im Store (lokal bzw. Server), `user_day` wird an der Tagesgrenze in `user.timezone` zurückgesetzt. Key-Namensraum ist pro Scope global; der Linter warnt, wenn verschiedene Sets denselben Key schreiben. Werte max. 4 KiB je Key.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `increment` auf `state.session.pushes` ist nach drei erlaubten Pushes `3`; ein Fork mit `up_to_seq` zwischen zweitem und drittem Push hat den Wert `2` (Test mit Replay).
  - [ ] AC2 — Effects einer Regel mit `ask` werden erst nach Freigabe (`decision: allow`) angewendet, bei `deny` mit Default `effects_on` ebenfalls, bei Ablauf (`via: timeout`) nicht.
  - [ ] AC3 — `user_day`-Zähler werden um 00:00 in der Zeitzone des Users zurückgesetzt (simulierte Uhr).
- **Abhängigkeiten:** POL-006, DATA-002 (siehe 06-data-sync-protocol.md)

### POL-010 — Approvals (`ask`) & Approval-Card
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Eine `ask`-Entscheidung erzeugt genau eine Approval-Card mit allen Gründen. Berechtigt zur Entscheidung sind der Session-Owner und User mit Freigabe `comment_approve` oder `drive` (COL-001, ab M4). Optionen: `approve`, `approve_session` (nur wenn alle beteiligten Regeln `remember: session` erlauben), `deny` (optional mit Kommentar, den das Modell erhält). Timeout und `on_timeout` regeln das Verhalten ohne Antwort (Async-Fall siehe ASY-008 in [02](02-agents.md)). Darstellung: Inbox UX-001, Approval-Bar WEB-007, TUI-004.
- **Details:** Payload (`approval.requested`):
  ```json
  {
    "approval_id": "apr_01JB…", "session_id": "ses_…", "run_id": null, "phase": "tool_call",
    "title": "Shell-Kommando bestätigen", "summary": "git push origin feature/x",
    "subject": { "tool": { "name": "Bash", "kind": "shell", "args": { "command": "git push origin feature/x" },
                 "args_modified": null }, "tool_call_seq": 1842 },
    "reasons": [ { "rule_id": "confirm-destructive-shell", "scope": "project", "set": "git-and-budget",
                   "reason": "Destruktives oder veröffentlichendes Shell-Kommando", "severity": "high" } ],
    "options": ["approve", "approve_session", "deny"],
    "requested_by": { "actor": "agent", "harness": "claude", "agent": "pr-fixer" },
    "context": { "worktree": "/…/wt/feature-x", "branch": "feature/x", "session_cost_usd": 1.23 },
    "expires_at": "2026-10-03T14:30:00Z", "on_timeout": "deny"
  }
  ```
  Auflösung (PROTO-002): `approval.resolved { approval_id, decision: allow|deny|abort, via: user|timeout|policy|system, actor, remember: none|session, comment?, on_timeout_applied? }`. Timeout: kleinster `approval.timeout` aller beteiligten Regeln, sonst `async.approval.timeout` des Agents, sonst 24 h; `on_timeout`: strengster Wert aller beteiligten Regeln (`abort` > `deny` > `allow`); `allow` ist nur wirksam, wenn keine Regel einer autoritativeren Ebene `allow_on_timeout: false` setzt. Grant-Fingerprint: `remember_key` der Regel (CEL), Default `tool.kind + ":" + tool.command` bzw. Hash der normalisierten Args. Offene Approvals überleben Server-/Runner-Neustarts.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Zwei gleichzeitig greifende `ask`-Regeln erzeugen genau eine Card mit zwei Gründen.
  - [ ] AC2 — Nach `approve_session` wird derselbe Befehl in derselben Session ohne Card erlaubt (`policy.decision` verweist auf den Grant); ein anderer Befehl erzeugt wieder eine Card.
  - [ ] AC3 — (ab M4) Ein User mit Freigabe `view` erhält beim Entscheiden 403.
  - [ ] AC4 — Nach Neustart des Daemons ist eine offene Approval weiterhin sichtbar und entscheidbar.
  - [ ] AC5 — Timeout-Kombination: Regel A `30m/deny`, Regel B `2h/abort` → Card läuft nach 30 min ab, angewendet wird `abort`.
- **Abhängigkeiten:** POL-006, COL-001 (ab M4, siehe 07-sessions-collaboration.md), UX-001 (siehe 11-platform-features.md), WEB-007 (siehe 08-clients.md)

### POL-011 — Regeltyp `spend_cap`
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Kostenobergrenze pro `session`, `run` oder `subtree` (Session inkl. aller Child-Sessions). Schwellen in `ask_at_usd` fragen je einmal nach; bei Erreichen von `limit_usd` greift `on_exceed`.
- **Details:** `params: { scope: session|run|subtree, limit_usd, ask_at_usd: [], on_exceed: deny|ask|route, fallback_model?, fallback_effort? }`. Entzuckerung: Regeln in Phase `model_request` (bei `granularity = turn` vor jedem Turn) und `session_start`; `route` erzeugt eine `modify`-Regel auf `fallback_model` statt `deny`. "Je einmal" über `state.<scope>.spend_cap_<id>_asked`. Bei Subscription-Sessions ohne `cost_usd` wird `cost_source = catalog` (Listenpreis-Äquivalent) verwendet, wenn konfiguriert, sonst greift die Regel nicht und meldet `policy.rule_inapplicable` *(Annahme)*.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `limit_usd: 5, ask_at_usd: [3]`: bei 3,10 USD genau eine Card, bei 5,00 USD `deny` vor dem nächsten Model-Request/Turn.
  - [ ] AC2 — `on_exceed: route, fallback_model: claude-haiku-4-5` setzt ab Überschreitung das Modell um; `session.settings_changed.mechanism` ist `live` oder `restart`.
  - [ ] AC3 — `scope: subtree` zählt Child-Kosten mit (Test mit Parent + 2 Childs).
  - [ ] AC4 — `beton policy explain` zeigt die entzuckerten CEL-Regeln.
- **Abhängigkeiten:** POL-009, HAR-017 ([01](01-harnesses.md))

### POL-012 — Regeltyp `daily_budget`
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Tagesbudget pro User über alle Sessions (`scope: user`), im Team-Betrieb zusätzlich pro Team (`scope: team`, M4). Tagesgrenze in `timezone` (Default `user.timezone`). Offline gilt die Budget-Lease (POL-021).
- **Details:** `params: { scope: user|team, limit_usd, ask_at_usd: [], on_exceed: deny|ask|route, fallback_model?, timezone? }`; Prüfung in `session_start` und `model_request`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Zwei parallele Sessions eines Users mit je 30 USD überschreiten `limit_usd: 50`; die nächste Model-Request-/Turn-Auswertung beider Sessions liefert `deny`.
  - [ ] AC2 — Neue Sessions werden nach Überschreitung bereits in `session_start` abgelehnt bzw. umgeroutet.
  - [ ] AC3 — Um 00:00 in der konfigurierten Zeitzone ist das Budget wieder verfügbar.
- **Abhängigkeiten:** POL-009, POL-011

### POL-013 — Regeltyp `model_route`
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Manuelles Modell-Routing per Policy (kein Smart Routing): geordnete Routen setzen Modell/Effort, optional `allowed_models` als Allowlist (alles andere `deny`).
- **Details:**
  ```yaml
  - id: routing
    type: model_route
    params:
      allowed_models: ["claude-sonnet-*", "claude-haiku-*", "gpt-5*"]
      routes:
        - { when: "session.is_async", model: claude-sonnet-4-5, reasoning_effort: medium }
        - { when: "user.daily_cost_usd > 30.0", model: claude-haiku-4-5 }
  ```
  Erste passende Route je Regel gewinnt; entzuckert zu `modify` (Phasen `session_start`, `model_request`) plus `deny` für nicht erlaubte Modelle (Glob-Match).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Start mit `claude-opus-*` bei obiger Allowlist wird mit Begründung abgelehnt.
  - [ ] AC2 — In einer Async-Session wird ohne Zutun das Modell `claude-sonnet-4-5` mit Effort `medium` verwendet.
  - [ ] AC3 — Unterstützt der Harness keinen Modellwechsel (`model_switch: none`), greift `on_unsupported` (Default `deny`) mit erklärender Meldung.
- **Abhängigkeiten:** POL-007, HAR-017 ([01](01-harnesses.md))

### POL-014 — Regeltyp `require_approval`
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Zucker für `ask`: Approval für bestimmte Tools, Tool-Klassen, MCP-Server oder einen CEL-Ausdruck, mit Timeout-, `on_timeout`- und `remember`-Einstellungen.
- **Details:** `params: { tools: ["mcp:github/create_pull_request"], kinds: [shell, file_write], servers: [], match: "<cel>", timeout: 30m, on_timeout: deny, remember: session|none }` (Kriterien ODER-verknüpft).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `kinds: [file_write]` erzeugt für `Write` (Claude), `apply_patch` (Codex) und ACP-`edit` jeweils eine Card.
  - [ ] AC2 — `remember: none` blendet `approve_session` in der Card aus.
- **Abhängigkeiten:** POL-005, POL-010

### POL-015 — Regeltyp `tool_allowlist`
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Erlaubt nur gelistete Tools; alles andere wird abgelehnt. Wird zu `defaults.tool_call: deny` der jeweiligen Ebene plus `allow`-Regeln entzuckert (Schnittmengen-Semantik über Ebenen).
- **Details:** `params: { allow: ["Read", "mcp:github/*"], allow_kinds: [system, file_read, search], allow_servers: [beton], deny_reason: "…" }`; Muster mit `*`-Glob gegen `tool.name`, `mcp:<server>/<tool>` oder `tool.kind`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit `allow_kinds: [system, file_read, search]` werden `Bash` und `Edit` abgelehnt, `Read` und `session_spawn` erlaubt.
  - [ ] AC2 — Allowlists auf Projekt- und Agent-Ebene wirken als Schnittmenge (Policy-Test).
- **Abhängigkeiten:** POL-007

### POL-016 — Regeltyp `path_guard`
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Lese-/Schreibregeln auf normalisierten Pfaden (`tool.paths`). Standard: Schreiben nur im Session-Worktree; Lesen/Schreiben sensibler Muster (`**/.env*`, `**/.git/config`, `~/.ssh/**`, `~/.aws/**`) abgelehnt. Bei Shell-Kommandos ist die Pfaderkennung best effort; die verbindliche Durchsetzung leistet die Sandbox (SBX-003, SBX-004).
- **Details:** `params: { read: { allow: [], deny: [] }, write: { allow: ["${worktree}/**"], deny: [] }, on_violation: deny|ask }`. Variablen `${worktree}`, `${home}`, `${project_root}`. Symlinks werden vor dem Vergleich aufgelöst.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `Write` auf `${worktree}/../other/x` wird abgelehnt (Pfad-Normalisierung).
  - [ ] AC2 — Ein Symlink im Worktree, der auf `~/.ssh/id_ed25519` zeigt, wird beim Lesen abgelehnt.
  - [ ] AC3 — `echo x > /etc/hosts` in einem Shell-Call liefert `/etc/hosts` in `tool.paths` und wird abgelehnt.
- **Abhängigkeiten:** POL-005, SBX-003, SBX-004 (siehe 04-sandbox.md)

### POL-017 — Regeltyp `git_guard`
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Schützt vor riskanten Git-Operationen über Shell **und** MCP-Tools: `push`, `force_push`, Push/Merge auf geschützte Branches, `merge`, `delete_branch`, `history_rewrite` (`reset --hard`, `rebase`, `filter-branch`, `commit --amend` auf bereits gepushte Commits).
- **Details:** `params: { push: allow|ask|deny (Default ask), force_push: deny (Default deny), protected_branches: [main, master], protected_action: deny, merge: ask, delete_branch: ask, history_rewrite: ask, mcp_tools: { merge: ["github/merge_pull_request"], push: [] } }`. Force-Push-Erkennung: `-f`, `--force`, `--force-with-lease`, `+<refspec>`, `--mirror`, `push --delete` (→ `delete_branch`). Erkennung auch mit `git -C <dir>`, `-c key=val`, in verketteten Kommandos; `parse_uncertain` → `ask`. Push-Ziel-Branch aus Refspec bzw. aktuellem Branch (`session.branch`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Policy-Tests decken ab: `git push --force`, `git push origin +main`, `git -C x push -f`, `cd a && git push --force-with-lease` → alle `deny`.
  - [ ] AC2 — `git push origin feature/x` → `ask`; `git push origin main` → `deny` (geschützter Branch).
  - [ ] AC3 — MCP-Tool `github/merge_pull_request` → Entscheidung gemäß `merge`.
  - [ ] AC4 — Ein nicht sicher parsebares Kommando mit `git` darin → `ask`.
- **Abhängigkeiten:** POL-005

### POL-018 — Regeltyp `loop_detection`
- **Meilenstein:** M2 · **Priorität:** Should
- **Beschreibung:** Erkennt festgefahrene Agents: identische Tool-Calls (`tool.kind` + Hash der normalisierten Args) mehrfach im gleitenden Fenster bzw. Serien fehlschlagender Tool-Calls.
- **Details:** `params: { window: 10, identical_threshold: 3, consecutive_errors: 5, action: ask|deny|notify }`; nutzt `tool.repeat_count`, `tool.consecutive_errors`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Der dritte identische Call innerhalb von 10 Calls erzeugt `ask`; ein abweichender Call dazwischen setzt den Zähler nicht zurück, solange das Fenster ihn enthält.
  - [ ] AC2 — Fünf fehlgeschlagene Tool-Calls in Folge lösen die konfigurierte Aktion aus.
- **Abhängigkeiten:** POL-009

### POL-019 — Regeltyp `pii_redaction`
- **Meilenstein:** M2 · **Priorität:** Could
- **Beschreibung:** Regex-basierte Erkennung und Schwärzung von PII und Secrets (`email`, `iban`, `credit_card` mit Luhn-Prüfung, `api_key`-Muster, `private_key`-Blöcke, eigene Muster) in Tool-Results und – beim Direkt-API-Harness – in Model-Requests. Wirkt nur, wo beton den Inhalt kontrolliert (Enforcement-Matrix); sonst `notify`.
- **Details:** `params: { detectors: [email, iban, credit_card, api_key, private_key], custom: [{ name, regex }], phases: [tool_result, model_request], action: redact|deny|notify, replacement: "[REDACTED:{type}]" }`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein MCP-Tool-Result mit einer gültigen Kreditkartennummer erreicht das Modell als `[REDACTED:credit_card]`; eine ungültige (Luhn falsch) bleibt unverändert.
  - [ ] AC2 — Auf einem Harness ohne Result-Modifikation wird statt Schwärzung `notify` mit Hinweis ausgeführt und `enforcement: observe_only` geloggt.
- **Abhängigkeiten:** POL-006, POL-024

### POL-020 — Regeltyp `browser_guard`
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Policies für den eingebetteten Browser (BRW-019, siehe 09-browser.md): Navigations-Allowlist, `ask` vor Formular-Submit, Steuerung von Downloads, Uploads, Passwortfeldern und JS-Auswertung.
- **Details:** `params: { navigate: { allow: ["localhost", "*.github.com"], default: deny|ask }, form_submit: ask, password_fields: deny, downloads: ask, uploads: deny, eval: ask, user_initiated: allow }` (`eval: ask` entspricht dem Default von BRW-012). Vom User selbst ausgelöste Navigation (`browser.initiator == "user"`) ist per Default erlaubt. Entzuckert zu `defaults.browser_navigate` + `allow`-Regeln (Phase `browser_navigate`) und Regeln in `browser_action`. POL-020 ist Owner der Regeltyp-Semantik; die Hook-Punkte im Browser (Submit-Erkennung, Download-Pausierung) implementiert BRW-019.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Agent-Navigation auf eine nicht gelistete Domain wird abgelehnt; das Modell erhält die Begründung als Tool-Fehler.
  - [ ] AC2 — `click` auf einen Submit-Button eines Formulars erzeugt eine Approval-Card mit Screenshot-Ausschnitt-Referenz.
  - [ ] AC3 — Tippen in ein Passwortfeld durch den Agent wird abgelehnt.
- **Abhängigkeiten:** POL-003, POL-004, BRW-019 (siehe 09-browser.md)

### POL-021 — Budget-Leases (Offline)
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Ist ein lokaler Knoten offline, prüfen `daily_budget`/`spend_cap` mit Team-/Org-Bezug gegen eine vom Server vergebene Budget-Lease (Lease-Protokoll und Abrechnung: Owner SYNC-007 in 06-data-sync-protocol.md; POL-021 regelt nur die Auswertung). Ist die Lease aufgebraucht oder abgelaufen, greift `on_lease_exhausted` (Org-Einstellung: `ask` Default oder `deny`) bis zum Reconnect.
- **Details:** Variablen `budget.online`, `budget.lease_remaining_usd`, `budget.lease_expires_at`. Kosten offline werden lokal von der Lease abgezogen und beim Reconnect abgeglichen; Überzug durch laufende Turns wird gemeldet (`budget.lease_overdrawn`, PROTO-002), nicht rückwirkend blockiert.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Offline mit Lease 10 USD: nach 10 USD Verbrauch liefert der nächste Turn `ask` (Default) mit Grund "Offline-Budget aufgebraucht".
  - [ ] AC2 — Nach Reconnect werden Offline-Kosten dem Server-Zähler gutgeschrieben; `user.daily_cost_usd` stimmt mit der Summe überein.
  - [ ] AC3 — Lokal-only-Betrieb (kein Server konfiguriert) nutzt keine Leases; Budgets werden lokal gezählt.
- **Abhängigkeiten:** POL-012, SYNC-007 (siehe 06-data-sync-protocol.md)

### POL-022 — Durchsetzung im Native-TUI-Modus (Vendor-Hooks)
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Im PTY-Modus werden Policies über Vendor-Mechanismen durchgesetzt: Claude Code über session-spezifische Hooks (`UserPromptSubmit` → `model_request` auf Turn-Ebene, `PreToolUse` → `tool_call`, `PostToolUse` → `tool_result`, Mechanismus HAR-013), Codex über seinen Approval-Mechanismus (HAR-014). `ask` wird stets als beton-Approval-Card behandelt, nicht als Vendor-Dialog.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Dieselbe Policy-Test-Suite für `git_guard` liefert im Claude-TUI-Modus dieselben Entscheidungen wie im nativen Modus (E2E mit Fake-TUI bzw. Golden-Hook-Payloads).
  - [ ] AC2 — `PostToolUse` mit Entscheidung `deny` liefert dem Modell die Begründung als Blockier-Feedback.
  - [ ] AC3 — Jede Hook-Entscheidung erzeugt ein `policy.decision` mit `enforcement` gemäß Matrix.
- **Abhängigkeiten:** HAR-013, HAR-014 ([01](01-harnesses.md)), POL-024

### POL-023 — Durchsetzung bei ACP (Permission-Requests)
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Bei ACP-Agents ist `session/request_permission` der Policy-Hook für `tool_call`. beton antwortet immer mit einer einmaligen Option (`allow_once`/`reject_once`), nie mit "immer erlauben", damit jede weitere Aktion erneut geprüft wird. Tool-Calls, die der Agent ohne Permission-Request meldet, sind `observe_only` (Audit + `notify`); Aktionen über beton-Client-Methoden (`fs/*`, `terminal/*`) und MCP sind voll durchsetzbar.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Bietet der Agent `allow_always` an und die Policy erlaubt, antwortet beton mit der `allow_once`-Option (Test mit ACP-Test-Agent).
  - [ ] AC2 — Ein `fs/write_text_file`-Aufruf wird unabhängig von Permission-Requests gegen `path_guard` geprüft.
  - [ ] AC3 — Ein ohne Permission-Request gemeldeter Tool-Call erzeugt `policy.decision { enforcement: "observe_only" }`; matcht eine `deny`-Regel, wird zusätzlich `notify` ausgelöst und der Turn per `session/cancel` abgebrochen.
- **Abhängigkeiten:** HAR-007 ([01](01-harnesses.md)), POL-024

### POL-024 — Enforcement-Matrix & Fail-Closed beim Start
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** beton kennt pro Harness-Transport, Phase und Tool-Klasse das erreichbare Enforcement-Level. Beim `session_start` prüft die Engine, ob jede Regel mit `deny`/`ask`/`modify` durchsetzbar ist. Nicht durchsetzbare Pflichtregeln verhindern den Start; Lücken bei nicht-mutierenden Klassen erzeugen eine Warnung.
- **Details:**

  | Phase | claude nativ / TUI | codex nativ | codex TUI | acp | direct |
  |---|---|---|---|---|---|
  | `session_start` | full | full | full | full | full |
  | `model_request` | full (turn; modify via `set_model`) | full (turn; modify via `turn/start`) | observe_only | full (turn; modify nur mit `set_model`) | full (pro Call, inkl. `redact`) |
  | `tool_call` | full (PreToolUse-Hook für jeden Call, inkl. `args`) | approval_only (shell, file_edit); observe_only (lesende Kommandos) | wie codex nativ (Strategie A) bzw. screen_mirror (B) | approval_only; full für `fs/*`, `terminal/*` | full |
  | `tool_result` | allow/deny/notify (kein `redact`) | observe_only | observe_only | observe_only; full für `fs/*`, `terminal/*` | full |
  | MCP-Tools (alle Harnesses) | full in allen Phasen über den beton-MCP-Proxy (HAR-009) | ← | ← | ← | ← |
  | `browser_*` (alle Harnesses) | full (beton-Browser-Tools) | ← | ← | ← | ← |

  Regel-Feld `enforcement: required|best_effort` (Default `required` für `deny`/`ask`/`modify`, `best_effort` für `notify`). Welche Tool-Klassen eine Regel betrifft, ermittelt eine statische Analyse des CEL-Ausdrucks (`tool.kind == …`, `tool.kind in […]`, `tool.name == …`) oder das optionale Feld `kinds:`; ist das nicht bestimmbar, gilt "alle Klassen". Lücke bei mutierenden Klassen (`shell`, `file_write`, `file_edit`, `mcp`, `browser`) + `required` → Start verweigert (`session.start_denied { reason: enforcement_unavailable, rules }`); Lücke nur bei `file_read`/`search`/`web_fetch` → `policy.enforcement_degraded`-Warnung, Sandbox ist Backstop.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Codex nativ: eine `required`-Regel `deny` mit `tool.kind == "file_read"` startet mit `policy.enforcement_degraded`; ein Fake-Harness mit `observe_only` für `file_edit` und einer `required`-Regel `deny` mit `tool.kind == "file_edit"` startet nicht, die Fehlermeldung nennt die Regel-ID.
  - [ ] AC2 — Die Matrix ist als Daten aus den Harness-Capabilities (HAR-002) abgeleitet, nicht hart codiert; ein Test prüft Konsistenz für alle Katalog-Harnesses.
  - [ ] AC3 — Fehler oder Timeout der Engine während einer Auswertung führen zu `deny` (Fake-Gate mit Fehlerinjektion).
- **Abhängigkeiten:** HAR-002 ([01](01-harnesses.md)), POL-003

### POL-025 — Audit: `policy.decision`-Event
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Jede Auswertung – auch `allow` ohne Treffer – erzeugt genau ein `policy.decision`-Event im Session-Log. Eingaben werden nicht dupliziert, sondern über `subject_ref` (Seq des zugehörigen Events) und Hash referenziert.
- **Details:**
  ```json
  { "type": "policy.decision", "decision_id": "pd_01JB…", "phase": "tool_call",
    "subject_ref": { "seq": 1842, "tool_call_id": "tc_…" }, "subject_hash": "sha256:…",
    "outcome": "ask", "modified": null, "approval_id": "apr_01JB…",
    "matched": [ { "rule_id": "confirm-destructive-shell", "scope": "project", "set": "git-and-budget",
                   "set_version": "sha256:…", "verdict": "ask", "reason": "…", "dry_run": false } ],
    "defaults_applied": [ { "scope": "project", "phase": "tool_call", "default": "allow" } ],
    "conflicts": [], "errors": [], "grants_used": [], "enforcement": "full",
    "policy_set_hash": "sha256:…", "eval_us": 412 }
  ```
  Exportierbar mit dem Session-Export und in OTel-Traces als Span-Events (OBS-003).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Für jedes `tool.call.requested` im Log existiert genau ein `policy.decision` mit passender `subject_ref` (Invarianten-Test über Fake-Sessions).
  - [ ] AC2 — Das Event enthält keine Tool-Argumente im Klartext, nur `subject_hash`.
  - [ ] AC3 — Dry-Run-Treffer erscheinen mit `dry_run: true` und beeinflussen `outcome` nicht.
- **Abhängigkeiten:** POL-006, PROTO-002 (siehe 06-data-sync-protocol.md), OBS-003 (siehe 11-platform-features.md)

### POL-026 — Deklarative Policy-Tests (YAML)
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Policy-Autoren schreiben Tests als YAML: Ebenen-Simulation, Fixtures, Eingabe-Event, erwartete Entscheidung. `beton policy test` führt sie aus (lokal und in CI, inkl. JUnit-Ausgabe). Die eingebauten Regeltypen werden mit derselben Mechanik getestet (Coverage-Floor `beton-policy`, ADR-0031). POL-026 ist Owner des Testformats und von `beton policy test`; das CI-Gate über alle mitgelieferten Policies regelt QA-004.
- **Details:**
  ```yaml
  # .beton/policies/tests/git.policytest.yaml
  spec_version: 1
  layers:                                   # fehlende Ebenen = leer
    user:    [~/.beton/policies/defaults.yaml]
    project: [../git-and-budget.yaml]
    agent:   [../../agents/pr-fixer/agent.yaml]     # nutzt dessen policies-Block
  harness: codex                            # optional: Enforcement-Matrix/modify-Fähigkeiten
  fixtures:
    session: { harness: codex, cost_usd: 0.5, branch: feature/x }
    user: { daily_cost_usd: 12.0, timezone: Europe/Berlin }
  cases:
    - name: Force-Push wird geblockt
      phase: tool_call
      input: { tool: { name: exec_command, kind: shell, args: { command: "git push --force origin main" } } }
      expect: { outcome: deny, rules: [no-force-push], reason_contains: force }
    - name: Opus wird bei hohen Kosten umgeroutet
      phase: model_request
      given: { session: { cost_usd: 25.0 } }
      input: { request: { model: claude-opus-4-1, granularity: turn } }
      expect: { outcome: allow, modified: { model: claude-sonnet-4-5 } }
    - name: Dritter identischer Call fragt nach
      steps:
        - { phase: tool_call, input: { tool: { name: Bash, kind: shell, args: { command: "npm test" } } }, expect: { outcome: allow } }
        - { repeat: 1 }
        - { advance_time: 10s, cost_delta_usd: 0.2 }
        - { phase: tool_call, input: { tool: { name: Bash, kind: shell, args: { command: "npm test" } } },
            expect: { outcome: ask, rules: [loop] }, resolve: { decision: allow, remember: none } }
      expect_state: { session: { pushes: 0 } }
  ```
  `expect`-Felder: `outcome`, `rules` (Teilmenge), `reason_contains`, `modified` (Teilmenge), `notifications`, `conflicts`, `enforcement`. Uhr ist simuliert und startet bei `fixtures.now` (Default 2026-01-01T12:00:00Z).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton policy test .beton/policies/tests/` liefert Exit-Code 0 bei Erfolg, 1 bei Fehlschlag und zeigt pro Fehlschlag erwartete vs. tatsächliche Entscheidung samt Explain-Trace.
  - [ ] AC2 — `--junit out.xml` erzeugt valides JUnit-XML.
  - [ ] AC3 — Mehrschrittige Fälle tragen Zustand (`effects`, Zähler, Uhr) korrekt von Schritt zu Schritt.
  - [ ] AC4 — Jeder eingebaute Regeltyp hat eine mitgelieferte Testdatei; CI erzwingt den Coverage-Floor von `beton-policy`.
- **Abhängigkeiten:** POL-007, POL-009, QA-004, QA-011 (siehe 12-distribution-quality.md)
- **Referenz:** ADR-0031

### POL-027 — Dry-Run, Shadow-Modus & Explain
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Nutzer müssen verstehen, *warum* etwas geblockt wurde, und neue Regeln gefahrlos einführen können. `explain` rekonstruiert jede Entscheidung; `eval` wertet eine hypothetische Anfrage ohne Seiteneffekte aus (auch über das System-Tool `policy_query`, AGT-007); `mode: dry_run` aktiviert Regeln im Schattenbetrieb.
- **Details:** `beton policy explain --session <id> --seq <n>` bzw. Klick auf "Warum?" an einer Ablehnung in der UI: zeigt effektives Set (mit Herkunft je Regel), Pass-1-Modifikationen, Konflikte, Pass-2-Verdikte, Defaults (aufgehoben/nicht aufgehoben), Fehler, Grants, Endergebnis. `beton policy eval --phase tool_call --input req.json [--session <id>]` nutzt den Kontext einer bestehenden Session oder Fixtures. `beton policy show --effective [--session <id>]` listet alle geltenden Regeln mit Ebene. Explain-Daten werden aus `policy.decision` + gespeichertem Set-Snapshot rekonstruiert (Sets werden pro Version im Blob-Store gehalten).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Für eine abgelehnte Aktion nennt `explain` die auslösende Regel mit Datei/Ebene und den ausgewerteten Teilausdruck.
  - [ ] AC2 — `explain` einer Entscheidung von vor einer Policy-Änderung zeigt die damals gültige Set-Version.
  - [ ] AC3 — `policy eval` erzeugt keine Events, keine Approvals und ändert keinen Zustand.
  - [ ] AC4 — Eine Regel im `dry_run` erzeugt `would_deny`-Einträge, die in der Usage-/Policy-Ansicht aggregiert sichtbar sind (Anzahl Treffer je Regel und Tag).
- **Abhängigkeiten:** POL-025

### POL-028 — WASM-Policy-Erweiterungen
- **Meilenstein:** M5 · **Priorität:** Could
- **Beschreibung:** Für Logik, die CEL nicht ausdrücken kann, können Regeln ein WASM-Modul (wasmtime + WIT) referenzieren. Module erhalten denselben Kontext wie CEL und liefern ein Verdikt; sie laufen ohne Netz- und Dateizugriff mit Speicher- und Fuel-Limit. Paketierung und Installation über das Plugin-System (PLG-005, siehe 10-runners-extensibility.md).
- **Details:**
  WIT-Welt `beton:policy` und wasmtime-Host: Owner PLG-014 (siehe 10-runners-extensibility.md); POL-028 ist Owner der Policy-Syntax und -Semantik. Regel: `{ id: x, on: tool_call, wasm: { module: "acme/risk@1.2.0", params: {…} } }` (Plugin-Referenz) oder `wasm: { module: ./risk.wasm, sha256: "…", params: {…} }` (lokaler Pfad, `sha256` Pflicht). Limits gemäß PLG-014: 64 MiB Speicher, Fuel-Limit und 50 ms Wall-Clock pro Aufruf; Überschreitung = `error` (fail closed).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Beispielmodul, das `deny` für Kommandos mit mehr als 3 Pipes liefert, wird geladen und in Policy-Tests genutzt.
  - [ ] AC2 — Ein Modul mit Endlosschleife wird nach Fuel-Limit abgebrochen; Ergebnis `deny` mit `error` im Trace.
  - [ ] AC3 — Ein Modul ohne passende WIT-Version wird beim Laden abgelehnt.
- **Abhängigkeiten:** POL-002, PLG-014 (siehe 10-runners-extensibility.md)
- **Referenz:** ADR-0008, ADR-0018

## Nicht in v1

- LLM-basierte Policies (Risiko-Klassifizierer, Prompt-Policies, Intent-Gate, "trivial → teures Modell"-Erkennung, Themenwechsel-Erkennung) – v2, voraussichtlich als WASM- oder Plugin-Erweiterung; Modellaufrufe dann über eine eingeloggte Vendor-CLI im Einmal-Modus bzw. den Harness der Session, ein API-Key ist nur zusätzliche Option (ADR-0034).
- Smart Routing / lernender Router (ADR-0022); v1 kennt nur manuelles Routing per `model_route`. Ein Router in v2 folgt ADR-0034 (Subscription-first).
- Policies in natürlicher Sprache per Chat anlegen ("Plain-Language-Policies").
- Ad-hoc-Session-Policies als eigene Hierarchie-Ebene unterhalb von Agent.
- Proxy-basierte Inspektion von Model-Requests nativer Harnesses (Phase `model_request` pro Call statt pro Turn bei Claude/Codex).
- ML-basierte PII-Erkennung (v1 nur Regex/Luhn, POL-019).
- Domänenspezifische Regeltypen für Google Workspace, Gmail o. Ä. (Omnigent `gdrive_policy`, `gmail_policy`).
