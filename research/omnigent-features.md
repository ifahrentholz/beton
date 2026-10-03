# Omnigent – Feature-Inventar (Grundlage für einen Rust-Neubau)

Stand der Recherche: 2026-10-02. Quellen: Doku auf https://omnigent.ai (alle Seiten der Sidebar inkl. Reference, Releases, Blog; zusätzlich `llms.txt` / `llms-full.txt` und `sitemap.xml`) sowie das GitHub-Repo https://github.com/omnigent-ai/omnigent (Commit `2b7ef45`, Branch `main`, gelesen als Shallow-Clone).

Konventionen:
- **[Doku]** = steht so auf omnigent.ai, **[Code]** = im Repo gelesen, **[Vermutung]** = eigene Ableitung, nicht belegt.
- URL-Kürzel: `D:` = `https://omnigent.ai`, `GH:` = `https://github.com/omnigent-ai/omnigent/blob/main`.

---

## 1. Kurzbeschreibung & Architektur

### 1.1 Was ist Omnigent?

Omnigent nennt sich "open-source meta-harness": eine gemeinsame Schicht über Coding-Agent-Harnesses (Claude Code, Codex, Cursor, OpenCode, Hermes, Pi, Devin, Copilot, Goose, Kimi, Qwen, Kiro, Antigravity, beliebige ACP-Agents) und über eigene, in YAML deklarierte Agents. Die Schicht bietet:
- Austausch/Kombination von Harnesses ohne Umschreiben des Agents (eine Zeile `executor.harness`),
- zustandsbehaftete "Contextual Policies" (ALLOW/ASK/DENY), z.B. Spend Caps, Model Routing, Risk Escalation,
- ein OS-Sandbox ("Omnibox") für Dateisystem, Netzwerk, Umgebungsvariablen und Credential-Brokering,
- persistente, geteilte Live-Sessions mit Kollaboration über Terminal, Web, Desktop, Mobile, Slack und REST API.

Quellen: D:/ , D:/llms.txt, GH:/README.md. Status laut Doku/FAQ und README-Badge: **alpha** ("Is it ready for production? No. Omnigent is alpha." – D:/faq). Rechtsträger ist **Databricks, Inc.** (`pyproject.toml` authors, `NOTICE`, macOS-Signing-Identity "Databricks, Inc."). [Code]

### 1.2 Komponenten

Die Doku nennt drei Komponenten: **Server**, **Runner**, **UI** (D:/docs/deploy/overview). Dazu kommen der **Host** und die **Harness**-Prozesse.

| Komponente | Aufgabe | Quelle |
| --- | --- | --- |
| **Server** | Zentraler Koordinator: Session-Historie (DB), Artifacts (Dateien, Agent-Bundles, Uploads), Katalog registrierter/eingebauter Agent-Specs, MCP-Proxy & serverseitige Policy-Durchsetzung, Skills, Auth & Accounts, Scheduler (Scheduled Tasks), Web-UI-Auslieferung, SSE/WebSocket-Endpunkte. FastAPI/Starlette/uvicorn. | D:/docs/deploy/overview, GH:/deploy/README.md |
| **Host** | Eine beim Server registrierte Maschine (Laptop, Dev-Container, Cloud-Sandbox, K8s-Pod). Läuft als Daemon (`omni host`), hält einen WebSocket-Tunnel zum Server, startet Runner. Kann als User-Service installiert werden (`omni host enable`, macOS/Linux). | D:/docs/deploy/overview, Release 0.12.0 |
| **Runner** | Pro Session ein Prozess, der die Agent-Loop ausführt: verwaltet das Harness, führt Tools aus, streamt Events über WebSocket zurück zum Server. Läuft auf dem Host, d.h. hat Zugriff auf lokale Dateien/Tools/Credentials. | D:/docs/deploy/overview |
| **Harness** | Die eigentliche Agent-Runtime (Vendor-SDK in-process, Vendor-CLI pro Turn, ACP-Subprozess, residentes Vendor-TUI in tmux oder Vendor-Server). Jedes Harness-Modul exportiert `create_app() -> FastAPI`, das der Runner importiert. | D:/docs/build/harnesses/community, GH:/omnigent/harness_capabilities.py |
| **UI** | Web-UI (React-SPA, vom Server ausgeliefert), Terminal (tmux-basiert), Desktop (Electron-Shell), iOS (SwiftUI/WKWebView-Shell), Android (Kotlin/WebView-Shell), VS Code-Extension (iframe), Slack-Bot. UIs sprechen **nur mit dem Server**, nie direkt mit dem Runner. | D:/docs/deploy/overview, GH:/web/*/README.md |

Zitat zur Trennung (D:/): "A runner wraps any agent in a sandboxed, uniform session. A server adds policies and shared history, and exposes every session over the terminal, the web, a native app, mobile, and a REST API."

### 1.3 Prozessmodell (lokal)

[Doku + Code, Details siehe Abschnitt 1.6, sofern durch Code-Analyse ergänzt]
- `omni` / `omnigent` (identische Entry-Points, `pyproject.toml [project.scripts]`) ist das CLI.
- Lokal startet das CLI automatisch einen **lokalen Server** (Default `http://127.0.0.1:6767`; ist der Port belegt, wird der nächste freie genommen) und einen **Host-Daemon**; `omni start` startet beides und registriert die Maschine als Host, `omni stop` stoppt alles. `omni host --background` startet Daemon (+ im Local-Mode auch den Server) detached, druckt URL/PID/Log-Pfad. (D:/docs/interact/web-ui, GH:/README.md)
- Ein bloßes `omni` ohne Argumente startet auf einem interaktiven Terminal Server/Host im Hintergrund und kehrt zurück; ohne TTY druckt es `--help`. (D:/docs/interact/terminal)
- Native-TUI-Harnesses laufen in **tmux** (Pflicht, ab 0.13 tmux ≥ 3.3); das Web-Terminal nutzt tmux Control Mode (CC) als Transport. (Release 0.5.0, 0.13.0)
- MCP-Server werden vom Runner als Subprozesse gestartet (außerhalb des OS-Sandbox). (D:/docs/policies/os-sandbox)
- Lokale PID-Datei `~/.omnigent/local_server.pid` (genutzt von der VS-Code-Extension zur Discovery). (GH:/editors/vscode/README.md)

### 1.4 Datenfluss (vereinfacht)

```
User (Terminal / Web / Desktop / Mobile / Slack / REST)
        │  HTTP(S), SSE (GET /v1/sessions/{id}/stream), WebSockets
        ▼
     SERVER  ── DB (SQLite | Postgres | CockroachDB), Artifact-Store (lokal | S3)
        │  Policies (Server-Level), MCP-Proxy, Scheduler, Auth
        │  WebSocket-Tunnel (Host/Runner wählen sich beim Server ein:
        │  WS /v1/hosts/{host_id}/tunnel, WS /v1/runners/{runner_id}/tunnel)
        ▼
     HOST-Daemon (lokal, Cloud-Sandbox, K8s-Pod)
        │  spawnt pro Session
        ▼
     RUNNER ── Harness (SDK in-process | CLI | ACP | Native TUI in tmux | Native Server)
        │       ├─ LLM-Provider (API-Key, Subscription-CLI, Gateway, Databricks)
        │       ├─ sys_os_* Tools & Terminals  → Omnibox-Sandbox (bwrap/Seatbelt/JobObject)
        │       │                                 + L7-Egress-Proxy + Credential-Proxy
        │       └─ MCP-Server-Subprozesse (unsandboxed)
        ▼
     Events (Responses-artige Stream-Events) → Server → persistiert → an alle verbundenen Clients
```

Wichtige Konsequenz (GH:/deploy/README.md "Execution model"): Das Server-Image ist klein – kein tmux, keine Harness-SDKs, keine LLM-Keys; **kein Agent-Code läuft im Server**. Code und Model-Keys bleiben auf den Hosts.

Event-Typen des SSE-Streams (`ServerStreamEvent`, aus `openapi.json` [Code]): `response.created|in_progress|queued|completed|failed|incomplete|cancelled|error|retry|heartbeat`, `response.output_text.delta`, `response.output_item.done`, `response.output_file.done`, `response.function_call_output.delta`, `response.reasoning.started`, `response.reasoning_text.delta`, `response.reasoning_summary_text.delta`, `response.compaction.in_progress|completed|failed`, `response.elicitation_request|elicitation_resolved`, `response.policy_denied`, `response.client_task.cancel`, `browser.action_request`, `session.created|status|title|todos|usage|model|model_options|reasoning_effort|permission_mode|collaboration_mode|codex_approval_mode|agent_changed|child_session.updated|input.consumed|interrupted|presence|heartbeat|superseded|sandbox_status|mcp_startup|resource_created|resource_deleted|terminal_activity|terminal_pending|changed_files.invalidated|btw_sidechat`, `turn.started|completed|failed|cancelled`. Das Schema ist erkennbar an die **OpenAI Responses API** angelehnt (Item-/Delta-Events). [Code; die Ableitung "angelehnt an" ist Vermutung]

### 1.5 Konfigurationsebenen

| Ebene | Datei/Befehl | Zweck | Quelle |
| --- | --- | --- | --- |
| Projekt | `.omnigent/config.yaml` oder `omni config set KEY=VALUE` | Projekt-Defaults, überschreibt User-Werte (Deep-Merge bei Harness-Overrides) | D:/docs/reference/configuration |
| User | `~/.omnigent/config.yaml` oder `omni config set --global KEY=VALUE` | User-Defaults, Einstellungen des lokalen Servers | dito |
| Deployed Server | `omni server --config /path/config.yaml` (`-c`); Docker: `/data/config.yaml` | Serverweite Settings (Policies, Sandbox-Provider, Branding, LLM, Routing, Attachment-Limits, Auth-Listen) | dito |
| Agent | `config.yaml` im Agent-Verzeichnis oder einzelne `agent.yaml` | Portable Agent-Definition | D:/docs/use/custom-agents |
| Env-Variablen | `OMNIGENT_*` | Server-/Host-Prozess-Settings (Auth, OIDC, DB, Sharing, Features, Telemetrie, Pfade) | diverse Seiten |

`omni config list` zeigt die effektive Konfiguration; `omni config unset [--global] KEY`.

### 1.6 Interne Architektur (Code-Analyse)

Alle Angaben [Code], sofern nicht anders markiert; Pfade relativ zum Repo-Root.

**Prozesse und wer wen startet**
- **CLI** `omnigent.cli:main` (`omnigent/cli.py`, > 9000 Zeilen). CLI-Kontrakt: stdout nur maschinenlesbare Daten, stderr Banner/Spinner/Diagnose (`designs/CLI_CONTRACT.md`).
- **Lokaler Server**: `ensure_local_omnigent_server()` (`omnigent/host/local_server.py`) spawnt einen detached `omnigent server` (FastAPI + uvicorn), bevorzugt Port 6767, sonst freier Loopback-Port; lokal im Accounts-Auth-Modus; Wiederverwendung über `local_server.pid`/`.sig`, Respawn bei geänderter Config-Signatur. Nur der Host-Daemon startet ihn.
- **Host-Daemon** (`omnigent host`, `omnigent/host/connect.py`): öffnet ausgehenden WS zu `/v1/hosts/{host_id}/tunnel`; spawnt pro Session einen Runner bei `host.launch_runner`; nutzt auf POSIX per Default einen Copy-on-write-**Forkserver/Zygote** (`omnigent/runner/_zygote.py`, NDJSON `ping|fork|poll`; `OMNIGENT_RUNNER_ZYGOTE=0` deaktiviert), sonst `Popen`. Reconnect mit exponentiellem Backoff 0.5 s → max 3 s, Jitter 0.5.
- **Runner** (`omnigent/runner/_entry.py`, FastAPI-App `create_runner_app` in `runner/app.py`, ~8k Zeilen): ein Prozess pro Session, wählt sich aus zu `/v1/runners/{runner_id}/tunnel`, wird nie direkt angesprochen; Idle-Timeout 1 h; überwacht Parent via `RUNNER_PARENT_PID`.
- **Harness-Subprozess**: `HarnessProcessManager` (`runtime/harnesses/process_manager.py`) startet lazy pro Konversation `python -m …_runner --harness --module --socket --conversation-id --parent-pid`; das Modul liefert `create_app() -> FastAPI`, das über einen **Unix-Domain-Socket** bedient wird. Die Agent-Loop läuft hier (nicht im Runner).
- **Native TUIs** laufen in tmux-Panes der `TerminalRegistry` (`terminals/registry.py`), Key `(conv_id, terminal_name, session_key)`; Browser-Attach via tmux Control Mode (`tmux -C`, `terminals/control_bridge.py`: `%output` → xterm.js, Eingabe via `send-keys -H`).
- **MCP**: Spec-MCP-Server startet der Runner (`runner/mcp_manager.py`); für Native/ACP-Harnesses gibt es einen stdio-Relay `serve-mcp` (`harnesses/claude_native/bridge.py`), über den Omnigents eigene Tools (`sys_*`) bereitgestellt werden.

**Datenverzeichnis** `~/.omnigent` (`OMNIGENT_DATA_DIR`; `docs/DATA_DIR_LAYOUT.md`): `chat.db` (SQLite), `artifacts/`, `attachments/`, `logs/{cli,host,runner,server}`, `runners/runner_id`, `daemons/`, `auth_tokens.json`, `config.yaml`, `telemetry.json`, `claude-native/`, `codex-native/` … (State-Dirs je Konversations-Digest), `debug/events-<sid>.jsonl`, `local_server.pid`.

**Server-Interna** (`server/app.py` `create_app`, ~4.2k Zeilen): Middleware `AccountAuthorityMiddleware`, `AccountAuthenticationMiddleware`, `WebSocketOriginMiddleware` (CSWSH-Schutz), `BasePathMiddleware`; SPA aus `server/static`. Tunnel-Limits (`util/tunnel_limits.py`): WS-Max-Message 100 MiB, Ping 30 s, Timeout 90 s. Router-Module u.a. `sessions/routes_core.py`, `routes_events.py` (POST events, SSE stream), `routes_items.py`, `routes_elicitations.py`, `routes_hooks.py` (`hooks/permission-request`, `codex-…`, `cursor-…`, `antigravity-…`, `native-…`, `policies/evaluate`, `route-turn`, `route-subagent`), `routes_resources.py`, `routes_agent.py` (Runner holt Bundle über `/agent/contents`), `routes_permissions.py`, `routes_browser.py`, `codex/sessions.py`.
- **SSE**: nur Live-Tail, kein Replay, keine Sequenznummern, Ende mit `data: [DONE]`; Reconnect-Kontrakt: erst Stream abonnieren, dann Snapshot holen, nach Item-ID deduplizieren (`server/API.md`). Event-Schemas = Pydantic-Klassen in `server/schemas.py` (Union `ServerStreamEvent` → `openapi.json`).
- **Persistenz**: SQLAlchemy 2 + Alembic (`db/migrations`, 111 Revisionen). Zwei Declarative Bases: `OmnigentBase` (agents, files, users, preferences, account_tokens, connections, device_grants, session_permissions, omnigent_conversation_metadata, projects, comments, policies, hosts, user_daily_cost, scheduled_tasks, scheduled_task_runs) und `ConversationBase` (conversations, conversation_items, conversation_labels; kann in separater DB liegen). Alle zusammengesetzten PKs beginnen mit `workspace_id` (Multi-Tenancy-fähig). `conversation_items`: `type` (smallint), `data` (JSON als TEXT, zstd-komprimiert), `position`, `response_id`, `search_text` (Volltextsuche).
- **Artifact-Store**: ABC `ArtifactStore` mit `local`, `s3`, `databricks_volumes` (`stores/artifact_store/`); Agent-Bundles als `.tar.gz` mit `config.yaml`.
- **Scheduler**: `server/scheduled/scheduler.py` `ScheduledTaskScheduler` – In-Process-RRULE-Timer, DB = Source of Truth, kein Replay, Overlap-Skip, Misfire-Grace (`fire.py`, `run_reconciler.py`).

**Server↔Runner-Tunnel** (`runner/transports/ws_tunnel/*`, `server/routes/runner_tunnel.py`):
- Runner wählt sich aus; JSON-Text-Frames mit Feld `kind`.
- Handshake `hello`: `runner_version`, `frame_protocol_version` (Major muss 1 sein), `harnesses`, `envs`, `capabilities`, `direct_attach_port/_token`, `connection_id`.
- **HTTP-über-WS**: Server sendet `request` {id, method, path, headers, query_string, body, encoding utf-8|base64, stream}; Runner antwortet `response.head` → n× `response.body` → `response.end` (optional `error`); `request.cancel`. Der Runner dispatcht in seine eigene ASGI-App – das "Protokoll" ist faktisch die **REST-API des Runners**, die die Server-API spiegelt.
- WS-Kanäle: `ws.open`/`ws.frame`/`ws.close` mit `ch_id` (Terminal-Attach; falsche Replica → Close 4400).
- Keepalive `ping`/`pong` {ts} alle 30 s, 3 Misses.
- Event-Ingest-Extension `session-event-ingest-v1`: `event.ready` → `event.batch` {id, session_id, ≤ 32 Events, ≤ 256 KiB} → `event.ack` {applied, error, retryable}; Runner hält Events bis zum Ack.
- Auth: Loopback-Peers dürfen unauthentifiziert sein (Owner `local`); Remote-Runner senden `X-Omnigent-Runner-Tunnel-Token`, `runner_id = runner_token_<sha256("omnigent-runner:"+token)[:32]>` (Mismatch → Close 4004); Managed-Runner holen 30-min-Bearer über `POST /v1/runners/{id}/token`.

**Host-Tunnel** (`host/frames.py`, `routes/host_tunnel.py`): reine Control-Frames – `host.hello` (Version, Name, Runner, `configured_harnesses`, Capabilities), `launch_runner` {request_id, binding_token, workspace, session_id, harness, inference_config} + `_result`, `stop_runner`, `runner_exited`, `runner_status`, `stat`, `list_dir`, Worktree-Ops, `install_harness`, `store_secret`, `detect_credentials`, `fs_request`/`fs_write`, `model_options`, `skills`, `mcp_servers`, `import_local*`. Auth: `Authorization: Bearer` (User-Token) oder `X-Omnigent-Host-Token` (Managed Sandbox).

**Dispatch-Ablauf** (`routes/_sessions/orchestration.py`; teils [Vermutung] des Analyse-Agents): (1) Server mintet Binding-Token, sendet `host.launch_runner`; (2) Runner wählt sich ein, Server bindet `conversations.runner_id`; (3) Server ruft über den Tunnel `POST /v1/sessions` (Session-Init); (4) User-Events werden per `POST /v1/sessions/{id}/events` weitergeleitet; (5) Server hält einen langen `GET /v1/sessions/{id}/stream` zum Runner (15 s `session.heartbeat`, 45 s Read-Timeout), persistiert Items und republiziert auf dem Server-SSE; (6) Native-Forwarder posten stattdessen `external_*`-Events (HTTP oder `event.batch`).

**Session-/Event-Modell** (`entities/conversation.py`): Item-Typen `message`, `function_call`, `function_call_output`, `reasoning`, `error`, `compaction`, `native_tool`, `resource_event`, `routing_decision`, `slash_command`, `terminal_command`; Content-Blöcke `input_text` u.a.; IDs `conv_…`, `ag_…`, `resp_…`. Session-Status `idle|running|waiting|failed`. Gepostete Event-Typen: `message`, `function_call_output`, `interrupt` (umgeht Queue, `cancel_loop`, emittiert `response.incomplete` + `session.interrupted`), `compact`, `stop_session`, intern `external_conversation_item|output_text_delta|session_status|session_usage|session_todos|compaction_status`. Bei Native-Sessions liefert ein Web-`message` eine `pending_id`; der Transcript-Forwarder ist Single Writer. Elicitations nutzen die MCP-`ElicitationResult`-Form (`POST …/elicitations/{id}/resolve`). Fork kopiert Items tief (optional bis `up_to_response_id`), klont die Agent-Zeile, bleibt ungebunden (außer Managed Host); Native-Ziel rekonstruiert seinen Transcript. Compaction serverseitig für In-Process-Harnesses in drei Stufen (`runtime/compaction.py`: alte Tool-Ergebnisse leeren → LLM-Summary → Truncation); Native-Harnesses kompaktieren selbst (Claude bekommt `/compact` injiziert); Imports > 2 MB werden auf letzte Compaction-Grenze gekürzt. Queue lebt clientseitig (localStorage); "Steer" = sofortiges POST (SDK: `enqueue_session_message`, codex-native: RPC `turn/steer`, claude-native: tmux `send-keys`).

**Harness-Schnittstelle**: Modul exportiert `create_app() -> FastAPI`, meist via `HarnessApp` (`runtime/harnesses/_scaffold.py`) oder `ExecutorAdapter` um `inner.executor.Executor` (`run_turn`, `interrupt_session`, `enqueue_session_message`, `supports_*`). Endpoint `POST /v1/sessions/{conv}/events` mit Body `message` (liefert per-Turn-SSE), `interrupt`, `tool_result`, `approval`, `policy_verdict`. `TurnContext`: `emit`, `dispatch_tool` (emittiert `function_call` mit Status `action_required`, parkt bis `tool_result`), `elicit`, `evaluate_policy`, `next_injection`. Der Runner fängt `action_required` ab und führt Tools lokal aus (`runner/tool_dispatch.py`): `sys_os_*` → `OSEnvironment`, Terminal-Tools → `TerminalRegistry`, Datei-/REST-Tools → Server-API, MCP → `RunnerMcpManager`.
- **Native TUI (z.B. Claude Code)**: Eingabe per tmux `send-keys`; Ausgabe durch Tailing der Vendor-Transcript-JSONL + Hook-Records (`harnesses/claude_native/forwarder.py`) → `external_*`-Events; Omnigent-Tools über `--mcp-config` → stdio-`serve-mcp`-Relay → per-Turn-Localhost-Relay → `dispatch_tool`.
- **codex-native**: Codex-App-Server über JSON-RPC/WebSocket (`codex_native/app_server.py`), Approvals → `/hooks/codex-elicitation-request`.
- **ACP** (`inner/acp_harness.py`, `inner/_acp_omnigent_mcp.py`): startet `HARNESS_ACP_COMMAND`, injiziert `serve-mcp` via `session/new.mcpServers`, mappt `session/request_permission` auf Elicitations.
- **Policies auf Native-Tool-Calls**: Claude/Codex-Command-Hooks `PreToolUse`/`PostToolUse`/`UserPromptSubmit` (`native/native_policy_hook.py`, `claude_native/hook.py`) posten an `/v1/sessions/{id}/policies/evaluate`; fail-closed bei `tool_call`/`request`, Retry-Budget 30 s, Long-Poll während ASK; `PermissionRequest`-Hooks → `hooks/permission-request`.

**Policy-Engine-Ort**: Server `runtime/policies/{engine,builder,approval}.py` (`PolicyEngine`, Label-State in `conversation_labels`, Server besitzt ASK-Elicitations); Runner `runner/policy.py` (Fast-Path für Function-Policies bei MCP-Dispatch, eskaliert ASK zum Server); Harnesses fordern Evaluation über `policy_evaluation.requested` an, `runner/policy_proxy.py` leitet an `/policies/evaluate` weiter. Policy-Typen `function` und `prompt` (LLM-Klassifikator); Phasen-Selektoren `tool_call:<name>`.

**Auth-Interna** (`server/auth.py` `resolve_auth_source()`): `OMNIGENT_AUTH_PROVIDER` überschreibt alles; sonst Default **`header`** (vertraut `X-Forwarded-Email`, konfigurierbar `OMNIGENT_AUTH_HEADER`); mit `OMNIGENT_AUTH_ENABLED` → `oidc` (wenn `OMNIGENT_OIDC_ISSUER`) sonst `accounts`; Loopback-Single-User `local`. Zugriffslevel READ=1, EDIT=2, MANAGE=3, OWNER=4. Session-JWT HS256 mit `*_COOKIE_SECRET` (≥ 32 Byte), aus Cookie oder `Authorization: Bearer`. OIDC-CLI-Login `/auth/cli-login` + `/auth/cli-poll`. Device Grant nur Accounts-Mode. Client Credentials: `OMNIGENT_MACHINE_CLIENT_ID`, `_SECRET_HASH` (HMAC-SHA256), `_SUB`, `_TTL` (≤ 3600 s). Token-Matrix: Device-Grant (scope+grant_id, Pfad-Allowlist, Revocation-Check), Login-Grant (grant_id, volle Rechte, Revocation-Check), Machine (scope, Pfad-Allowlist, kein Revocation-Lookup).

**Telemetrie-Interna**: `telemetry/client.py` – fire-and-forget Async-Queue, POST `{"records":[{data:{event_name,…}, partition-key}]}` an Ingestion-URL; Opt-out zusätzlich `DISABLE_TELEMETRY` und jede CI-Umgebung; `installation_id` in `telemetry.json`.

**Feature-Flags**: `Feature`-Enum `usage_page`, `harness_install`, `canvas`, `customize`; unbekannte Namen brechen den Start ab; sichtbar in `/v1/info.features` (`server/feature_flags.py`).

**SDKs**: `sdks/python-client/omnigent_client` (`OmnigentClient`, `session()`, `query()`, `sessions_chat`; drei Schichten: rohe typisierte SSE-Events → `BlockStream`-Semantikblöcke → Transforms `pipe`, `skip_blocks`); `sdks/ui/omnigent_ui_sdk` (Terminal-UI mit Rich/prompt_toolkit); `sdks/web-extension` (`@omnigent/extension-sdk`, iframe-seitige MessageChannel-API). Extensions: `EXTENSION_API_VERSION=1`, Permissions `navigation`, `projects.*`, `sessions.read`, `storage.user`.

**Komplexitäts-Hotspots** (Zeilenzahlen laut Analyse): `runner/app.py` (~8k), `runner/native/orchestration.py` (~10k), `harnesses/*` (Native-Glue), `server/app.py` (~4.2k), `cli.py` (> 9k).

---

## 2. Tech-Stack des Originals & Lizenz

### 2.1 Sprachen (GitHub Languages API, Bytes)

Python 60.9 MB, TypeScript 13.5 MB, JavaScript 1.3 MB, Swift 434 KB, Kotlin 193 KB, Rust 175 KB, HTML/CSS, Shell, Dockerfile, Ruby (fastlane), Just. [Code]
- **Rust** existiert nur als Dev-Tooling: `dev/omnidev` ("Per-repo dev pod supervisor TUI", ratatui/crossterm/tokio) und `tests/codex_parity/sidecar`. Das Produkt selbst ist **nicht** in Rust. [Code]

### 2.2 Backend / CLI / Runner (Python ≥ 3.12)

Wichtigste Abhängigkeiten aus `pyproject.toml` (Version `0.17.0.dev0` auf main) [Code]:
- Web/API: `fastapi`, `starlette`, `uvicorn[standard]`, `websockets<15` (gepinnt wegen macOS-Hänger), `httpx`.
- CLI/TUI: `click`, `rich`, `prompt_toolkit`; POSIX-Terminal-Stack `pexpect`, `pyte` (nicht auf Windows).
- Persistenz: `sqlalchemy>=2`, `alembic` (Migrationen), `zstandard` (clientseitige Kompression von Text-/JSON-Spalten), Treiber als Extras: `psycopg[binary]` (Postgres), `sqlalchemy-cockroachdb`, `pymysql` (Databricks-Extra; MySQL wird beim Pool-Sizing erwähnt).
- Agent-SDKs (Basis): `claude-agent-sdk`, `openai-agents`, `openai`, `mcp`; optional `github-copilot-sdk`, `cursor-sdk`, `google-antigravity`.
- Policies: `cel-python` (CEL, hängt an `google-re2` C++-Extension).
- Security/Auth: `PyJWT[crypto]`, `argon2-cffi` (Passwort-Hashing), `keyring` (OS-Keychain für Provider-Keys, Fallback 0600-Datei), `certifi`.
- Sonstiges: `tiktoken`, `protobuf` (Routing-API `omnigent/api/routing/v1/routing.proto`), `python-dateutil` (RFC-5545-RRULE), `tzdata` (Windows), `pillow` (Branding-Asset-Validierung), `tomlkit`, `json5` (OpenClaw-Registry), `psutil`, `opentelemetry-api` (+ optional SDK/Exporter, Extra `tracing`).
- Extras: Modell-Provider `databricks`, `bedrock`, `vertex`; Sandbox-Provider `modal`, `daytona`, `blaxel`, `boxlite`, `microsandbox`, `cwsandbox`, `e2b`, `islo`, `openshell`, `kubernetes`; Storage `s3`, `kms`, `vault`; `hindsight`/`memory`, `nimble`, `slack` (separates Paket `omnigent-slack`), `dictation` (`sherpa-onnx`, `numpy`), `postgres`, `cockroachdb`, `loadtest` (locust).
- Separat versionierte, im Lockstep veröffentlichte Pakete: `omnigent-client` (Python SDK, Import `omnigent_client`), `omnigent-ui-sdk`, `omnigent-slack`. (`sdks/`, `integrations/slack`)
- Plugin-Mechanismus: Python **Entry Points** (`omnigent.community.harness`, Sandbox-Provider-Gruppe, `omnigent.extensions`). [Doku+Code]

### 2.3 Web-UI (TypeScript)

`web/` – Vite + React + TypeScript, pnpm. Auswahl der Dependencies [Code]: `react`, `react-router-dom`, `@tanstack/react-query`, `@tanstack/react-virtual`, `zustand`, `tailwindcss` v4, `radix-ui`, `shadcn`, `antd`, `cmdk` (Command Palette), `monaco-editor`/`@monaco-editor/react` + `shiki` (Code-Editor, Highlighting), `@xterm/xterm` (+webgl/fit/web-links Addons) für Terminals, `@tiptap/*` (Rich-Text/Markdown-Editor), `streamdown` (+mermaid/math/code/cjk) und `react-markdown`/`remark-gfm`/`rehype-*` für Markdown-Rendering, `katex`, `pdfjs-dist`/`react-pdf`, `@pierre/diffs` (Diffs), `@xyflow/react` (Sub-Agent-Graph, Canvas), `three`, `recharts` (Usage-Page), `rrule`, `@dnd-kit/*`, `zod`, `react-hook-form`, OpenTelemetry-Web-Tracing. Tests: vitest, testing-library, Storybook, Playwright (Python-seitig). Lint: oxlint, prettier.
- Die SPA ist **PWA** (installierbar, ohne Offline-Modus). (D:/docs/interact/web-ui)
- Native Shells erkennen sich via `window.omnigentNative` (`kind: "android"` etc.; `web/src/lib/nativeBridge.ts`). (GH:/web/android/README.md)

### 2.4 Native Clients

| Client | Technik | Quelle |
| --- | --- | --- |
| Desktop (macOS/Linux/Windows) | **Electron** (`electron ^42`, `electron-builder ^26`, `electron-updater`), dünne Shell, lädt die SPA vom Server. appId `ai.omnigent.desktop`, Targets: macOS dmg+zip (x64/arm64, hardened runtime), Linux AppImage+deb, Windows NSIS. Auto-Update über `https://omnigent.ai/_desktop/updates/` (generic provider). URL-Scheme `omnigent://`. | GH:/web/electron/package.json, README |
| iOS | SwiftUI + WKWebView-Shell (Xcode 26, iOS 26), fastlane | GH:/web/ios/README.md |
| Android | Kotlin + WebView-Shell (compileSdk/targetSdk 36, minSdk 28), Bridge via `WebViewCompat.addWebMessageListener` (origin-allowlisted) | GH:/web/android/README.md |
| VS Code | Minimal-Extension, iframe auf lokalen Server | GH:/editors/vscode/README.md |

Bemerkenswert für einen Rust/Tauri-Neubau: Das Electron-README begründet Electron u.a. damit, dass es File-Drops nicht abfängt "the way Tauri does by default", und dokumentiert, dass Web Speech in Electron nicht transkribiert (daher serverseitiges Diktat). [Code]

### 2.5 Storage & Protokolle

- DB: SQLite (Default/Lite), PostgreSQL (empfohlen, Pflicht bei >1 Server-Instanz), CockroachDB (≥ v23.2.28, READ COMMITTED). Gleiches Schema/gleiche Alembic-Migrationen. Pool-Defaults 200 + 20 Overflow, 10 s Timeout (`OMNIGENT_DB_POOL_*`). (D:/docs/deploy/database)
- Artifact-Store: lokal (`/data/artifacts/...`) oder S3-kompatibel (`OMNIGENT_ARTIFACT_URI=s3://bucket/prefix`, AWS S3/R2/MinIO). [Code, pyproject-Kommentar]
- Credential Store (verschlüsselt at rest): AWS KMS oder HashiCorp Vault Transit. (D:/docs/deploy/credential-store)
- Protokolle: HTTP/REST (`/v1/*`, OpenAPI 3, 89–90 Pfade öffentlich dokumentiert), **SSE** für Session-Streams, **WebSockets** (Host-/Runner-Tunnel, Terminal-Attach, Session-Updates, Diktat-Stream), **MCP** (stdio und HTTP/SSE), **ACP** (Agent Client Protocol über stdio, JSON-RPC), OAuth 2.0 (Device Grant RFC 8628, Client Credentials, OIDC, PKCE), Protobuf/JSON für die externe Routing-API, RFC 5545 RRULE.
- Default-Port **6767**.

### 2.6 Lizenz

**Apache License 2.0** (`LICENSE`, GitHub-Metadaten `Apache-2.0`), Copyright 2026 Databricks, Inc.; `NOTICE` listet enthaltenen Fremdcode (u.a. openai, databricks-mcp/sdk, mlflow-tracing, ftfy, OpenTelemetry-Exporter). Beiträge laufen über DCO (`DCO`-Datei). Repo-Kennzahlen am 2026-10-02: ~10.4k Stars, ~1.7k Forks, ~1.6k offene Issues/PRs, erstellt 2026-06-11. [Code/GitHub API]

---

## 3. Feature-Katalog

### 3.1 Harness-Adapter

**Begriff**: Ein Harness ist "the runtime that executes your agent loop". Tools, Policies, Prompts und Models bleiben beim Wechsel des Harness gleich. (D:/docs/build/harnesses)

**Ausführungsmodi** (D:/docs/build/harnesses/supported):
- **Direct** – Omnigent treibt Model und Tools selbst (volle Plattform: Web-UI, Streaming, Policies, Persistenz, Mobile).
- **Native TUI** – Omnigent bootet das Vendor-TUI in einem Pane (tmux) und spiegelt es zurück; IDs enden auf `-native`.

**Support-Stufen**: Fully supported / Maintained / Community-supported (beschreibt Wartung, nicht Funktionsfähigkeit).

**Harness-Tabelle** (D:/docs/build/harnesses/supported):

| Harness | IDs (Aliase) | Direct | Native TUI | Support |
| --- | --- | --- | --- | --- |
| Claude Code | `claude-sdk` (`claude`), `claude-native` | ja | ja | Fully supported |
| Codex | `codex`, `codex-native` | ja | ja | Fully supported |
| OpenAI Agents SDK | `openai-agents` (`openai-agents-sdk`) | ja | nein | Fully supported |
| Copilot | `copilot` (`github-copilot`) | ja | nein | Maintained |
| Cursor | `cursor`, `cursor-native` | ja | ja | Maintained |
| OpenCode | `opencode-native` (`opencode`) | nein | ja | Maintained |
| Pi | `pi`, `pi-native` | ja | ja | Maintained |
| Antigravity (Gemini, `agy`) | `antigravity` (`agy`, `google-antigravity`), `antigravity-native` (`agy-native`) | ja | ja | Community |
| Devin | `devin-native` (`devin`, `native-devin`; `devin-acp` seit 0.14 entfernt/alias) | nein | ja | Community |
| Hermes | `hermes`, `hermes-native` | ja | ja | Community |
| Kimi | `kimi` (`kimi-code`), `kimi-native` | ja | ja | Community |
| Kiro | `kiro-native` | nein | ja | Community |
| Qwen Code | `qwen` (`qwen-code`), `qwen-native` | ja | ja | Community |
| Rovo Dev | `rovo` (`rovo-cli`) – externes Plugin `shbhmrzd/omnigent-rovo` | ja | nein | Community |
| Goose (via ACP) | `goose`, `goose-native` | ja | ja | – |
| Grok Build (via ACP) | `grok` (`grok-build`) | ja | nein | – |
| Jcode (via ACP) | `jcode` | ja | nein | – |
| Generischer ACP-Agent | `acp:<slug>` | ja | nein | – |
| (intern) Open Responses | `open-responses` | – | – | nur im Code gesehen |

**Capability-Matrix** (deklariert in `GH:/omnigent/harness_plugins.py::_BUILTIN_CAPABILITIES`, Datentyp `HarnessCapabilities` in `GH:/omnigent/harness_capabilities.py`, ausgeliefert über `GET /v1/harnesses`) [Code]:

Achsen: `integration_mode` (`sdk-in-process`, `cli-subprocess`, `acp-subprocess`, `native-tui`, `native-server`), `elicitation` (`none`, `hook` = Vendor-PreToolUse-Hook postet an Omnigent, `jsonrpc` = Codex-App-Server-Elicitation, `approval-mirror` = TUI-Approval-Pane wird gepollt und ins Web gespiegelt, `sse-permission` = Permission-Events über SSE/ACP), `resume` (`none`, `warm-reattach`, `cold-only`), `effort` (anthropic/openai/gemini/copilot/pi/codex-native), `model_family` (claude/gpt/gemini/multi), `auth` (`omnigent-credential`, `own-auth`, `session-scoped-config`), `subagents`, `interrupt`, `streaming`, `steering`, `live_queue`, `images`, `compaction`, `fork_history` (`none`, `rebuild` = Vendor-Session-Datei aus kopierten Items neu bauen, `preamble` = alte Turns als Text-Präambel), `instruction_delivery` (`composed-per-turn`, `composed-session-snapshot`, `agent-startup-additive`, `first-user-prefix`, `not-delivered`).

| Harness | Mode | Elicitation | Resume | Auth | Fork-History | Sub-Agents | Instructions |
| --- | --- | --- | --- | --- | --- | --- | --- |
| claude-native | native-tui | hook | warm | omnigent-cred | rebuild | ja | startup-additive |
| codex-native | native-tui | jsonrpc | warm | omnigent-cred | rebuild | ja | startup-additive |
| pi-native | native-tui | none | warm | session-scoped | rebuild | nein | not-delivered |
| cursor-native | native-tui | approval-mirror | warm | own-auth | preamble | nein | not-delivered |
| kiro-native | native-tui | approval-mirror | warm | own-auth | none | nein | not-delivered |
| antigravity-native | native-tui | none | warm | own-auth | none | nein | not-delivered |
| goose-native | native-tui | approval-mirror | warm | own-auth | none | nein | not-delivered |
| qwen-native | native-tui | approval-mirror | warm | own-auth | rebuild | nein | not-delivered |
| kimi-native | native-tui | hook | warm | session-scoped | none | nein | not-delivered |
| opencode-native | native-server | sse-permission | warm | own-auth | preamble | ja | per-turn |
| devin-native | native-tui | hook | warm | own-auth | preamble | ja (steering ja) | startup-additive |
| hermes-native | native-tui | approval-mirror | warm | own-auth | rebuild | nein | not-delivered |
| claude-sdk | sdk-in-process | none | cold | omnigent-cred | – | – | session-snapshot |
| codex | cli-subprocess | jsonrpc | warm | omnigent-cred | – | – | per-turn |
| pi | cli-subprocess | none | cold | omnigent-cred | – | – | per-turn |
| openai-agents | sdk-in-process | none | cold | omnigent-cred | – | – | per-turn |
| cursor | sdk-in-process | none | warm | own-auth | – | – | first-user-prefix |
| antigravity | sdk-in-process | none | cold | own-auth | – | – | per-turn |
| acp / goose / qwen | acp-subprocess | sse-permission | cold | own-auth | – | – | first-user-prefix |
| kimi | cli-subprocess | none | warm | session-scoped | – | – | not-delivered |
| hermes | cli-subprocess | hook | cold | own-auth | – | – | first-user-prefix |
| copilot | sdk-in-process | none | cold | own-auth | – | – | per-turn |

**Harness-spezifische Details** (D:/docs/build/harnesses/configuration, GH:/docs/AGENT_YAML_SPEC.md):
- **Claude Code**: `claude-sdk` nutzt `claude-agent-sdk` in-process; `claude-native` wrappt das `claude`-TUI. Permission-Modi im Composer (`--permission-mode`, z.B. Accept edits, Bypass, Plan, Auto); seit 0.11 live umschaltbar (inkl. Shift+Tab im Terminal). `/rename` im TUI synchronisiert den Session-Namen. Claude-native MCP-Helper laufen seit 0.16 über Unix-Socket statt Loopback-Port. Skills-Frontmatter `disable-model-invocation`/`allowed-tools` werden nur in `claude-native` honoriert.
- **Codex**: Approval-Presets Default / Full access (`--sandbox danger-full-access --ask-for-approval never`) / Read only / Bypass (`--dangerously-bypass-approvals-and-sandbox`, nur bei Erstellung). Native `/side`-Fork (ephemeral, In-Process). Codex-Skills und Codex-Plugins (`codex plugin list --json`, Namespace `<plugin>:<skill>`) werden im Skill-Menü angezeigt. Goal-Mode (`/goal`) via REST `/v1/sessions/{id}/codex_goal`.
- **Copilot**: GitHub-Copilot-SDK (Extra `copilot`, bündelt Copilot-CLI). Token-Auflösung: `COPILOT_GITHUB_TOKEN`/`GH_TOKEN`/`GITHUB_TOKEN` → in `omnigent setup` gespeicherter Token → `gh auth token`. GHE-Host via `copilot.github_host` (→ `COPILOT_GH_HOST`). `omni copilot` = `omni run --harness copilot`.
- **Grok Build**: `grok agent stdio` über ACP; eigene Auth (`grok login --device-auth`), `/model` wird abgelehnt; `XAI_API_KEY` nur via `OMNIGENT_RUNNER_ENV_PASSTHROUGH`.
- **Devin**: native TUI; `--model` (Familien-Slug) + `--effort low|medium|high|xhigh|max` werden zum Vendor-Modell-ID rekombiniert; `--permission-mode normal|accept-edits|smart|dangerous|bypass`, `--sandbox`, `-p`, `--resume`. Devins Sub-Agents erscheinen als Child-Sessions; Consent-Prompts werden als Chat-Approval-Cards gespiegelt.
- **Jcode**: `jcode acp`; unterstützt kein session-scoped MCP → Omnigent-Builtins werden nicht bereitgestellt.
- **Hermes**: eigene file-basierte Auth unter `HERMES_HOME`; `/model` wird via `HARNESS_HERMES_MODEL` durchgereicht; Sandbox/Workspace greifen.
- **Kimi**: Curl-Installer, `kimi login` → `~/.kimi-code/credentials/kimi-code.json`; Headless via `kimi --print --output-format stream-json` pro Turn; Gateway via `HARNESS_KIMI_GATEWAY_BASE_URL/_API_KEY`.
- **Antigravity**: `antigravity-native` wrappt `agy` (≥ 1.1.13); `GEMINI_API_KEY` hat Vorrang vor OAuth (`~/.gemini/oauth_creds.json`, Linux-Tokenpfad, macOS Keychain). SDK-Variante `antigravity` (Default-Modell Gemini 3.5 Flash, auch Vertex AI).
- **Pi**: "headless multi-model worker", läuft gegen jedes Gateway-Modell; Omnigent schreibt ein managed `models.json` + per-Session-Settings; kuratierte `/model`-Shortlist aus `providers.<name>.openai.models`-Tier-Map (Aliase, `[...]`-Suffix-Stripping, `enabledModels`); `OMNIGENT_PI_ENV_UNSET` entfernt fremde Credential-Variablen; Agent-Optionen `executor.context_files: false`, `executor.system_prompt_mode: append|replace`; `/effort` mid-session.
- **Cursor**: `cursor` (SDK) und `cursor-native` (`cursor-agent`); nur Cursor-Backend (keine Gateways), `CURSOR_API_KEY`; Exec-Modi (Plan, `--yolo`, Smart Auto `--auto-review`).
- **Kiro**: `kiro-cli` + Kiro-Login; TUI bleibt autoritative Approval-Oberfläche, einmalige Approvals werden als Chat-Cards gespiegelt; Credit-Usage als Session-Cost.
- **OpenCode**: `opencode-native` als "native-server" (runner-owned Vendor-Server + HTTP/SSE-Bridge).
- **Qwen**: `qwen --acp`.
- **Generic ACP** (`acp:<slug>`): Registrierung via `omnigent setup` → Custom ACP agent (Name, Launch-Command, optional Model), gespeichert unter `acp.agents` in `~/.omnigent/config.yaml`. Omnigent rendert Streaming/Reasoning/Tool-Cards, routet Permission-Requests durch Policies als Elicitation-Cards, unterstützt Interrupts und stellt über eine **MCP-Relay-Bridge** eigene Builtin-Tools (Session, Sub-Agent, Skill, Policy) bereit (`OMNIGENT_ACP_MCP=0` bzw. `omnigent_mcp: false` deaktiviert). `OMNIGENT_ACP_ENV_UNSET`. Kuratierte Model-Picker via `auth: {type: provider, name: ...}`. Import aus OpenClaw/acpx (`~/.acpx/config.json`, `~/.openclaw/openclaw.json`, JSON5), `omni run --from-openclaw "<name>"`, OpenClaw-Gateway-Bridge (`openclaw acp --url ... --token-file ...`).

**Harness-Binary-Overrides**: Top-level `harness:` in Config (Legacy-Skalar oder Mapping mit `default`, `<id>.command`, `<id>.args`). Präzedenz für Binary: `OMNIGENT_<NAME>_PATH` → `harness.<id>.command` → Default; `<NAME>` = Binary ohne `-native` (z.B. `OMNIGENT_CLAUDE_PATH`). Deprecated `HARNESS_<NAME>_PATH` (Entfernung angekündigt für v0.8.0 – obwohl aktuell 0.16, siehe Offene Punkte). (D:/docs/build/harnesses/configuration)

**Community-Harness-Plugins**: Python-Paket mit Entry-Point `omnigent.community.harness`, `get_contribution() -> HarnessContribution(name, valid_harnesses, harness_modules, aliases, harness_labels, ...)`; Modul exportiert `create_app() -> FastAPI`. Keine Overrides von Built-ins, keine flachen Paketpfade; Native-TUI-Harnesses (noch) nicht pluggable. Contract-Details in `GH:/designs/harness-plugin-interface.md` (Install/Auth-Metadaten, Model-Override-Env-Vars, per-Spawn-Env-Builder). (D:/docs/build/harnesses/community)

**Harness-Installation/Setup aus dem Web**: `POST /v1/hosts/{host_id}/harnesses/{harness}/install` und `/credential`, Feature-Flag `harness_install`; Picker zeigt unkonfigurierte Harnesses mit Badge oder versteckt sie (Setting "Hide unconfigured harnesses"). [Code/Doku]

**Harness-Testbench**: `tests/harness_bench` prüft deklarierte Capabilities gegen beobachtetes Verhalten (Interrupt-, Streaming-, Tool/Policy-Probes). [Code]

### 3.2 Built-in Agents

Alle Built-ins sind "just YAML configs" (D:/docs/use/builtin-agents), liegen in `examples/` (polly, debby, deep-research, remy, scribe, sentinel, aws_analyst, extensions, kimi_hello.yaml). [Code]

**Polly** (`omni polly`; D:/docs/use/builtin-agents/polly, GH:/examples/polly):
- Supervisor, der selbst nie Code schreibt; zerlegt Ziele in Sub-Tasks, delegiert an Sub-Agents mit eigenem Harness und **eigenem git worktree**; ein Agent implementiert, ein Agent eines **anderen Vendors** reviewt; jeder Implementer öffnet eigene PR; Polly merged nie.
- Sub-Agents: `claude_code` (claude-native), `codex` (codex-native), `opencode` (opencode-native), `cursor` (cursor-native), `hermes` (hermes-native), `agy` (antigravity-native), `pi` (pi, headless Review/Explore). Preflight prüft Binaries auf `PATH`.
- Headless-Worker laufen mit Bypass (`permission_mode: auto`, `yolo: true`), `blast_radius`-Policy blockt dennoch Katastrophales.
- Skills: `/fanout` (parallel, je Worktree+PR), `/cross-review` (Diff an anderen Vendor, Blocking Issues loopen zurück), `/investigate` (read-only Delegation, Synthese).
- **Goal-Mode**: Button "Goal" im Composer für Top-Level-Polly-Sessions mit Brain-Harness `claude-sdk` oder `codex`; sendet `/goal <condition>`; `/goal clear` (nur Claude). Ergebnisse: erreicht / blocked / usageLimited / budgetLimited / unachievable.

**Debby** (`omni debby`; D:/docs/use/builtin-agents/debby, GH:/examples/debby/config.yaml):
- Brain auf `claude-sdk`; zwei "plain" Sub-Agents `claude` (claude-sdk) und `gpt` (codex). Jede Frage wird parallel via `sys_session_send` an beide geschickt; Ergebnisse kommen über Inbox (`sys_read_inbox`); Antworten nebeneinander.
- Skill `/debate`: mehrrundige gegenseitige Kritik bis zur Synthese (Rundenzahl konfigurierbar).
- Benötigt Claude- und OpenAI-Credential.

**Weitere Beispiele** (Release 0.9.0, README): Deep Research (Cited Report, MCP-Suchserver in `tools/mcp/*.yaml`), repro-agent, resolve-agent, `remy` (Hindsight-Memory-Beispiel).

### 3.3 Agent-Definition in YAML

Quellen: D:/docs/use/custom-agents, D:/docs/build/*, GH:/docs/AGENT_YAML_SPEC.md, GH:/omnigent/spec/types.py.

**Formen**: einzelne YAML-Datei (`omni run agent.yaml`) oder **Verzeichnis-Bundle** mit `config.yaml` sowie optional `skills/<name>/SKILL.md`, `tools/mcp/*.yaml` (Sidecar-MCP), `tools/python/`, `tools/typescript/` (lokale Tools), `agents/<name>/config.yaml` (Sub-Agents), `AGENTS.md`. Agents können per Chat von einem Agent erzeugt und registriert werden. Registrierung als Template: `omnigent server --background --agent <dir>`. `omni run` lädt das Bundle zum Server hoch.

**Top-Level-Felder** (Dataclass `AgentSpec`, YAML-Keys) [Code+Doku]:

| Feld | Bedeutung |
| --- | --- |
| `spec_version` | aktuell `1` |
| `name`, `description` | Identität |
| `prompt` / `instructions` | System-Prompt inline; `instructions` auch Dateipfad (relativ zur YAML), gewinnt bei beidem. Fallback-Auto-Discovery: `AGENTS.md` → `CLAUDE.md` → `.cursorrules` (first wins) |
| `executor` | `type` (`omnigent` Default; außerdem `claude_sdk`, `agents_sdk`), `harness`, `model`, `reasoning_effort` (`low|medium|high|xhigh`, harness-abhängig validiert), `auth` (`{type: api_key, api_key, base_url}` / `{type: databricks, profile}` / `{type: provider, name}`), `timeout` (Default 3600 s), `max_iterations` (Default 1000), `connection`, `context_window`, `config` (Bag: `harness`, `os_env`, `yolo`, `permission_mode`, `exec_mode`, `smart_routing_harness`, `context_files`, `system_prompt_mode` …). Kurzform `executor.harness` und Langform `executor.config.harness` existieren beide. |
| `llm` | Legacy-LLM-Block (`model`, `connection`, `profile`, `request_timeout` 300, `retry`, `fallback_models`, `reasoning_effort` → wird nach `executor.reasoning_effort` gehoben) |
| `tools` | Map Name → Tool-Def (`type: mcp|function|agent`, `inherit`, `self`), plus `tools.agents: [names]`, `tools.builtins: [{name, ...}]`, `tools.timeout` (60), `tools.retry`, `tools.sandbox` (`container_image`, `container_runtime: docker|podman`) |
| `skills` | Filter für entdeckte Skills: `all` (Default) / `none` / Liste |
| `policies` | Map Name → Policy (`type: function`, `handler` + `factory_params` oder `function: {path, arguments}`, optional `on: [phases]`) |
| `guardrails` | Alternativer Block: `labels` (LabelDef `initial`, `values`), `policies`, `ask_timeout` (Default 1 Tag); Policies mit `condition` (Label-Gate), `set_labels`, `ask_timeout` |
| `params` | Typisierte User-Parameter für Tools/Skills |
| `os_env` | lokale OS-Tools (`sys_os_*`) + Sandbox (siehe 3.5) |
| `terminals` | Benannte tmux-Terminals: `command`, `args`, `env`, `os_env` (`inherit` oder eigener), `allow_cwd_override`, `allow_sandbox_override`, `scrollback` |
| `async` | Async-Dispatch-Builtins (`sys_call_async`, `sys_read_inbox`, `sys_cancel_async`), Default `true` |
| `cancellable` | Default `true` |
| `timers` | `sys_timer_set/cancel`, Default `false` |
| `spawn` | erlaubt `sys_session_create` (beliebige Agents/Bundles), Default `false` |
| `agent_session_sharing` | `none` (Default) / `non-public` / `public` → registriert `sys_session_share` |
| `compaction` | `trigger_threshold` 0.8, `recent_window` 5 |
| `interaction` | `conversational`, `modalities.input/output` |
| `model_egress` | Liste (Bedeutung im Code nicht dokumentiert; vermutlich erlaubte Model-Endpunkte im Sandbox – [Vermutung]) |
| `pass_history`, `pass_histories`, `max_sessions` | Wenn als Sub-Agent genutzt |

**Tools** (D:/docs/build/tools):
- **MCP**: `command`+`args`+`env` (stdio) oder `url`+`headers` (HTTP/SSE); `${ENV}`-Expansion; `tools:`-Allowlist; `auth: {type: databricks, profile}` (einzig unterstützter Auth-Typ); Sidecar-Dateien `tools/mcp/*.yaml` (`name`, `transport: stdio|http`, …). Bundled MCP-Server laut älterer Doku: Google (Drive/Docs/Sheets/Slides/Gmail/Calendar), GitHub, Slack, Jira, Confluence, Glean, PagerDuty (auf der aktuellen Seite nicht mehr aufgeführt – siehe Offene Punkte). Session-Owner kann MCP-Server im Web ("Agent info" → Manage MCP servers) verwalten.
- **Python function tools**: `type: function`, `callable` (vollqualifizierter Importpfad), optional JSON-Schema `parameters` (sonst aus Signatur generiert); `runtime: client` für clientseitige Tools.
- **Sub-Agent-Tools**: inline (`type: agent`, `prompt`, `executor`, `os_env: inherit`, `pass_history`, `max_sessions`) oder extern (`config: agents/x.yaml` bzw. `tools.agents: [name]` → `agents/<name>/config.yaml`).
- **Vererbung**: `tools.<name>: inherit`, `spec: self` (klont Parent-Spec).
- **Builtins** (`tools.builtins`): `web_search` (OpenAI-native `web_search_preview` bei OpenAI-Modellen; sonst `search_provider`: `duckduckgo` (keyless), `keenable` (keyless/optional key, `max_results` 1–20), `google` (+`engine_id`), `perplexity`, `nimble`, `tavily`; kein Default-Provider), `hindsight_retain/recall/reflect` (Long-Term-Memory, `api_key`, `api_url`, `bank_id`, `budget`, `max_tokens`, `tags`, `recall_tags`, `recall_tags_match`), `nimble_research`, `nimble_extract` (`template`).
- **Framework-eigene Tools** (immer/konditional registriert, aus Doku+Code): `sys_os_read`, `sys_os_write`, `sys_os_edit`, `sys_os_shell`; `sys_terminal_launch` u.a. `sys_terminal_*`; `load_skill`, `read_skill_file`; `sys_agent_list`, `sys_agent_get`, `sys_agent_download`; `sys_session_create|send|list|get_info|get_history|close|rename|share`; `sys_scheduled_task_create|list|update|delete`; `sys_call_async`, `sys_read_inbox`, `sys_cancel_async`; `sys_timer_set|cancel`; Browser-Tools `browser_navigate|snapshot|click|type|screenshot` (nur mit Desktop-App); Policy-Tools (Agent kann Policies auf Wunsch anhängen).

**Skills** (D:/docs/build/prompts): Ordner mit `SKILL.md` (YAML-Frontmatter `name`, `description`, optional `user-invocable`; Body = Instruktionen), optional `references/`, `scripts/`, `assets/` (lesbar via `read_skill_file`). Aufruf per `/skill-name` oder `load_skill`. Discovery: gebündelte `skills/` (immer) + Directory-Walk nach oben über `.agents/skills/` und `.claude/skills/` + `~/.agents/skills/`, `~/.claude/skills/`; auf Codex-Harnesses zusätzlich Codex-Home-Skills und Codex-Plugin-Skills (`<plugin>:<skill>`, Manifest `.codex-plugin/plugin.json` oder `.claude-plugin/plugin.json`). Im Composer: `/` (bzw. `$` bei Codex) zum Einfügen.

**Models & Credentials** (D:/docs/build/models): `executor.model`, `--model`, `/model` mid-session (History bleibt), `⌘⇧M`. Credential-Typen: API key (Anthropic, OpenAI, OpenRouter, Groq, DeepSeek, xAI, Mistral, Together, Fireworks), Subscription (Claude Pro/Max, ChatGPT Plus/Pro via `claude`/`codex`-CLI-Login), Gateway (OpenAI-/Anthropic-kompatibel: Databricks Unity AI Gateway, MLflow AI Gateway, OpenRouter, LiteLLM, Portkey, Helicone, Cloudflare/Kong/Vercel AI Gateway, Azure OpenAI, Ollama, LM Studio, vLLM, LocalAI, TGI, GPT4All), Databricks (Profil aus `~/.databrickscfg`, Modell-Präfix `databricks-`). Weitere Provider-Extras: AWS Bedrock, Vertex. Setup-Wizard `omni setup` erkennt Env-Keys, eingeloggte CLIs und lokale Server; Defaults pro Agent; Keys im OS-Keychain (`keyring`). Provider-Config in `~/.omnigent/config.yaml` unter `providers:` (`kind: local|gateway|...`, `default`, Familie `openai|anthropic` mit `base_url`, `api_key`, `wire_api`, `models` (Tier-Map), `pricing` (`input_per_million`, `output_per_million`, optional Cache-Raten; Fallback cache-read 0.10×, cache-write 1.25×)). Live-Model-Discovery aus Provider-Katalogen und CLIs mit Last-Known-Good-Fallback (0.8.0).

### 3.4 Contextual Policies

Quellen: D:/docs/policies/overview, /builtin, /custom, GH:/docs/POLICIES.md, GH:/omnigent/spec/types.py.

**Konzept**: Policies fangen Aktionen ab und entscheiden in Echtzeit **ALLOW / ASK / DENY**; sie sind **zustandsbehaftet** (Session-State über die ganze Session) und erhalten den vollen Kontext (kumulierte Kosten, Tool-Historie, Labels, eigener State).

**Phasen** (`Phase`-Enum) [Code]: `request` (User-Nachricht vor dem LLM-Turn), `tool_call`, `tool_result`, `response` (finale Assistant-Nachricht vor Persistenz), `llm_request` (vor jedem LLM-Call, voller Prompt), `llm_response`. Öffentliche Doku dokumentiert `request`, `tool_call`, `tool_result`, `response` (ältere Doku: `llm_request` mit `messages`, `model`).

**Auswertung**: Reihenfolge der Deklaration; erste Entscheidung gewinnt; `None` = keine Meinung → nächste Policy. DENY short-circuited. ASK parkt bis Approve/Reject/Timeout (Default `ask_timeout` 1 Tag; Timeout → DENY). Fehlender Turn-Kontext → fail closed (0.3.0).

**Drei Ebenen** (alle gleichzeitig aktiv): Session (End-User, per Chat "ask your Omnigent" oder UI-Panel, nicht persistent, Priorität 1) → Agent-Config (Developer, `policies`-Block, Priorität 2) → Server-wide (Admin, Server-Config, Settings-Seite `/settings/policies` ohne Restart, REST `POST/GET/PATCH/DELETE /v1/policies`, Priorität 3). Session-Policies: `GET/POST/PATCH/DELETE /v1/sessions/{id}/policies`, `POST .../policies/evaluate`. Registry: `GET /v1/policy-registry`.

**Formate**: Agent-YAML `handler` + `factory_params`; Server-Config `function: {path, arguments}`; optional `on:`.

**Builtin-Policies** (alle unter `omnigent.policies.builtins.*`; Module u.a. `safety`, `cost`, `cel`, `context`, `google`, `routing`):

| Policy | Funktion | Parameter |
| --- | --- | --- |
| `ask_on_os_tools` | ASK vor jeder Datei-/Shell-Operation | – |
| `block_skills` | verhindert Laden bestimmter Skills | `blocked` |
| `block_working_dir_changes` | blockt `cd`/Worktree-Wechsel | `block_cd`, `block_worktree`, `allowed_dirs`, `action` |
| `cel_policy` | CEL-Ausdruck → Map `{result, reason, state_updates}`; `reason` Fallback | `expression`, `reason` |
| `deny_pii_in_llm_request` | PII-Scan ausgehender Nachrichten | `pii_types`, `action` |
| `detect_thrashing` | Fehlerserien/Fehlerrate im Rolling Window (heuristisch) | `consecutive_threshold` 5, `window` 10, `window_error_rate` 0.8, `action` |
| `detect_loop` | identische `(tool,args)`-Hashes im Sliding Window → ASK | `window` 10, `threshold` 3 |
| `enforce_sandbox` | erzwingt Sandbox beim Agent-Start | `sandbox_type`, `allow_network`, `write_paths`, `read_paths` |
| `gcalendar_policy` | Calendar read-only default | – |
| `gdrive_policy` | Drive/Docs/Sheets/Slides; Bell-LaPadula "no write-down" | `read_all`, `allow_create`, `write_files`, `read_files`, `comment_files`, `confidential_files`, `write_down_action` |
| `github_policy` | GitHub-Lese/Schreibrechte über MCP-Tools **und** Shell-Kommandos; Deletes und Force-Push default verboten | `read_all`, `write_repos`, `write_branches`, `allow_destructive`, `deny_force_push` |
| `gmail_policy` | read+draft, kein send default | `allow_read`, `allow_send`, `allow_drafts` |
| `intent_based_authorization` (`intent_gate`) | erste User-Nachricht = Intent; ASK bei unpassenden Tool-Calls (braucht `llm:`) | – |
| `max_tool_calls_per_session` | DENY nach Limit | `limit` 100 |
| `prompt_policy` | LLM bewertet Event → ALLOW/ASK/DENY | `prompt` |
| `risk_score_policy` | akkumulierter Risk-Score aus Tools/Labels → Eskalation | `threshold`, `tool_points`, `sensitive_labels`, `guarded_tools`, `escalate_action` |
| `blast_radius` | Shell-Kommandos: safe / risky (ASK/DENY) / catastrophic (immer DENY, z.B. `rm -rf /`, Force-Push); deckt `sys_os_shell`, `Bash` (Claude/Codex), Cursor, Pi, Hermes, Goose ab | `gate_pushes`, `risky_action`, `deny_reason` |
| `spawn_bounds` | max. Sub-Agent-Dispatches pro Turn | `max_dispatches_per_turn` 5, `dispatch_tools` |
| `headless_subagent_purpose_guard` | Dispatch muss `purpose` (implement/review/explore/search) angeben | `allowed_purposes`, `deny_reason` |
| `worktree_guard` | Writes nur im eigenen git worktree | `allowed_root` `.worktrees` |
| `read_only_os` | verbietet alle mutierenden Datei-Tools | `deny_reason` |
| `cost_budget` | kumulierte Session-Kosten; ASK an Soft-Thresholds (je einmal), Hard-Limit blockt nur "expensive models" (Downgrade statt Stop) | `max_cost_usd`, `ask_thresholds_usd`, `expensive_models` (Default u.a. Opus, GPT-5.5) |
| `user_daily_cost_budget` | wie oben, pro User und UTC-Tag über alle Sessions | dito |
| `subagent_cost_budget` | Budget auf Sub-Agent-Subtree, via `sys_session_send(cost_budget=...)` | dito |
| `detect_task_switch` | Server-LLM erkennt Themenwechsel → schlägt neue Session vor | `min_turns`, `history_window`, `action`, `classification_prompt` |
| `deny_trivial_to_expensive_model` | LLM-Klassifikation TRIVIAL/COMPLEX, trivial → nicht auf teure Modelle | `expensive_models`, `classification_prompt` |

**Custom Policies**: (a) `cel_policy` (kein Deploy nötig; `event.type`, `event.data`, `event.session_state`); (b) Python-Funktion `def p(event: PolicyEvent) -> PolicyResponse | None` oder Factory; Registrierung über `POLICY_REGISTRY = [{handler, kind: "factory", name, description, params_schema}]` im Modul + `policy_modules: [myorg.policies]` in der Server-Config. Dann im Chat, YAML und UI wählbar.

**PolicyEvent** (D:/docs/policies/custom): `type` (`request` mit `user_content`, `attachments[{filename, content_type, text}]`; `tool_call` mit `name`, `arguments`; `tool_result` mit `result` + `request_data`; `response`), `target`, `session_state` (read-only), `context` (`actor.run_as`, `actor.client_id`, `usage.input_tokens/output_tokens/total_cost_usd`, `model`, `harness`, `labels`).

**PolicyResponse**: `result` (ALLOW/DENY/ASK, case-insensitive), `reason`, `data` (Ersatz-Payload bei ALLOW, z.B. PII-redigierte Argumente), `state_updates` (`[{key, action: set|increment|append|delete, value}]`; angewendet bei ALLOW/DENY, bei ASK zurückgehalten), `set_labels` (gefiltert durch deklarierte Allowlist).

**Policy auf nativen Harnesses**: Durchsetzung über Vendor-Hooks (Claude Code PreToolUse-Hook, Kimi/Devin/Hermes-Hooks), Codex-JSON-RPC-Elicitation, gespiegelte TUI-Approval-Panes oder ACP-Permission-Requests (siehe Capability-Matrix). [Code]

**Plain-Language-Policies im Chat**: Agent wählt passende Policy aus Registry, konfiguriert Parameter, fragt nach Bestätigung; gilt ab dem nächsten Turn (Tutorial: PII-Policy). (D:/quickstart/policies, /coding-agent)

### 3.5 Sandbox ("Omnibox") und Cloud-Sandbox-Hosts

Zwei getrennte Konzepte (D:/docs/omnibox, D:/docs/deploy/cloud-sandbox-host): **Omnibox/OS-Sandbox** = *was* der Agent darf; **Cloud Sandbox Host** = *wo* der Runner läuft. Sie sind kombinierbar.

#### 3.5.1 OS-Sandbox – Konfiguration (D:/docs/policies/os-sandbox)

- Gilt für `sys_os_read/write/edit/shell` und deklarierte Terminals (Doku). Die README behauptet zusätzlich, die Native-TUI-Wrapper-Terminals und `pi` würden auf Linux zwingend in bwrap laufen; der Code deklariert für Native-TUI-Harnesses jedoch `sandbox: none` (siehe 3.5.3) – nur SDK/CLI/ACP-Harness-Binaries werden über einen Exec-Launcher gesandboxt. **Nicht** für MCP-Server und den Omnigent-Supervisor-Prozess.
- Wird ein Sandbox verlangt und ist nicht verfügbar → **Fehler statt unsandboxed** (einziger Opt-out `type: none|null` oder `sandbox: false`).
- Backends: `linux_bwrap` (Bubblewrap-Namespaces + seccomp), `darwin_seatbelt` (`sandbox-exec`, SBPL-Profile), `windows_jobobject` (Job Objects), `none`; `auto`/weggelassen = Plattform-Default.
- `os_env`-Felder: `type` (`caller_process`), `cwd` (`.`), `sandbox`, `start_in_scratch` (Start in beschreibbarem Scratch-Tmpdir, Workspace read-only).
- `os_env.sandbox`-Felder: `type`, `write_paths` (Strings oder `{path, copy_on_write}`), `write_files`, `read_paths`, `allow_network` (Default `true`), `cwd_allow_hidden` (Default `[".venv"]`), `cwd_hidden_scan_recursive` (Default `false`), `mask_paths`, `cwd_hidden_scan_max_entries` (50000), `cwd_hidden_scan_overflow` (`error|warn|unlimited`), `env_passthrough`, `egress_rules`, `egress_allow_private_destinations` (Default `false`), `credential_proxy`.
- **Filesystem**: `cwd` per Default read-only auf gehärteten Backends; Dotfiles in `cwd` und `read_paths` werden maskiert (außer `cwd_allow_hidden`), d.h. `~` freigeben exponiert nicht `~/.ssh`/`~/.aws`; macOS zusätzlich `~/Library` deny. Mask nicht-rekursiv per Default; `mask_paths` für gezielte Pfade (Verzeichnis → leere Ansicht, Datei → leere Datei).
- **Copy-on-write (Linux)**: `write_paths`-Eintrag `{path, copy_on_write: true}` → Overlay (bwrap ≥ 0.11 `--overlay-src`/`--tmp-overlay`, OverlayFS mit `userxattr`, tmpfs-xattrs Linux ≥ 6.6). Aktuell nur mit `executor.harness: openai-agents`; viele Einschränkungen.
- **Netzwerk**: `allow_network` an/aus; `egress_rules` (`"METHODS host/path-glob"`, `*` für alle Methoden, `*.domain`, `**` beliebige Tiefe) → aller HTTP(S)-Traffic durch **MITM-Proxy mit Default-Deny**; private IPs (RFC1918, Loopback, `169.254.169.254`) default geblockt.
- **Environment**: deny-by-default; vererbt nur Basis (`HOME`, `PATH`, Proxy/TLS, Locale, `TMPDIR`, Session-Marker) + Variablen-Familie des laufenden Harness; alles andere nur via `env_passthrough`. Desktop-Keyring-Variablen (`DBUS_SESSION_BUS_ADDRESS`, `XDG_RUNTIME_DIR`) werden in aktiven Sandboxes entfernt bzw. durch privates `XDG_RUNTIME_DIR` ersetzt.
- **Credential-Proxy ("secretless")**: `credential_proxy`-Einträge mit `type` `https_bearer`, `https_basic`, `git_https`, `gh_basic` (host-gebunden, `target`/`targets`, `source` z.B. `{env: GH_TOKEN}`, Datei, Unix-Socket, Shell-Command) und `databricks_cli` (profil-gebunden, Platzhalter-`.databrickscfg` mit `oa_cred_*`-Tokens, nur `linux_bwrap`). Agent sieht nur Platzhalter; Proxy tauscht bei erlaubten Requests den echten Wert ein. Refresh über `refresh_interval_seconds` (Datei/Unix-Socket-Broker `GET /token`, 5-s-Deadline, ≤ 64 KiB), umfangreiche Pfad-/Symlink-Schutzregeln. Erfordert `egress_rules` + bwrap/Seatbelt.
- Wiederverwendung via YAML-Anchors oder `os_env: inherit`; Multi-Harness: jeder Sub-Agent eigener Sandbox in `agents/<x>/config.yaml`.
- **YOLO-Mode**: Ziel des Omnibox ist, Agents "unattended, in YOLO mode" laufen zu lassen; der Kernel erzwingt die Regeln, Kindprozesse erben die Grenze. (D:/docs/omnibox)
- **Tool-Container**: lokale Python-Tools optional in Docker/Podman-Image (`tools.sandbox.container_image`, `container_runtime`, `OMNIGENT_CONTAINER_RUNTIME`). (GH:/docs/AGENT_YAML_SPEC.md)
- **Windows**: laut README "degraded mode": Job Object für Prozessbaum-Containment und Ressourcenlimits, **keine** FS-/Netz-Isolation, kein L7-Proxy; keine tmux/PTY-Native-Wrapper. (GH:/README.md)

_(Implementierungsdetails pro OS siehe Abschnitt 3.5.3.)_

#### 3.5.2 Cloud Sandbox Hosts (Runner remote)

Quellen: D:/docs/deploy/cloud-sandbox-host, D:/docs/reference/configuration/cloud-sandbox, /kubernetes, D:/docs/deploy/community-sandbox-providers.
- Provider (shipped): **Modal** (24 h Cap, Named Secrets), **Daytona** (kein Cap, Env-Copy, Free-Tier-Egress-Allowlist → Cloudflare-Worker-Relay), **Blaxel** (24 h Default `ttl`), **Islo** (Idle-Pause 15 min), **Gensee** (nur server-managed), **NVIDIA OpenShell** (Gateway), **Boxlite** (lokale Micro-VMs KVM/HVF oder Remote-Pool), **microsandbox** (libkrun-MicroVMs, Sub-Sekunden-Boot), **Kubernetes** (Runner-Pods/Jobs, `agent_sandbox` mit Warm-Pools, tolerations, `runtime_class`, persistente Volumes, `secret_mounts`), **E2B**, **CoreWeave** (`cwsandbox`), Podman (0.2.0), Databricks.
- Server-managed: `sandbox:`-Block (`provider` oder `providers:` Liste, `server_url`, `host_config`, `git_clone` {`depth`, `single_branch`, `filter: blob:none`, `tags`}, Provider-Block, `reaper` {`enabled`, `terminate_after_offline_days` 30, `sweep_interval_s` 86400}). Web-UI: Host-Picker "New Sandbox"; Repo-Auswahl (GitHub-Connect oder Clone-URL, Branch; bis 10 Repos parallel bei Multi-Repo-Providern). REST: `POST /v1/sessions` mit `host_type: "managed"`, `workspace` oder `workspaces`.
- CLI: `omni sandbox create --provider <p>`, `omni sandbox connect --provider <p> --sandbox-id <id> --server <url>`.
- Image: `ghcr.io/omnigent-ai/omnigent-host:latest` (Build-Arg `EXTRA_HARNESS_CLIS`); Server-Variante `ghcr.io/omnigent-ai/omnigent-server-kubernetes`.
- Provider-Credentials bleiben beim launchenden Prozess; User loggen sich nur bei Omnigent ein.
- Native-Harness-Bridge-Server-Netzwerk: `OMNIGENT_BRIDGE_BIND_HOST`, `OMNIGENT_BRIDGE_PORT_POOL` (28700–28715).
- **Community Sandbox Provider**: Plugin-Paket mit Launcher `ExecModelHostLauncher` (`prepare/provision/run/terminate`) oder `SandboxHostLauncher` (`prepare/provision/start_host/terminate`), validiertem Config-Model und optionalen Capabilities (`managed_launch`, `programmatic_terminate`, `cli_bootstrap`, `file_copy`, `streaming_exec`, `foreground_exec`, `local_port_forward`, `resume_stopped`, `snapshot_restore`, `multi_repo`, `classifies_runner_by_agent`).

#### 3.5.3 OS-spezifische Mechanismen (Code-Analyse)

Alle Angaben [Code], Pfade relativ zum Repo. Der Produktname "Omnibox" existiert im Code nicht als Symbol; die Implementierung liegt in `omnigent/inner/` (`omnigent/sandbox/*` re-exportiert nur).

**Abstraktion** (`inner/sandbox.py`): abstrakte Klasse `SandboxBackend` mit `type_name`, `resolve(spec, cwd) -> SandboxPolicy`, `activate(policy)` (läuft *innerhalb* des Sandbox), `wrap_launcher_argv(argv, policy, cwd, chdir, target)` (setzt Launcher vor das Kommando), `post_spawn(policy, pid) -> ContainmentHandle | None` (Parent-Seite, nur Windows). Registry `register_backend`/`_BACKENDS`; bwrap+Seatbelt immer geladen, Windows nur bei `os.name == "nt"`. `resolve_sandbox()` wählt bei fehlendem/`auto`-Typ den Plattform-Default ohne Binary-Check (Backend-`resolve` scheitert laut). `type: none` → inaktive Policy; `allow_network: false` damit unzulässig; Pfad-Grants erweitern trotzdem die erreichbaren Roots der Datei-Tools. Resolvierte `SandboxPolicy` enthält zusätzlich `deny_unix_socket_paths`, `credential_source_paths`, `egress_relay_port`, `egress_socket_path`, `spawn_env_allowlist`, CoW-Felder. Pfade expandieren nur `~` (kein `$VAR`, "H4 hardening"). Transport der Policy als base64url-JSON (ohne `credential_proxy`). `OSEnvSpec` kennt zusätzlich `fork` (Workspace in Temp-Dir kopieren).

**Linux – Bubblewrap** (`inner/bwrap_sandbox.py`, `BwrapSandboxBackend`), Argument-Reihenfolge (spätere Mounts gewinnen):
1. `--ro-bind-try` für `/usr /lib /lib64 /lib32 /bin /sbin`; `/etc`-Dateien `resolv.conf hosts nsswitch.conf passwd group localtime ld.so.cache ld.so.conf`; `/etc`-Verzeichnisse `ld.so.conf.d ssl ca-certificates pki alternatives`.
2. `--proc /proc` (bzw. `--bind /proc /proc` auf "Lakebox"-Hosts), `--dev /dev`, `--tmpfs /tmp`.
3. Binds für Interpreter- und Target-Binary inkl. Symlink-Ketten (≤ 40 Hops).
4. `read_roots` `--ro-bind-try`; `cwd` `--bind` oder `--ro-bind`; `write_roots` `--bind-try` (fehlende werden angelegt); `write_files` `--bind-try`.
5. **Masken**: Verzeichnisse `--tmpfs <dir>`, Dateien/Sockets `--bind-try /dev/null <file>` (vorher `lstat`, Symlinks übersprungen); maskierte Interpreter-Pfade werden re-exponiert; verbotene AF_UNIX-Sockets (z.B. tmux-Control-Socket) via `/dev/null`-Bind.
6. `--setenv TMPDIR/TMP/TEMP/TEMPDIR <scratch>`.
7. `--unshare-net` wenn `!allow_network` oder Egress aktiv.
8. `--unshare-pid --unshare-uts --unshare-ipc --die-with-parent --new-session --chdir <dir> -- argv`. Kein explizites `--unshare-user` (außer CoW-Keeper); `$HOME` wird nie gemountet. Prüfung, dass keine Credential-Quelle unter einem Bind liegt.
- **Mask-Walker** (`inner/_cwd_scan.py`, `scan_cwd_mask_entries`): maskiert Dot-Entries außerhalb `cwd_allow_hidden` (`"*"` = alle erlaubt) und Symlinks, deren Ziel die exponierten Roots verlässt; Top-Level per Default; scannt `cwd` + alle Read/Write-Roots; Entry-Cap; priorisiert `node_modules`/`.venv` herunter.
- **seccomp** (`inner/_seccomp.py`): libseccomp via `ctypes`, BPF in-process (`seccomp_init(ALLOW)` → `seccomp_rule_add[_array]` → `seccomp_load`), inkl. Compat-ABIs (x86/x32 auf x86_64, arm auf aarch64). `activate()` setzt `PR_SET_NO_NEW_PRIVS` und lädt (1) Baseline-Denylist (EPERM): Kernel-Module, `mount`/`umount2`/`pivot_root`/`chroot` + neue Mount-API (`fsopen`, `fsmount`, `open_tree` …), `open_by_handle_at`, `nfsservctl`, `sysfs`, `_sysctl`, `ustat`, `uselib`, `quotactl*`, `unshare`, `setns`, `bpf`, `perf_event_open`, `userfaultfd`, `kcmp`, `lookup_dcookie`, `fanotify_init`, Uhr-Setzen, `reboot`, `kexec*`, `syslog`, `swapon/off`, `acct`, NUMA-Mempolicy, `ioperm`, `iopl`, `vm86*`, `sethostname`, `setdomainname`, `vhangup`, `pidfd_getfd`, `process_madvise`, Keyring-Syscalls, `ptrace`, `process_vm_readv/writev`; (2) bwrap-Extras: `clone` mit `CLONE_NEW*` → EPERM, `clone3` → ENOSYS (glibc-Fallback), `socket` nur AF_UNIX/AF_INET/AF_INET6. Filter wird im Python-Launcher nach dem bwrap-exec geladen, vor `subprocess.run(target)`.
- **Netzwerk**: Mit Egress `--unshare-net` (nur Loopback im Namespace); TCP-Relay (`inner/egress/relay.py`) lauscht im Namespace auf `127.0.0.1:<random>` und pipe't zum Unix-Socket `<scratch>/.egress.sock` des Proxys im Parent. Kein slirp. Ohne `egress_rules` und `allow_network: true` → Host-Netz geteilt.
- **Copy-on-write** (`sandbox/copy_on_write.py`): `CopyOnWriteEnvironment._start` startet einen Keeper (`bwrap --unshare-user --new-session --bind / /` + `--overlay-src R --tmp-overlay R` je Root + `--tmpfs probe -- python -m …copy_on_write --keeper`), der `user.*`-xattrs prüft und lebt, bis sein stdin schließt; Konsumenten treten via `setns`/`fchdir`/`chroot` in dessen User+Mount-Namespace ein und bauen darin die normale bwrap-Sicht.

**macOS – Seatbelt** (`inner/seatbelt_sandbox.py`): Launcher `[/usr/bin/sandbox-exec, -f, <Profil-Datei 0600, ≤ 256 KiB>, *argv]`. Profil (`_build_profile`) in dieser Reihenfolge: `(version 1)(deny default (with no-log))`; `process-fork`, `process-exec*`, `process-info*`/`signal` (target self), `mach-lookup`, `ipc-posix-shm/sem`, `sysctl-read`, `file-ioctl`, `file-read-metadata`; `/dev`-Lesen + Schreiben auf `/dev/null`, `/dev/tty`, `/dev/dtracehelper`; System-Read-Subpaths (`/usr /System /Library /bin /sbin /opt /private/etc /private/var/db/{timezone,mds,dyld}`), Literale `/ /etc /var /tmp`, dyld-Cache; `cwd` read (+write), Interpreter-Pfade, Scratch RW, `read_roots`, `write_roots` RW, `write_files`; `file-read-metadata` auf Vorfahren (realpath-Walks); `(deny file-read* file-write* …)` für maskierte Pfade; `(deny … (subpath "$HOME/Library"))` außer explizit freigegeben; Netzwerk: mit Egress nur `localhost:<relay-port>` (bind/inbound/outbound) + Unix-Socket des Proxys, sonst `(allow network*)` bei `allow_network`, sonst nichts; Deny für verbotene Unix-Sockets und Credential-Quellen. `$HOME` ist durch Default-Deny verborgen. Kein PID-Namespace, kein seccomp; maskierte Dateien sind `stat`-bar, aber nicht les-/schreibbar. Relay lauscht auf geteiltem Host-Loopback (Schutz: Zufallsport, lauter Bind-Fehler).

**Windows – Job Object** (`inner/windows_jobobject_sandbox.py`): `ctypes windll.kernel32` (kein pywin32). `post_spawn`: `CreateJobObjectW` → `SetInformationJobObject` (Klasse 9, `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`, `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) → `OpenProcess(PROCESS_SET_QUOTA|PROCESS_TERMINATE)` → `AssignProcessToJobObject`; Schließen des Handles killt den Prozessbaum. Fehler → nur Warnung, weiter ohne Containment. Docstring erwähnt CPU/Memory-Limits, **Code setzt keine**. **Nicht isoliert**: Dateisystem, Netzwerk, Syscalls; `read_paths`/`write_paths`/`allow_network` sind nur advisory (einmalige Warnung). Egress scheitert mit `OSError` (braucht Unix-Socket).

**Egress-Proxy** (`inner/egress/`): reines Python-`asyncio`-MITM in eigenem Thread (`proxy.py`, `EgressProxy`), stdlib `ssl` + `cryptography`.
- CA (`ca.py`): RSA-2048 self-signed "Omnigent Egress MITM CA", 365 Tage, Cache `~/.cache/omnigent-egress/ca.pem` + `ca-key.pem` (0600); Bundle = System-CAs + capath + Extra-CAs + eigene CA. Leaf-Certs (`certs.py`, `HostCertCache`): RSA-2048, 24 h, SAN = Hostname, LRU 256.
- Lifecycle (`controller.py`, `start_egress_proxy`): Bundle-Kopie im Scratch für den Sandbox, Upstream-Trust des Proxys an Host-Kopie gepinnt; Proxy lauscht auf Unix-Socket.
- Env-Injection (`apply_egress_env`): `HTTP_PROXY`/`HTTPS_PROXY` (+ lowercase) = `http://127.0.0.1:<port>`; `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`, `CURL_CA_BUNDLE`, `PIP_CERT`, `GIT_SSL_CAINFO` → Bundle.
- Proxy-Auth: Helper-Pfad `require_auth=True` (256-bit-Token über Config-FD, sonst 407); Terminal-Pfad `require_auth=False`; Kommentare in `relay.py`/`SandboxPolicy` widersprechen (älteres argv-Token entfernt).
- Regeln (`rules.py`, `parse_rule`): `"METHODS host[/path]"`, Methoden kommagetrennt oder `*` (GET, HEAD, POST, PUT, DELETE, PATCH, OPTIONS, TRACE); Host exakt oder `*.domain` (matcht auch Bare-Domain); Pfad-Default `/**`; `*` = ein Segment, `**` = beliebig tief, `?` = ein Zeichen; Hosts `[A-Za-z0-9.-]+` (→ `host:port`-Regeln matchen nicht, [Vermutung des Analyse-Agents]); Default-Deny. CONNECT gegen Host geprüft, entschlüsselter innerer Request gegen Methode/Host/Pfad. HTTP/2 wird nur für `* host/**`-Hosts ohne Credential-Binding durchgereicht, sonst ALPN auf HTTP/1.1 begrenzt.
- Private-IP-Block (`_assert_destination_allowed`): einmal auflösen, Adressen `not is_global`, Multicast und Azure `168.63.129.16` ablehnen; DNS-Fehler = Deny; Verbindung zur aufgelösten IP (Schutz gegen DNS-Rebinding).
- Erzwingung: Linux leerer Netz-Namespace (harte Garantie), macOS SBPL-Netzregel (nur Loopback-Relay + Socket).

**Credential-Proxy** (`inner/credential_proxy.py`, `spec/parser.py`):

| Typ | Upstream-Header | Targets | Platzhalter im Env |
| --- | --- | --- | --- |
| `https_bearer` | `Bearer` | `target`/`targets` | nur wenn `env` gesetzt |
| `https_basic` | `Basic` (User default `x-access-token`) | `target`/`targets` | nur wenn `env` gesetzt |
| `git_https` | `Basic` | `target`/`targets` | nie |
| `gh_basic` | `basic` für `github.com`, `token` für `api.*` | default `github.com` + `api.github.com` | `GH_TOKEN` + `GITHUB_TOKEN` |
| `databricks_cli` | refreshender Bearer pro Profil | Workspace-Host je Profil | Platzhalter-`.databrickscfg` im Scratch, `DATABRICKS_CONFIG_FILE`, optional `DATABRICKS_CONFIG_PROFILE` |

Quellen: genau eine von `{env}`, `{file}`, `{command}` (Shell im Parent, 30 s Timeout), `{unix_socket}` (Broker `GET /token`); Refresh nur für file/socket (`RefreshingSecretProvider`); Quellen außerhalb aller Sandbox-Pfade, keine Hardlinks (`protect_credential_source`). Platzhalter-Format `oa_cred_` + `secrets.token_urlsafe(24)`. Swap (`EgressProxy._rewrite_authorization`): TRACE/OPTIONS nie; kein `Authorization`-Header + gebundener Host → echten Wert injizieren; `oa_cred_`-Wert für diesen Host → ersetzen; unbekannter/fremder Platzhalter → 403; Nicht-Platzhalter-Header bleibt unverändert; Quellfehler → 502; ein Binding pro Host. Databricks-Token-Cache max. 60 s.

**Env-Stripping**:
- Helper (`inner/os_env.py`, `build_helper_env`): behält `PATH HOME USER LOGNAME SHELL PWD TERM TZ LANG LANGUAGE`, `PYTHON*`-Basics, Session-Marker, Windows-Konstanten, Präfix `LC_`, plus `env_passthrough`; entfernt Desktop-Variablen, Runner-Auth-Secrets, CoW-Handle; setzt `TMPDIR`/`TMP`/`TEMP`/`TEMPDIR`/`XDG_RUNTIME_DIR` auf Scratch. Launcher (`run_launcher`) pruned `os.environ` auf `spawn_env_allowlist` vor exec (kein bwrap `--clearenv`, weil argv in `/proc/cmdline` sichtbar wäre).
- Harness-CLIs (`inner/agent_env.py`, `clean_agent_env`): Basis-Präfixe `HTTP_ HTTPS_ ALL_PROXY NO_PROXY SSL_ XDG_ LANG LC_`, Basis-Namen `HOME PATH TERM TMPDIR TMP TEMP NODE_EXTRA_CA_CERTS SSH_AUTH_SOCK` + Marker; pro Harness eigene Familien (codex `OPENAI_ REQUESTS_ CODEX_HOME …`, qwen `QWEN_ OPENAI_ DASHSCOPE_`, kimi `KIMI_ MOONSHOT_`, goose `GOOSE_`, hermes `HERMES_`, pi eigene Liste, ACP keine) + deklarierte Passthroughs.

**Was tatsächlich gesandboxt wird**:
- Gesandboxt: `sys_os_read/write/edit/shell` über einen JSON-Line-RPC-Helper (`python -m omnigent.inner.os_env helper --config-fd N`, gewrappt durch das Backend); `sys_terminal_*`-tmux-Panes (der tmux-Server selbst läuft unsandboxed, Pane-Kommando ist ein Exec-Launcher-Skript, Socket wird maskiert); SDK/CLI-Harness-Binaries via `create_exec_launcher` (claude_sdk, codex_worker inkl. Egress, acp, kimi, qwen, goose, pi). Claude-SDK testet den Wrap und fällt notfalls auf unwrapped CLI mit deaktivierten Native-Tools zurück.
- **Nicht** gesandboxt: **Native-TUI-Harnesses** – `omnigent/native/native_coding_agents.py` und `harnesses/*_native/main.py` deklarieren `os_env: caller_process` mit `sandbox: {type: none}` (für Agent und User-Shells; Kommentar: entspricht der "unsandboxed stance" des Vendor-CLIs). Ebenso MCP-stdio-Server (altes `sandbox:`-Key wird abgelehnt), Runner/Supervisor, Egress-Proxy und tmux-Server. **Widerspruch** zur README-Aussage, Native-Wrapper würden auf Linux zwingend in bwrap laufen (siehe Offene Punkte).
- Hinweis: `pexpect`/`pyte` stehen in `pyproject.toml`, werden vom Paket aber nicht mehr importiert (nur Tests); der Terminal-Stack ist tmux-basiert (`inner/terminal.py`, privater tmux-Server pro Instanz `tmux -S <private>/tmux.sock -f /dev/null`, `send-keys` + `capture-pane -p -e`).

### 3.6 Sessions & Collaboration

- **Session = Konversation**; gehört dem User, nicht dem Agent (Agent-Wechsel mid-conversation, `POST /v1/sessions/{id}/switch-agent`); persistent mit Conversation-ID (`conv_...`), `omni resume <id>`; andere Agents können Historie zugänglicher Sessions lesen. (D:/docs/interact/terminal)
- **Live-Sync**: Terminal, Browser, Desktop, Mobile sind Views derselben Session in Echtzeit.
- **Sharing**: Share-Dialog mit **Read** / **Edit** (REST-Level 1=read, 2=edit/interact, 3=manage), Public-Grant `__public__` (nur read, Login bleibt nötig). Read-Shares sehen per Default keine Workspace-Dateien (Toggle "Workspace files" durch Owner). Edit = Code-Ausführung auf dem Host (Warnung!). Owner-only: absolute Host-Pfade browsen, Raw-Shell (`POST .../environments/{id}/shell`), MCP-Server/Agent-Bundle ändern (`PUT /v1/sessions/{id}/agent`). Pending Prompts (Approvals, Fragen, Plan-Review, MCP-Forms) beantwortbar von Edit+Owner. Mehrere Autoren → Model-sichtbare Präfixe `[user@x]:` (abschaltbar `OMNIGENT_SHARED_MESSAGE_ATTRIBUTION_ENABLED=0`). (D:/docs/collaborate)
- **Server-Sharing-Policy**: `OMNIGENT_SHARING_MODE` `on|read_only|restricted_read_only|off`, `OMNIGENT_PUBLIC_SHARING`, `OMNIGENT_DEFAULT_PUBLIC_SESSIONS` `off|sandbox|all`; runtime über Settings › Sharing bzw. `PUT /v1/sharing`. (D:/docs/collaborate/auth)
- **Co-drive** im Terminal: `omni attach <session_id>`. **Fork**: Web "Clone Session", `omni run --fork <id>`, REST `POST /v1/sessions/{id}/fork` (`up_to_response_id`, `title`, `side_chat`, `host_type`, `sandbox_provider`, `workspace`; eigener Model/Effort/Approval-Mode möglich); Fork auf neue Cloud-Sandbox. Fork kann mit anderem Harness weiterlaufen (Claude Code → Codex).
- **Side Chats**: `/side <frage>`, `+`-Menü, "Ask in side chat"; Fork mit `omnigent.side_chat`-Label, im Sidebar versteckt, als Tab im Workspace-Rail; Codex nativ ephemeral. Claude Codes `/btw`-Overlay ist davon getrennt.
- **Queue & Steer**: Follow-ups während eines Turns landen in clientseitiger Queue (Steer/Edit/Delete/Reorder), automatisches Draining bei Idle. (D:/docs/interact/web-ui, Blog)
- **Inline Comments & Addressing**: Kommentare an Agent-Output/Dateien, "Addressing" lässt den Agent darauf reagieren; REST `/v1/sessions/{id}/comments` (+ `/send`).
- **History/Persistenz**: Items paginiert (`GET /items`, `after`-Cursor), SSE ohne Replay; Export als JSONL (`session_meta` + `item`-Records), `omnigent session export|import`; Import fremder Chats (`omnigent import --harness claude|codex|all --session|--last N [--force]`, Web: Settings › Import sessions, läuft auf dem Host über den Tunnel). Archivieren, Löschen, Bulk-Move/-Delete, Leave shared session, Move session to another machine (0.10).
- **Multi-Device**: Unread/Seen-State geräteübergreifend (0.4.0); Presence-Events.
- **Projects**: Sidebar-Ordner (nicht verschachtelt), Defaults (Host, Working Dir, Agent, Model, Random Worktree), manuelle/alphabetische Sortierung; Löschen archiviert Sessions; REST `/v1/projects` (+ `/order`). 
- **Worktrees**: Branch im Composer → eigener git worktree (Base-Branch, Default-Base in Settings › Git, "Always use a random worktree"); Löschen von Worktree+Branch beim Session-Delete.
- **Automatische Titel**: Generator mit `session_title_instructions` (User/Server-Scope, ≤ 4000 Zeichen), Limits 100/200 Zeichen, `sys_session_rename`; abschaltbar pro User.
- **Compaction**: `/compact`, Kontextfenster-Anzeige, automatische Compaction (Default-Trigger 80 %).
- **Scheduled Tasks / Automations**: Seite `/tasks`; Felder `name`, `prompt`, `rrule` (RFC 5545, min. 60 min Abstand, ≥ 2 Fires), `agent_id`, `execution_target` (`connected_host`|`managed_sandbox`), `host_id`, `workspace`, `timezone` (IANA, Default UTC), `model_override`, `reasoning_effort`, `state` (`active|paused`), `max_cost_usd` (0.11); Run-Historie `GET /v1/scheduled-tasks/{id}/runs`, "Run now" (`POST .../run`); keine Backfills, Overlaps werden übersprungen. Agents verwalten Tasks über `sys_scheduled_task_*`. (D:/docs/build/scheduled-tasks)
- **Smart Routing**: Option "Auto" im Harness-Picker; erster Prompt → Router wählt Harness+Model. Built-in-Judge über Server-`llm:`-Block (9-s-Cap inkl. Fallbacks) oder externe API `POST <base_url>/routes:select` (Schema `omnigent.api.routing.v1`, proto3/JSON; `route_options`, `task.prompt` ≤ 4000 Zeichen, `route_selector`, `session_history` → `route_selection[0]`, `rationale`); Fehler → Fallback auf Default-Harness. (D:/docs/build/routing, /docs/reference/routing-api)
- **Usage-Page**: Session-Kosten, Tagesausgaben, Breakdown nach Harness und Model (`GET /v1/usage`, Feature `usage_page`), `omni usage`.
- **Canvas** (Feature-Flag `canvas`): räumliche Board-Ansicht pro Projekt mit Session-Karten (Status, Branch, PR-Link). (D:/docs/interact/web-ui)
- **GitHub-Tab**: verfolgte PRs pro Session (created/worked_on/attached/inferred), Summary, Kommentare, CI-Checks, gestapelter Diff; Link/Unlink per URL. (D:/docs/interact/web-ui)

### 3.7 Clients

#### CLI / TUI (`omni`/`omnigent`)
Top-Level-Kommandos (AST-Analyse von `omnigent/cli.py` u.a.) [Code]: `run`, `resume`, `attach`, `claude`, `codex`, `cursor`, `opencode`, `pi`, `hermes`, `goose`, `qwen`, `kimi`, `kiro`, `devin`, `copilot`, `antigravity` (`agy`), `polly`, `debby`, `setup`, `login`, `host` (`enable|disable|status|stop|stop-session|reset-id`), `server` (`start|status|stop`), `start`, `stop`, `sandbox` (`create|connect`), `session` (`export|import`), `import`, `config` (`list|set|unset`), `usage`, `upgrade` (`--check`, `--nightly`, `--extra`, `--target-version`, `--dry-run`, `--force`), `uninstall` (`--purge`, `--purge-workspace`), `diagnose` (secret-freier Env-Snapshot), `doctor`, `extensions` (`list|doctor`), `integration slack` (`status|stop|logs`), `debug` (`logs|db-upgrade|migrate-accounts-to-oidc`); versteckt: `version`, `pane-split`, `pane-picker`, `lakebox`, `_internal`.
`omni run`-Optionen: `-p/--prompt`, `--server`, `--no-session`, `--model`, `--harness`, `--continue/-c`, `--resume`, `--fork`, `--from-openclaw`, `--profile`. REPL mit `!`-Shell-Passthrough (0.4.0). Terminal-UI benötigt tmux. Update-Hinweis bei neuer PyPI-Version (`OMNIGENT_NO_UPDATE_CHECK=1`).

#### Web-UI (D:/docs/interact/web-ui)
Session-Liste (My/Shared with me, Pinned, Projects, Filter All/My/Shared/Archived), Composer (Workspace, Agent, Model, Effort, Permission-Dropdown, Attachments inkl. Bilder/PDF/Text, ZIP/Office/SQLite ≤ 50 MiB nur claude-/codex-native, `@`-Mentions für Workspace-Dateien, `/`-Slash-Menü, Voice-Dictation), rechtes Workspace-Rail mit Tabs (Files, Changes, GitHub, Agents/Subagents, Shells als Tabs, Browser, Side Chats), File-Editor (Monaco, Markdown-/Notebook-/PDF-Preview, Find-in-File, Mermaid), Diffs (Wrap-Lines), "Attach to agent" für Zeilenbereiche, Sub-Agent-Graph, Plan/Todo-Card, Transcript-Navigation, Command Palette `⌘K`, Session-Switcher `⌘⌥S`, Shortcuts-Overlay `⌘/`, plattformabhängige Modifier, Themes (Omnigent, Dracula, GitHub, Catppuccin, Gruvbox, Nord + Theme-Editor), Code-Font-Settings, PWA, Approval-Bar (`⌘↵`), Inbox, Members/Policies/Sharing-Admin, Usage, Tasks, Canvas, Extensions-Seiten, Branding/White-Labeling (`branding:` Block: `app_name`, `heading`, `logo.{main,loading,favicon}`, `powered_by`).

#### Desktop-App (D:/docs/interact/desktop, GH:/web/electron/README.md)
Electron-Shell um die Server-SPA; Downloads: macOS dmg (arm64/x64), Linux .deb/.AppImage, Windows .exe (alle laut aktueller Seite verfügbar). Features: OS-Notifications (Turn-Ende, Elicitation, Runner-Disconnect; nur für nicht aktiv betrachtete Sessions; Vorschau der ersten Zeilen), optionaler Sound (macOS, `afplay`), Dock-Bounce/Taskbar-Flash, Dock/Taskbar-Badge mit Unread-Zahl, mehrere Fenster (`Cmd+N`), Multi-Server (Server › New Window on Different Server; Notifications aggregiert), One-Click-Host-Reconnect, **eingebetteter Browser** mit Agent-Tools (`browser_navigate`, `browser_snapshot` mit `[ref=N]`-Accessibility-Tree, `browser_click`, `browser_type`, `browser_screenshot`), Local-Network-Permission-Popover (Allow once/Always/Deny, `settings.json` → `browser_local_network_permissions`), Show in Finder, Chat-Links im In-App-Browser, macOS-MDM Managed Preferences (`ai.omnigent.desktop` → `serverUrls`, ≤ 10, https), Deep-Links `omnigent://<host>/c/<session_id>` (Bestätigung bei unbekanntem Server), Mikrofon-Permission für Diktat (Fallback auf Server-Transkription `WS /v1/dictation/stream`), Databricks-System-Browser-OAuth, Auto-Update, About-Modal mit CLI-Versionen, kann Server+Runner automatisch managen (0.3.0) und CLI installieren (`cli_install.js`, gebündeltes `install_oss.sh`).

#### iOS (D:/docs/interact/mobile, GH:/web/ios/README.md)
App Store; SwiftUI/WKWebView-Shell; nur `https://`; Recent Servers; Foreground-Notifications + Badge (kein APNs/Background-Polling); OIDC via System-Browser, Cookie-Übernahme; Databricks-Native-OAuth; Deep-Links `omnigent://`; Managed App Configuration (`serverUrls`).

#### Android (D:/docs/interact/mobile, GH:/web/android/README.md)
Google Play; Kotlin-WebView-Shell; Recent Servers; Foreground-Notifications; Badge; Edge-to-edge-Insets; Back-Handling; Downloads (`blob:`/`data:`); EMM/MDM Managed Configuration `serverUrls` (≤ 8, komma-/zeilengetrennt).

#### Mobile Web
Vollständige, touch-optimierte Web-UI; ChatGPT-artiger Drawer (0.12); LAN-Zugriff über IP.

#### Slack (D:/docs/interact/slack)
Socket-Mode-App (kein öffentlicher Endpoint); 1 Thread ↔ 1 Session; Mention/DM startet Session; Live-Streaming (Markdown), Approve/Deny-Cards, Multiple-Choice-Formulare, Todo-Plan-Message in-place, `/omnigent` (Agent/Host/Workspace wählen), `/omnigent logout`; per-User-Identität (Device Grant RFC 8628 in accounts-Mode, OIDC-Ticket-Flow, Databricks-Web-Auth mit PKCE); Token-Verschlüsselung (Fernet), SQLite-Store; Daemon via `omni integration slack [--background|status|stop|logs]`; Thread-Owner-Modell.

#### VS Code
Iframe des lokalen Servers, Discovery via `~/.omnigent/local_server.pid` + `/health`.

#### REST API & Python SDK (D:/docs/programmatic, D:/reference, `openapi.json`)
- OpenAPI 3 (`GET /openapi.json`, Reference-Seite rendert sie; 89 Pfade auf der Website, 90 im Repo), u.a.: `/v1/sessions` (CRUD, fork, items, child_sessions, stream (SSE), events (message/interrupt), permissions, policies, comments, labels, read-state, resources (environments/filesystem/search/changes/shell, files, terminals, github), agent (+ mcp-servers, contents), switch-agent, auto-title, codex_goal, model-override/reset, sign-in-link), `/v1/agents`, `/v1/hosts` (filesystem, directories, worktrees, credentials, harness install/credential/model-options, mcp-servers, runners), `/v1/runners` (status, token), `/v1/projects`, `/v1/policies`, `/v1/policy-registry`, `/v1/skills`, `/v1/harnesses`, `/v1/sharing`, `/v1/usage`, `/v1/imports`, `/v1/extensions`, `/v1/branding/logo/{variant}`, `/v1/info`, `/v1/me`, `/v1/sandbox-providers/...`, `/health`, `/api/version`, `/.well-known/omnigent.json`.
- Nicht in der öffentlichen OpenAPI, aber im Code [Code]: `/v1/scheduled-tasks` (+ `/runs`, `/run`), `/v1/connections/github/*`, `/v1/connections/databricks/*`, Auth-Routen (`/auth/login|logout|register|setup|invite|magic|magic/redeem|callback|cli-login|cli-poll|users...`), OAuth (`/oauth/token`, `/oauth/device`, `/oauth/device/authorize|approve|deny`, `/oauth/revoke`), WebSockets (`/v1/hosts/{host_id}/tunnel`, `/v1/runners/{runner_id}/tunnel`, `/v1/sessions/updates`, `/v1/sessions/{id}/resources/terminals/{tid}/attach`, `/v1/dictation/stream`).
- Python-SDK `omnigent_client.OmnigentClient` (async): `sessions.resolve_agent`, `resolve_online_runner`, `create_from_agent_id`, `bind_runner`, `post_event`, `list`, `get`, `list_items`, `stream`, `subtree_busy`, `interrupt`, `fork`, `set_archived`.
- Automation über CLI `omni run -p` (stdout = Antwort, stderr = Session-URL).

### 3.8 Omnibox (Produktseite)
Siehe 3.5. Drei Schutzschichten: Filesystem-Isolation, Network-Isolation (Default-Deny-Proxy, private IPs geblockt), Credential-Injection (Platzhalter-Token, Proxy tauscht; nur Platzhalter in Logs/Transcripts/Model-Kontext). (D:/docs/omnibox)

### 3.9 Install & Distribution

- Voraussetzungen: Python 3.12+, Node.js 22 LTS + npm (für Harness-CLIs), pnpm (Web-UI-Build aus Source), tmux (≥ 3.3), git, `uv`; Linux: bubblewrap. (D:/quickstart/install, GH:/README.md)
- Wege: `curl -fsSL https://omnigent.ai/install.sh | sh` (bzw. `scripts/install_oss.sh`, mit `--extra`), `uv tool install omnigent`, `pip install omnigent`, `brew install omnigent-ai/tap/omnigent` (Bottles; Source-Build mit Mirror-Overrides inkl. Cargo-Index), Git-Install. Windows: nur `uv tool install` (degraded).
- Upgrade `omni upgrade` (erkennt Installationsart, drained Sessions, Nightlies), Uninstall `omnigent uninstall` (+ `scripts/uninstall_oss.sh`).
- `omnigent run` installiert fehlende Harness-CLIs (npm). [README]
- Desktop: Download-Links `D:/download/mac`, `/download/mac-x64`, Linux/Windows-Builds; Auto-Update-Feed.
- Server-Deploy: Docker Compose (`deploy/docker`, `bootstrap.sh`), Railway, Render (One-Click), Fly.io (SQLite-Volume, 1 GB RAM nötig – Server idlet ~275 MB RSS), Hugging Face Spaces (ephemeral), Modal, Cloudflare Containers (D1 + R2, scale-to-zero), Databricks Apps (Lakebase + UC Volumes, Asset Bundles) und Managed "Omnigent on Databricks (Beta)", Kubernetes (kustomize, ArgoCD-Overlay), UBI9-Image für RHEL/OpenShift, Cloudflare Quick Tunnel/Tailscale für Laptop-Server. Subpath-Hosting `OMNIGENT_WEB_BASE_PATH`/`--base-path`. (D:/docs/deploy/overview, GH:/deploy/README.md)

### 3.10 Auth & Accounts

(D:/docs/collaborate/auth, D:/docs/programmatic)
- Auswahl im Code (`server/auth.py`): `OMNIGENT_AUTH_PROVIDER` > Default **`header`** (vertraut `X-Forwarded-Email`, `OMNIGENT_AUTH_HEADER`) > mit `OMNIGENT_AUTH_ENABLED` → `oidc` (wenn Issuer gesetzt) sonst `accounts`. Der lokal vom CLI gestartete Server läuft laut Code im Accounts-Modus; Loopback-Peers erhalten den reservierten User `local`. Zugriffslevel READ=1, EDIT=2, MANAGE=3, OWNER=4. [Code]
- Modi: lokal Single-User ohne Auth (reservierter User `local`), **Built-in Accounts** (`OMNIGENT_AUTH_ENABLED=1`; Docker/Cloud default; invite-only, Single-Use-Invite-Links ohne Mailserver; Admin-Setup ohne Auto-Passwort, `needs_setup`, `POST /auth/setup` bis Admin existiert, `OMNIGENT_ACCOUNTS_INIT_ADMIN_PASSWORD`/`--admin-password`; Passwort-Hashing Argon2 [Code]), **OIDC SSO** (`OMNIGENT_OIDC_ISSUER`, `_CLIENT_ID`, `_CLIENT_SECRET`, `OMNIGENT_DOMAIN`, Callback `/auth/callback`, `OMNIGENT_OIDC_EMAIL_CLAIM`, `OMNIGENT_OIDC_SKIP_EMAIL_VERIFICATION`, `OMNIGENT_OIDC_ALLOW_INVITES`; Google, GitHub, Okta, Microsoft), **Header-based** (`X-Forwarded-Email` von vorgelagertem Proxy; laut OpenAPI-Beschreibung konfigurierbarer Header, `OMNIGENT_AUTH_PROVIDER`).
- Session-Cookie `ap_session` bzw. `__Host-ap_session` (HTTPS); JWT; Bearer-Token für CLI (`omni login`, Device-Code-Consent-Seite); `POST /auth/login` liefert Token (nur Accounts). Magic-Login-Links.
- Machine Tokens: OAuth 2.0 Client Credentials (`POST /oauth/token`, 1 h Laufzeit; nicht für Scheduled Tasks).
- Device Grant (RFC 8628) für Slack-Bot u.a. (`OMNIGENT_DEVICE_GRANT_ENABLED=1`, `OMNIGENT_DEVICE_CLIENT_SECRET`).
- Access Control: `allowed_domains` (OIDC), `admins` + runtime `<data_dir>/admins`; Admin-Settings Members/Policies/Sharing; Account-Seite; Member-Removal revoziert transaktional Cookies/JWTs/Refresh-Grants, Invites, Shares, Connections, Projects, Budgets, Tasks, Hosts. Migration `omni debug migrate-accounts-to-oidc <db-url> --domain ... --commit`. OIDC-Hosts refreshen Credentials (30-Tage-Fenster, 0.11).
- Per-User-Connections: **GitHub App** (OAuth user-to-server Token + Refresh, gespeichert im Credential Store; Sandbox bekommt `git`/`gh`-Credentials + SSH-Public-Keys; `OMNIGENT_GITHUB_APP_*`), **Databricks** (OAuth U2M + PKCE, per-Workspace, Credential-Broker im Sandbox; `OMNIGENT_DATABRICKS_*`). Benötigt Credential Store (KMS/Vault, `OMNIGENT_CREDENTIAL_CIPHER`, Identity-Binding `(workspace_id, user_id, provider, account_id)`).
- WebSocket-Origin-Allowlist `OMNIGENT_WS_ALLOWED_ORIGINS` (inkl. `*.`-Wildcards, 0.16).

### 3.11 Telemetrie & Observability

- Anonyme Usage-Telemetrie des **Servers** seit v0.6.0, default an; Opt-out `OMNIGENT_ANALYTICS=0`, `DO_NOT_TRACK=1`, `telemetry: false`; Host signalisiert Opt-out im Tunnel-Handshake. Hintergrund-Thread; Remote-Config mit Endpoint, globalem Kill-Switch, Event-Disable-Liste, OS-Exclusion, Rollout-Prozent. Keine Prompts/Inhalte/Credentials. (D:/docs/deploy/telemetry) Im Code zusätzlich: Opt-out über `DISABLE_TELEMETRY` und automatisch in CI-Umgebungen; `installation_id` in `~/.omnigent/telemetry.json`; Versand als `{"records":[…]}`-Batches an eine Ingestion-URL. [Code]
- OpenTelemetry-Tracing (Extra `tracing`; Web-Frontend mit OTel-Fetch/XHR-Instrumentierung), MLflow-Tracing (Databricks), `omnigent diagnose`, `omni debug logs`, Performance-Metriken. [Code/Releases]

### 3.12 Weitere Features

- **Voice Dictation**: Web Speech API im Browser; serverseitige Transkription (Extra `dictation`, sherpa-onnx), Streaming-Partials über `WS /v1/dictation/stream`, Offload an Remote-Worker; `⌘⌥V`. (Release 0.7.0, Electron-README)
- **Feature-Flags**: `OMNIGENT_FEATURES` (z.B. `canvas`, `usage_page`, `harness_install`). (Release 0.10.0, D:/docs/interact/web-ui)
- **Extensions**: operator-installierte Python-Distributionen (Entry-Point `omnigent.extensions`), V1: namespaced Seiten `/extensions/{id}/{route}` + Sidebar-Links; UI in opaque-origin-sandboxed iframe ohne Netz, Kommunikation über versionierte, permission-geprüfte MessageChannel-SDK (`sdks/ui`, `sdks/web-extension`); `omni extensions list|doctor`, `/v1/extensions/diagnostics`. (GH:/docs/extending/extensions.md)
- **Attachment-Limits** (Server-Config): `filesystem_attachment_max_bytes` (50 MiB), `_max_files` (20), `_max_total_bytes` (200 MiB), `_denied_extensions`. Ablage `~/.omnigent/attachments/<session-key>/`.
- **Embedded Browser Tools**, **Inbox**, **Notifications**, **Presence** (siehe Clients).
- **MLflow/Databricks-Integration**: Foundation Model API, AI Gateway, Unity Catalog, Lakebase, Managed Service.
- **Hindsight-Memory**, **Nimble**-Web-Research (siehe Tools).

---

## 4. Release-Historie

Quelle: D:/releases, GH:/CHANGELOG.md, GitHub Releases API. Kadenz: ca. wöchentlich; alle Releases als stabil (non-prerelease) markiert; Nightlies via `omni upgrade --nightly`. `main` steht auf `0.17.0.dev0`.

| Version | Datum | Highlights |
| --- | --- | --- |
| v0.1.0 | 2026-06-13 | Erster getaggter Release (Repo erstellt 2026-06-11) |
| v0.1.1 | 2026-06-16 | – |
| v0.2.0 | 2026-06-19 | Antigravity, Cursor (SDK + ACP), Codex `/compact`; Sandbox-Provider OpenShell, E2B, CoreWeave, Podman; Cloudflare-Deploy (D1+R2, S3-Store); `omni upgrade`; **Credential-Proxy** (secretless egress); MLflow-Tracing |
| v0.3.0 | 2026-06-26 | 7 neue Harnesses (Hermes, Copilot, OpenCode, Goose, Qwen, Kiro, Kimi), Native-Harness-Parität (Compaction, Cost, Resume, Fork, Model-Switch, Approval-Cards); Desktop managt Server+Runner; Projects; Databricks Apps + Lakebase, Bedrock, K8s-Runner-Pods, Boxlite; **native Windows (Core-Features)**; Custom-Agent-Creation-UI; In-App-MCP-Management; `sys_session_share` |
| v0.4.0 | 2026-07-03 | Harness-Plugin-SDK (Entry-Points, `/v1/harnesses`-Katalog, Capability-Modell); Polly mit cursor/hermes/opencode; VS-Code-Extension (angekündigt); `intent_gate`; Sub-Agent-Budgets; geräteübergreifender Unread-State; `!`-Shell-Passthrough; PWA; keyless DuckDuckGo-Websearch |
| v0.5.0/0.5.1 | 2026-07-10 | **iOS-App**; generischer **ACP-Harness**; Command Palette; Themes; **Message Queue/Steering**; K8s-Server-Image; Git-Worktrees im Composer; **Embedded Browser** im Desktop; tmux Control Mode als Default-Terminal-Transport |
| v0.6.0 | 2026-07-21 | PDF-Preview, Find-in-File, Notebook-Preview; Theme-Editor; `omni import` (Claude/Codex-Chats); Transcript-Navigation; **Slack-Integration**; **Desktop für Windows und Linux**; Telemetrie eingeführt |
| v0.7.0 | 2026-07-27 | **Automations/Scheduled Tasks** (`/tasks`); First-class **Projects** mit Defaults; **Voice Dictation** (serverseitig); Smart Routing "Auto" (Harness + Model) |
| v0.8.0–0.8.2 | 2026-08-03/04 | Live-Model-Discovery; Editor-artiger Workspace (Tabs, Mermaid, Sub-Agent-Graph); Antigravity-OAuth, `--from-openclaw`, Goal-Mode für Claude/Codex-Children; Policies `detect_loop`, `detect_thrashing`; GitHub-Policy blockt Deletes default; `omni upgrade`-Optionen, `omnigent diagnose`, Nightlies |
| v0.9.0 | 2026-08-11 | **Smart Routing** (LLM-Judge / Databricks AI Gateway); UI-Redesign (Zinc); Session-Filter; `sandbox.type: auto`; Boxlite-Disk-Size; Grok Build; Beispiel-Agents (deep-research, repro, resolve); Nimble-Builtins |
| v0.10.0 | 2026-08-19 | Mehrere Sandbox-Provider parallel (`sandbox.providers`), **Blaxel**, K8s-Runner als Jobs, ArgoCD-Overlay; Devin built-in; Copilot via `gh auth`; **Usage-Page**; `OMNIGENT_FEATURES`; Session-Move/Fork-to-top-level/Bulk-Delete; Web-Terminal über Loopback (<10 ms Echo) |
| v0.11.0 | 2026-08-24 | (nur im CHANGELOG) Claude-Permission-Mode live umschaltbar; OIDC-Host-Token-Refresh; `max_cost_usd` für Scheduled Tasks; Plan-Card im Chat; K8s-Resume dormanter Hosts u.v.m. |
| v0.12.0 | 2026-09-01 | Import lokaler CLI-Sessions aus dem Web; Projects steuern neue Sessions; Mobile/Wide-Screen-Refresh; `omni agy`; Pi-`/effort`; Custom Pricing; Fork mit eigenem Model/Effort/Approval; `omni host enable/disable` (User-Service) |
| v0.13.0 | 2026-09-09 | **GitHub-PR-Panel**; Canvas als Built-in (`OMNIGENT_FEATURES=canvas`); **microsandbox** + K8s agent-sandbox (Suspend/Resume), Reaper; Instant Session Switching; Jcode; `EXTRA_HARNESS_CLIS`; OAuth Client Credentials; Vault-Transit. Breaking: tmux ≥ 3.3, `omnigent-canvas` entfernt, `omnigent update` → `upgrade` |
| v0.14.0 | 2026-09-15 | Mehrere PRs pro Session (multi-repo), Multi-Repo-Sandboxes; Composer-Redesign; `⌘⌥S`, `⌘⇧M`; `/compact` mit Kontextanzeige (Claude SDK); Gensee; K8s-Tolerations; **CockroachDB**; Approvals überleben Restarts |
| v0.15.0 | 2026-09-22/24 | **Devin native** (ersetzt ACP-Variante); kuratierte Model-Kataloge (Pi, ACP, Managed Sandboxes); Composer (Bildvorschau, mehr Dateitypen, Queue/Sub-Agents verwalten, `/`- bzw. `$`-Skills); **Side Chats**; Session-Export; Projekt-Reihenfolge; `--base-path` Subpath-Deploy |
| **v0.16.0** | **2026-09-29** | **Aktuelle Version.** Unified Workspace-Browser/File-Panel mit Change-Badges; Desktop: Show in Finder, In-App-Browser für Chat-Links, Local-Network-Permission; **Copy-on-write**-Sandbox-Writes (Linux); Shallow/Blobless-Clones; Daytona-Resume; Admin-Default für public Sessions; `OMNIGENT_WS_ALLOWED_ORIGINS` Wildcards; Slack-Setup-Defaults; Pi `context_files`/`system_prompt_mode`; native Bilder aus MCP/Browser. Breaking: 65.535-Byte-Cap für Preferences/Snapshots; nur Owner verwalten MCP/Agent-Bundles. Runner-Reconnect-Fixes. Desktop-Onboarding-Preview in Arbeit |

**Alpha-Status**: README-Badge "status-alpha", PyPI-Classifier "Development Status :: 3 - Alpha", FAQ "Omnigent is alpha". Viele Breaking Changes zwischen Minor-Versionen; Deprecations werden teils nicht zum angekündigten Termin vollzogen.

---

## 5. Offene Punkte / Unklarheiten (Doku beantwortet nicht)

1. **Policy-Priorität vs. Reihenfolge**: Doku sagt Session → Agent → Server ("Session first … Server last"), gleichzeitig "first policy to return a decision wins". Damit könnte eine Session-Policy mit ALLOW eine Server-Policy aushebeln. Ob ALLOW tatsächlich short-circuited oder nur DENY, ist widersprüchlich: POLICIES.md: "A DENY from any policy short-circuits the rest"; Website: "The first policy to return a decision wins." Genaue Kombinationssemantik (insb. ALLOW vs. ASK über Ebenen) muss im Code verifiziert werden.
2. **Policy-Phasen**: Website-Referenz listet 4 Event-Typen; Code kennt 6 Phasen (`llm_request`, `llm_response`). Welche Phasen pro Harness (insb. Native TUI, ACP) tatsächlich feuern, ist undokumentiert (z.B. `llm_request` bei claude-native, wo Omnigent das LLM nicht selbst aufruft – [Vermutung]: feuert dort nicht bzw. nur über Proxy).
3. **Kostenberechnung**: Woher die Preise des "pricing catalog" kommen, wie Subscription-Nutzung (Claude Pro/Max) bepreist wird, und wie Native-TUI-Kosten erfasst werden (Log-Parsing? Vendor-Reports?), ist nicht dokumentiert.
4. **Server↔Runner-Protokoll**: Nachrichtenformat, Versionierung, Auth und Reconnect-Semantik des WebSocket-Tunnels sind öffentlich nicht spezifiziert (siehe Code-Analyse in 1.6 für das, was erkennbar ist).
5. **Bundled MCP-Server** (Google, GitHub, Slack, Jira, Confluence, Glean, PagerDuty): in `llms-full.txt` als "out of the box" beschrieben, auf der aktuellen MCP-&-Tools-Seite nicht mehr – unklar, ob es diese im OSS-Build gibt oder nur im Databricks-Managed-Service.
6. **Deprecation-Termine**: `HARNESS_<NAME>_PATH` "removed in v0.8.0" – aktuell 0.16 und Doku sagt weiterhin "still read". Unklar, was gilt.
7. **Windows**: Website listet `windows_jobobject` als Backend, README nennt Windows "degraded mode" ohne FS-/Netz-Isolation. Die Website-Formulierung "Every command runs in an OS-level sandbox" (FAQ) gilt also nicht für Windows. Kein Hinweis, ob WSL-Betrieb unterstützt/empfohlen ist (README: "use Linux/macOS, or WSL").
8. **Seccomp-Profil** (Linux) und **SBPL-Profil** (macOS) sind nicht öffentlich beschrieben (siehe Code-Analyse 3.5.3).
9. **Egress-Proxy & CA**: Wie das MITM-CA in diverse Toolchains (Node, Python, Go, Rust, Java) injiziert wird und was mit Non-HTTP-Traffic (SSH, raw TCP, UDP/DNS) passiert, wenn `egress_rules` gesetzt ist, ist nur teilweise dokumentiert (Databricks-Go-CLI ignoriert `SSL_CERT_FILE` auf macOS).
10. **Sandbox für Native-TUI-Harnesses**: Laut README werden die Native-Wrapper-Terminals in bwrap gesperrt; unklar, welche Sandbox-Config dafür gilt, wenn keine Agent-YAML existiert (`omni claude`), und ob Netzwerk-Egress-Regeln für das Vendor-CLI (das selbst zum LLM muss) gelten.
11. **Mobile-Apps**: Store-Links/Bundle-IDs der iOS/Android-Apps werden nur auf der Homepage als Buttons verlinkt; Push (APNs/FCM) ist ausdrücklich nicht implementiert.
12. **Desktop auf Windows/Linux**: Docs-Seite zeigt Downloads, in der llms-full-Version noch "Coming soon"; MDM nur macOS. Code-Signing für Windows nicht ersichtlich.
13. **Versionierung der REST-API**: OpenAPI-`info.version` = `0.1.0` trotz Produkt-0.16; keine Aussage zu Stabilitätsgarantien.
14. **Telemetrie-Endpoint und Event-Schema**: nicht dokumentiert (nur Mechanismus).
15. **Multi-Instance-Server**: Postgres "required if you run more than one server instance" – wie SSE-Fan-out, WebSocket-Tunnel-Affinität und Scheduler-Leader-Election über Instanzen gelöst sind, ist nicht dokumentiert (Release 0.16 erwähnt "replica failover").
16. **Agent-YAML-Dualität**: Zwei Schreibweisen (`executor.harness` vs. `executor.type: omnigent` + `executor.config.harness`; `policies` vs. `guardrails.policies`; `handler/factory_params` vs. `function/path/arguments`). Code nennt `executor.config` "TECH DEBT – remove when omnigent compat ends". Für eine Neuimplementierung muss eine kanonische Form gewählt werden.
17. **`model_egress`**-Feld der AgentSpec: Zweck nicht dokumentiert.
18. **Skills-Loader-Unterschiede** (`user-invocable` neu, Claude-Felder ignoriert) – Frontmatter-Spezifikation nicht vollständig.
19. **Native-TUI-Harnesses und Sandbox (Widerspruch Doku/Code)**: README: Native-Wrapper-Terminals (`claude`, `codex`, `cursor`, `devin`, `hermes`, `kiro`, `pi`) laufen auf Linux zwingend in bwrap. Code: `native_coding_agents.py` und `harnesses/*_native/main.py` deklarieren `sandbox: {type: none}`. Website-FAQ: "Every command runs in an OS-level sandbox". Für den Neubau muss geklärt werden, welches Verhalten Soll ist.
20. **Proxy-Authentifizierung des Egress-Proxys**: Code-Kommentare (`relay.py`, `SandboxPolicy`) sagen "Token entfernt", Helper-Pfad nutzt aber ein 256-bit-Token (407 ohne); Terminal-Pfad ohne Auth. Sicherheitsmodell für lokale Prozesse, die den Relay-Port auf macOS (geteiltes Loopback) erreichen, ist unklar.
21. **Landlock**: `designs/SANDBOX_CREDENTIAL_PROXY.md` referenziert ein `landlock_sandbox.py`, das im Repo nicht existiert – unklar, ob geplant/verworfen.
22. **Windows-Ressourcenlimits**: Docstring nennt CPU/Memory-Limits, Code setzt nur `KILL_ON_JOB_CLOSE`.
23. **Default-Auth-Modus**: Code-Default ist `header` (vertraut `X-Forwarded-Email`). Die OpenAPI-Beschreibung bestätigt das ("Trusted proxy header (default)"). Wie ein öffentlich exponierter Server ohne `OMNIGENT_AUTH_ENABLED` gegen gefälschte Header geschützt ist, ist nicht dokumentiert ([Vermutung]: Loopback-Bindung als Schutz).
24. **MySQL als DB**: Pool-Settings erwähnen MySQL, `pymysql` steckt im Databricks-Extra; MySQL ist aber nicht als unterstützte DB dokumentiert.

---

## 6. Besonders schwer nachzubauende Teile

1. **OS-Sandbox auf Windows mit echter Isolation**: Das Original bietet auf Windows nur Job Objects (Prozessbaum, Ressourcenlimits) – keine FS- oder Netzwerkisolation. Eine echte Lösung bräuchte AppContainer/LPAC-Tokens + ACL-Management, Restricted Tokens, ggf. Windows Filtering Platform (WFP) für Netz-Egress, oder einen Hypervisor-/WSL2-/Windows-Sandbox-Ansatz. Dotfile-Masking und Copy-on-write haben kein direktes Windows-Pendant (ggf. Projected FS/ProjFS, Minifilter, Bind-Links ab Win11 24H2 – [Vermutung]).
2. **macOS Seatbelt**: `sandbox-exec`/SBPL ist von Apple offiziell deprecated und undokumentiert; Profilgenerierung (read/write-Grants, Dotfile-Masking, `~/Library`-Deny, Netzwerk nur zum lokalen Proxy) muss reverse-engineered werden. Alternativen (Endpoint Security, Network Extension) brauchen Entitlements/Notarisierung.
3. **Linux bwrap + seccomp + Overlay**: Abhängig von Kernel-Features (User-Namespaces, OverlayFS-`userxattr`, tmpfs-xattrs ≥ 6.6) und Distro-Policies (Ubuntu 24.04 AppArmor). In Rust ließe sich bwrap weiter als externes Binary nutzen oder Namespaces/Landlock/seccomp direkt implementieren (z.B. `landlock`, `seccompiler`, `nix`-Crates) – beides aufwendig zu härten und zu testen.
4. **L7-MITM-Egress-Proxy + Credential-Proxy**: On-the-fly-CA, TLS-Interception, Regel-Engine (Methode/Host/Pfad-Glob), Private-IP-Blocking inkl. DNS-Rebinding-Schutz, Platzhalter-Swap für Bearer/Basic/git/gh/Databricks, Token-Refresh-Broker, CA-Injection in alle Toolchains, Zwangsrouting des Traffics (Netz-Namespace bzw. Seatbelt-Netzregeln). Sicherheitskritisch.
5. **Native-TUI-Mirroring**: Vendor-TUIs (Claude Code, Codex, Cursor, Devin, Kiro, Goose, Qwen, Kimi, Hermes, Antigravity, Pi) in tmux starten, Ausgabe in strukturierte Konversations-Items übersetzen, Approval-Panes erkennen und spiegeln, Vendor-Hooks (PreToolUse) injizieren, Interrupts/Model-Switch/Resume/Fork (Rebuild der Vendor-Session-Dateien) – pro Vendor eigene, fragile Integrationen, die bei jedem Vendor-Release brechen können. Zusätzlich Web-Terminal via tmux Control Mode und xterm.js.
6. **Breite der Harness-Integrationen**: ~20 Harnesses mit SDK-, CLI-, ACP- und Native-Pfaden; viele Vendor-SDKs existieren nur in Python/TypeScript (claude-agent-sdk, openai-agents, github-copilot-sdk, cursor-sdk, google-antigravity). Ein Rust-Neubau muss diese entweder per Subprozess (CLI/ACP/Node-Sidecar) ansprechen oder Python einbetten. ACP und MCP haben Rust-SDKs – [Vermutung, nicht verifiziert in dieser Recherche].
7. **Policy-Engine mit Python-Erweiterbarkeit**: Custom Policies sind Python-Funktionen, die im Server/Runner geladen werden (`policy_modules`, `POLICY_REGISTRY`). Rust-Alternative: CEL (z.B. `cel-interpreter`-Crate) + WASM-Plugins oder eingebettetes Python (PyO3) für Kompatibilität. Builtins mit LLM-Aufrufen (prompt_policy, intent, trivial-routing, task-switch) und Shell-Kommando-Klassifikation (`blast_radius`, `github_policy` auf Shell-Ebene) sind inhaltlich anspruchsvoll.
8. **Echtzeit-Kollaboration & Persistenz**: Event-Streaming (SSE) an viele Clients, Multi-Instanz-Betrieb mit Postgres/CockroachDB, Approvals über Restarts, Session-Fork/Compaction/Rebuild, Permission-Modell (Read/Edit/Manage/Owner/Public), transaktionale Account-Revocation.
9. **Cloud-Sandbox-Provider-Ökosystem**: ~13 Provider-SDKs (Modal, Daytona, Blaxel, Islo, E2B, CoreWeave, OpenShell, Boxlite, microsandbox, Gensee, K8s/agent-sandbox, Databricks), meist Python-SDKs; Lifecycle (Warm-Pools, Suspend/Resume, Reaper).
10. **Native Desktop-App für 3 OS**: Notifications/Badges/Dock-Bounce, Multi-Window/Multi-Server, eingebetteter, agent-steuerbarer Browser mit Accessibility-Snapshot und Local-Network-Permission (Chromium-spezifisch – mit Tauri/WebView2/WKWebView/WebKitGTK nicht 1:1 verfügbar), MDM-Preferences, Deep-Links, Auto-Update, Code-Signing/Notarisierung. Das Original nutzt Electron (Chromium überall); ein Rust-Ansatz mit Tauri hätte je OS unterschiedliche Web-Engines – insbesondere der Embedded-Browser mit CDP-artigen Fähigkeiten ist dann schwierig (ggf. eigenes Chromium via CEF – [Vermutung]).
11. **Web-UI-Umfang**: Die React-SPA ist ein großer Teil (13+ MB TypeScript): Monaco-Editor, xterm, Diffs, PDF, Mermaid, Canvas, Usage-Charts. Für einen Rust-Neubau mit nativer Desktop-App müsste entschieden werden, ob die Web-UI (Web-Stack) bleibt oder nativ (z.B. egui/Slint/Iced) neu gebaut wird – letzteres verliert Mobile/Web-Parität.
12. **Serverseitige Diktat-Transkription** (sherpa-onnx), **Smart Routing**, **Import fremder Chat-Formate** (Claude/Codex-Session-Dateien), **Codex/Claude-Session-Rebuild** für Fork – jeweils formatabhängig und vendor-gekoppelt.
