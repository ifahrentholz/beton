#!/usr/bin/env python3
"""Legt Milestones, Labels und die Arbeitspaket-Issues eines Meilensteins auf GitHub an.

Rollierende Planung (siehe Issue #17 und AGENTS.md): Pro Meilenstein werden die
Arbeitspakete (WPS), ggf. Sonder-Issues für den Maintainer (HUMAN, Label `human-required`)
und das Abschluss-/Planungs-Issue (PLAN) unten definiert. Für den nächsten Meilenstein `MS`
umstellen und WPS/HUMAN/PLAN ersetzen. Die Definitionen früherer Meilensteine (M0: WP-01 bis
WP-16 und LEGAL; M1: WP-17 bis WP-31 und HR-SIGNING/HR-DOMAIN/HR-TRADEMARK; M2: WP-32 bis WP-50
und HR-CODEOWNERS) bleiben als Vorlage in der Git-History.

Das Skript prüft vorab, dass jedes Feature des Meilensteins (laut docs/spec) genau
einem Paket zugeordnet ist. Abhängigkeiten sind Schlüssel anderer Pakete oder (als ganze Zahl)
Nummern bereits existierender Issues aus EXTERNAL, z. B. der `human-required`-Issues. Bereits existierende Issues (gleicher Titel) werden
übersprungen, Labels und Milestones idempotent angelegt.

Voraussetzung: `gh` ist angemeldet und hat Schreibrechte auf REPO.

Aufruf:
  python3 scripts/create_milestone_issues.py --check     # nur Zuordnung prüfen
  python3 scripts/create_milestone_issues.py --dry-run   # zeigt die gh-Aufrufe und Issue-Texte
  python3 scripts/create_milestone_issues.py             # legt alles an
"""
import json, re, glob, subprocess, sys, os

REPO = "ifahrentholz/beton"
ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
MS = "M3"  # Meilenstein, für den Issues angelegt werden
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
    "area:desktop": ("c5def5", "apps/desktop, Tauri-Hülle"),
    "area:browser": ("c5def5", "beton-browser, eingebetteter Browser"),
    "area:voice": ("c5def5", "beton-voice, Spracheingabe"),
}

# ---------- Arbeitspakete M3 ----------
# Die Definitionen für M0 (WP-01 … WP-16, LEGAL), M1 (WP-17 … WP-31, HR-SIGNING, HR-DOMAIN, HR-TRADEMARK) und
# M2 (WP-32 … WP-50, HR-CODEOWNERS) stehen in der Git-History dieses Skripts.
# Gemeinsame Hinweise (ADR-0032 Design-first, ADR-0033 lokal, ADR-0034 Subscription-first, Sicherheit) stehen pro Paket,
# damit jedes Issue für sich verständlich ist. Die Zuordnung der aus M2 übernommenen und der schon in M3 offenen Issues
# steht in docs/plan/m2-triage.md.
def design(screens, extra=""):
    """Hinweis auf die Prototyp-Screens (ADR-0032); `screens` sind Screen-IDs aus design/prototype/src/screens."""
    s = ", ".join(f"`{x}`" for x in screens.split())
    return (f"**Design-first (ADR-0032):** UI nur nach dem abgenommenen Prototyp (`design/prototype`), Screens {s}{extra}. "
            "Abweichungen im PR begründen. Hat ein Feature einen Screen, aber kein UI-AC, wird die Ansicht nicht stillschweigend "
            f"mitgebaut, sondern als eigenes Issue im Milestone {MS} angelegt.")
SUBSCRIPTION = "**Subscription-first (ADR-0034, ADR-0005):** Default-Auth ist der Login der Vendor-CLI; beton liest, speichert oder verwendet keine Subscription-Tokens. Kein Feature setzt einen API-Key voraus; Tests laufen ohne `*_API_KEY` im Env."
LOCAL = "**Lokal ohne externe Server (ADR-0033):** Tests nur mit Fake-CLIs, Golden Transcripts und Mocks auf Loopback; kein Internet."
LOCAL_NET = ("**Lokal ohne externe Server (ADR-0033):** Netz-Tests nur gegen lokale Test-Upstreams (TLS mit Test-CA) und einen Test-Resolver, "
             "nie gegen `example.com`, `github.com` oder öffentliche Registries; die Linux-CI hat kein Netzwerk. Weil der Proxy Loopback und "
             "private Ziele sperrt (PRX-007), brauchen Tests einen ausdrücklichen Test-Zugang, der in einer Produktionskonfiguration nicht erreichbar ist.")
SECURITY = ("**Sicherheitskritisch (AGENTS.md §4, QA-013):** strikt TDD – erst der rote Test, dann die Implementierung. Fail closed: "
            "Fehler oder Ausfall von Engine, Sandbox, Proxy oder Secret-Store führen zur Ablehnung, nie zu stillschweigendem Erlauben. "
            "Merge erst nach Maintainer-Freigabe (ADR-0036; zur Praxis der delegierten Agent-Reviews ADR-0040).")
RELEASE_SECURITY = ("**Release-, Signing- und Update-Pipeline (AGENTS.md §5):** strikt TDD, fail closed (eine fehlende oder ungültige Signatur bricht ab, nie "
                    "„ohne Signatur weiter“). Merge nur nach ausdrücklicher Zustimmung des Maintainers selbst; diese Freigabe wird nicht an Agent-Reviews "
                    "delegiert (ADR-0040, Vorschlag). Schlüssel, Zertifikate und Tokens nie ins Repo, nur als Secrets der Environment `release`.")
ESCAPE = "Sandbox-Escape-Tests (SBX-011, QA-005) werden nie abgeschwächt, übersprungen oder auf `#[ignore]` gesetzt, damit die CI grün wird."
FILTER = ("**Sicherheitsfilter (ADR-0040, Vorschlag):** Hält der Sicherheitsfilter des Modells den Agent beim Ausformulieren von Ausbruchs- oder "
          "Umgehungsfällen an, stoppt er diese Teilaufgabe, formuliert nicht um und übergibt per Issue-Kommentar an den Maintainer.")
PLATFORMS = ("**Plattformen:** macOS (arm64) und Linux sind Pflicht und laufen in der CI. Windows (Beta) bekommt die Sandbox erst mit M5 (SBX-016): "
             "Der Windows-Build bleibt grün, die Backends melden dort `unavailable` (fail closed), es gibt keinen stillen Fallback.")
NEW_CI = ("**Neue CI-Jobs:** Jeder Job in `.github/workflows/ci.yml` muss Required Check auf `main` sein (`scripts/check_branch_protection.py` im Job "
          "„spec + roadmap“); das Eintragen ist eine Einstellung des Maintainers. Neue Jobs (z. B. Windows-Tests, `tauri-driver`) deshalb als "
          "Vorschlag in einem `human-required`-Issue beschreiben (Vorlage #301) und erst nach der Freigabe einchecken.")
RELEASE_GATE = ("**Kein Release 0.1 vor dem M2-Abschluss:** Veröffentlicht wird erst, wenn M2 abgeschlossen ist (Escape-Suite SBX-011/QA-005 als "
                "Required Check, die in M2 verbliebenen `security-review`-Befunde, `docs/plan/m2-triage.md`) und #177 erledigt ist (ADR-0036). "
                "Das Veröffentlichen ist unumkehrbar und bleibt beim Maintainer (AGENTS.md §5).")

WPS = [
 dict(key="WP-51", title="Binärkanäle, `beton-pty` & Workspace-Terminals in der Sandbox", labels=["area:protocol", "area:runner", "area:web", "area:security", "security-review"], deps=[],
      features=["PROTO-007", "DATA-011", "SES-019", "WEB-010"],
      goal="Terminal-Bytes, Browser-Frames und Audio laufen als Binärkanäle mit kreditbasierter Flusskontrolle im selben WebSocket. Workspace-Terminals laufen als PTY im Runner (`beton-pty`, ohne tmux) in Stufe 2 der Sandbox, überleben Client-Disconnects und erscheinen in der Web-UI als xterm.js-Tabs. Terminal-Bytes werden nie gespeichert, nur Snapshots.",
      notes=[
        "Aus #74 (PROTO-007): Binärframes im selben Socket statt Close `4400`. Frame-Layout, `open_channel`/`close_channel`, Kredit und `nack` als Rust-Typen in `beton-proto`, TS-Typen per `cargo xtask codegen`.",
        "Neues Crate `beton-pty` (auf `portable-pty`, Unix-PTY bzw. ConPTY) im Runner. Es ist die Grundlage für die Panes der TUI (TUI-002, WP-52) und den Native-TUI-Modus (WP-53). Nie tmux.",
        SECURITY + " " + ESCAPE,
        "SES-019 AC3 nutzt den Mechanismus aus SBX-017: Das PTY hält `beton-pty` in Stufe 0, der Kindprozess startet über `SandboxProvider::spawn` in Stufe 2 (Default-Sandbox, Proxy, Masken, `cargo xtask spawn-lint`). Den Terminal-Teil baut dieses Paket; SBX-017 als Feature (Vendor-TUI in Stufe 1, AC1) schließt WP-53 ab. Aus #202: je Start genau ein `sandbox.started {stage: tools}` im Session-Log; für Starts außerhalb einer Session festlegen, ob sie ins Audit-Log gehen.",
        "Aus #241: Werkzeuge außerhalb der Systempfade (Homebrew, nvm, rustup, `/usr/bin/git` mit Xcode.app als Developer-Verzeichnis) laufen in Stufe 2 heute nicht; ein `npm run dev` im Terminal (SES-019 AC1) braucht sie. Entscheiden und umsetzen (Preset `dev`, aus dem `PATH` des Daemons abgeleitete Installationsverzeichnisse ohne Home-Dotfiles oder Vorschlag in `beton sandbox explain`), Spec SBX-012 anpassen, Tests unter macOS und Linux.",
        "Aus #326 (ACP `terminal/*` und `fs/read_text_file`): Vom Agent gestartete Terminals laufen über denselben Weg in Stufe 2 und erscheinen als Terminal (SES-019); danach deklariert ACP `tool_isolation: beton` (ADR-0037). Lesen mit Masken.",
        "Aus #93: User-Schreibzugriffe während eines Turns (Workspace-API und User-Terminals) werden nicht dem Agent zugeschrieben (genau ein `fs.changed` mit `actor: user`).",
        "Aus #72: Der Runner reagiert auf `problem` nach gescheitertem Push mit Retry und Backoff und meldet das als `runner.status` (PROTO-015); `config set/unset` erhält Kommentare und Reihenfolge der YAML-Datei (CLI-008); Idempotency-Replay mit `Location` bei 201.",
        "PROTO-007 AC3 (Browser-Frames) und DATA-011 AC3 (Audio) hier mit synthetischen Kanälen nachweisen; WP-56 und WP-60 stellen die Tests auf die echten Quellen um.",
        "**Klären:** SES-019 AC2 (`comment_approve`), WEB-010 AC3 (Nur-Lese-Rollen) und PROTO-007 AC2 (`view`) setzen Rollen voraus (COL-001, M4). Lokal gibt es nur den Owner: die Autorisierung je Kanalart als Schnittstelle mit Owner-Default testen, den Rollen-Teil nicht vorziehen und die Spec im PR klarstellen (wie in M2 bei SEC-012 und DATA-012).",
        "WEB-010 AC1 (≤ 30 ms p95) als Test mit Schwelle in der CI, nach dem Muster der bestehenden Benchmarks.",
        LOCAL,
        PLATFORMS,
        design("workspace-terminals diag-connection diag-storage"),
      ]),
 dict(key="WP-52", title="ratatui-TUI: Session-Ansicht, Panes, Approvals & Windows", labels=["area:cli"], deps=["WP-51"],
      features=["TUI-001", "TUI-002", "TUI-004", "TUI-005"],
      goal="`beton tui` zeigt Session-Liste, Chat-Stream, Rail und Composer wie die Web-UI, teilt Terminal-Panes ohne tmux, entscheidet Approvals per Taste und läuft auch in Windows Terminal und Konsole.",
      notes=[
        "Nutzt `beton-sdk` und dieselben Events wie die Web-UI (TUI-001 AC1); keine eigene Policy- oder Kostenlogik in der TUI.",
        "TUI-002 baut auf `beton-pty` aus WP-51 auf und ergänzt VT-Parser, Panes (teilen, fokussieren, zoomen), Prefix `Ctrl+G` und die Wiederherstellung nach einem Neustart der TUI (AC2).",
        "Aus #265 (USE-008 AC3, „ab M3“): Kontextfüllstand in der Statuszeile mit derselben Logik wie `apps/web/src/lib/context.ts` (Stufen 70/85/95 %, ohne Fenster nur die Tokenzahl), Test `use_008_ac3_…`.",
        "Aus #81: Event `plan.updated` für ACP-`plan` und Codex `turn/plan/updated` in PROTO-002 aufnehmen (Codegen) und in TUI und Web-UI anzeigen; vor dem ersten Release, damit es Teil des ersten Schema-Stands ist (PROTO-014, WP-67).",
        "TUI-004: Approvals mit `y`/`n`/`s`/`d` über die Approval-API aus M2 (POL-010); Approvals anderer Sessions als Hinweis in der Statuszeile. Das Banner im Native-TUI-Pane kommt mit WP-53.",
        "TUI-001 AC2 (500 Tokens/s, 30 fps, Diff-Rendering) als Test mit gedrosseltem Fake-Harness; Layouts als Snapshot-Tests gegen ein Test-Backend von ratatui.",
        "TUI-005: crossterm und ConPTY. Für Tests unter Windows ist ein neuer CI-Job nötig. " + NEW_CI,
        LOCAL,
        design("tui-session tui-panes tui-windows", " (Gruppe `tui`)"),
      ]),
 dict(key="WP-53", title="Native-TUI-Modus: Vendor-TUI im PTY mit Hooks, Approvals & Sandbox", labels=["area:harness", "area:security", "security-review", "needs-vendor-cli"], deps=["WP-51", "WP-52"],
      features=["HAR-012", "HAR-013", "HAR-014", "POL-022", "SBX-017", "TUI-003"],
      goal="Die Original-TUIs von Claude Code und Codex laufen im PTY von beton in Stufe 1 der Sandbox und werden an alle Clients gespiegelt. Policies greifen über Vendor-Hooks bzw. die Approval-Integration von Codex, `ask` wird zur beton-Approval-Card, und Nachrichten erscheinen als strukturierte Events.",
      notes=[
        SECURITY + " " + ESCAPE,
        "`beton run claude --mode tui` (CLI-005) startet den Modus. Das Hook-Kommando `beton hook claude` und den session-spezifischen `PreToolUse`-Hook über `--settings` gibt es seit M2 (HAR-005, WP-35); HAR-013 ergänzt `UserPromptSubmit` und `PostToolUse`. Die globale `~/.claude/settings.json` wird nie verändert; `--setting-sources user` wie im strukturierten Modus (#233).",
        "Aus #197 (POL-022): Claude 2.1.296 ersetzt mit `PostToolUse` das Ergebnis nicht (`tool_result_gate: false`, `observe_only`). Neuere CLI-Versionen prüfen; sonst bleibt es bei `observe_only`, und die Enforcement-Matrix (POL-024) meldet das.",
        "HAR-014: Der Codex-Adapter wählt per `probe()` die stärkste verfügbare Strategie und deklariert sie. Aus #239 (ADR-0037): Codex startet Kommandos über die Login-Shell; Stufe 2 für Codex (`shell_zsh_fork`, `exec-server`, Bind-Mount über die Login-Shell unter Linux) mit der aktuellen Codex-Version neu prüfen. Ohne Lösung bleibt Codex nur mit `sandbox.require_tool_isolation: false` und *degraded* nutzbar.",
        "SBX-017: Der Harness am PTY-Slave startet über `SandboxProvider::spawn` in Stufe 1, Shell-Kommandos der Vendor-TUI gehen über `bt-exec` in Stufe 2 (Claude über `CLAUDE_CODE_SHELL`, ADR-0037). Den Terminal-Teil (AC2) liefert WP-51.",
        "Aus #212 und #240: Die Suche nach Muster-Masken unter Linux kostet bei großen Workspaces und breiten Lese-Grants bei jedem Stufe-2-Start Zeit (in der CI bis zur 30-s-Frist). Startzeit messen (Benchmark), Suche begrenzen oder cachen, bei Überschreitung Hinweis statt Zeitüberschreitung.",
        "HAR-012 AC3: Transcript-Tailing der Vendor-Dateien mit den Import-Parsern aus M1 (HAR-023, HAR-024); Credential-Dateien liest beton nie (ADR-0005). HAR-012 AC4 verlangt einen Fake-TUI-Test im PTY auch unter Windows (neuer CI-Job, siehe unten).",
        "Hooks im TUI-Modus und die Codex-Strategie gegen die echten CLIs verifizieren und Golden Transcripts aufnehmen (`cargo xtask golden-scan`); die CI nutzt Fake-TUI und Fake-CLI. " + SUBSCRIPTION,
        "Zugehörige Issues (Design-first, eigene Issues): Durchsetzung je Harness und Policy-Audit (#194, Screens `policy-enforcement`, `harness-catalog`) und Stufen der Sandbox (#238, Screen `sandbox-stages`); der TUI-Modus muss dort sichtbar werden.",
        NEW_CI,
        PLATFORMS,
        design("tui-native harness-catalog harness-setup-cli policy-enforcement sandbox-stages"),
      ]),
 dict(key="WP-54", title="Eingebetteter Browser: Discovery, Chrome for Testing, CDP-Lebenszyklus & Profile", labels=["area:browser", "area:security", "security-review"], deps=[],
      features=["BRW-001", "BRW-002", "BRW-003", "BRW-004", "BRW-008"],
      goal="beton findet einen installierten Chromium-basierten Browser oder lädt auf Klick eine gepinnte Chrome for Testing, startet ihn lazy pro Session über CDP mit isoliertem Profil, erkennt Abstürze und wechselt zwischen eingebettetem und sichtbarem Fenster.",
      notes=[
        "Neues Crate `beton-browser` (`chromiumoxide`), läuft im Runner (ADR-0016). Chromium startet über `SandboxProvider::spawn` (SBX-001 AC2, `cargo xtask spawn-lint`); das Seatbelt- bzw. Landlock-Profil für den Browser mit eigenem Schreib-Root für das Profil entsteht hier.",
        SECURITY,
        "BRW-002 (ADR-0033): Download nur auf Klick, von der gepinnten URL, Prüfung per SHA-256 gegen die im Binary mitgelieferten Prüfsummen; fehlt die Prüfsumme oder weicht sie ab, wird nichts entpackt (fail closed). Import einer lokalen Datei als Alternative.",
        "BRW-004: Profil `~/.beton/browser/profiles/<session_id>/`, nie das Alltagsprofil; Löschen der Session löscht das Profil. Dass Forks mit leerem Profil starten, ist eine *Annahme* der Spec: im PR bestätigen oder korrigieren.",
        "BRW-001: Ergebnis in `beton doctor` (OBS-005). Tests mit Fake-Browser-Binaries bzw. einem lokal vorhandenen Headless-Chromium, ohne Download in der CI.",
        "Der Netzweg über den Proxy (PRX-011, BRW-018) kommt mit WP-55. Bis dahin hat der Browser kein Netz außer lokalen Testseiten über einen ausdrücklichen Test-Zugang (fail closed, kein Direktzugang).",
        LOCAL_NET,
        PLATFORMS,
        design("browser-setup browser-panel browser-window"),
      ]),
 dict(key="WP-55", title="Browser-Traffic über den Proxy, Browser-Policies & Dev-Server-Erkennung", labels=["area:browser", "area:security", "security-review"], deps=["WP-51", "WP-54"],
      features=["PRX-011", "BRW-018", "BRW-019", "POL-020", "BRW-017"],
      goal="Sämtlicher Browser-Traffic läuft über eine eigene Proxy-Identität mit Regeln und Credential-Injection, Chromium vertraut der beton-CA nur in dieser Instanz, Navigation, Formular-Submit und Downloads sind Policy-Hook-Punkte mit dem Regeltyp `browser_guard`, und lokale Dev-Server der Session werden erkannt.",
      notes=[
        SECURITY,
        "PRX-011 ist Owner von Proxy-Identität, Start-Flags (`--proxy-server`, `--proxy-bypass-list=<-loopback>`, SPKI-Pinning des Instanz-Leafs) und Zertifikatsvertrauen; die beton-CA kommt nie in einen System-Trust-Store (PRX-002). BRW-018 weist das Ergebnis nach, auch für WebSockets und Service Worker.",
        "BRW-019 und POL-020: Phasen `browser_navigate` und `browser_action` (POL-003) mit den Variablen aus POL-004, die es seit M2 nur als Typen gibt. Aus WP-35 (POL-003 AC1): Die Integrationstests je Phase für die beiden Browser-Phasen kommen hier dazu.",
        "`browser_guard` wird zu CEL entzuckert (sichtbar in `beton policy explain`) und bekommt eine mitgelieferte `*.policytest.yaml` (POL-026 AC4, QA-004).",
        "BRW-017: Loopback-Ausnahmen nur für erkannte Ports der eigenen Session und nur in der Browser-Proxy-Identität; sie lockern weder Stufe 1 noch Stufe 2 und enden mit der Session (PRX-007).",
        "Zugehörige Issues (Design-first, eigene Issues): Policy-Übersicht und Ladefehler (#183, Screens `policy-overview`, `policy-editor`) und Proxy-Ansicht mit Egress-Log (#190, Screens `egress-proxy`, `egress-log`); `browser_guard` und die Browser-Identität des Proxys sollen dort sichtbar werden.",
        LOCAL_NET,
        design("browser-policies browser-panel egress-proxy egress-log policy-editor policy-overview"),
      ]),
 dict(key="WP-56", title="Browser-Panel: Screencast, Input-Weiterleitung, Tabs & Co-Viewer", labels=["area:browser", "area:web"], deps=["WP-51", "WP-54"],
      features=["BRW-005", "BRW-006", "BRW-007", "BRW-009", "BRW-020"],
      goal="Der aktive Tab wird als Screencast über den Binärkanal an alle betrachtenden Clients gestreamt, nimmt Maus, Tastatur, IME und Paste aus dem Panel entgegen, passt Qualität und DPR an und zeigt Tabs, Adressleiste, Modus-Umschalter und „Agent steuert“.",
      notes=[
        "Baut auf den Binärkanälen aus WP-51 (höchstens 2 ausstehende Frames, Drop statt Stau) und auf `beton-browser` aus WP-54 auf; PROTO-007 AC3 hier auf echte Browser-Frames umstellen.",
        "**Klären:** BRW-006 und BRW-020 nennen Rollen (`drive`, Viewer ohne Workspace-Freigabe; COL-001, M4). Lokal steuert der Owner, Co-Viewer sind in M3 weitere Fenster und Geräte desselben Users. Rollen-Teil nicht vorziehen, Spec im PR klarstellen.",
        "Aus #104 (Design der Workspace-Rail): Das Browser-Panel sitzt in der Rail (WEB-008); Ein-/Ausblenden und Breite mit diesem Design-Issue abstimmen.",
        "Performance-Budgets der Web-UI (WEB-015) einhalten; nichts aus einem CDN (ADR-0033). E2E mit Playwright gegen eine lokale Testseite und den Fake-Harness.",
        design("browser-panel browser-inspect"),
      ]),
 dict(key="WP-57", title="Browser-Agent-Tools, Accessibility-Snapshot & Inspect-Mode", labels=["area:browser", "area:security", "security-review"], deps=["WP-55", "WP-56"],
      features=["BRW-010", "BRW-011", "BRW-012", "BRW-013", "BRW-014", "BRW-015", "BRW-016"],
      goal="Jeder Harness steuert den Browser über System-Tools von `beton-mcp` mit stabilen refs aus dem Accessibility-Snapshot; im Inspect-Mode wählt der User Elemente im Panel, und der Agent bekommt Selektor, HTML-Ausschnitt, Styles, Screenshot-Crop und Kommentar.",
      notes=[
        SECURITY,
        "BRW-010: Browser-Tools durchlaufen `tool_call` (Klasse `browser`, Abbildung in POL-005) und die Browser-Phasen aus WP-55; je Phase genau ein `policy.decision` (POL-025).",
        "BRW-012: `browser_eval` ist standardmäßig nicht registriert, wird per Agent-Config aktiviert und ist dann je Aufruf mit Default `ask` geschützt. Ergebnis- und Fehlertexte laufen durch die Redaction (SEC-013).",
        "BRW-014: Payload laut Design; der Crop geht als Bild an Harnesses mit `images` (HAR-002), sonst Text mit Hinweis. Blobs über DATA-006.",
        "BRW-016 (Could) ist Stretch und blockiert das Paket nicht; `confidence` ehrlich melden.",
        "Golden Transcripts für einen Browser-Tool-Call über Claude und Codex aufnehmen, wenn eine eingeloggte CLI verfügbar ist; sonst als Folge-Issue mit `needs-vendor-cli` (wie #97). Die CI nutzt den Fake-Harness.",
        LOCAL,
        design("browser-inspect browser-panel browser-policies browser-setup"),
      ]),
 dict(key="WP-58", title="Desktop-App: Shell, Daemon-Sidecar, Tray, Benachrichtigungen & Deep-Links", labels=["area:desktop"], deps=[],
      features=["DESK-001", "DESK-002", "DESK-004", "DESK-005", "DESK-006", "UX-006"],
      goal="Tauri-2-App für macOS und Linux mit mehreren Fenstern und nativem Menü, die den lokalen Daemon als Sidecar startet oder einen laufenden nutzt, mit Tray und Dock-Badge, Benachrichtigungen nur für nicht betrachtete Sessions, Deep-Links `beton://` und einer Einstellungsseite für Benachrichtigungen.",
      notes=[
        "`apps/desktop` (Tauri 2, ADR-0004) lädt die gebündelte Web-UI; eigene Logik nur für Fenster, Tray, Menü und Benachrichtigungen. Windows bleibt Beta (Build grün, Bundles erst mit M5).",
        "DESK-002: Sidecar ist dasselbe `beton`-Binary; ein bereits laufender kompatibler Daemon wird genutzt (Versionsfenster DIST-018, WP-67). Das lokale Token erscheint nie in Logs oder URLs.",
        "Aus #217 (UX-001 AC5, „ab M3“): Inbox-Zähler als Dock- bzw. Tray-Badge, folgt Änderungen binnen 1 s (Test `ux_001_ac5_…`).",
        "DESK-006: Deep-Links öffnen nur Ansichten (Session, Kommentar-Anker) und lösen ohne Bestätigung keine Aktion aus; Pairing kommt mit DESK-003 (M4). Single-Instance.",
        "UX-006: Einstellungen pro User serverseitig; das Routing (COL-010) ist M4, Web-Push bleibt aus (ADR-0033).",
        "Zugehörige Issues: Hinweise nach Runner-Neustart (#135) und automatisch abgelehnte Freigaben (#153) bestimmen mit, welche Ereignisse eine Benachrichtigung auslösen (DESK-005).",
        "Die E2E-Tests der Desktop-App mit `tauri-driver` kommen mit QA-008 (WP-67); hier Unit- und Integrationstests.",
        LOCAL,
        design("desktop-shell desktop-daemon desktop-tray desktop-notifications desktop-deeplink settings-notifications inbox-list"),
      ]),
 dict(key="WP-59", title="Lokales Whisper: Engine, Modelle on demand, Beschleunigung & Sprache", labels=["area:voice", "area:core"], deps=[],
      features=["VOI-001", "VOI-002", "VOI-003", "VOI-006", "VOI-008"],
      goal="Neues Crate `beton-voice` transkribiert lokal mit whisper.cpp, lädt Modelle nur auf Anforderung gegen gepinnte Prüfsummen, nutzt Metal bzw. CPU-Beschleunigung, erkennt Deutsch und Englisch und baut außer dem ausdrücklichen Download keine Verbindung auf (ADR-0023).",
      notes=[
        "`whisper-rs` (whisper.cpp): Lizenz und Build in `cargo deny` prüfen; Metal unter macOS, die CUDA-Variante erst mit der Build-Matrix (DIST-001, WP-65).",
        "VOI-002 (ADR-0033): Download nur nach Bestätigung, von gepinnten URLs, Prüfung per SHA-256 (fail closed); Import einer lokalen Datei als Alternative. Tests mit einem kleinen Test-Modell bzw. Fixture, nie mit Download in der CI.",
        "VOI-008: Test, dass `beton-voice` außer dem ausdrücklichen Download keine Verbindung aufbaut (Netz-Mock bzw. Offline-Namespace), und Lint gegen die Web-Speech-API im Frontend.",
        "VOI-003: gewähltes Backend und Real-Time-Factor in `beton doctor`; bei Initialisierungsfehlern Rückfall auf CPU mit Hinweis.",
        "Audio wird nie gespeichert (DATA-011 AC3, WP-51 und WP-60).",
        LOCAL,
        design("desktop-voice desktop-ptt"),
      ]),
 dict(key="WP-60", title="Diktat: Audio-Streaming, Teil-Ergebnisse & Push-to-Talk", labels=["area:voice", "area:desktop", "area:web"], deps=["WP-51", "WP-59", "WP-58"],
      features=["VOI-004", "VOI-005", "VOI-007", "DESK-008"],
      goal="Desktop, Web und PWA nehmen auf, streamen 16-kHz-PCM über einen eigenen Binärkanal, zeigen Teil-Ergebnisse im Composer, und ein globaler Push-to-Talk-Shortcut der Desktop-App diktiert in die zuletzt aktive Session.",
      notes=[
        "Baut auf dem Audio-Kanal aus WP-51, der Engine aus WP-59 und der Desktop-App aus WP-58 auf; DATA-011 AC3 hier auf echte Spracheingaben umstellen.",
        "VOI-007 ist Owner von Verhalten, Modi `hold`/`toggle` und Default-Shortcut, DESK-008 von der Registrierung im Desktop. Kollisionen mit System-Shortcuts melden statt still zu überschreiben.",
        "Aus QA-007 („ab M3: Voice mit Audio-Fixture“): Playwright-Test mit Audio-Fixture über das Fake-Mikrofon von Chromium, ohne echtes Mikrofon.",
        "Mikrofon-Berechtigung im Tauri-Bundle (macOS `NSMicrophoneUsageDescription`); die Signatur kommt mit WP-65.",
        LOCAL,
        design("desktop-ptt desktop-voice session-stream"),
      ]),
 dict(key="WP-61", title="Projects als Gruppierung und Policy-Ebene & MCP-Server-Verwaltung", labels=["area:server", "area:web", "area:security", "security-review"], deps=[],
      features=["SES-013", "SES-014", "UX-010"],
      goal="Projects gruppieren Sessions, liefern Defaults (Repo-Root, Harness, Modell, Effort, Modus, Worktree, Base-Branch) und tragen die Projekt-Ebene der Policies; MCP-Server lassen sich auf User- und Projekt-Ebene in UI und CLI verwalten und testen.",
      notes=[
        SECURITY,
        "Aus #74 (SES-013): Tabelle `projects` und Fremdschlüssel für `sessions.project_id`; die Migration legt vorher ein Backup an (DIST-018 AC3).",
        "Übernahmen „ab M3“: SES-008 AC3 (importierte Sessions dem Project zuordnen, dessen Root ihr `cwd` enthält), SES-015 (Base-Branch aus dem Project-Default) und POL-001 (Projekt-Policies der Project-Entität, SES-014).",
        "SES-014 (ADR-0041): Projekt-Policies verschärfen nur. Die Regeln der Projekt-Ebene aus M2 (`modify.args` erst ab der User-Ebene, eigene Effect-Keys #268, `on_timeout`) gelten auch für im Project gespeicherte Policies. Aus #236: Semantik von `allow_on_timeout: false` über Ebenen hinweg in POL-010 klären.",
        "Aus #323: Das effektive Set am Server (`GET /v1/policies?session_id=`, `beton policy show --session`) zeigt Agent- und Project-Ebene. Aus #324: `beton agent validate` prüft Policies und Sandbox wie der Start (ADR-0041).",
        "UX-010: Konfigurationsformat und Merge-Regeln sind AGT-006, der Transport HAR-009. Secret-Werte nur als SecretRef (SEC-001), nie im Klartext in Datei, Events oder Logs (AC3). Aus #249: Projekt-`.mcp.json` lädt Claude seit `--setting-sources user` nicht mehr; Hinweis beim Start und Übernahme nach `.beton/mcp.yaml` nur als Server-Definition und nach Bestätigung. Aus #96: MCP-Server-Fehler im Verlauf und Schalter „System-Tools bereitstellen“.",
        "Aus #147: Default-Modell je Harness (`harnesses.<id>.model`) für Sessions und Sub-Agents, wenn weder Request noch Agent eines nennen; die Reihenfolge zusammen mit dem Project-Default festlegen und in HAR-017 und SES-013 dokumentieren.",
        "Aus #122: Importierte Sessions nur über einen Fork in einem selbst gewählten Verzeichnis fortsetzen; kein Pfad aus der Importdatei erreicht den Runner.",
        "Zugehörige Issues (Design-first, eigene Issues): Worktree der Session und Rückfrage beim Löschen (#91), Import und Export in der Web-UI (#119, #121), Tab „Policies & Sandbox“ im Agent-Detail (#322).",
        design("workspace-projects workspace-new-session settings-mcp"),
      ]),
 dict(key="WP-62", title="Bedienung: Command-Palette, Session-Switcher, Shortcuts, Themes, Barrierefreiheit & Sprachen", labels=["area:web"], deps=[],
      features=["UX-002", "UX-003", "UX-004", "UX-005", "WEB-016", "WEB-017"],
      goal="Command-Palette mit zentraler Befehls-Registry, Session-Switcher, plattformabhängige Shortcuts mit Overlay, Themes mit passenden Varianten für Monaco, xterm.js und Shiki, Barrierefreiheit der Kernflüsse nach WCAG 2.2 AA und Deutsch/Englisch.",
      notes=[
        "Eine Registry für Befehle und Shortcuts (UX-002, UX-004); Palette, Switcher und Overlay lesen daraus. Bestehende Shortcuts der Approval-Bar (WEB-007) übernehmen.",
        "Aus QA-007 („ab M3“): Playwright-Kernflüsse für Command-Palette und Themes.",
        "WEB-016: axe-Tests für Session öffnen, Nachricht senden, Approval entscheiden und Diff lesen in allen Themes; ARIA-Live-Region gedrosselt.",
        "WEB-017: `i18next` ist eine *Annahme* der Spec; Bundle-Größe gegen WEB-015 prüfen. CLI- und TUI-Ausgaben bleiben Englisch.",
        "Zugehörige Issues (Design-first, jeweils eigenes Issue, nicht Teil der Definition of Done): Sessions-Seite und Sub-Agents in der Liste (#98, #145), Fork-Dialog (#109), Agent starten und Vorab-Prüfung (#125, #142), Agent-Bibliothek, Partitur, Debatte und Sub-Agents im Verlauf (#78, #139, #140, #141), Harness-Befehle im Slash-Menü (#131), Session-Titel (#126, #129), Experimentelle Funktionen und Umgebung (#99), Queue und Split-Diff (#100, #105), Button-Varianten des Prototyps (#86), Verbrauch und Preise (#179, #180, #266, #267). Die Palette ruft deren Befehle auf, sobald es sie gibt.",
        design("inbox-palette inbox-switcher inbox-shortcuts settings-appearance"),
      ]),
 dict(key="WP-63", title="Onboarding-Wizard, CLI-Installationsangebot & Opt-in-Telemetrie", labels=["area:web", "area:cli", "needs-vendor-cli"], deps=["WP-51"],
      features=["UX-008", "DIST-015", "OBS-007", "OBS-008"],
      goal="Beim ersten Start führt ein Wizard durch dieselben Schritte wie `beton setup`: Harness-Erkennung mit Installationsangebot, Login über die CLI selbst, optionale API-Keys, Sandbox- und Proxy-Check, Opt-in für Telemetrie und Crash-Reports. Nichts davon passiert ohne ausdrückliche Zustimmung.",
      notes=[
        SUBSCRIPTION + " Der Login läuft nur über die Vendor-CLI im eingebetteten Terminal (UX-008 AC3).",
        "Gemeinsame Zustandsmaschine im Daemon für `beton setup` und Wizard (`/v1/setup/*` ist eine *Annahme* der Spec). Der Login-Schritt nutzt die Terminals aus WP-51 und markiert den Kanal als `sensitive` (nie in `beton diagnose`, OBS-006 AC4).",
        "DIST-015: gepflegte Installationsbefehle je Plattform; der angezeigte Befehl ist exakt der ausgeführte (AC2), nie still, nie als Root ohne Rückfrage. Tests mit Fake-Installern, nie mit echtem `npm install`.",
        "OBS-007 und OBS-008 (ADR-0025, ADR-0033): standardmäßig aus, keine Vorauswahl, `DO_NOT_TRACK=1` und CI erzwingen „aus“; ohne Opt-in keine Verbindung (Netz-Mock-Test). **Klären:** Wohin Telemetrie und Crash-Reports gehen (Endpunkt, Betreiber), entscheidet der Maintainer; bis dahin nur lokal schreiben bzw. konfigurierbar lassen, kein fest eingebauter externer Server.",
        "Zugehörige Issues (Design-first, eigene Issues) für den Sandbox-Schritt: Sandbox-Einstellungen als Ziel von „Sandbox einrichten“ (#321, #201), Bestätigung von `backend: none` und Proxy-Ausfall (#320), Fortsetzen nach Proxy-Ausfall (#325). Für die Harness-Erkennung: Harness-Katalog und ACP-Registrierung (#82), Provider-Ansicht (#113).",
        "Login-Erkennung und Installationsbefehle gegen die echten CLIs prüfen (Checkliste im PR, Grundlage für QA-019); die CI nutzt Fake-CLIs.",
        LOCAL,
        design("inbox-onboarding cli-setup settings-privacy"),
      ]),
 dict(key="WP-64", title="Diagnose-Bundle, Verstoß-Reporting & PII-Schwärzung", labels=["area:security", "area:cli", "area:web", "security-review"], deps=[],
      features=["OBS-006", "POL-019", "SBX-014"],
      goal="`beton diagnose` erstellt ein secret-freies Bundle, Sandbox-Verstöße erscheinen mit Tool-Call-ID an der verursachenden Tool-Card samt bestätigungspflichtigem Regel-Vorschlag, und `pii_redaction` schwärzt PII in Tool-Results, wo beton den Inhalt kontrolliert.",
      notes=[
        SECURITY,
        "OBS-006 und SEC-013 AC4 („ab M3“): Das Bundle läuft durch die Redaction-Pipeline aus M2; Test mit Fixture-Secrets über alle Dateien des Bundles. Kein automatischer Upload. Aus #214: Entropie-Heuristik der Redaction (abschaltbar, Default aus) ohne Regression des Benchmarks SEC-013 AC3.",
        "SBX-014 (aus M2 verschoben, #299): AC1 braucht eine Zuordnung Request → Tool-Call (Proxy-Token oder Instanz je Tool-Call, Markierung durch den Exec-Broker); vorher ein ADR. AC3: Ratenbegrenzer am Event-Sink der Session. **Klären:** AC2 nennt als Ziel des Vorschlags `.beton/config.yaml`, die Projektebene lehnt `sandbox:` aber ab (ADR-0041); Ziel (User-Konfiguration) im PR festlegen und die Spec anpassen.",
        "POL-019 (aus M2 verschoben, #279): Result-Modifikation im Direkt-Harness (und wo möglich bei ACP `fs/*`, `terminal/*`), Detektoren mit Prüfziffer (Luhn, IBAN) ohne IO in `beton-policy`; ohne Result-Modifikation `notify` mit `enforcement: observe_only`.",
        "Aus #185: Retention des lokalen Audit-Logs mit Anker der Hash-Kette; `beton audit verify` bleibt nach dem Kürzen gültig.",
        "Zugehörige Issues (Design-first, eigene Issues): Event-Log zum Schwärzen (#215), SSE-Befehl im Event-Log (#130), Sandbox-Probe und Verstöße (#213, #300), Explain, Policy-Tests und Shadow (#254), Approval „Agent wiederholt sich“ (#280), Budgets und Modell-Routing (#281, dazu #282 Zeitzone je Regel), Secrets und Audit-Log (#184), Credential-Bindings und Git-Verbindungen (#290).",
        LOCAL,
        design("diag-bundle diag-events secret-redaction sandbox-violation", " (Zustand `suggest`)"),
      ]),
 dict(key="WP-65", title="Release-Pipeline: Build-Matrix, Signing, Desktop-Bundles, GitHub Releases & Subscription-Checkliste", labels=["area:release", "area:qa", "security-review", "needs-vendor-cli"], deps=["WP-58", 54, 56],
      features=["DIST-001", "DIST-002", "DIST-008", "DIST-013", "DIST-017", "DIST-019", "QA-019"],
      goal="Release-Artefakte entstehen nur in der CI für die Build-Matrix: signiert und notarisiert (macOS), mit Sigstore-Bundles, SBOM und SLSA-Provenance, als Desktop-Bundles und GitHub-Release mit Kanälen. release-please pflegt Version und Changelog, und ohne bestandene Subscription-Checkliste bleibt ein Release Draft.",
      notes=[
        RELEASE_SECURITY,
        RELEASE_GATE,
        "Gate aus #54 (Signing-Accounts) und #56 (Markenrecht): Signatur-Identität, Bundle-IDs und Artefaktnamen hängen am Namen. Bis zur Entscheidung nur mit Test-Identitäten arbeiten und nichts veröffentlichen. Environment `release`, Secrets und Required Checks richtet der Maintainer ein (`human-required`-Issue „Release-Infrastruktur einrichten …“).",
        "**Pflichten aus #74 (Review von #313/#304):** (1) Release-Binaries nie mit `--all-targets` oder `--all-features` bauen (z. B. `cargo build --release --locked -p beton-cli`), sonst ist das Cargo-Feature `test-budget` der Policy-Engine im ausgelieferten Binary aktiv. (2) Nach dem Bauen das Binary selbst prüfen: `beton` mit gesetztem `BETON_TEST_POLICY_BUDGET_MS` starten, die Warnung zum Testbudget darf nicht erscheinen; zusätzlich `! grep -qa BETON_TEST_POLICY_BUDGET_MS <artefakt>`. (3) `cargo tree --workspace -e normal,build --target all` im Prüfschritt, damit plattformabhängige Abhängigkeiten aller Ziele mitgeprüft werden.",
        "Aus #74 (QA-002 AC4, „ab M3“): Prüfung, dass `beton-fake-cli` und andere Test-Binaries (z. B. Fuzz- und Probe-Binaries) in keinem Release-Archiv stecken.",
        "DIST-013: Signing-Secrets nur in der Environment `release` (AC4); Windows-Signing ab M5; cosign keyless über GitHub OIDC. DIST-008: deb/rpm liefern das AppArmor-Profil `deploy/apparmor/beton` für `/usr/bin/beton` mit `userns`-Recht (SBX-008).",
        "QA-019: Vorlage unter `docs/release/verifications/`, das Protokoll erstellt der Maintainer mit echten Logins; der CI-Check parst es und hält das Release als Draft (AC2). In die Checkliste gehören die Vendor-Hosts des Passthroughs (ADR-0038), die Tool-Isolation je CLI-Version (ADR-0037) und die offenen Aufnahmen mit echten CLIs: #80 (ACP-Presets), #97 (System-Tools), #108 (Codex-Fork), #111 (Codex-Compaction), #112 (`/compact`), #127 und #128 (Einmal-Modus), #136 (Claude-Modi), #137 (Effort je Modell), #143 (Tool-Timeouts). " + SUBSCRIPTION,
        "Release-Workflows laufen nicht in PRs; die PR-Matrix baut im Check-Modus (DIST-001). " + NEW_CI,
        design("install-ways install-verify install-update"),
      ]),
 dict(key="WP-66", title="Installation & Wartung: Installer, Homebrew, binstall, `upgrade`, `uninstall` & Completion", labels=["area:release", "area:cli", "security-review"], deps=["WP-65", 55],
      features=["DIST-003", "DIST-004", "DIST-005", "DIST-016", "DIST-020", "CLI-013", "CLI-014"],
      goal="Installation per Skript, Homebrew und `cargo binstall`, `beton upgrade` mit Erkennung der Installationsart und Signaturprüfung (auch offline aus einer Datei), `beton uninstall` und Shell-Completion.",
      notes=[
        RELEASE_SECURITY,
        "Gate aus #55 (Domain): Installationshinweise und Doku-Links erst nach der Entscheidung festschreiben. Zur Laufzeit hängt nichts an einer Domain (ADR-0033); Installer-URLs zeigen auf GitHub Releases (DIST-002).",
        "DIST-003 und DIST-016: Download nur bei ausdrücklichem Aufruf, SHA-256 und Signatur vor dem atomaren Austausch, `beton.old` als Rückfall; ein manipuliertes Archiv ändert nichts (DIST-016 AC3, AC5). Tests mit lokal signierten Test-Archiven im Offline-Namespace.",
        "DIST-004: Tap `ifahrentholz/homebrew-tap` (Repository und Token richtet der Maintainer ein, `human-required`-Issue „Release-Infrastruktur einrichten …“); Tests gegen ein lokales Bare-Repo.",
        "DIST-020: `--purge` nur nach Bestätigung, entfernt beton-eigene Keychain-Einträge, nie Vendor-Credentials (ADR-0005).",
        "CLI-014 (Could): Completion mit dynamischen Session-IDs ohne laufenden Daemon read-only aus der lokalen DB (wie USE-007).",
        LOCAL_NET,
        design("install-ways install-verify install-update cli-upgrade cli-completion"),
      ]),
 dict(key="WP-67", title="Desktop-Auto-Update, Versionierung, Schema-Kompatibilität, Desktop-E2E & M3-Demo", labels=["area:release", "area:desktop", "area:qa", "security-review"], deps=["WP-53", "WP-57", "WP-60", "WP-65"],
      features=["DESK-007", "DIST-014", "DIST-018", "PROTO-014", "QA-008", "QA-012"],
      goal="Die Desktop-App aktualisiert sich nur auf Klick oder nach Opt-in über signierte Manifeste (auch offline aus einer Datei), Server, Runner und Clients prüfen ein Kompatibilitätsfenster, die CI vergleicht Schemas mit dem letzten Release, und die Desktop-App wird per `tauri-driver` getestet – einschließlich des M3-Demo-Szenarios im Offline-Namespace.",
      notes=[
        RELEASE_SECURITY,
        "DIST-014 und DESK-007 (ADR-0033): ohne Klick oder `update.auto_check` keine Verbindung (Netz-Mock-Test über 48 h simulierte Laufzeit). Der Ed25519-Schlüssel des Updaters ist vom Code-Signing getrennt; Erzeugung und Ablage übernimmt der Maintainer (`human-required`-Issue „Release-Infrastruktur einrichten …“), Tests nutzen Test-Schlüssel.",
        "DIST-018 und PROTO-014: Versionsfenster ±1 Minor, Schema-Vergleich gegen den letzten Release-Tag. Vor dem ersten Release gibt es keinen Tag: den Check mit Fixture-Schemas testen und mit 0.1 scharf schalten. Aus #74 (PROTO-004 AC3, „ab M3“): Kompatibilitätstests aktueller Server gegen den Client der Vorversion und umgekehrt, sobald es ein erstes Release gibt. DIST-018 AC3: Backup vor jeder Migration, Downgrade mit neuerem Schema wird verweigert.",
        "QA-008: `tauri-driver` unter Linux (WebKitWebDriver); macOS über die Web-Suite (WebKit) plus Smoke-Checkliste. **Klären:** AC1 nennt Windows, die Desktop-App ist dort bis M5 Beta; Spec im PR auf Linux präzisieren oder den Windows-Job begründen.",
        "Aus QA-018 AC5 („ab M3“): Das M3-Demo-Szenario läuft im Offline-Namespace mit Fake-Harness – Desktop (`tauri-driver`, Linux) bzw. Web-UI und `beton tui`, zwei Sessions in Worktrees eines lokalen Repos, Inspect-Mode gegen einen lokalen Dev-Server und Diktat mit vorab importiertem Modell und Audio-Fixture. Deshalb hängt das Paket an WP-53, WP-57 und WP-60.",
        "QA-012 (Should): Benchmarks mit Schwellen, Startzeit-Budgets als Regressionstest. Aus #123, #251 und #292: zeitabhängige Tests (`run_003_ac4_…`, `ses_011_ac1_…`, `har_020_ac1_…`) auf Bereitschaftssignale statt fester Wartezeiten umstellen.",
        NEW_CI,
        LOCAL_NET,
        design("desktop-update install-update install-compat hosts-overview"),
      ]),
]

# ---------- bestehende Issues (nicht vom Skript angelegt) ----------
# Ganze Zahlen in `deps` verweisen auf diese bereits existierenden Issues; das Skript setzt dafür nur „blocked by“.
EXTERNAL = {
    54: "M3 · Signing-Accounts für macOS und Windows klären und einrichten (human-required)",
    55: "M3 · Domain für beton prüfen und sichern (human-required)",
    56: "M3 · Markenrecht „beton“ prüfen (human-required)",
    177: "M3 · Eigene GitHub-Identität für Agents und Code-Owner-Approval (human-required)",
}

# ---------- human-required für M3 ----------
# #54 (Signing-Accounts), #55 (Domain), #56 (Markenrecht) und #177 (Agent-Identität) existieren bereits im Milestone M3.
# Neu ist nur die Einrichtung der Release-Infrastruktur, die Einstellungen und Schlüssel des Maintainers braucht.
HUMAN = [
 dict(key="HR-RELEASE-INFRA", title="Release-Infrastruktur einrichten: Environment `release`, Homebrew-Tap, Updater-Schlüssel und Required Checks", labels=["human-required", "area:release", "area:qa"], deps=[], features=[],
      goal="Die Release-, Signing- und Update-Pakete von M3 (WP-65 bis WP-67) brauchen Einstellungen, Repositories und Schlüssel, die nur der Maintainer anlegen kann. Dieses Issue sammelt sie, damit die Pakete mit Test-Identitäten vorarbeiten und vor dem Release 0.1 umgestellt werden können.",
      notes=[
        "GitHub-Environment `release` mit Schutzregeln (nur Tags, nur der Release-Workflow) und den Signing-Secrets aus #54; Workflows außerhalb der Environment haben keinen Zugriff (DIST-013 AC4).",
        "Repository `ifahrentholz/homebrew-tap` und ein Token mit minimalen Rechten für das automatische Aktualisieren von Formel und Cask (DIST-004).",
        "Ed25519-Schlüsselpaar für den Tauri-Updater (getrennt vom Code-Signing, DIST-014): privater Schlüssel nur als Secret der Environment `release`, öffentlicher Schlüssel im Repo.",
        "Neue CI-Jobs aus M3 (z. B. Windows-Tests für PTY und TUI, `tauri-driver`, Schema-Kompatibilität) als Required Checks eintragen, sobald die Pakete sie vorschlagen (`scripts/check_branch_protection.py`).",
        "Keine Tokens oder Schlüssel ins Repo. Ergebnis als Kommentar festhalten; bei grundsätzlichen Abweichungen von ADR-0026 ein Folge-ADR.",
        "Fällig vor dem Release 0.1; die Pakete arbeiten bis dahin mit Test-Schlüsseln und lokalen Attrappen.",
      ]),
]

MAINTAINER_RULE = """## Vorgabe des Maintainers: Arbeitsstand vollständig in GitHub
Sobald die Planung eines Meilensteins steht, muss **seine gesamte Arbeit als Issues in GitHub sichtbar** sein, damit der Arbeitsstand jederzeit nachvollziehbar ist:
- Jedes Feature des Meilensteins ist genau einem Arbeitspaket-Issue zugeordnet (Checkliste mit Feature-ID und Spec-Link); vor dem Anlegen per Skript prüfen, dass nichts fehlt und nichts doppelt ist.
- Zusätzliche Arbeit, die beim Umsetzen entsteht (Nacharbeiten, Spec-Korrekturen, Bugs, Folge-Aufgaben), wird ebenfalls als Issue im passenden Milestone angelegt – nicht nur im PR-Text erwähnt.
- Checklisten in den Issues werden beim Umsetzen abgehakt, Issues per PR (`Closes #…`) geschlossen; der Milestone-Fortschritt spiegelt so den echten Stand.
- Diese Vorgabe wird in jedes folgende Planungs-Issue („Mx-Abschluss & Planung Mx+1“) wörtlich übernommen.
"""

PLAN = dict(key="WP-68", title="M3-Abschluss & Planung M4", labels=["type:planning"], deps=["WP-61", "WP-62", "WP-63", "WP-64", "WP-66", "WP-67", "HR-RELEASE-INFRA", 177], features=[],
      goal="M3 formal abschließen, das erste öffentliche Release 0.1 vorbereiten und die Arbeitspakete für M4 schneiden (rollierende Planung).",
      notes=[
        "Prüfen: alle M3-Issues geschlossen (inklusive der aus M2 übernommenen, siehe `docs/plan/m2-triage.md`, und #74), Demo-Szenario (Milestone-Beschreibung) erfüllt – manuell mit der signierten Desktop-App auf einem frischen Rechner und als Offline-E2E (QA-018 AC5, WP-67) –, Subscription-Checkliste für 0.1 protokolliert (QA-019), `cargo xtask spec-coverage --milestone M3` ohne fehlende Must-ACs, `docs/spec/roadmap.md` (`python3 scripts/gen_roadmap.py`) und der Feature-Katalog (`python3 scripts/gen_feature_catalog.py`) aktuell.",
        "Release 0.1 erst, wenn M2 abgeschlossen ist (Escape-Suite SBX-011/QA-005 als Required Check, die in M2 verbliebenen `security-review`-Befunde) und #177 erledigt ist; danach ADR-0036 (und ADR-0040, falls angenommen) durch ein neues ADR ersetzen. Das Veröffentlichen ist unumkehrbar und eine Handlung des Maintainers (AGENTS.md §5).",
        "Erkenntnisse aus M3 in Spec/ADRs zurückführen (z. B. Vendor-Hooks und Approval-Strategien im TUI-Modus, Codex in Stufe 2, Chromium in der Sandbox, Whisper-Backends, Signing und Notarisierung, Update-Kanäle; neue ADRs statt stiller Änderungen).",
        "M4-Features (`docs/spec/roadmap.md` → M4) in Arbeitspakete mit 3–8 Features schneiden – jedes M4-Feature genau einem Paket zugeordnet – und mit `scripts/create_milestone_issues.py` (MS und Paketdefinitionen anpassen) als Issues im Milestone `M4 — Team` anlegen: gleiche Struktur wie die M3-Issues, Labels, „blocked by“-Beziehungen, plus ein Issue „M4-Abschluss & Planung M5“. Pakete in `beton-policy`, `beton-sandbox`, `beton-proxy`, `beton-secrets`, Auth/Sharing sowie Release-, Signing- und Update-Pipeline bekommen `security-review` und werden strikt per TDD umgesetzt.",
        "Übernahmen in die M4-Pakete: die „(ab M4)“-ACs früherer Features (`grep -rn \"(ab M4\" docs/spec/*.md`), die offenen Issues im Milestone M4 (u. a. #75, #246, #255) und alle Übernahmen, die in den M3-Issues entstanden sind (z. B. die Rollen-Teile von SES-019, WEB-010, PROTO-007, BRW-006 und BRW-020, falls verschoben).",
        "Offene Punkte aus `docs/spec/00-overview.md` §9 prüfen (#54, #55, #56, #177) und erledigte streichen.",
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
        out.append("## Abhängigkeiten\nStart erst, wenn diese Issues geschlossen sind:\n" + "\n".join(f"- #{d if isinstance(d, int) else num_of[d]}" for d in wp["deps"]) + "\n")
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
        print("DRY gh", " ".join(args)[:160])
        if inp:
            print("\n".join("    | " + l for l in inp.splitlines()))
        return "{}"
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
    bad_deps += [(wp["key"], d) for d in wp["deps"] if (d not in EXTERNAL if isinstance(d, int) else d not in seen)]
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
        if isinstance(d, int) and d not in id_of:
            id_of[d] = json.loads(gh(["api", f"repos/{REPO}/issues/{d}"]))["id"]
        r = subprocess.run(["gh", "api", "-X", "POST", f"repos/{REPO}/issues/{num_of[wp['key']]}/dependencies/blocked_by",
                            "-F", f"issue_id={id_of[d]}"], capture_output=True, text=True)
        if r.returncode != 0 and "already" not in r.stderr.lower():
            dep_fail.append((wp["key"], d, r.stderr.strip()[:200]))
print("dependency failures:", dep_fail)
print("issues:", num_of)
