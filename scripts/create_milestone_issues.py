#!/usr/bin/env python3
"""Legt Milestones, Labels und die Arbeitspaket-Issues eines Meilensteins auf GitHub an.

Rollierende Planung (siehe Issue #17 und AGENTS.md): Pro Meilenstein werden die
Arbeitspakete (WPS), ggf. Sonder-Issues für den Maintainer (HUMAN, Label `human-required`)
und das Abschluss-/Planungs-Issue (PLAN) unten definiert. Für den nächsten Meilenstein `MS`
umstellen und WPS/HUMAN/PLAN ersetzen. Die Definitionen früherer Meilensteine (M0: WP-01 bis
WP-16 und LEGAL) bleiben als Vorlage in der Git-History.

Das Skript prüft vorab, dass jedes Feature des Meilensteins (laut docs/spec) genau
einem Paket zugeordnet ist. Bereits existierende Issues (gleicher Titel) werden
übersprungen, Labels und Milestones idempotent angelegt.

Voraussetzung: `gh` ist angemeldet und hat Schreibrechte auf REPO.

Aufruf:
  python3 scripts/create_milestone_issues.py --check     # nur Zuordnung prüfen
  python3 scripts/create_milestone_issues.py --dry-run   # zeigt die gh-Aufrufe
  python3 scripts/create_milestone_issues.py             # legt alles an
"""
import json, re, glob, subprocess, sys, os

REPO = "ifahrentholz/beton"
ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
MS = "M1"  # Meilenstein, für den Issues angelegt werden
BLOB = f"https://github.com/{REPO}/blob/main"
DRY = "--dry-run" in sys.argv

# ---------- Spec einlesen ----------
feats = {}
for f in sorted(glob.glob(f"{ROOT}/docs/spec/[0-9]*.md")):
    cur = None
    for line in open(f, encoding="utf-8"):
        m = re.match(r"###\s+([A-Z]+-\d{3})\s+—\s+(.*)", line)
        if m:
            cur = {"id": m.group(1), "title": m.group(2).strip(), "file": os.path.basename(f), "heading": line[4:].strip()}
            continue
        if cur:
            m2 = re.search(r"\*\*Meilenstein:\*\*\s*(M\d|v2)\s*·\s*\*\*Priorität:\*\*\s*(\w+)", line)
            if m2:
                cur["ms"], cur["prio"] = m2.group(1), m2.group(2)
                feats[cur["id"]] = cur
                cur = None

def slug(h):
    s = h.strip().lower()
    s = re.sub(r"[^\w\- ]", "", s, flags=re.UNICODE)
    return s.replace(" ", "-")

def flink(fid):
    f = feats[fid]
    return f"[{fid} — {f['title']}]({BLOB}/docs/spec/{f['file']}#{slug(f['heading'])}) · {f['prio']}"

# ---------- Milestones ----------
MILESTONES = [
    ("M0 — Fundament", "`beton run claude` startet eine persistente Claude-Code-Session (Subscription über die offizielle CLI), die im Browser live mitläuft, nach einem Verbindungsabbruch ab `seq` fortgesetzt wird und einen Neustart des Daemons überlebt."),
    ("M1 — Meta-Harness", "Eine Session startet auf Claude Code, wird auf Codex geforkt und weitergeführt; `maestra` lässt Claude implementieren und Codex reviewen (parallel in eigenen Worktrees); vorhandene Claude- und Codex-Chats lassen sich importieren."),
    ("M2 — Kontrolle", "Ein Agent läuft im YOLO-Mode in der Sandbox: Er kann `~/.ssh` nicht lesen und nur erlaubte Hosts erreichen, er sieht Secrets nur als `bt_cred_*`, `git push --force` erzeugt eine Approval-Card, und bei Überschreitung des Budgets wird gestoppt."),
    ("M3 — Desktop & TUI (Release 0.1)", "Ein neuer Nutzer installiert die signierte Desktop-App, `beton setup` erkennt die CLI-Logins, und er arbeitet mit mehreren Sessions in Worktrees, nutzt den Inspect-Mode im eingebetteten Browser und diktiert Prompts per Push-to-Talk – vollständig lokal. Danach: erstes öffentliches Release 0.1."),
    ("M4 — Team", "Zwei Personen melden sich per OIDC an einem zentralen Server an, teilen eine Session, steuern sie gemeinsam, kommentieren inline und verfolgen den PR/MR im Panel; ein Laptop arbeitet offline mit einer Budget-Lease weiter und synchronisiert danach."),
    ("M5 — Autonomie & Breite (v1.0)", "Ein nächtlicher Schedule startet auf einem Kubernetes-Runner einen Async-Agent, der Dependencies aktualisiert und einen PR öffnet; eine `ask`-Policy pausiert ihn bis zur Freigabe vom Handy; ein Community-Plugin wird signiert installiert; Windows-Nutzer arbeiten mit der Beta-Sandbox. Danach: v1.0."),
]

# ---------- Labels ----------
LABELS = {
    "type:work-package": ("1d76db", "Arbeitspaket aus der Spec (ein PR bzw. Agent-Auftrag)"),
    "type:planning": ("5319e7", "Meilenstein-Abschluss bzw. Planung des nächsten Meilensteins"),
    "human-required": ("b60205", "Braucht eine Entscheidung oder Handlung des Maintainers; nicht von Agents erledigbar"),
    "security-review": ("d93f0b", "Merge nur nach expliziter Freigabe durch den Maintainer (AGENTS.md §5)"),
    "needs-vendor-cli": ("fbca04", "Braucht eingeloggte Vendor-CLI (claude/codex) lokal; CI nutzt Golden Transcripts"),
    "area:core": ("c5def5", "beton-core, Workspace, Querschnitt"),
    "area:protocol": ("c5def5", "beton-proto, Event-Modell, WebSocket"),
    "area:store": ("c5def5", "beton-store, Persistenz"),
    "area:server": ("c5def5", "beton-server, REST/OpenAPI"),
    "area:runner": ("c5def5", "beton-host, beton-runner, Tunnel"),
    "area:harness": ("c5def5", "beton-harness und Adapter"),
    "area:cli": ("c5def5", "beton-cli"),
    "area:web": ("c5def5", "apps/web"),
    "area:sdk": ("c5def5", "beton-sdk, packages/sdk-ts"),
    "area:qa": ("c5def5", "Tests, CI, Qualität"),
    "area:security": ("c5def5", "Auth, Sandbox, Proxy, Secrets"),
    "area:legal": ("c5def5", "Lizenzen, Nutzungsbedingungen, Markenrecht"),
    "area:release": ("c5def5", "Distribution, Signing, Updates, Domain"),
}

# ---------- Arbeitspakete M1 ----------
# Die Definitionen für M0 (WP-01 … WP-16, LEGAL) stehen in der Git-History dieses Skripts.
# Gemeinsame Hinweise (ADR-0032 Design-first, ADR-0033 lokal, ADR-0034 Subscription-first) stehen pro Paket,
# damit jedes Issue für sich verständlich ist.
def design(screens, extra=""):
    """Hinweis auf die Prototyp-Screens (ADR-0032); `screens` sind Screen-IDs aus design/prototype/src/screens."""
    s = ", ".join(f"`{x}`" for x in screens.split())
    return (f"**Design-first (ADR-0032):** UI nur nach dem abgenommenen Prototyp (`design/prototype`), Screens {s}{extra}. "
            "Abweichungen im PR begründen. Hat ein Feature einen Screen, aber kein UI-AC, wird die Ansicht nicht stillschweigend "
            "mitgebaut, sondern als eigenes Issue im Milestone M1 angelegt.")
SUBSCRIPTION = "**Subscription-first (ADR-0034, ADR-0005):** Default-Auth ist der Login der Vendor-CLI; beton liest, speichert oder verwendet keine Subscription-Tokens. Kein Feature setzt einen API-Key voraus; Tests laufen ohne `*_API_KEY` im Env."
LOCAL = "**Lokal ohne externe Server (ADR-0033):** Tests nur mit Fake-CLIs, Golden Transcripts und Mocks auf Loopback; kein Internet."

WPS = [
 dict(key="WP-17", title="Worktrees pro Session & Workspace-API", labels=["area:runner", "area:server"], deps=[],
      features=["SES-015", "SES-016", "SES-017", "SES-018"],
      goal="Jede Session kann in einem eigenen `git worktree` arbeiten, und Clients lesen, durchsuchen und ändern den Workspace über eine abgesicherte API inklusive Änderungs- und Diff-Sichten.",
      notes=[
        "Neues Crate `beton-git` (00-overview §4.4) nur mit dem M1-Umfang (Worktrees); Git-Provider (GitHub/GitLab) kommen erst mit M4.",
        "SES-015: Engine und API (`--worktree`, Fork mit `new_worktree`, Sub-Agents mit `worktree: new`) ab M1; die UI dazu (Branch-Auswahl im Composer, Project-Defaults) erst mit M3. „(ab M2)“-AC (Sandbox mit Worktree als Schreib-Root) nicht vorziehen.",
        "**Klären:** Die Übernahme aus WP-11 (#12) ordnet `beton run --worktree/--base` M3 zu, SES-015 nennt `--worktree` ab M1. Spec (CLI-002/SES-015) im PR angleichen.",
        "SES-015 AC5: Ohne erreichbares Remote wird der Worktree aus dem lokalen Stand der Base erstellt (ADR-0033); Tests ohne Netzwerk.",
        "SES-017: Pfadbegrenzung (`..`, Symlinks aus dem Workspace hinaus → `403`) test-first, gern mit Property-Tests (QA-001). Berührt die Umsetzung Auth-Code in `beton-server`, Label `security-review` nachziehen.",
        "SES-018: Diffs zeilengenau adressierbar – Grundlage für die Diff-Ansicht (WEB-011, WP-24) und Inline-Kommentare (M4).",
        design("workspace-new-session workspace-worktrees workspace-files workspace-changes", "; die Web-Ansichten selbst kommen mit WP-24 bzw. M3"),
      ]),
 dict(key="WP-18", title="Codex-Adapter (app-server), ACP-Harness & Contract-Suite", labels=["area:harness", "area:qa", "needs-vendor-cli"], deps=[],
      features=["HAR-006", "HAR-007", "HAR-008", "QA-016"],
      goal="Codex (`codex app-server`) und beliebige ACP-Agents als Harnesses anbinden, jeweils mit dem Login ihrer eigenen CLI, und alle Adapter gegen eine gemeinsame Contract-Suite prüfen.",
      notes=[
        "**Erster Schritt:** alle in 01-harnesses.md als *(Annahme)* markierten Flags, RPC-Methoden und Nachrichtenformate gegen die aktuelle `codex`-CLI bzw. einen ACP-Agent (z. B. Gemini CLI) verifizieren, Abweichungen in der Spec korrigieren (im selben PR, klar markiert) und Golden Transcripts aufnehmen (`beton dev record-golden`, Secret-Scan `cargo xtask golden-scan`).",
        "Aus WP-07 (#8): Fake-CLI-Protokolle `app-server` (Codex) und `acp` in `beton-fake-cli` (QA-002). CI nutzt nur Fake-CLIs und Golden Transcripts.",
        "Aus WP-07 (#8): HAR-025 AC3, Golden-Szenariosätze für Codex und ACP.",
        "Aus WP-07 (#8): HAR-002 AC1 vollständig – Katalogeinträge `codex` und `acp:*` hier, `direct:*` mit WP-25.",
        "Codex `app-server` und ACP sprechen beide JSON-RPC 2.0 über stdio: einen gemeinsamen Transport in `beton-harness` bauen (ADR-0005), nicht zwei.",
        SUBSCRIPTION,
        "ACP-Presets (HAR-008) werden nur aktiv, wenn das Binary gefunden wird; nichts wird still installiert (ADR-0026, ADR-0033).",
        "Approvals laufen in M1 über das bestehende `PolicyGate` und die minimale Approval-Karte; „ab M2“-Abhängigkeiten von HAR-007 (SBX-002, POL-016, POL-023) nicht vorziehen.",
        "QA-016: Die Contract-Suite prüft Claude, Codex, ACP und den Fake-Harness über Fake-CLIs; `direct` kommt mit WP-25 dazu. Sie bleibt für Harness-Plugins (PLG-013, M5) wiederverwendbar.",
        design("harness-catalog harness-acp harness-setup", " (`beton setup acp add`)"),
      ]),
 dict(key="WP-19", title="Agent-Format v1, JSON-Schema, Verzeichnis & `beton agent`", labels=["area:core", "area:cli"], deps=[],
      features=["AGT-001", "AGT-002", "AGT-003", "AGT-013", "CLI-009"],
      goal="Agents als validierte YAML-Definitionen (Format v1 mit generiertem JSON-Schema), aufgelöst über einen Suchpfad und verwaltet mit `beton agent`.",
      notes=[
        "Neues Crate `beton-agents`. Die Rust-Typen sind Single Source of Truth; das Schema entsteht per `cargo xtask codegen` (AGT-002 nennt `schemas/agent.v1.json`, AGENTS.md `schemas/v1/`; Pfad im PR vereinheitlichen).",
        "Format v1 kennt alle Top-Level-Felder (auch `timers`, `schedules`, `async`, `policies`, `sandbox`), unbekannte Felder sind Fehler. Die Wirkung der Felder späterer Meilensteine (M2, M5) wird nicht vorgezogen.",
        "`beton agent new` verweist im `$schema`-Kommentar auf die lokal abgelegte Schema-Datei, nie auf eine URL (ADR-0033).",
        "Built-in-Agents liegen unter `agents/` und werden ins Binary eingebettet (AGT-003); `maestra` und `duetto` selbst kommen mit WP-27.",
        "AGT-013 ist Owner der Semantik, CLI-009 nur der CLI-Konsistenz; deshalb beide in diesem Paket.",
        "Aus WP-04 (#5): PROTO-011 AC2 am echten Endpunkt – entsteht ein Endpunkt zum Hochladen bzw. Validieren von Agent-YAML, meldet er Fehler mit JSON-Pointer (`extract::parse_json`).",
        "Aus WP-14 (#15): Navigationseintrag „Agents“ (WEB-001) gehört zur Agent-Bibliothek.",
        design("agent-library agent-detail agent-cli cli-agent", " (Agent-Detail mit YAML-Schema-Prüfung)"),
      ]),
 dict(key="WP-20", title="Queue & Steer, Session-Suche & Feature-Flags", labels=["area:server", "area:web", "area:core"], deps=["WP-18"],
      features=["SES-004", "WEB-005", "SES-012", "UX-007"],
      goal="Inputs während eines laufenden Turns einreihen oder in den Turn einspeisen, Sessions filtern, durchsuchen und als gelesen markieren, und unfertige Funktionen hinter internen Feature-Flags halten.",
      notes=[
        "UX-007 früh liefern, damit andere M1-Pakete unfertige Funktionen dahinter verstecken können; aktive Flags in `GET /v1/info`, unbekannte Flags als Warnung auch in `beton doctor`.",
        "SES-004 AC3 (Steering nativ) mit Codex über die Fake-CLI aus WP-18; ohne Capability `steering` antwortet die API mit `409 capability_unsupported`.",
        "Queue serverseitig und für alle Clients mit `drive` sichtbar; Reorder/Delete ≤ 500 ms beim anderen Client (Mehr-Client-Test).",
        "Aus WP-14 (#15): WEB-004 „Steer“ während eines Turns im Composer, zusammen mit WEB-005.",
        "Aus WP-14 (#15): WEB-003 Gruppe „Pinned“ der Session-Liste mit SES-012. Die Übernahme nennt Projekte als M1, laut Spec ist SES-013 aber M3; Gruppen „Projekte“ (M3) und „geteilt“ (M4) nicht vorziehen.",
        "SES-012: Volltextsuche über Titel und Nachrichten so bauen, dass Postgres (DATA-004, M4) später die gleiche Semantik liefern kann.",
        "WEB-005: Der Interrupt-Shortcut (`Esc` doppelt) ist in der Spec als *(Annahme)* markiert; mit dem Prototyp abgleichen und festschreiben.",
        design("session-stream collab-codrive workspace-sessions settings-flags diag-doctor", " (Queue über dem Composer in `session-stream`)"),
      ]),
 dict(key="WP-21", title="Fork ab Event X & Harness-Wechsel mit Handover-Kontext", labels=["area:harness", "area:server", "area:web", "needs-vendor-cli"], deps=["WP-17", "WP-18"],
      features=["HAR-018", "HAR-019", "SES-006", "SES-007"],
      goal="Sessions ab einem beliebigen Event forken: auf demselben Harness mit nativer History, auf einem anderen Harness mit Handover-Kontext (Claude Code → Codex und zurück).",
      notes=[
        "Kern des M1-Demo-Szenarios (Claude-Session → Fork auf Codex → weiterführen): Fake-Harness-Tests plus manuelle Verifikation mit echter `claude`- und `codex`-CLI als Checkliste im PR.",
        "HAR-019: `--resume <id> --fork-session` und die Rekonstruktion einer Vendor-Session-Datei aus dem Event-Log gegen die echte `claude`-CLI verifizieren (Annahmen in 01-harnesses.md), Golden Transcripts aufnehmen.",
        "Ein Harness-Wechsel geschieht ausschließlich per Fork, die Ursprungs-Session bleibt auf ihrem Harness (HAR-018, SES-007 AC5).",
        "Der Fork wählt den Workspace-Modus explizit, inklusive `new_worktree` (SES-015 aus WP-17).",
        "„(ab M2)“-AC von SES-007 (Handover ohne Secret-Werte, SEC-013) nicht vorziehen.",
        "Aus WP-11 (#12): `beton run --fork` (CLI-002) mit SES-006.",
        design("workspace-fork harness-notices workspace-cli", " („Weiter mit …“, Banner „Fortgesetzt aus …“)"),
      ]),
 dict(key="WP-22", title="Import vorhandener Claude-Code- und Codex-Chats", labels=["area:harness", "area:server", "area:web", "needs-vendor-cli"], deps=["WP-21"],
      features=["HAR-023", "HAR-024", "SES-008"],
      goal="Vorhandene Claude-Code- und Codex-Chats als beton-Sessions übernehmen, dedupliziert und fortsetzbar.",
      notes=[
        "Teil des M1-Demo-Szenarios.",
        "**Erster Schritt:** die Formate unter `~/.claude/projects/*/*.jsonl` und `~/.codex/sessions/…/rollout-*.jsonl` (bzw. `$CODEX_HOME`) gegen aktuelle CLI-Versionen verifizieren, Spec bei Abweichungen korrigieren. Anonymisierte Beispieldateien als Fixtures bzw. Goldens (Secret-Scan `cargo xtask golden-scan`), keine echten Nutzerdaten im Repo.",
        "Vendor-Dateien werden nur gelesen, nie verändert; Discovery nur in den Transcript-Verzeichnissen, keine Credential-Dateien der CLIs (ADR-0005).",
        "SES-008 AC2: Eine importierte Claude-Session ist nativ fortsetzbar, solange die Vendor-Datei existiert, sonst per Handover (HAR-018 aus WP-21).",
        "Projektzuordnung importierter Sessions (SES-008 AC3, SES-013) ist M3. Die CLI-Front `beton import` ist CLI-007 in WP-29; für das Demo-Szenario reichen API und UI.",
        design("workspace-import-export harness-import"),
      ]),
 dict(key="WP-23", title="MCP-Injektion, System-Tools (`beton`-MCP) & Skills", labels=["area:harness", "area:security", "security-review", "needs-vendor-cli"], deps=["WP-18", "WP-19"],
      features=["HAR-009", "AGT-006", "AGT-007", "AGT-008"],
      goal="Tools gibt es ausschließlich als MCP-Server: Agent-, Projekt- und User-MCP-Server plus der eingebaute `beton`-MCP-Server mit System-Tools und Skills erreichen jeden Harness.",
      notes=[
        "Neues Crate `beton-mcp` (MCP-Client/-Server, System-Tools).",
        "**Erster Schritt:** wie die MCP-Konfiguration je Harness ankommt (Claude, Codex, ACP) gegen die echten CLIs verifizieren, Annahmen in 01-harnesses.md/02-agents.md korrigieren, Golden Transcripts aufnehmen.",
        "Sicherheitskritisch: Der `beton`-MCP-Server gibt Agents Zugriff auf beton selbst (z. B. `session_spawn`, `ask_user`). Authentisierung pro Session, sichtbar sind nur freigeschaltete Tools (`tools.system`), fail closed. **Strikt TDD**, Merge erst nach Maintainer-Freigabe.",
        "„ab M2“-Abhängigkeiten (SBX-002, PRX-006, POL-027, UX-001) nicht vorziehen. MCP-Server laufen in M1 ohne Sandbox; im PR-Text vermerken.",
        "HTTP-MCP-Server nur, wenn konfiguriert; keine Verbindungen nach außen ohne Konfiguration (ADR-0033).",
        "AGT-008: Skill-Discovery inklusive `.claude/skills/` und `.agents/skills/`; die Skills stehen auch dem Slash-Menü (WEB-006, WP-30) zur Verfügung.",
        design("agent-detail harness-catalog harness-notices", " (Tools, System-Tools, Skills im Agent-Detail)"),
      ]),
 dict(key="WP-24", title="Web-UI: Workspace-Rail, Diff-Ansicht & Code-Editor", labels=["area:web"], deps=["WP-17"],
      features=["WEB-008", "WEB-011", "WEB-009"],
      goal="Rechte Workspace-Rail in der Web-UI mit Dateibaum, Änderungen, Diff-Ansicht und Monaco-Editor auf Basis der Workspace-API.",
      notes=[
        design("session-stream workspace-files workspace-changes", " (Rail und Diff in `session-stream`)"),
        "Nur die M1-Tabs (Files, Changes, Agents) bauen; Terminals und Browser (M3), PR/MR, Side-Chats und Kommentare (M4) nicht vorziehen. Der Agents-Tab zeigt den Sub-Agent-Graph (WEB-012), der mit WP-27 kommt.",
        "Aus WP-14 (#15): Navigation bzw. Workspace-Rail (WEB-001, WEB-008).",
        "Monaco und Shiki aus dem eigenen Bundle, kein CDN-Loader (WEB-001, ADR-0033); lazy laden und die Performance-Budgets (WEB-015) einhalten.",
        "Speichern mit `If-Match`; `412` führt zum Konfliktzustand laut Prototyp.",
        "E2E (Playwright gegen Fake-Harness): Datei öffnen, ändern, speichern, Diff ansehen.",
      ]),
 dict(key="WP-25", title="Direkt-API-/Gateway-Harness & Compaction", labels=["area:harness", "area:security", "security-review", "needs-vendor-cli"], deps=["WP-23"],
      features=["HAR-010", "HAR-011", "HAR-022", "SES-011"],
      goal="Eigener Agent-Loop für API-Keys und Gateways als zusätzliche Option, dazu Compaction und Kontextfüllstand über alle Harnesses.",
      notes=[
        "**ADR-0034:** Der Direkt-API-Harness ist nur eine zusätzliche Option; kein anderes Feature darf ihn voraussetzen. Ohne `providers`-Konfiguration funktioniert alles andere unverändert.",
        "Tests nur gegen lokale Mocks (Anthropic- und OpenAI-Wire-Format inklusive Tool-Calls und Streaming) auf Loopback, ohne echten API-Key (AGENTS.md §3a, ADR-0033). Lokale Gateways wie Ollama oder LM Studio brauchen keinen Key.",
        "Sicherheitskritisch: Mit `api_key_env` darf der Key-Wert nie in Event-Log, Logs oder Config-Dateien landen (Test mit Marker-Wert, HAR-011). `secret://` und Proxy-Platzhalter kommen mit M2 und werden nicht vorgezogen. **Strikt TDD**, Merge erst nach Maintainer-Freigabe.",
        "Jeder Tool-Call geht durch das `PolicyGate`; die Ausführung läuft über MCP (WP-23).",
        "Aus WP-07 (#8): HAR-002 AC1 – Katalogeintrag `direct:*`; den Direct-Adapter in die Contract-Suite (QA-016, WP-18) aufnehmen.",
        "Aus WP-08 (#9): HAR-022 – Compaction-Durchreichung im Claude-Adapter (`compact()` meldet bis dahin `capability_unsupported`). Den Mechanismus bei Codex gegen die echte CLI verifizieren.",
        "SES-011 liefert den Kontextfüllstand als Event. Die Anzeige (USE-008) und der Trigger (USE-009) sind M2.",
        design("harness-providers harness-catalog harness-notices workspace-resume"),
      ]),
 dict(key="WP-26", title="Agent-Ausführung: Executor, Snapshot, Instructions & Parameter", labels=["area:harness", "area:core", "needs-vendor-cli"], deps=["WP-19", "WP-25"],
      features=["AGT-004", "AGT-005", "AGT-010"],
      goal="Agents starten: Executor auflösen, Agent-Snapshot pro Session festhalten, Instructions an jeden Harness ausliefern und Parameter übergeben.",
      notes=[
        "Aus WP-11 (#12): `beton run <agent>` (Agent als TARGET) und `--param k=v` (CLI-002).",
        "Aus WP-08 (#9): HAR-004 AC5, Agent-Instructions im Claude-Adapter über `--append-system-prompt-file`.",
        "AGT-005 AC1 verlangt alle vier Harness-Typen (Claude, Codex, ACP, Direkt), daher die Abhängigkeit auf WP-25. Bei `project_files: auto` gegen die echten CLIs prüfen, welcher Harness `CLAUDE.md` bzw. `AGENTS.md` selbst liest.",
        "Snapshot mit Inhalts-Hash im Blob-Store (DATA-006); `agent.resolved` hält Forks (WP-21) konsistent.",
        "AGT-010: Parameter aus CLI, UI-Formular und `session_spawn`; Schedule- und Webhook-Quellen (ASY, M5) nicht vorziehen.",
        design("agent-detail agent-start agent-cli", " (Parameter-Formular in `agent-start`)"),
      ]),
 dict(key="WP-27", title="Sub-Agents über Harness-Grenzen, `maestra` & `duetto`", labels=["area:harness", "area:web", "area:qa", "needs-vendor-cli"], deps=["WP-21", "WP-22", "WP-23", "WP-24", "WP-26"],
      features=["AGT-009", "AGT-011", "AGT-012", "WEB-012"],
      goal="Sub-Agents auf beliebigen Harnesses und die Built-in-Agents `maestra` (Claude implementiert, Codex reviewt, parallel in eigenen Worktrees) und `duetto`; schließt das M1-Demo-Szenario ab.",
      notes=[
        "Das Demo-Szenario (Milestone-Beschreibung) wird hier zum automatisierten E2E-Test mit Fake-Harnesses: Claude-Session → Fork auf Codex → weiter; `beton run maestra` (Plan → zwei parallele Implementierungen in eigenen Worktrees → Cross-Review → Zusammenfassung); Import vorhandener Claude- und Codex-Chats. Dazu eine manuelle Verifikation mit echten, eingeloggten `claude`- und `codex`-CLIs als Checkliste im PR.",
        SUBSCRIPTION + " AGT-011 AC5 prüft genau das (keine `*_API_KEY`, keine `providers`, `auth_source: vendor_cli`).",
        "Built-ins als reines YAML unter `agents/maestra/` und `agents/duetto/`. maestra merged nie, Implementer committen auf eigene Branches.",
        "**Klären:** AGT-009 AC1 (Codex-Child mit `worktree: new`) ist als „ab M3“ markiert, AGT-011 AC1 verlangt in M1 aber eigene Worktrees je Implementierung. Spec im PR angleichen.",
        "„(ab M2)“-AC von AGT-011 (Policy lehnt `Bash`/`Edit` für maestra ab) nicht vorziehen.",
        "WEB-012: xyflow aus dem eigenen Bundle, im Agents-Tab der Workspace-Rail (WP-24), aktualisiert sich live.",
        design("workspace-agents maestra-score duetto-debate subagent-tree agent-library agent-start"),
      ]),
 dict(key="WP-28", title="Modell-, Effort- & Permission-Mode-Wechsel, Warm-Resume", labels=["area:harness", "area:web", "area:security", "security-review", "needs-vendor-cli"], deps=["WP-21", "WP-25"],
      features=["HAR-017", "HAR-027", "HAR-020"],
      goal="Modell, Reasoning-Effort und Permission-Mode einheitlich über alle Harnesses steuern und laufende Sessions nach einem Runner-Neustart warm fortsetzen.",
      notes=[
        "Sicherheitskritisch (HAR-027): `yolo` startet nur mit Sandbox Stufe 2 und Egress-Proxy. Beide kommen erst mit M2, in M1 endet `yolo` also immer mit `sandbox_required` (fail closed, AC1). **Strikt TDD**, Merge erst nach Maintainer-Freigabe.",
        "Aus WP-11 (#12): `beton run --effort/--permission-mode` (CLI-002) inklusive Session-Einstellungen über PATCH bzw. `session.set`.",
        "Aus WP-14 (#15): WEB-004 – Permission-Mode-Picker und Effort beim Anlegen einer Session bis zum Runner durchreichen (`CreateSessionRequest`, `SessionSpec`).",
        "Die Mechanismen je Adapter (live oder Neustart mit Resume; Claude `--resume`, Codex `thread/resume`, ACP `session/load`) gegen die echten CLIs verifizieren und die Contract-Suite (QA-016) um Modellwechsel und Resume erweitern.",
        "HAR-020: `native_session_ref` vor dem ersten Turn persistieren; ohne Warm-Resume greift die Präambel aus HAR-018 (WP-21).",
        design("harness-switching harness-notices agent-detail"),
      ]),
 dict(key="WP-29", title="Session-Export/-Import (JSONL) & `beton import`/`export`", labels=["area:store", "area:cli"], deps=["WP-22"],
      features=["DATA-010", "SES-009", "CLI-007"],
      goal="Sessions verlustfrei als JSONL (mit Blobs als `.tar.zst`) exportieren und auf einer anderen Instanz importieren; `beton import`/`export` als CLI-Front, auch für fremde Chats.",
      notes=[
        "DATA-010 ist Owner von Format und Implementierung; der Import validiert jede Zeile gegen das generierte Schema (QA-006).",
        "Round-Trip-Tests: Export → Import ergibt dieselben Events bis auf neue Session-ID und `imported_from`, auch mit Blobs.",
        "`beton import` ohne Argumente listet die Kandidaten aus WP-22 (Claude- und Codex-Chats) interaktiv; ohne TTY nicht interaktiv mit den Exit-Codes aus CLI-001.",
        "Export und Import nur über lokale Dateien (ADR-0033).",
        design("workspace-import-export settings-data cli-import"),
      ]),
 dict(key="WP-30", title="SSE-Stream, automatische Session-Titel & Composer-Erweiterungen", labels=["area:protocol", "area:server", "area:web", "needs-vendor-cli"], deps=["WP-17", "WP-23"],
      features=["PROTO-012", "API-003", "SES-010", "UX-009", "WEB-006"],
      goal="Read-only SSE-Stream für Skripte, automatische Session-Titel über die eingeloggte Vendor-CLI und ein Composer mit Anhängen, @-Mentions und Slash-Menü.",
      notes=[
        "PROTO-012 ist Owner des Endpunkts, API-003 beschreibt die Skript-Sicht. Resume über `Last-Event-ID` bzw. `?from_seq=` lückenlos testen (wie das WebSocket-Resume aus WP-05).",
        "SES-010 (ADR-0034): Titel entstehen über den Einmal-Modus einer eingeloggten Vendor-CLI (z. B. `claude -p`, `codex exec`) bzw. den Harness der Session, nie über einen API-Key. Den Einmal-Modus gegen die echten CLIs verifizieren. Vom User gesetzte Titel werden nie überschrieben; die Einstellung „Titel automatisch erzeugen“ (UX-009) schont das Subscription-Kontingent.",
        "WEB-006: `@`-Suche über die Workspace-API (WP-17), Slash-Menü inklusive Skills (WP-23); nur Befehle anzeigen, die es schon gibt (z. B. `/fork` ab WP-21, `/compact` ab WP-25).",
        "Attachments: Limits serverseitig konfigurierbar, Ablage im Blob-Store (DATA-006).",
        design("workspace-composer inbox-titles workspace-resume cli-sse diag-events"),
      ]),
]

# ---------- human-required: offene Punkte aus 00-overview §9, fällig vor M3 ----------
# Punkt 4 (Nutzungsbedingungen) ist Issue #1 aus M0. Punkt 3 (GitHub-Org/npm-Scope, „bei Bedarf“) und
# Punkt 6 (Crate-Name `beton`, „optional“) haben keine Frist und bekommen bewusst kein Issue.
# `ms` legt den Milestone fest (Default: MS); die Punkte sind vor dem ersten Release (M3) fällig.
HUMAN = [
 dict(key="HR-SIGNING", ms="M3", title="Signing-Accounts für macOS und Windows klären und einrichten", labels=["human-required", "area:release"], deps=[], features=[],
      goal="Offener Punkt 1 aus `docs/spec/00-overview.md` §9 und ADR-0026: Entscheiden, wer die Signing-Accounts hält (Apple Developer Program für Developer-ID und Notarisierung, Azure Trusted Signing für Windows), und sie einrichten, damit die Desktop-App in M3 signiert ausgeliefert werden kann.",
      notes=[
        "Voraussetzungen, Kosten und Dauer der Identitätsprüfung beim jeweiligen Anbieter aktuell prüfen; die Verifizierung kann dauern, deshalb früh beginnen.",
        "Zertifikate und Schlüssel nie ins Repo; nur als geschützte CI-Secrets der Release-Pipeline (Label `security-review` für die Pipeline-Arbeit).",
        "Linux- und Container-Signing (Sigstore/cosign, SBOM, SLSA) brauchen keinen kostenpflichtigen Account und sind nicht Teil dieses Issues.",
        "Ergebnis (Kontoinhaber, Ablageort der Zugangsdaten) als Kommentar festhalten und ADR-0026 per Folge-ADR ergänzen.",
        "Fällig vor M3 (erstes öffentliches Release 0.1); Gate für das signierte Desktop-Paket im M3-Demo-Szenario.",
      ]),
 dict(key="HR-DOMAIN", ms="M3", title="Domain für beton prüfen und sichern", labels=["human-required", "area:release"], deps=[], features=[],
      goal="Offener Punkt 2 aus `docs/spec/00-overview.md` §9 und ADR-0028: Klären, ob und welche Domain (z. B. `beton.dev`, `getbeton.dev`) beton bekommt, Verfügbarkeit prüfen und sie gegebenenfalls registrieren.",
      notes=[
        "Erst nach bzw. zusammen mit der Markenprüfung entscheiden (siehe Issue „Markenrecht „beton“ prüfen“).",
        "Das Produkt darf zur Laufzeit von keiner Domain abhängen (ADR-0033); eine Domain dient nur Doku, Website und Installationshinweisen.",
        "Ergebnis als Kommentar festhalten und ADR-0028 per Folge-ADR ergänzen; danach Doku-Links zentral anpassen.",
        "Fällig vor M3 (erstes öffentliches Release 0.1).",
      ]),
 dict(key="HR-TRADEMARK", ms="M3", title="Markenrecht „beton“ prüfen", labels=["human-required", "area:legal"], deps=[], features=[],
      goal="Offener Punkt 5 aus `docs/spec/00-overview.md` §9 und ADR-0028: Prüfen, ob der Name „beton“ für eine Software-Entwicklerwerkzeug-Marke kollidiert, und entscheiden, ob der Name bleibt (und ggf. geschützt wird) oder vor dem ersten Release geändert wird.",
      notes=[
        "Recherche in den relevanten Registern und Klassen für Software; bei Unklarheit fachlichen Rat einholen.",
        "Eine Umbenennung nach dem Release ist teuer: Binary, Crates `beton-*`, npm-Scope-Pakete, Pfade `~/.beton/` und `.beton/`, Deep-Links `beton://`, Platzhalter `bt_cred_*` (ADR-0028). Deshalb vor M3 entscheiden.",
        "Verwandt, aber ohne Frist: belegter Crate-Name `beton` auf crates.io (§9 Punkt 6, optional).",
        "Ergebnis als Kommentar festhalten; bei Namensänderung neues ADR, das ADR-0028 ablöst.",
        "Fällig vor M3 (erstes öffentliches Release 0.1).",
      ]),
]

MAINTAINER_RULE = """## Vorgabe des Maintainers: Arbeitsstand vollständig in GitHub
Sobald die Planung eines Meilensteins steht, muss **seine gesamte Arbeit als Issues in GitHub sichtbar** sein, damit der Arbeitsstand jederzeit nachvollziehbar ist:
- Jedes Feature des Meilensteins ist genau einem Arbeitspaket-Issue zugeordnet (Checkliste mit Feature-ID und Spec-Link); vor dem Anlegen per Skript prüfen, dass nichts fehlt und nichts doppelt ist.
- Zusätzliche Arbeit, die beim Umsetzen entsteht (Nacharbeiten, Spec-Korrekturen, Bugs, Folge-Aufgaben), wird ebenfalls als Issue im passenden Milestone angelegt – nicht nur im PR-Text erwähnt.
- Checklisten in den Issues werden beim Umsetzen abgehakt, Issues per PR (`Closes #…`) geschlossen; der Milestone-Fortschritt spiegelt so den echten Stand.
- Diese Vorgabe wird in jedes folgende Planungs-Issue („Mx-Abschluss & Planung Mx+1“) wörtlich übernommen.
"""

PLAN = dict(key="WP-31", title="M1-Abschluss & Planung M2", labels=["type:planning"], deps=["WP-20", "WP-27", "WP-28", "WP-29", "WP-30"], features=[],
      goal="M1 formal abschließen und die Arbeitspakete für M2 schneiden (rollierende Planung).",
      notes=[
        "Prüfen: alle M1-Issues geschlossen, Demo-Szenario (Milestone-Beschreibung) erfüllt – E2E aus WP-27 grün, manuelle Verifikation mit echten `claude`- und `codex`-CLIs protokolliert –, `cargo xtask spec-coverage --milestone M1` ohne fehlende Must-ACs, `docs/spec/roadmap.md` (`python3 scripts/gen_roadmap.py`) und der Feature-Katalog (`python3 scripts/gen_feature_catalog.py`) aktuell.",
        "Erkenntnisse aus M1 in Spec/ADRs zurückführen (z. B. verifizierte Annahmen zu Codex `app-server`, ACP und den Import-Formaten; neue ADRs statt stiller Änderungen).",
        "M2-Features (`docs/spec/roadmap.md` → M2) in Arbeitspakete mit 3–8 Features schneiden – jedes M2-Feature genau einem Paket zugeordnet – und mit `scripts/create_milestone_issues.py` (MS und Paketdefinitionen anpassen) als Issues im Milestone `M2 — Kontrolle` anlegen: gleiche Struktur wie die M1-Issues, Labels, „blocked by“-Beziehungen, plus ein Issue „M2-Abschluss & Planung M3“. Pakete in `beton-policy`, `beton-sandbox`, `beton-proxy` und `beton-secrets` bekommen `security-review` und werden strikt per TDD umgesetzt.",
        "Übernahmen in die M2-Pakete: die „(ab M2)“-ACs der M1-Features (z. B. HAR-011 AC2, HAR-027 AC2, SES-007 AC3, SES-015 AC2, AGT-011 AC2), die M2-Übernahmen aus #17 (PROTO-010 AC3 an `PUT /v1/policies/…`, HAR-005 `PreToolUse`-Hook mit `tool_call_gate: full`) und alle Übernahmen, die in den M1-Issues entstanden sind.",
        "Stand der `human-required`-Issues zu Signing-Accounts, Domain und Markenrecht prüfen (fällig vor M3).",
      ],
      extra=MAINTAINER_RULE)

ALL = HUMAN + WPS + [PLAN]

def body(wp, num_of):
    out = []
    if wp["key"].startswith("WP-"):
        out.append(f"> **Arbeitspaket {wp['key']}** · Meilenstein {MS} · Quelle der Wahrheit ist die Spec – dieses Issue verweist nur darauf.\n")
    out.append("## Ziel\n" + wp["goal"] + "\n")
    if wp["features"]:
        out.append("## Features (Spec)\nUmgesetzt ist ein Feature erst, wenn **alle** seine Akzeptanzkriterien durch Tests abgedeckt sind.\n")
        for fid in wp["features"]:
            out.append(f"- [ ] {flink(fid)}")
        out.append("")
    if wp["notes"]:
        out.append("## Hinweise\n" + "\n".join(f"- {n}" for n in wp["notes"]) + "\n")
    if wp.get("extra"):
        out.append(wp["extra"])
    if wp["deps"]:
        out.append("## Abhängigkeiten\nStart erst, wenn diese Issues geschlossen sind:\n" + "\n".join(f"- #{num_of[d]}" for d in wp["deps"]) + "\n")
    if wp["key"].startswith("WP-") and wp["features"]:
        out.append("""## Definition of Done
- [ ] Alle Akzeptanzkriterien der oben gelisteten Features sind durch Tests abgedeckt (Testnamen bzw. -kommentare nennen die Feature-ID).
- [ ] CI ist grün (Gates laut QA-010, sobald vorhanden).
- [ ] Commits nach Conventional Commits mit Feature-ID und DCO (`git commit -s`), PR enthält `Closes #<dieses Issue>`.
- [ ] Generierte Artefakte (Schemas, TS-Typen, OpenAPI) sind aktualisiert, falls betroffen.
- [ ] Widerspricht die Umsetzung der Spec oder einem ADR: Spec-Änderung bzw. neues ADR im selben PR, im PR-Text hervorgehoben.
- [ ] Nichts aus späteren Meilensteinen „auf Vorrat“ gebaut (AGENTS.md §1).""")
        if "security-review" in wp["labels"]:
            out.append("- [ ] **Security-Review:** Merge erst nach expliziter Freigabe durch den Maintainer.")
        out.append("")
    out.append("---\nKontext: [AGENTS.md](%s/AGENTS.md) · [Überblick](%s/docs/spec/00-overview.md) · [Roadmap](%s/docs/spec/roadmap.md)" % (BLOB, BLOB, BLOB))
    return "\n".join(out)

def gh(args, inp=None):
    if DRY:
        print("DRY gh", " ".join(args)[:160]); return "{}"
    r = subprocess.run(["gh"] + args, input=inp, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"gh {' '.join(args)} failed: {r.stderr}")
    return r.stdout

# ---------- Coverage-Check ----------
ms_feats = {k for k, v in feats.items() if v.get("ms") == MS}
assigned = [f for wp in ALL for f in wp["features"]]
dups = {f for f in assigned if assigned.count(f) > 1}
missing = ms_feats - set(assigned)
wrong = set(assigned) - ms_feats
print(f"{MS} features: {len(ms_feats)} · assigned: {len(assigned)} · missing: {sorted(missing)} · dup: {sorted(dups)} · not-{MS}: {sorted(wrong)}")
keys = [wp["key"] for wp in ALL]
seen, bad_deps = set(), []
for wp in HUMAN + WPS + [PLAN]:  # Anlegereihenfolge: deps müssen vorher angelegt sein
    bad_deps += [(wp["key"], d) for d in wp["deps"] if d not in seen]
    seen.add(wp["key"])
bad_size = [(wp["key"], len(wp["features"])) for wp in WPS if not 3 <= len(wp["features"]) <= 8]
print(f"packages: {len(WPS)} · human-required: {len(HUMAN)} · bad deps: {bad_deps} · bad size: {bad_size}")
if missing or dups or wrong or bad_deps or bad_size or len(set(keys)) != len(keys):
    raise SystemExit("coverage check failed")

if "--check" in sys.argv:
    sys.exit(0)

# ---------- Labels ----------
for name, (color, desc) in LABELS.items():
    gh(["label", "create", name, "--repo", REPO, "--color", color, "--description", desc, "--force"])

# ---------- Milestones ----------
existing = {} if DRY else {m["title"]: m["number"] for m in json.loads(gh(["api", f"repos/{REPO}/milestones?state=all&per_page=100"]))}
for title, demo in MILESTONES:
    if title in existing:
        continue
    desc = f"Demo-Szenario (Exit-Kriterium): {demo}\n\nFeature-Liste: docs/spec/roadmap.md"
    gh(["api", f"repos/{REPO}/milestones", "-f", f"title={title}", "-f", f"description={desc}"])

# ---------- Issues (topologische Reihenfolge) ----------
order = HUMAN + WPS + [PLAN]
num_of, id_of = {}, {}
existing_titles = {} if DRY else {i["title"]: i["number"] for i in json.loads(
    gh(["issue", "list", "--repo", REPO, "--state", "all", "--limit", "1000", "--json", "title,number"]))}
for wp in order:
    ms = wp.get("ms", MS)
    title = f"{ms} · {wp['key']} · {wp['title']}" if wp["key"].startswith("WP-") else f"{ms} · {wp['title']}"
    if title in existing_titles:
        print("skip (exists):", title)
        num_of[wp["key"]] = existing_titles[title]
        id_of[wp["key"]] = json.loads(gh(["api", f"repos/{REPO}/issues/{existing_titles[title]}"]))["id"] if not DRY else 0
        continue
    labels = list(wp["labels"])
    if wp["key"].startswith("WP-") and "type:planning" not in labels:
        labels.insert(0, "type:work-package")
    ms_title = next(t for t, _ in MILESTONES if t.startswith(ms + " "))
    args = ["issue", "create", "--repo", REPO, "--title", title, "--milestone", ms_title, "--body-file", "-"]
    for l in labels:
        args += ["--label", l]
    if "human-required" in labels:
        args += ["--assignee", "ifahrentholz"]
    url = gh(args, inp=body(wp, num_of)).strip()
    if DRY:
        num_of[wp["key"]] = 0; continue
    n = int(url.rstrip("/").split("/")[-1])
    num_of[wp["key"]] = n
    id_of[wp["key"]] = json.loads(gh(["api", f"repos/{REPO}/issues/{n}"]))["id"]
    print(wp["key"], "->", url)

# ---------- blocked-by-Beziehungen ----------
dep_fail = []
for wp in order:
    for d in wp["deps"]:
        if DRY:
            continue
        r = subprocess.run(["gh", "api", "-X", "POST", f"repos/{REPO}/issues/{num_of[wp['key']]}/dependencies/blocked_by",
                            "-F", f"issue_id={id_of[d]}"], capture_output=True, text=True)
        if r.returncode != 0 and "already" not in r.stderr.lower():
            dep_fail.append((wp["key"], d, r.stderr.strip()[:200]))
print("dependency failures:", dep_fail)
print("issues:", num_of)
