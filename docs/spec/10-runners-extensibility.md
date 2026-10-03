# 10 — Runner & Erweiterbarkeit

Dieses Kapitel beschreibt, **wo** Sessions ausgeführt werden (Runner, Hosts, RunnerProvider) und **wie** beton erweitert wird (eingebaute Adapter-Crates, Out-of-Process-Plugins, WASM-Policy-Erweiterungen). Grundlage sind ADR-0017 (Runner) und ADR-0018 (Erweiterbarkeit); Meilensteine nach ADR-0030.

Scope:
- **RUN** — RunnerProvider-Trait, lokaler Runner, Remote-Host (`beton host`), Docker/Podman, Kubernetes, offizielles Runner-Image, Workspace/Repo-Clone, Labels & Scheduling, Health/Reconnect, Ressourcen-Limits, Reaper.
- **PLG** — Plugin-Protokoll (JSON-RPC 2.0 über stdio), Manifest, Installation, Signaturen, Berechtigungen, Versionierung, Rust-Plugin-SDK, WASM-Policy-Plugins.

Abgrenzung: *Was* ein Prozess darf (OS-Sandbox, Egress-Proxy) regelt SBX/PRX (siehe 04-sandbox.md). *Wo* er läuft, regelt dieses Kapitel. Beides ist kombinierbar: Ein Docker-Runner führt die Tool-Ausführung zusätzlich in der OS-Sandbox (Landlock + seccomp) aus. Harness-Adapter selbst sind in HAR (siehe 01-harnesses.md) spezifiziert, Git-Provider in GIT (siehe 07-sessions-collaboration.md), Async-Dispatch in ASY-006 (siehe 02-agents.md).

## Konzepte & Begriffe

| Begriff | Bedeutung |
| --- | --- |
| **Server** | `beton serve` — lokal als Daemon auf localhost oder zentral (Postgres). Koordiniert, persistiert, führt **nie** Agent-Code aus. |
| **Host** | Eine beim Server registrierte Maschine, auf der ein Host-Daemon läuft (`beton host`). Lokal ist der lokale Daemon implizit der einzige Host (`host_id = local`). Ein Host besitzt einen oder mehrere RunnerProvider. |
| **Runner** | Ein Prozess pro Session (Crate `beton-runner`): supervidiert den Harness-Prozess, führt Tools in der Sandbox aus, streamt Events an den Server. |
| **RunnerProvider** | Backend, das Runner-Umgebungen bereitstellt: `local` (Prozess), `docker` (Container, auch Podman), `kubernetes` (Pod/Job) oder ein Plugin. |
| **Runner-Umgebung** | Was ein Provider provisioniert: Prozess-Arbeitsverzeichnis, Container oder Pod inkl. Workspace-Volume. |
| **Binding-Token** | Das session-gebundene Runner-Token `bt_run_` (TTL 15 min, über den Tunnel erneuerbar), mit dem sich ein frisch gestarteter Runner beim Server einwählt und an genau eine Session gebunden wird (Details AUTH-011, siehe 05-security-identity.md). |
| **Labels** | Key/Value-Paare eines Hosts (`os=linux`, `repo=beton`, `gpu=true`) für Dispatch-Entscheidungen. |
| **Selector** | Label-Ausdruck einer Session/eines Schedules, z.B. `os=linux,repo in (beton,web),!gpu`. |
| **Reaper** | Hintergrundjob, der verwaiste Runner-Umgebungen (Container, Pods, Volumes) findet und aufräumt. |
| **Plugin** | Out-of-Process-Erweiterung (beliebige Sprache), spricht JSON-RPC 2.0 über stdio; Arten `harness`, `runner_provider`, `git_provider`. |
| **Eingebauter Adapter** | Rust-Implementierung desselben Traits, einkompiliert (z.B. `beton-harness-claude`). |
| **WASM-Policy-Plugin** | WebAssembly-Komponente (wasmtime + WIT), die als zusätzlicher Regeltyp in Policies ausgewertet wird. |

Grundsätze:
1. Runner bauen **ausgehende** Verbindungen auf (WebSocket-Tunnel zum Server bzw. zum Host-Daemon). Keine eingehenden Ports in Runner-Umgebungen.
2. Provider-Credentials (Docker-Socket, Kube-ServiceAccount) liegen beim **Host**, nie beim Server. Der Server sendet nur `runner.launch`-Aufträge über den Host-Tunnel (PROTO-015).
3. Eingebaute Adapter und Plugins implementieren **dieselben Traits**; ein Plugin wird über eine `PluginBridge` in den Trait eingehängt — kein Sonderpfad.
4. Lokal und zentral = derselbe Code. Lokal entfallen Labels/Scheduling (genau ein Host).

## Design

### RunnerProvider-Trait (Skizze, `beton-host`)

```rust
#[async_trait]
pub trait RunnerProvider: Send + Sync {
    fn id(&self) -> &str;                                   // "local" | "docker" | "kubernetes" | Plugin-Name
    fn capabilities(&self) -> RunnerCapabilities;
    /// Ressourcen anlegen (Container, Pod, PVC, Arbeitsverzeichnis). Idempotent über spec.runner_id.
    async fn provision(&self, spec: &RunnerSpec) -> Result<Provisioned, RunnerError>;
    /// beton-Runner in der Umgebung starten; boot enthält Server-URL/Socket + Binding-Token.
    async fn start(&self, env: &Provisioned, boot: &RunnerBoot) -> Result<RunnerHandle, RunnerError>;
    /// Hilfskommando in der Umgebung (git clone, interaktive Login-Shell). pty=true → interaktiv.
    async fn exec(&self, h: &RunnerHandle, req: ExecRequest) -> Result<ExecStream, RunnerError>;
    async fn status(&self, h: &RunnerHandle) -> Result<RunnerStatus, RunnerError>;
    /// Graceful (SIGTERM, Grace-Periode) oder Force; Workspace gemäß spec.retain behalten/löschen.
    async fn terminate(&self, h: &RunnerHandle, mode: TerminateMode) -> Result<(), RunnerError>;
    async fn snapshot(&self, _h: &RunnerHandle) -> Result<SnapshotRef, RunnerError> { Err(RunnerError::Unsupported) }
    async fn restore(&self, _s: &SnapshotRef, _spec: &RunnerSpec) -> Result<Provisioned, RunnerError> { Err(RunnerError::Unsupported) }
    /// Alle von diesem Provider/Host verwalteten Ressourcen (für den Reaper).
    async fn list_managed(&self) -> Result<Vec<ManagedResource>, RunnerError>;
}

pub struct RunnerCapabilities {
    pub isolation: Isolation,              // Process | Container | Pod
    pub workspace_modes: Vec<WorkspaceMode>, // HostPath | Bind | Volume | Clone
    pub persistent_workspace: bool,
    pub interactive_exec: bool,            // PTY-Exec (Device-Flow-Login)
    pub snapshot: bool,
    pub resource_limits: Vec<ResourceKind>, // Cpu | Memory | Pids | Disk | Timeout
    pub platforms: Vec<Platform>,          // os/arch der erzeugten Runner
    pub max_concurrent: Option<u32>,
}
```

Lebenszyklus eines Runners (Zustände, im Event-Log als `runner.status`-Events (PROTO-002)):

```
requested → provisioning → starting → connected → busy ⇄ idle → draining → terminated
                 │             │           │
                 └──── failed ─┴── lost ───┘   (lost → reconnecting → connected | failed)
```

### Konfiguration (Host-seitig, `~/.beton/host.yaml`)

```yaml
host:
  name: build-box-01
  labels: { os: linux, arch: amd64, repo: beton, tier: ci }
  max_runners: 8
providers:
  - id: local
    type: local
  - id: docker
    type: docker                 # Docker oder Podman (Docker-kompatible API)
    endpoint: unix:///var/run/docker.sock   # oder ssh://ci@docker-01, tcp://docker-01:2376 (TLS)
    image: ghcr.io/ifahrentholz/beton-runner:1.0
    workspace: { mode: clone }   # bind | volume | clone
    resources: { cpu: "2", memory: 4Gi, pids: 1024, disk: 20Gi }
  - id: k8s
    type: kubernetes
    namespace: beton-runners
    kind: pod                    # pod | job
    storage_class: encrypted-ssd
    workspace: { mode: clone, pvc_size: 20Gi, retain: { keep_days: 7 } }
    resources: { requests: { cpu: "1", memory: 2Gi }, limits: { cpu: "4", memory: 8Gi } }
    node_selector: { kubernetes.io/arch: amd64 }
reaper: { interval: 10m, orphan_grace: 30m, dry_run: false }
```

### Plugin-Manifest (`beton-plugin.toml`, kanonisch TOML *(Annahme)*)

```toml
[plugin]
name        = "beton-runner-hetzner"
version     = "0.3.1"
kind        = "runner_provider"          # harness | runner_provider | git_provider
plugin_api  = 1                          # Major-Version des Plugin-Protokolls
beton       = ">=1.0, <2.0"              # kompatible beton-Versionen (SemVer-Range)
description = "Runner auf Hetzner-Cloud-VMs"
license     = "Apache-2.0"
homepage    = "https://github.com/acme/beton-runner-hetzner"

[permissions]
network    = ["api.hetzner.cloud:443"]   # Egress-Allowlist (via Egress-Proxy)
fs_read    = ["~/.config/hcloud"]
fs_write   = []
env        = []                          # weitergereichte Env-Variablen (deny-by-default)
secrets    = ["hetzner/api-token"]       # nur als bt_cred_*-Platzhalter sichtbar
exec       = ["ssh"]                     # erlaubte Fremd-Binaries

[binaries."aarch64-apple-darwin"]
url    = "https://github.com/acme/beton-runner-hetzner/releases/download/v0.3.1/plugin-aarch64-apple-darwin.tar.gz"
sha256 = "4f1c…"
path   = "bin/beton-runner-hetzner"
[binaries."x86_64-unknown-linux-musl"]
url    = "…"
sha256 = "…"

[signature]
cosign_identity = "https://github.com/acme/beton-runner-hetzner/.github/workflows/release.yml@refs/tags/v0.3.1"
cosign_issuer   = "https://token.actions.githubusercontent.com"
```

### Plugin-Protokoll (Überblick)

Newline-delimited JSON-RPC 2.0 über stdin/stdout; stderr = Log-Zeilen (werden in `tracing` übernommen). Bidirektional: beton → Plugin (Kind-Methoden) und Plugin → beton (`host/*`-Callbacks).

| Methode | Richtung | Zweck |
| --- | --- | --- |
| `initialize` | beton → P | `{beton_version, plugin_api, platform, config}` → `{plugin_api, name, version}` |
| `initialized` | beton → P (Notification) | Handshake abgeschlossen |
| `capabilities` | beton → P | kind-spezifische Capabilities (z.B. `RunnerCapabilities`), jederzeit erneut abfragbar |
| `runner/provision`, `runner/start`, `runner/exec`, `runner/status`, `runner/terminate`, `runner/snapshot`, `runner/restore`, `runner/list_managed` | beton → P | 1:1 zum RunnerProvider-Trait |
| `harness/*` | beton → P | Spiegel des Harness-Adapter-Traits (HAR-001, siehe 01-harnesses.md); Events als Notification `harness/event` im beton-Event-Modell |
| `git/*` | beton → P | Spiegel des Git-Provider-Traits (GIT-001, siehe 07-sessions-collaboration.md) |
| `host/approval.request`, `host/secret.placeholder`, `host/log` | P → beton | Policy-/Approval-Hook, Platzhalter für deklarierte Secrets, strukturiertes Logging |
| `$/progress`, `$/stream` | beide (Notification) | Fortschritt; Streams (z.B. `exec`-Output, base64 bei Binärdaten) |
| `$/cancelRequest`, `$/ping` | beide | Abbruch laufender Requests (LSP-Stil), Liveness |
| `shutdown` → `exit` | beton → P | geordnetes Beenden (Grace 5 s, danach Kill) |

Fehlercodes: JSON-RPC-Standard plus `-32001 Unsupported`, `-32002 PermissionDenied`, `-32003 NotFound`, `-32004 Conflict`, `-32005 Transient` (retrybar), `-32006 IncompatibleVersion`.

## Features

### RUN — Runner

### RUN-001 — RunnerProvider-Trait & Capabilities
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Definiert den `RunnerProvider`-Trait (provision/start/exec/status/terminate, optional snapshot/restore, list_managed) und `RunnerCapabilities` in `beton-host`. Alle Provider — eingebaut oder Plugin — werden nur über diesen Trait angesprochen. Der Server kennt Provider nur über die vom Host gemeldeten Capabilities.
- **Details:** Trait-Skizze siehe Design. `RunnerSpec` enthält `runner_id`, `session_id`, `harness`, `workspace`, `resources`, `env_allowlist`, `labels`, `retain`. Nicht unterstützte optionale Methoden liefern `RunnerError::Unsupported`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given ein Provider ohne Snapshot-Capability, When `snapshot()` aufgerufen wird, Then liefert er `Unsupported` und der Server bietet in UI/API keine Snapshot-Aktion an.
  - [ ] AC2 — `provision()` ist idempotent: zweimaliger Aufruf mit gleicher `runner_id` erzeugt genau eine Umgebung (Contract-Test gegen jeden eingebauten Provider).
  - [ ] AC3 — Die vom Host gemeldeten Capabilities erscheinen unverändert in `GET /v1/hosts/{id}` (JSON-Schema-Snapshot-Test).
  - [ ] AC4 — Eine gemeinsame Contract-Test-Suite (`runner_provider_contract!`-Makro) läuft gegen `local` und ist für alle weiteren Provider wiederverwendbar.
- **Abhängigkeiten:** PROTO-015 (siehe 06-data-sync-protocol.md)
- **Referenz:** ADR-0017; Omnigent `SandboxHostLauncher` (prepare/provision/start/terminate + Capabilities)

### RUN-002 — Lokaler Runner-Provider
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Provider `local` startet pro Session einen Runner-Prozess auf der eigenen Maschine (Kindprozess des Daemons). Workspace ist ein Host-Pfad (Projektverzeichnis oder Worktree). Der Runner verbindet sich über einen Unix-Domain-Socket (Windows: Named Pipe) mit dem lokalen Daemon.
- **Details:** Runner-Prozess erhält `BETON_RUNNER_PARENT_PID` und beendet sich, wenn der Daemon stirbt. Env des Runners ist deny-by-default (Allowlist analog SBX-005, siehe 04-sandbox.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton run claude` im Projektverzeichnis startet genau einen Runner-Prozess, dessen cwd das Projektverzeichnis ist.
  - [ ] AC2 — When der Daemon per `SIGKILL` beendet wird, Then terminiert der Runner (und sein Harness-Prozessbaum) innerhalb von 5 s.
  - [ ] AC3 — Der Runner öffnet keinen TCP-Listen-Port (Test: `lsof`/`netstat`-Prüfung in CI auf macOS/Linux).
  - [ ] AC4 — Env-Variablen außerhalb der Allowlist (z.B. `AWS_SECRET_ACCESS_KEY`) sind im Runner-Prozess nicht sichtbar.
- **Abhängigkeiten:** RUN-001
- **Referenz:** ADR-0003, ADR-0017

### RUN-003 — Runner-Lebenszyklus & Supervision
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Zustandsmaschine (requested → … → terminated, plus failed/lost) mit persistierten Statuswechseln. Der Host supervidiert Runner, meldet Exit-Codes und räumt Prozesse/Umgebungen auf. Idle-Runner werden nach `idle_timeout` (Default 1 h) beendet; die Session bleibt fortsetzbar (Resume über HAR-020/SES-003).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Jeder Zustandswechsel erzeugt ein Event mit `runner_id`, `from`, `to`, `reason`; ungültige Übergänge (z.B. terminated → busy) werden abgelehnt (Unit-Test der Zustandsmaschine).
  - [ ] AC2 — Given ein Runner ohne Turn-Aktivität und ohne angeschlossenen Client, When `idle_timeout` abläuft, Then wird er graceful beendet und die Session wechselt in Status `stopped` (siehe SES-001 in 07-sessions-collaboration.md).
  - [ ] AC3 — Ein Harness-Crash (Exit ≠ 0) führt zu Runner-Status `failed` mit Exit-Code und letzten 50 stderr-Zeilen (secret-redigiert) im Event.
  - [ ] AC4 — `terminate(Graceful)` sendet SIGTERM (Windows: Job-Close nach CTRL_BREAK), wartet `grace` (Default 10 s) und eskaliert dann zu Force.
- **Abhängigkeiten:** RUN-001, RUN-002
- **Referenz:** Omnigent Runner-Idle-Timeout 1 h

### RUN-004 — Remote-Host-Daemon `beton host`
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** `beton host` registriert eine Maschine bei einem zentralen Server und hält einen ausgehenden WebSocket-Tunnel (`/v1/tunnel`, Protokoll-Owner PROTO-015). Über den Tunnel empfängt der Host Control-Frames (`runner.launch`, `runner.stop`, `rpc.request` u. a. für Worktree-Operationen) und meldet Runner-Status (`runner.launched`, `runner.exited`, `host.status`). Authentifizierung über Device-Pairing-Token im OS-Keychain (AUTH-008, AUTH-011, siehe 05-security-identity.md).
- **Details:** `beton host pair --server https://beton.example.com` (Code/QR), danach `beton host` (Vordergrund) bzw. `--background`. Hello-Frame: `host_version`, `protocol`, `labels`, `providers[]` mit Capabilities, `harnesses[]` (gefundene CLIs + Versionen), `max_runners`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given ein gepairter Host hinter NAT ohne offene Ports, When der Server eine Session für diesen Host anlegt, Then startet der Runner auf dem Host und streamt Events an den Server.
  - [ ] AC2 — Ein widerrufenes Gerätetoken führt beim nächsten Verbindungsversuch zu Close-Code 4401 und der Host beendet sich mit verständlicher Meldung (Exit-Code 1, CLI-001).
  - [ ] AC3 — Der Hello-Frame enthält Labels, Provider-Capabilities und Harness-Versionen; der Server zeigt sie in `GET /v1/hosts`.
  - [ ] AC4 — Inkompatible Protokollversion (außerhalb des Fensters, PROTO-004/DIST-018) wird mit Close-Code 4400 und Upgrade-Hinweis abgelehnt.
- **Abhängigkeiten:** RUN-001, RUN-003; AUTH-008, AUTH-011 (siehe 05-security-identity.md); PROTO-015 (siehe 06-data-sync-protocol.md)
- **Referenz:** ADR-0003; Omnigent `omni host`, Host-Tunnel-Frames

### RUN-005 — Host-Labels & automatische Labels
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Hosts tragen statische Labels aus `host.yaml` plus automatisch ermittelte Labels unter dem reservierten Präfix `beton.` (`beton.os`, `beton.arch`, `beton.provider.<id>`, `beton.harness.<name>`). Labels sind Grundlage für Scheduling (RUN-016) und Anzeige.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Auf einem Linux-arm64-Host mit Docker und `claude`-CLI erscheinen automatisch `beton.os=linux`, `beton.arch=arm64`, `beton.provider.docker=true`, `beton.harness.claude=<version>`.
  - [ ] AC2 — Statische Labels mit Präfix `beton.` werden beim Start mit Fehler abgelehnt.
  - [ ] AC3 — Label-Keys/-Werte werden gegen `^[a-z0-9]([a-z0-9._-]{0,62})$` validiert; Verstöße erzeugen einen Konfigurationsfehler mit Zeilenangabe.
  - [ ] AC4 — Admins können Labels serverseitig ergänzen (`PATCH /v1/hosts/{id}/labels`); Host-seitige Labels bleiben unverändert und haben bei Konflikt Vorrang *(Annahme)*.
- **Abhängigkeiten:** RUN-004
- **Referenz:** ADR-0012 (Dispatch an Runner mit passenden Labels)

### RUN-006 — Host als User-Service
- **Meilenstein:** M4 · **Priorität:** Should
- **Beschreibung:** `beton host enable|disable|status` installiert den Host-Daemon als Benutzer-Dienst (macOS launchd LaunchAgent, Linux systemd `--user`, Windows geplanter Task bei Anmeldung), damit Remote-Hosts nach Reboot wieder online sind.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton host enable` erzeugt die Service-Definition, startet den Dienst und `beton host status` meldet `running` mit PID und Server-URL.
  - [ ] AC2 — `beton host disable` stoppt den Dienst und entfernt die Definition rückstandsfrei.
  - [ ] AC3 — Logs des Dienstes landen in `~/.beton/logs/host.log` (siehe OBS-001 in 11-platform-features.md).
- **Abhängigkeiten:** RUN-004
- **Referenz:** Omnigent `omni host enable`

### RUN-007 — Heartbeat, Health & Reconnect
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Host- und Runner-Tunnel nutzen Keepalive und Reconnect gemäß PROTO-015 (Owner): WS-Ping alle 20 s, 60 s ohne Pong → `lost`; Reconnect mit exponentiellem Backoff (0,5 s → max. 30 s, ±20 % Jitter). Runner puffern Events lokal bis zum Ack und setzen nach Reconnect ab der letzten bestätigten `seq` fort (Resume, PROTO-015 in 06-data-sync-protocol.md). Bleibt ein Runner länger als `lost_grace` (Default 5 min) weg, wird er `failed`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given eine laufende Session, When das Netz 60 s unterbrochen wird, Then verbindet sich der Runner neu und alle während der Unterbrechung erzeugten Events erscheinen lückenlos und ohne Duplikate (Test mit Netzwerk-Fault-Injection).
  - [ ] AC2 — Nach 60 s ohne Pong zeigt der Server den Runner als `lost`, Clients erhalten ein Status-Event innerhalb von 70 s.
  - [ ] AC3 — Der lokale Event-Puffer ist auf 64 MiB begrenzt; bei Überlauf wird die Session pausiert statt Events zu verwerfen.
  - [ ] AC4 — Backoff-Intervalle liegen nachweislich zwischen 0,5 s und 30 s (Unit-Test mit deterministischem Jitter-Seed).
- **Abhängigkeiten:** RUN-003, RUN-004, PROTO-015 (siehe 06-data-sync-protocol.md)
- **Referenz:** Omnigent Tunnel-Keepalive (30 s, 3 Misses), Event-Ack-Extension

### RUN-008 — Docker/Podman-Provider (lokal)
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Provider `docker` startet pro Session einen Container aus dem Runner-Image über die Docker-Engine-API (`bollard`); Podman wird über dessen Docker-kompatiblen API-Socket unterstützt (inkl. rootless). Lokal verbindet sich der Runner über einen in den Container gemounteten Unix-Socket des Daemons (`/run/beton/daemon.sock`) — kein Netzzugang zum Host nötig *(Annahme; Fallback TCP über `host-gateway` mit Binding-Token)*.
- **Details:** Härtung: `--cap-drop=ALL`, `no-new-privileges`, Non-root-User (UID 10001), read-only Root-FS + tmpfs `/tmp`, `--pids-limit`, eigenes Bridge-Netz `beton-runners`. Container-Labels `beton/runner-id`, `beton/host-id`, `beton/session-id`. Innerhalb des Containers greift zusätzlich die Tool-Sandbox (Landlock + seccomp, SBX-008 in 04-sandbox.md). Abgrenzung: RUN-008 ist Owner des RunnerProviders (wo der Runner läuft); Docker/Podman als Sandbox-Backend für Stufe 1/2 und das gemeinsame Härtungsprofil sind SBX-015.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given Docker oder Podman ist verfügbar, When eine Session mit `provider: docker` startet, Then läuft der Runner im Container und die Session verhält sich funktional wie mit `local` (gleiche E2E-Szenarien mit Fake-Harness, siehe QA-007 in 12-distribution-quality.md).
  - [ ] AC2 — `docker inspect` des Runner-Containers zeigt `CapDrop=ALL`, `NoNewPrivileges`, Non-root-User und `ReadonlyRootfs=true`.
  - [ ] AC3 — Ist kein Docker/Podman-Socket erreichbar, meldet `beton doctor` den Provider als `unavailable`, und Sessions mit diesem Provider schlagen mit klarer Fehlermeldung fehl (kein Fallback auf `local`).
  - [ ] AC4 — Nach `terminate` existiert kein Container mit dem Label `beton/runner-id=<id>` mehr; Volumes gemäß `retain`.
- **Abhängigkeiten:** RUN-001, RUN-010, RUN-014, SBX-015 (siehe 04-sandbox.md)
- **Referenz:** ADR-0007 (Docker als starke Option), ADR-0017

### RUN-009 — Remote-Docker-Host
- **Meilenstein:** M5 · **Priorität:** Should
- **Beschreibung:** Der Docker-Provider kann einen entfernten Docker-Daemon ansprechen (`ssh://user@host` oder `tcp://host:2376` mit mTLS). Da Bind-Mounts sich auf das Dateisystem des Remote-Hosts beziehen, ist dort `workspace.mode: clone` oder `volume` Default; `bind` nur mit explizit konfigurierten `remote_paths`. Runner verbinden sich direkt mit dem Server (Server-URL + Binding-Token).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit `endpoint: ssh://ci@docker-01` startet eine Session auf dem Remote-Daemon; lokal läuft kein Container.
  - [ ] AC2 — `workspace.mode: bind` ohne `remote_paths` wird bei Remote-Endpunkt mit Konfigurationsfehler abgelehnt.
  - [ ] AC3 — `tcp://` ohne TLS wird abgelehnt, außer `insecure_tcp: true` ist gesetzt (dann Warnung in Log und `beton doctor`).
- **Abhängigkeiten:** RUN-008, RUN-011
- **Referenz:** ADR-0017 (lokal oder Remote-Docker-Host)

### RUN-010 — Workspace-Strategien
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Einheitliche Workspace-Modi für alle Provider: `host_path` (lokal, Projektverzeichnis/Worktree), `bind` (Host-Pfad in Container), `volume` (benanntes Volume/PVC pro Session) und `clone` (Repo wird in der Umgebung geklont, RUN-011). `retain` steuert, ob Workspaces nach Session-Ende gelöscht (`delete`) oder `keep_days: N` behalten werden.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Jeder Provider lehnt nicht unterstützte Modi bei `provision` mit `Unsupported` ab, bevor Ressourcen angelegt werden.
  - [ ] AC2 — Bei `bind` sind nur der Workspace und deklarierte Pfade gemountet; `~/.ssh` des Hosts ist im Container nicht sichtbar.
  - [ ] AC3 — `retain: { keep_days: 7 }` hält das Volume nach Session-Ende vor; der Reaper (RUN-013) entfernt es nach Ablauf.
- **Abhängigkeiten:** RUN-001
- **Referenz:** Omnigent `git_clone`-Block, `retain`/Reaper

### RUN-011 — Repo-Clone in Remote-Runnern
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Bei `workspace.mode: clone` klont der Runner vor dem Harness-Start das Repository (`url`, `ref`, `depth`, `filter`, `submodules`). Git-Credentials werden nie in die Umgebung kopiert: der Clone läuft durch den Egress-/Credential-Proxy, der den Git-Provider-Token (SEC-008, siehe 05-security-identity.md) für den Ziel-Host injiziert. Danach wird der Session-Worktree/Branch angelegt (SES-015, siehe 07-sessions-collaboration.md).
- **Details:**
  ```yaml
  workspace:
    mode: clone
    git: { url: https://github.com/ifahrentholz/beton.git, ref: main, depth: 1, filter: "blob:none", submodules: false }
  ```
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein privates GitHub-Repo wird erfolgreich geklont, obwohl im Container weder `GH_TOKEN` noch `~/.git-credentials` einen echten Token enthalten (nur `bt_cred_*`).
  - [ ] AC2 — Clone-Fehler (404, Auth, Timeout 10 min) setzen den Runner auf `failed` mit Ursache; es startet kein Harness.
  - [ ] AC3 — `depth`/`filter` werden an `git clone` durchgereicht (Integrationstest mit lokalem Git-Server-Fixture).
  - [ ] AC4 — Clone-Fortschritt erscheint als Progress-Events in der UI.
- **Abhängigkeiten:** RUN-010; PRX-006 (siehe 04-sandbox.md), SEC-008 (siehe 05-security-identity.md), GIT-004, SES-015 (siehe 07-sessions-collaboration.md)
- **Referenz:** Omnigent `git_clone` (depth, single_branch, blob:none)

### RUN-012 — Kubernetes-Provider
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Provider `kubernetes` (`kube-rs`) erzeugt pro Session einen Pod (`kind: pod`, `restartPolicy: Never`) oder Job (`kind: job`, `backoffLimit: 0`, `ttlSecondsAfterFinished`, empfohlen für Async-Agents) plus PVC für den Workspace. Der Provider läuft in einem `beton host`-Deployment im Cluster mit namespace-beschränkter ServiceAccount-RBAC (Helm-Chart, DIST-012 in 12-distribution-quality.md). Runner-Pods wählen sich direkt beim Server ein.
- **Details:** Pod-SecurityContext: `runAsNonRoot`, `allowPrivilegeEscalation: false`, `capabilities.drop: [ALL]`, `seccompProfile: RuntimeDefault`. Unterstützt `node_selector`, `tolerations`, `runtime_class`, `image_pull_secrets`, `resources`. Labels `beton/runner-id`, `beton/host-id`, `beton/session-id`, `app.kubernetes.io/managed-by=beton`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — In einem kind-Cluster (CI) startet eine Session als Pod mit PVC; nach `terminate` sind Pod und (bei `retain: delete`) PVC gelöscht.
  - [ ] AC2 — Die ServiceAccount-Rolle des Hosts erlaubt nur `pods`, `pods/log`, `pods/exec`, `persistentvolumeclaims`, `jobs`, `events` im konfigurierten Namespace (RBAC-Test: Zugriff auf anderen Namespace → 403).
  - [ ] AC3 — Ein Pod, der wegen fehlender Ressourcen `Pending` bleibt, führt nach `schedule_timeout` (Default 5 min) zu `failed` mit dem Kubernetes-Event-Grund.
  - [ ] AC4 — `kind: job` erzeugt einen Job mit `backoffLimit: 0`; ein Fehlschlag wird nicht automatisch neu gestartet.
- **Abhängigkeiten:** RUN-001, RUN-010, RUN-014
- **Referenz:** ADR-0017; Omnigent K8s-Runner-Pods/Jobs, persistente Volumes

### RUN-013 — Reaper für verwaiste Runner
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Jeder Host führt periodisch (Default alle 10 min) einen Reaper aus: `list_managed()` aller Provider wird mit dem Server-Zustand abgeglichen. Ressourcen, deren Runner unbekannt, `terminated` oder länger als `orphan_grace` (Default 30 min) `lost` ist, werden beendet; abgelaufene `retain`-Volumes gelöscht. Ressourcen ohne beton-Labels werden nie angefasst.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given ein Container/Pod mit `beton/runner-id` eines unbekannten Runners, älter als `orphan_grace`, When der Reaper läuft, Then wird er entfernt und ein Audit-Event `runner.reaped` *(Annahme)* geschrieben.
  - [ ] AC2 — Ressourcen ohne `beton/host-id` des eigenen Hosts bleiben unangetastet (Test mit fremd gelabelten Containern).
  - [ ] AC3 — `reaper.dry_run: true` listet Kandidaten im Log, ohne zu löschen.
  - [ ] AC4 — Ist der Server nicht erreichbar, löscht der Reaper nichts außer abgelaufenen `retain`-Volumes (fail-safe).
- **Abhängigkeiten:** RUN-008, RUN-012
- **Referenz:** Omnigent `reaper` (terminate_after_offline_days, sweep_interval_s)

### RUN-014 — Offizielles Runner-Image
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** `ghcr.io/ifahrentholz/beton-runner:<version>` (multi-arch amd64/arm64) enthält `beton`, Node.js LTS, die offiziellen CLIs `claude` und `codex`, mindestens eine ACP-CLI (Gemini CLI; weitere per Build-Arg `EXTRA_CLIS`), `git`, `ripgrep`, CA-Bundle, `tini` als Init. Läuft als Non-root-User `beton` (UID 10001). CLI-Versionen sind in `deploy/runner-image/versions.toml` gepinnt; Variante `-slim` ohne Harness-CLIs. Die Registry ist keine Voraussetzung (ADR-0033): Das Image lässt sich aus `deploy/runner-image/` lokal bauen oder per `docker load` aus einer Datei laden; der Docker-/K8s-Provider akzeptiert jeden lokalen Image-Namen (`image: beton-runner:local`) mit `pull: never | if_missing` (Default `if_missing`, gezogen wird nur, was explizit konfiguriert ist). RUN-014 ist Owner des Image-Inhalts; Build, Signatur und Veröffentlichung regelt DIST-010.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `docker run --rm ghcr.io/ifahrentholz/beton-runner:<v> beton doctor --json` meldet `claude`, `codex` und die ACP-CLI mit den gepinnten Versionen.
  - [ ] AC2 — Image läuft als UID 10001; `id -u` im Container ≠ 0.
  - [ ] AC3 — Das Image ist cosign-signiert und mit SBOM versehen (DIST-010, DIST-013 in 12-distribution-quality.md); Trivy-Scan in CI ohne `CRITICAL`-Findings mit verfügbarem Fix.
  - [ ] AC4 — Build mit `--build-arg EXTRA_CLIS="@qwen-code/qwen-code"` installiert die zusätzliche CLI.
  - [ ] AC5 — Ein lokal gebautes bzw. per `docker load` geladenes Image wird mit `pull: never` vom Docker-Provider ohne Registry-Zugriff genutzt; eine Session läuft damit wie mit dem veröffentlichten Image (E2E mit Fake-Harness).
- **Abhängigkeiten:** DIST-010 (siehe 12-distribution-quality.md)
- **Referenz:** ADR-0017; Omnigent `omnigent-host`-Image mit `EXTRA_HARNESS_CLIS`

### RUN-015 — CLI-Login im Container & verschlüsseltes Credential-Volume
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Für Subscription-Nutzung in Container-/Pod-Runnern führt der User einmalig den Login **der Vendor-CLI selbst** aus: `beton runner login --harness claude --host <host>` öffnet eine interaktive PTY-Exec-Sitzung (RUN-001 `exec`, `pty: true`) mit `claude auth login` bzw. `codex login --device-auth`; der User schließt den Device-Flow im eigenen Browser ab. Die CLI-Konfigurationsverzeichnisse (`~/.claude`, `~/.codex`, `~/.gemini`) liegen auf einem persistenten Volume pro `(user, host, harness)`, das verschlüsselt gespeichert wird. beton implementiert keinen OAuth-Flow und parst/liest die Token-Dateien nie.
- **Details:** Verschlüsselung *(Annahme)*: Kubernetes — Pflicht einer verschlüsselnden StorageClass (`credential_storage_class`); Docker — der Runner hält das Verzeichnis als opakes, mit AES-256-GCM verschlüsseltes Archiv auf dem Volume, entschlüsselt beim Start in ein tmpfs und verschlüsselt bei Änderung/Stop zurück. Der Schlüssel liegt im Keychain des Hosts bzw. in einem Kubernetes-Secret; er verlässt den Host nie. Der Inhalt wird nie an den Server übertragen, nie geloggt, nie in `beton diagnose` aufgenommen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach einmaligem `beton runner login --harness claude` laufen weitere Claude-Sessions auf diesem Host im Container ohne erneuten Login (auch nach Container-Neustart).
  - [ ] AC2 — Der PTY-Kanal der Login-Sitzung ist als `sensitive` markiert: sein Inhalt erscheint weder im Event-Log noch in Logs noch in Diagnose-Bundles (Test: Suche nach Marker-String).
  - [ ] AC3 — Das Credential-Volume von User A wird nie in Runner von User B gemountet (Test mit zwei Usern auf einem zentralen Host).
  - [ ] AC4 — Der Volume-Inhalt auf dem Docker-Host ist ohne Host-Schlüssel nicht im Klartext lesbar (Test: Suche nach bekanntem Fake-Token-String im Volume-Verzeichnis schlägt fehl).
  - [ ] AC5 — Auf Kubernetes verweigert der Provider die Anlage eines Credential-Volumes, wenn keine `credential_storage_class` konfiguriert ist.
- **Abhängigkeiten:** RUN-008, RUN-012, RUN-014; AUTH-008, SEC-002 (siehe 05-security-identity.md)
- **Referenz:** ADR-0005 (Subscription nur via Vendor-CLI), ADR-0017

### RUN-016 — Labels & Scheduling (Dispatch)
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Sessions, Schedules und Async-Agents (ASY-004, ASY-006 in 02-agents.md) können einen `runner.selector` und optional `provider` angeben. Der Server filtert online Hosts nach Selector, Provider-Capabilities, Harness-Verfügbarkeit (`beton.harness.<name>`) und freier Kapazität (`max_runners`) und wählt den Host mit der geringsten Auslastung (Tie-Break: jüngster Heartbeat). Ohne Treffer wird der Auftrag bis `dispatch_timeout` (Default 10 min) gequeued, danach schlägt er fehl. RUN-016 ist Owner der Dispatch-Implementierung; ASY-006 beschreibt die Sicht der Async-Runs.
- **Details:** Selector-Grammatik: `key=value`, `key!=value`, `key in (a,b)`, `key notin (a,b)`, `key`, `!key`; Terme durch Komma = UND.
  ```yaml
  runner:
    selector: "os=linux,repo=beton,!gpu"
    provider: docker
  ```
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given zwei Hosts (`os=linux` mit 1/8 Runnern, `os=linux` mit 6/8), When eine Session mit `selector: os=linux` dispatcht wird, Then landet sie auf dem weniger ausgelasteten Host.
  - [ ] AC2 — Ein Selector ohne passenden online Host führt zu Queue-Status `waiting_for_runner`; kommt innerhalb des Timeouts ein passender Host online, startet die Session dort.
  - [ ] AC3 — Eine Session mit `harness: codex` wird nie an einen Host ohne `beton.harness.codex` dispatcht.
  - [ ] AC4 — Selector-Parser ist property-getestet (Parse ∘ Print = Identität) und lehnt ungültige Ausdrücke mit Positionsangabe ab.
- **Abhängigkeiten:** RUN-004, RUN-005; ASY-006 (siehe 02-agents.md)
- **Referenz:** ADR-0012 (Dispatch an Runner mit passenden Labels)

### RUN-017 — Ressourcen-Limits
- **Meilenstein:** M5 · **Priorität:** Should
- **Beschreibung:** Einheitliches `resources`-Schema (`cpu`, `memory`, `pids`, `disk`, `timeout`, `idle_timeout`) wird je Provider umgesetzt: Docker (`--cpus`, `--memory`, `--pids-limit`, Storage-Opt), Kubernetes (requests/limits, `ephemeral-storage`, `activeDeadlineSeconds`), lokal best effort (Linux cgroups v2 via systemd-Scope falls verfügbar, sonst rlimits; macOS rlimits; Windows Job-Object-Limits). Nicht durchsetzbare Limits werden in den Capabilities ausgewiesen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Fake-Harness-Szenario, das 6 GiB alloziert, wird bei `memory: 4Gi` im Docker-Provider OOM-beendet; die Session zeigt Ursache `oom`.
  - [ ] AC2 — `timeout: 8h` beendet den Runner nach Ablauf mit Ursache `timeout`, unabhängig von Aktivität.
  - [ ] AC3 — Fordert eine Session ein Limit, das der Provider nicht unterstützt, wird beim Dispatch gewarnt (Default) bzw. abgelehnt (`resources.strict: true`).
- **Abhängigkeiten:** RUN-008, RUN-012
- **Referenz:** ADR-0017

### RUN-018 — Snapshot & Restore (optional)
- **Meilenstein:** M5 · **Priorität:** Could
- **Beschreibung:** Provider mit `snapshot`-Capability (z.B. Kubernetes über VolumeSnapshots) können den Workspace einer Session sichern und eine neue Umgebung daraus erzeugen — etwa für Fork einer Session samt Arbeitsstand (SES-006, siehe 07-sessions-collaboration.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Bei einem Provider mit `snapshot: true` erzeugt `POST /v1/runners/{id}/snapshot` einen SnapshotRef; `restore` liefert eine Umgebung mit identischem Workspace-Inhalt (Hash-Vergleich).
  - [ ] AC2 — Bei Providern ohne Capability ist die Aktion in UI/CLI nicht verfügbar.
- **Abhängigkeiten:** RUN-001, RUN-012
- **Referenz:** Omnigent Capability `snapshot_restore`

### RUN-019 — Runner- & Host-Verwaltung (CLI/API)
- **Meilenstein:** M4 · **Priorität:** Should
- **Beschreibung:** Übersicht und Steuerung von Hosts und Runnern: `beton hosts list`, `beton runners list [--host]`, `beton runners logs <id>`, `beton runners stop <id>`; REST `GET /v1/hosts`, `GET /v1/runners`, `POST /v1/runners/{id}/stop`. Sichtbarkeit gemäß Rollen (AUTH-014, AUTH-015, siehe 05-security-identity.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton runners list --json` liefert pro Runner `id`, `host`, `provider`, `session_id`, `status`, `started_at`, `resources`.
  - [ ] AC2 — Ein Member sieht nur Runner eigener bzw. mit ihm geteilter Sessions; Admins sehen alle.
  - [ ] AC3 — `beton runners stop <id>` beendet den Runner graceful; die Session wechselt in `stopped`.
- **Abhängigkeiten:** RUN-003, RUN-004
- **Referenz:** Omnigent `omni host status|stop-session`

### PLG — Plugins & Erweiterbarkeit

### PLG-001 — Eingebaute Adapter als Crates
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Erstanbieter-Integrationen sind einkompilierte Rust-Implementierungen derselben Traits, die auch Plugins bedienen: `beton-harness-claude`, `beton-harness-codex`, `beton-harness-acp`, `beton-harness-direct` (Harness-Trait, HAR-001 in 01-harnesses.md); Docker- und Kubernetes-Provider in `beton-host` hinter Cargo-Features `provider-docker`, `provider-kubernetes` *(Annahme: Module statt eigener Crates, um das Crate-Layout beizubehalten)*; GitHub/GitLab in `beton-git` (GIT-002, GIT-003 in 07-sessions-collaboration.md, ab M4). Neue Harnesses sollen bevorzugt über ACP angebunden werden statt als Plugin.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eingebaute Adapter registrieren sich über eine gemeinsame Registry (`AdapterRegistry`) mit `kind`, `id`, `capabilities`; der Server unterscheidet in API-Antworten nur über das Feld `source: builtin | plugin`.
  - [ ] AC2 — (ab M5) Ein Build mit `--no-default-features --features provider-docker` enthält keinen Kubernetes-Code (Prüfung via `cargo tree`).
  - [ ] AC3 — (ab M5) Dieselbe Contract-Test-Suite (RUN-001 AC4, HAR-/GIT-Contracts) läuft gegen eingebaute Adapter und gegen ein Plugin über die PluginBridge.
- **Abhängigkeiten:** RUN-001
- **Referenz:** ADR-0018

### PLG-002 — Plugin-Protokoll (JSON-RPC 2.0 über stdio)
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Out-of-Process-Plugins sprechen newline-delimited JSON-RPC 2.0 über stdin/stdout, bidirektional (Plugin kann `host/*`-Callbacks aufrufen). stderr wird zeilenweise als Log übernommen. Das Protokoll ist kind-spezifisch namespaced (`runner/*`, `harness/*`, `git/*`) und spiegelt die jeweiligen Traits; ein JSON-Schema des Protokolls wird aus Rust-Typen generiert und veröffentlicht.
- **Details:** Methodenübersicht und Fehlercodes siehe Design. Max. Nachrichtengröße 16 MiB; Binärdaten base64 in `$/stream`. Default-Timeouts: `initialize` 10 s, sonstige Requests 60 s (pro Methode überschreibbar, `runner/provision` 15 min).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Das generierte JSON-Schema liegt unter `schemas/plugin-protocol-v1.json` und ist Teil der Snapshot-Tests (QA-006, siehe 12-distribution-quality.md).
  - [ ] AC2 — `$/cancelRequest` für einen laufenden `runner/provision` führt innerhalb von 5 s zu einer Antwort mit Fehler `-32800 RequestCancelled`.
  - [ ] AC3 — Eine Nachricht > 16 MiB oder ungültiges JSON beendet die Verbindung mit Log-Eintrag; beton startet das Plugin gemäß PLG-003 neu.
  - [ ] AC4 — Ein Plugin-Callback `host/approval.request` durchläuft die reguläre Policy-Auswertung (POL-003, POL-006, siehe 03-policies.md) und liefert `allow|deny|ask`-Ergebnis.
- **Abhängigkeiten:** PLG-001; PROTO-013 (siehe 06-data-sync-protocol.md)
- **Referenz:** ADR-0018 (MCP/ACP-Stil)

### PLG-003 — Plugin-Lifecycle & Supervision
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** beton startet Plugin-Prozesse lazy beim ersten Bedarf, führt den Handshake `initialize` → `initialized` → `capabilities` durch, überwacht per `$/ping` (alle 30 s) und beendet sie mit `shutdown`/`exit`. Abgestürzte Plugins werden mit Backoff (1 s → 60 s, max. 5 Versuche in 10 min) neu gestartet; danach Status `crashlooping` und Meldung in `beton doctor`/UI.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Plugin, das `initialize` mit inkompatibler `plugin_api` beantwortet, wird nicht verwendet; Fehler `-32006` erscheint in `beton plugin list` als Status `incompatible`.
  - [ ] AC2 — Nach `shutdown` beendet beton ein Plugin, das nicht innerhalb von 5 s `exit` erreicht, per Kill.
  - [ ] AC3 — Fünf Abstürze in 10 min führen zu `crashlooping`; weitere Aufrufe liefern sofort einen Fehler statt erneut zu starten.
- **Abhängigkeiten:** PLG-002
- **Referenz:** LSP/MCP-Lifecycle

### PLG-004 — Manifest-Format
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Jedes Plugin hat ein `beton-plugin.toml` mit `name`, `version` (SemVer), `kind` (`harness | runner_provider | git_provider`), `plugin_api`, `beton` (Versions-Range), Metadaten, `permissions`, `binaries` pro Rust-Target-Triple (URL, SHA-256, Pfad im Archiv) und `signature` (cosign-Identität). Ein JSON-Schema des Manifests wird veröffentlicht; `beton plugin validate <pfad>` prüft es.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton plugin validate` meldet fehlende Pflichtfelder, ungültige SemVer-Angaben und unbekannte `kind`-Werte mit Feldpfad.
  - [ ] AC2 — Fehlt ein Binary für die aktuelle Plattform, schlägt die Installation mit Liste der verfügbaren Targets fehl.
  - [ ] AC3 — Plugin-Namen müssen `^[a-z][a-z0-9-]{2,63}$` entsprechen; Namen von eingebauten Adaptern (`claude`, `codex`, `acp`, `direct`, `docker`, `kubernetes`, `github`, `gitlab`, `local`) sind reserviert.
- **Abhängigkeiten:** PLG-002
- **Referenz:** ADR-0018

### PLG-005 — Installation `beton plugin install`
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** `beton plugin install <name>[@version]` (Registry), `beton plugin install git+https://…[#tag]` (Git-URL; Manifest im Repo-Root, Binaries per Manifest-URL oder `cargo build --release` bei `build = "cargo"` *(Annahme)*) und `beton plugin install ./pfad` (lokales Verzeichnis oder Archiv; **immer möglich**, auch ohne Netzwerk und ohne Registry, ADR-0033). Installiert nach `~/.beton/plugins/<name>/<version>/`; `~/.beton/plugins/installed.toml` hält Quelle, Version, Checksumme und gewährte Berechtigungen. Im zentralen Betrieb installieren Admins Plugins auf Server (git_provider) bzw. Hosts (harness, runner_provider).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton plugin install beton-runner-hetzner@0.3.1` lädt das Plugin aus dem Registry-Index, prüft Checksumme/Signatur (PLG-007), fragt Berechtigungen ab (PLG-008) und listet es danach in `beton plugin list`.
  - [ ] AC2 — Abbruch an beliebiger Stelle hinterlässt keinen halb installierten Zustand (atomares Verschieben aus Staging-Verzeichnis).
  - [ ] AC3 — Lokale Pfad-Installationen sind als `source: path` markiert und werden in UI/doctor als „unverifiziert“ gekennzeichnet; sie gelingen ohne Netzwerk und ohne konfigurierten Index (Test im Netz-Namespace nur mit Loopback).
  - [ ] AC4 — Im zentralen Modus darf nur die Rolle Admin/Owner Plugins installieren (API liefert 403 für Member).
- **Abhängigkeiten:** PLG-004, PLG-006, PLG-007, PLG-008
- **Referenz:** ADR-0018 (`beton plugin install <name>` Registry/Git/Pfad)

### PLG-006 — Registry-Index als Git-Repository
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Der offizielle Index ist ein Git-Repository (`github.com/ifahrentholz/beton-plugins` *(Annahme)*) mit einer Datei pro Plugin (`plugins/<name>.toml`), die Versionen, Manifest-URL, Manifest-SHA-256 und die erwartete cosign-Identität enthält. Aufnahme per Pull-Request. beton klont den Index flach in einen Cache und aktualisiert ihn nur bei ausdrücklichem `install <name>`/`search`/`update` (max. 1×/h), nie im Hintergrund. Die Registry ist optional; ohne sie bleibt die Installation aus lokalem Pfad möglich (PLG-005, ADR-0033). Weitere Indizes konfigurierbar (`plugins.registries`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton plugin search runner` listet passende Einträge aus dem gecachten Index, auch offline (mit Hinweis auf Alter des Caches).
  - [ ] AC2 — Weicht die Manifest-Checksumme vom Index-Eintrag ab, wird die Installation abgebrochen.
  - [ ] AC3 — Ein zusätzlicher Index (`plugins.registries: [{ name: acme, url: git+https://git.acme/beton-index }]`) wird durchsucht; Namenskonflikte erfordern `acme/<name>`.
- **Abhängigkeiten:** PLG-004
- **Referenz:** Cargo-/Homebrew-Index-Modell

### PLG-007 — Signatur- & Checksum-Prüfung
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Jedes heruntergeladene Binary-Archiv wird gegen die SHA-256 aus dem Manifest geprüft. Zusätzlich wird ein Sigstore-Bundle (`.sigstore.json`) gegen die im Registry-Index gepinnte cosign-Identität/Issuer verifiziert (Bibliothek `sigstore-rs`). Unsignierte Plugins (Git/Pfad ohne Signatur) erfordern `--allow-unsigned` und erhalten eine dauerhafte Warnmarkierung.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein manipuliertes Archiv (1 Byte verändert) wird mit Fehler `checksum mismatch` abgelehnt; nichts wird installiert.
  - [ ] AC2 — Eine gültige Signatur einer anderen Identität als im Index gepinnt wird abgelehnt.
  - [ ] AC3 — Ohne `--allow-unsigned` schlägt die Installation eines unsignierten Git-Plugins fehl; mit Flag gelingt sie und `beton plugin list` zeigt `unsigned`.
  - [ ] AC4 — Vor jedem Plugin-Start wird die Checksumme der installierten Binary erneut geprüft; Abweichung → Start verweigert.
- **Abhängigkeiten:** PLG-005, PLG-006; DIST-013 (siehe 12-distribution-quality.md)
- **Referenz:** ADR-0026 (Sigstore/cosign)

### PLG-008 — Berechtigungen & Bestätigung
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Plugins deklarieren im Manifest `network`, `fs_read`, `fs_write`, `env`, `secrets`, `exec`. Bei Installation und bei jedem Update mit erweiterten Berechtigungen zeigt beton die (Diff-)Liste und verlangt explizite Bestätigung. Nicht-interaktiv nur mit `--accept-permissions=<sha256 der Berechtigungsliste>`. Gewährte Berechtigungen werden in `installed.toml` gespeichert.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Update, das `network` um einen Host erweitert, pausiert mit Diff-Anzeige; ohne Bestätigung bleibt die alte Version aktiv.
  - [ ] AC2 — `--yes` allein reicht nicht für neue Berechtigungen; nur `--accept-permissions` mit passendem Hash.
  - [ ] AC3 — `beton plugin info <name>` zeigt gewährte Berechtigungen und Zeitpunkt der Bestätigung.
- **Abhängigkeiten:** PLG-004
- **Referenz:** ADR-0018 (deklarierte Berechtigungen)

### PLG-009 — Durchsetzung der Plugin-Berechtigungen
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Plugin-Prozesse laufen in der OS-Sandbox (SBX-001, SBX-002, siehe 04-sandbox.md) mit genau den gewährten Pfaden; Netzwerk nur über den Egress-Proxy mit `permissions.network` als Allowlist; Env deny-by-default; Secrets ausschließlich als `bt_cred_*`-Platzhalter mit Proxy-Injection. Ist die Sandbox nicht verfügbar, startet das Plugin nicht (fail closed).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Test-Plugin, das außerhalb von `fs_read` liest (`~/.ssh/id_ed25519`), erhält einen Fehler; der Versuch wird geloggt.
  - [ ] AC2 — Ein HTTP-Request des Plugins an einen nicht deklarierten Host wird vom Proxy mit 403 abgelehnt.
  - [ ] AC3 — Das Plugin sieht für ein deklariertes Secret nur den Platzhalter; ein Request an den gebundenen Host erreicht den Upstream mit echtem Wert (Proxy-Integrationstest).
  - [ ] AC4 — Auf einem System ohne verfügbare Sandbox schlägt der Plugin-Start mit Verweis auf `beton doctor` fehl.
- **Abhängigkeiten:** PLG-003, PLG-008; SBX-001, PRX-003, PRX-006 (siehe 04-sandbox.md)
- **Referenz:** ADR-0007, ADR-0018

### PLG-010 — Versionierung & Kompatibilität
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Das Plugin-Protokoll hat eine eigene Major-Version (`plugin_api`, Start `1`). beton unterstützt die aktuelle und die vorherige Major-Version (N, N-1) *(Annahme)*; Erweiterungen innerhalb einer Major-Version werden über `capabilities` ausgehandelt, nie durch Pflichtfelder. Zusätzlich prüft beton die `beton`-SemVer-Range des Manifests bei Installation und Start. `beton plugin update` wählt die höchste kompatible Version.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Plugin mit `beton = ">=2.0"` wird auf beton 1.x nicht installiert (Fehler mit kompatiblen Versionen aus dem Index).
  - [ ] AC2 — `beton plugin update` überspringt Versionen mit inkompatibler `plugin_api` und meldet dies.
  - [ ] AC3 — Unbekannte Felder in Plugin-Antworten werden ignoriert (Forward-Compat-Test).
- **Abhängigkeiten:** PLG-002, PLG-004
- **Referenz:** ADR-0026 (SemVer), ADR-0019 (Versionsaushandlung)

### PLG-011 — Plugin-Verwaltung
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** `beton plugin list|info|update|remove|enable|disable|doctor` (CLI-Oberfläche: CLI-012). `doctor` führt Handshake, Capability-Abfrage und Checksumme aus und zeigt Status (`ok`, `disabled`, `incompatible`, `crashlooping`, `unsigned`). Entfernen löscht Binaries und Konfiguration, nicht aber von Runner-Providern erzeugte Ressourcen (Reaper-Hinweis).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton plugin list --json` liefert `name`, `version`, `kind`, `source`, `status`, `signed`.
  - [ ] AC2 — Ein deaktiviertes Plugin wird nicht gestartet; Sessions, die es benötigen, schlagen mit klarer Meldung fehl.
  - [ ] AC3 — `beton plugin remove` eines Runner-Providers mit aktiven Runnern verweigert ohne `--force` und listet die betroffenen Runner.
- **Abhängigkeiten:** PLG-003, PLG-005
- **Referenz:** Omnigent `omni extensions list|doctor`

### PLG-012 — Rust-Plugin-SDK & Beispiel-Plugin
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Crate `beton-plugin-sdk` *(Annahme: zusätzliches Crate, veröffentlicht auf crates.io)* stellt Traits `RunnerProviderPlugin`, `HarnessPlugin`, `GitProviderPlugin`, `serve_stdio(plugin)`, Typen des Protokolls, Manifest-Validierung und Test-Helfer (`testing::PluginTestHost`) bereit. `beton plugin new --kind <kind> <name>` erzeugt ein Template. Beispiel-Plugin `examples/plugins/echo-harness` (Harness, der Eingaben deterministisch zurückspielt) dient als Referenz und CI-Fixture.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton plugin new --kind runner_provider demo` erzeugt ein Projekt, das ohne Änderung mit `cargo build` baut und `beton plugin doctor` besteht.
  - [ ] AC2 — `echo-harness` lässt sich per `beton plugin install ./examples/plugins/echo-harness` installieren; `beton run echo-harness -p "hi"` liefert `hi` als Agent-Nachricht.
  - [ ] AC3 — Die SDK-Dokumentation (rustdoc) enthält für jede Protokollmethode ein lauffähiges Beispiel (Doctests in CI).
- **Abhängigkeiten:** PLG-002, PLG-004
- **Referenz:** ADR-0018

### PLG-013 — Plugin-Conformance-Tests
- **Meilenstein:** M5 · **Priorität:** Should
- **Beschreibung:** `beton plugin test <pfad|name>` führt die kind-spezifische Contract-Suite (dieselbe wie für eingebaute Adapter) gegen ein Plugin aus: Handshake, deklarierte vs. beobachtete Capabilities, Idempotenz, Cancel, Fehlercodes, Timeouts. Ergebnis als JUnit-XML für Plugin-CI.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Runner-Provider-Plugin, das `snapshot: true` deklariert, aber `Unsupported` liefert, fällt durch den Test mit konkreter Meldung.
  - [ ] AC2 — `--junit out.xml` erzeugt valides JUnit-XML.
  - [ ] AC3 — `echo-harness` besteht die Harness-Suite vollständig (CI-Gate des Repos).
- **Abhängigkeiten:** PLG-012, PLG-001
- **Referenz:** Omnigent `tests/harness_bench`

### PLG-014 — WASM-Policy-Plugins (wasmtime + WIT)
- **Meilenstein:** M5 · **Priorität:** Could
- **Beschreibung:** Policies können als zusätzlichen Regeltyp eine WebAssembly-Komponente referenzieren, die in `beton-plugin` per `wasmtime` (Component Model) ausgeführt wird. Die Schnittstelle ist eine WIT-Welt `beton:policy`; das Modul erhält Hook-Name und Kontext (dieselben Variablen wie CEL, als JSON) und liefert optional eine Entscheidung. Strikte Isolation: keine WASI-Dateisystem-/Netzzugriffe, Fuel-/Epoch-Limit, Speicherlimit.
- **Details:**
  ```wit
  package beton:policy@0.1.0;
  world policy-extension {
    enum action { allow, deny, ask, notify }
    record decision { action: action, reason: option<string>, modify-json: option<string> }
    import log: func(level: u8, msg: string);
    export evaluate: func(phase: string, context-json: string, params-json: string) -> result<option<decision>, string>;
  }
  ```
  Einbindung im Policy-YAML (Syntax-Hoheit bei POL-028, siehe 03-policies.md): `wasm: { module: ./risk.wasm, sha256: "…", params: {…} }` oder Plugin-Referenz `module: "acme/risk@1.2.0"`. PLG-014 ist Owner von WIT-Welt und wasmtime-Host. Limits: 64 MiB Speicher, 50 ms Wall-Clock pro Aufruf, Ergebnis-JSON ≤ 64 KiB. Laufzeitfehler/Timeouts werden wie `deny` behandelt (fail closed).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Beispielmodul, das bei `tool.name == "bash"` `ask` liefert, führt in einer Session zu einer Approval-Card (Integrationstest mit Fake-Harness).
  - [ ] AC2 — Ein Modul mit Endlosschleife wird nach ≤ 50 ms abgebrochen; Ergebnis `deny` mit Grund `wasm timeout`.
  - [ ] AC3 — Ein Modul, das WASI-Dateisystem- oder Socket-Imports benötigt, wird beim Laden abgelehnt.
  - [ ] AC4 — Stimmt `sha256` nicht mit dem Modul überein, wird die Policy nicht geladen und die Session startet nicht (fail closed).
- **Abhängigkeiten:** POL-002, POL-028 (siehe 03-policies.md)
- **Referenz:** ADR-0008 (später WASM), ADR-0018

## Nicht in v1

- **UI-Extensions** (sandboxed iframe + Message-Bridge) — v2 (ADR-0018).
- **SaaS-Sandbox-/Runner-Provider** (E2B, Daytona, Modal, Fly) — v2; können später als `runner_provider`-Plugins entstehen.
- **MicroVMs** als Runner-Isolation — v2 (Community-Provider).
- **KMS-/Vault-Master-Key-Backends als Plugin** — v2 (siehe 05-security-identity.md).
