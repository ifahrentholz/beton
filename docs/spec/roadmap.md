# Roadmap

> Generiert aus den Feature-Einträgen in `docs/spec/*.md`. Maßgeblich ist immer der Meilenstein **im Feature-Eintrag**; diese Datei ist eine Sicht darauf. Neu generieren: `python3 scripts/gen_roadmap.py`.

Grundlage: ADR-0030 (Meilensteine M0–M5, erstes öffentliches Release nach M3, Solo-Entwicklung mit Coding-Agents).

## Übersicht

| Meilenstein | Thema | Must | Should | Could | Gesamt |
|---|---|---:|---:|---:|---:|
| [M0](#m0) | Fundament | 67 | 1 | 0 | 68 |
| [M1](#m1) | Meta-Harness | 33 | 20 | 0 | 53 |
| [M2](#m2) | Kontrolle | 64 | 9 | 2 | 75 |
| [M3](#m3) | Desktop & TUI → öffentliches Release 0.1 | 68 | 15 | 2 | 85 |
| [M4](#m4) | Team | 57 | 8 | 1 | 66 |
| [M5](#m5) | Autonomie & Breite → v1.0 | 35 | 9 | 3 | 47 |
| **Summe v1.0** | | **324** | **62** | **8** | **394** |

**Arbeitsweise:** Innerhalb eines Meilensteins zuerst alle *Must*-Features (in Abhängigkeitsreihenfolge, siehe Feld *Abhängigkeiten*), dann *Should*. *Could*-Features sind Stretch-Goals und blockieren den Meilenstein nicht. Ein Meilenstein gilt als erreicht, wenn alle *Must*-Features grün sind **und** das Demo-Szenario manuell (und so weit möglich als E2E-Test) funktioniert.

## M0

### M0 — Fundament

**Umfang**

- Cargo-Workspace, Event-Modell und Wire-Protokoll, SQLite-Event-Log
- Lokaler Daemon (`serve` + lokaler Runner), CLI-Grundgerüst
- Claude-Code-Adapter (stream-json) inkl. Permission-Bridge
- Minimale Web-UI (Session-Liste, Chat-Stream, Composer)
- Test-Fundament: Fake-Harness, Golden Transcripts, Schema-Snapshots, CI-Gates, Offline-E2E ohne Netzwerk (QA-018)

**Demo-Szenario (Exit-Kriterium):** `beton run claude` startet eine persistente Claude-Code-Session (Subscription über die offizielle CLI), die im Browser live mitläuft, nach einem Verbindungsabbruch ab `seq` fortgesetzt wird und einen Neustart des Daemons überlebt.

<details><summary>Features (68)</summary>


**[01 — Harnesses](01-harnesses.md)**

| ID | Feature | Priorität |
|---|---|---|
| HAR-001 | Adapter-Trait & Session-Lebenszyklus | Must |
| HAR-002 | Capabilities-Modell & Harness-Katalog | Must |
| HAR-003 | Harness-Registry, Binary-Auflösung & Versionsprüfung | Must |
| HAR-004 | Claude-Code-Adapter (stream-json) | Must |
| HAR-005 | Claude-Code-Permission-Bridge | Must |
| HAR-015 | Subscription-Regel & Auth-Herkunft | Must |
| HAR-016 | Credential-Erkennung in `beton setup` | Must |
| HAR-021 | Usage- & Kontext-Reporting | Should |
| HAR-025 | Golden-Transcript-Tests | Must |
| HAR-026 | Fake-Harness | Must |

**[05 — Security & Identity](05-security-identity.md)**

| ID | Feature | Priorität |
|---|---|---|
| AUTH-001 | Lokaler Modus: Loopback-Bindung & Token-Datei | Must |
| AUTH-002 | Schutz gegen DNS-Rebinding & CSRF | Must |
| AUTH-003 | WebSocket-Origin-Allowlist | Must |
| AUTH-004 | Lokale Browser-Anmeldung per Einmal-Link | Must |

**[06 — Daten, Sync & Protokoll](06-data-sync-protocol.md)**

| ID | Feature | Priorität |
|---|---|---|
| PROTO-001 | Event-Envelope & ID-Schema | Must |
| PROTO-002 | Event-Katalog v1 | Must |
| PROTO-003 | Transiente Events & Delta-Ringpuffer | Must |
| PROTO-004 | WebSocket-Handshake & Versionsaushandlung | Must |
| PROTO-005 | Attach & Resume ab seq | Must |
| PROTO-006 | Client-Kommandos über WebSocket | Must |
| PROTO-008 | Backpressure & Batching | Must |
| PROTO-009 | Heartbeats & Reconnect | Must |
| PROTO-010 | REST-Konventionen | Must |
| PROTO-011 | Fehlerformat RFC 9457 | Must |
| PROTO-013 | Schema-Generierung aus Rust & Snapshot-Tests | Must |
| PROTO-015 | Tunnel-Protokoll Host/Runner → Server | Must |
| DATA-001 | Datenmodell & Entitäten | Must |
| DATA-002 | Event-Log-Speicherung (Append-only) | Must |
| DATA-003 | SQLite-Backend & Migrationen | Must |
| DATA-005 | Projektionen & Read-Modelle | Must |
| DATA-006 | Blob-Store: Dateisystem | Must |
| DATA-008 | Archivieren & Löschen | Must |

**[07 — Sessions, Collaboration & Git-Provider](07-sessions-collaboration.md)**

| ID | Feature | Priorität |
|---|---|---|
| SES-001 | Session-Lebenszyklus (create / archive / delete) | Must |
| SES-002 | Multi-Client-Live-Stream mit Resume ab seq | Must |
| SES-003 | Resume gestoppter Sessions | Must |
| SES-005 | Interrupt | Must |

**[08 — Clients: Desktop, Web/PWA, CLI, TUI, API & SDKs](08-clients.md)**

| ID | Feature | Priorität |
|---|---|---|
| WEB-001 | App-Shell, Routing & Layout | Must |
| WEB-002 | Chat-Stream-Rendering | Must |
| WEB-003 | Session-Liste | Must |
| WEB-004 | Composer mit Pickern | Must |
| WEB-015 | Performance-Budgets | Must |
| WEB-018 | Minimale Approval-Karte (M0) | Must |
| CLI-001 | CLI-Grundgerüst & Ausgabekonventionen | Must |
| CLI-002 | `beton run` | Must |
| CLI-003 | `resume`, `attach` & `session`-Verwaltung | Must |
| CLI-004 | `serve` & `host` | Must |
| CLI-005 | `setup`, `doctor` & `diagnose` | Must |
| CLI-008 | `config` | Must |
| API-001 | REST-API mit OpenAPI 3.1 (utoipa) | Must |
| API-002 | Ressourcen-Übersicht | Must |
| API-004 | TypeScript-SDK | Must |
| API-005 | Rust-SDK | Must |
| API-006 | Skript-Modus `beton run -p` | Must |

**[10 — Runner & Erweiterbarkeit](10-runners-extensibility.md)**

| ID | Feature | Priorität |
|---|---|---|
| RUN-001 | RunnerProvider-Trait & Capabilities | Must |
| RUN-002 | Lokaler Runner-Provider | Must |
| RUN-003 | Runner-Lebenszyklus & Supervision | Must |
| PLG-001 | Eingebaute Adapter als Crates | Must |

**[11 — Plattform-Features](11-platform-features.md)**

| ID | Feature | Priorität |
|---|---|---|
| OBS-001 | Strukturierte Logs (`tracing`) | Must |
| OBS-005 | `beton doctor` | Must |

**[12 — Distribution & Qualität](12-distribution-quality.md)**

| ID | Feature | Priorität |
|---|---|---|
| QA-001 | Teststrategie & Testpyramide | Must |
| QA-002 | Protokoll-Fake-CLIs | Must |
| QA-003 | Golden-Transcript-Gate & Drift-Prozess | Must |
| QA-006 | Schema- & OpenAPI-Snapshot-Tests | Must |
| QA-007 | E2E-Tests Web (Playwright) | Must |
| QA-010 | CI-Gates | Must |
| QA-014 | Agent-Workflow & Feature-IDs | Must |
| QA-015 | DCO-Check | Must |
| QA-018 | Offline-E2E-Garantie (Netz-Namespace nur mit Loopback) | Must |

</details>

## M1

### M1 — Meta-Harness

**Umfang**

- Codex-Adapter (app-server), generisches ACP, Direkt-API-/Gateway-Harness
- Fork über Harness-Grenzen mit Handover-Kontext, Import, Export
- Agent-YAML + JSON-Schema, MCP als Tool-Mechanismus, System-Tools, Skills, Sub-Agents
- Built-in-Agents `maestra` und `duetto`

**Demo-Szenario (Exit-Kriterium):** Eine Session startet auf Claude Code, wird auf Codex geforkt und weitergeführt; `maestra` lässt Claude implementieren und Codex reviewen; vorhandene Claude- und Codex-Chats lassen sich importieren.

<details><summary>Features (53)</summary>


**[01 — Harnesses](01-harnesses.md)**

| ID | Feature | Priorität |
|---|---|---|
| HAR-006 | Codex-Adapter (app-server) | Must |
| HAR-007 | Generischer ACP-Harness | Must |
| HAR-008 | ACP-Agent-Registrierung & Presets | Must |
| HAR-009 | MCP-Injektion & System-Tool-Bridge | Must |
| HAR-010 | Direkt-API-/Gateway-Harness: Agent-Loop | Must |
| HAR-011 | Provider-Konfiguration & Wire-APIs | Must |
| HAR-017 | Modell- & Effort-Wechsel mid-session | Must |
| HAR-018 | Handover-Kontext für Harness-Wechsel & Fork | Must |
| HAR-019 | Native History-Rebuild (gleicher Harness) | Should |
| HAR-020 | Resume nach Runner-Neustart | Should |
| HAR-022 | Compaction-Durchreichung | Should |
| HAR-023 | Transcript-Import: Claude-Code-Parser | Must |
| HAR-024 | Transcript-Import: Codex-Parser | Must |
| HAR-027 | Permission-Mode-Mapping | Should |

**[02 — Agents & Automation](02-agents.md)**

| ID | Feature | Priorität |
|---|---|---|
| AGT-001 | Agent-Format v1 (YAML) | Must |
| AGT-002 | JSON-Schema-Veröffentlichung & Validierung | Must |
| AGT-003 | Agent-Verzeichnis & Auflösung | Must |
| AGT-004 | Executor & Agent-Snapshot pro Session | Must |
| AGT-005 | Instructions & Auslieferung | Must |
| AGT-006 | Tools = MCP-Server (Agent/Projekt/User) | Must |
| AGT-007 | System-Tools via MCP-Server `beton` | Must |
| AGT-008 | Skills (`SKILL.md`) | Must |
| AGT-009 | Sub-Agents über Harness-Grenzen | Must |
| AGT-010 | Parameter (`params`) | Should |
| AGT-011 | Built-in-Agent `maestra` (Orchestrator) | Must |
| AGT-012 | Built-in-Agent `duetto` (Debatte) | Should |
| AGT-013 | Agent-CLI | Should |

**[06 — Daten, Sync & Protokoll](06-data-sync-protocol.md)**

| ID | Feature | Priorität |
|---|---|---|
| PROTO-012 | SSE read-only | Should |
| DATA-010 | Export-/Import-Format (JSONL) | Should |

**[07 — Sessions, Collaboration & Git-Provider](07-sessions-collaboration.md)**

| ID | Feature | Priorität |
|---|---|---|
| SES-004 | Queue & Steer | Must |
| SES-006 | Fork ab Event X | Must |
| SES-007 | Fork auf anderen Harness mit Handover-Kontext | Must |
| SES-008 | Import fremder Chats (Session-Seite) | Must |
| SES-009 | Session-Export/-Import (JSONL) | Should |
| SES-010 | Automatische Session-Titel | Should |
| SES-011 | Compaction & Kontextanzeige (Session-Anbindung) | Should |
| SES-012 | Session-Liste, Suche & Unread-State | Must |
| SES-015 | Worktree pro Session (Erstellung & Base-Branch) | Must |
| SES-016 | Worktree-Aufräumen | Must |
| SES-017 | Workspace-API: Files & Suche | Must |
| SES-018 | Workspace-API: Changes & Diffs | Must |

**[08 — Clients: Desktop, Web/PWA, CLI, TUI, API & SDKs](08-clients.md)**

| ID | Feature | Priorität |
|---|---|---|
| WEB-005 | Queue- & Steer-UI | Must |
| WEB-006 | Attachments, @-Mentions & Slash-Menü | Should |
| WEB-008 | Workspace-Rail mit Tabs | Must |
| WEB-009 | Code-Editor (Monaco) | Should |
| WEB-011 | Diff-Ansicht | Must |
| WEB-012 | Sub-Agent-Graph (xyflow) | Should |
| CLI-007 | `import` & `export` | Should |
| CLI-009 | `agent` | Should |
| API-003 | SSE-Stream für Skripte | Should |

**[11 — Plattform-Features](11-platform-features.md)**

| ID | Feature | Priorität |
|---|---|---|
| UX-007 | Interne Feature-Flags | Must |
| UX-009 | Automatische Session-Titel (UI) | Should |

**[12 — Distribution & Qualität](12-distribution-quality.md)**

| ID | Feature | Priorität |
|---|---|---|
| QA-016 | Harness-Contract-Suite | Should |

</details>

## M2

### M2 — Kontrolle

**Umfang**

- CEL-Policies mit Hierarchie (User → Projekt → Agent), Regeltypen, Approvals, Policy-Tests, Explain
- Usage/Kosten inkl. Subscription-Usage, Inbox
- Sandbox macOS (Seatbelt) + Linux (Landlock/seccomp), zweistufig, Escape-Suite in der CI
- Egress- und Credential-Proxy, lokaler Secret-Store, Audit, Redaction

**Demo-Szenario (Exit-Kriterium):** Ein Agent läuft im YOLO-Mode in der Sandbox: Er kann `~/.ssh` nicht lesen und nur erlaubte Hosts erreichen, er sieht Secrets nur als `bt_cred_*`, `git push --force` erzeugt eine Approval-Card, und bei Überschreitung des Budgets wird gestoppt.

<details><summary>Features (75)</summary>


**[02 — Agents & Automation](02-agents.md)**

| ID | Feature | Priorität |
|---|---|---|
| AGT-014 | Agent-Policies & Sandbox-Block | Must |

**[03 — Policies](03-policies.md)**

| ID | Feature | Priorität |
|---|---|---|
| POL-001 | Policy-Dokumentformat, Ladeorte & Hot-Reload | Must |
| POL-002 | CEL-Engine & Erweiterungsfunktionen | Must |
| POL-003 | Hook-Phasen & Evaluationspunkte | Must |
| POL-004 | Kontext-Variablen je Phase | Must |
| POL-005 | Tool-Normalisierung | Must |
| POL-006 | Aktionen & Entscheidungsmodell | Must |
| POL-007 | Hierarchie & Kombination (lokale Ebenen) | Must |
| POL-009 | Zustand & Zähler | Must |
| POL-010 | Approvals (`ask`) & Approval-Card | Must |
| POL-011 | Regeltyp `spend_cap` | Must |
| POL-012 | Regeltyp `daily_budget` | Must |
| POL-013 | Regeltyp `model_route` | Must |
| POL-014 | Regeltyp `require_approval` | Must |
| POL-015 | Regeltyp `tool_allowlist` | Must |
| POL-016 | Regeltyp `path_guard` | Must |
| POL-017 | Regeltyp `git_guard` | Must |
| POL-018 | Regeltyp `loop_detection` | Should |
| POL-019 | Regeltyp `pii_redaction` | Could |
| POL-023 | Durchsetzung bei ACP (Permission-Requests) | Must |
| POL-024 | Enforcement-Matrix & Fail-Closed beim Start | Must |
| POL-025 | Audit: `policy.decision`-Event | Must |
| POL-026 | Deklarative Policy-Tests (YAML) | Must |
| POL-027 | Dry-Run, Shadow-Modus & Explain | Must |

**[04 — Sandbox & Egress-/Credential-Proxy](04-sandbox.md)**

| ID | Feature | Priorität |
|---|---|---|
| SBX-001 | Sandbox-Provider-Trait & Registry | Must |
| SBX-002 | Zweistufiges Modell & Exec-Broker | Must |
| SBX-003 | Sandbox-Konfiguration & Pfadauflösung | Must |
| SBX-004 | Default-Masken für Dotfiles & Credential-Pfade | Must |
| SBX-005 | Environment deny-by-default | Must |
| SBX-006 | Fail-closed: verlangt, aber nicht verfügbar → Fehler | Must |
| SBX-007 | macOS-Backend: Seatbelt (SBPL, deny-default) | Must |
| SBX-008 | Linux-Backend: Landlock + seccomp + Namespaces | Must |
| SBX-009 | Linux-Fallback: bubblewrap | Should |
| SBX-010 | Capability-Probe & `beton sandbox`-CLI | Must |
| SBX-011 | Sandbox-Escape-Testsuite | Must |
| SBX-012 | Eingebaute Presets | Should |
| SBX-013 | Ressourcenlimits & Prozessbaum-Lebenszyklus | Should |
| SBX-014 | Verstoß-Reporting | Could |
| PRX-001 | Proxy-Kern & Instanzmodell | Must |
| PRX-002 | beton-CA pro Installation & TLS-MITM | Must |
| PRX-003 | Regel-Syntax & Matching (Default-Deny) | Must |
| PRX-004 | HTTP/2, WebSocket & Protokollgrenzen | Must |
| PRX-005 | Passthrough für Vendor-Hosts (Stufe 1) | Must |
| PRX-006 | Credential-Injection mit Platzhaltern | Must |
| PRX-007 | Private-IP- & DNS-Rebinding-Schutz | Must |
| PRX-008 | Erzwingung „Netz nur zum Proxy“ & Toolchain-Integration | Must |
| PRX-009 | Logging, Events & Audit des Egress | Must |
| PRX-010 | Antwort-Redaction echter Secret-Werte | Should |

**[05 — Security & Identity](05-security-identity.md)**

| ID | Feature | Priorität |
|---|---|---|
| SEC-001 | Secret-Store-Abstraktion & SecretRefs | Must |
| SEC-002 | Lokaler Store: OS-Keychain | Must |
| SEC-003 | Lokaler Fallback: verschlüsselte Datei (Argon2id) | Must |
| SEC-004 | Secret-Verwaltung über CLI & UI | Must |
| SEC-011 | Git-PAT-Fallback | Must |
| SEC-012 | Audit-Log | Must |
| SEC-013 | Secret-Redaction in Logs, Events & Transkripten | Must |

**[06 — Daten, Sync & Protokoll](06-data-sync-protocol.md)**

| ID | Feature | Priorität |
|---|---|---|
| DATA-012 | Payload-Redaktion | Must |

**[08 — Clients: Desktop, Web/PWA, CLI, TUI, API & SDKs](08-clients.md)**

| ID | Feature | Priorität |
|---|---|---|
| WEB-007 | Approval-Bar | Must |
| CLI-010 | `usage` | Should |

**[11 — Plattform-Features](11-platform-features.md)**

| ID | Feature | Priorität |
|---|---|---|
| USE-001 | Usage-Records & Normalisierung | Must |
| USE-002 | Versionierter Preis-Katalog | Must |
| USE-003 | Eigene Preise für Gateway-Modelle | Must |
| USE-004 | Subscription-Usage | Must |
| USE-005 | Usage-Aggregation & API | Must |
| USE-006 | Usage-Seite | Must |
| USE-007 | CLI `beton usage` | Must |
| USE-008 | Kontextfenster-Anzeige | Should |
| USE-009 | Compaction-Trigger | Should |
| UX-001 | Inbox | Must |
| OBS-002 | Secret-Redaction | Must |
| OBS-003 | OpenTelemetry-Traces (Trace pro Session) | Should |

**[12 — Distribution & Qualität](12-distribution-quality.md)**

| ID | Feature | Priorität |
|---|---|---|
| QA-004 | Deklarative Policy-Tests | Must |
| QA-005 | Sandbox-Escape-Suite pro OS | Must |
| QA-009 | Property- & Fuzz-Tests (Proxy-Parser, CEL) | Must |
| QA-011 | Coverage-Floor für `beton-policy` und `beton-sandbox` | Must |
| QA-013 | TDD- & Review-Pflicht für sicherheitskritische Crates | Must |

</details>

## M3

### M3 — Desktop & TUI → öffentliches Release 0.1

**Umfang**

- Tauri-2-Desktop-App, ratatui-TUI, Native-TUI/PTY-Modus mit Hooks
- Projects, Worktrees, Terminals
- Eingebetteter Browser (CDP-Screencast), Agent-Tools, Inspect-Mode
- Lokales Whisper, Command-Palette, Themes, Onboarding
- Release-Pipeline: Signing, Notarisierung, Homebrew, Installer, Updates (Prüfung nur auf Klick/opt-in), Subscription-Checkliste pro Release (QA-019)

**Demo-Szenario (Exit-Kriterium):** Ein neuer Nutzer installiert die signierte Desktop-App, `beton setup` erkennt die CLI-Logins, und er arbeitet mit mehreren Sessions in Worktrees, nutzt den Inspect-Mode im eingebetteten Browser und diktiert Prompts per Push-to-Talk – vollständig lokal.

<details><summary>Features (85)</summary>


**[01 — Harnesses](01-harnesses.md)**

| ID | Feature | Priorität |
|---|---|---|
| HAR-012 | Native-TUI-/PTY-Modus | Must |
| HAR-013 | Claude-Code-TUI: Hooks-Integration | Must |
| HAR-014 | Codex-TUI: Approval-Integration | Must |

**[03 — Policies](03-policies.md)**

| ID | Feature | Priorität |
|---|---|---|
| POL-020 | Regeltyp `browser_guard` | Must |
| POL-022 | Durchsetzung im Native-TUI-Modus (Vendor-Hooks) | Must |

**[04 — Sandbox & Egress-/Credential-Proxy](04-sandbox.md)**

| ID | Feature | Priorität |
|---|---|---|
| SBX-017 | PTY-/Native-TUI-Modus unter Sandbox | Must |
| PRX-011 | Browser-Traffic über den Proxy | Must |

**[06 — Daten, Sync & Protokoll](06-data-sync-protocol.md)**

| ID | Feature | Priorität |
|---|---|---|
| PROTO-007 | Binärkanäle & Flusskontrolle | Must |
| PROTO-014 | Kompatibilitätsprüfung der Schemas | Must |
| DATA-011 | Ephemere Daten & Snapshots | Must |

**[07 — Sessions, Collaboration & Git-Provider](07-sessions-collaboration.md)**

| ID | Feature | Priorität |
|---|---|---|
| SES-013 | Projects: Gruppierung & Defaults | Must |
| SES-014 | Projects als Policy-Ebene | Must |
| SES-019 | Workspace-Terminals | Must |

**[08 — Clients: Desktop, Web/PWA, CLI, TUI, API & SDKs](08-clients.md)**

| ID | Feature | Priorität |
|---|---|---|
| DESK-001 | Tauri-2-App-Shell & Fenster | Must |
| DESK-002 | Lokaler Daemon-Start & -Management | Must |
| DESK-004 | Tray & Dock-/Taskbar-Badge | Should |
| DESK-005 | Native Notifications nur für nicht betrachtete Sessions | Must |
| DESK-006 | Deep-Links `beton://` | Must |
| DESK-007 | Auto-Update | Must |
| DESK-008 | Globaler Push-to-Talk-Shortcut | Should |
| WEB-010 | Terminals (xterm.js) | Must |
| WEB-016 | Barrierefreiheit (Basics) | Should |
| WEB-017 | Internationalisierung DE/EN | Should |
| CLI-013 | `upgrade` & `uninstall` | Must |
| CLI-014 | Shell-Completion | Could |
| TUI-001 | ratatui-TUI: Session-Ansicht | Must |
| TUI-002 | Eigenes PTY-Multiplexing & Panes | Must |
| TUI-003 | Native-TUI-Modus (Vendor-TUI im Pane) | Must |
| TUI-004 | Approval-Handling in der TUI | Must |
| TUI-005 | TUI auf Windows | Should |

**[09 — Eingebetteter Browser & Inspect-Mode](09-browser.md)**

| ID | Feature | Priorität |
|---|---|---|
| BRW-001 | Chromium-Discovery | Must |
| BRW-002 | Chrome for Testing auf Nachfrage | Must |
| BRW-003 | CDP-Lebenszyklus pro Session | Must |
| BRW-004 | Isoliertes Profil pro Session | Must |
| BRW-005 | Screencast-Streaming ins Panel | Must |
| BRW-006 | Input-Weiterleitung | Must |
| BRW-007 | Adaptive Framerate/Qualität & HiDPI | Should |
| BRW-008 | Umschalter auf sichtbares Fenster | Must |
| BRW-009 | Browser-Panel: Tabs & Navigation | Must |
| BRW-010 | Agent-Tools (Basis) | Must |
| BRW-011 | Accessibility-Snapshot mit stabilen refs | Must |
| BRW-012 | `browser_eval` nur mit Policy | Should |
| BRW-013 | Inspect-Mode / Element-Picker | Must |
| BRW-014 | Picker-Payload an den Agent | Must |
| BRW-015 | Mehrfachauswahl im Picker | Should |
| BRW-016 | Quelldatei-Mapping (Stretch) | Could |
| BRW-017 | Erkennung lokaler Dev-Server | Should |
| BRW-018 | Browser-Traffic durch den Egress-Proxy | Must |
| BRW-019 | Browser-Policies: Navigation, Formular-Submit, Downloads | Must |
| BRW-020 | Sichtbarkeit für Co-Viewer | Must |

**[11 — Plattform-Features](11-platform-features.md)**

| ID | Feature | Priorität |
|---|---|---|
| VOI-001 | Transkriptions-Engine `beton-voice` | Must |
| VOI-002 | Modellverwaltung on demand mit Checksum | Must |
| VOI-003 | Hardware-Beschleunigung | Must |
| VOI-004 | Audio-Streaming über WS-Binärkanal | Must |
| VOI-005 | Teil-Ergebnisse | Should |
| VOI-006 | Sprache DE/EN mit Autoerkennung | Must |
| VOI-007 | Push-to-Talk (globaler Shortcut im Desktop) | Must |
| VOI-008 | Keine Cloud-Transkription (Garantie) | Must |
| UX-002 | Command-Palette (⌘K) | Must |
| UX-003 | Session-Switcher | Must |
| UX-004 | Tastenkürzel & Shortcuts-Overlay | Must |
| UX-005 | Themes | Should |
| UX-006 | Benachrichtigungs-Einstellungen | Must |
| UX-008 | Onboarding-Wizard (UI-Variante von `beton setup`) | Must |
| UX-010 | MCP-Server-Verwaltung (UI & CLI) | Must |
| OBS-006 | `beton diagnose` (secret-freies Bundle) | Must |
| OBS-007 | Opt-in-Produkt-Telemetrie | Must |
| OBS-008 | Opt-in-Crash-Reports | Should |

**[12 — Distribution & Qualität](12-distribution-quality.md)**

| ID | Feature | Priorität |
|---|---|---|
| DIST-001 | Build-Matrix & reproduzierbare Builds | Must |
| DIST-002 | GitHub Releases & Artefakt-Layout | Must |
| DIST-003 | Installer-Skripte (curl|sh, PowerShell) | Must |
| DIST-004 | Homebrew-Tap | Must |
| DIST-005 | `cargo binstall` | Should |
| DIST-008 | Desktop-Bundles | Must |
| DIST-013 | Signing: macOS, Windows, Sigstore, SBOM, SLSA | Must |
| DIST-014 | Desktop-Auto-Update (Tauri-Updater) | Must |
| DIST-015 | Harness-CLI-Installationsangebot | Must |
| DIST-016 | `beton upgrade` mit Installationsart-Erkennung | Must |
| DIST-017 | Release-Kanäle stable/beta/nightly | Should |
| DIST-018 | Versionierung & Kompatibilitätsfenster | Must |
| DIST-019 | Release-Prozess & Changelog | Must |
| DIST-020 | Deinstallation | Must |
| QA-008 | E2E-Tests Desktop (tauri-driver) | Must |
| QA-012 | Performance-Benchmarks | Should |
| QA-019 | Subscription-Verifikations-Checkliste pro Release | Must |

</details>

## M4

### M4 — Team

**Umfang**

- Zentraler Server mit Postgres + S3, OIDC, Device-Pairing, PATs, Rollen
- Sharing, Co-Drive, Presence, Inline-Kommentare, Side-Chats
- Sync (Single-Writer, Ownership, Fork bei Divergenz), Policy-Cache, Budget-Leases
- Remote-Hosts, PWA + Web-Push, GitHub-/GitLab-Panel, Envelope-Encryption

**Demo-Szenario (Exit-Kriterium):** Zwei Personen melden sich per OIDC an einem zentralen Server an, teilen eine Session, steuern sie gemeinsam, kommentieren inline, verfolgen den PR/MR im Panel; ein Laptop arbeitet offline mit einer Budget-Lease weiter und synchronisiert danach.

<details><summary>Features (66)</summary>


**[03 — Policies](03-policies.md)**

| ID | Feature | Priorität |
|---|---|---|
| POL-008 | Org-/Team-Ebene & Server-Autorität | Must |
| POL-021 | Budget-Leases (Offline) | Must |

**[05 — Security & Identity](05-security-identity.md)**

| ID | Feature | Priorität |
|---|---|---|
| AUTH-005 | OIDC-Login mit Just-in-Time-User-Anlage | Must |
| AUTH-006 | Org-Bootstrap & erster Owner | Must |
| AUTH-007 | Web-Sessions & Cookies | Must |
| AUTH-008 | Device-Pairing per Code (geräteinitiiert) | Must |
| AUTH-009 | Pairing per QR & Freigabe des lokalen Servers fürs Handy | Must |
| AUTH-010 | Geräteverwaltung & Widerruf | Must |
| AUTH-011 | Host- & Runner-Credentials | Must |
| AUTH-012 | Personal Access Tokens mit Scopes | Must |
| AUTH-013 | Service-Accounts | Must |
| AUTH-014 | Rollen Org/Team | Must |
| AUTH-015 | Autorisierungsdurchsetzung & Session-Freigaben | Must |
| AUTH-016 | Offboarding von Mitgliedern | Should |
| AUTH-017 | Passkey-Login (WebAuthn) | Could |
| SEC-005 | Zentrale Envelope-Encryption | Must |
| SEC-006 | Master-Key-Rotation | Must |
| SEC-007 | Bindung & Auflösung von Secrets | Must |
| SEC-008 | Secret-Auslieferung an Runner (Leases) | Must |
| SEC-009 | GitHub-Verbindung per OAuth-App (inkl. Enterprise Server) | Must |
| SEC-010 | GitLab-Verbindung per OAuth (inkl. self-hosted) | Must |
| SEC-014 | Server-Härtung: TLS, Security-Header, Rate-Limits | Must |

**[06 — Daten, Sync & Protokoll](06-data-sync-protocol.md)**

| ID | Feature | Priorität |
|---|---|---|
| DATA-004 | Postgres-Backend | Must |
| DATA-007 | Blob-Store: S3-kompatibel | Must |
| DATA-009 | Retention-Regeln | Should |
| SYNC-001 | Home-Knoten & Single-Writer mit Epochen | Must |
| SYNC-002 | Replikation & Read-Replicas | Must |
| SYNC-003 | Input-Forwarding an den Home-Knoten | Must |
| SYNC-004 | Geplante Ownership-Übernahme | Must |
| SYNC-005 | Erzwungene Übernahme & Divergenz → automatischer Fork | Must |
| SYNC-006 | Policy-Cache (nur verschärfbar) | Must |
| SYNC-007 | Budget-Leases | Must |
| SYNC-008 | Offline-Outbox & Reconnect-Verhalten | Should |

**[07 — Sessions, Collaboration & Git-Provider](07-sessions-collaboration.md)**

| ID | Feature | Priorität |
|---|---|---|
| COL-001 | Session-Freigaben mit Rollen | Must |
| COL-002 | Share-Dialog, Einladungen & „Mit mir geteilt“ | Must |
| COL-003 | Co-Drive mit Autor-Attribution im Modell-Kontext | Must |
| COL-004 | Presence | Should |
| COL-005 | Inline-Kommentare an Nachrichten | Must |
| COL-006 | Inline-Kommentare an Dateien und Diff-Zeilen | Must |
| COL-007 | Kommentare an den Agent adressieren | Must |
| COL-008 | Side-Chats | Must |
| COL-009 | Inbox-Integration | Should |
| COL-010 | Notification-Routing | Must |
| GIT-001 | Git-Provider-Trait & Registry | Must |
| GIT-002 | GitHub-Provider (inkl. Enterprise Server) | Must |
| GIT-003 | GitLab-Provider (inkl. self-hosted) | Must |
| GIT-004 | Provider-Verbindung & Credential-Nutzung | Must |
| GIT-005 | CR-Tracking pro Session (created / attached / inferred) | Must |
| GIT-006 | CR-Panel: Status & CI-Checks/Pipelines | Must |
| GIT-007 | Review-Kommentare im Panel | Should |
| GIT-008 | CR-Diff | Must |
| GIT-009 | CR aus der Session erstellen | Must |

**[08 — Clients: Desktop, Web/PWA, CLI, TUI, API & SDKs](08-clients.md)**

| ID | Feature | Priorität |
|---|---|---|
| DESK-003 | Multi-Server-Profile | Must |
| WEB-013 | PWA & Mobile-Layout | Must |
| WEB-014 | Web-Push | Must |
| CLI-006 | `login` & `profile` | Must |

**[10 — Runner & Erweiterbarkeit](10-runners-extensibility.md)**

| ID | Feature | Priorität |
|---|---|---|
| RUN-004 | Remote-Host-Daemon `beton host` | Must |
| RUN-005 | Host-Labels & automatische Labels | Must |
| RUN-006 | Host als User-Service | Should |
| RUN-007 | Heartbeat, Health & Reconnect | Must |
| RUN-019 | Runner- & Host-Verwaltung (CLI/API) | Should |

**[11 — Plattform-Features](11-platform-features.md)**

| ID | Feature | Priorität |
|---|---|---|
| OBS-004 | Metriken & Prometheus `/metrics` | Must |
| OBS-009 | Health- & Readiness-Endpunkte | Must |

**[12 — Distribution & Qualität](12-distribution-quality.md)**

| ID | Feature | Priorität |
|---|---|---|
| DIST-009 | Container-Image `beton-server` | Must |
| DIST-011 | docker-compose | Must |
| QA-017 | Datenbank-Matrix-Tests | Must |

</details>

## M5

### M5 — Autonomie & Breite → v1.0

**Umfang**

- Async-Agents, Timer, Schedules, Webhook-API, Approval ohne Zuschauer
- Docker/Podman- und Kubernetes-Runner, Runner-Image, Reaper
- Out-of-Process-Plugins mit Registry und Signaturen
- Windows-Beta (Sandbox, Desktop), winget/Scoop/deb/rpm, Helm

**Demo-Szenario (Exit-Kriterium):** Ein nächtlicher Schedule startet auf einem Kubernetes-Runner einen Async-Agent, der Dependencies aktualisiert und einen PR öffnet; eine `ask`-Policy pausiert ihn bis zur Freigabe vom Handy; ein Community-Plugin wird signiert installiert; Windows-Nutzer arbeiten mit der Beta-Sandbox.

<details><summary>Features (47)</summary>


**[02 — Agents & Automation](02-agents.md)**

| ID | Feature | Priorität |
|---|---|---|
| ASY-001 | Async-Session & Run-Modell | Must |
| ASY-002 | `spawn(async: true)` & Ergebnis-Zustellung | Must |
| ASY-003 | Timer | Must |
| ASY-004 | Schedules (Cron, Timezone, catch_up) | Must |
| ASY-005 | Lokaler Scheduler & `keep_awake` | Must |
| ASY-006 | Zentraler Dispatch an Runner mit Labels | Must |
| ASY-007 | Webhook-Trigger über die API | Must |
| ASY-008 | Approval ohne Zuschauer | Must |
| ASY-009 | Inbox-Anbindung | Must |
| ASY-010 | Kosten- & Laufzeit-Caps pro Run | Must |
| ASY-011 | Schedule-/Run-Verwaltung (CLI, API, System-Tools) | Should |
| ASY-012 | Concurrency-Limits & Warteschlange | Should |

**[03 — Policies](03-policies.md)**

| ID | Feature | Priorität |
|---|---|---|
| POL-028 | WASM-Policy-Erweiterungen | Could |

**[04 — Sandbox & Egress-/Credential-Proxy](04-sandbox.md)**

| ID | Feature | Priorität |
|---|---|---|
| SBX-015 | Docker/Podman-Backend | Must |
| SBX-016 | Windows-Beta-Backend | Must |

**[08 — Clients: Desktop, Web/PWA, CLI, TUI, API & SDKs](08-clients.md)**

| ID | Feature | Priorität |
|---|---|---|
| DESK-009 | Windows-Desktop (Beta) | Should |
| CLI-011 | `schedule` | Should |
| CLI-012 | `plugin` | Should |

**[09 — Eingebetteter Browser & Inspect-Mode](09-browser.md)**

| ID | Feature | Priorität |
|---|---|---|
| BRW-021 | Browser in Remote- und Container-Runnern | Should |

**[10 — Runner & Erweiterbarkeit](10-runners-extensibility.md)**

| ID | Feature | Priorität |
|---|---|---|
| RUN-008 | Docker/Podman-Provider (lokal) | Must |
| RUN-009 | Remote-Docker-Host | Should |
| RUN-010 | Workspace-Strategien | Must |
| RUN-011 | Repo-Clone in Remote-Runnern | Must |
| RUN-012 | Kubernetes-Provider | Must |
| RUN-013 | Reaper für verwaiste Runner | Must |
| RUN-014 | Offizielles Runner-Image | Must |
| RUN-015 | CLI-Login im Container & verschlüsseltes Credential-Volume | Must |
| RUN-016 | Labels & Scheduling (Dispatch) | Must |
| RUN-017 | Ressourcen-Limits | Should |
| RUN-018 | Snapshot & Restore (optional) | Could |
| PLG-002 | Plugin-Protokoll (JSON-RPC 2.0 über stdio) | Must |
| PLG-003 | Plugin-Lifecycle & Supervision | Must |
| PLG-004 | Manifest-Format | Must |
| PLG-005 | Installation `beton plugin install` | Must |
| PLG-006 | Registry-Index als Git-Repository | Must |
| PLG-007 | Signatur- & Checksum-Prüfung | Must |
| PLG-008 | Berechtigungen & Bestätigung | Must |
| PLG-009 | Durchsetzung der Plugin-Berechtigungen | Must |
| PLG-010 | Versionierung & Kompatibilität | Must |
| PLG-011 | Plugin-Verwaltung | Must |
| PLG-012 | Rust-Plugin-SDK & Beispiel-Plugin | Must |
| PLG-013 | Plugin-Conformance-Tests | Should |
| PLG-014 | WASM-Policy-Plugins (wasmtime + WIT) | Could |

**[12 — Distribution & Qualität](12-distribution-quality.md)**

| ID | Feature | Priorität |
|---|---|---|
| DIST-006 | winget & Scoop | Must |
| DIST-007 | deb- & rpm-Pakete | Must |
| DIST-010 | Container-Image `beton-runner` | Must |
| DIST-012 | Helm-Chart | Must |

</details>

## v2

Bewusst **nicht** in v1.0 (siehe ADRs und die Abschnitte "Nicht in v1" der Kapitel):

- Smart Routing („Auto“-Harness- und Modellwahl, lernender Router) — ADR-0022
- Native Mobile-Apps (Tauri Mobile) — ADR-0015
- Slack-Bot, VS-Code-Extension — ADR-0015
- UI-Extensions (sandboxed iframe + Message-Bridge) — ADR-0018
- Öffentliche (login-freie) Share-Links, Canvas-Ansicht — ADR-0014
- SaaS-Sandbox-Provider (E2B, Daytona, Modal, Fly) und MicroVMs als Community-Provider — ADR-0017
- Vault-/KMS-Backends für den Master-Key (als Plugin) — ADR-0024
- Branding/White-Labeling — ADR-0021
- Prompt-Cleanup nach Diktat — ADR-0023
- Python-SDK, Flatpak, AUR/Nix, SCIM
