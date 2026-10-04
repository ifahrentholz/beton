# 08 — Clients: Desktop, Web/PWA, CLI, TUI, API & SDKs

Dieses Kapitel spezifiziert alle Oberflächen, über die Menschen und Programme mit beton arbeiten: die **Desktop-App** (Tauri 2, ADR-0004), die **Web-UI/PWA** (React, ADR-0020), die **CLI** (`beton …`), die **TUI** (ratatui) sowie die **REST-API** mit OpenAPI 3.1 und den SDKs für TypeScript und Rust (ADR-0015). Alle Clients sprechen ausschließlich mit dem Server (lokal: Daemon auf localhost) über REST + WebSocket (PROTO, siehe 06-data-sync-protocol.md); keiner spricht direkt mit einem Runner.

Fachliche Funktionen (Sessions, Kommentare, PR-Panel) sind in 07-sessions-collaboration.md, der eingebettete Browser in 09-browser.md (ADR-0016), Command-Palette/Themes/Shortcuts/Usage-Seite/Voice in USE/VOI/UX (siehe 11-platform-features.md) spezifiziert. Hier geht es um Shell, Layout, Interaktion, Performance und Kommandooberflächen. Meilensteine: minimale Web-UI und CLI M0; Desktop und TUI M3 (Release 0.1); PWA + Web-Push M4; Windows-Desktop Beta M5.

## Konzepte & Begriffe

| Begriff | Bedeutung |
| --- | --- |
| **Server-Profil** | Benannte Verbindung zu einem beton-Server (`local` oder URL), mit Token im OS-Keychain. Desktop und CLI kennen mehrere Profile. |
| **Lokaler Daemon** | `beton serve` + `beton host` auf localhost im Lokal-Modus (ADR-0003); ein Daemon pro User-Datenverzeichnis `~/.beton/`. |
| **Shell** | Plattformspezifische Hülle um die gemeinsame React-UI: Tauri-Fenster (Desktop) oder Browser/PWA. |
| **Workspace-Rail** | Rechte, tab-basierte Seitenleiste einer Session (Files, Changes, Terminals, Browser, Agents, PR/MR, Side-Chats, Kommentare). |
| **Composer** | Eingabebereich mit Pickern, Attachments, Mentions, Slash-Menü, Queue-Anzeige. |
| **Approval-Bar** | Fest positionierte Leiste für offene Approvals/Fragen der aktuellen Session. |
| **Skript-Modus** | Nicht-interaktiver Lauf `beton run -p` mit Antwort auf stdout. |
| **Bundled UI** | Die im Desktop mitgelieferten Frontend-Assets (`apps/web`-Build); dieselben Assets liefert `beton serve` als Web-UI aus. |

## Design

### Web-/Desktop-Layout (≥ 1280 px)

```
┌───────────────┬──────────────────────────────────────────┬──────────────────────────┐
│ Sessions      │ Titel · Harness/Modell · Kontext ▓▓▓░ 62% │ [Files|Changes|Term|Brw| │
│ [+ Neu] [⌘K]  │ Presence ●● · Share · ⋯                    │  Agents|PR|Side|💬]      │
│ ▸ Pinned      ├──────────────────────────────────────────┤                          │
│ ▸ Projekt A   │  User: …                                  │  (aktiver Rail-Tab:      │
│   · Session 1 │  Agent: … ▸ Tool-Card bash (✔ 1.2s)       │   Monaco / Diff /        │
│   · Session 2●│  Agent: … (streamt)                       │   xterm.js / Browser-    │
│ ▸ Geteilt     │                                           │   Canvas / xyflow-Graph) │
│ ▸ Archiv      │  ┌─ Approval-Bar ──────────────────────┐  │                          │
│               │  │ bash: git push origin …  [Erlauben] │  │                          │
│ Inbox (3)     │  │ Policy: project/no-push  [Ablehnen] │  │                          │
│ Usage         │  └─────────────────────────────────────┘  │                          │
│ Settings      │  Queue (2) ▾                              │                          │
│               │  ┌ Composer ───────────────────────────┐  │                          │
│               │  │ @datei /skill …           🎤 📎 ⏎   │  │                          │
│               │  │ [claude ▾][opus ▾][high ▾][default ▾]│  │                          │
│               │  └─────────────────────────────────────┘  │                          │
└───────────────┴──────────────────────────────────────────┴──────────────────────────┘
```
< 1024 px: Session-Liste als Drawer, Workspace-Rail als Bottom-Sheet/Vollbild-Tab; < 640 px (PWA mobil): Ein-Spalten-Ansicht mit Tab-Leiste unten (Chat · Workspace · Inbox).

### Desktop-Prozessmodell

```
beton.app (Tauri-Prozess, Rust)                      beton (Sidecar-Binary, gleicher Release)
 ├─ WebView (Bundled UI, React)  ──HTTP/WS──►        ├─ beton serve  (lokaler Server, localhost)
 ├─ beton-sdk (Profile, Health, Notifications)       └─ beton host   (Runner-Start, lokal)
 ├─ Keychain, Tray, Badge, Deep-Links, Global-Shortcut, Updater
 └─ Audio-Capture (Push-to-Talk) ──WS──► Server (beton-voice, VOI-004)
```
Die App startet den Daemon nur, wenn für das Profil `local` keiner läuft (Lock-Datei `~/.beton/daemon.lock` + Health-Check); Sessions laufen weiter, wenn das Fenster geschlossen wird.

### CLI-Kommandobaum

```
beton
├── run [TARGET] [-p PROMPT|-] [--harness H] [--model M] [--effort E] [--permission-mode P]
│       [--project ID] [--cwd DIR] [--worktree[=BRANCH]] [--base BRANCH] [--mode native|tui]
│       [-c|--continue] [--resume ID] [--fork ID[@SEQ]] [--title T] [--detach] [--param K=V]…
│       [--output-format text|json|stream-json] [--on-ask wait|deny] [--max-cost USD] [--timeout DUR]
│       [--scenario FILE]                                  # nur Harness `fake` (HAR-026)
├── resume <SESSION> [--mode native|tui]
├── attach <SESSION> [--read-only]
├── open     [SESSION]                                 # Web-UI per Einmal-Link öffnen (AUTH-004)
├── session  list|show|rename|archive|unarchive|delete|fork|share|unshare|interrupt|take [--force]
├── serve    [--bind ADDR] [--port N] [--config FILE] [--database-url URL] [--foreground] [--dev]
├── host     [--server URL] [--label k=v]… [--background] | pair|enable|disable|status|stop
├── hosts    list
├── runners  list [--host H]|logs <ID>|stop <ID>
├── runner   login --harness H [--host H]                 # Vendor-CLI-Login im Container (RUN-015)
├── setup    [--non-interactive] [--check] [--json] [--install-clis] | acp add <SLUG> --command C [--arg A]…
├── login    [SERVER_URL] [--profile NAME] [--device-code] | logout | whoami
├── profile  list|add|use|remove
├── doctor   [--json]
├── diagnose [-o FILE] [--anonymize] [--include-session ID]
├── import   [--harness claude|codex|all] [--session REF]… [--last N] [--force] [FILE.jsonl]
├── export   <SESSION> [-o FILE] [--with-blobs] [--with-raw]
├── config   get|set|unset|list|edit [--global|--project]
├── usage    [--by day|session|project|harness|model|user|team] [--since DATE|DUR] [--from DATE --to DATE]
│           [--session ID] [--json|--csv] | pricing explain --model M
├── agent    list|show|validate|new|schema
├── mcp      list|add|remove|test [--global|--project]  # UX-010, AGT-006
├── policy   test|explain|eval|show                    # POL-026, POL-027
├── sandbox  probe|explain|exec|test                   # SBX-010
├── proxy    ca rotate                                 # PRX-002
├── secrets  add|list|rm|test|rotate|unlock            # SEC-003, SEC-004
├── audit    list|export|verify                        # SEC-012
├── auth     rotate-local                              # AUTH-001
├── admin    secrets rewrap|projections rebuild|blobs migrate   # SEC-006, DATA-005, DATA-007
├── voice    models list|pull|import|remove            # VOI-002
├── telemetry status|enable|disable|show|reset-id      # OBS-007
├── schedule list|create|update|pause|resume|delete|run-now
├── run-history <SCHEDULE|AGENT>
├── plugin   install|list|update|remove|enable|disable|info|search|new|validate|test|doctor
├── upgrade  [--check] [--channel stable|beta|nightly] [--version V] [--from-file FILE] [--dry-run] [--force]
├── uninstall [--purge] [--yes]
├── tui      [SESSION]
├── completion <bash|zsh|fish|powershell>
└── version
(versteckt/intern: `mcp serve|proxy` (HAR-009), `hook` (HAR-005, HAR-013), `dev record-golden` (HAR-025), `__runner` (Runner-Prozess, RUN-002), `__exec`/`__sandbox-exec` (SBX-002, SBX-008))
Globale Flags: --server PROFILE|URL · --json · -q/--quiet · -v/--verbose · --no-color · --config FILE
```

### TUI-Layout

```
┌ Sessions ─────┬ Session: refactor-auth (claude · opus) ──────────────┬ Rail ───────────┐
│> refactor-auth│ Agent: Ich passe middleware.rs an …                   │ Changes (3)     │
│  fix-ci     ● │ ▸ edit src/middleware.rs  (+12 −4)                    │  M middleware.rs│
│  docs         │ ┌ Approval ─────────────────────────────────────────┐ │  M lib.rs       │
│               │ │ bash: cargo test -p beton-auth                    │ │ Terminals       │
│               │ │ [y] erlauben  [n] ablehnen  [d] Details           │ │  1 dev-server   │
│               │ └───────────────────────────────────────────────────┘ │                 │
├───────────────┴───────────────────────────────────────────────────────┴─────────────────┤
│ > _                                         Queue: 1 · Ctx 41% · $0.42 · Ctrl+G ? Hilfe │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

## Features

### DESK-001 — Tauri-2-App-Shell & Fenster
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Native Desktop-App für macOS und Linux auf Tauri 2, die die Bundled UI im System-WebView (WKWebView, WebKitGTK) lädt. Unterstützt mehrere Fenster (`Cmd/Ctrl+N`), natives Menü (inkl. Standard-Edit-Menü für Copy/Paste), Fensterzustand-Persistenz, „Im Finder/Dateimanager zeigen“ und Datei-Drop als Attachment.
- **Details:** Strikte CSP; Tauri-Capabilities nur für benötigte Plugins (`notification`, `deep-link`, `global-shortcut`, `updater`, `shell` nur für Sidecar). Fenster ↔ Profil ↔ Session als URL-Route.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `.dmg` (macOS) und `.AppImage`/`.deb` (Linux) starten und zeigen die Session-Liste des Profils `local` ohne weitere Konfiguration.
  - [ ] AC2 — Zwei Fenster können verschiedene Sessions gleichzeitig live anzeigen.
  - [ ] AC3 — Eine auf das Fenster gezogene Datei landet als Attachment im Composer der aktiven Session.
  - [ ] AC4 — Fenstergröße/-position werden nach Neustart wiederhergestellt.
- **Abhängigkeiten:** WEB-001, DIST-008 (siehe 12-distribution-quality.md)
- **Referenz:** ADR-0004; Omnigent Desktop (Electron)

### DESK-002 — Lokaler Daemon-Start & -Management
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Die App bringt das `beton`-Binary als Sidecar mit und startet bei Bedarf den lokalen Daemon. Läuft bereits ein kompatibler Daemon (z.B. aus der CLI gestartet), verbindet sie sich damit. Das Tray-/App-Menü zeigt Daemon-Status und bietet Neustart, Stop und Log-Ansicht.
- **Details:** Kompatibilität via Protokoll-Versionsaushandlung (PROTO-004, siehe 06-data-sync-protocol.md). Beim Beenden mit laufenden Sessions fragt die App: „Daemon weiterlaufen lassen“ (Default) oder „stoppen“.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ohne laufenden Daemon startet die App ihn innerhalb von 3 s und zeigt „verbunden“.
  - [ ] AC2 — Läuft ein Daemon aus `beton serve`, startet die App keinen zweiten (nur ein Prozess hält `daemon.lock`).
  - [ ] AC3 — Ist der laufende Daemon inkompatibel (mehr als eine Minor-Version Abstand), zeigt die App einen Dialog mit Upgrade-Option statt eines generischen Fehlers.
  - [ ] AC4 — Schließen des letzten Fensters beendet laufende Sessions nicht.
- **Abhängigkeiten:** CLI-004, DESK-001

### DESK-003 — Multi-Server-Profile
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Neben `local` können (ab M4, weil Remote-Server OIDC bzw. Device-Pairing voraussetzen) beliebige Server-Profile (z.B. Team-Server, Tailscale-Laptop) angelegt werden; Login per OIDC im System-Browser bzw. Device-Pairing, Token im OS-Keychain. Jedes Fenster ist an ein Profil gebunden; Badges und Notifications werden über alle Profile aggregiert.
- **Details:** Profile in `~/.beton/profiles.yaml` (ohne Tokens), gemeinsam genutzt mit der CLI (`beton profile`). Bei abgelaufenem Token: Re-Login-Banner im betroffenen Fenster.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein in der CLI angelegtes Profil erscheint in der App und umgekehrt.
  - [ ] AC2 — Tokens liegen nicht in `profiles.yaml` (Test: Datei-Scan), sondern im Keychain.
  - [ ] AC3 — Unread-Badge zählt ungelesene Sessions aller verbundenen Profile.
- **Abhängigkeiten:** AUTH-005, AUTH-008, AUTH-009 (siehe 05-security-identity.md), CLI-006

### DESK-004 — Tray & Dock-/Taskbar-Badge
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** Tray-Icon (macOS Menüleiste, Linux AppIndicator) mit laufenden Sessions, offenen Approvals und Schnellaktionen (Neue Session, Fenster öffnen, Daemon-Status). Dock-/Taskbar-Badge zeigt die Anzahl offener Approvals + ungelesener Sessions.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein neues Approval erhöht den Badge innerhalb von 2 s; Entscheiden verringert ihn.
  - [ ] AC2 — Klick auf eine Session im Tray öffnet bzw. fokussiert das Fenster mit dieser Session.
- **Abhängigkeiten:** DESK-001, UX-001 (siehe 11-platform-features.md), SES-012 (siehe 07-sessions-collaboration.md)

### DESK-005 — Native Notifications nur für nicht betrachtete Sessions
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** OS-Notifications für Turn-Ende, Approval/Frage, Runner-Disconnect, fertigen Async-Agent und (ab M4) Erwähnungen. Keine Notification, wenn die betroffene Session in einem fokussierten Fenster sichtbar ist. Klick öffnet die Session am relevanten Event.
- **Details:** Inhalt: Session-Titel + erste Zeile (abschaltbar, `notifications.preview: false`). Ab M4 wird die serverseitige Routing-Entscheidung (COL-010, siehe 07-sessions-collaboration.md) respektiert; lokal entscheidet die App anhand des Fensterfokus.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Session im fokussierten Fenster sichtbar → bei `turn.completed` keine Notification; Fenster unfokussiert → genau eine.
  - [ ] AC2 — Klick auf eine Approval-Notification öffnet die Session mit sichtbarer Approval-Bar.
  - [ ] AC3 — Mit `notifications.preview=false` enthält die Notification keinen Nachrichtentext.
- **Abhängigkeiten:** DESK-001, UX-006 (siehe 11-platform-features.md), COL-010 (ab M4, siehe 07-sessions-collaboration.md)

### DESK-006 — Deep-Links `beton://`
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Die App registriert das Schema `beton://` und leitet Links an eine laufende Instanz weiter (Single-Instance). Unterstützt Session-Links, Pairing und Kommentar-Anker.
- **Details:** `beton://<profil-oder-host>/s/<session_id>[?seq=N|&comment=<id>]`, `beton://pair?server=<url>&code=<code>`. Unbekannter Server → Bestätigungsdialog „Profil anlegen?“; niemals automatische Verbindung.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `open beton://local/s/<id>` fokussiert die laufende App und öffnet die Session; es startet keine zweite Instanz.
  - [ ] AC2 — Ein Link auf einen unbekannten Host führt zu einem Bestätigungsdialog ohne Netzwerkzugriff vor Zustimmung.
  - [ ] AC3 — `?seq=N` scrollt zum Event N.
- **Abhängigkeiten:** DESK-001, DESK-003 (ab M4; bis dahin nur Profil `local`)

### DESK-007 — Auto-Update
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Tauri-Updater mit signierten Manifesten, Kanäle `stable`/`beta`/`nightly`. Die Update-Prüfung kontaktiert den Update-Server **nur auf Klick** („Nach Updates suchen“) oder, wenn der User die automatische Prüfung eingeschaltet hat (`update.auto_check`, Default aus; dann beim Start und alle 6 h); Installation nur nach Zustimmung. Offline-Alternative: „Update aus Datei installieren“ mit einem lokal vorliegenden, signierten Update-Paket (ADR-0033). Das Sidecar-Binary wird mit aktualisiert; der Daemon wird erst neu gestartet, wenn keine Session läuft oder der User zustimmt. Implementierung (Manifeste, Kanäle, Signaturprüfung): Owner DIST-014; DESK-007 beschreibt nur das App-Verhalten.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Manifest mit ungültiger Signatur wird verworfen und protokolliert.
  - [ ] AC2 — Bei laufendem Turn wartet der Daemon-Neustart, bis die Session `idle` ist oder der User „jetzt“ wählt.
  - [ ] AC3 — Kanalwechsel in den Einstellungen wirkt bei der nächsten Prüfung.
  - [ ] AC4 — Ohne Klick und ohne eingeschaltete automatische Prüfung baut die App keine Verbindung zum Update-Server auf (Netz-Mock-Test über 48 h simulierte Laufzeit).
  - [ ] AC5 — „Update aus Datei installieren“ installiert ein lokales, signiertes Update-Paket ohne Netzwerk; ein Paket mit ungültiger Signatur wird abgelehnt.
- **Abhängigkeiten:** DIST-014, DIST-017 (siehe 12-distribution-quality.md)

### DESK-008 — Globaler Push-to-Talk-Shortcut
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** Systemweiter Shortcut (Default `CmdOrCtrl+Shift+Space` *(Annahme)*, konfigurierbar; Verhalten, Modi `hold`/`toggle` und Default: Owner VOI-007) startet bei Druck die Aufnahme und stoppt bei Loslassen; Audio geht per WS an den Server (lokales Whisper, VOI-004, siehe 11-platform-features.md). Das Transkript landet im Composer der zuletzt aktiven Session; ist die App nicht im Vordergrund, zeigt ein kleines HUD Aufnahme- und Transkriptstatus.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Halten des Shortcuts bei unfokussierter App zeigt das HUD; Loslassen fügt das Transkript in den Composer ein, ohne es zu senden (außer bei `voice.auto_send: true`, VOI-007).
  - [ ] AC2 — Fehlende Mikrofonberechtigung führt zu einem Hinweis mit Link zu den Systemeinstellungen.
  - [ ] AC3 — Es wird kein Audio an Dritte gesendet (Netzwerk-Test: nur Verbindung zum konfigurierten beton-Server).
- **Abhängigkeiten:** VOI-004, VOI-007 (siehe 11-platform-features.md)

### DESK-009 — Windows-Desktop (Beta)
- **Meilenstein:** M5 · **Priorität:** Should
- **Beschreibung:** Desktop-Build für Windows x64/arm64 mit WebView2, Installer `.msi`/NSIS, Deep-Links, Notifications, Tray. Als „Beta“ gekennzeichnet, solange die Windows-Sandbox Beta ist; Features ohne Windows-Unterstützung sind ausgegraut mit Erklärung.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Installer installiert ohne Admin-Rechte (per-user) und registriert `beton://`.
  - [ ] AC2 — Eine Claude-Session lässt sich starten, streamen und mit Approval steuern (E2E mit Fake-Harness auf Windows-CI).
  - [ ] AC3 — Die UI zeigt einen Beta-Hinweis mit dem aktuellen Sandbox-Status aus `beton doctor`.
- **Abhängigkeiten:** SBX-016 (siehe 04-sandbox.md), DIST-006, DIST-008 (siehe 12-distribution-quality.md)

### WEB-001 — App-Shell, Routing & Layout
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** React-19-SPA (Vite, TanStack Router/Query, Zustand, Tailwind, shadcn/ui), ausgeliefert von `beton serve` und gebündelt im Desktop. Drei-Spalten-Layout (Session-Liste · Chat · Workspace-Rail) mit responsiven Breakpoints gemäß Design; M0 liefert Liste + Chat + Composer.
- **Details:** Server-State ausschließlich über TanStack Query + WS-Event-Store; UI-State in Zustand. Routen: `/s/:sessionId`, `/projects/:id`, `/inbox`, `/usage`, `/settings/*`. Alle Assets (Schriften, Icons, Monaco- und Shiki-Dateien, Mermaid, Web-Worker) sind gebündelt und werden vom eigenen Origin ausgeliefert; kein CDN, keine externen Origins (ADR-0033). „Offline“ (kein Netz außer Loopback) ist ein normaler Zustand ohne Fehlerdialog. `beton serve` liefert den Build aus `BETON_WEB_DIR` bzw. neben dem Binary (`share/beton/web`, `web/`); Client-Routen bekommen `index.html`, Assets sind `immutable`, HTML trägt eine CSP nur für den eigenen Origin (ohne `unsafe-eval`/`wasm-unsafe-eval`; Shiki nutzt deshalb die JavaScript-Regex-Engine). Ohne Anmeldung bekommt eine Browser-Navigation (401) eine Hinweisseite auf `beton open` statt JSON. M0: Navigationsspalte, Session-Liste (unter 1024 px als Drawer) und Session-Ansicht; die Workspace-Rail folgt mit WEB-008.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton run claude` öffnet (oder druckt) eine URL, unter der die Session live im Browser sichtbar ist.
  - [ ] AC2 — Deep-Link `/s/<id>` lädt direkt die Session (Reload-fest).
  - [ ] AC3 — Bei 375 px Breite gibt es keinen horizontalen Scroll; Rail und Liste sind über Navigation erreichbar.
  - [ ] AC4 — Playwright-Test: Beim Laden und Bedienen der UI gehen alle Requests an den eigenen Origin; ein Request an einen fremden Origin (z. B. Schriften-CDN) lässt den Test fehlschlagen. Ein Build-Check findet keine absoluten `http(s)://`-Asset-URLs im Bundle.
- **Abhängigkeiten:** PROTO-004, PROTO-005 (siehe 06-data-sync-protocol.md), API-001, API-004
- **Referenz:** ADR-0020

### WEB-002 — Chat-Stream-Rendering
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Darstellung des Event-Stroms: Markdown (Shiki-Highlighting, Mermaid lazy), einklappbares Reasoning, Tool-Cards mit Status/Dauer/Ergebnis, Diff-Vorschau bei Edits, Fehler- und Policy-Entscheidungs-Cards, Kosten/Kontext im Header. Sub-Agent-Turns erscheinen als verlinkte Karten.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein unvollständiger Markdown-Codeblock während des Streamings rendert ohne Layout-Sprünge > 1 Zeile.
  - [ ] AC2 — Jede Tool-Card zeigt Name, Argument-Kurzform, Status (`requested|running|completed|failed|denied`) und ist ausklappbar.
  - [ ] AC3 — `policy.decision` mit `deny` erscheint als Card mit Regelname und Begründung.
- **Abhängigkeiten:** WEB-001, WEB-015

### WEB-003 — Session-Liste
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Linke Spalte mit Sessions gruppiert nach Pinned, Projekten, „Mit mir geteilt“ (M4) und Archiv; Status-Indikatoren (läuft, wartet auf Approval, Fehler, ungelesen), Suche/Filter, Kontextmenü (umbenennen, archivieren, löschen, forken, in Projekt verschieben).
- **Details:** M0: Liste nach jüngster Aktivität, Suche, Archiv; Live-Aktualisierung über `GET /v1/sessions?updated_after=<ts>` alle 500 ms (Deltas statt Neuladen). Gruppen (Pinned, Projekte, geteilt) und Kontextmenü folgen mit ihren Features. M1: Gruppe „Angepinnt“ oben, ungelesene Sessions fett, Suche über Titel und Nachrichten per `q` (SES-012).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Statuswechsel einer Session (z.B. `waiting_approval`) aktualisiert den Indikator ohne Reload innerhalb von 1 s.
  - [ ] AC2 — Die Liste bleibt bei 5 000 Sessions flüssig scrollbar (virtualisiert, ≥ 55 fps).
- **Abhängigkeiten:** SES-001, SES-012 (ab M1, siehe 07-sessions-collaboration.md)

### WEB-004 — Composer mit Pickern
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Mehrzeiliger Composer mit Pickern für Harness/Agent, Modell, Effort und Permission-Mode (Werte aus den Harness-Capabilities). In M0 nur Claude Code; Harness-Wechsel ab M1, ausschließlich als Fork („Weiter mit <Harness>“, SES-007 in 07-sessions-collaboration.md). Senden mit `⏎`, Zeilenumbruch `⇧⏎`; während eines Turns zeigt der Senden-Button „Einreihen“ bzw. „Steer“.
- **Details:** M0: Modell-Picker aus `capabilities.models` (bei neuer Session wirksam), Effort-Picker nur bei `effort_switch` ≠ `none` und vorhandenen `capabilities.efforts`. Eingaben während eines Turns reiht der Client ein und sendet sie nach dem Turn-Ende. Permission-Mode-Picker und „Steer“ folgen, sobald Server und Runner sie durchreichen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Picker zeigen nur Optionen, die der aktuelle Harness unterstützt (Capability-gesteuert, Test mit Fake-Harness-Capabilities).
  - [ ] AC2 — (ab M1) Modellwechsel mid-session erzeugt ein Event und wirkt ab dem nächsten Turn.
  - [ ] AC3 — Der Composer-Entwurf pro Session überlebt Reload (localStorage) *(Annahme)*.
- **Abhängigkeiten:** HAR-002, HAR-017 (ab M1, siehe 01-harnesses.md), WEB-001

### WEB-005 — Queue- & Steer-UI
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Über dem Composer zeigt eine einklappbare Liste die serverseitige Queue mit Autor, Bearbeiten, Löschen, Drag-&-Drop-Reihenfolge und „Als Steer senden“. Interrupt-Button (`Esc` doppelt, wie in den Shortcut-Defaults von UX-004) im Composer.
- **Details:** Prototyp-Screen `session-stream` (Zeilen „Eingereiht n“ mit „Jetzt lenken“, „Bearbeiten“, „Entfernen“); den Autor zeigt die Zeile, sobald mehrere Personen eingereiht haben (`collab-codrive`). `Esc` zweimal binnen 600 ms unterbricht den laufenden Turn. Einklappen der Liste und eine Fortsetzen-Aktion für die pausierte Queue folgen, sobald der Prototyp sie zeigt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Drag-&-Drop-Reorder wird an alle Clients propagiert.
  - [ ] AC2 — „Als Steer senden“ ist deaktiviert (mit Tooltip), wenn der Harness kein Steering unterstützt.
- **Abhängigkeiten:** SES-004, SES-005 (siehe 07-sessions-collaboration.md)

### WEB-006 — Attachments, @-Mentions & Slash-Menü
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Attachments per Button, Drop oder Paste (Bilder, PDF, Text; Limits serverseitig konfigurierbar, Default 20 MiB/Datei, 10 Dateien). `@` öffnet Fuzzy-Suche über Workspace-Dateien (fügt Referenz ein), `/` ein Menü aus Slash-Befehlen (beton-eigene wie `/side`, `/compact`, `/fork` sowie Skills und Harness-Befehle).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein eingefügter Screenshot erscheint als Vorschau und wird als Blob mit dem Input gesendet.
  - [ ] AC2 — `@mid` listet `src/middleware.rs` innerhalb von 150 ms (Repo mit 20 000 Dateien).
  - [ ] AC3 — Das Slash-Menü zeigt Skills des aktiven Agents mit Beschreibung.
- **Abhängigkeiten:** SES-017 (siehe 07-sessions-collaboration.md), AGT-008 (siehe 02-agents.md), DATA-006 (siehe 06-data-sync-protocol.md)

### WEB-007 — Approval-Bar
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Löst die minimale Approval-Karte aus M0 (WEB-018) ab. Offene Approvals und Fragen der Session erscheinen in einer fixierten Leiste über dem Composer: Tool, Argumente (ausklappbar, Diff bei Datei-Edits), auslösende Policy und Begründung, Aktionen „Erlauben“ (`⌘⏎`), „Ablehnen“ (`⌘⌫` *(Annahme)*), ggf. „Für Session erlauben“, falls die Policy es zulässt. Mehrere Approvals werden gestapelt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `⌘⏎` erlaubt das oberste Approval; die Bar zeigt danach das nächste.
  - [ ] AC2 — Entscheidet ein anderer Client, verschwindet das Approval mit Vermerk „entschieden von …“.
  - [ ] AC3 — Die Bar ist per Tastatur erreichbar und wird von Screenreadern angekündigt (`role="alertdialog"`, axe-Test).
- **Abhängigkeiten:** POL-010 (siehe 03-policies.md), WEB-018

### WEB-008 — Workspace-Rail mit Tabs
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Rechte Seitenleiste mit Tabs: Files (Baum + Suche), Changes (Sichten uncommitted/branch/turn), Terminals (M3), Browser (M3, BRW-009 in 09-browser.md), Agents (Sub-Agent-Graph), PR/MR (M4), Side-Chats (M4), Kommentare (M4). Breite verstellbar, Tabs per Shortcut wechselbar; mehrere Datei-Tabs möglich.
- **Details:** Tabs wechseln mit `Alt+1` … in der Reihenfolge der sichtbaren Tabs; die Breite wird am linken Rand gezogen (Doppelklick setzt den Standard je Tab zurück) und bleibt im Browser gespeichert. Der Knopf „Workspace“ im Session-Kopf blendet die Rail ein und aus; unter 1024 px öffnet er sie als Vollbild. Das Change-Badge zählt die Dateien aus `fs.changed` seit dem letzten Blick auf den Tab; beim ersten Öffnen gilt der Gelesen-Stand der Session (SES-012) als gesehen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `fs.changed` setzt ein Change-Badge am Files-/Changes-Tab.
  - [ ] AC2 — Nicht verfügbare Tabs (fehlende Rolle, Feature-Meilenstein, Harness-Capability) sind ausgeblendet, nicht kaputt.
- **Abhängigkeiten:** SES-017, SES-018 (siehe 07-sessions-collaboration.md)

### WEB-009 — Code-Editor (Monaco)
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Dateien öffnen sich in Monaco (lazy geladen aus dem eigenen Bundle, kein CDN-Loader, WEB-001) mit Highlighting, Suche, Markdown-Vorschau und Speichern (`⌘S`) mit Konfliktprüfung (`If-Match`). Zeilenbereich markieren → „An Agent anhängen“ fügt eine Referenz mit Ausschnitt in den Composer ein.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ändert der Agent eine geöffnete, ungespeicherte Datei, zeigt der Editor einen Konfliktdialog statt stillem Überschreiben.
  - [ ] AC2 — Monaco ist nicht im initialen Bundle (Bundle-Analyse).
- **Abhängigkeiten:** SES-017 (siehe 07-sessions-collaboration.md)

### WEB-010 — Terminals (xterm.js)
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Terminal-Tabs auf Basis xterm.js (WebGL-Renderer, Fit-, Weblinks-Addon), angebunden an den Binärkanal des Runner-PTY. Mehrere Terminals pro Session, Umbenennen, Schließen; Native-TUI-Sessions (Vendor-TUI im PTY) werden ebenfalls hier gerendert.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Tastendruck-zu-Echo-Latenz lokal ≤ 30 ms (p95).
  - [ ] AC2 — Fenster-Resize sendet die neue Größe an das PTY; `vim` rendert korrekt.
  - [ ] AC3 — Nur-Lese-Rollen sehen den Output, Eingaben sind deaktiviert.
- **Abhängigkeiten:** SES-019 (siehe 07-sessions-collaboration.md), TUI-002, PROTO-007 (siehe 06-data-sync-protocol.md)

### WEB-011 — Diff-Ansicht
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Unified- und Split-Diff mit Shiki-Highlighting, Wrap-Umschalter, Datei-Navigation und Zeilenankern (Grundlage für Inline-Kommentare in M4 und „An Agent anhängen“). Große Diffs werden pro Datei lazy geladen.
- **Details:** Teilbarer Link auf eine Zeile: `/s/<session>?tab=changes&scope=<sicht>[&turn=<turn>]&file=<pfad>&line=N16` (`N` neue, `A` alte Seite); er öffnet die Rail mit Sicht, Datei und markierter Zeile. Ohne Git-Repository (`GET …/workspace` → `git_repo: false`, SES-018) zeigt die Ansicht gleich die Änderungen pro Turn.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Diff mit 5 000 geänderten Zeilen bleibt flüssig scrollbar (virtualisiert).
  - [ ] AC2 — Klick auf eine Zeilennummer erzeugt einen teilbaren Link auf diese Zeile.
- **Abhängigkeiten:** SES-018 (siehe 07-sessions-collaboration.md)

### WEB-012 — Sub-Agent-Graph (xyflow)
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Graph der Session-Hierarchie (Parent → Sub-Agents, inkl. Harness, Status, Kosten) mit xyflow; Klick öffnet die Sub-Session. Aktualisiert sich live.
- **Details:** Tab „Agents“ der Workspace-Rail (WEB-008), sichtbar bei Harnesses mit MCP-Injektion (System-Tools) oder nativen Sub-Agents. Daten aus `GET /v1/sessions/{id}/subagents` (AGT-009): je Session Agent, Harness, Modell, Status, Auftrag, eigene und kumulierte Kosten bzw. Tokens, Worktree. Neu geladen wird bei `agent.spawned`, `agent.completed` und `agent.message` im Stream der Session und alle 1 s, solange der Tab offen ist. Bei Subscription zeigt der Knoten Tokens und „Subscription“ statt eines Betrags (HAR-021). xyflow (`@xyflow/react`, MIT) kommt aus dem eigenen Bundle und wird erst mit dem Tab geladen (WEB-015, CSP `'self'`). Die Ansicht „Partitur“ des Screens folgt mit #139.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein neu gespawnter Sub-Agent erscheint ≤ 1 s nach seinem Start-Event als Knoten.
  - [ ] AC2 — Knoten zeigen Harness-Icon, Status und kumulierte Kosten.
- **Abhängigkeiten:** AGT-009 (siehe 02-agents.md)

### WEB-013 — PWA & Mobile-Layout
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Die Web-UI ist als PWA installierbar (Manifest, Icons, Service Worker für App-Shell-Caching, kein Offline-Arbeiten). Das Mobile-Layout priorisiert Lesen, Approvals und kurze Inputs (inkl. Diktat über VOI). Zugriff auf einen lokalen Server von unterwegs erfordert Freigabe via Tailscale/LAN + Device-Pairing und HTTPS.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Lighthouse-PWA-Installierbarkeit besteht; auf iOS ≥ 16.4 und Android lässt sich die App zum Homescreen hinzufügen.
  - [ ] AC2 — Ein Approval lässt sich auf 375 px Breite mit maximal zwei Taps entscheiden (vom Öffnen der Notification an).
  - [ ] AC3 — Nach Deployment einer neuen Version zeigt die PWA „Update verfügbar – neu laden“, statt veraltete Assets dauerhaft zu nutzen.
- **Abhängigkeiten:** WEB-001, AUTH-009 (siehe 05-security-identity.md)

### WEB-014 — Web-Push
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Push-Notifications über den Web-Push-Standard (VAPID, RFC 8291) für Approvals, Fragen, Turn-Ende, Erwähnungen, gemäß serverseitigem Routing (COL-010, siehe 07-sessions-collaboration.md). Weil Web-Push über die Push-Dienste der Browser-Hersteller läuft, ist er **optional und standardmäßig aus**: Erst wenn der User ihn in den Einstellungen bzw. in der PWA ausdrücklich aktiviert, wird eine Subscription angelegt; Standard sind Inbox/In-App und lokale Desktop-Notifications (DESK-005, ADR-0033). Payload minimal; Klick öffnet die PWA direkt an der Approval-Card. Approvals werden in der App entschieden, nicht per Notification-Action *(Annahme: verhindert Entscheidungen vom Sperrbildschirm)*.
- **Details:** `POST /v1/push/subscriptions {endpoint, keys}`; VAPID-Schlüssel serverseitig generiert und gespeichert. Payload: `{type, session_id, title, preview?}`, `preview` nur bei `notifications.push_preview=true` (Default aus).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit installierter PWA, aktiviertem Web-Push und geschlossener App erzeugt ein `approval.requested` eine Push-Notification innerhalb von 5 s.
  - [ ] AC2 — Abgelaufene Subscriptions (HTTP 404/410 vom Push-Dienst) werden automatisch entfernt.
  - [ ] AC3 — Ohne `push_preview` enthält die Payload keinen Nachrichtentext.
  - [ ] AC4 — Ohne ausdrückliche Aktivierung existiert keine Push-Subscription und der Server kontaktiert keinen Push-Dienst (Netz-Mock-Test); Approvals bleiben über Inbox und PWA entscheidbar.
- **Abhängigkeiten:** WEB-013, COL-010 (siehe 07-sessions-collaboration.md)

### WEB-015 — Performance-Budgets
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Verbindliche Budgets, in CI gemessen (Playwright + Fake-Harness): Streaming ohne Ruckeln, schnelle Session-Wechsel, begrenzter Speicher. Umsetzung über Event-Batching pro Animation-Frame, inkrementelles Markdown-Rendering und Virtualisierung (`@tanstack/react-virtual`).
- **Details:** In der PR-CI läuft das Streaming-Budget 20 s statt 10 min (`BETON_PERF_STREAM_SECONDS`, gleiche Rate); Messwerte stehen als Annotation im Playwright-Bericht. Markdown wird inkrementell gerendert: fertige Absätze/Codeblöcke bleiben gemerkt, nur der letzte Block wird neu geparst.

  | Metrik | Budget |
  | --- | --- |
  | Streaming 500 Tokens/s, 10 min | ≥ 58 fps median, < 5 % Frames > 32 ms |
  | Session-Wechsel (gecacht / kalt, 10 000 Events) | ≤ 150 ms / ≤ 800 ms bis erster Inhalt |
  | Initial-JS (gzip, ohne Monaco/xterm/xyflow) | ≤ 450 KB |
  | Heap bei Session mit 10 000 Events | ≤ 300 MB |
  | Time-to-Interactive lokal (Desktop) | ≤ 1,5 s |
- **Akzeptanzkriterien:**
  - [ ] AC1 — CI-Performance-Job misst alle Budgets und schlägt bei Überschreitung fehl.
  - [ ] AC2 — Deltas werden pro Frame gebündelt angewendet (Unit-Test des Event-Stores: 100 Deltas in einem Frame → ein Render).
- **Abhängigkeiten:** QA-007 (siehe 12-distribution-quality.md)

### WEB-016 — Barrierefreiheit (Basics)
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** Ziel WCAG 2.2 AA für Kernflüsse (Session öffnen, Nachricht senden, Approval entscheiden, Diff lesen): vollständige Tastaturbedienung, sichtbarer Fokus, Kontraste in allen Themes, `prefers-reduced-motion`, ARIA-Live-Region für Streaming (gedrosselt, nur abgeschlossene Absätze).
- **Akzeptanzkriterien:**
  - [ ] AC1 — axe-core meldet in den Kernflüssen keine `serious`/`critical`-Verstöße (Playwright-Test).
  - [ ] AC2 — Alle Kernflüsse sind ohne Maus durchführbar (E2E nur mit Tastatur).
- **Abhängigkeiten:** WEB-001

### WEB-017 — Internationalisierung DE/EN
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** UI-Texte über `i18next` *(Annahme)* mit Sprachen Englisch (Fallback) und Deutsch; Auswahl automatisch aus Browser/OS-Sprache, überschreibbar in den Einstellungen. Datums-/Zahlen-/Währungsformate über `Intl`. CLI- und TUI-Ausgaben sind in v1 nur Englisch *(Annahme)*.
- **Akzeptanzkriterien:**
  - [ ] AC1 — CI-Check schlägt fehl, wenn ein Key in `de` oder `en` fehlt.
  - [ ] AC2 — Umschalten der Sprache wirkt ohne Reload.
- **Abhängigkeiten:** WEB-001

### WEB-018 — Minimale Approval-Karte (M0)
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Damit Rückfragen des Claude-Code-Adapters (HAR-005, siehe 01-harnesses.md) schon vor der Policy-Engine beantwortbar sind, zeigt der Chat-Stream ein offenes `approval.requested` als einfache Karte: Tool-Name, Argumente als JSON (ausklappbar), Buttons „Erlauben“ und „Ablehnen“ (optional mit Kommentar an das Modell). In der CLI fragt `beton run` interaktiv mit `[y/N]`, ohne TTY gilt `--on-ask` (CLI-002). Ab M2 ersetzt die Approval-Bar (WEB-007) die Karte und ergänzt Policy-Begründung, Diff-Vorschau, Stapelung und „Für Session erlauben“. Die Karte bleibt als Inline-Darstellung im Verlauf erhalten.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given eine Claude-Session (Fake-Harness), die ein `can_use_tool` auslöst, Then erscheint im Web die Karte; „Erlauben“ setzt den Tool-Call fort, „Ablehnen“ liefert dem Modell `deny` mit dem eingegebenen Kommentar.
  - [ ] AC2 — Sind zwei Clients verbunden und entscheidet einer, verschwindet die Karte beim anderen mit Vermerk „entschieden“ (`approval.resolved`).
  - [ ] AC3 — `beton run` auf einem TTY fragt `[y/N]` (Enter = Nein). Ohne TTY gilt `--on-ask`: Default `wait` (Entscheidung über die Web-Karte, API-006), `deny` lehnt sofort ab. Ein Tool-Call wird nie ohne ausdrückliche Zustimmung ausgeführt.
- **Abhängigkeiten:** WEB-002, HAR-005 (siehe 01-harnesses.md), PROTO-002 (siehe 06-data-sync-protocol.md)

### CLI-001 — CLI-Grundgerüst & Ausgabekonventionen
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Binary `beton` (Crate `beton-cli`, `clap`) mit Kommandobaum gemäß Design. Konventionen: stdout nur Nutzdaten (bei `--json` maschinenlesbar), stderr für Fortschritt/Diagnose; Farben nur auf TTY; stabile Exit-Codes.
- **Details:** Exit-Codes *(Annahme)*: `0` ok · `1` allgemeiner Fehler · `2` Usage-Fehler · `3` Policy-Deny · `4` Approval abgelehnt/Timeout · `5` Budget erschöpft · `6` Harness-Fehler · `7` Server nicht erreichbar · `130` unterbrochen. Kommandospezifische Codes (z. B. `doctor`, OBS-005; `agent validate`/`policy test`: 0/1) sind beim jeweiligen Feature dokumentiert.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton session list --json | jq` funktioniert; stdout enthält dabei keine Spinner/Banner.
  - [ ] AC2 — Ein unbekanntes Flag liefert Exit-Code 2 mit Hilfetext auf stderr.
  - [ ] AC3 — `--help` jedes Kommandos ist durch Snapshot-Tests abgedeckt.
- **Abhängigkeiten:** API-005

### CLI-002 — `beton run`
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Startet eine Session mit Harness oder Agent (`TARGET` = Harness-ID, Agent-Referenz wie `builtin:<name>` oder Pfad zu Agent-Verzeichnis) und attacht interaktiv im Terminal (Zeilenmodus mit Streaming) oder mit `--mode tui` die Vendor-TUI im PTY (HAR-012, ab M3, siehe 01-harnesses.md). Startet bei Bedarf den lokalen Daemon. Druckt die Web-URL der Session auf stderr. Skript-Modus (`-p`) siehe API-006.
- **Details:** Ohne `TARGET` gilt `harnesses.default`, sonst `claude`. Läuft für `BETON_HOME` kein Daemon, startet `run` ihn abgelöst (`beton serve` im Hintergrund, Ausgaben in `~/.beton/logs/serve.out`); `serve --foreground` bleibt im Vordergrund. `-c` wählt unter den nicht archivierten Sessions die mit der jüngsten Aktivität, deren `session.created.cwd` dem aktuellen Verzeichnis entspricht; gestoppte Sessions werden dabei fortgesetzt (SES-003). Interaktiv: Zeilen von stdin sind Eingaben (während eines Turns gepuffert), Freigaben fragt das Terminal mit `[y/N]` (Enter = Nein); ohne Terminal gilt `--on-ask`. Endet stdin, wartet `run` den laufenden Turn ab und koppelt ab. `--detach` legt die Session an und gibt nur ihre ID aus. Ist `TARGET` kein Harness (`claude`, `codex`, `fake`, `acp:<slug>`, `direct:<provider>`; Harness-IDs haben Vorrang), ist es ein Agent-Ref: `run` startet den Agent mit Harness und Modell aus `executor`, `--harness`/`--model` überschreiben sie, `--param k=v` (mehrfach) setzt Parameter; ungültige oder fehlende Parameter, ein unbekannter oder ungültiger Agent und ein unpassender Harness brechen vor dem Start mit Meldung und Exit-Code 2 ab, eine Verschattung (AGT-003) meldet `run` auf stderr (AGT-004, AGT-010). In M0 umgesetzt sind `TARGET`, `-p`, `--model`, `--cwd`, `-c`, `--resume`, `--title`, `--detach`, `--output-format`, `--on-ask`, `--timeout`, `--scenario`; mit M1 `--worktree[=BRANCH]` und `--base` (SES-015, Engine und API ab M1, die UI dazu ab M3) sowie `--fork ID[@SEQ]` mit `--harness` (oder TARGET) und `--workspace new-worktree|shared|fresh` (SES-006, SES-007; meldet einen auf das Turn-Ende verschobenen Fork-Punkt auf stderr und hängt sich an den Fork); die übrigen Flags folgen mit ihren Features.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton run claude` ohne laufenden Daemon startet ihn, erstellt eine Session und streamt die Antwort; die Session ist danach in der Web-UI sichtbar.
  - [ ] AC2 — `beton run -c` setzt die zuletzt genutzte Session im aktuellen Verzeichnis fort.
  - [ ] AC3 — (ab M1) `--worktree` erzeugt einen Worktree gemäß SES-015 (siehe 07-sessions-collaboration.md) und startet die Session darin.
  - [ ] AC4 — `Ctrl+C` während eines Turns sendet Interrupt; zweimal `Ctrl+C` binnen 1 s detacht, ohne die Session zu stoppen.
- **Abhängigkeiten:** CLI-004, SES-001, SES-002 (siehe 07-sessions-collaboration.md), API-006

### CLI-003 — `resume`, `attach` & `session`-Verwaltung
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** `attach` verbindet sich mit einer laufenden Session (Replay + Live, `--read-only` ohne Eingaberecht), `resume` startet eine gestoppte Session neu. `session …` bietet Liste, Details, Umbenennen, Archivieren, Löschen, Fork, Freigabe (M4) und Interrupt. Session-Referenzen akzeptieren ID, ID-Präfix (eindeutig) oder `last`.
- **Details:** `last` ist die nicht archivierte Session mit der jüngsten Aktivität; ein Präfix darf `ses_` weglassen. `attach` zeigt den Verlauf ab `seq` 0 und danach live; `--read-only` sendet weder Eingaben noch Freigaben. `session rename` setzt den Titel über `PATCH /v1/sessions/{id}` (`title`, ohne laufenden Runner); `session show` ergänzt `cwd` und die Web-URL. `session delete` beantwortet die Rückfragen beim Entfernen eines Worktrees mit `--uncommitted commit|discard` und `--branch keep|delete` (SES-016).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton attach <präfix>` mit mehrdeutigem Präfix listet die Kandidaten und endet mit Exit-Code 2.
  - [ ] AC2 — (ab M1) `beton session fork <id>@120 --harness codex` erzeugt einen Fork laut SES-006/SES-007 (siehe 07-sessions-collaboration.md).
- **Abhängigkeiten:** SES-001, SES-003, SES-005 (siehe 07-sessions-collaboration.md), SES-006 (ab M1), COL-001 (ab M4)

### CLI-004 — `serve` & `host`
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** `beton serve` startet den Server (lokal: localhost-Bindung auf Port 7420, Token-Datei 0600, siehe AUTH-001; zentral: Postgres, OIDC per Config). `beton host` startet den Host-Daemon, der sich per ausgehendem WebSocket am Server anmeldet; `host enable|disable` installiert ihn als User-Service (launchd/systemd --user).
- **Details:** `serve` liest Default, User-Konfiguration und Env (nicht die Projektebene, CLI-008), hält einen exklusiven Lock auf `~/.beton/daemon.lock` (ein Daemon je Datenverzeichnis) und schreibt `~/.beton/run/daemon.json` (`pid`, `http`, `version`), über die CLI und SDK den Daemon finden. `--bind` ersetzt die Adressen aus `server.listen`, `--port` den Port (`0` = frei wählbar). Runner startet der Daemon als `beton __runner` aus demselben Binary (RUN-002). `--dev` (und Debug-Builds) schaltet den Fake-Harness frei (HAR-026). Der Blob-GC (DATA-006) läuft 10 min nach dem Start, danach täglich. `--foreground`, `--database-url` und `host` folgen mit dem Hintergrundbetrieb bzw. dem zentralen Server und RUN-006 (ab M4).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton serve` ohne Config bindet ausschließlich an Loopback; `--bind 0.0.0.0` ohne konfigurierte Auth wird verweigert.
  - [ ] AC2 — (ab M4, RUN-006) `beton host enable` erzeugt einen User-Service, der nach Reboot automatisch verbindet; `disable` entfernt ihn.
  - [ ] AC3 — `SIGTERM` an `serve` beendet sauber: laufende Runner werden benachrichtigt, Event-Log ist konsistent.
- **Abhängigkeiten:** AUTH-001 (siehe 05-security-identity.md), RUN-002, RUN-006 (ab M4, siehe 10-runners-extensibility.md)

### CLI-005 — `setup`, `doctor` & `diagnose`
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** `setup` führt interaktiv durch die Erstkonfiguration (erkennt `claude`/`codex`-CLIs und deren Login, **bietet** Installation fehlender CLIs an, nie still). `doctor` prüft Umgebung und gibt Handlungsempfehlungen; `diagnose` (M3) erzeugt ein secret-freies Support-Bundle. Inhalte der Prüfungen, `--json`-Schema und Exit-Codes: Owner OBS-005, `diagnose`: OBS-006 (siehe 11-platform-features.md); Setup-Logik: Owner HAR-016 (siehe 01-harnesses.md). CLI-005 regelt nur die CLI-Oberfläche.
- **Details:** `setup` fragt nur auf einem Terminal nach (`[y/N]`, Enter = Nein); ohne Terminal, mit `--non-interactive` oder `--check` installiert und meldet es nie etwas an. Exit-Code 1, wenn die CLI eines in diesem Release nutzbaren Harness fehlt (M0: `claude`; ab M1 auch `codex`). `setup acp add <SLUG> --command C [--arg A]…` trägt einen ACP-Agent in die User-Konfiguration ein (HAR-008). `setup --check --json` folgt `schemas/v1/setup-check.schema.json`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `setup --non-interactive` installiert nie etwas und meldet fehlende CLIs mit Exit-Code ≠ 0.
  - [ ] AC2 — `doctor --json` liefert pro Check `{id, status: ok|warn|fail, message, hint}` (OBS-005).
- **Abhängigkeiten:** HAR-016 (siehe 01-harnesses.md), OBS-005, OBS-006 (ab M3, siehe 11-platform-features.md), DIST-015 (ab M3, siehe 12-distribution-quality.md)

### CLI-006 — `login` & `profile`
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** `login <url>` meldet die CLI per OIDC (Browser-Flow mit Loopback-Callback, `--device-code` für Geräte ohne Browser) an einem Server an und legt ein Profil an; `profile` verwaltet Profile (geteilt mit Desktop). Host-Pairing per Code/QR läuft über `beton host pair` (RUN-004/AUTH-008, siehe 10-runners-extensibility.md, 05-security-identity.md). Tokens liegen im OS-Keychain.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach `beton login https://team.example` funktioniert `beton --server team session list` ohne weitere Eingabe.
  - [ ] AC2 — `beton logout` widerruft das Token serverseitig und entfernt es aus dem Keychain.
- **Abhängigkeiten:** AUTH-005, AUTH-008, AUTH-012 (siehe 05-security-identity.md), DESK-003

### CLI-007 — `import` & `export`
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** CLI-Front für den Import fremder Chats und JSONL-Export/-Import (SES-008/SES-009 in 07-sessions-collaboration.md; Format-Owner DATA-010). Ohne Argumente listet `import` Kandidaten interaktiv zur Auswahl.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton import --harness claude --last 5` importiert die fünf jüngsten Claude-Sessions und gibt deren beton-IDs aus.
  - [ ] AC2 — `beton export <id> -o s.jsonl && beton import s.jsonl` erzeugt eine inhaltsgleiche Session.
- **Abhängigkeiten:** SES-008, SES-009 (siehe 07-sessions-collaboration.md), DATA-010 (siehe 06-data-sync-protocol.md)

### CLI-008 — `config`
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Lesen/Schreiben von Konfiguration auf User- (`~/.beton/config.yaml`) und Projekt-Ebene (`.beton/config.yaml`); `list` zeigt die effektive Konfiguration mit Herkunft je Key. Validierung gegen das veröffentlichte JSON-Schema.
- **Details:** Vorrang `default` < `user` < `project` < `env`. Projektdatei ist das nächste `.beton/config.yaml` ab dem aktuellen Verzeichnis aufwärts (sonst unter der Git-Wurzel); das Datenverzeichnis `~/.beton` zählt nicht als Projekt. Env-Ebene: `BETON_CFG_<SCHLÜSSEL>` mit `__` als Trenner, Werte als YAML (z. B. `BETON_CFG_EVENTS__STORE_RAW=false`). Die Projektebene (also das Repository) darf nur `harnesses.*` setzen und dort keine Auth-Herkunft (HAR-015); daemonweite Schlüssel wie `server.*` und `events.*` gehören dem Benutzer. `set`/`unset` schreiben ohne Flag in die User-Datei; Werte werden als YAML gelesen (`false`, `7420`, `[a, b]`). Geprüft wird jede Ebene gegen `schemas/v1/config.schema.json` (generiert aus den Rust-Typen); beim Schreiben bleiben Kommentare der Datei derzeit nicht erhalten. `config edit` öffnet `$VISUAL`/`$EDITOR` und verwirft ungültige Änderungen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton config list` zeigt pro Key die Quelle (`default|user|project|env`).
  - [ ] AC2 — Ein ungültiger Wert wird mit Schema-Fehlermeldung abgelehnt, die Datei bleibt unverändert.
- **Abhängigkeiten:** —

### CLI-009 — `agent`
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Verwaltung von Agent-Definitionen: auflisten (eingebaut, User, Projekt), anzeigen, validieren gegen JSON-Schema, Gerüst erzeugen (`new`), Schema ausgeben (`schema`). Format und Semantik: Owner AGT-002/AGT-013 (siehe 02-agents.md); CLI-009 regelt nur die Konsistenz der CLI-Oberfläche.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton agent validate ./my-agent` meldet Schemafehler mit Pfad (`executor.harness`) und Exit-Code 1 (AGT-002).
  - [ ] AC2 — `beton agent new reviewer` erzeugt ein Verzeichnis, das `validate` besteht.
- **Abhängigkeiten:** AGT-002, AGT-003, AGT-013 (siehe 02-agents.md)

### CLI-010 — `usage`
- **Meilenstein:** M2 · **Priorität:** Should
- **Beschreibung:** Tabellarische Kosten-/Usage-Übersicht (gruppiert nach Session, Tag, Harness, Modell; Subscription-Usage separat) mit `--json`. Datenmodell und Implementierung: Owner USE-005/USE-007 (siehe 11-platform-features.md); CLI-010 regelt nur die Konsistenz der CLI-Oberfläche.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton usage --by model --since 2026-10-01 --json` liefert dieselben Summen wie die Usage-Seite.
  - [ ] AC2 — Subscription-Harnesses erscheinen mit Token-/Rate-Limit-Nutzung statt Geldbetrag; Mischsummen werden nicht gebildet.
- **Abhängigkeiten:** USE-005, USE-007 (siehe 11-platform-features.md)

### CLI-011 — `schedule`
- **Meilenstein:** M5 · **Priorität:** Should
- **Beschreibung:** Verwaltung von Schedules (`schedule list|create|update|pause|resume|delete|run-now`) und Lauf-Historie (`run-history`), inkl. Cron-Ausdruck, Agent, Prompt, Ziel-Runner-Labels und `catch_up`. Semantik und Felder: ASY-004/ASY-011 (siehe 02-agents.md); dieses Feature regelt nur die CLI-Oberfläche.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton schedule create --cron "0 7 * * 1-5" --agent triage --prompt "…"` legt einen Schedule an, der in `list` mit nächstem Ausführungszeitpunkt erscheint.
  - [ ] AC2 — `beton schedule run-now <name>` startet sofort einen Lauf und gibt die Session-ID aus; `--json` liefert `{run_id, session_id}`.
- **Abhängigkeiten:** ASY-004, ASY-011 (siehe 02-agents.md)

### CLI-012 — `plugin`
- **Meilenstein:** M5 · **Priorität:** Should
- **Beschreibung:** Installation und Verwaltung von Out-of-Process-Plugins aus Registry, Git oder Pfad (`install|list|update|remove|info|search`) sowie Entwickler-Kommandos (`new|validate|test|doctor`); vor Installation werden die deklarierten Berechtigungen angezeigt und müssen bestätigt werden. Semantik: PLG-005, PLG-008, PLG-011 (siehe 10-runners-extensibility.md); dieses Feature regelt nur Konsistenz der CLI-Oberfläche (Flags, `--json`, Exit-Codes).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton plugin install <name>` zeigt die Berechtigungen und installiert erst nach Bestätigung (`--yes` für Skripte).
  - [ ] AC2 — `beton plugin list --json` listet Name, Version, Typ (`harness|runner_provider|git_provider`, PLG-004) und Status.
- **Abhängigkeiten:** PLG-005, PLG-011 (siehe 10-runners-extensibility.md)

### CLI-013 — `upgrade` & `uninstall`
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** `upgrade` erkennt die Installationsart (Installer-Skript, Homebrew, winget, Scoop, cargo-binstall, deb/rpm) und aktualisiert passend bzw. nennt den richtigen Befehl; Kanäle `stable|beta|nightly`. Netzzugriff nur bei ausdrücklichem Aufruf; `--from-file` aktualisiert offline aus einem lokalen, signierten Release-Archiv (ADR-0033). `uninstall` entfernt Binary und Services, `--purge` zusätzlich `~/.beton/` nach Bestätigung. Implementierung: Owner DIST-016 (`upgrade`) und DIST-020 (`uninstall`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Bei Homebrew-Installation führt `upgrade` kein Self-Replace aus, sondern meldet `brew upgrade beton`.
  - [ ] AC2 — `upgrade` verweigert den Daemon-Neustart, solange Turns laufen, außer mit `--force`.
- **Abhängigkeiten:** DIST-016, DIST-020 (siehe 12-distribution-quality.md)

### CLI-014 — Shell-Completion
- **Meilenstein:** M3 · **Priorität:** Could
- **Beschreibung:** `beton completion <shell>` erzeugt Completion-Skripte (bash, zsh, fish, PowerShell) inkl. dynamischer Vervollständigung von Session-IDs und Profilen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — In zsh vervollständigt `beton attach <TAB>` die IDs der letzten 20 Sessions mit Titeln.
  - [ ] AC2 — Ist der Server nicht erreichbar, liefert die dynamische Vervollständigung binnen 300 ms leer zurück, ohne die Shell zu blockieren.
- **Abhängigkeiten:** CLI-001

### TUI-001 — ratatui-TUI: Session-Ansicht
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** `beton tui` öffnet eine Vollbild-TUI (ratatui + crossterm) mit Session-Liste, Chat-Stream (Markdown-Basisrendering, Tool-Cards einklappbar), Rail (Changes, Terminals, Approvals) und Composer gemäß Layout. Nutzt `beton-sdk` und dieselben Events wie die Web-UI.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Die TUI zeigt einen laufenden Turn synchron zur Web-UI (gleiche Event-Folge).
  - [ ] AC2 — Rendering bleibt bei 500 Tokens/s ohne sichtbares Flackern (Frame-Rate gedrosselt auf 30 fps, Diff-Rendering).
  - [ ] AC3 — Terminalgrößen ab 80×24 werden unterstützt; darunter erscheint ein Hinweis statt kaputtem Layout.
- **Abhängigkeiten:** API-005

### TUI-002 — Eigenes PTY-Multiplexing & Panes
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** PTYs laufen im Runner (`beton-pty`, auf Basis `portable-pty`; Unix-PTY bzw. ConPTY), **ohne tmux**. Die TUI rendert Terminal-Panes über einen VT-Parser (z.B. `vt100`/`alacritty_terminal` *(Annahme)*) in ratatui-Buffer; Panes lassen sich horizontal/vertikal teilen, fokussieren, zoomen. Prefix-Taste `Ctrl+G` *(Annahme, kollidiert nicht mit tmux/Vendor-TUIs)*.
- **Details:** PTY-Bytes über WS-Binärkanal (PROTO), Resize-Events zurück an den Runner. Detach/Reattach: PTY und Scrollback bleiben im Runner; Layout pro Session wird gespeichert.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `htop` in einem Pane rendert korrekt, Resize des Terminals passt das PTY an.
  - [ ] AC2 — TUI beenden und neu öffnen stellt Panes inkl. laufender Prozesse und Scrollback wieder her.
  - [ ] AC3 — Auf keinem Testsystem wird ein `tmux`-Binary aufgerufen (Prozess-Trace im Test).
- **Abhängigkeiten:** RUN-002 (siehe 10-runners-extensibility.md), PROTO-007 (siehe 06-data-sync-protocol.md)

### TUI-003 — Native-TUI-Modus (Vendor-TUI im Pane)
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Für Sessions im PTY/Native-TUI-Modus (`--mode tui`, z.B. Original-Claude-Code-TUI) zeigt die TUI das Vendor-TUI in einem Pane mit vollständiger Tastatur-Durchreichung; beton-Funktionen (Approvals aus Vendor-Hooks, Session-Wechsel) bleiben über den Prefix erreichbar.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Alle Tasten außer der Prefix-Sequenz erreichen das Vendor-TUI unverändert (inkl. `Esc`, `Shift+Tab`, Bracketed Paste).
  - [ ] AC2 — Dieselbe Native-TUI-Session ist parallel im Web-Terminal sichtbar und bedienbar.
- **Abhängigkeiten:** TUI-002, HAR-012 (siehe 01-harnesses.md)

### TUI-004 — Approval-Handling in der TUI
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Offene Approvals erscheinen als Overlay-Karte (Tool, Argumente, Policy, Begründung) mit Tasten `y` erlauben, `n` ablehnen, `s` für Session erlauben (falls zulässig), `d` Details. Approvals anderer Sessions erscheinen als Statuszeilen-Hinweis mit Sprung-Taste; auch im Native-TUI-Pane als Banner.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `y` auf einer Approval-Karte erzeugt `approval.resolved {decision: allow}` mit `actor` des TUI-Users.
  - [ ] AC2 — Ein Approval in einer nicht angezeigten Session erhöht den Zähler in der Statuszeile; `Ctrl+G a` springt hin.
- **Abhängigkeiten:** POL-010 (siehe 03-policies.md)

### TUI-005 — TUI auf Windows
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** Die TUI läuft in Windows Terminal und Konsole (crossterm-Backend, ConPTY), inklusive Pane-Multiplexing und Native-TUI-Modus, soweit der Harness auf Windows läuft.
- **Akzeptanzkriterien:**
  - [ ] AC1 — CI-Smoke-Test auf Windows: TUI starten, Fake-Harness-Session öffnen, Approval mit `y` entscheiden.
  - [ ] AC2 — Ein PowerShell-Pane funktioniert interaktiv (Eingabe, Farben, Resize).
- **Abhängigkeiten:** TUI-002

### API-001 — REST-API mit OpenAPI 3.1 (utoipa)
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Alle REST-Endpunkte sind in Rust annotiert und erzeugen über `utoipa` ein OpenAPI-3.1-Dokument unter `GET /v1/openapi.json`; Schemas stammen aus denselben Rust-Typen wie das WS-Protokoll. Eine Referenz-Doku wird daraus generiert.
- **Details:** Konventionen: Präfix `/v1`, JSON, `snake_case`, Cursor-Pagination (`?cursor=&limit=`, Antwort `{items, next_cursor}`), Fehler im Format RFC 9457 (`application/problem+json`, Feld `code` maschinenlesbar, z.B. `capability_unsupported`), Idempotency-Key-Header für POSTs, die Ressourcen erzeugen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Snapshot-Test des generierten OpenAPI-Dokuments in CI; Änderungen erfordern bewusstes Update.
  - [ ] AC2 — Jeder Endpunkt ist im Dokument enthalten (Test: Router-Routen ⊆ OpenAPI-Pfade).
  - [ ] AC3 — Alle Fehlerantworten validieren gegen das Problem-Schema.
- **Abhängigkeiten:** PROTO-010, PROTO-011, PROTO-013 (siehe 06-data-sync-protocol.md)

### API-002 — Ressourcen-Übersicht
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Grobe Endpunktliste; Details definieren die jeweiligen Kapitel. Endpunkte entstehen mit dem Meilenstein ihres Features.
- **Details:**

  | Ressource | Endpunkte (Auszug) | Kapitel |
  | --- | --- | --- |
  | System | `GET /v1/info`, `/healthz`, `/metrics` | OBS |
  | Sessions | `GET|POST /v1/sessions`, `GET|PATCH|DELETE /v1/sessions/{id}`, `POST …/archive|unarchive|interrupt|fork|compact|resume`, `GET …/events?after_seq=`, `POST …/input`, `GET|POST|PATCH|DELETE …/queue`, `GET …/subagents` (AGT-009), `GET …/export`, `GET …/events/stream` (SSE, PROTO-012) | 07 |
  | Workspace | `GET …/workspace/tree|search|changes|diff`, `GET|PUT …/workspace/files/{path}`, `GET|POST|DELETE …/terminals` | 07 |
  | Approvals | `GET …/approvals`, `POST …/approvals/{aid}/resolve` | 03 |
  | Collaboration | `…/shares`, `…/comments` (+ `/address`), `…/side-chats`, `GET /v1/inbox` | 07 |
  | Git | `…/change-requests`, `GET|POST /v1/git/connections` | 07 |
  | Browser | `…/browser/tabs`, `POST …/browser/mode`, `POST …/browser/pick` | 09 |
  | Projects | `GET|POST|PATCH|DELETE /v1/projects` | 07 |
  | Imports | `GET /v1/imports/candidates`, `POST /v1/imports` | 07 |
  | Agents/Harnesses | `GET /v1/agents`, `GET /v1/harnesses`, `GET /v1/models` | 01/02 |
  | Policies/Usage | `/v1/policies`, `POST /v1/policies/evaluate`, `GET /v1/usage` | 03/11 |
  | Hosts/Runner | `GET /v1/hosts`, `GET /v1/runners` | 10 |
  | Auth/Identität | `GET /v1/me`, `/v1/tokens` (PAT), `/v1/service-accounts`, `/v1/devices` | 05 |
  | Schedules/Webhooks | `/v1/schedules`, `POST /v1/triggers/{id}/fire` | 02 |
  | Push | `POST|DELETE /v1/push/subscriptions` | 08 |
  | WebSocket | `GET /v1/ws` (Upgrade; Session-Abos, Binärkanäle, Voice) | 06 |
- **Akzeptanzkriterien:**
  - [ ] AC1 — Für jeden Meilenstein existiert ein Contract-Test pro neu eingeführter Ressource (Happy Path + 403 + 404).
  - [ ] AC2 — Alle Endpunkte erzwingen die Autorisierung aus AUTH-015 (siehe 05-security-identity.md); ein unauthentifizierter Aufruf an einen Nicht-Loopback-Server liefert `401`.
- **Abhängigkeiten:** API-001

### API-003 — SSE-Stream für Skripte
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Read-only Server-Sent-Events unter `GET /v1/sessions/{id}/events/stream?from_seq=` für Werkzeuge ohne WebSocket (Owner des Endpunkts: PROTO-012); jedes Event trägt `id: <seq>`, sodass `Last-Event-ID` beim Reconnect lückenlos fortsetzt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `curl -N …/events/stream?from_seq=0` liefert alle bisherigen und danach Live-Events.
  - [ ] AC2 — Reconnect mit `Last-Event-ID: 42` liefert ab seq 43.
- **Abhängigkeiten:** PROTO-012 (siehe 06-data-sync-protocol.md)

### API-004 — TypeScript-SDK
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** `@ifahrentholz/beton-sdk` (`packages/sdk-ts`) mit generierten Typen (aus Rust via `ts-rs`/`specta`) und REST-Client (aus OpenAPI), plus handgeschriebenem WS-Client mit automatischem Resume ab `seq`. Die Web-UI nutzt ausschließlich dieses SDK.
- **Details:** Typen (Events, WebSocket-Nachrichten, REST-Modelle) sind aus den Rust-Typen generiert (`ts-rs`, `src/gen`); der REST-Client ist eine dünne, handgeschriebene Schicht über diesen Typen, `openapi/v1.json` bleibt der Vertrag (oasdiff in CI). Im Browser authentisiert das Session-Cookie, in Node das lokale Token (WebSocket-Header über die `webSocketFactory`; Node 20 ohne globales `WebSocket` übergibt das Paket `ws`). `fork()` folgt mit SES-006 (M1). API-Form: `const c = new BetonClient({baseUrl, token}); const s = await c.sessions.create({target: "claude", cwd}); for await (const ev of s.events({fromSeq: 0})) {…}; await s.send("…"); await s.interrupt(); await s.fork({atSeq: 120, harness: "codex"});`
- **Akzeptanzkriterien:**
  - [ ] AC1 — Typen werden in CI neu generiert; Abweichungen zum eingecheckten Stand lassen den Build fehlschlagen.
  - [ ] AC2 — Ein WS-Abbruch während des Streamings wird transparent mit Resume überbrückt (Test mit simuliertem Disconnect, keine Lücken/Duplikate).
  - [ ] AC3 — Das Paket läuft in Node ≥ 20 und modernen Browsern (ESM, keine Node-only-Abhängigkeiten im Browser-Pfad).
- **Abhängigkeiten:** API-001, PROTO-005, PROTO-013 (siehe 06-data-sync-protocol.md)

### API-005 — Rust-SDK
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Crate `beton-sdk` (async, `tokio`, `reqwest`, `tokio-tungstenite`) mit denselben Fähigkeiten wie das TS-SDK; typisiert über `beton-proto`. CLI und TUI nutzen ausschließlich dieses SDK für Server-Zugriffe (auch für Proben lokaler Modell-Server, `beton_sdk::probe`).
- **Details:** `Client::local(data_dir)` findet den Daemon über `run/daemon.json`; `Client::subscribe(session, from_seq)` liefert einen Event-Strom mit automatischem Reconnect (gleiche Parameter wie das TS-SDK), der `overflow`, `seq_ahead` und Close `4503` selbst behandelt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton-cli` und `beton-tui` haben keine direkten HTTP-Aufrufe außerhalb von `beton-sdk` (Lint/`cargo deny`-Regel oder Architekturtest).
  - [ ] AC2 — Beispiel `examples/stream.rs` erstellt eine Session gegen den Fake-Harness und gibt Deltas aus (läuft in CI).
- **Abhängigkeiten:** API-001

### API-006 — Skript-Modus `beton run -p`
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Nicht-interaktiver Lauf für Skripte und CI: Prompt per Argument oder stdin (`-p -`), finale Antwort auf stdout, Session-URL und Fortschritt auf stderr. Approvals ohne Zuschauer verhalten sich ab M5 wie bei Async-Agents (ASY-008: Inbox + Push, Pause mit Timeout, `on_timeout`); `--on-ask deny` lehnt sofort ab (für CI).
- **Details:** `--output-format text` (Default: nur Antworttext) · `json` (am Ende ein Objekt `{session_id, status, result, cost_usd, usage, duration_ms}`; `status`: `completed`, `failed`, `interrupted`, `denied`, `timed_out` – `--timeout` oder `executor.timeout`, AGT-004) · `stream-json` (NDJSON der PROTO-Events). `--max-cost` setzt ein Session-Budget, `--timeout` bricht ab (Exit-Code 1 bzw. 5).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `echo "sag hallo" | beton run claude -p - > out.txt` schreibt nur den Antworttext in `out.txt`; stderr enthält die Session-URL.
  - [ ] AC2 — `--output-format json` liefert genau ein valides JSON-Objekt auf stdout.
  - [ ] AC3 — Mit `--on-ask deny` endet ein Lauf mit gefordertem Approval mit Exit-Code 4, ohne zu hängen.
  - [ ] AC4 — (ab M2) Überschreitet der Lauf `--max-cost`, endet er mit Exit-Code 5.
- **Abhängigkeiten:** CLI-002, POL-011 (ab M2, siehe 03-policies.md), ASY-008 (ab M5, siehe 02-agents.md)
- **Referenz:** Omnigent `omni run -p`

## Nicht in v1

- Native Mobile-Apps (Tauri Mobile) – v1 nutzt PWA + Web-Push (Web-Push opt-in, WEB-014).
- VS-Code-Extension, Slack-Bot.
- UI-Extensions (sandboxed iframe + Message-Bridge).
- Branding/White-Label.
- Canvas-Ansicht.
- Python-SDK.
- Flatpak-Paket des Desktops.
- Prompt-Cleanup nach Diktat.
