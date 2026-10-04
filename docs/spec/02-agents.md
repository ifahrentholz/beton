# 02 — Agents & Automation

Dieses Kapitel spezifiziert eigene Agents und ihre autonome Ausführung. Teil **AGT** beschreibt das Agent-Format (YAML + veröffentlichtes JSON-Schema), das Agent-Verzeichnis, den Executor, Instructions, Tools (ausschließlich MCP), die eingebauten System-Tools, Skills im `SKILL.md`-Format, Sub-Agents über Harness-Grenzen und die mitgelieferten Built-in-Agents. Teil **ASY** beschreibt Async-Agents: Sessions ohne angeschlossenen Client, gestartet per `spawn(async: true)`, Timer, Schedule (Cron) oder API-Webhook, inklusive Approvals ohne Zuschauer, Inbox-Anbindung und Kosten-Caps.

**Scope:** Crates `beton-agents` (Format, Schema, Skills, Built-ins), `beton-mcp` (System-Tools), Scheduler-Teil von `beton-server`. Nicht hier: Harness-Anbindung (HAR in [01](01-harnesses.md)), Policy-Semantik (POL in [03](03-policies.md)), Sandbox-Felder im Detail (SBX in 04-sandbox.md), Session-/Fork-/Worktree-Semantik (SES/GIT in 07-sessions-collaboration.md), Runner-Provider (RUN in 10-runners-extensibility.md), Inbox-UI und Notifications (UX in 11-platform-features.md, WEB/DESK in 08-clients.md).

**Bezug:** ADR-0012 (Agent-Definition), ADR-0013 (Async-Agents, Timer, Schedules), ADR-0005/0006 (Agents laufen *auf* Harnesses und nutzen so Subscriptions), ADR-0008 (Policies im Agent).

## Konzepte & Begriffe

| Begriff | Bedeutung |
|---|---|
| **Agent** | Verzeichnis mit `agent.yaml` (+ Prompts, Skills, Sub-Agents, Policies); läuft immer auf einem Harness (`executor.harness`), ruft nie selbst APIs. |
| **Agent-Ref** | Verweis auf einen Agent: Name (`maestra`), relativer/absoluter Pfad (`./agents/reviewer`) oder Built-in (`builtin:maestra`). |
| **Resolved Agent** | Vollständig aufgelöste, validierte Agent-Definition inkl. Inhalts-Hash; wird pro Session als Snapshot gespeichert. |
| **System-Tools** | Eingebaute Tools von beton (Sub-Session starten, Nachricht senden, warten, Policy abfragen, Timer, Schedules, Skills), ausgeliefert über den MCP-Server `beton`. |
| **Skill** | Ordner mit `SKILL.md` (YAML-Frontmatter + Anleitung) und optionalen Dateien; kompatibel zu Claude Code/Codex. |
| **Sub-Agent / Child-Session** | Von einer Parent-Session gestartete eigene Session, ggf. auf anderem Harness; bildet einen Session-Baum. |
| **Async-Agent** | Session ohne angeschlossenen Client. Auslöser (`trigger`): `spawn`, `timer`, `schedule`, `webhook`. |
| **Run** | Eine Ausführung eines Async-Agents mit Status, Budget und Ergebnis; gehört zu genau einer Session. |
| **Schedule** | Wiederkehrender Auslöser (Cron + IANA-Timezone). **Timer** = einmaliger Auslöser ("in 2h", Zeitpunkt). |
| **Trigger** | Per API auslösbarer Endpunkt (Webhook) mit Token, der einen Run startet. |

## Design

### Verzeichnislayout eines Agents

```
.beton/agents/pr-fixer/
  agent.yaml                 # Pflicht
  prompts/system.md          # referenziert über instructions.file
  skills/fix-ci/SKILL.md     # agent-eigene Skills
  skills/fix-ci/scripts/run-ci-locally.sh
  agents/reviewer/agent.yaml # Sub-Agent als eigenes Verzeichnis
  policies/strict.yaml       # Policy-Dateien (Format: POL-001)
```

### Vollständiges Beispiel `agent.yaml`

```yaml
# Schema: lokale Datei, die `beton agent new` unter .beton/schemas/v1/ ablegt (AGT-013), nie eine URL
# yaml-language-server: $schema=../../schemas/v1/agent.schema.json
spec_version: 1
name: pr-fixer                       # [a-z0-9-], eindeutig im Suchpfad
description: Behebt fehlschlagende CI-Checks auf einem Branch und öffnet einen PR.
version: 0.3.0                       # frei, SemVer empfohlen

executor:
  harness: claude                    # claude | codex | acp:<slug> | direct:<provider> | fake
  mode: native                       # native | tui
  model: claude-sonnet-4-5           # optional; sonst Harness-Default
  reasoning_effort: medium           # low | medium | high | xhigh (gegen Harness/Modell validiert)
  permission_mode: accept_edits      # plan | default | accept_edits | yolo (HAR-027)
  max_turns: 200
  timeout: 2h                        # Wanduhr pro Run
  mcp_bridge: true                   # System-Tools via MCP (HAR-009)

instructions:
  file: prompts/system.md            # relativ zum Agent-Verzeichnis; alternativ `text:`
  append: |
    Arbeite ausschließlich im zugewiesenen Worktree. Öffne am Ende einen PR, merge nie.
  project_files: auto                # auto | none | [AGENTS.md, CLAUDE.md]

params:                              # typisierte Eingaben, nutzbar als {{ params.x }}
  branch:      { type: string, default: main }
  max_attempts: { type: integer, default: 3, minimum: 1, maximum: 10 }

tools:
  mcp:
    github:
      command: github-mcp-server
      args: [stdio]
      env: { GITHUB_PERSONAL_ACCESS_TOKEN: "${secret:github}" }   # → bt_cred_*-Platzhalter
      allow: [get_pull_request, create_pull_request, list_check_runs]
    docs:
      url: https://mcp.example.com/mcp
      headers: { Authorization: "Bearer ${secret:docs_token}" }
  system: [session_spawn, session_wait, session_send, inbox_read, policy_query, timer_set, ask_user]

skills: [fix-ci, ci-triage]          # all | none | Liste von Namen

agents:                              # Sub-Agents für session_spawn
  reviewer:
    ref: ./agents/reviewer
  quick-check:                       # inline definiert
    executor: { harness: codex, model: gpt-5-codex, reasoning_effort: low, permission_mode: plan }
    instructions: { text: "Reviewe nur den übergebenen Diff. Maximal 10 Punkte, nach Schwere sortiert." }

spawn:
  agents: [reviewer, quick-check]    # nur diese dürfen gestartet werden
  max_depth: 2
  max_concurrent: 3
  worktree: new                      # Default für Kinder: new | inherit | none
  budget_share: 0.5                  # max. Anteil am Run-Budget je Kind

timers: true

schedules:
  - id: nightly-ci
    cron: "0 3 * * 1-5"
    timezone: Europe/Berlin
    prompt: "Prüfe die CI von {{ params.branch }} und behebe Fehler."
    params: { branch: develop }
    catch_up: run_once               # run_once | skip
    overlap: skip                    # skip | queue
    keep_awake: during_run           # false | during_run | always
    runner: { selector: "os=linux,repo=beton" }      # nur zentral relevant (RUN-016)
    budget: { max_cost_usd: 5, max_duration: 1h, max_turns: 150 }

async:
  on_result: wake                    # wake | inbox_only  (Zustellung an Parent)
  approval: { timeout: 4h, on_timeout: deny }   # deny | allow | abort
  notify: [push, inbox]

policies:
  - ref: ./policies/strict.yaml
  - id: no-force-push
    type: git_guard
    params: { force_push: deny, push: ask, protected_branches: [main] }
  - id: spend
    type: spend_cap
    params: { scope: run, limit_usd: 5, ask_at_usd: [3] }

sandbox:                             # Felder und Semantik: SBX (04-sandbox.md)
  preset: default
  workspace: rw
  allow_network: true
  egress_rules: ["GET,POST api.github.com/**", "GET registry.npmjs.org/**"]
```

### System-Tools (MCP-Server `beton`)

| Tool | Zweck | Freischaltung |
|---|---|---|
| `session_spawn` | Child-Session für einen in `spawn.agents` erlaubten Agent starten (`agent`, `prompt`, `async`, `worktree`, `budget_usd`, `params`) → `session_id` | `spawn.agents` nicht leer |
| `session_send` | Nachricht an eine eigene Child-Session senden | mit `session_spawn` (mit AGT-009) |
| `session_wait` | Auf Ergebnis einer oder mehrerer Childs warten (`ids`, `mode: all\|any`, `timeout`) | mit `session_spawn` (mit AGT-009) |
| `session_status` / `session_list` / `session_cancel` | Status, Liste, Abbruch von Childs | mit `session_spawn` (mit AGT-009) |
| `inbox_read` | Ergebnisse/Nachrichten an diese Session lesen (Agent-Inbox) | immer (ab M2, UX-001) |
| `ask_user` | Frage an den Menschen (erzeugt Inbox-Frage; blockiert bis Antwort/Timeout) | immer (ab M2, UX-001) |
| `policy_query` | "Wäre Aktion X erlaubt?" → Entscheidung + Begründung (Explain, POL-027) | immer |
| `timer_set` / `timer_cancel` / `timer_list` | Einmalige Wiederaufnahme dieser Session | `timers: true` (ab M5, ASY-003) |
| `schedule_create` / `schedule_list` / `schedule_update` / `schedule_delete` | Schedules für erlaubte Agents verwalten | Agent-Feld `tools.system` enthält sie (ab M5, ASY-011) |
| `skill_load` / `skill_read_file` | Skill-Inhalt bzw. Zusatzdatei laden (für Harnesses ohne native Skills) | wenn Skills aktiv |

Alle System-Tool-Aufrufe durchlaufen die Policy-Phase `tool_call` mit `tool.kind = "system"` (ab M2; im Event-Log `tool.call.requested` mit `mcp_server: beton`, `source: beton_mcp`). `tools.system` wählt aus (leer bzw. fehlend: alle verfügbaren), die Spalte „Freischaltung“ begrenzt; `skill_load`/`skill_read_file` kommen mit aktiven Skills immer hinzu. Tools, deren Freischaltung fehlt oder die erst ein späterer Meilenstein bringt, sind unsichtbar, ein direkter Aufruf endet mit `tool_not_enabled`.

## Features — Agents (AGT)

### AGT-001 — Agent-Format v1 (YAML)
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Agents werden in genau einer kanonischen YAML-Schreibweise beschrieben (Felder siehe Beispiel). Top-Level-Felder: `spec_version`, `name`, `description`, `version`, `executor`, `instructions`, `params`, `tools`, `skills`, `agents`, `spawn`, `timers`, `schedules`, `async`, `policies`, `sandbox`. Unbekannte Felder sind Fehler (keine stillen Tippfehler); `x-`-präfixierte Felder sind für Erweiterungen erlaubt.
- **Details:** Template-Ausdrücke `{{ params.x }}`, `{{ trigger.payload.x }}`, `{{ now }}` nur in `prompt`/`instructions.append`/Schedule-Feldern (Minimal-Templating ohne Logik). Secret-Referenzen `${secret:<name>}` werden zur Laufzeit in `bt_cred_*`-Platzhalter aufgelöst (SEC-001/PRX-006, ab M2), nie in Klartext. Die Felder `timers`, `schedules` und `async` werden ab M5 ausgewertet (ASY-001 ff.). `sandbox` ist kanonisch; `os_env` wird nicht als Alias akzeptiert *(Annahme)*. `x-`-Felder sind nur auf oberster Ebene erlaubt. Felder späterer Meilensteine (`policies`, `sandbox` ab M2; `timers`, `schedules`, `async` ab M5) liest und prüft v1 bereits, wertet sie aber nicht aus; bei Inline-Policy-Regeln prüft v1 nur `ref`/`id`, die Regelfelder prüft ab M2 das Policy-Schema (POL-001). Der `sandbox`-Block hat die Felder der Sandbox-Konfiguration (SBX, 04-sandbox.md). Parameter vom Typ `enum` listen ihre Werte unter `values`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Das Beispiel-`agent.yaml` dieses Kapitels wird fehlerfrei geladen (Fixture-Test).
  - [ ] AC2 — Ein unbekanntes Feld `instruction:` (Tippfehler) führt zu Fehler mit Datei, Zeile, Spalte und Vorschlag `instructions`.
  - [ ] AC3 — (ab M2) `${secret:github}` erscheint nach Auflösung nirgends im Klartext; der Harness erhält einen `bt_cred_*`-Wert.
  - [ ] AC4 — `spec_version: 2` wird mit klarer Meldung "nicht unterstützte Spec-Version" abgelehnt.
- **Abhängigkeiten:** SEC-001 (ab M2, siehe 05-security-identity.md), PRX-006 (ab M2, siehe 04-sandbox.md)
- **Referenz:** ADR-0012; Omnigent `AGENT_YAML_SPEC.md`

### AGT-002 — JSON-Schema-Veröffentlichung & Validierung
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Das JSON-Schema wird aus den Rust-Typen generiert (`schemars`), als `schemas/v1/agent.schema.json` im Repo versioniert (`cargo xtask codegen`, wie alle Schemas unter `schemas/v1/`) und mit jedem Release veröffentlicht. `beton agent validate <pfad>` prüft Schema **und** semantische Regeln (Harness existiert, Effort zum Modell passend, referenzierte Dateien/Skills/Sub-Agents vorhanden, keine Zyklen in `agents`).
- **Details:** Befunde haben die Form `{ file, path, line, column, code, message, severity }` (`severity`: `error` oder `warning`). Warnungen lassen den Exit-Code bei 0, z. B. `harness_unavailable` (gültige Harness-ID, in dieser Installation aber nicht registriert) und `effort_mapped`. Weitere Codes: `unknown_field`, `missing_field`, `invalid_value`, `unknown_harness`, `unsupported_spec_version`, `file_not_found`, `path_outside_agent`, `skill_not_found`, `unknown_subagent`, `agent_cycle`, `agent_not_found`, `conflicting_fields`, `invalid_param`, `duplicate_key`, `yaml_syntax`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Snapshot-Test: generiertes Schema == eingechecktes `schemas/v1/agent.schema.json`; Abweichung lässt CI fehlschlagen.
  - [ ] AC2 — `beton agent validate` liefert Exit-Code 0/1 und mit `--json` eine Liste von `{ path, line, column, code, message }`.
  - [ ] AC3 — Ein Sub-Agent-Zyklus (A → B → A) wird als Fehler `agent_cycle` erkannt.
  - [ ] AC4 — `reasoning_effort: xhigh` auf einem Harness/Modell ohne diese Stufe erzeugt eine Warnung mit dem effektiven Wert (Mapping wie HAR-017).
- **Abhängigkeiten:** AGT-001, QA-006 (siehe 12-distribution-quality.md)

### AGT-003 — Agent-Verzeichnis & Auflösung
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Agents werden über einen Suchpfad gefunden: Projekt `.beton/agents/<name>/` → User `~/.beton/agents/<name>/` → Built-ins (in das Binary eingebettet, Quelle `agents/` im Repo). Der erste Treffer gewinnt; Verschattung eines Built-ins erzeugt eine Warnung. Pfade (`./x`, `/abs/x`) umgehen den Suchpfad; `builtin:<name>` erzwingt das Built-in.
- **Details:** `beton run <agent-ref> [-p "prompt"] [--param k=v]`; den Start liefert AGT-004 (CLI-002), die Auflösung samt Verschattungswarnung dieses Feature. Das Projekt ist dasselbe wie für `.beton/config.yaml` (CLI-008). Zentral (M4) können Agents zusätzlich serverseitig registriert werden; der Runner erhält dann das Agent-Bundle als Blob (tar.zst) mit Hash *(Annahme)*.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Existiert `maestra` im Projekt und als Built-in, startet `beton run maestra` die Projektversion und gibt eine Verschattungswarnung aus; `beton run builtin:maestra` startet das Built-in.
  - [ ] AC2 — `beton agent list` zeigt Name, Quelle (`project|user|builtin|server`), Version, Harness und Pfad.
  - [ ] AC3 — Ein Agent-Verzeichnis ohne `agent.yaml` wird übersprungen und in `beton agent list --all` als ungültig angezeigt.
- **Abhängigkeiten:** AGT-001

### AGT-004 — Executor & Agent-Snapshot pro Session
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** `executor` legt Harness, Modus, Modell, Effort, Permission-Mode und Limits fest. Beim Start wird der Agent vollständig aufgelöst (inkl. Prompts, Skills, Sub-Agent-Definitionen, Policies) und als Snapshot mit Inhalts-Hash in den Blob-Store geschrieben; das Event `agent.resolved { name, version, source, hash, blob_ref }` macht Sessions reproduzierbar und Forks konsistent.
- **Details:** CLI-Overrides (`--model`, `--harness`, `--effort`) werden im Snapshot als `overrides` vermerkt. Ein Harness-Override auf einen Harness ohne benötigte Capabilities (z. B. Sub-Agents ohne MCP-Injection) wird beim Start abgelehnt.
  Umsetzung (M1): Den Start löst der Server auf (`POST /v1/sessions {agent, target?, model?, params?, cwd}`; `target` ist dann der Harness-Override): Suchpfad wie AGT-003 (Projekt wie CLI-008), Validierung wie `beton agent validate` (Fehler → `422 agent_invalid`), Executor (`harness`, `model`; Overrides gewinnen), Parameter (AGT-010), Capability-Prüfung (`422 harness_incompatible`), dann Snapshot und Session. Der Snapshot ist ein JSON-Blob im Blob-Store der Session (DATA-006) mit allen Dateien des Agent-Verzeichnisses, Sub-Agents außerhalb des Verzeichnisses (`builtin:<name>`, `../x`) samt Verweis, aufgelösten Parametern und Overrides; `hash` ist für einen Agent ohne äußere Sub-Agents genau der Hash von `beton agent show`, sonst um die äußeren Bäume erweitert; Parameter und Overrides gehen nicht in den Hash ein. Eine Verschattungswarnung (AGT-003) steht als `notice` im Log. Runner, Resume und Forks lesen den Agent nur aus dem Snapshot (Fork: derselbe Blob und dasselbe `agent.resolved` in der neuen Session); Sessions ohne `agent.resolved` laden ihn wie bisher über `agent_ref`. Aus `executor` wirken in M1 `harness`, `model`, `max_turns` (HAR-010) und `timeout`; `reasoning_effort` und `permission_mode` kommen mit HAR-017/HAR-027 (WP-28). `timeout` ist die Wanduhr ab dem ersten Turn eines Runs (Runner-Prozess): Danach unterbricht der Runner den laufenden Turn (`turn.interrupted {reason: timed_out}`), setzt die Session auf `session.status {status: stopped, reason: timed_out}` und endet; reagiert der Harness binnen 10 s nicht, endet der Run trotzdem. `beton run -p` meldet dann `status: timed_out` (Exit-Code 1). Ein neuer Input setzt die Session fort und startet einen neuen Run. Vendor-eigene MCP-Konfigurationen (z. B. `~/.claude.json`) schließen Agent-Läufe in M1 nicht aus *(Entscheidung zu HAR-009: Codex und ACP-Agents bieten keinen Ausschluss; ein nur für Claude geltendes `--strict-mcp-config` machte das Verhalten eines Agents harness-abhängig; vollständig isoliert läuft Claude mit `harnesses.claude.isolated: true`)*.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Zwei Starts mit unveränderten Dateien erzeugen denselben `hash`; Änderung eines Prompts ändert ihn.
  - [ ] AC2 — Ein Fork der Session nutzt den Snapshot, auch wenn die Agent-Dateien inzwischen geändert wurden.
  - [ ] AC3 — `timeout` wird durchgesetzt: nach Ablauf wird der Turn unterbrochen und der Run endet mit Status `timed_out`.
- **Abhängigkeiten:** AGT-001, HAR-001 ([01](01-harnesses.md)), DATA-006, PROTO-002 (siehe 06-data-sync-protocol.md)

### AGT-005 — Instructions & Auslieferung
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Instructions kommen aus `instructions.text` oder `instructions.file` plus optionalem `append`. Die Auslieferung erfolgt je Harness-Capability `instructions_delivery`: Claude über `--append-system-prompt-file`, Codex über Developer-Instructions des Threads, Direkt-API als System-Message, ACP als Präfix der ersten Nachricht. `project_files: auto` sorgt dafür, dass `AGENTS.md` und `CLAUDE.md` des Projekts bei jedem Harness ankommen: Was der Harness nicht selbst liest, fügt beton an.
- **Details:** Reihenfolge: Text bzw. Datei, dann `append` (mit `{{ params.x }}`, AGT-010), dann die Projektdateien je als Block `--- Projektdatei <name> --- … --- Ende <name> ---` (höchstens 256 KiB je Datei, längere gekürzt mit `notice`). Default ist `project_files: auto` (`AGENTS.md`, `CLAUDE.md` im Arbeitsverzeichnis); eine Liste nennt Dateien relativ dazu (Pfade außerhalb und fehlende Dateien: `notice`), `none` liefert keine. Welche Dateien ein Harness selbst liest, deklariert die Capability `native_project_files`: Claude `CLAUDE.md` (nicht mit `isolated: true`, weil `--safe-mode` laut `claude --help` 2.1.285 auch `CLAUDE.md` abschaltet), Codex `AGENTS.md`, ACP-Agents und Direkt-API keine *(Annahme für ACP; Gemini CLI liest z. B. `GEMINI.md`)*. Der Runner setzt die Instructions beim Start zusammen (`SessionSpec.instructions`); eine fehlende `instructions.file` verhindert den Start (fail closed). Genau einmal: Claude erhält die Datei (0600, privates Verzeichnis) bei jedem Prozessstart (der System-Prompt gehört nicht zum Verlauf), Codex nur bei `thread/start` (ein fortgesetzter Thread hat sie im Verlauf), ACP und Fake-Harness nur bei einer neuen Session (nicht nach `session/load`), Direkt-API in jedem Request im System-Prompt. Im Event-Log steht die Nutzer-Nachricht ohne Präfix. Verifiziert ohne Modellaufruf: `--append-system-prompt-file` laut `claude --help` 2.1.285, `developerInstructions` in `ThreadStartParams` laut `codex app-server generate-json-schema` 0.153.2. Ein Child von `session_spawn` erhält die Instructions seines Sub-Agents (AGT-007).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Für jeden der vier Harness-Typen enthält der beim Fake-/Mock-Harness ankommende Kontext die Agent-Instructions genau einmal.
  - [ ] AC2 — Mit `project_files: auto` erhält Codex den Inhalt von `CLAUDE.md`, Claude den von `AGENTS.md`; Dateien, die der Harness selbst liest, werden nicht doppelt geliefert.
  - [ ] AC3 — Fehlt `instructions.file`, schlägt die Validierung mit Pfadangabe fehl.
- **Abhängigkeiten:** AGT-001, HAR-004, HAR-006, HAR-007, HAR-010 ([01](01-harnesses.md))

### AGT-006 — Tools = MCP-Server (Agent/Projekt/User)
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Tools werden ausschließlich als MCP-Server deklariert: stdio (`command`, `args`, `env`) oder HTTP (`url`, `headers`), jeweils mit optionaler `allow`-Liste von Tool-Namen. Neben dem Agent können User (`~/.beton/mcp.yaml`) und Projekt (`.beton/mcp.yaml`) MCP-Server definieren. Es gibt keine Funktions-Tools in anderen Sprachen.
- **Details:** Format der User- und Projekt-Datei (`~/.beton/mcp.yaml` bzw. `.beton/mcp.yaml`, Schema `schemas/v1/mcp.schema.json`): `servers: { <name>: { command, args, env, url, headers, allow } }` mit denselben Feldern wie `tools.mcp` im Agent; Namen `[a-z0-9_-]` (höchstens 64 Zeichen), `beton` ist für die System-Tools reserviert. Genau eines von `command` (stdio) und `url` (HTTP, `http://` oder `https://`) ist Pflicht; eine fehlerhafte Datei wird mit Hinweis (`notice`) ignoriert, ein ungültiger Eintrag meldet `mcp.server_failed`. HTTP-Server verbindet der Runner nur, wenn sie konfiguriert sind (ADR-0033). `${secret:…}` in `env`/`headers` wird erst ab M2 aufgelöst; bis dahin startet ein solcher Server nicht (`mcp.server_failed`, fail closed). `allow` filtert `tools/list` im Relay-Hub (HAR-009) und lehnt `tools/call` anderer Tools mit `tool_not_enabled` ab. Merge-Reihenfolge User → Projekt → Agent; gleicher Name: die spezifischere Ebene ersetzt den Eintrag vollständig. Agent-Läufe (Agent-Ref angegeben) verwenden User-/Projekt-Server nur, wenn `tools.inherit: true` *(Annahme: Default `false` für Reproduzierbarkeit; interaktive Sessions ohne Agent nutzen User+Projekt)*. Die Sandbox-Stufe von stdio-MCP-Servern regelt SBX-002. UI und CLI zur MCP-Server-Verwaltung: UX-010 (siehe 11-platform-features.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein in `allow` nicht gelistetes Tool eines MCP-Servers ist für das Modell nicht sichtbar (Tool-Liste im `harness.ready`-Event geprüft).
  - [ ] AC2 — Ein Projekt-Server mit gleichem Namen wie ein User-Server ersetzt diesen vollständig (Unit-Test des Merges).
  - [ ] AC3 — (ab M2) `${secret:x}` in `env`/`headers` erreicht den MCP-Server nur als Platzhalter; der Proxy setzt den echten Wert ein.
  - [ ] AC4 — Ein nicht startbarer MCP-Server erzeugt `mcp.server_failed`, die Session läuft ohne ihn weiter.
- **Abhängigkeiten:** AGT-001, HAR-009 ([01](01-harnesses.md)), SBX-002 (ab M2), PRX-006 (ab M2, siehe 04-sandbox.md)

### AGT-007 — System-Tools via MCP-Server `beton`
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** `beton-mcp` implementiert die System-Tools (Tabelle im Design) als MCP-Server, den HAR-009 in jeden Harness injiziert. Welche Tools sichtbar sind, steuern `tools.system` und die Freischaltungsregeln; Sessions ohne Agent erhalten `policy_query`, `inbox_read`, `ask_user`, `skill_load`, `skill_read_file`.
- **Details:** Jeder Aufruf trägt das session-gebundene Relay-Token; Aufrufe sind als `tool.call.*` mit `mcp_server: beton` und `source: beton_mcp` im Log (Policy-Klasse `tool.kind = "system"`, ab M2). `session_wait` blockiert höchstens `timeout` (Default 30 min, max. 24 h) und liefert dann den Zwischenstand. Verfügbar in M1: `policy_query`, `session_spawn` (nur `async: false`), `skill_load`, `skill_read_file`; `inbox_read` und `ask_user` kommen mit der Inbox (UX-001, ab M2), `session_send`/`session_wait`/`session_status`/`session_list`/`session_cancel` mit AGT-009, `timer_*` und `schedule_*` ab M5 (ASY-003, ASY-011). Sessions ohne Agent erhalten daher in M1 `policy_query` und, wenn Skills gefunden werden, `skill_load`/`skill_read_file`. `policy_query` antwortet ohne Policy-Engine (bis POL-027, M2) nie mit `allow`, sondern mit `decision: ask` und Begründung. `session_spawn` startet den Sub-Agent mit dem Harness und Modell aus seinem `executor` (das Modell wählt nur den Namen aus `spawn.agents`), als Session `kind: subagent`, `trigger: spawn` mit `parent_session_id` im Arbeitsverzeichnis des Parents; der Server führt das über `system.call` im Tunnel aus (PROTO-015) und wartet höchstens 30 min auf das Turn-Ende. Seit AGT-004/AGT-005 erhält das Child einen eigenen Snapshot (aus dem Snapshot des Parents, `agent.resolved` mit `source: subagent`), seine Instructions, Tools und Skills sowie die Parameter aus `session_spawn(params)` (AGT-010). Grenzen (`max_depth`, `max_concurrent`, Worktree, Budget) sowie `agent.spawned`/`agent.completed` folgen mit AGT-009; bis dahin bekommt ein Child selbst keine Sub-Agents.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ohne `spawn.agents` ist `session_spawn` nicht in der Tool-Liste; ein direkter Aufruf wird mit `tool_not_enabled` abgelehnt.
  - [ ] AC2 — `session_spawn(agent: "quick-check", prompt: "…", async: false)` startet eine Child-Session auf Codex und liefert deren Abschlussnachricht als Tool-Result (Integrationstest mit Fake-Harnesses).
  - [ ] AC3 — (ab M2) `policy_query` liefert für eine geplante Aktion dieselbe Entscheidung, die die Engine bei tatsächlicher Ausführung träfe (Testfall mit `git push`).
  - [ ] AC4 — (ab M2) `ask_user` erzeugt eine Inbox-Frage; die Antwort eines Users erscheint als Tool-Result.
- **Abhängigkeiten:** AGT-006, HAR-009 ([01](01-harnesses.md)), POL-027 (ab M2, [03](03-policies.md)), UX-001 (ab M2, siehe 11-platform-features.md)

### AGT-008 — Skills (`SKILL.md`)
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Skills sind Ordner mit `SKILL.md` (Frontmatter `name`, `description`, optional `user-invocable`, `disable-model-invocation`, `allowed-tools`) und optionalen `scripts/`, `references/`, `assets/`. Discovery-Reihenfolge (erster Treffer pro Name gewinnt): Agent-Verzeichnis `skills/` → Projekt `.beton/skills/`, `.claude/skills/`, `.agents/skills/` → User `~/.beton/skills/`, `~/.claude/skills/`, `~/.agents/skills/` → Built-in-Skills. `skills:` im Agent filtert (`all`, `none`, Liste).
- **Details:** Auslieferung: Harnesses mit nativer Skill-Unterstützung erhalten die ausgewählten Skills in einem session-spezifischen Skill-Verzeichnis (Kopie, Symlinks nach außen ausgelassen), das beton dem Harness bekannt macht – verifiziert ohne Modellaufruf: Claude Code 2.1.285 lädt es als Plugin `beton` über `--plugin-dir` (Skills erscheinen als `beton:<name>` mit Alias `<name>`), Codex 0.153.2 über den Request `skills/extraRoots/set` mit `<dir>/skills`. Skills aus dem Verzeichnis, das die CLI selbst liest (Claude: `.claude/skills`, Codex: `.agents/skills`), liefert beton dort nicht noch einmal aus. Alle Harnesses können Inhalte über `skill_load`/`skill_read_file` laden; der Skill-Index (Name + Beschreibung) steht in der Beschreibung von `skill_load` und damit harness-unabhängig im Modell-Kontext *(Entscheidung mit AGT-005: nicht zusätzlich in den Instructions, damit der Index genau einmal im Kontext steht)*. Skills mit `disable-model-invocation: true` erscheinen nicht im Index und lassen sich nicht über `skill_load` laden. `skill_read_file` liefert nur Textdateien bis 256 KiB innerhalb des Skill-Ordners. Ohne Agent gelten alle gefundenen Skills (`skills: all`). Im Composer sind user-invocable Skills über `/name` aufrufbar (WEB-006).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Skill gleichen Namens im Agent-Verzeichnis verschattet den Projekt-Skill (Test der Discovery-Reihenfolge).
  - [ ] AC2 — Auf einem Harness ohne native Skills (ACP; Direkt-API mit HAR-010) ruft das Modell `skill_load("fix-ci")` auf und erhält den Body ohne Frontmatter.
  - [ ] AC3 — `skill_read_file` verweigert Pfade außerhalb des Skill-Ordners (`../`, Symlinks nach außen).
  - [ ] AC4 — Eine `SKILL.md` ohne `name`/`description` wird mit Warnung übersprungen.
- **Abhängigkeiten:** AGT-007

### AGT-009 — Sub-Agents über Harness-Grenzen
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Ein Agent kann über `session_spawn` Child-Sessions für in `agents`/`spawn.agents` deklarierte Sub-Agents starten, die auf beliebigen Harnesses laufen (Claude implementiert, Codex reviewt). Childs sind eigenständige Sessions mit `parent_session_id`, eigenem Event-Log, eigener Policy-Auswertung (Agent-Ebene des Childs) und optional eigenem Worktree.
- **Details:** `worktree: new` legt einen neuen Worktree vom aktuellen HEAD des Parents an (SES-015, siehe 07-sessions-collaboration.md), `inherit` teilt den Worktree (nur sequentiell sinnvoll), `none` = read-only Sicht auf den Parent-Worktree. Events im Parent: `agent.spawned { child_session_id, agent_ref, harness, async }`, `agent.completed { child_session_id, status, summary, cost_micro }` (PROTO-002). Kosten der Childs zählen zum Run-Budget des Parents (Subtree, ASY-010). Die UI zeigt den Session-Baum (xyflow, WEB-012 in 08-clients.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — (ab M3) Ein Claude-Parent startet einen Codex-Child mit `worktree: new`; beide arbeiten in unterschiedlichen Verzeichnissen, der Child-Branch basiert auf dem Parent-HEAD.
  - [ ] AC2 — `max_depth: 2` verhindert, dass ein Enkel einen weiteren Urenkel startet (`spawn_denied: max_depth`).
  - [ ] AC3 — `max_concurrent: 3`: der vierte gleichzeitige Spawn wird mit `spawn_denied: max_concurrent` abgelehnt.
  - [ ] AC4 — `agent.completed` erscheint im Parent-Log mit Status und Kosten; die Summe der Child-Kosten ist in `run.cost_usd` des Parents enthalten.
  - [ ] AC5 — Wird der Parent abgebrochen, werden laufende Childs mit `cancel_reason: parent_cancelled` beendet.
- **Abhängigkeiten:** AGT-007, HAR-009 ([01](01-harnesses.md)), SES-006, SES-015 (siehe 07-sessions-collaboration.md), WEB-012 (siehe 08-clients.md)
- **Referenz:** Omnigent Polly (Worktree pro Sub-Agent)

### AGT-010 — Parameter (`params`)
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Agents deklarieren typisierte Parameter (`string`, `integer`, `number`, `boolean`, `enum`, jeweils mit `default`, `minimum`/`maximum`, `required`). Werte kommen aus CLI (`--param k=v`), UI-Formular, Schedule-`params`, `session_spawn(params)` oder Webhook-Payload-Mapping und sind in Templates als `{{ params.x }}` verfügbar.
- **Details:** Der Server löst die Werte vor dem Start auf (`POST /v1/sessions {agent, params}`; `session_spawn` mit `params` gegen die Deklaration des Sub-Agents): Texte (CLI, Formular) werden in den deklarierten Typ umgewandelt, Defaults eingesetzt, unbekannte Namen und verletzte Typen, `minimum`/`maximum` oder `values` ergeben `422 invalid_param`, fehlende Pflichtwerte ohne Default `422 params_required`; beide nennen jeden Parameter in `errors[]` mit `pointer: /params/<name>` (Grundlage des UI-Formulars). Ein nicht gesetzter optionaler Parameter ohne Default ergibt im Template einen leeren Text. `beton agent validate` meldet Template-Ausdrücke in `instructions.text` (`invalid_value`) sowie nicht deklarierte Parameter (`invalid_param`) und unbekannte Ausdrücke (`invalid_value`) in `instructions.append`. Schedule- und Webhook-Quellen folgen mit ASY (M5); das UI-Formular (Screen `agent-start`) folgt mit #125.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `--param max_attempts=20` bei `maximum: 10` wird vor dem Start abgelehnt.
  - [ ] AC2 — Fehlt ein `required`-Parameter ohne Default, fragt die UI per Formular nach; die CLI bricht mit Fehlermeldung ab.
  - [ ] AC3 — Aufgelöste Parameterwerte stehen im `agent.resolved`-Event.
- **Abhängigkeiten:** AGT-001, AGT-004

### AGT-011 — Built-in-Agent `maestra` (Orchestrator)
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Mitgelieferter Orchestrator als reines YAML (`agents/maestra/`), Name orchester-thematisch *(Annahme)*. maestra schreibt selbst keinen Code: Sie zerlegt ein Ziel in Teilaufgaben (`/plan`), delegiert Implementierung an Sub-Agents mit eigenem Worktree (`/fanout`), lässt jedes Ergebnis von einem Agent **eines anderen Vendors** reviewen (`/cross-review`, max. 3 Runden) und fasst zusammen. Implementer committen auf eigene Branches; maestra merged nie.
- **Details:**
  ```yaml
  name: maestra
  executor: { harness: claude, permission_mode: plan }
  agents:
    impl-claude:   { ref: ./agents/impl-claude }     # claude, accept_edits, worktree new
    impl-codex:    { ref: ./agents/impl-codex }      # codex, accept_edits, worktree new
    review-claude: { ref: ./agents/review-claude }   # claude, plan (read-only)
    review-codex:  { ref: ./agents/review-codex }    # codex, plan (read-only)
  spawn: { agents: [impl-claude, impl-codex, review-claude, review-codex], max_depth: 1, max_concurrent: 4, worktree: new }
  skills: [plan, fanout, cross-review, investigate]
  policies:
    - { id: maestra-no-code, type: tool_allowlist, params: { allow_kinds: [system, file_read, search] } }
    - { id: maestra-no-merge, type: git_guard, params: { merge: deny, push: deny } }
  ```
  Preflight: Fehlt ein Harness (z. B. kein Codex-Login), weicht maestra auf den verfügbaren Vendor aus und kennzeichnet das Review als "same-vendor". maestra und alle Sub-Agents laufen auf den eingeloggten Vendor-CLIs (Subscription, HAR-015); kein Teil setzt einen API-Key oder den Direkt-API-Harness voraus (ADR-0034).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton run maestra -p "…"` mit Fake-Harnesses durchläuft Plan → 2 Implementierungen (parallel, jede mit eigenem Worktree, SES-015) → Cross-Review → Zusammenfassung; das Log zeigt, dass jedes Review von einem anderen Harness als die Implementierung kam.
  - [ ] AC2 — (ab M2) Ein Versuch von maestra, selbst `Bash` oder `Edit` aufzurufen, wird per Policy abgelehnt.
  - [ ] AC3 — Meldet ein Review blockierende Punkte, wird der Implementer erneut beauftragt; nach 3 Runden endet die Aufgabe mit Status "needs_human".
  - [ ] AC4 — Fehlt Codex, enthält die Zusammenfassung den Hinweis "same-vendor review".
  - [ ] AC5 — Der Lauf aus AC1 gelingt in einer Umgebung ohne `*_API_KEY`-Variablen und ohne `providers`-Konfiguration; alle Child-Sessions melden `auth_source: vendor_cli`.
- **Abhängigkeiten:** AGT-009, AGT-008, POL-015 (ab M2), POL-017 (ab M2, siehe [03](03-policies.md))
- **Referenz:** ADR-0012; Omnigent Polly

### AGT-012 — Built-in-Agent `duetto` (Debatte)
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Mitgelieferter Debatten-Agent (`agents/duetto/`), Name orchester-thematisch *(Annahme)*. Jede Frage geht parallel an zwei Stimmen auf verschiedenen Harnesses (`voce-claude`, `voce-codex`, beide read-only); die Antworten werden nebeneinander dargestellt. `/debate rounds=N` (Default 2, max. 5) lässt die Stimmen die Antwort der jeweils anderen kritisieren; danach synthetisiert duetto ein Ergebnis mit markierten Konsens- und Dissenspunkten. Stimmen und Synthese laufen auf den eingeloggten Vendor-CLIs (Subscription, HAR-015); ein API-Key ist nie erforderlich (ADR-0034).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Frage erzeugt zwei Child-Sessions auf unterschiedlichen Harnesses, die parallel laufen (`session_wait(mode: all)`).
  - [ ] AC2 — `/debate rounds=2` erzeugt genau 2 Kritikrunden pro Stimme und eine Synthese mit Abschnitten "Konsens" und "Dissens".
  - [ ] AC3 — Fehlt eine Stimme (Harness nicht eingerichtet), bricht duetto mit klarer Meldung ab, statt mit einer Stimme zu "debattieren".
  - [ ] AC4 — Debatte und Synthese gelingen ohne `*_API_KEY`-Variablen und ohne `providers`-Konfiguration; alle beteiligten Sessions melden `auth_source: vendor_cli`.
- **Abhängigkeiten:** AGT-009, AGT-008
- **Referenz:** Omnigent Debby

### AGT-013 — Agent-CLI
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** `beton agent list | show <ref> | validate <pfad> | new <name> [--from builtin:maestra]`. `show` gibt den aufgelösten Agent (inkl. Herkunft jedes Felds) aus, `new` erzeugt ein Gerüst mit `$schema`-Kommentar (verweist auf die von beton lokal abgelegte Schema-Datei, nicht auf eine URL, ADR-0033), Prompt-Datei und Beispiel-Skill. AGT-013 ist Owner der Semantik; CLI-009 regelt nur die Konsistenz der CLI-Oberfläche.
- **Details:** `new` legt das Gerüst unter `<projekt>/.beton/agents/<name>/` an und schreibt das aktuelle Schema nach `<projekt>/.beton/schemas/v1/agent.schema.json`; `agent.yaml` verweist relativ darauf (`# yaml-language-server: $schema=../../schemas/v1/agent.schema.json`). `--from <ref>` kopiert einen vorhandenen Agent und setzt `name`. `show --json` liefert `{ name, version, source, path, hash, spec, origins, warnings }`: `origins` nennt je Feld `agent.yaml:<zeile>` oder `default`, `hash` ist `sha256` über Pfade und Inhalte aller Dateien des Agent-Verzeichnisses. `list --all` zeigt zusätzlich verschattete und ungültige Einträge. `schema` gibt das JSON-Schema auf stdout aus.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton agent new demo` erzeugt `.beton/agents/demo/` mit einer Struktur, die `beton agent validate` sofort besteht.
  - [ ] AC2 — `beton agent show maestra --json` liefert den Resolved Agent inkl. `hash`.
- **Abhängigkeiten:** AGT-002, AGT-003, CLI-001, CLI-009 (siehe 08-clients.md)

### AGT-014 — Agent-Policies & Sandbox-Block
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** `policies` im Agent bildet die Agent-Ebene der Policy-Hierarchie (POL-007): Einträge sind Policy-Datei-Referenzen (`ref`) oder Inline-Regeln (Format POL-001). `sandbox` beschreibt die Sandbox-Anforderungen des Agents (Semantik in SBX-003, SBX-006); beide können nur verschärfen, was höhere Ebenen vorgeben.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Inline-Regel `git_guard` im Agent erscheint im effektiven Policy-Set mit `scope: agent` (`beton policy explain`).
  - [ ] AC2 — Eine Agent-Regel `allow` kann ein `deny` der Projekt-Ebene nicht aufheben (Policy-Test).
  - [ ] AC3 — Ein Agent mit `sandbox`-Anforderung startet nicht, wenn die geforderte Sandbox auf dem Host nicht verfügbar ist (Fehler `sandbox_unavailable`).
- **Abhängigkeiten:** AGT-001, POL-001, POL-007 ([03](03-policies.md)), SBX-003, SBX-006 (siehe 04-sandbox.md)

## Features — Async-Agents (ASY)

### ASY-001 — Async-Session & Run-Modell
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Eine Async-Session läuft ohne angeschlossenen Client; Clients können jederzeit zuschauen oder übernehmen. Jede Ausführung ist ein Run mit `run_id`, `trigger { kind: spawn|timer|schedule|webhook, ref }`, Status und Budget.
- **Details:** Statusmaschine: `queued → dispatched → running ⇄ paused_approval → completed | failed | aborted | timed_out | budget_exceeded`. Events: `async.run.queued`, `async.run.started`, `async.run.paused { reason }`, `async.run.resumed`, `async.run.finished { status, cost_micro, duration_s, summary }` (PROTO-002). Die Zusammenfassung ist die letzte Assistant-Nachricht (max. 4 KiB).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein per API gestarteter Run läuft vollständig, ohne dass je ein Client verbunden war; das Log enthält alle Statusübergänge in gültiger Reihenfolge.
  - [ ] AC2 — Ein Client, der sich während `running` verbindet, erhält den Stream ab `seq` 0 (Resume) und sieht live weiter.
  - [ ] AC3 — Ungültige Übergänge (z. B. `completed → running`) werden vom Store abgelehnt (Unit-Test der Statusmaschine).
- **Abhängigkeiten:** PROTO-002, DATA-002 (siehe 06-data-sync-protocol.md), SES-001 (siehe 07-sessions-collaboration.md)
- **Referenz:** ADR-0013

### ASY-002 — `spawn(async: true)` & Ergebnis-Zustellung
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** `session_spawn(async: true)` kehrt sofort mit `session_id` zurück. Bei Abschluss erhält der Parent `agent.completed` und einen Inbox-Eintrag der Agent-Inbox (`inbox_read`). Mit `async.on_result: wake` (Default) startet ein idle Parent automatisch einen neuen Turn mit einer Systemnachricht ("Sub-Agent X abgeschlossen: …"); ist der Parent beschäftigt, wird die Nachricht nach dem aktuellen Turn zugestellt. `inbox_only` stellt nur in die Agent-Inbox.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein idle Parent mit `on_result: wake` beginnt innerhalb von 2 s nach Child-Abschluss einen neuen Turn, dessen Eingabe die Child-Zusammenfassung enthält.
  - [ ] AC2 — Mit `inbox_only` startet kein neuer Turn; `inbox_read` liefert das Ergebnis genau einmal (danach als gelesen markiert).
  - [ ] AC3 — Ergebnisse mehrerer gleichzeitig fertiger Childs werden in einer Wake-Nachricht gebündelt (Debounce 1 s).
- **Abhängigkeiten:** ASY-001, AGT-009

### ASY-003 — Timer
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** `timer_set(in: "2h" | at: "2026-10-04T09:00:00+02:00", prompt, id?)` plant eine einmalige Wiederaufnahme der eigenen Session; Timer sind auch per CLI/API setzbar. Timer sind persistiert und überleben Daemon-Neustarts; verpasste Timer feuern beim nächsten Start sofort (mit `late_by_s`).
- **Details:** Events `timer.set`, `timer.fired { late_by_s }`, `timer.cancelled`. Pro Session max. 20 aktive Timer, Mindestabstand 1 min *(Annahme)*.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit simulierter Uhr feuert ein `in: "2h"`-Timer exakt nach 2 h und startet einen Turn mit dem hinterlegten Prompt.
  - [ ] AC2 — Nach Daemon-Neustart während der Wartezeit feuert der Timer zum ursprünglichen Zeitpunkt; war der Zeitpunkt verpasst, feuert er sofort mit `late_by_s > 0`.
  - [ ] AC3 — Der 21. Timer einer Session wird mit `timer_limit_exceeded` abgelehnt.
- **Abhängigkeiten:** ASY-001, AGT-007

### ASY-004 — Schedules (Cron, Timezone, catch_up)
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Schedules starten wiederkehrend einen neuen Run eines Agents. Felder: `id`, `agent`, `cron` (5 Felder, Standard-Cron-Syntax), `timezone` (IANA, Default: Zeitzone des Users), `prompt`, `params`, `catch_up: run_once|skip`, `overlap: skip|queue`, `keep_awake`, `runner.selector` (RUN-016), `budget`, `state: active|paused`. Deklarierbar im Agent (`schedules:`) oder per CLI/API/System-Tool.
- **Details:** DST-Regeln: nicht existierende Ortszeit (Sprung vorwärts) → nächste gültige Minute; doppelte Ortszeit (Sprung zurück) → nur das erste Vorkommen. `catch_up` gilt für Ausfälle (Rechner aus/Schlaf): `run_once` startet genau einen Run mit `trigger.missed_count`, `skip` protokolliert `schedule.skipped { reason: missed, count }`. Toleranz `misfire_grace: 5m` – innerhalb gilt ein Run als pünktlich. `overlap: skip` (Default) überspringt, solange der vorige Run läuft (`schedule.skipped { reason: overlap }`). Events: `schedule.created|updated|deleted`, `schedule.fired { run_id, scheduled_for }`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `cron: "30 2 * * *"`, `timezone: Europe/Berlin` am Tag der Zeitumstellung im März feuert um 03:00 Ortszeit; im Oktober genau einmal um 02:30 (Test mit simulierter Uhr).
  - [ ] AC2 — Nach 3 verpassten Fire-Zeitpunkten startet `catch_up: run_once` genau einen Run mit `missed_count: 3`; `skip` startet keinen und schreibt `schedule.skipped`.
  - [ ] AC3 — Mit `overlap: skip` und laufendem Vor-Run wird der nächste Fire-Zeitpunkt übersprungen und protokolliert.
  - [ ] AC4 — Ein im Agent deklarierter Schedule wird beim Laden des Agents angelegt bzw. aktualisiert (idempotent über `agent` + `id`); ein entfernter Eintrag wird pausiert, nicht gelöscht.
- **Abhängigkeiten:** ASY-001, ASY-010
- **Referenz:** ADR-0013; Omnigent Scheduled Tasks

### ASY-005 — Lokaler Scheduler & `keep_awake`
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Im Lokal-only-Modus führt der Daemon Schedules und Timer aus, solange der Rechner läuft. `keep_awake` verhindert Leerlauf-Ruhezustand: `during_run` während laufender Runs, `always` solange aktive Schedules existieren. Ein Aufwecken aus dem Ruhezustand erfolgt in v1 nicht; dafür gibt es `catch_up`.
- **Details:** macOS: `IOPMAssertionCreateWithName` (`PreventUserIdleSystemSleep`); Linux: `systemd-inhibit`-äquivalente Inhibitor-Locks über D-Bus (logind); Windows: `SetThreadExecutionState(ES_SYSTEM_REQUIRED | ES_CONTINUOUS)`. Nach Systemaufwachen wird die Schedule-Liste sofort neu ausgewertet.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Während eines Runs mit `keep_awake: during_run` existiert auf macOS eine Power-Assertion (`pmset -g assertions` zeigt sie), nach Run-Ende nicht mehr.
  - [ ] AC2 — Nach simuliertem Aufwachen (Uhr springt um 6 h) wertet der Scheduler innerhalb von 5 s `catch_up` aus.
  - [ ] AC3 — Ist der Daemon nicht gestartet, zeigt `beton schedule list` einen Hinweis, dass Schedules nur bei laufendem Daemon ausgeführt werden (inkl. Befehl zur Autostart-Einrichtung).
- **Abhängigkeiten:** ASY-004

### ASY-006 — Zentraler Dispatch an Runner mit Labels
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Im Team-Server-Modus dispatcht der Scheduler Runs an online Hosts/Runner, deren Labels zum Selektor passen (`runner.selector`, Grammatik RUN-016). Hosts melden Labels beim Verbindungsaufbau (`beton host --label os=linux --label repo=beton`) plus automatische Labels gemäß RUN-005 (`beton.os`, `beton.arch`, `beton.harness.<name>` für eingerichtete Harnesses).
- **Details:** Host-Auswahl und Warteschlange: Owner RUN-016 (dieses Feature beschreibt nur die Sicht der Async-Runs). Kein passender Host online: Run bleibt `queued` bis `dispatch_timeout` (Default 10 min, RUN-016), dann `failed { reason: no_runner }` + Inbox-Eintrag. Genau ein Scheduler-Leader bei mehreren Server-Instanzen (Postgres-Advisory-Lock). Docker/K8s-Provider siehe RUN-008, RUN-012 (10-runners-extensibility.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Run mit `selector: "os=linux"` wird nie an einen macOS-Host dispatcht.
  - [ ] AC2 — Ohne passenden Host endet der Run nach `dispatch_timeout` mit `no_runner`, und der Owner erhält einen Inbox-Eintrag.
  - [ ] AC3 — Mit zwei Server-Instanzen feuert jeder Schedule-Zeitpunkt genau einmal (Integrationstest mit Postgres).
- **Abhängigkeiten:** ASY-004, RUN-005, RUN-016 (siehe 10-runners-extensibility.md), DATA-004 (siehe 06-data-sync-protocol.md)

### ASY-007 — Webhook-Trigger über die API
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Externe Systeme starten Runs über die REST-API. Ein Trigger bindet Agent, Prompt-Template und Budget an einen Endpunkt; Aufrufe authentifizieren sich mit PAT/Service-Account-Token (Scope `webhooks:trigger`, AUTH-012) oder einem trigger-eigenen Token. Es gibt in v1 keinen eingebauten Empfänger für vendor-spezifische Webhook-Formate (z. B. GitHub-Events).
- **Details:**
  ```http
  POST /v1/triggers/{trigger_id}/fire
  Authorization: Bearer <token>
  Idempotency-Key: 3f1c…
  Content-Type: application/json

  { "payload": { "pr": 42, "repo": "beton" }, "params": { "branch": "fix/ci" } }
  → 202 { "run_id": "run_…", "session_id": "ses_…" }
  ```
  Trigger-Definition: `{ id, agent, prompt: "Reviewe PR {{ trigger.payload.pr }}", params_from_payload: { branch: "$.branch" }, budget, runner }`. Payload max. 256 KiB; nur in Templates nutzbar, nie als Instructions-Ersatz. Rate-Limit pro Trigger (Default 60/h). Gleicher `Idempotency-Key` innerhalb 24 h → selber Run. OpenAPI siehe API-001/API-002 (08-clients.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein `fire`-Aufruf mit gültigem Token liefert 202 und startet einen Run, dessen Prompt die Payload-Werte enthält.
  - [ ] AC2 — Wiederholter Aufruf mit gleichem `Idempotency-Key` liefert dieselbe `run_id` und startet keinen zweiten Run.
  - [ ] AC3 — Token ohne Scope `webhooks:trigger` → 403; Payload > 256 KiB → 413; 61. Aufruf pro Stunde → 429.
- **Abhängigkeiten:** ASY-001, API-002 (siehe 08-clients.md), AUTH-012, AUTH-013 (siehe 05-security-identity.md)

### ASY-008 — Approval ohne Zuschauer
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Trifft eine Policy `ask` in einer Session ohne verbundenen Client, wird die Session pausiert (`async.run.paused { reason: approval }`), eine Approval-Card in die Inbox gestellt und eine Benachrichtigung ausgelöst: standardmäßig lokal als Desktop-Notification (DESK-005), an die PWA per Web-Push nur, wenn der User Web-Push aktiviert hat (WEB-014, ADR-0033). Läuft der Timeout ab, greift `on_timeout`: `deny` (Default; Aktion abgelehnt, Agent läuft weiter), `allow` oder `abort` (Run endet mit `aborted`).
- **Details:** Timeout-Quelle (erste gewinnt): Regel-Parameter `approval.timeout` → Agent `async.approval.timeout` → Default 24 h. `on_timeout: allow` ist nur zulässig, wenn die auslösende Regel es nicht verbietet (`approval.allow_on_timeout: false` in höheren Ebenen sperrt es; strengere Variante gewinnt). Pausierte Sessions belegen keinen Model-Traffic; der Harness-Prozess bleibt bestehen, solange die Capability das erfordert, und wird nach 1 h Pause per Resume (HAR-020) neu aufgebaut *(Annahme)*. Das Ergebnis ist `approval.resolved { decision: allow|deny|abort, via: user|timeout, actor, on_timeout_applied }` (PROTO-002).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ohne Client und mit `ask` pausiert die Session, ein Inbox-Eintrag und eine Benachrichtigung entstehen (Fake-Notification-Sink im Test); ohne aktivierten Web-Push wird kein Push-Dienst kontaktiert.
  - [ ] AC2 — Nach Ablauf eines 1-min-Timeouts ohne Entscheidung wird die Aktion abgelehnt (`via: timeout`, `on_timeout_applied: deny`) und der Run läuft weiter.
  - [ ] AC3 — Mit `on_timeout: abort` endet der Run mit Status `aborted`.
  - [ ] AC4 — Eine Freigabe vom Handy (PWA) setzt die Session innerhalb von 2 s fort.
  - [ ] AC5 — Agent-`on_timeout: allow` wird ignoriert (→ `deny`), wenn eine höhere Ebene `allow_on_timeout: false` setzt; `policy.decision` dokumentiert das.
- **Abhängigkeiten:** POL-010 ([03](03-policies.md)), HAR-020 ([01](01-harnesses.md)), UX-001 (siehe 11-platform-features.md), WEB-014, DESK-005 (siehe 08-clients.md)

### ASY-009 — Inbox-Anbindung
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Async-Agents erzeugen Inbox-Einträge für: offene Approvals, `ask_user`-Fragen, fertige Runs (`completed`), fehlgeschlagene/abgebrochene Runs (`failed`, `aborted`, `timed_out`, `budget_exceeded`) und nicht dispatchbare Runs. Die Inbox selbst (Projektion, Item-Schema, UI, Filter) ist Owner UX-001 (11-platform-features.md); hier wird nur das Erzeugen spezifiziert.
- **Details:** Eintrag gemäß UX-001 `{ type: approval|question|async_done|async_failed, session_id, ref: { run_id }, title, summary, actions[], created_at }`; Empfänger: Session-Owner plus User mit Freigabe `comment_approve` oder `drive` (COL-001). Approval-Einträge werden beim Auflösen automatisch geschlossen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Jeder der fünf Endstatus erzeugt genau einen Inbox-Eintrag mit Link auf die Session.
  - [ ] AC2 — Wird eine Approval in der Session-Ansicht entschieden, verschwindet der Inbox-Eintrag auf allen Geräten innerhalb von 2 s.
  - [ ] AC3 — Ein User mit Freigabe `view` erhält keine Approval-Einträge.
- **Abhängigkeiten:** ASY-001, ASY-008, UX-001 (siehe 11-platform-features.md), COL-001, COL-009 (siehe 07-sessions-collaboration.md)

### ASY-010 — Kosten- & Laufzeit-Caps pro Run
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Jeder Run hat ein Budget: `max_cost_usd`, `max_duration`, `max_turns`, `max_tool_calls`. Kosten von Child-Sessions zählen zum Run (Subtree). Das Budget wird intern als `spend_cap`-Regel mit `scope: run` auf Agent-Ebene ins Policy-Set eingefügt; damit gilt "strengere Regel gewinnt" auch gegenüber Team- und User-Budgets.
- **Details:** Bei 80 % des Kostenbudgets `notify`; bei Überschreitung: laufender Turn wird unterbrochen, Run endet mit `budget_exceeded`, Inbox-Eintrag. Default-Budget für Async-Runs ohne Angabe: `max_cost_usd: 10`, `max_duration: 4h` *(Annahme; per User/Projekt-Config änderbar)*. Bei Subscription-Nutzung zählen `max_turns`/`max_duration` sowie Token-Äquivalente (USE-004).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Fake-Run, der pro Turn 1 USD verbraucht, endet bei `max_cost_usd: 3` nach dem dritten Turn mit `budget_exceeded`.
  - [ ] AC2 — Child-Kosten zählen zum Parent-Budget: Parent 1 USD + Child 2,5 USD bei Limit 3 USD → Abbruch beider mit `budget_exceeded`.
  - [ ] AC3 — `max_duration: 10m` beendet den Run nach 10 min (simulierte Uhr) mit `timed_out`.
  - [ ] AC4 — Ein Async-Run ohne explizites Budget erhält das Default-Budget; `policy explain` zeigt die erzeugte Regel.
- **Abhängigkeiten:** ASY-001, POL-011 ([03](03-policies.md)), USE-001, USE-004 (siehe 11-platform-features.md)

### ASY-011 — Schedule-/Run-Verwaltung (CLI, API, System-Tools)
- **Meilenstein:** M5 · **Priorität:** Should
- **Beschreibung:** Verwaltung über `beton schedule list|create|update|pause|resume|delete|run-now`, `beton run-history <schedule|agent>`, entsprechende REST-Endpunkte und die System-Tools `schedule_*`. "Run now" startet sofort einen Run mit den Schedule-Einstellungen (`trigger.kind = schedule`, `manual: true`). CLI-Oberfläche: CLI-011.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton schedule run-now nightly-ci` startet einen Run, der in der Run-Historie mit `manual: true` erscheint.
  - [ ] AC2 — Die Run-Historie zeigt pro Run Status, Start, Dauer, Kosten und Link zur Session; `--json` liefert dieselben Felder.
  - [ ] AC3 — Ein Agent kann über `schedule_create` nur Schedules für sich selbst oder für Agents in seiner `spawn.agents`-Liste anlegen.
- **Abhängigkeiten:** ASY-004, AGT-007, CLI-011, API-002 (siehe 08-clients.md)

### ASY-012 — Concurrency-Limits & Warteschlange
- **Meilenstein:** M5 · **Priorität:** Should
- **Beschreibung:** Um Hosts und Rate-Limits zu schützen, begrenzt beton gleichzeitige Async-Runs: pro Host (`host.max_async_runs`, Default 4), pro User (Default 8) und pro Agent (`async.max_concurrent_runs`, Default 2). Überzählige Runs warten in einer FIFO-Queue (Status `queued`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Bei Limit 2 und 3 gleichzeitig ausgelösten Runs startet der dritte erst nach Ende eines der ersten beiden.
  - [ ] AC2 — `beton schedule list --queue` zeigt wartende Runs mit Position.
- **Abhängigkeiten:** ASY-001, ASY-006

## Nicht in v1

- Python-/JS-Funktionen als Tools (dauerhaft ausgeschlossen, ADR-0012); Agent-Erstellung per Chat-Assistent bzw. grafischem Agent-Editor.
- Eingebaute Empfänger für vendor-spezifische Webhooks (GitHub-, GitLab-Events) – v1 nur generische API-Trigger (ADR-0013).
- Dispatch an SaaS-Cloud-Sandboxes (E2B, Daytona, Modal, Fly) und MicroVMs (v2).
- Aufwecken des Rechners aus dem Ruhezustand für Schedules (Wake-Timer).
- RRULE-basierte Schedules (RFC 5545) – v1 nur Cron + Timezone.
- Langzeit-Gedächtnis-Tools (Omnigent Hindsight) und Goal-Mode als Built-in-Feature.
- Smart Routing (automatische Harness-Wahl für Agents), v2.
