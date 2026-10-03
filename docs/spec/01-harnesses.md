# 01 — Harnesses

Dieses Kapitel spezifiziert, wie beton fremde Coding-Agents ("Harnesses") anbindet: das gemeinsame Adapter-Interface mit drei Transporten (natives Protokoll, ACP, PTY/Native-TUI), das Capabilities-Modell, die v1-Adapter (Claude Code, Codex, generisches ACP, Direkt-API/Gateway), die Subscription-Regel, die Credential-Erkennung in `beton setup`, Modell-/Harness-Wechsel, das Parsen fremder Transcripts für den Import sowie die Testbarkeit über Golden-Transcripts.

**Scope:** Crates `beton-harness` (Trait, Transporte), `beton-harness-claude`, `beton-harness-codex`, `beton-harness-acp`, `beton-harness-direct`, Teile von `beton-pty` und `beton-mcp`. Nicht hier: Session-Lebenszyklus, Fork-/Import-UX (SES in 07-sessions-collaboration.md), Event-Katalog und Wire-Format (PROTO in 06-data-sync-protocol.md), Sandbox/Proxy (SBX/PRX in 04-sandbox.md), Policy-Semantik (POL in [03](03-policies.md)), Agent-YAML (AGT in [02](02-agents.md)).

**Bezug:** ADR-0005 (Adapter mit drei Transporten, Subscription-Regel), ADR-0006 (v1-Harness-Umfang), ADR-0008 (Policy-Hooks), ADR-0012 (Agents laufen *auf* Harnesses), ADR-0031 (Golden-Transcript-Tests), ADR-0034 (Subscription-first: kein Feature setzt einen API-Key voraus).

## Konzepte & Begriffe

| Begriff | Bedeutung |
|---|---|
| **Harness** | Runtime, die die Agent-Schleife ausführt: `claude`, `codex`, ein ACP-Agent, der Direkt-API-Loop, der Fake-Harness. |
| **HarnessId** | Stabiler Bezeichner: `claude`, `codex`, `acp:<slug>` (z. B. `acp:gemini`), `direct:<provider>` (z. B. `direct:openrouter`), `fake`. |
| **Modus** | `native` (strukturiertes Protokoll, Default) oder `tui` (Original-Vendor-TUI im PTY). Ein Harness kann beide anbieten. |
| **Transport** | Technischer Kanal eines Adapters: `native` (stream-json bzw. JSON-RPC), `acp` (Agent Client Protocol über stdio), `pty` (Pseudo-Terminal + Vendor-Hooks + Transcript-Tailing), `inproc` (Direkt-API-Loop im Runner). |
| **Adapter** | Implementierung des `HarnessAdapter`-Traits; übersetzt Vendor-Protokoll ⇄ neutrales Event-Modell. |
| **Capabilities** | Pro Adapter (und ggf. pro CLI-Version) deklarierte Fähigkeiten, z. B. Approval-Mechanismus, Resume, Fork-History, Modellwechsel. UI, Policy-Engine und Fork-Logik richten sich danach. |
| **Auth-Herkunft** (`auth_source`) | Woher der Harness seine Model-Credentials hat: `vendor_cli` (Subscription oder Login der offiziellen CLI), `api_key` (Secret aus beton, per Platzhalter injiziert), `gateway` (eigener Endpoint), `none`. |
| **PolicyGate** | Vom Runner bereitgestellte Schnittstelle, über die Adapter vor Model-Requests/Tool-Calls eine Entscheidung einholen (Semantik in [03](03-policies.md)). |
| **Handover-Kontext** | Aus dem Event-Log erzeugtes, harness-neutrales Paket (Transcript, Änderungen, Plan), mit dem ein anderer Harness eine Session fortsetzt. |
| **Golden-Transcript** | Aufgezeichnete echte Vendor-Ausgabe (`raw.jsonl`) plus erwartete normalisierte Events; Regressionstest für Adapter. |

**Grundprinzipien**

1. **Ein Trait, viele Transporte.** Runner und Server kennen nur `HarnessAdapter`/`HarnessSession`; Vendor-Details bleiben im Adapter-Crate.
2. **Kein `raw` geht verloren.** Jedes normalisierte Event trägt optional den Original-Payload (`raw`), damit Import, Debugging und spätere Re-Normalisierung möglich sind.
3. **Fail closed.** Kann ein Adapter eine aktive Policy nicht durchsetzen (z. B. kein Approval-Mechanismus), verweigert er den Start mit klarer Fehlermeldung, statt still zu degradieren (Details POL-024).
4. **Subscription-Regel.** beton startet nur offizielle Vendor-CLIs, fasst deren OAuth-Tokens nie an, bietet keinen eigenen OAuth-Login (HAR-015).

## Design

### Adapter-Trait (Skizze, `beton-harness`)

```rust
pub enum Transport { Native, Acp, Pty, InProc }
pub enum Mode { Native, Tui }

#[async_trait]
pub trait HarnessAdapter: Send + Sync + 'static {
    fn id(&self) -> HarnessId;
    fn modes(&self) -> &[Mode];
    /// Statisch deklariert, ggf. nach `probe()` versionsabhängig verfeinert.
    fn capabilities(&self, mode: Mode, probe: &ProbeReport) -> Capabilities;
    /// Binary finden, Version prüfen, Login-Status über die Vendor-CLI erfragen (nie Token-Dateien lesen).
    async fn probe(&self, env: &HostEnv) -> ProbeReport;
    async fn start(&self, spec: SessionSpec, ctx: AdapterContext) -> Result<Box<dyn HarnessSession>>;
    fn transcript_importer(&self) -> Option<&dyn TranscriptImporter> { None }
}

#[async_trait]
pub trait HarnessSession: Send {
    async fn send(&mut self, input: UserInput) -> Result<TurnId>;      // neuer Turn
    async fn steer(&mut self, input: UserInput) -> Result<()>;         // Eingabe in laufenden Turn (falls Capability)
    async fn interrupt(&mut self) -> Result<()>;
    async fn set_model(&mut self, m: ModelRef, effort: Option<Effort>) -> Result<SwitchOutcome>;
    async fn set_permission_mode(&mut self, m: PermissionMode) -> Result<()>;
    async fn compact(&mut self) -> Result<()>;
    fn events(&mut self) -> BoxStream<'static, NormalizedEvent>;       // inkl. `raw`
    fn native_session_ref(&self) -> Option<NativeSessionRef>;          // z. B. Claude-Session-UUID, Codex-Thread-ID
    async fn shutdown(self: Box<Self>, how: Shutdown) -> Result<ExitInfo>;
}

pub struct AdapterContext {
    pub policy: Arc<dyn PolicyGate>,          // evaluate(PolicyRequest) -> Decision (POL)
    pub mcp: McpInjection,                    // generierte MCP-Konfiguration inkl. System-Tools (HAR-009)
    pub sandbox: Arc<dyn SandboxLauncher>,    // Stufe-1-Wrapper für den Harness-Prozess (SBX)
    pub egress: EgressConfig,                 // Proxy-Adresse, CA-Bundle, Platzhalter-Env (PRX)
    pub workdir: PathBuf,                     // Worktree der Session
    pub handover: Option<HandoverContext>,    // bei Fork/Harness-Wechsel (HAR-018)
    pub clock: Arc<dyn Clock>,                // deterministisch in Tests
}
```

### Capabilities (Beispiel, ausgeliefert über den Harness-Katalog)

```yaml
id: claude
mode: native
transport: native
version_range: ">=2.0.0"            # Annahme: getestete CLI-Versionen
auth_sources: [vendor_cli, api_key]
approval: native_request            # native_request | hook | acp_permission | screen_mirror | none
tool_call_gate: full                # full | approval_only | observe_only
model_switch: live                  # live | restart | none
effort_switch: live
resume: warm                        # warm | cold | none
fork_history: rebuild               # rebuild | preamble | none
interrupt: true
steering: false
subagents: native                   # native | none  (vendor-interne Sub-Agents → Child-Events)
usage_reporting: tokens_and_cost    # tokens_and_cost | tokens | none
compaction: native                  # native | none
instructions_delivery: append_system_prompt   # append_system_prompt | system_prompt | developer_instructions | first_message_prefix
mcp_injection: true
images: true
transcript_import: true
```

### Normalisierung

Adapter erzeugen ausschließlich Events aus dem Event-Katalog PROTO-002 (siehe 06-data-sync-protocol.md), u. a. `session.started`, `turn.started`, `message.delta`, `message.completed`, `reasoning.delta`, `tool.call.requested`, `tool.call.started`, `tool.call.completed`, `approval.requested`, `approval.resolved`, `cost.delta`, `fs.changed`, `terminal.output`. Zusätzlich nutzt dieses Kapitel folgende, in PROTO-002 aufgenommene Typen: `harness.ready`, `harness.exited`, `harness.auth_required`, `harness.incompatible`, `harness.unmapped`, `mcp.server_failed`, `session.settings_changed`, `context.usage`, `usage.subscription`, `compaction.started`, `compaction.completed`.

Für Tool-Calls setzt jeder Adapter `tool.native_name` (z. B. `Bash`, `exec_command`) und die kanonische Klasse `tool.kind` (`shell`, `file_read`, `file_write`, `file_edit`, `search`, `web_fetch`, `mcp`, `system`, `browser`, `other`); die Abbildungstabelle ist Teil von POL-005.

## Features

### HAR-001 — Adapter-Trait & Session-Lebenszyklus
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** `beton-harness` definiert `HarnessAdapter` und `HarnessSession` (siehe Design) sowie die Transport-Enumeration. Der Runner startet genau eine `HarnessSession` pro Session, liest deren Event-Stream, versieht Events mit `session_id`/`seq`/`ts`/`actor` und persistiert sie. Prozess-Supervision (Start, Crash-Erkennung, Shutdown mit Grace-Period) ist für alle Prozess-basierten Transporte gemeinsam implementiert.
- **Details:** `Shutdown::Graceful { timeout: 10s }` sendet zuerst das protokolleigene Ende (stdin schließen bzw. `shutdown`-RPC), danach SIGTERM, nach weiteren 5 s SIGKILL des gesamten Prozessbaums. Ein unerwarteter Exit erzeugt `harness.exited { code, signal, stderr_tail }` (max. 8 KiB stderr).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given ein Adapter, der nach dem ersten Turn abstürzt, When der Runner das bemerkt, Then erscheint `harness.exited` mit `code`/`signal` und `stderr_tail` im Log und die Session wechselt in den Status `failed`.
  - [ ] AC2 — `shutdown(Graceful)` beendet Harness inkl. aller Kindprozesse innerhalb von 15 s; ein Test prüft, dass kein Prozess der Prozessgruppe übrig bleibt.
  - [ ] AC3 — Jedes vom Adapter emittierte Event kann `raw` tragen; ein Unit-Test stellt sicher, dass `raw` unverändert (byte-identisches JSON) im Log landet.
  - [ ] AC4 — Der Trait ist objekt-sicher (`Box<dyn HarnessSession>`) und wird vom Fake-Harness (HAR-026) vollständig implementiert.
- **Abhängigkeiten:** PROTO-001, PROTO-002, DATA-002 (siehe 06-data-sync-protocol.md), RUN-003 (siehe 10-runners-extensibility.md)
- **Referenz:** ADR-0005; Omnigent `HarnessApp`/`ExecutorAdapter`

### HAR-002 — Capabilities-Modell & Harness-Katalog
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Jeder Adapter deklariert Capabilities je Modus (Schema siehe Design). Nach `probe()` können Capabilities versionsabhängig eingeschränkt werden (z. B. kein Live-Modellwechsel bei älterer CLI). Der Server stellt einen Katalog aller verfügbaren Harnesses pro Host bereit, den Clients für Picker, Badges ("nicht eingerichtet") und das Deaktivieren nicht unterstützter Aktionen nutzen.
- **Details:** Capabilities sind Rust-Typen mit `schemars`; der Katalog ist über die API (API-002, siehe 08-clients.md) als `GET /v1/harnesses?host=<id>` abrufbar und enthält `id`, `modes`, `capabilities`, `probe` (`installed`, `version`, `auth_status`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Der Katalog liefert für `claude`, `codex`, jeden registrierten `acp:*`, jeden konfigurierten `direct:*` und `fake` einen Eintrag mit vollständigem Capabilities-Objekt, validiert gegen das generierte JSON-Schema.
  - [ ] AC2 — Given eine CLI-Version unterhalb von `version_range`, Then ist der Harness mit `incompatible: true` und Begründung markiert und `start()` schlägt mit `harness.incompatible` fehl.
  - [ ] AC3 — Eine UI-Aktion, deren Capability `none` ist (z. B. `model_switch`), wird vom Server mit Fehler `capability_unsupported` abgelehnt, statt still ignoriert.
  - [ ] AC4 — Snapshot-Test des Capability-JSON-Schemas ist Teil der CI.
- **Abhängigkeiten:** HAR-001, API-001, API-002 (siehe 08-clients.md)
- **Referenz:** Omnigent `HarnessCapabilities`, `GET /v1/harnesses`

### HAR-003 — Harness-Registry, Binary-Auflösung & Versionsprüfung
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Eingebaute Adapter registrieren sich statisch; Out-of-Process-Harness-Plugins (PLG-002, ab M5, siehe 10-runners-extensibility.md) über denselben Katalog. Das zu startende Binary wird deterministisch aufgelöst, die Version geprüft und das Ergebnis gecacht.
- **Details:** Präzedenz: Env `BETON_<NAME>_PATH` (z. B. `BETON_CLAUDE_PATH`) → `harnesses.<id>.command` in Projekt-Config `.beton/config.yaml` → User-Config `~/.beton/config.yaml` → `PATH`. Versionsermittlung über `<bin> --version`, Timeout 5 s, Cache pro (Pfad, mtime).
  ```yaml
  harnesses:
    claude: { command: /opt/homebrew/bin/claude, extra_args: [], env_passthrough: [] }
    codex:  { command: codex }
    default: claude
  ```
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given `BETON_CLAUDE_PATH` und `harnesses.claude.command` sind gesetzt, Then wird der Env-Pfad verwendet; ein Test deckt alle vier Präzedenzstufen ab.
  - [ ] AC2 — Hängt `<bin> --version` länger als 5 s, wird der Harness als `probe_failed` markiert, ohne den Daemon zu blockieren.
  - [ ] AC3 — `beton doctor` listet pro Harness Pfad, Version, Kompatibilität und Auth-Status.
- **Abhängigkeiten:** HAR-002, OBS-005 (siehe 11-platform-features.md), PLG-002 (ab M5, siehe 10-runners-extensibility.md)

### HAR-004 — Claude-Code-Adapter (stream-json)
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Rust spricht das stream-json-Protokoll der offiziellen `claude`-CLI direkt (kein SDK). Ein langlebiger Prozess pro Session nimmt User-Nachrichten als JSON-Zeilen auf stdin entgegen und liefert strukturierte Nachrichten auf stdout, die auf das neutrale Event-Modell abgebildet werden.
- **Details:** Startkommando *(Annahme: Flags gegen die getestete CLI-Version verifizieren und über Golden-Transcripts absichern)*:
  ```
  claude -p --input-format stream-json --output-format stream-json --verbose
         --include-partial-messages --permission-prompt-tool stdio
         --mcp-config <runner>/mcp.json --model <model>
         --append-system-prompt-file <runner>/instructions.md
         [--session-id <uuid> | --resume <uuid> [--fork-session]]
  ```
  Abbildung: `system/init` → `harness.ready` (+ `native_session_ref`, Tools, MCP-Status); `stream_event` mit `text_delta` → `message.delta`, `thinking_delta` → `reasoning.delta`; `assistant` → `message.completed`, enthaltene `tool_use`-Blöcke → `tool.call.requested`; `user` mit `tool_result` → `tool.call.completed`; `result` → `turn.completed` + `cost.delta` + `context.usage`; `control_request` → siehe HAR-005. Unbekannte Nachrichtentypen werden als `harness.unmapped` mit `raw` geloggt, nicht verworfen. Vendor-interne Sub-Agents (Task-Tool) werden als verschachtelte Tool-Calls mit `parent_tool_call_id` abgebildet.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton run claude` startet eine Session, deren Antwort-Text inkrementell als `message.delta` im Browser erscheint und nach Neuladen der Web-UI vollständig aus dem Event-Log rekonstruiert wird.
  - [ ] AC2 — Golden-Transcripts für mindestens: reiner Text-Turn, Turn mit Bash-Tool, Turn mit Datei-Edit, Turn mit Reasoning, Fehler-Result (`error_max_turns`) sind grün (HAR-025).
  - [ ] AC3 — `interrupt()` bricht einen laufenden Turn ab; das Log enthält `turn.interrupted` und die Session nimmt danach eine neue Eingabe an.
  - [ ] AC4 — Eine unbekannte stdout-Zeile führt nicht zum Abbruch, sondern zu einem `harness.unmapped`-Event und einer `tracing`-Warnung.
  - [ ] AC5 — Instructions des Agents (AGT-005, ab M1, siehe [02](02-agents.md)) werden über `--append-system-prompt-file` geliefert; ein Test prüft den generierten Dateiinhalt.
- **Abhängigkeiten:** HAR-001, HAR-002, HAR-015
- **Referenz:** ADR-0005; Omnigent `claude-sdk`/`claude-native`

### HAR-005 — Claude-Code-Permission-Bridge
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Mit `--permission-prompt-tool stdio` fragt die CLI vor genehmigungspflichtigen Tool-Calls per `control_request` (`can_use_tool`) bei beton an. Der Adapter leitet das an das `PolicyGate` weiter und beantwortet mit `allow` (optional mit `updatedInput`) oder `deny` (mit Begründung, die das Modell sieht). In M0 ohne Policy-Engine fragt das Gate den User per minimaler Approval-Karte (WEB-018, siehe 08-clients.md; CLI: `[y/N]`-Prompt); ab M2 entscheidet die Policy-Engine (POL-006, POL-024).
- **Details:** Weitere Control-Requests: `interrupt`, `set_model`, `set_permission_mode` (von beton an die CLI). Ab M2 installiert der Adapter zusätzlich einen session-spezifischen `PreToolUse`-Hook (gleicher Mechanismus wie HAR-013, über `--settings`; das Hook-Kommando `beton hook claude` entsteht damit bereits in M2 und wird von HAR-013 im TUI-Modus wiederverwendet), damit jeder Tool-Call – auch vom Vendor automatisch erlaubte Reads und Calls im Modus `yolo` – das Gate passiert; `can_use_tool` bleibt für Vendor-Rückfragen zuständig. Ausstehende Approvals überleben einen Client-Disconnect; die CLI wartet blockierend. Bei Runner-Neustart mit offener Approval wird die Session per Resume (HAR-020) wiederhergestellt und der Tool-Call erneut angefragt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given ein Turn, der `Bash` ausführen will, When das Gate `deny("nicht erlaubt")` liefert, Then führt die CLI das Kommando nicht aus und die nächste Assistant-Nachricht bezieht sich auf die Ablehnung (Golden-Transcript).
  - [ ] AC2 — Given das Gate liefert `allow` mit geänderten Argumenten, Then führt die CLI die geänderten Argumente aus (`tool.call.started.args` zeigt die modifizierte Fassung, `tool.call.requested.args` die ursprüngliche).
  - [ ] AC3 — Jede Anfrage erzeugt genau ein `approval.requested` bzw. (ab M2) ein `policy.decision`-Event; Antwort-Latenz des Adapters ohne menschliche Interaktion < 20 ms p99.
  - [ ] AC4 — Antwortet das Gate nicht (Fehler/Timeout), wird `deny` gesendet (fail closed).
- **Abhängigkeiten:** HAR-004, WEB-018 (siehe 08-clients.md), POL-006 (ab M2, siehe [03](03-policies.md))

### HAR-006 — Codex-Adapter (app-server)
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Anbindung von `codex app-server` (JSON-RPC 2.0 über stdio). Der Adapter verwaltet Thread und Turns, bildet Item-Notifications auf Events ab und beantwortet Server-Requests für Approvals über das `PolicyGate`.
- **Details:** *(Annahme: Methodennamen gemäß app-server-Protokoll v2; Rust-Typen werden aus dem von der CLI exportierten JSON-Schema generiert, z. B. `codex app-server generate-json-schema`, und pro getesteter CLI-Version eingefroren.)* Ablauf: `initialize` → `thread/start` (cwd, model, approval_policy, sandbox, developer_instructions, MCP-Server) bzw. `thread/resume` → `turn/start` → Notifications `item/started`, `item/agentMessage/delta`, `item/reasoning/*`, `item/completed`, `thread/tokenUsage/updated`, `turn/completed`; `turn/interrupt` für Abbruch. Server-Requests `item/commandExecution/requestApproval` und `item/fileChange/requestApproval` → `PolicyGate`. Die Codex-eigene Sandbox wird auf `danger-full-access` gesetzt, wenn beton-Sandbox Stufe 2 aktiv ist (keine doppelte Sandbox), sonst auf `workspace-write` *(Annahme)*; `approval_policy` mindestens `untrusted`, damit Shell- und Schreibaktionen das Gate erreichen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton run codex` liefert gestreamte Antworten, Reasoning und Tool-Calls als normalisierte Events; Golden-Transcripts analog HAR-004 AC2 sind grün.
  - [ ] AC2 — Ein Approval-Request für `git push` wird über das Gate entschieden; bei `deny` führt Codex das Kommando nicht aus.
  - [ ] AC3 — `thread/tokenUsage/updated` erzeugt `context.usage`; `turn/completed` erzeugt `cost.delta` mit Tokens (Kosten aus Preis-Katalog bzw. Subscription-Usage, USE-002/USE-004 ab M2, siehe 11-platform-features.md).
  - [ ] AC4 — Abweichende Protokollversion (unbekannte Pflichtfelder, Schema-Mismatch beim `initialize`) erzeugt `harness.incompatible` mit erkannter und erwarteter Version.
- **Abhängigkeiten:** HAR-001, HAR-002, HAR-015
- **Referenz:** Omnigent `codex-native` (app_server.py)

### HAR-007 — Generischer ACP-Harness
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** beton ist ACP-Client und kann jeden ACP-fähigen Agent (Gemini CLI, Goose, Qwen, eigene) als Harness `acp:<slug>` betreiben. Streaming, Reasoning, Tool-Cards, Plan, Interrupt und Permission-Requests werden auf das Event-Modell abgebildet.
- **Details:** Ablauf: `initialize` (beton meldet `clientCapabilities.fs.readTextFile/writeTextFile` und `terminal`) → `session/new` (cwd, `mcpServers`) bzw. `session/load` → `session/prompt` → Notifications `session/update` (`agent_message_chunk` → `message.delta`, `agent_thought_chunk` → `reasoning.delta`, `tool_call`/`tool_call_update` → `tool.call.*`, `plan` → `plan.updated`) → `session/cancel`. Client-Methoden `fs/*` und `terminal/*` führt beton selbst in der Tool-Sandbox (Stufe 2) aus; damit sind diese Aktionen voll policy-kontrolliert. `session/request_permission` → `PolicyGate` (POL-023). ACP-`kind` (`read`, `edit`, `delete`, `move`, `search`, `execute`, `think`, `fetch`, `other`) wird auf `tool.kind` abgebildet. `session/set_model`/`session/set_mode` werden genutzt, wenn der Agent sie anbietet.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit einem ACP-Test-Agent (Teil der Testsuite, deterministisch) laufen Prompt, Streaming, Tool-Call mit Permission-Request und Cancel durch; die Events entsprechen dem Golden-File.
  - [ ] AC2 — (ab M2) Ein `fs/write_text_file`-Aufruf außerhalb des Worktrees wird von der Tool-Sandbox bzw. `path_guard` abgelehnt und dem Agent als JSON-RPC-Fehler gemeldet.
  - [ ] AC3 — Unterstützt der Agent `session/load` nicht, meldet der Katalog `resume: cold` und Fork nutzt `preamble` (HAR-018).
  - [ ] AC4 — Agent-Prozess-Exit während eines Turns erzeugt `harness.exited` und `turn.failed`.
- **Abhängigkeiten:** HAR-001, HAR-002, SBX-002 (ab M2, siehe 04-sandbox.md), POL-016 (ab M2), POL-023 (ab M2, siehe [03](03-policies.md))
- **Referenz:** ADR-0005; Omnigent `acp_harness.py`

### HAR-008 — ACP-Agent-Registrierung & Presets
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** ACP-Agents werden in der User- oder Projekt-Config registriert (interaktiv über `beton setup acp add` oder per Datei). beton liefert Presets für verbreitete Agents mit, die nur aktiv werden, wenn das Binary gefunden wird.
- **Details:**
  ```yaml
  harnesses:
    acp:
      agents:
        gemini:
          command: gemini
          args: ["--experimental-acp"]   # Annahme: Preset-Flags je Vendor verifizieren
          # Auth: Google-Login der Gemini CLI (Standard, kein Key nötig); GEMINI_API_KEY nur optional
          # über `env_passthrough: [GEMINI_API_KEY]` bzw. als Secret (ADR-0034)
          mcp_bridge: true               # System-Tools via MCP (HAR-009)
          models: [gemini-2.5-pro, gemini-2.5-flash]   # optional, für den Modell-Picker
        goose: { command: goose, args: [acp] }
        qwen:  { command: qwen,  args: ["--acp"] }
  ```
  Presets: `gemini`, `goose`, `qwen` *(Annahme)*. Ein eigener Eintrag mit gleichem Slug überschreibt das Preset. Presets setzen keinen API-Key voraus: Der Agent authentifiziert sich mit seinem eigenen Login (z. B. Google-Login der Gemini CLI); API-Keys sind eine zusätzliche Option (ADR-0034).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton setup acp add mein-agent --command ./agent --arg acp` schreibt einen validen Eintrag in `~/.beton/config.yaml`; danach erscheint `acp:mein-agent` im Katalog.
  - [ ] AC2 — Ein Preset, dessen Binary nicht im `PATH` liegt, erscheint im Katalog als `installed: false` und ist nicht startbar.
  - [ ] AC3 — Ungültige Einträge (fehlendes `command`, Slug mit unerlaubten Zeichen außerhalb `[a-z0-9-]`) werden beim Laden mit Datei/Zeile gemeldet und übersprungen, ohne andere Harnesses zu beeinträchtigen.
  - [ ] AC4 — Das Preset `gemini` ist ohne gesetzten `GEMINI_API_KEY` startbar und reicht standardmäßig keine `*_API_KEY`-Variable durch (Test mit ACP-Test-Agent und leerer Umgebung).
- **Abhängigkeiten:** HAR-003, HAR-007

### HAR-009 — MCP-Injektion & System-Tool-Bridge
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Der Runner erzeugt pro Session eine MCP-Konfiguration aus Agent-, Projekt- und User-MCP-Servern (AGT-006) plus dem eingebauten `beton`-MCP-Server mit den System-Tools (AGT-007) und reicht sie an jeden Harness durch. Die Bridge ist pro Harness bzw. ACP-Agent abschaltbar.
- **Details:** Claude: `--mcp-config <datei>`; Codex: MCP-Server in `thread/start`-Config; ACP: `session/new.mcpServers`; Direkt-API: In-Process-MCP-Client. Alle MCP-Server (auch externe aus Agent/Projekt/User) werden nicht direkt, sondern über den beton-MCP-Proxy (`beton mcp proxy --server <name>`) an den Harness gegeben; damit sind MCP-Tool-Calls und -Results unabhängig vom Harness voll policy-kontrolliert (POL-024) *(Annahme)*. Der `beton`-Server läuft als stdio-Relay (`beton mcp serve --session <id>`), das per Unix-Socket bzw. Named Pipe mit dem Runner spricht und ein session-gebundenes Token nutzt. Abschalten: `harnesses.acp.agents.<slug>.mcp_bridge: false` bzw. `executor.mcp_bridge: false` im Agent. Vendor-eigene MCP-Konfigurationen des Users (z. B. in `~/.claude.json`) bleiben bei interaktiven Sessions aktiv; Agent-Läufe nutzen ausschließlich die beton-Konfiguration *(Annahme: für Reproduzierbarkeit, z. B. via `--strict-mcp-config`)*.
- **Akzeptanzkriterien:**
  - [ ] AC1 — In Claude-, Codex- und ACP-Sessions ist das System-Tool `policy_query` aufrufbar und liefert eine Antwort des Runners (Integrationstest je Harness).
  - [ ] AC2 — Mit `mcp_bridge: false` erscheint der `beton`-Server nicht in der an den Harness übergebenen Konfiguration.
  - [ ] AC3 — Das Relay-Token ist an `session_id` gebunden; ein Aufruf mit Token einer anderen Session wird abgelehnt.
  - [ ] AC4 — Startfehler eines MCP-Servers erzeugen `mcp.server_failed { name, error }`, ohne die Session abzubrechen.
- **Abhängigkeiten:** HAR-004, HAR-006, HAR-007, AGT-006, AGT-007 ([02](02-agents.md))

### HAR-010 — Direkt-API-/Gateway-Harness: Agent-Loop
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Eigener Agent-Loop in Rust (`beton-harness-direct`) für API-Keys und Gateways (zusätzliche Option neben den Subscription-Harnesses; kein anderes Feature setzt ihn voraus, ADR-0034): Model-Request streamen → Tool-Uses einsammeln → je Tool-Call `PolicyGate` → Ausführung über MCP → Tool-Results zurück → wiederholen bis `end_turn` oder Limit. Da beton hier jeden Model-Request selbst sendet, sind alle Policy-Phasen voll durchsetzbar.
- **Details:** Coding-Tools stellt der eingebaute MCP-Server `beton-workspace` bereit (`fs_read`, `fs_write`, `fs_edit`, `fs_glob`, `fs_grep`, `shell_exec`), ausgeführt in der Tool-Sandbox Stufe 2 (SBX-002, ab M2) *(Annahme: eigener Server, nur für Direkt-API standardmäßig aktiv)*. Limits: `executor.max_turns` (Default 200 Model-Requests pro User-Turn), Request-Timeout 300 s, Retries mit Exponential-Backoff bei 429/5xx (max. 5, `retry-after` respektiert). Eigene Compaction: bei > 80 % Kontextfenster werden alte Tool-Results gekürzt, danach ältere Turns zusammengefasst (Summary-Request an dasselbe Modell). Parallele Tool-Calls eines Model-Requests werden parallel ausgeführt (max. 4).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Gegen einen Mock-Server (Anthropic- und OpenAI-Wire-Format) erledigt der Loop eine Aufgabe mit drei aufeinanderfolgenden Tool-Calls; das Event-Log entspricht dem Golden-File.
  - [ ] AC2 — `max_turns` wird eingehalten; danach `turn.completed { stop_reason: "max_turns" }`.
  - [ ] AC3 — Bei 429 mit `retry-after: 2` wartet der Loop ≥ 2 s und wiederholt; nach 5 Fehlschlägen `turn.failed` mit Fehlertext im `problem`.
  - [ ] AC4 — Bei simuliertem Kontext-Füllstand > 80 % wird vor dem nächsten Request Compaction ausgeführt (`compaction.started`/`compaction.completed`).
  - [ ] AC5 — (ab M2) Jeder Model-Request durchläuft die Policy-Phase `model_request` vor dem Senden (Test: `deny` verhindert den HTTP-Request am Mock-Server).
- **Abhängigkeiten:** HAR-001, HAR-011, AGT-006, SBX-002 (ab M2, siehe 04-sandbox.md), POL-003 (ab M2, siehe [03](03-policies.md))

### HAR-011 — Provider-Konfiguration & Wire-APIs
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Provider werden deklarativ konfiguriert; zwei Wire-Familien decken Anthropic direkt sowie alle OpenAI-kompatiblen Endpunkte ab (OpenRouter, LiteLLM, vLLM, Ollama, LM Studio). API-Keys sind Secret-Referenzen; der Harness-Prozess bzw. Loop sieht nur Platzhalter, die der Proxy ersetzt. **Übergang in M1** (vor Secret-Store und Proxy): Keys werden über `api_key_env: <VAR>` aus einer Umgebungsvariable des Daemons gelesen und nie in Config-Dateien, Events oder Logs geschrieben. Ab M2 ist `secret://…` der Standard; `api_key_env` bleibt als Quelle erlaubt und wird intern wie ein Secret der Quelle `env` behandelt (SEC-001).
- **Details:**
  ```yaml
  providers:
    anthropic:
      kind: anthropic                 # anthropic | openai
      base_url: https://api.anthropic.com
      api_key: secret://anthropic/default        # Secret-Ref (SEC-001, siehe 05-security-identity.md)
      prompt_caching: true
    openrouter:
      kind: openai
      base_url: https://openrouter.ai/api/v1
      api_key_env: OPENROUTER_API_KEY          # M1-Übergangsform; ab M2 alternativ secret://openrouter/default
      models:
        - id: qwen/qwen3-coder
          context_window: 262144
          pricing: { input_per_mtok: 0.4, output_per_mtok: 1.6 }   # eigene Preise = Preis-Override (Owner USE-003)
    ollama:
      kind: openai
      base_url: http://127.0.0.1:11434/v1
      api_key: none
  ```
  `anthropic` nutzt die Messages-API (Streaming, `tool_use`, `thinking`, `cache_control`); `openai` nutzt Chat Completions (Streaming, `tools`). Modell-Discovery über `GET /models` mit Last-Known-Good-Cache. Lokale Endpunkte (Loopback/LAN) benötigen eine explizite Egress-Freigabe (PRX-003, PRX-007). Preisangaben unter `providers.*.models[].pricing` werden als Preis-Override nach USE-003 behandelt (Owner der Preislogik: USE-003).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `direct:openrouter` mit Modell aus `models` läuft gegen einen Mock mit OpenAI-Wire-Format inkl. Tool-Calls und Streaming.
  - [ ] AC2 — (ab M2) Der Klartext-Key erscheint weder im Event-Log noch in Prozess-Env oder Logs; im Loop-Request steht ein `bt_cred_*`-Platzhalter, den der Proxy ersetzt (Integrationstest mit Proxy).
  - [ ] AC3 — Fällt `GET /models` aus, nutzt der Picker die zuletzt erfolgreiche Liste und kennzeichnet sie als veraltet.
  - [ ] AC4 — Ungültige Provider-Config (unbekanntes `kind`, fehlende `base_url`) wird beim Laden mit Pfad und Grund gemeldet.
  - [ ] AC5 — (M1) Mit `api_key_env: OPENROUTER_API_KEY` liest der Loop den Key aus der Daemon-Umgebung; ist die Variable nicht gesetzt, schlägt der Session-Start mit Hinweis auf die Variable fehl; der Key-Wert erscheint nicht im Event-Log und nicht in Logs (Test mit Marker-Wert).
- **Abhängigkeiten:** HAR-010, PRX-006 (ab M2, siehe 04-sandbox.md), SEC-001 (ab M2, siehe 05-security-identity.md), USE-003 (ab M2, siehe 11-platform-features.md)

### HAR-012 — Native-TUI-/PTY-Modus
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Statt des strukturierten Protokolls läuft die Original-Vendor-TUI (`claude`, `codex`) in einem PTY des eigenen Multiplexers (`beton-pty`, kein tmux). Terminal-Bytes werden über einen Binärkanal an alle Clients gespiegelt (xterm.js in Web/Desktop, ratatui-TUI); Eingaben kommen vom aktuell steuernden Client. Strukturierte Events (Nachrichten, Tool-Calls, Usage) entstehen parallel aus Vendor-Hooks und dem Tailing der Vendor-Transcript-Datei.
- **Details:** Start mit `--mode tui` bzw. `executor.mode: tui`. Terminal-Größe folgt dem steuernden Client; andere Clients skalieren. Transcript-Tailing: Claude `~/.claude/projects/<slug>/<session>.jsonl`, Codex `~/.codex/sessions/**/rollout-*.jsonl` (dieselben Parser wie HAR-023/HAR-024, inkrementell). `terminal.output` ist ephemer; im Log landen nur periodische Snapshots (DATA-011). Sandbox Stufe 1 gilt wie im nativen Modus.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton run claude --mode tui` zeigt die Original-TUI im Browser; Tastatureingaben aus dem Browser erreichen die TUI mit < 50 ms Echo-Latenz lokal (p95).
  - [ ] AC2 — Zwei Clients sehen dieselbe Ausgabe; nur der steuernde Client kann tippen, der andere sieht einen "zuschauen"-Hinweis.
  - [ ] AC3 — Nachrichten und Tool-Calls aus der TUI erscheinen innerhalb von 2 s als strukturierte Events (Transcript-Tailing), sodass Chat-Ansicht und Suche funktionieren.
  - [ ] AC4 — Läuft unter macOS, Linux und Windows (ConPTY) ohne tmux; CI-Test startet eine Fake-TUI im PTY auf allen drei OS.
- **Abhängigkeiten:** HAR-001, TUI-002, WEB-010 (siehe 08-clients.md), PROTO-007, DATA-011 (siehe 06-data-sync-protocol.md), SBX-017 (siehe 04-sandbox.md)
- **Referenz:** ADR-0005 Transport 3; Omnigent `*-native` (tmux)

### HAR-013 — Claude-Code-TUI: Hooks-Integration
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Im TUI-Modus setzt beton Policies über Claude-Code-Hooks durch. beton erzeugt dafür eine session-spezifische Settings-Datei und übergibt sie per `--settings`; die globale `~/.claude/settings.json` des Users wird nie verändert.
- **Details:** Hooks `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `Notification` rufen `beton hook claude --event <Name>` auf. Das Hook-Kommando liest den Hook-JSON von stdin, verbindet sich über `BETON_HOOK_SOCKET` (+ `BETON_HOOK_TOKEN`) mit dem Runner und gibt die Entscheidung als Hook-Output zurück (`permissionDecision: allow|deny|ask`, `permissionDecisionReason`, ggf. `updatedInput`). `ask` wird **in beton** als Approval-Card behandelt; der Hook wartet bis zur Entscheidung (Hook-`timeout` = Approval-Timeout + 30 s). Im TUI-Modus ist der Approval-Timeout deshalb auf **höchstens 1 h** begrenzt (überschreibt den globalen Default von 24 h, POL-010); danach gilt `deny` mit dem Grund „Approval-Timeout (TUI-Modus)“. Runner nicht erreichbar → Exit-Code 2 (blockierend), also fail closed.
  ```json
  { "hooks": { "PreToolUse": [ { "matcher": "*", "hooks": [
      { "type": "command", "command": "beton hook claude --event PreToolUse", "timeout": 1830 } ] } ] } }
  ```
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given eine Policy `deny` für `git push --force`, When der User in der TUI Claude einen Force-Push ausführen lässt, Then wird er blockiert und die TUI zeigt die beton-Begründung.
  - [ ] AC2 — Given eine `ask`-Policy, Then erscheint eine Approval-Card in Web/Desktop; nach Freigabe läuft das Tool in der TUI weiter.
  - [ ] AC3 — Ist der Runner-Socket weg, blockiert `PreToolUse` mit Exit-Code 2 (Test mit gestopptem Runner).
  - [ ] AC4 — Nach Session-Ende ist die generierte Settings-Datei gelöscht; `~/.claude/settings.json` ist byte-identisch zum Zustand vor dem Start.
  - [ ] AC5 — Given eine `ask`-Policy mit Approval-Timeout 24 h im TUI-Modus, Then wird der effektive Timeout auf 1 h begrenzt; ohne Antwort endet der Hook danach mit `deny` und dem Grund „Approval-Timeout (TUI-Modus)“, ohne dass der Hook-Prozess vorher abgebrochen wird.
- **Abhängigkeiten:** HAR-012, POL-022 ([03](03-policies.md))

### HAR-014 — Codex-TUI: Approval-Integration
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Im TUI-Modus von Codex werden Policies über Codex' Approval-Mechanismus durchgesetzt. Weil Codex-Versionen unterschiedliche Erweiterungspunkte bieten, wählt der Adapter per `probe()` die stärkste verfügbare Strategie und deklariert sie in den Capabilities.
- **Details:** Strategien in absteigender Präferenz *(Annahme, gegen aktuelle Codex-Version verifizieren)*: (A) Vendor-Hooks bzw. Approval-Callback, falls von der CLI angeboten → `approval: hook`, `tool_call_gate: approval_only`; (B) `--ask-for-approval untrusted` + Erkennung des Approval-Dialogs im PTY-Bildschirm, Spiegelung als Approval-Card, Antwort per Tastendruck → `approval: screen_mirror`. Lesende Tools, die Codex ohne Approval ausführt, sind nur beobachtbar (`observe_only`), die Sandbox ist dann der Backstop. Verlangt das effektive Policy-Set eine Durchsetzung, die die gewählte Strategie nicht leisten kann, verweigert der Start (POL-024).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton doctor` zeigt die gewählte Strategie und `tool_call_gate` für die installierte Codex-Version.
  - [ ] AC2 — Mit Strategie B erzeugt ein Approval-Dialog der TUI innerhalb von 1 s eine Approval-Card; "deny" in der Card lehnt den Dialog in der TUI ab.
  - [ ] AC3 — Given eine Policy mit `deny` auf `file_read`-Tools und Capability `observe_only` für Reads, Then startet die Session nicht und die Fehlermeldung nennt Regel-ID und fehlende Capability.
- **Abhängigkeiten:** HAR-012, POL-024 ([03](03-policies.md))

### HAR-015 — Subscription-Regel & Auth-Herkunft
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Subscriptions (Claude Pro/Max, ChatGPT Plus/Pro) funktionieren ausschließlich über die offizielle CLI, die sich selbst authentifiziert. beton liest, kopiert, speichert oder injiziert keine OAuth-Tokens und bietet keinen eigenen OAuth-Login an. Pro Harness wählt der User explizit die Auth-Herkunft; beton sorgt dafür, dass genau diese wirksam ist.
- **Details:**
  ```yaml
  harnesses:
    claude: { auth: subscription }       # subscription | api_key
    codex:  { auth: api_key, api_key: secret://openai/default }
  ```
  Ohne Angabe gilt `auth: subscription` für alle Vendor-CLI-Harnesses (`claude`, `codex`, `acp:*`); `api_key` ist nur eine zusätzliche Option und wird von keinem Feature vorausgesetzt (ADR-0034). Modell-Hilfsfunktionen (z. B. Session-Titel, SES-010) rufen dieselbe CLI im Einmal-Modus (z. B. `claude -p`, `codex exec`) mit derselben Auth-Herkunft und Umgebungsbereinigung auf. `subscription` (= `auth_source: vendor_cli`): beton entfernt `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN` bzw. `OPENAI_API_KEY` aus der Harness-Umgebung (sonst würde die CLI ggf. API-Billing nutzen). Die Model- und Auth-Hosts des Vendors werden im Egress-Proxy als TLS-Passthrough (CONNECT ohne Terminierung) konfiguriert, sodass beton den Token auch im Transit nie sieht *(Annahme; Konfiguration siehe PRX-005 in 04-sandbox.md)*. Die Sandbox Stufe 1 erlaubt der CLI Lesezugriff auf ihre eigenen Credential-Speicher. `api_key`: Env-Variable enthält nur einen `bt_cred_*`-Platzhalter, der Proxy injiziert den echten Wert. Läuft die CLI mit abgelaufenem Login, wird `harness.auth_required { harness, hint: "claude auth login" }` emittiert. Offener Punkt (Nutzungsbedingungen) siehe Überblick, offener Punkt 4.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit `auth: subscription` und gesetztem `ANTHROPIC_API_KEY` in der Shell enthält die Harness-Prozess-Umgebung keinen der beiden Anthropic-Key-Variablen (Test über `/proc/<pid>/environ` bzw. `ps eww`).
  - [ ] AC2 — Code-Review-Check in CI: kein Code in `beton-harness-*` öffnet bekannte Vendor-Credential-Pfade (`~/.claude/.credentials.json`, `~/.codex/auth.json`, Keychain-Einträge der Vendors); ein Lint-Test grept nach diesen Pfaden.
  - [ ] AC3 — Abgelaufener Login (simuliert über Fake-CLI, QA-002) erzeugt `harness.auth_required`, und die UI zeigt die Anweisung, den Login in der Vendor-CLI auszuführen.
  - [ ] AC4 — (ab M2) Mit `auth: api_key` erscheint im Harness-Env nur ein `bt_cred_*`-Wert; der echte Key wird erst im Proxy eingesetzt.
  - [ ] AC5 — Ohne `auth`-Angabe und ohne `*_API_KEY` in der Umgebung startet eine Claude-Session mit `auth_source: vendor_cli` (Fake-CLI-Test, ab M1 ebenso Codex); kein Start- oder Hilfspfad bricht wegen eines fehlenden API-Keys ab.
- **Abhängigkeiten:** HAR-004, PRX-005 (ab M2), PRX-006 (ab M2), SBX-002 (ab M2, siehe 04-sandbox.md), SEC-001 (ab M2, siehe 05-security-identity.md)
- **Referenz:** ADR-0005 Subscription-Regel, ADR-0034 Subscription-first

### HAR-016 — Credential-Erkennung in `beton setup`
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** `beton setup` ermittelt, welche Harnesses nutzbar sind, und führt den User durch fehlende Schritte. Erkannt werden installierte CLIs (Pfad, Version), Login-Status über die Vendor-CLI selbst, API-Keys in der Umgebung, lokale Modell-Server und ACP-Agents im `PATH`. Fehlende CLIs werden zur Installation **angeboten**, nie still installiert.
- **Details:** Login-Status nur über Vendor-Kommandos (z. B. `claude auth status`, `codex login status`; *Annahme: exakte Subkommandos versionsabhängig, sonst Status "unbekannt"*), niemals durch Lesen von Token-Dateien. Env-Keys: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `OPENROUTER_API_KEY` → Angebot, sie in den Keychain zu übernehmen (SEC-002, ab M2). Lokale Server: Probe auf `127.0.0.1:11434` (Ollama) und `:1234` (LM Studio) mit 500 ms Timeout. Login: beton startet auf Wunsch `claude auth login` bzw. `codex login` im Vordergrund mit geerbtem TTY; die CLI übernimmt die Anmeldung vollständig. Installation: zeigt den exakten Befehl des Vendors und führt ihn erst nach Bestätigung aus. HAR-016 ist Owner von Erkennung und Installationsangebot; DIST-015 (ab M3, siehe 12-distribution-quality.md) ergänzt die gepflegten Installationsbefehle je Plattform, `--install-clis` und die Anbindung des UI-Wizards (UX-008). `beton setup --check --json` liefert den Status maschinenlesbar.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Auf einem System ohne `codex` zeigt `beton setup` "Codex: nicht installiert" mit Installationsbefehl; ohne Bestätigung wird nichts installiert (Test mit gemocktem Prozess-Spawner).
  - [ ] AC2 — `beton setup --check --json` liefert pro Harness `installed`, `version`, `auth_status ∈ {logged_in, logged_out, unknown}`, `api_key_env_found` und validiert gegen ein JSON-Schema.
  - [ ] AC3 — (ab M2) Ein gefundener `OPENAI_API_KEY` wird nur nach expliziter Zustimmung in den Keychain übernommen; der Wert erscheint nie in der Ausgabe (nur `sk-…abcd` maskiert).
  - [ ] AC4 — Ein laufendes Ollama wird erkannt und als Vorschlag `providers.ollama` angeboten.
- **Abhängigkeiten:** HAR-003, HAR-015, CLI-005 (siehe 08-clients.md), SEC-002 (ab M2, siehe 05-security-identity.md), DIST-015 (ab M3, siehe 12-distribution-quality.md)

### HAR-017 — Modell- & Effort-Wechsel mid-session
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Modell und Reasoning-Effort sind während einer Session wechselbar (UI-Picker, `/model`, `/effort`, Policy-Aktion `modify`). Der Adapter wählt je Capability den Mechanismus: live oder per Neustart mit Resume; die History bleibt erhalten.
- **Details:** Claude nativ: Control-Request `set_model` (live, ab nächstem Request). Codex: `model`/`effort` als Override in `turn/start` (live ab nächstem Turn). ACP: `session/set_model`, falls angeboten, sonst `restart`. Direkt-API: trivial live. TUI-Modi: Injektion des Slash-Kommandos (`/model <id>`) *(Could, Annahme)*. Effort-Mapping: `low|medium|high|xhigh` → Vendor-Werte; nicht unterstützte Stufen werden auf die nächstniedrigere gemappt und gemeldet. Ergebnis: `session.settings_changed { model, effort, requested_effort?, mechanism: live|restart, effective_from_turn }` (PROTO-002).
- **Akzeptanzkriterien:**
  - [ ] AC1 — In einer Claude-Session ändert ein Wechsel auf ein anderes Modell das Modell des nächsten Turns (sichtbar in `cost.delta.model` des nächsten Turns), ohne Verlust der History.
  - [ ] AC2 — Bei Capability `model_switch: restart` wird der Harness mit Resume neu gestartet; der nächste Turn kennt den vorherigen Kontext (Golden-Test mit Fake-Harness).
  - [ ] AC3 — Ein Effort-Wert, den das Modell nicht unterstützt, wird gemappt und in `session.settings_changed.effort` als effektiver Wert mit `requested_effort` ausgewiesen.
  - [ ] AC4 — Ein Wechsel während eines laufenden Turns wird auf das Turn-Ende verschoben (`effective_from_turn` = nächster Turn).
- **Abhängigkeiten:** HAR-004, HAR-006, HAR-007, HAR-010, PROTO-002 (siehe 06-data-sync-protocol.md)

### HAR-018 — Handover-Kontext für Harness-Wechsel & Fork
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Für Fork ab Event X auf einen anderen Harness (UX und Session-Semantik: SES-006, SES-007 in 07-sessions-collaboration.md; ein Harness-Wechsel geschieht **ausschließlich per Fork**, eine Session bleibt immer auf ihrem Harness) erzeugt `beton-harness` aus dem Event-Log einen harness-neutralen Handover-Kontext und liefert ihn dem Ziel-Adapter je Capability `fork_history` aus: `rebuild` (gleicher Harness, HAR-019) oder `preamble`.
- **Details:** `HandoverContext { transcript, changed_files, worktree, branch, plan, open_todos, agent_ref, source_harness, up_to_seq }`. Präambel-Rendering (deterministisch, ohne LLM): Markdown-Dokument `.beton/handover/<session>.md` im Worktree plus erste Nachricht mit Kurzfassung und Verweis. Budget: max. 40 % des Ziel-Kontextfensters (`context_window` aus Katalog), Kürzungsregeln in Reihenfolge: (1) Tool-Results > 2 KiB auf Kopf/Ende kürzen, (2) Reasoning entfernen, (3) ältere Turns auf User-Nachricht + Tool-Call-Einzeiler reduzieren, (4) älteste Turns auslassen mit Marker `[… N Turns ausgelassen …]`. Die letzten 3 Turns bleiben immer vollständig.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Fork einer Claude-Session auf Codex ab `seq` N: Codex erhält die Präambel, kennt Ziel und letzte Änderungen (Test mit Fake-Harness, der die erhaltene erste Nachricht spiegelt).
  - [ ] AC2 — Das Präambel-Rendering ist deterministisch: gleicher Log-Ausschnitt → byte-identisches Markdown (Golden-Test).
  - [ ] AC3 — Bei einem 1-MB-Transcript und 128k-Kontextfenster bleibt die Präambel unter dem Budget; die letzten 3 Turns sind vollständig enthalten.
  - [ ] AC4 — Das Log der neuen Session enthält `session.forked { from_session, at_seq, harness, history_mode: rebuild|preamble }` (PROTO-002).
- **Abhängigkeiten:** HAR-002, SES-006, SES-007 (siehe 07-sessions-collaboration.md), PROTO-002 (siehe 06-data-sync-protocol.md)

### HAR-019 — Native History-Rebuild (gleicher Harness)
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Bei Fork oder Resume auf denselben Harness wird die volle native History verwendet statt einer Präambel. Für Claude nutzt der Adapter bevorzugt den Vendor-Mechanismus (`--resume <id> --fork-session`); ist die native Session nicht mehr vorhanden (anderer Rechner, Import), wird eine Vendor-Session-Datei aus dem Event-Log rekonstruiert.
- **Details:** Rekonstruktion nutzt die gespeicherten `raw`-Payloads (verlustfrei, wenn vorhanden), sonst eine Synthese aus normalisierten Events. Ziel: Claude-Projektverzeichnis bzw. Codex-Session-Verzeichnis des Users *(Annahme: Schreiben von Session-Dateien ist zulässig, da keine Credentials betroffen)*. Fork ab `seq` X kürzt die History exakt auf X. Bei Fehlschlag automatischer Fallback auf `preamble` mit Hinweis-Event.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Fork einer Claude-Session ab Turn 3 von 5: Die neue Session kennt Turn 1–3, nicht Turn 4–5 (Fake-Harness bzw. Golden-Test mit echter CLI im manuellen Testlauf).
  - [ ] AC2 — Eine importierte Claude-Session (HAR-023) kann per Rebuild fortgesetzt werden.
  - [ ] AC3 — Schlägt der Rebuild fehl (z. B. Datei-Schema unbekannt), wird `preamble` genutzt und `session.forked.history_mode = "preamble"` mit `fallback_reason` gesetzt.
- **Abhängigkeiten:** HAR-018, HAR-023, HAR-024

### HAR-020 — Resume nach Runner-Neustart
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Startet ein Runner neu (Crash, Host-Reboot, Update), setzt beton laufende Sessions mit Capability `resume: warm` über die native Session-Referenz fort (Claude `--resume`, Codex `thread/resume`, ACP `session/load`). Ohne Warm-Resume greift HAR-018 (Präambel).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach Kill des Runners während einer idle Claude-Session nimmt die neu gestartete Session die nächste Nachricht mit erhaltenem Kontext an.
  - [ ] AC2 — `native_session_ref` wird bei `harness.ready` persistiert; ein Test prüft, dass sie im Log steht, bevor der erste Turn startet.
  - [ ] AC3 — War zum Zeitpunkt des Absturzes ein Turn aktiv, endet er im Log mit `turn.failed` (Problem `runner_restarted`); offene Approvals werden als `approval.resolved { decision: "abort", via: "system" }` geschlossen.
- **Abhängigkeiten:** HAR-001, HAR-018

### HAR-021 — Usage- & Kontext-Reporting
- **Meilenstein:** M0 · **Priorität:** Should
- **Beschreibung:** Adapter liefern Token-Verbrauch und, falls vom Harness gemeldet, Kosten als `cost.delta` sowie den Kontext-Füllstand als `context.usage`. Bei Subscription-Nutzung wird statt Euro die Token-/Rate-Limit-Nutzung gemeldet. Preisberechnung und Usage-Seite liegen in USE-001 bzw. USE-006 (siehe 11-platform-features.md).
- **Details:** `cost.delta { model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cost_micro?, currency, source: reported|estimated|subscription, auth_source }` (PROTO-002); `context.usage { used_tokens, window_tokens }`. Rate-Limit-Informationen, die die CLI meldet, werden als `usage.subscription { vendor, window, used_pct, resets_at }` (PROTO-002) durchgereicht *(Annahme)*.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach jedem Claude-Turn steht genau ein `cost.delta` mit den Werten aus der `result`-Nachricht im Log.
  - [ ] AC2 — Bei `auth_source: vendor_cli` ist `cost_micro` leer und `source: subscription`; ein API-Äquivalent erscheint nie als tatsächliche Ausgabe (USE-004).
  - [ ] AC3 — (ab M2) `session.cost_usd` (Policy-Variable) ist nach jedem `cost.delta` aktualisiert (Test über Policy-Explain, POL-027).
- **Abhängigkeiten:** HAR-004, PROTO-002 (siehe 06-data-sync-protocol.md), USE-001 (ab M2, siehe 11-platform-features.md)

### HAR-022 — Compaction-Durchreichung
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** `/compact` und automatische Compaction werden an Harnesses mit `compaction: native` durchgereicht (Claude: `/compact` als Nachricht bzw. Control-Request; Codex: entsprechendes RPC, falls vorhanden). Der Direkt-API-Harness kompaktiert selbst (HAR-010). Ergebnis-Events: `compaction.started`, `compaction.completed { before_tokens, after_tokens }`. HAR-022 ist Owner der Adapter-Mechanik; Session-API und Events: SES-011, UI-Trigger und `compaction.auto`: USE-009.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `compact()` auf einer Claude-Session erzeugt `compaction.started`/`compaction.completed` und senkt `context.usage.used_tokens`.
  - [ ] AC2 — Bei `compaction: none` ist der Compact-Button deaktiviert und die API antwortet `capability_unsupported`.
- **Abhängigkeiten:** HAR-004, HAR-006, HAR-010

### HAR-023 — Transcript-Import: Claude-Code-Parser
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** `TranscriptImporter` für Claude Code: findet lokale Sessions unter `~/.claude/projects/*/*.jsonl` und übersetzt sie in normalisierte Events (Import-UX, Deduplikation und Session-Anlage siehe SES-008 in 07-sessions-collaboration.md).
- **Details:** Format ist vendor-intern und undokumentiert → tolerantes Parsen. Datensätze `user`, `assistant`, `system`, `summary`; Verkettung über `uuid`/`parentUuid` (Baum; der aktive Zweig ist der Pfad zum letzten Blatt); `isSidechain` = Vendor-Sub-Agent → verschachtelte Tool-Calls. Unbekannte Records → `harness.unmapped`. Ergebnis enthält `native_session_ref`, Projektpfad (`cwd`), Zeitstempel und Modell.
  ```rust
  pub trait TranscriptImporter {
      fn discover(&self, env: &HostEnv) -> Result<Vec<ExternalSessionRef>>;   // id, path, cwd, title?, updated_at
      fn parse(&self, r: &ExternalSessionRef) -> Result<ImportedTranscript>;   // events + warnings
  }
  ```
- **Akzeptanzkriterien:**
  - [ ] AC1 — Fixtures aus mindestens zwei Claude-CLI-Versionen werden ohne Fehler geparst; die normalisierten Events entsprechen Golden-Files.
  - [ ] AC2 — Bei verzweigten Transcripts wird nur der aktive Zweig importiert; verworfene Zweige werden als Warnung gezählt.
  - [ ] AC3 — Eine beschädigte Zeile führt zu einer Warnung mit Zeilennummer; der Rest wird importiert.
  - [ ] AC4 — `discover()` liest nur Session-Dateien und keine Credential-Dateien (Test mit überwachten Dateizugriffen bzw. Pfad-Allowlist).
- **Abhängigkeiten:** HAR-001, SES-008 (siehe 07-sessions-collaboration.md)
- **Referenz:** Omnigent `omnigent import --harness claude`

### HAR-024 — Transcript-Import: Codex-Parser
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** `TranscriptImporter` für Codex: findet Sessions unter `~/.codex/sessions/**/rollout-*.jsonl` (bzw. `$CODEX_HOME`) und übersetzt sie in normalisierte Events.
- **Details:** Records `session_meta` (id, cwd, Modell), `turn_context`, `response_item` (Nachrichten, Reasoning, Function-Calls/-Outputs), `event_msg` (u. a. Token-Usage) *(Annahme: Feldnamen versionsabhängig, tolerantes Parsen)*. Function-Calls `exec_command`/`shell` → `tool.kind = shell`, `apply_patch` → `file_edit` mit Diff.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Fixtures aus mindestens zwei Codex-Versionen werden geparst; Golden-Files sind grün.
  - [ ] AC2 — `apply_patch`-Aufrufe erzeugen `tool.call.*`-Events mit Diff und `fs.changed`-Einträgen für die betroffenen Pfade.
  - [ ] AC3 — `$CODEX_HOME` wird respektiert; ohne Variable gilt `~/.codex`.
- **Abhängigkeiten:** HAR-001, SES-008 (siehe 07-sessions-collaboration.md)

### HAR-025 — Golden-Transcript-Tests
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Jeder Adapter wird gegen aufgezeichnete echte Vendor-Ausgaben getestet. Ein Test spielt `raw.jsonl` als simulierten Prozess ab, prüft die normalisierten Events und – bidirektional – die vom Adapter an den Prozess gesendeten Nachrichten. Aufnahmen entstehen manuell mit echten CLIs; CI benötigt keine Subscription. HAR-025 ist Owner von Format, Aufnahme und Test-Runner; CI-Gate und Drift-Prozess bei neuen CLI-Versionen regelt QA-003.
- **Details:**
  ```
  crates/beton-harness-claude/tests/golden/bash-tool/
    meta.yaml              # cli_version, recorded_at, scenario, platform
    script.yaml            # Eingaben + Gate-Entscheidungen (deterministisch)
    raw.jsonl              # Vendor-stdout, mit Zeitmarken
    expected.stdin.jsonl   # erwartete Nachrichten von beton an die CLI
    expected.events.jsonl  # normalisierte Events
  ```
  Normalisierung vor Vergleich: Zeitstempel → relative Offsets, UUIDs/IDs → `<id:n>`, Pfade → `<workdir>`. Aufnahme: `beton dev record-golden --harness claude --scenario bash-tool` (Secret-Scrubbing: bekannte Key-Muster und `bt_cred_*` werden ersetzt; Abbruch, falls nach dem Scrubbing noch ein Key-Muster gefunden wird). Neuaufnahme bei neuer CLI-Version: Diff-Report statt stillem Überschreiben.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `cargo nextest run -p beton-harness-claude` führt alle Golden-Fälle offline aus; ein Abweichungsfall zeigt einen lesbaren Event-Diff.
  - [ ] AC2 — `record-golden` bricht ab, wenn die Aufnahme nach dem Scrubbing noch ein Muster wie `sk-ant-` oder `sk-` mit ≥ 20 Zeichen enthält.
  - [ ] AC3 — Für Claude, Codex und ACP existiert je ein Mindest-Satz von Szenarien (Text, Tool-Call, Approval-deny, Interrupt, Fehler).
  - [ ] AC4 — `meta.yaml` jeder Aufnahme nennt die CLI-Version; der Katalog-`version_range` (HAR-002) wird in CI gegen die Versionen der Aufnahmen geprüft.
- **Abhängigkeiten:** HAR-001, QA-003 (siehe 12-distribution-quality.md)
- **Referenz:** ADR-0031

### HAR-026 — Fake-Harness
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Deterministischer Harness `fake`, der Szenarien aus YAML abspielt (Text-Deltas, Tool-Calls mit Gate-Anfragen, Fehler, Verzögerungen) und alle Capabilities konfigurierbar deklariert. Er ist Grundlage für E2E-Tests (Playwright gegen Web/Desktop), Policy-Integrationstests und Demos ohne Subscription. HAR-026 ist Owner des Szenario-Formats; die Protokoll-Fake-CLIs (QA-002) spielen dieselben Szenarien über die echte Prozessgrenze der Adapter ab.
- **Details:**
  ```yaml
  # tests/fake/push-ask.yaml
  capabilities: { approval: native_request, model_switch: live, fork_history: preamble }
  turns:
    - expect_input: "Bitte pushen"
      emit:
        - { message_delta: "Ich pushe jetzt.", chunk: 4, delay_ms: 10 }
        - { tool_call: { name: Bash, kind: shell, args: { command: "git push origin main" } }, gate: true }
        - { on_gate: { allow: [{ tool_result: "ok" }], deny: [{ message: "Push abgelehnt." }] } }
        - { usage: { input_tokens: 1200, output_tokens: 80, cost_usd: 0.01 } }
  ```
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton run fake --scenario tests/fake/push-ask.yaml` erzeugt bei identischer Eingabe ein byte-identisches Event-Log (abzüglich `ts`).
  - [ ] AC2 — Der Fake-Harness kann Capabilities so deklarieren, dass jeder Fehlerpfad (Start verweigert, `capability_unsupported`, Crash, Auth-Ablauf) testbar ist.
  - [ ] AC3 — Der Fake-Harness ist in Release-Builds nur mit `--dev` bzw. Feature-Flag verfügbar und erscheint sonst nicht im Katalog.
- **Abhängigkeiten:** HAR-001, QA-002, QA-007 (siehe 12-distribution-quality.md)

### HAR-027 — Permission-Mode-Mapping
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** beton bietet harness-übergreifend vier Betriebsmodi und bildet sie auf die Vendor-Mechanismen ab: `plan` (nur lesen/planen), `default` (Vendor-Default, Approvals laut Policy), `accept_edits` (Datei-Edits im Worktree ohne Rückfrage), `yolo` (keine Vendor-Rückfragen; Schutz ausschließlich durch beton-Policies und Sandbox). `yolo` ist nur startbar, wenn Sandbox Stufe 2 und Egress-Proxy aktiv sind.
- **Details:** Claude: `--permission-mode plan|default|acceptEdits|bypassPermissions` bzw. Control-Request `set_permission_mode` (live). Codex: `approval_policy` bleibt in allen Modi mindestens `untrusted`, damit Approval-Requests das Gate erreichen; im Modus `yolo` beantwortet beton diese Requests automatisch gemäß Policy (ohne menschliche Rückfrage, außer bei `ask`-Regeln), `plan` setzt die Codex-Sandbox auf `read-only`. ACP: `session/set_mode`, falls angeboten. Policies gelten in jedem Modus unverändert: Im Modus `yolo` entfällt nur die Vendor-Rückfrage, nicht die beton-Policy-Prüfung.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton run claude --permission-mode yolo` ohne verfügbare Sandbox bricht mit Fehler `sandbox_required` ab (kein unsandboxed Start).
  - [ ] AC2 — (ab M2) Im Modus `yolo` wird eine `deny`-Policy auf `git push --force` weiterhin durchgesetzt (Golden-/Fake-Test).
  - [ ] AC3 — Ein Moduswechsel während der Session erzeugt `session.settings_changed { permission_mode, mechanism }` (PROTO-002).
- **Abhängigkeiten:** HAR-004, HAR-006, HAR-007, SBX-006 (ab M2), PRX-008 (ab M2, siehe 04-sandbox.md)

## Nicht in v1

Folgende Harnesses und Funktionen sind ausdrücklich **nicht** Teil von v1 (ADR-0006). Sie können über ACP (HAR-007) oder Out-of-Process-Plugins (PLG-002, siehe 10-runners-extensibility.md) durch die Community ergänzt werden:

- Dedizierte Adapter für **Pi, GitHub Copilot, Cursor, OpenCode, Devin, Kiro, Kimi, Hermes, Antigravity/Gemini nativ** (Gemini CLI, Goose, Qwen sind über ACP abgedeckt).
- In-Process-Vendor-SDKs (Claude Agent SDK, OpenAI Agents SDK) – beton spricht die CLI-Protokolle direkt.
- **Smart Routing** / "Auto"-Harness-Wahl und lernender Router (v2, ADR-0022); in v2 subscription-first über eine eingeloggte Vendor-CLI bzw. den Harness der Session, API-Key nur optional (ADR-0034).
- Eigener OAuth-Login oder Speicherung von Subscription-Tokens (dauerhaft ausgeschlossen, ADR-0005).
- OpenAI-Responses-API als Wire-Format des Direkt-API-Harness (v2; v1 nutzt Chat Completions und Anthropic Messages).
- Automatische Capability-Verifikation per Probe-Benchmark (Omnigent `harness_bench`) – v2; v1 deklariert Capabilities statisch + Golden-Tests.
