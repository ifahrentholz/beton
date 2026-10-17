<!-- Generiert von `cargo xtask codegen` aus beton_policy::vars. Nicht von Hand ändern. -->

# Policy-Kontext (CEL-Variablen)

Variablen, die CEL-Ausdrücke in Policies sehen, mit Typ und Phasen. Spezifikation: POL-004 in [03-policies.md](../spec/03-policies.md#pol-004--kontext-variablen-je-phase). Nicht gesetzte Felder sind `null`; eine Variable außerhalb ihrer Phasen ist ein Ladefehler.

| Variable | Typ | Phasen | Beschreibung |
| --- | --- | --- | --- |
| `now` | `timestamp` | alle | Zeitpunkt der Auswertung. |
| `session.attached_clients` | `int` | alle | Verbundene Clients. |
| `session.branch` | `string` | alle | Git-Branch des Arbeitsverzeichnisses. |
| `session.cost_usd` | `double` | alle | Kosten der Session in USD aus der Usage-Projektion (ohne Subscriptions). |
| `session.depth` | `int` | alle | Tiefe im Sub-Agent-Baum (0 = Wurzel). |
| `session.duration_s` | `double` | alle | Laufzeit in Sekunden. |
| `session.equivalent_usd` | `double` | alle | Listenpreis-Äquivalent der Subscription-Nutzung der Session in USD laut Preis-Katalog; nie Geld (ADR-0034). Budgets zählen `cost_usd + equivalent_usd` (POL-011). |
| `session.harness` | `string` | alle | Harness, z. B. `claude`, `codex`, `acp:gemini`. |
| `session.id` | `string` | alle | ID der Session (`ses_…`). |
| `session.is_async` | `bool` | alle | Asynchroner Lauf (Timer, Schedule, Spawn mit `async`). |
| `session.labels` | `list(string)` | alle | Labels der Session. |
| `session.mode` | `string` | alle | `native` oder `tui`. |
| `session.model` | `string` | alle | Aktuelles Modell. |
| `session.parent_id` | `string` | alle | Eltern-Session eines Sub-Agents. |
| `session.permission_mode` | `string` | alle | Permission-Mode (`plan`, `default`, `accept_edits`, `yolo`). |
| `session.project` | `string` | alle | Projekt der Session. |
| `session.project_root` | `string` | alle | Projektverzeichnis (absolut): nächstes Verzeichnis mit `.beton/config.yaml`, sonst die Git-Wurzel des Arbeitsverzeichnisses (CLI-008); `${project_root}` in `path_guard`. |
| `session.reasoning_effort` | `string` | alle | Aktueller Reasoning-Effort. |
| `session.started_at` | `timestamp` | alle | Start der Session. |
| `session.title` | `string` | alle | Titel der Session. |
| `session.tokens_in` | `int` | alle | Eingabe-Tokens bisher. |
| `session.tokens_out` | `int` | alle | Ausgabe-Tokens bisher. |
| `session.tool_calls` | `int` | alle | Tool-Calls bisher. |
| `session.turns` | `int` | alle | Abgeschlossene oder laufende Turns. |
| `session.unpriced_models` | `list(string)` | alle | Modelle der Session, deren Nutzung weder Geld noch Listenpreis-Äquivalent hat (ohne Katalogpreis); Budgets fragen dann nach (`on_unpriced`, POL-011). |
| `session.worktree` | `string` | alle | Arbeitsverzeichnis (absolut). |
| `run.budget_usd` | `double` | alle | Budget des Runs in USD. |
| `run.cost_usd` | `double` | alle | Kosten des Runs in USD. |
| `run.duration_s` | `double` | alle | Laufzeit in Sekunden. |
| `run.equivalent_usd` | `double` | alle | Listenpreis-Äquivalent der Subscription-Nutzung des Runs in USD (ADR-0034). |
| `run.id` | `string` | alle | ID des Runs. |
| `run.subtree_cost_usd` | `double` | alle | Kosten des Runs inklusive aller Child-Sessions. |
| `run.subtree_equivalent_usd` | `double` | alle | Listenpreis-Äquivalent des Runs inklusive aller Child-Sessions in USD (ADR-0034). |
| `run.subtree_unpriced_models` | `list(string)` | alle | Modelle ohne Preis im Run inklusive aller Child-Sessions (POL-011). |
| `run.trigger` | `string` | alle | `interactive`, `spawn`, `timer`, `schedule`, `webhook`, `fork` oder `import`. |
| `run.turns` | `int` | alle | Turns des Runs. |
| `run.unpriced_models` | `list(string)` | alle | Modelle des Runs ohne Preis (POL-011). |
| `user.daily_cost_usd` | `double` | alle | Kosten des Users heute (00:00 in `user.timezone`) in USD über alle Sessions. |
| `user.daily_equivalent_usd` | `double` | alle | Listenpreis-Äquivalent der Subscription-Nutzung des Users heute in USD (ADR-0034). |
| `user.daily_unpriced_models` | `list(string)` | alle | Modelle ohne Preis, die der User heute genutzt hat (POL-012). |
| `user.email` | `string` | alle | E-Mail-Adresse (nur mit Identität, ab M4). |
| `user.id` | `string` | alle | ID (`usr_…`). |
| `user.monthly_cost_usd` | `double` | alle | Kosten des Users im laufenden Monat in USD. |
| `user.roles` | `list(string)` | alle | Rollen. |
| `user.teams` | `list(string)` | alle | Teams (ab M4). |
| `user.timezone` | `string` | alle | Zeitzone (IANA), Default `UTC`; bestimmt die Tagesgrenze. |
| `org.id` | `string` | alle | ID (`org_…`). |
| `team.ids` | `list(string)` | alle | IDs der Teams. |
| `agent.hash` | `string` | alle | Inhalts-Hash des Agent-Snapshots. |
| `agent.name` | `string` | alle | Name des Agents. |
| `agent.source` | `string` | alle | Herkunft (`builtin`, `project`, `user`, Pfad). |
| `agent.version` | `string` | alle | Version des Agents. |
| `host.arch` | `string` | alle | Architektur (`aarch64`, `x86_64`). |
| `host.home` | `string` | alle | Home-Verzeichnis des Users auf dem Host (absolut); `~` und `${home}` in `path_guard`. |
| `host.id` | `string` | alle | ID (`hst_…`). |
| `host.labels` | `list(string)` | alle | Labels des Hosts. |
| `host.local` | `bool` | alle | Lokaler Host (derselbe Rechner wie der Server). |
| `host.os` | `string` | alle | Betriebssystem (`macos`, `linux`, `windows`). |
| `actor.id` | `string` | alle | ID des Akteurs. |
| `actor.kind` | `string` | alle | `user`, `agent` oder `system`. |
| `budget.lease_expires_at` | `timestamp` | alle | Ablauf der Lease. |
| `budget.lease_remaining_usd` | `double` | alle | Restbetrag der Lease in USD. |
| `budget.online` | `bool` | alle | Verbindung zum Server. |
| `state.run` | `map(string, dyn)` | alle | Zustand des Runs. |
| `state.session` | `map(string, dyn)` | alle | Zustand der Session. |
| `state.user` | `map(string, dyn)` | alle | Zustand des Users. |
| `state.user_day` | `map(string, dyn)` | alle | Zustand des Users für den aktuellen Tag. |
| `approvals.denied_in_session` | `int` | alle | Abgelehnte Freigaben in dieser Session. |
| `approvals.granted_in_session` | `int` | alle | Erteilte Freigaben in dieser Session. |
| `start.agent_ref` | `string` | `session_start` | Agent-Referenz (`builtin:<name>`, Pfad). |
| `start.fork_of` | `string` | `session_start` | Quelle eines Forks. |
| `start.harness` | `string` | `session_start` | Harness der Session. |
| `start.model` | `string` | `session_start` | Angefragtes Modell (modifizierbar). |
| `start.network.proxy_enabled` | `bool` | `session_start` | Egress-Proxy aktiv. |
| `start.permission_mode` | `string` | `session_start` | Angefragter Permission-Mode (modifizierbar, nur strenger). |
| `start.reasoning_effort` | `string` | `session_start` | Angefragter Effort (modifizierbar). |
| `start.sandbox.backend` | `string` | `session_start` | Backend (`seatbelt`, `landlock`, `none`, …). |
| `start.sandbox.enabled` | `bool` | `session_start` | Tool-Sandbox aktiv. |
| `start.sandbox.profile` | `string` | `session_start` | Profil der Sandbox. |
| `start.trigger` | `string` | `session_start` | `interactive`, `spawn`, `timer`, `schedule`, `webhook`, `fork`, `resume` oder `import`. |
| `start.worktree` | `string` | `session_start` | Arbeitsverzeichnis. |
| `request.context_used_pct` | `double` | `model_request` | Füllstand des Kontextfensters in Prozent. |
| `request.estimated_input_tokens` | `int` | `model_request` | Geschätzte Eingabe-Tokens. |
| `request.granularity` | `string` | `model_request` | `call` (jeder Model-Request, Direkt-API) oder `turn` (vor jedem Turn). |
| `request.messages` | `list(dyn)` | `model_request` | Nachrichten des Requests (nur Direkt-API). |
| `request.model` | `string` | `model_request` | Modell des Requests (modifizierbar). |
| `request.prompt` | `string` | `model_request` | Letzte User-Nachricht (höchstens 64 KiB). |
| `request.provider` | `string` | `model_request` | Anbieter (`anthropic`, `openai`, …), soweit bekannt. |
| `request.reasoning_effort` | `string` | `model_request` | Effort (modifizierbar). |
| `request.tools` | `list(string)` | `model_request` | Namen der angebotenen Tools. |
| `tool.args` | `map(string, dyn)` | `tool_call`, `tool_result` | Argumente (modifizierbar per JSON-Merge-Patch). |
| `tool.argv` | `list(list(string))` | `tool_call`, `tool_result` | Geparste Teilkommandos (an `;`, `&&`, `\|\|`, `\|` getrennt). |
| `tool.call_index` | `int` | `tool_call`, `tool_result` | Laufende Nummer des Calls in der Session. |
| `tool.command` | `string` | `tool_call`, `tool_result` | Shell-Kommando als String. |
| `tool.consecutive_errors` | `int` | `tool_call`, `tool_result` | Fehlgeschlagene Calls in Folge (POL-018). |
| `tool.cwd` | `string` | `tool_call`, `tool_result` | Arbeitsverzeichnis des Calls. |
| `tool.is_subagent_spawn` | `bool` | `tool_call`, `tool_result` | Startet einen Sub-Agent (Claude `Task`, `session_spawn`). |
| `tool.kind` | `string` | `tool_call`, `tool_result` | Kanonische Klasse: `shell`, `file_read`, `file_write`, `file_edit`, `search`, `web_fetch`, `mcp`, `system`, `browser`, `other`. |
| `tool.name` | `string` | `tool_call`, `tool_result` | Tool-Name ohne MCP-Präfix. |
| `tool.native_name` | `string` | `tool_call`, `tool_result` | Name beim Harness, z. B. `Bash`, `commandExecution`, `mcp__github__get`. |
| `tool.parse_uncertain` | `bool` | `tool_call`, `tool_result` | Kommando nicht sicher parsebar (`eval`, `$(…)`, …). |
| `tool.paths` | `list(string)` | `tool_call`, `tool_result` | Betroffene Pfade, absolut und normalisiert. |
| `tool.repeat_count` | `int` | `tool_call`, `tool_result` | Wiederholungen eines identischen Calls im Fenster (POL-018). |
| `tool.server` | `string` | `tool_call`, `tool_result` | MCP-Server oder `builtin`. |
| `tool.urls` | `list(string)` | `tool_call`, `tool_result` | URLs in den Argumenten. |
| `tool.write_paths` | `list(string)` | `tool_call`, `tool_result` | Pfade aus `paths`, in die der Call schreibt: Datei-Tools der Klassen `file_write` und `file_edit`, bei Shell-Kommandos Umleitungsziele und Ziele bekannter Datei-Kommandos (best effort, POL-016). |
| `result.bytes` | `int` | `tool_result` | Größe in Bytes. |
| `result.duration_ms` | `int` | `tool_result` | Dauer in Millisekunden. |
| `result.error` | `string` | `tool_result` | Fehlertext. |
| `result.exit_code` | `int` | `tool_result` | Exit-Code (Shell). |
| `result.ok` | `bool` | `tool_result` | Erfolgreich. |
| `result.text` | `string` | `tool_result` | Text des Ergebnisses (höchstens 64 KiB). |
| `result.truncated` | `bool` | `tool_result` | `text` wurde gekürzt. |
| `browser.host` | `string` | `browser_navigate` | Host. |
| `browser.initiator` | `string` | `browser_navigate` | `agent`, `user` oder `page`. |
| `browser.path` | `string` | `browser_navigate` | Pfad. |
| `browser.profile` | `string` | `browser_navigate` | Browser-Profil. |
| `browser.scheme` | `string` | `browser_navigate` | Schema (`https`, `http`, …). |
| `browser.tab_id` | `string` | `browser_navigate` | Tab. |
| `browser.url` | `string` | `browser_navigate` | Ziel-URL. |
| `browser.action` | `string` | `browser_action` | `click`, `type`, `submit`, `upload`, `download`, `eval` oder `select`. |
| `browser.element_role` | `string` | `browser_action` | ARIA-Rolle des Elements. |
| `browser.file_name` | `string` | `browser_action` | Dateiname bei Upload oder Download. |
| `browser.host` | `string` | `browser_action` | Host der Seite. |
| `browser.is_form_submit` | `bool` | `browser_action` | Die Aktion sendet ein Formular ab. |
| `browser.is_password_field` | `bool` | `browser_action` | Ziel ist ein Passwortfeld. |
| `browser.selector` | `string` | `browser_action` | Selektor des Elements. |
| `browser.text_len` | `int` | `browser_action` | Länge des getippten Texts. |
| `browser.url` | `string` | `browser_action` | URL der Seite. |
