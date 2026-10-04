# 00 — Überblick

> **beton** — ein Meta-Harness für KI-Coding-Agents, geschrieben in Rust.
> *Baton* ist der Taktstock, der viele Stimmen zusammenführt; *Beton* das Fundament, auf dem sie sicher stehen.

Dieses Kapitel ist der Einstieg in die Spezifikation. Es beschreibt Vision, Ziele, Architektur und Begriffe und verweist auf die Detailkapitel. Alle Entscheidungen sind als ADRs in [`docs/adr/`](../adr/README.md) festgehalten.

---

## 1. Vision

Coding-Agents wie Claude Code, Codex, Gemini CLI oder Goose sind jeweils eigene Inseln: eigene CLI, eigenes Session-Format, eigene Permissions, eigene Sandbox (oder keine). beton legt **eine gemeinsame Schicht** darüber:

- **Komposition** — Harnesses tauschen und kombinieren, ohne etwas umzubauen: Claude Code implementiert, Codex reviewt; eine Session auf einem anderen Harness forken; eigene Agents in YAML definieren, die auf beliebigen Harnesses laufen.
- **Kontrolle** — kontextabhängige, hierarchische Policies (Budgets, Approvals, Modell-Routing, Git-Guards) und eine echte OS-Sandbox mit Egress- und Credential-Proxy, sodass auch "YOLO-Mode" sicher ist.
- **Collaboration** — dieselbe Live-Session aus Desktop-App, Browser, Terminal und Handy; teilen, gemeinsam steuern, kommentieren, Approvals unterwegs erteilen.

beton orientiert sich an [Omnigent](https://omnigent.ai) (siehe [Feature-Inventar](../../research/omnigent-features.md)), ist aber ein **eigenständiges Produkt ohne Kompatibilitätspflicht** (ADR-0001). Die Unterschiede sind gewollt: ein einziges statisches Rust-Binary statt Python-Stack, eine native Tauri-App statt Electron, ein PTY-Multiplexer statt tmux, Event-Resume statt SSE ohne Replay, Isolation auch unter Windows (Beta).

## 2. Ziele & Nicht-Ziele

**Ziele (v1.0)**
1. **Subscriptions first:** Claude Pro/Max, ChatGPT Plus/Pro und Google-Login (Gemini CLI via ACP) funktionieren über die offiziellen CLIs; kein Feature setzt einen API-Key voraus, API-Keys sind nur eine zusätzliche Option (ADR-0005, ADR-0034).
2. **Lokal-first:** Auf einem einzelnen Rechner ist beton ohne Server-Setup und ohne Account vollständig nutzbar, ohne Abhängigkeit von externen Servern außer den Modell-Anbietern; alles andere, was ins Netz geht, ist optional und standardmäßig aus. Ein Offline-E2E-Test belegt das (ADR-0003, ADR-0033).
3. **Team-fähig:** Derselbe Code läuft als zentraler Server mit OIDC, Rollen, Sharing und Sync (ADR-0010, ADR-0011).
4. **Sicher by default:** Sandbox und Proxy sind standardmäßig aktiv; verlangt man eine Sandbox und sie ist nicht verfügbar, gibt es einen Fehler statt einer stillen Degradierung (ADR-0007).
5. **Plattformen:** macOS und Linux vollwertig, Windows als Beta (Sandbox "best effort").
6. **Offen:** Apache-2.0; neue Agents kommen über ACP oder Out-of-Process-Plugins hinzu, ohne Fork (ADR-0018, ADR-0027).

**Nicht-Ziele (v1)**
- Kompatibilität zu Omnigent-YAML, -Policies oder -API.
- Ein eigener OAuth-Login bei Anthropic oder OpenAI bzw. das Speichern von Subscription-Tokens.
- Smart Routing ("Auto"-Harness), native Mobile-Apps, Slack-Bot, VS-Code-Extension, Branding/White-Label, öffentliche Links, Canvas und SaaS-Sandbox-Provider. Das sind alles **v2**-Themen, siehe [roadmap.md](roadmap.md#v2).
- Cloud-Transkription (Spracheingabe nur lokal mit Whisper, ADR-0023).

## 3. Personas & Einsatzszenarien

| Persona | Szenario | Modus |
|---|---|---|
| **Solo-Dev** | Mehrere Agents parallel in Worktrees, Cross-Vendor-Review, YOLO in der Sandbox, Approvals vom Handy | Lokal-only (+ Tailscale fürs Handy) |
| **Team-Lead** | Team-Budgets, Org-Policies (z. B. "kein `git push --force`"), Sessions teilen und gemeinsam reviewen | Zentraler Server + lokale Knoten mit Sync |
| **Platform-Engineer** | Betrieb auf Kubernetes, OIDC über Entra ID, Runner-Pools, Observability via OTel | Zentraler Server, K8s-Runner |
| **Automation** | Nächtliche Schedules ("Dependencies updaten, PR öffnen"), Webhook-getriggerte Review-Agents | Async-Agents auf Remote-Runnern |
| **Contributor** | Neuer Harness via ACP oder Plugin, neue Policy-Regel | Repo + Plugin-SDK |

## 4. Architektur

### 4.1 Komponenten

```
                ┌──────────── Clients ────────────┐
                │ Desktop (Tauri) · Web/PWA · TUI │
                │ CLI · REST/SDK                  │
                └───────────────┬─────────────────┘
                                │ WebSocket (Events, Resume ab seq) + REST
                ┌───────────────▼────────────────┐
                │          beton serve           │  Policies (CEL), History (Event-Log),
                │  (lokal als Daemon oder        │  Auth, Sharing, Scheduler, Sync,
                │   zentral als Team-Server)     │  Secrets, Usage, Whisper
                └───────────────┬────────────────┘
                                │ ausgehender WS-Tunnel (Host → Server)
                ┌───────────────▼────────────────┐
                │          beton host            │  Runner-Lifecycle, Labels,
                │ (lokal / Remote / Docker / K8s)│  RunnerProvider
                └───────────────┬────────────────┘
                                │ eine Session pro Runner
                ┌───────────────▼────────────────┐
                │          Runner                │  Harness-Adapter (native/ACP/PTY),
                │  ┌─────────┐   ┌────────────┐  │  MCP-System-Tools, Browser (CDP),
                │  │ Harness │   │ Egress- &  │  │  Worktree
                │  │  CLI    │──▶│ Cred-Proxy │──┼──▶ Internet (Allowlist)
                │  └────┬────┘   └────────────┘  │
                │  Sandbox Stufe 1 (Harness)     │
                │  └─▶ Tools in Sandbox Stufe 2  │
                └────────────────────────────────┘
```

- **`beton serve`:** die zentrale Wahrheit für Sessions (Event-Log), Policies, Identitäten und Secrets. Auf dem Laptop läuft er als Daemon gegen `localhost`, im Team als Server mit Postgres. **Lokal und zentral sind derselbe Code** (ADR-0003).
- **`beton host`:** startet Runner und verbindet sich ausschließlich **ausgehend** mit dem Server, also ohne offene Ports. Er stellt sie über einen `RunnerProvider` bereit (lokal, Docker/Podman, Kubernetes; siehe [10](10-runners-extensibility.md)).
- **Runner:** kapselt genau eine Session: Harness-Prozess, Sandbox, Proxy, MCP-Bridge, optional Browser.
- **Harness-Adapter:** übersetzen zwischen Vendor-Protokoll und dem neutralen Event-Modell ([01](01-harnesses.md), [06](06-data-sync-protocol.md)).

### 4.2 Deployment-Modi

| Modus | Beschreibung | Was fehlt |
|---|---|---|
| **Lokal-only** (Default) | Desktop-App bzw. CLI startet `serve` + `host` auf localhost, SQLite, OS-Keychain | Collaboration mit anderen; Handy nur über freigegebenes Netz (Tailscale/LAN + Device-Pairing) |
| **Team-Server** | `serve` zentral (Postgres, S3, OIDC), Hosts auf Dev-Rechnern bzw. in K8s | — |
| **Hybrid** | Lokaler Knoten synchronisiert mit dem Team-Server (Single-Writer, Budget-Leases) | — |

### 4.3 Kern-Prinzipien

1. **Event-sourced:** Jede Session ist ein Append-only-Log typisierter Events. Replay, Resume, Fork, Sync, Export und Audit bauen darauf auf ([06](06-data-sync-protocol.md)).
2. **Rust-Typen als Single Source of Truth:** JSON-Schema, TypeScript-Typen und OpenAPI werden generiert, nie von Hand gepflegt.
3. **Subscription-Regel:** beton startet die offizielle CLI und fasst deren Credentials nie an ([01](01-harnesses.md)).
4. **Secrets sind nie im Agent-Kontext:** Agents sehen nur Platzhalter (`bt_cred_*`), der Proxy setzt den echten Wert ein ([04](04-sandbox.md), [05](05-security-identity.md)).
5. **Die strengere Policy gewinnt:** deny > ask > allow, über alle Ebenen (Org → Team → User → Projekt → Agent) hinweg ([03](03-policies.md)).
6. **Fail closed:** Fehlt eine Sandbox, ist der Proxy nicht erreichbar oder liefert die Policy-Engine einen Fehler, wird der Vorgang abgelehnt, nie stillschweigend erlaubt.

### 4.4 Crate- und Repo-Layout

```
crates/
  beton-cli            # Binary "beton": Subcommands, Modus-Auswahl
  beton-core           # Domänentypen, Event-Modell, IDs, Fehler
  beton-proto          # Wire-Protokoll, WS-Framing, Versionierung, Schema-Generierung
  beton-store          # sqlx (SQLite/Postgres), Event-Log, Blob-Store, Migrationen
  beton-server         # HTTP/WS-API (axum), Auth, Sharing, Scheduler, Sync
  beton-host           # Host-Daemon, Runner-Lifecycle, Tunnel
  beton-runner         # Session-Runtime, Harness-Supervision
  beton-harness        # Adapter-Trait + Transporte (native, acp, pty)
  beton-harness-claude · beton-harness-codex · beton-harness-acp · beton-harness-direct
  beton-policy         # CEL-Engine, Hierarchie, Budgets/Leases
  beton-sandbox        # Provider-Trait + seatbelt / landlock / windows / docker
  beton-proxy          # Egress-/Credential-Proxy (TLS-MITM)
  beton-secrets        # Keychain, Envelope-Encryption, Audit
  beton-mcp            # MCP-Client/-Server, System-Tools
  beton-agents         # Agent-YAML, JSON-Schema, Skills, Built-in-Agents
  beton-browser        # CDP, Screencast, Inspect-Mode
  beton-pty            # PTY-Multiplexing (ohne tmux)
  beton-git            # Worktrees, Änderungen/Diffs, Turn-Snapshots (M1); Git-Provider-Trait, GitHub, GitLab (M4)
  beton-voice          # Whisper
  beton-plugin         # Out-of-Process-Plugin-Host, WASM-Host
  beton-plugin-sdk     # Rust-SDK für Out-of-Process-Plugins, auf crates.io veröffentlicht (PLG-012)
  beton-tui            # ratatui
  beton-sdk            # Rust-Client-SDK
  beton-fake-cli       # Protokoll-Fake-CLIs für Tests, nicht veröffentlicht (QA-002)
apps/
  desktop/             # Tauri 2 (src-tauri), nutzt apps/web
  web/                 # React-Frontend (Desktop + vom Server ausgeliefert)
packages/
  sdk-ts/              # @ifahrentholz/beton-sdk
agents/                # mitgelieferte Built-in-Agents (YAML)
deploy/                # docker-compose, Helm-Chart
docs/                  # spec/, adr/
```

### 4.5 Tech-Stack (Kurzfassung)

| Bereich | Wahl |
|---|---|
| Sprache Core | Rust (stable), tokio, axum, sqlx, serde, schemars, utoipa, ts-rs/specta |
| Policies | `cel-rust`; später wasmtime + WIT |
| Sandbox | Seatbelt (macOS), Landlock + seccomp + Namespaces (Linux), Restricted Token + Job Object (Windows Beta), Docker/Podman |
| Browser | Chromium via CDP (`chromiumoxide`), Screencast |
| TUI | ratatui + eigener PTY-Multiplexer |
| Voice | `whisper-rs` (whisper.cpp), lokal |
| Desktop | Tauri 2 |
| Frontend | React 19, TypeScript, Vite, TanStack Router/Query, Tailwind, shadcn/ui, Zustand, Monaco, xterm.js, Shiki, xyflow |
| Storage | SQLite (lokal), Postgres (zentral), FS/S3-Blob-Store |
| Observability | `tracing`, OpenTelemetry, Prometheus |

## 5. Glossar

| Begriff | Bedeutung |
|---|---|
| **Harness** | Die Runtime, die die Agent-Schleife ausführt (Claude Code, Codex, ein ACP-Agent, der Direkt-API-Loop). |
| **Adapter** | beton-Komponente, die einen Harness über einen **Transport** (native, ACP, PTY) anbindet. |
| **Agent** | Eine YAML-Definition (Instructions, Tools, Policies, Skills, Executor), die *auf* einem Harness läuft. |
| **Session** | Eine Konversation inkl. Event-Log; gehört einem User, nicht einem Agent. |
| **Event** | Ein typisierter Eintrag im Session-Log mit `session_id`, `seq`, `ts`, `actor`. |
| **Runner** | Prozess bzw. Container, der genau eine Session ausführt. |
| **Host** | Maschine bzw. Daemon (`beton host`), die Runner bereitstellt. |
| **RunnerProvider** | Backend, das Runner provisioniert (lokal, Docker, Kubernetes, Plugin). |
| **Home-Knoten** | Der einzige Knoten, der in das Log einer Session schreiben darf (Single-Writer). |
| **Policy** | Eine Regel (YAML + CEL), die an Hook-Punkten allow/deny/ask/modify/notify entscheidet. |
| **Approval** | Eine menschliche Entscheidung auf eine `ask`-Policy (Approval-Card). |
| **Budget-Lease** | Ein vom Server vergebenes Offline-Budget für einen lokalen Knoten. |
| **Sandbox Stufe 1/2** | Stufe 1 isoliert den Harness-Prozess (die CLI darf ihre eigenen Credentials lesen), Stufe 2 die Tool-Ausführung (strenger). |
| **Egress-Proxy** | TLS-MITM-Proxy, der Netzwerkzugriffe gegen eine Allowlist prüft und Credentials einsetzt. |
| **Platzhalter** | `bt_cred_*`-Token, das der Agent statt eines echten Secrets sieht. |
| **Async-Agent** | Eine Session, die ohne angeschlossenen Client läuft (gestartet per spawn, Schedule, Timer oder API). |
| **Inbox** | Zentrale Liste offener Approvals, Fragen, Mentions und fertiger Async-Agents. |
| **Side-Chat** | Versteckter Fork für Nebenfragen, ohne den Haupt-Kontext zu verschmutzen. |
| **Inspect-Mode** | Element-Picker im eingebetteten Browser, der ein DOM-Element mit Kontext an den Agent schickt. |

## 6. Kapitelübersicht

| Kapitel | Inhalt | Feature-Prefixe |
|---|---|---|
| [01 — Harnesses](01-harnesses.md) | Adapter-Trait, Claude Code, Codex, ACP, Direkt-API, PTY, Subscriptions | `HAR` |
| [02 — Agents & Automation](02-agents.md) | Agent-YAML, MCP, Skills, Sub-Agents, Built-ins, Async, Timer, Schedules | `AGT`, `ASY` |
| [03 — Policies](03-policies.md) | CEL, Hooks, Hierarchie, Budgets, Approvals, Policy-Tests | `POL` |
| [04 — Sandbox & Proxy](04-sandbox.md) | Sandbox-Backends, zwei Stufen, Egress-/Credential-Proxy | `SBX`, `PRX` |
| [05 — Security & Identity](05-security-identity.md) | Auth, OIDC, Device-Pairing, Rollen, Secrets, Bedrohungsmodell | `AUTH`, `SEC` |
| [06 — Daten, Sync & Protokoll](06-data-sync-protocol.md) | Event-Modell, WebSocket, REST, Storage, Sync, Budget-Leases | `PROTO`, `DATA`, `SYNC` |
| [07 — Sessions & Collaboration](07-sessions-collaboration.md) | Lifecycle, Fork, Worktrees, Projects, Sharing, Kommentare, GitHub/GitLab | `SES`, `COL`, `GIT` |
| [08 — Clients](08-clients.md) | Desktop, Web/PWA, CLI, TUI, API/SDK | `DESK`, `WEB`, `CLI`, `TUI`, `API` |
| [09 — Browser](09-browser.md) | Eingebetteter Browser, Agent-Tools, Inspect-Mode | `BRW` |
| [10 — Runner & Erweiterbarkeit](10-runners-extensibility.md) | RunnerProvider, Docker, K8s, Runner-Image, Plugins | `RUN`, `PLG` |
| [11 — Plattform-Features](11-platform-features.md) | Usage/Kosten, Voice, Inbox/Palette/Themes, Observability | `USE`, `VOI`, `UX`, `OBS` |
| [12 — Distribution & Qualität](12-distribution-quality.md) | Builds, Kanäle, Signing, Updates, Teststrategie, CI | `DIST`, `QA` |
| [Roadmap](roadmap.md) | Meilensteine M0–M5, Feature-Zuordnung, v2 | — |

## 7. Konventionen dieser Spec

- **Feature-IDs:** `PREFIX-NNN`, pro Prefix fortlaufend. IDs sind stabil: Wird ein Feature verworfen, bekommt es den Status *gestrichen*, aber seine Nummer wird nicht neu vergeben.
- **Feature-Eintrag:**
  ```markdown
  ### PREFIX-NNN — Kurztitel
  - **Meilenstein:** M2 · **Priorität:** Must | Should | Could
  - **Beschreibung:** …
  - **Details:** (optional)
  - **Akzeptanzkriterien:**
    - [ ] AC1 — konkret, testbar
  - **Abhängigkeiten:** IDs oder "—"
  - **Referenz:** (optional) ADR / Omnigent-Vorbild
  ```
- **Prioritäten:** *Must* ist für den Meilenstein zwingend, *Should* gewünscht, *Could* ein Stretch-Goal.
- **Sprache:** Fließtext auf Deutsch; Bezeichner, Config-Keys, Event-Namen und CLI auf Englisch.
- **Commits und PRs** referenzieren Feature-IDs, z. B. `feat(policy): POL-003 spend_cap rule`. Siehe [12](12-distribution-quality.md).
- **Änderungen an Entscheidungen** nur über ein neues ADR (`Ersetzt durch ADR-XXXX`), nicht durch stilles Umschreiben der Spec.

## 8. Entscheidungen auf einen Blick

| ADR | Entscheidung |
|---|---|
| 0001 | Eigenständiges Produkt nach Omnigent-Vorbild, keine Kompatibilität |
| 0002 | Open Source; interner Team-Einsatz als erster echter Use-Case |
| 0003 | Ein Binary mit Modi (`serve`/`host`/`run`); lokal = zentral, nur Konfiguration |
| 0004 | Desktop mit Tauri 2 |
| 0005 | Adapter mit drei Transporten (native/ACP/PTY); Subscription nur über die offizielle CLI |
| 0006 | v1-Harnesses: Claude Code, Codex, generisches ACP, Direkt-API |
| 0007 | Sandbox-Provider, zwei Stufen, Egress-/Credential-Proxy in Rust; Windows Beta |
| 0008 | Policies: YAML + CEL, hierarchisch, die strengere gewinnt |
| 0009 | SQLite lokal + Postgres zentral, event-sourced |
| 0010 | Sync: Single-Writer + Fork, Budget-Leases |
| 0011 | Auth: Token lokal; OIDC, Device-Pairing, PATs zentral; kein SCIM |
| 0012 | Agents in YAML, Tools nur über MCP, Skills, Cross-Harness-Sub-Agents |
| 0013 | Async-Agents, Timer, Schedules in v1; Webhooks über die API |
| 0014 | Collaboration-Umfang v1 inkl. Inline-Kommentare, Side-Chats, GitHub/GitLab |
| 0015 | Clients: Desktop, Web/PWA, CLI, TUI ohne tmux, OpenAPI + TS/Rust-SDK |
| 0016 | Browser: CDP-Screencast eingebettet + Umschalter auf Fenster; Inspect-Mode |
| 0017 | Runner: lokal, Remote, Docker/Podman, Kubernetes; Runner-Image |
| 0018 | Erweiterungen: Crates, Out-of-Process JSON-RPC, WASM (Policies) |
| 0019 | Eigenes Event-Modell, WebSocket mit Resume, generierte Typen |
| 0020 | React 19 + TS + Vite + TanStack + Tailwind/shadcn |
| 0021 | Weitere v1-Features (Usage, Compaction, Titel, MCP-Verwaltung, Inbox …) |
| 0022 | Smart Routing → v2 |
| 0023 | Spracheingabe nur mit lokalem Whisper |
| 0024 | Secrets: Keychain bzw. Envelope-Encryption, Proxy-Injection, Audit |
| 0025 | Telemetrie opt-in; OTel/Prometheus für Betreiber |
| 0026 | Distribution, Signing, Updates |
| 0027 | Apache-2.0 mit DCO |
| 0028 | Name "beton" |
| 0029 | Repo auf dem persönlichen Account `ifahrentholz` |
| 0030 | Roadmap M0–M5, öffentliches Release nach M3 |
| 0031 | Qualitätsstrategie für agent-getriebene Entwicklung |
| 0032 | Design-first: klickbarer Prototyp aller Screens und Zustände + Feature-Katalog vor den UI-Arbeitspaketen |
| 0033 | Lokal ohne externe Server (außer Modell-Anbietern); Online-Funktionen optional, Default aus; Offline-E2E-Test |
| 0034 | Subscription-first: kein Feature setzt einen API-Key voraus; Release-Checkliste mit echten Subscriptions |

## 9. Offene Punkte

| # | Punkt | Zu klären bis |
|---|---|---|
| 1 | Wer hält die Signing-Accounts (Apple Developer Program, Azure Trusted Signing)? | vor M3 (erstes öffentliches Release) |
| 2 | Domain (z. B. `beton.dev`, `getbeton.dev`), ungeprüft | vor M3 |
| 3 | Spätere GitHub-Org bzw. Umbenennung des npm-Scopes | bei Bedarf |
| 4 | Aktuelle Nutzungsbedingungen von Anthropic und OpenAI zur Nutzung von Subscriptions durch Drittwerkzeuge (Hinweis in der Claude-Agent-SDK-Doku zu Third-Party-claude.ai-Logins) | **vor M0-Release**, Gate für ADR-0005 |
| 5 | Markenrecht "beton", ungeprüft | vor M3 |
| 6 | Crate-Name `beton` auf crates.io ist belegt (Slab-Allocator); eventuell beim Maintainer anfragen | optional |
