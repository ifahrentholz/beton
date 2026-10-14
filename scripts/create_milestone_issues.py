#!/usr/bin/env python3
"""Legt Milestones, Labels und die Arbeitspaket-Issues eines Meilensteins auf GitHub an.

Rollierende Planung (siehe Issue #17 und AGENTS.md): Pro Meilenstein werden die
Arbeitspakete (WPS), ggf. Sonder-Issues für den Maintainer (HUMAN, Label `human-required`)
und das Abschluss-/Planungs-Issue (PLAN) unten definiert. Für den nächsten Meilenstein `MS`
umstellen und WPS/HUMAN/PLAN ersetzen. Die Definitionen früherer Meilensteine (M0: WP-01 bis
WP-16 und LEGAL; M1: WP-17 bis WP-31 und HR-SIGNING/HR-DOMAIN/HR-TRADEMARK) bleiben als Vorlage
in der Git-History.

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
MS = "M2"  # Meilenstein, für den Issues angelegt werden
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

# ---------- Arbeitspakete M2 ----------
# Die Definitionen für M0 (WP-01 … WP-16, LEGAL) und M1 (WP-17 … WP-31, HR-SIGNING, HR-DOMAIN, HR-TRADEMARK)
# stehen in der Git-History dieses Skripts.
# Gemeinsame Hinweise (ADR-0032 Design-first, ADR-0033 lokal, ADR-0034 Subscription-first, Sicherheit) stehen pro Paket,
# damit jedes Issue für sich verständlich ist.
def design(screens, extra=""):
    """Hinweis auf die Prototyp-Screens (ADR-0032); `screens` sind Screen-IDs aus design/prototype/src/screens."""
    s = ", ".join(f"`{x}`" for x in screens.split())
    return (f"**Design-first (ADR-0032):** UI nur nach dem abgenommenen Prototyp (`design/prototype`), Screens {s}{extra}. "
            "Abweichungen im PR begründen. Hat ein Feature einen Screen, aber kein UI-AC, wird die Ansicht nicht stillschweigend "
            f"mitgebaut, sondern als eigenes Issue im Milestone {MS} angelegt.")
SUBSCRIPTION = "**Subscription-first (ADR-0034, ADR-0005):** Default-Auth ist der Login der Vendor-CLI; beton liest, speichert oder verwendet keine Subscription-Tokens. Kein Feature setzt einen API-Key voraus; Tests laufen ohne `*_API_KEY` im Env."
LOCAL = "**Lokal ohne externe Server (ADR-0033):** Tests nur mit Fake-CLIs, Golden Transcripts und Mocks auf Loopback; kein Internet."
LOCAL_NET = ("**Lokal ohne externe Server (ADR-0033):** Netz-Tests nur gegen lokale Test-Upstreams (TLS mit Test-CA) und einen Test-Resolver, "
             "nie gegen `example.com`, `api.github.com` oder öffentliche Registries; die Linux-CI hat kein Netzwerk. Weil der Proxy Loopback und "
             "private Ziele sperrt (PRX-007), brauchen Tests einen ausdrücklichen Test-Zugang, der in einer Produktionskonfiguration nicht erreichbar ist.")
SECURITY = ("**Sicherheitskritisch (AGENTS.md §4, QA-013):** strikt TDD – erst der rote Test, dann die Implementierung. Fail closed: "
            "Fehler oder Ausfall von Engine, Sandbox, Proxy oder Secret-Store führen zur Ablehnung, nie zu stillschweigendem Erlauben. "
            "Merge erst nach Maintainer-Freigabe.")
ESCAPE = "Sandbox-Escape-Tests (SBX-011, QA-005) werden nie abgeschwächt, übersprungen oder auf `#[ignore]` gesetzt, damit die CI grün wird."
PLATFORMS = ("**Plattformen:** macOS (arm64) und Linux sind Pflicht und laufen in der CI. Windows (Beta) bekommt die Sandbox erst mit M5 (SBX-016): "
             "Der Windows-Build bleibt grün, die Backends melden dort `unavailable` (fail closed), es gibt keinen stillen Fallback.")

WPS = [
 dict(key="WP-32", title="Policy-Engine-Kern: CEL, Dokumentformat, Tool-Normalisierung & Hierarchie", labels=["area:security", "area:qa", "security-review"], deps=[],
      features=["QA-013", "QA-011", "POL-002", "POL-001", "POL-005", "POL-007"],
      goal="Neues Crate `beton-policy` mit CEL-Engine, Policy-Dokumentformat samt Hot-Reload, harness-übergreifender Tool-Normalisierung und dem verbindlichen Kombinationsalgorithmus über User → Projekt → Agent, dazu die Prozess-Gates für sicherheitskritische Crates (TDD-Check, Coverage-Floor).",
      notes=[
        "Neues Crate `beton-policy` (00-overview §4.4). Die Engine ist rein: ohne IO, deterministisch, seiteneffektfrei. Die Anbindung an Runner und Adapter folgt mit WP-35.",
        SECURITY,
        "QA-013 zuerst: `CODEOWNERS` für `crates/beton-policy`, `beton-sandbox`, `beton-proxy`, `beton-secrets` und den Auth-Code in `beton-server`, CI-Check `security-tests-touched`, Checkliste in `.github/pull_request_template.md`. AC1 (Merge nur mit Code-Owner-Approval) braucht die Entscheidung aus dem `human-required`-Issue „Code-Owner-Review und Branch-Protection für sicherheitskritische Crates einrichten“. Ist sie beim Abschluss dieses Pakets noch offen, AC1 als eigenes Issue im Milestone M2 anlegen statt es stillschweigend wegzulassen.",
        "QA-011: `cargo llvm-cov` in der CI, Floor `beton-policy` ≥ 90 % ab diesem Paket; `beton-sandbox` (≥ 80 %) kommt mit WP-39 ins Gate, `beton-proxy` und `beton-secrets` werden nur berichtet. Ratchet gegen `main` mit 0,2 Punkten Toleranz.",
        "Kombinationsalgorithmus exakt nach 03-policies.md (Design); POL-007 AC5 als Property-Test mit permutierter Lade-Reihenfolge. Hierarchie-Fälle hier als Rust-Tests, mit WP-40 zusätzlich als YAML-Policy-Tests (QA-004).",
        "POL-001: Schema per `cargo xtask codegen` nach `schemas/v1/policy.schema.json` (Rust-Typen sind Single Source of Truth); der `$schema`-Kommentar zeigt auf die lokale Datei, nie auf eine URL (ADR-0033). `beton setup` legt `~/.beton/policies/defaults.yaml` an. Die Agent-Ebene (`policies:` im `agent.yaml`, AGT-014) kommt mit WP-49; die Lade-Schnittstelle dafür hier vorsehen.",
        "POL-002: `cel-rust`; Kostenlimit und 10-ms-Budget pro `evaluate` mit Fehlerinjektion testen, Benchmark (AC4) mit Schwelle in der CI. Die Kontext-Typen (POL-004) entstehen hier als Deklaration für die Typprüfung, die Befüllung zur Laufzeit folgt mit WP-35.",
        "POL-005: Die Abbildungstabelle liegt als Daten vor (AC3) und wird gegen die vorhandenen Golden Transcripts von Claude, Codex und ACP geprüft.",
        design("policy-overview policy-editor", " (Ladefehler: Typfehler, falsche Phase, doppelte Regel-ID, Hot-Reload fehlgeschlagen); in diesem Paket gibt es kein UI-AC"),
      ]),
 dict(key="WP-33", title="Secret-Store: Keychain, verschlüsselte Datei & Audit-Log", labels=["area:security", "area:store", "area:cli", "security-review"], deps=[],
      features=["SEC-012", "SEC-001", "SEC-002", "SEC-003", "SEC-004"],
      goal="Neues Crate `beton-secrets`: Secrets werden per SecretRef adressiert, im OS-Keychain oder in einer verschlüsselten Datei gespeichert und nur über `get_for_use` herausgegeben; jede sicherheitsrelevante Aktion landet in einem hash-verketteten Audit-Log.",
      notes=[
        "Neues Crate `beton-secrets`. SEC-012 zuerst: Das Audit-Log (Append-only, Hash-Kette, `beton audit list|export|verify`, `GET /v1/audit`) ist Voraussetzung für SEC-001 AC3 und für Proxy und CA (WP-36).",
        SECURITY,
        "SEC-001: `SecretValue` ohne Klartext-`Debug`/`Serialize`, Nullen per `zeroize`; `get_for_use` nur über einen Capability-Typ für Proxy, Direkt-API-Harness und Git-Provider-Client (Sperre zur Compile-Zeit, AC2).",
        "SEC-002: `keyring`-Crate. Die macOS-CI nutzt einen temporären Keychain, die Linux-CI ohne D-Bus den Datei-Fallback (SEC-003); Tests verändern nie den Keychain des Entwicklers.",
        "SEC-003: Argon2id-Parameter laut Spec, atomares Schreiben (Kill-Test AC4), Entsperren über `BETON_SECRETS_PASSPHRASE_FILE` für Headless-Betrieb und CI. Gesperrt heißt `secrets_locked` (fail closed).",
        "SEC-004: Werte nie als CLI-Argument (AC2) und nie in API-Responses (OpenAPI-Snapshot-Test, AC3). `beton secrets test` prüft über den Proxy-Pfad und kommt deshalb mit SEC-011 in WP-46.",
        "Aus M0 (HAR-016 AC3, „ab M2“): Ein gefundener `OPENAI_API_KEY` wird nur nach ausdrücklicher Zustimmung in den Keychain übernommen; die Ausgabe zeigt ihn nur maskiert (`sk-…abcd`).",
        "**Klären:** SEC-012 AC4 (ein Member sieht nur eigene Einträge) setzt Rollen voraus (AUTH-014, M4). Lokal gibt es nur den Owner; den Rollen-Teil nicht vorziehen und die Spec im PR klarstellen.",
        "Lokal ohne externe Server (ADR-0033): kein Vault/KMS (v2), Envelope-Encryption für den zentralen Server erst mit M4.",
        design("secrets-store security-audit-log", "; die Einstellungs-UI hat in M2 kein UI-AC"),
      ]),
 dict(key="WP-34", title="Usage-Records, Preis-Katalog & eigene Preise", labels=["area:store", "area:core", "area:harness"], deps=[],
      features=["USE-001", "USE-002", "USE-003"],
      goal="Jedes `cost.delta` wird zu einem normalisierten Usage-Record, bewertet mit einem versionierten Preis-Katalog und eigenen Preisen für Gateway-Modelle; das ist die Grundlage für Budget-Policies und die Usage-Ansichten.",
      notes=[
        "USE-001 ist Owner der Projektion `usage_records` (Mechanismus DATA-005). `session.cost_usd` und `user.daily_cost_usd` für die Policy-Engine (POL-004, WP-35) kommen aus derselben Projektion.",
        "Aus M0 (DATA-005 AC2, „ab M2“): Ein `cost.delta` ist in derselben Transaktion wie der Append in der Usage-Projektion sichtbar.",
        "Aus M1 (HAR-006 AC3): Codex-Kosten aus dem Preis-Katalog; den Subscription-Teil (USE-004) liefert WP-44.",
        "**Subscription-first (ADR-0034):** Records mit `auth_source: vendor_cli` zählen nie zu Geld-Summen (USE-001 AC3); ein Listenpreis-Äquivalent (`equivalent_usd`) ist nur informativ.",
        "USE-002: Katalog als TOML im Binary; Updates nur mit Releases, nie per Netzabruf (ADR-0033). USE-003: Preisangaben aus `providers` (HAR-011) gelten als Overrides; vom zentralen Modus (AC4) gibt es lokal nur den `doctor`-Hinweis.",
        design("usage-overview usage-pricing", "; die Anzeige kommt mit WP-44 und WP-47"),
      ]),
 dict(key="WP-35", title="Policy-Gate im Runner: Phasen, Kontext, Entscheidungen, Durchsetzungsmatrix & Audit", labels=["area:harness", "area:server", "area:security", "security-review", "needs-vendor-cli"], deps=["WP-32", "WP-34"],
      features=["POL-003", "POL-004", "POL-006", "POL-024", "POL-025"],
      goal="Die Policy-Engine entscheidet an allen Hook-Phasen jeder Session über alle Harnesses, verweigert den Start bei nicht durchsetzbaren Pflichtregeln, und jede Auswertung erzeugt genau ein `policy.decision`.",
      notes=[
        "Ersetzt das bisherige `PolicyGate` aus M0/M1 durch die Engine aus WP-32; die minimale Approval-Karte (WEB-018) bleibt bis WP-37 in Betrieb.",
        SECURITY,
        "POL-024: Die Enforcement-Matrix wird aus den Harness-Capabilities (HAR-002, `tool_call_gate`) abgeleitet, nicht hart codiert. Nicht durchsetzbare Pflichtregeln verhindern den Start (`session.start_denied`); Engine-Fehler und Timeouts ergeben `deny`.",
        "Aus #73 (HAR-005): session-spezifischer `PreToolUse`-Hook im Claude-Adapter, damit auch vom Vendor automatisch erlaubte Tools (z. B. `Read`, `echo`) das Gate passieren; danach deklariert der Claude-Adapter `tool_call_gate: full`. Den Mechanismus gegen die echte `claude`-CLI verifizieren und Golden Transcripts aufnehmen (Secret-Scan `cargo xtask golden-scan`).",
        "Aus M0 (HAR-005 AC3, „ab M2“): Jede Permission-Anfrage erzeugt genau ein `policy.decision`; Adapter-Latenz ohne menschliche Interaktion < 20 ms p99.",
        "Aus M1 (HAR-010 AC5, „ab M2“): Beim Direkt-API-Harness durchläuft jeder Model-Request die Phase `model_request` (Test: `deny` verhindert den HTTP-Request am Mock-Server).",
        "Aus #101 (SES-004): Queue-Einträge durchlaufen beim Abarbeiten, nicht beim Einreihen, die Phase `model_request`; `deny` entfernt den Eintrag mit `policy.decision` im Verlauf, `ask` hält die Queue an. Direkte Eingaben und Steer genauso prüfen.",
        "Aus 02-agents.md (System-Tools, „ab M2“): Aufrufe des `beton`-MCP-Servers durchlaufen `tool_call` mit `tool.kind = \"system\"`.",
        "Aus #71 (HAR-027): Codex deklariert `yolo` erst, wenn beton dessen Approval-Requests automatisch laut Policy beantwortet (POL-006, POL-024); Claude-`yolo` (`bypassPermissions`) erst zusammen mit dem `PreToolUse`-Hook. Das Freischalten selbst (Sandbox-Status) folgt mit WP-49.",
        "POL-004: Kontext-Variablen als Rust-Typen, daraus CEL-Deklaration und Referenztabelle für die Doku (Snapshot-Test); `session.cost_usd` und `user.daily_cost_usd` aus der Usage-Projektion (WP-34). Variablen späterer Meilensteine (`browser.*` M3, `budget.*` und `team` M4) nur als Typen, ohne Wirkung.",
        "**Klären:** POL-003 AC1 verlangt einen Integrationstest pro Phase; `browser_navigate` und `browser_action` gibt es erst mit dem eingebetteten Browser (M3). Spec im PR klarstellen, welche Phasen in M2 getestet werden.",
        "POL-025: keine Tool-Argumente im Klartext (`subject_hash`); Invarianten-Test: zu jedem `tool.call.requested` genau ein `policy.decision`.",
        design("policy-enforcement policy-audit harness-catalog", " (Durchsetzung je Harness)"),
      ]),
 dict(key="WP-36", title="Egress-Proxy-Kern: Instanzen, beton-CA & TLS-MITM, Regeln, HTTP/2 & Private-IP-Schutz", labels=["area:security", "security-review"], deps=["WP-33"],
      features=["PRX-001", "PRX-002", "PRX-003", "PRX-004", "PRX-007"],
      goal="Neues Crate `beton-proxy`: ein Forward-Proxy in Rust mit eigener Identität pro Session-Stufe, beton-CA für TLS-MITM, Default-Deny-Regeln, Prüfung jedes HTTP/2-Streams und Schutz vor privaten Zielen und DNS-Rebinding.",
      notes=[
        "Neues Crate `beton-proxy` (tokio, hyper, rustls), läuft im Runner (Stufe 0). Die Erzwingung „Netz nur zum Proxy“ in der OS-Sandbox kommt mit WP-39, WP-41 und WP-45 (PRX-008).",
        SECURITY,
        "PRX-002: Der CA-Schlüssel liegt nur im Secret-Store (WP-33), nie als Klartextdatei; die CA wird nie in einen System-Trust-Store installiert; die Rotation (`beton proxy ca rotate`) geht ins Audit-Log. Ist der Store gesperrt, startet keine Session mit Netz (fail closed).",
        "PRX-003: Regeltests deklarativ unter `tests/egress/*.yaml` (AC1); Parser und Matcher von Anfang an mit `proptest` gegen eine Referenzimplementierung. Die Fuzz-Targets (QA-009) bündelt WP-48.",
        "PRX-007: eigener Resolver, Verbindung nur zur geprüften IP; der Test-Resolver ist der Injektionspunkt für DNS-Rebinding-Tests (AC2) und für alle anderen Netz-Tests.",
        LOCAL_NET,
        design("egress-proxy egress-log", "; die Proxy-Ansicht hat in M2 kein UI-AC"),
      ]),
 dict(key="WP-37", title="Approvals, Policy-Zustand, Inbox & Approval-Bar", labels=["area:server", "area:web", "area:security", "security-review"], deps=["WP-35"],
      features=["POL-009", "POL-010", "POL-014", "UX-001", "WEB-007"],
      goal="`ask`-Entscheidungen werden zu genau einer Approval-Card, die in der Approval-Bar der Session und in der zentralen Inbox beantwortbar ist, mit Grants, Timeouts, Zuständen und Zählern, die Neustarts überleben.",
      notes=[
        SECURITY,
        "POL-010: Offene Approvals überleben Daemon-Neustarts (AC4); Timeout-Kombination exakt nach Spec. AC3 (403 für `view`) ist „ab M4“ und wird nicht vorgezogen.",
        "POL-009: `session`- und `run`-Zustand als `policy.state_changed` im Session-Log (replay- und fork-fähig, ein Fork übernimmt bis `up_to_seq`); `user_day` und `user` im Store, Tagesgrenze in `user.timezone` (simulierte Uhr).",
        "UX-001: Inbox-Projektion aus dem Event-Log (Rebuild-Test, AC4). Aus M1 (AGT-007 AC4, „ab M2“) und 02-agents.md: System-Tools `inbox_read` und `ask_user`; `ask_user` erzeugt eine Inbox-Frage, die Antwort erscheint als Tool-Result.",
        "**Klären:** UX-001 AC3 verlangt einen Tray/Dock-Badge, die Desktop-App (DESK-004) kommt erst mit M3. In M2 nur der Zähler in der Web-UI; den Desktop-Teil per Spec-Änderung nach M3 verschieben (eigenes Issue).",
        "WEB-007 löst die minimale Approval-Karte (WEB-018) ab; Tastatur (`⌘⏎`, `⌘⌫` ist eine Annahme, mit dem Prototyp abgleichen) und axe-Test für `role=\"alertdialog\"`.",
        "**Design fehlt, vorher entwerfen und abnehmen lassen:** Der Prototyp zeigt in `session-stream` (Zustand „Freigabe offen“) nur eine einzelne Karte im Verlauf. Es fehlen die fixierte Leiste über dem Composer mit gestapelten Approvals, ausklappbare Argumente bzw. Diff bei Datei-Edits und der Zustand „entschieden von …“ (WEB-007 AC1, AC2).",
        design("session-stream policy-approvals inbox-list inbox-shortcuts"),
        "E2E (Playwright gegen Fake-Harness): Approval in der Bar erlauben, ein zweiter Client sieht „entschieden von …“, Approve aus der Inbox setzt eine pausierte Session fort.",
      ]),
 dict(key="WP-38", title="Redaction-Pipeline, Payload-Redaktion & OpenTelemetry-Traces", labels=["area:security", "area:store", "area:protocol", "security-review"], deps=["WP-33", "WP-35"],
      features=["SEC-013", "OBS-002", "DATA-012", "OBS-003"],
      goal="Secrets erscheinen nirgends im Klartext, weder in Events, Tool-Outputs und Logs noch in Traces, Exporten oder Diagnose-Bundles; nachträglich entdeckte Leaks lassen sich manuell schwärzen, und jede Session ist als OTel-Trace nachvollziehbar.",
      notes=[
        SECURITY,
        "SEC-013 ist Owner der Pipeline (Aho-Corasick über bekannte Werte inklusive Base64- und URL-Varianten, Muster-Detektoren); OBS-002 bindet sie als `tracing`-Layer ein (OBS-001). Benchmark ≥ 50 MB/s (AC3) mit Regressionsschwelle in der CI.",
        "Aus M1 (SES-007 AC3, „ab M2“): Der Handover-Kontext beim Harness-Wechsel enthält keine Secret-Werte, nur `bt_cred_*` (Test mit präpariertem Log).",
        "Aus M1 (SES-009 AC2, „ab M2“): Der Session-Export enthält keine Secret-Werte (Scan auf bekannte Test-Secrets).",
        "DATA-012: `POST /v1/sessions/{id}/events/{seq}/redact`, `event.redacted` an verbundene Clients, Audit-Eintrag (WP-33). **Klären:** Rollen (Owner/Admin, `view` → 403 in AC3) kommen erst mit M4 (COL-001); lokal ist der Owner berechtigt. Rollen-Teil nicht vorziehen, Spec im PR klarstellen.",
        "OBS-003: Export nur, wenn `observability.otlp.endpoint` bzw. `OTEL_EXPORTER_OTLP_ENDPOINT` gesetzt ist (ADR-0025, ADR-0033); ohne `capture_content` keine Prompts oder Tool-Argumente in Spans. **Klären:** AC1 nennt einen Testcontainer; die Linux-CI hat kein Netzwerk, deshalb einen OTLP-Empfänger im Prozess auf Loopback verwenden und die Spec anpassen.",
        design("secret-redaction diag-events diag-logs diag-bundle diag-metrics", " (Schwärzen im Event-Log: Zustände `redact` und `redacted`)"),
      ]),
 dict(key="WP-39", title="Sandbox-Kern & macOS-Backend (Seatbelt)", labels=["area:security", "area:runner", "security-review"], deps=["WP-36"],
      features=["SBX-001", "SBX-003", "SBX-004", "SBX-007"],
      goal="Neues Crate `beton-sandbox` mit Provider-Trait, Registry, deklarativer Konfiguration samt fail-closed-Pfadauflösung und Default-Masken, dazu das macOS-Backend auf Seatbelt (deny-default).",
      notes=[
        "Neues Crate `beton-sandbox`; ab hier gilt sein Coverage-Floor ≥ 80 % (QA-011, WP-32).",
        SECURITY + " " + ESCAPE,
        "SBX-001 AC2: Lint (`disallowed-methods`) gegen `Command::spawn` für Harness und Tools außerhalb von `beton-sandbox` und `beton-pty`. Die bestehenden Spawns aus M0/M1 (Runner, Adapter, MCP-stdio-Server) laufen dafür über `SandboxProvider::spawn`, vorerst mit `backend: none` als ausdrücklichem Provider. Das Verhalten bleibt bis WP-49 gleich; den Default „Sandbox erforderlich“ (SBX-006) schaltet erst WP-49 scharf.",
        "SBX-003: Pfade nur mit `~` expandieren, `realpath`, verbotene Grants sind Fehler. Aus M1 (SES-015, „ab M2“): Der Worktree (inklusive `.git/worktrees/<name>` und Objekt-DB) ist Schreib-Root; SES-015 AC2 (Schreiben ins Haupt-Checkout scheitert) wird mit WP-45 in Stufe 2 nachgewiesen.",
        "Aus #92: Die Workspace-API löst Pfade race-frei auf (`openat`-Ketten mit `O_NOFOLLOW`, unter Linux `openat2` mit `RESOLVE_BENEATH`). Das ist dieselbe Technik wie bei der Pfadauflösung hier, und mit der Sandbox wird die API zur relevanten Grenze; #92 im selben Zug schließen oder verlinken.",
        "SBX-007: Profil aus einem versionierten Template mit Parametern (`-D KEY=VALUE`), nie per String-Konkatenation; Fuzz-Test des Profil-Generators (AC3). `sandbox-exec` braucht weder Entitlements noch Notarisierung; die signierte App kommt erst mit M3 (#54). CI auf `macos-latest` und der Vorversion (SBX-011).",
        "Plattformübergreifende ACs (SBX-003 AC4, SBX-004) werden hier auf macOS nachgewiesen; dieselben Tests laufen ab WP-41 auch auf Linux.",
        "Netz-ACs (SBX-007 AC2, AC5) über den Proxy aus WP-36 gegen lokale Test-Upstreams, nicht gegen `example.com` (ADR-0033).",
        PLATFORMS,
        design("sandbox-settings sandbox-probe sandbox-unavailable sandbox-stages", "; die Einstellungs-UI hat in M2 kein UI-AC"),
      ]),
 dict(key="WP-40", title="Policy-Tests, Explain & Dry-Run", labels=["area:security", "area:cli", "area:web", "area:qa", "security-review"], deps=["WP-37"],
      features=["POL-026", "POL-027", "QA-004"],
      goal="Policies sind testbar und erklärbar: `beton policy test` mit YAML-Fällen und JUnit, `explain`, `eval` und `show` für jede Entscheidung, Shadow-Modus und ein CI-Gate für alle mitgelieferten Policies.",
      notes=[
        SECURITY,
        "Kommt vor den Regeltypen (WP-42, WP-43), damit jeder eingebaute Regeltyp seine mitgelieferte Testdatei im POL-026-Format bekommt (POL-026 AC4).",
        "QA-004: CI-Gate über alle mitgelieferten Policies (inklusive `defaults.yaml` aus `beton setup`) und Built-in-Agents (`maestra`, `duetto`); Hierarchie-Fälle sind Pflicht, dazu der Monotonie-Property-Test (AC3).",
        "POL-027: Set-Versionen im Blob-Store (DATA-006), damit `explain` alte Entscheidungen rekonstruiert (AC2); `eval` ohne Events, Approvals und Zustandsänderungen (AC3). AC4 braucht die aggregierte Shadow-Ansicht in der Web-UI (Screen `policy-shadow`).",
        "Aus M1 (AGT-007 AC3, „ab M2“): Das System-Tool `policy_query` liefert dieselbe Entscheidung wie die Engine bei echter Ausführung (Testfall `git push`).",
        "Aus M0 (HAR-021 AC3, „ab M2“): `session.cost_usd` ist nach jedem `cost.delta` aktualisiert (Test über `beton policy explain`).",
        "Aus #73 (PROTO-010 AC3): Schreibende Policy-Endpunkte (`PUT /v1/policies/…`) nutzen `extract::check_if_match` (412 bei veraltetem `If-Match`, nichts verändert). **Klären:** Kein M2-Feature definiert `PUT /v1/policies` für lokale Policies (API-002 nennt nur `/v1/policies` und `POST /v1/policies/evaluate`, die Org-/Team-Endpunkte sind POL-008 in M4). Entsteht der Endpunkt hier nicht, den Punkt aus #73 nach M4 verschieben.",
        design("policy-tests policy-explain policy-shadow"),
      ]),
 dict(key="WP-41", title="Linux-Backend: Landlock, seccomp, Namespaces, bubblewrap & Ressourcenlimits", labels=["area:security", "area:runner", "security-review"], deps=["WP-39"],
      features=["SBX-008", "SBX-009", "SBX-013"],
      goal="Linux-Sandbox nativ in Rust (Landlock, seccomp, unprivilegierte Namespaces mit Relay zum Proxy), bubblewrap als Fallback und begrenzte, vollständig aufräumbare Prozessbäume auf Linux und macOS.",
      notes=[
        SECURITY + " " + ESCAPE,
        "SBX-008: Launcher `beton __sandbox-exec`, Kernel-Matrix laut Spec (ABI 2 mit `truncate`-Sperre per seccomp, ABI ≥ 4 mit TCP-Regeln). Ubuntu 24.04 beschränkt unprivilegierte User-Namespaces per AppArmor: In der CI das Profil ausdrücklich setzen und den Fall „ohne Profil“ (AC5) als eigenen Test prüfen. Das AppArmor-Profil für deb/rpm wird mit der Paketierung (M3) ausgeliefert; hier entstehen Profil-Datei und Test.",
        "**Klären:** SBX-008 AC4 (Debian 12, Kernel 6.1) und die Matrix aus SBX-011 brauchen andere Kernel als die GitHub-Runner (Ubuntu 22.04/24.04); Container teilen den Host-Kernel. Einen Plan für VM-Jobs (z. B. KVM auf den Linux-Runnern) mit gecachten Images festlegen; der Testlauf selbst bleibt ohne Netz.",
        "SBX-009: bwrap ≥ 0.8; keine Env-Werte in bwrap-Argumenten (AC3).",
        "SBX-013: cgroup v2 über systemd-User-Delegation, sonst rlimits; macOS mit rlimits und Erfassung der Nachfahren (Restrisiko `setsid` dokumentieren). Die Fork-Bombe nur mit Limit testen, der Host muss bedienbar bleiben.",
        "Aus M0 (OBS-005 AC3, „ab M2“): `beton doctor` meldet auf Linux ohne Landlock `fail` für `sandbox.linux` mit Verweis auf die Kernel-Anforderung.",
        PLATFORMS,
        design("sandbox-probe sandbox-violation", " (Grund und Fallback in der Probe, Zustand `limit`)"),
      ]),
 dict(key="WP-42", title="Regeltypen für Tools, Pfade & Git, Schleifen- und PII-Erkennung", labels=["area:security", "security-review"], deps=["WP-40"],
      features=["POL-015", "POL-016", "POL-017", "POL-018", "POL-019"],
      goal="Eingebaute Regeltypen `tool_allowlist`, `path_guard`, `git_guard`, `loop_detection` und `pii_redaction`, jeweils zu CEL entzuckert und mit mitgelieferten Policy-Tests.",
      notes=[
        SECURITY,
        "Jeder Regeltyp wird zu CEL entzuckert (sichtbar in `beton policy explain`) und bekommt eine mitgelieferte `*.policytest.yaml` (POL-026 AC4, QA-004).",
        "POL-017 ist Teil des Demo-Szenarios: Force-Push-Erkennung (`-f`, `--force`, `--force-with-lease`, `+refspec`, `--mirror`, `git -C`, verkettete Kommandos), `parse_uncertain` ergibt `ask`; auch über MCP-Tools.",
        "**Klären (Demo-Szenario):** Laut Demo erzeugt `git push --force` eine Approval-Card. Laut POL-017 ist der Default aber `force_push: deny`, und `beton setup` legt `force_push: deny` auf User-Ebene an, die ein Projekt nicht lockern kann (POL-007). Entweder das Demo-Szenario auf „wird abgelehnt“ ändern oder für die Demo `force_push: ask` auf User-Ebene setzen; die Entscheidung im PR festhalten (Spec bzw. ADR).",
        "POL-016: Die Pfaderkennung in Shell-Kommandos ist best effort, die verbindliche Durchsetzung leistet die Sandbox (WP-39, WP-45). Die ACs prüfen nur die Normalisierung, daher keine Abhängigkeit auf die Sandbox-Pakete.",
        "POL-018 (Should) nutzt `tool.repeat_count` und `tool.consecutive_errors` aus WP-35. POL-019 (Could): Regex und Luhn, kein ML; ohne Result-Modifikation des Harness `notify` mit `enforcement: observe_only`.",
        design("policy-overview policy-editor policy-approvals git-conflicts", " (Zustand `loop` in `policy-approvals`)"),
      ]),
 dict(key="WP-43", title="Budgets & Modell-Routing: `spend_cap`, `daily_budget`, `model_route`", labels=["area:security", "area:cli", "security-review"], deps=["WP-40"],
      features=["POL-011", "POL-012", "POL-013"],
      goal="Kostenobergrenzen pro Session, Run, Teilbaum und Tag sowie manuelles Modell-Routing per Policy; bei Überschreitung wird gestoppt, nachgefragt oder umgeroutet.",
      notes=[
        SECURITY,
        "Teil des Demo-Szenarios („bei Überschreitung des Budgets wird gestoppt“). **Subscription-first (ADR-0034):** Subscription-Sessions haben kein `cost_usd`; `spend_cap` greift dann nur mit `cost_source = catalog` (Listenpreis-Äquivalent), sonst kommt `policy.rule_inapplicable`. **Klären:** Die Annahme in POL-011 festschreiben und das Demo damit führen, damit auch eine Subscription-Session stoppbar ist.",
        "Aus M0 (API-006 AC4, „ab M2“): `beton run -p --max-cost` endet bei Überschreitung mit Exit-Code 5 (über eine `spend_cap`-Regel).",
        "POL-011 AC2 und POL-013: Modellwechsel über HAR-017 (`session.settings_changed.mechanism` ist `live` oder `restart`); `on_unsupported` hat den Default `deny`.",
        "POL-012: Tagesgrenze in der Zeitzone des Users (simulierte Uhr); Team-Scope und Budget-Leases (POL-021) sind M4 und werden nicht vorgezogen.",
        design("policy-budgets policy-approvals", " (Zustand `spend` – Kostenschwelle)"),
      ]),
 dict(key="WP-44", title="Usage-API, Subscription-Usage & `beton usage`", labels=["area:server", "area:cli", "area:harness", "needs-vendor-cli"], deps=["WP-34", "WP-37"],
      features=["USE-004", "USE-005", "USE-007", "CLI-010"],
      goal="Aggregierte Usage über `GET /v1/usage` und `beton usage`, mit Subscription-Usage (Tokens, Rate-Limit-Fenster) statt Geldbeträgen für eingeloggte Vendor-CLIs.",
      notes=[
        SUBSCRIPTION + " Subscription-Usage zeigt Tokens und Rate-Limit-Fenster, nie Mischsummen mit Geld (CLI-010 AC2).",
        "USE-004: Quellen (Claude-`result`, Codex-Notifications zu Token-Usage und Rate-Limits) gegen die echten CLIs verifizieren, Annahmen in 11-platform-features.md und 01-harnesses.md korrigieren, Golden Transcripts aufnehmen. Ohne Reporting „nicht gemeldet“ statt 0. AC3 (Inbox-Hinweis ab 90 %) nutzt die Inbox aus WP-37 und ist per Konfiguration abschaltbar; die Einstellungs-UI (UX-006) ist M3.",
        "Aus M1 (HAR-006 AC3): der Subscription-Teil der Codex-Usage (`usage.subscription`).",
        "**Klären:** USE-005 AC2 (Sichtbarkeit nach Rolle) und AC3 (Postgres-Benchmark) setzen AUTH-014 bzw. DATA-004 voraus, beide M4. In M2 nur lokal (ein Owner, SQLite); AC2 und AC3 per Spec-Änderung als „ab M4“ markieren (eigenes Issue).",
        "USE-007: Ohne laufenden Daemon read-only direkt aus der lokalen DB (AC3); stdout nur Daten, Hinweise auf stderr (CLI-001). CLI-010 regelt nur die Konsistenz der Oberfläche, Owner sind USE-005 und USE-007.",
        design("usage-cli cli-usage settings-notifications"),
      ]),
 dict(key="WP-45", title="Zweistufige Sandbox: Exec-Broker, Vendor-Passthrough, Netz nur zum Proxy & Presets", labels=["area:security", "area:runner", "area:harness", "security-review", "needs-vendor-cli"], deps=["WP-39", "WP-41"],
      features=["SBX-002", "PRX-005", "PRX-008", "SBX-012"],
      goal="Harness-Prozesse laufen in Stufe 1 mit Passthrough nur zu ihren Vendor-Hosts, jede Tool-Ausführung in einer frischen Stufe-2-Sandbox über den Exec-Broker, und auf beiden Plattformen erreicht Netzverkehr nur noch den Proxy.",
      notes=[
        SECURITY + " " + ESCAPE,
        "**Erster Schritt:** die als *(Annahme)* markierten Mechanismen gegen die echten CLIs verifizieren und die Spec korrigieren: Claude-Bash über `CLAUDE_CODE_SHELL_PREFIX` bzw. Shell-Override, Codex über `bt-exec` als `bash`/`sh`-Shim im `PATH`, `credential_paths` und `keychain` je Adapter, Vendor-Hosts für den Passthrough (PRX-005). Golden Transcripts aufnehmen; die CI nutzt nur Fake-CLIs.",
        "SBX-002 AC1 und PRX-005 AC1: manuelle Verifikation mit eingeloggter `claude`-CLI als Checkliste im PR (Subscription funktioniert in Stufe 1, der Proxy-Dump zeigt für `api.anthropic.com` keine entschlüsselten Header). " + SUBSCRIPTION,
        "ACP (`fs/*`, `terminal/*`) und der Direkt-API-Harness führen Tools über beton aus, also immer in Stufe 2; ein Adapter mit `tool_isolation: inherited` startet nur mit `require_tool_isolation: false` (AC4). MCP-stdio-Server laufen ab hier in Stufe 2 (in M1 noch ohne Sandbox, WP-23).",
        "Aus M1 (SES-015 AC2, „ab M2“): Ein Tool-Call, der aus dem Worktree heraus ins Haupt-Checkout schreibt, scheitert an der Sandbox.",
        "PRX-008: Proxy- und CA-Variablen für `curl`, `git`, `npm`, `pip`, `cargo` und Node-`fetch` (AC1); bekannte Lücken (Go auf macOS, Java) scheitern geschlossen und werden dokumentiert. SBX-012 AC1 (`npm install`, `npm publish`) gegen eine lokale Registry-Attrappe statt `registry.npmjs.org`.",
        LOCAL_NET,
        PLATFORMS,
        design("sandbox-stages sandbox-settings egress-proxy"),
      ]),
 dict(key="WP-46", title="Credential-Injection, Env deny-by-default, Egress-Audit & Git-PAT", labels=["area:security", "area:cli", "security-review"], deps=["WP-45"],
      features=["PRX-006", "PRX-009", "SBX-005", "SEC-011", "PRX-010"],
      goal="Agents sehen Secrets nur als `bt_cred_*`: Der Proxy setzt echte Werte nur für gebundene Hosts ein, das Sandbox-Environment wird explizit gebaut, jede Egress-Entscheidung und Credential-Nutzung wird auditiert, und Git-PATs funktionieren über Platzhalter.",
      notes=[
        SECURITY + " " + ESCAPE,
        "Teil des Demo-Szenarios („sieht Secrets nur als `bt_cred_*`“).",
        "PRX-006: Platzhalter mit 160 bit pro Session und Binding; ersetzt wird nur in Headern und URL, nie im Body, nie bei `TRACE`, nie auf Passthrough-Verbindungen. Fehlgebrauch ergibt 403 `credential_host_mismatch` und den Audit-Eintrag `credential.misuse`. Das Fuzz-Target für die Ersetzung bündelt WP-48 (QA-009).",
        "Übernahmen „ab M2“: HAR-011 AC2 (M1, Direkt-API-Key nur als Platzhalter im Loop-Request, der Proxy ersetzt ihn), HAR-015 AC4 (M0, `auth: api_key` ergibt nur `bt_cred_*` im Harness-Env), AGT-001 AC3 (M1, `${secret:github}` wird zu `bt_cred_*`), AGT-006 AC3 (M1, `${secret:x}` in MCP-`env`/`headers` nur als Platzhalter).",
        "**Klären:** PRX-005 schließt Credential-Injection auf Passthrough-Verbindungen aus, Vendor-Hosts sind in Stufe 1 aber Passthrough. Für HAR-015 AC4 (`auth: api_key`) muss der Vendor-Host dieser Session über MITM statt Passthrough laufen, und der Direkt-API-Loop (Stufe 0) muss für HAR-011 AC2 über den Proxy gehen. Spec (PRX-005, HAR-011) im PR präzisieren.",
        "SEC-011: `beton setup` übernimmt `gh`- bzw. `glab`-Tokens nur nach ausdrücklicher Bestätigung (AC2); `beton secrets test` (SEC-004) prüft über den Proxy-Pfad. AC1 (`git clone`, `gh pr list`) gegen lokale Git- und API-Attrappen statt github.com.",
        "SBX-005: Werte nie über argv (AC4, `/proc/<pid>/cmdline`); `SSH_AUTH_SOCK`, `LD_PRELOAD`, `DYLD_*` und `BETON_TOKEN` werden immer entfernt.",
        "PRX-009: nie Header-Werte, Cookies oder Query-Werte in Logs und Events (Grep-Test mit Markern, AC2); `beton audit list --kind credential` (AC4).",
        LOCAL_NET,
        design("credential-injection egress-log security-audit-log git-connections secrets-store"),
      ]),
 dict(key="WP-47", title="Web-UI: Usage-Seite, Kontextanzeige & Compaction-Trigger", labels=["area:web"], deps=["WP-44"],
      features=["USE-006", "USE-008", "USE-009"],
      goal="Usage-Seite mit Kennzahlen, Zeitreihen, Tabellen und Subscription-Bereich, dazu der Kontextfüllstand im Composer und manuelle bzw. automatische Compaction.",
      notes=[
        design("usage-overview usage-pricing usage-context session-stream workspace-composer inbox-palette"),
        "USE-006: Summen identisch mit `GET /v1/usage` (E2E gegen den Fake-Harness-Datensatz), CSV-Export der sichtbaren Ansicht, Hinweis „ohne Preis – Override anlegen“ mit Link zu den Pricing-Einstellungen (Screen `usage-pricing`). Der Subscription-Bereich zeigt keine Geldbeträge (ADR-0034).",
        "Aus M1 (#113): Der Compact-Button im Composer kommt mit dem Zustand aus der Capability; USE-009 ergänzt Trigger, Tooltip mit Grund bei `compaction: none` und die Automatik `compaction.auto`.",
        "**Klären:** USE-008 AC3 verlangt die Anzeige in der TUI-Statuszeile; die TUI (TUI-001) kommt erst mit M3. In M2 nur Web; den TUI-Teil per Spec-Änderung nach M3 verschieben (eigenes Issue).",
        "Diagramme aus dem eigenen Bundle, kein CDN (ADR-0033); Performance-Budgets (WEB-015) einhalten.",
      ]),
 dict(key="WP-48", title="Sandbox-Escape-Suite, `beton sandbox`-CLI, Verstoß-Reporting & Fuzzing", labels=["area:security", "area:qa", "area:cli", "security-review"], deps=["WP-32", "WP-46"],
      features=["SBX-010", "SBX-011", "QA-005", "QA-009", "SBX-014"],
      goal="Die Sandbox ist nachweisbar dicht: deklarative Escape-Suite mit Probe-Binary in einer CI-Matrix pro OS als Merge-Gate, `beton sandbox probe|explain|exec|test`, Verstoß-Events an der Tool-Card und Fuzz-Targets für alle sicherheitskritischen Parser.",
      notes=[
        SECURITY + " " + ESCAPE,
        "SBX-011 ist Owner von Suite, Probe-Binary (`bt-escape-probe`) und Plattform-Matrix, QA-005 von Gate und JUnit-Report. Kontrollgruppe ohne Sandbox (QA-005 AC1) und Mutationstest mit gelockertem Profil (AC2) belegen die Wirksamkeit. Gefälschtes `HOME` mit Fixture-Secrets, die in keinem CI-Log auftauchen (SBX-011 AC3).",
        "CI-Matrix: macOS arm64 aktuell und Vorversion; Ubuntu 22.04, Ubuntu 24.04 mit AppArmor-Restriktion, Debian 12 und Fedora (VM-Plan aus WP-41). Jeder Linux-Job prüft seine erwartete Landlock-ABI (mindestens einer ≥ 4) und scheitert, statt Fälle zu überspringen (QA-005 AC3). Die neuen Required Checks setzt der Maintainer (human-required-Issue „Code-Owner-Review und Branch-Protection …“). Die Windows-Teilmenge kommt erst mit M5.",
        "QA-009: `cargo-fuzz`-Targets unter `fuzz/` für HTTP/1.1- und CONNECT-Parser, ClientHello/SNI, Egress-Regeln, Platzhalter-Ersetzung, CEL (WP-32), Policy-YAML-Loader, WS-Frame-Decoder und SBPL-Escaping; Seed-Korpora im Repo, im PR 60 s je betroffenem Target, nightly 30 min (`.github/workflows/nightly.yml`).",
        "SBX-010: `beton sandbox probe --json` und Einbindung in `beton doctor` (OBS-005).",
        "SBX-014 (Could): Der Vorschlag „Regel hinzufügen“ schreibt erst nach Bestätigung in `.beton/config.yaml`; Verstoß-Events höchstens 10/s je Session.",
        PLATFORMS,
        design("sandbox-probe sandbox-violation", " (Zustand `suggest`)"),
      ]),
 dict(key="WP-49", title="YOLO in der Sandbox: Fail-closed, Agent-Policies, ACP-Durchsetzung & M2-Demo", labels=["area:security", "area:harness", "area:web", "area:qa", "security-review", "needs-vendor-cli"], deps=["WP-42", "WP-43", "WP-46"],
      features=["SBX-006", "AGT-014", "POL-023"],
      goal="Sessions starten nur noch mit aktiver Sandbox (oder ausdrücklich bestätigtem `backend: none`), YOLO wird mit Sandbox und Proxy freigeschaltet, Agents bringen eigene Policies und Sandbox-Anforderungen mit, ACP-Agents werden lückenlos geprüft, und das M2-Demo-Szenario läuft als E2E-Test.",
      notes=[
        SECURITY + " " + ESCAPE,
        "Das Demo-Szenario (Milestone-Beschreibung) wird hier zum automatisierten E2E-Test mit Fake-Harness auf macOS und Linux: YOLO-Session in der Sandbox, `cat ~/.ssh/id_ed25519` scheitert, ein nicht erlaubter Host wird blockiert (`egress.blocked`), Env und Events zeigen nur `bt_cred_*`, `git push --force` erzeugt eine Approval-Card bzw. wird abgelehnt (Klärung in WP-42), Budget-Überschreitung stoppt (WP-43). Dazu eine manuelle Verifikation mit echten, eingeloggten `claude`- und `codex`-CLIs als Checkliste im PR.",
        "Aus #71 (YOLO freischalten): `beton_harness::SandboxStatus::current()` liefert in M1 fest „keine Sandbox, kein Proxy“, deshalb endet `yolo` beim Anlegen, Forken, Wechseln und Runner-Start mit `409 sandbox_required`. Der Status kommt jetzt aus der echten Sandbox- und Proxy-Konfiguration (SBX-006, PRX-008) und lässt sich nie über die Projekt-Konfiguration erzwingen.",
        "Aus #71 (Sandbox-Status für Clients): Status über die API (z. B. `GET /v1/info` bzw. an der Session) und der Mode-Picker mit den Zuständen „YOLO ohne Sandbox“ (gesperrt, Link „Sandbox einrichten“) und „Sandbox ✓ aktiv · Proxy ✓ aktiv“ (Screen `harness-switching`). Ziel des Links ist Screen `sandbox-settings`; diese Ansicht hat in M2 kein UI-AC und wird als eigenes Issue angelegt, wenn sie gebaut werden soll.",
        "Aus M1 (HAR-027 AC2, „ab M2“): Im Modus `yolo` wird eine `deny`-Policy auf `git push --force` weiterhin durchgesetzt (Golden- bzw. Fake-Test); Codex- und Claude-`yolo` wie in WP-35 (Approval-Automatik bzw. `PreToolUse`-Hook).",
        "Aus M1 (AGT-011 AC2, „ab M2“): `maestra` bekommt eine Agent-Policy (AGT-014), die eigene `Bash`- und `Edit`-Aufrufe ablehnt; Test über die mitgelieferten Policy-Tests der Built-in-Agents (QA-004).",
        "Aus M1 (HAR-007 AC2, „ab M2“) zusammen mit POL-023 AC2: Ein ACP-`fs/write_text_file` außerhalb des Worktrees wird von `path_guard` bzw. der Sandbox abgelehnt und als JSON-RPC-Fehler gemeldet.",
        "SBX-006: Default „Sandbox erforderlich“ für alle Harness-Sessions; `backend: none` nur mit einmaliger Bestätigung pro Session und Audit-Eintrag (SEC-012), per User-Policy verbietbar (AC4; die Org-Ebene kommt erst mit M4). Fällt der Proxy aus, werden neue Tool-Ausführungen abgelehnt und die Session geht auf `paused` (AC3). Bestehende Tests (Playwright, `offline-e2e`) laufen danach mit Sandbox oder setzen `backend: none` ausdrücklich.",
        "**Klären (Windows-Beta):** Mit dem Default „Sandbox erforderlich“ startet unter Windows bis M5 (SBX-016) keine Session ohne bestätigtes `backend: none`. Verhalten (Hinweis in `beton doctor`, Bestätigung) festlegen und dokumentieren.",
        design("harness-switching sandbox-unavailable agent-detail policy-enforcement", " (Zustand `yolo-blocked`)"),
      ]),
]

# ---------- human-required für M2 ----------
# Die offenen Punkte aus 00-overview §9 (Signing-Accounts, Domain, Markenrecht) sind die Issues #54–#56 im
# Milestone M3 und werden hier nicht wiederholt. Für die macOS-Sandbox sind weder Entitlements noch Notarisierung
# nötig (`sandbox-exec`); die signierte App kommt mit M3. Neu für M2 ist nur die Review-Pflicht aus QA-013.
HUMAN = [
 dict(key="HR-CODEOWNERS", title="Code-Owner-Review und Branch-Protection für sicherheitskritische Crates einrichten", labels=["human-required", "area:qa", "area:security"], deps=[], features=[],
      goal="QA-013 (M2) verlangt, dass Änderungen an `beton-policy`, `beton-sandbox`, `beton-proxy`, `beton-secrets` und am Auth-Code nur mit Approval des Maintainers als Code-Owner mergebar sind. Agents öffnen PRs derzeit unter dem Account des Maintainers, und GitHub zählt das Approval des PR-Autors nicht. Entscheiden, wie das gelöst wird, und die Repository-Einstellungen setzen.",
      notes=[
        "Optionen: (a) eine eigene Identität für Agents (Bot-Account oder GitHub App) für Commits und PRs, der Maintainer bleibt Code-Owner und gibt frei; (b) QA-013 per ADR ändern, z. B. dürfen nur der Maintainer bzw. eine Bypass-Liste eines Rulesets sicherheitskritische PRs mergen. Ein zusätzlicher Account bzw. eine App ist eine Account-Entscheidung des Maintainers.",
        "Gleiches Problem beim Label `no-test-needed`, das laut QA-013 nur Menschen setzen dürfen: Unter einem gemeinsamen Account ist das nicht unterscheidbar.",
        "Danach die Branch-Protection von `main` ergänzen: „Require review from Code Owners“ (bei Option a) und die neuen Required Checks aus M2 (Escape-Suite macOS/Linux aus SBX-011/QA-005, Coverage-Floor QA-011, `security-tests-touched` QA-013, Policy-Tests QA-004). `enforce_admins` ist aktiv; eine Fehlkonfiguration blockiert alle Merges.",
        "Keine Tokens oder Schlüssel ins Repo; ein Agent-Token nur mit minimalen Rechten.",
        "Ergebnis als Kommentar festhalten (ggf. neues ADR zu ADR-0031). Fällig, bevor WP-32 abgeschlossen wird (QA-013 AC1); sonst AC1 als eigenes Issue im Milestone M2 nachziehen.",
      ]),
]

MAINTAINER_RULE = """## Vorgabe des Maintainers: Arbeitsstand vollständig in GitHub
Sobald die Planung eines Meilensteins steht, muss **seine gesamte Arbeit als Issues in GitHub sichtbar** sein, damit der Arbeitsstand jederzeit nachvollziehbar ist:
- Jedes Feature des Meilensteins ist genau einem Arbeitspaket-Issue zugeordnet (Checkliste mit Feature-ID und Spec-Link); vor dem Anlegen per Skript prüfen, dass nichts fehlt und nichts doppelt ist.
- Zusätzliche Arbeit, die beim Umsetzen entsteht (Nacharbeiten, Spec-Korrekturen, Bugs, Folge-Aufgaben), wird ebenfalls als Issue im passenden Milestone angelegt – nicht nur im PR-Text erwähnt.
- Checklisten in den Issues werden beim Umsetzen abgehakt, Issues per PR (`Closes #…`) geschlossen; der Milestone-Fortschritt spiegelt so den echten Stand.
- Diese Vorgabe wird in jedes folgende Planungs-Issue („Mx-Abschluss & Planung Mx+1“) wörtlich übernommen.
"""

PLAN = dict(key="WP-50", title="M2-Abschluss & Planung M3", labels=["type:planning"], deps=["WP-38", "WP-47", "WP-48", "WP-49"], features=[],
      goal="M2 formal abschließen und die Arbeitspakete für M3 schneiden (rollierende Planung).",
      notes=[
        "Prüfen: alle M2-Issues geschlossen (inklusive #73, #92, #101 und des `human-required`-Issues zum Code-Owner-Review), Demo-Szenario (Milestone-Beschreibung) erfüllt – E2E aus WP-49 auf macOS und Linux grün, Escape-Suite (WP-48) als Required Check grün, manuelle Verifikation mit echten `claude`- und `codex`-CLIs protokolliert –, `cargo xtask spec-coverage --milestone M2` ohne fehlende Must-ACs, `docs/spec/roadmap.md` (`python3 scripts/gen_roadmap.py`) und der Feature-Katalog (`python3 scripts/gen_feature_catalog.py`) aktuell.",
        "Erkenntnisse aus M2 in Spec/ADRs zurückführen (z. B. verifizierte Annahmen zu `CLAUDE_CODE_SHELL_PREFIX`, dem Codex-Shell-Shim, den Vendor-Passthrough-Hosts, den Codex-Rate-Limit-Notifications und der Landlock-ABI-Matrix der CI; die Klärungen aus den M2-Paketen wie Force-Push im Demo, `cost_source: catalog`, Passthrough bei `auth: api_key`; neue ADRs statt stiller Änderungen).",
        "M3-Features (`docs/spec/roadmap.md` → M3) in Arbeitspakete mit 3–8 Features schneiden – jedes M3-Feature genau einem Paket zugeordnet – und mit `scripts/create_milestone_issues.py` (MS und Paketdefinitionen anpassen) als Issues im Milestone `M3 — Desktop & TUI (Release 0.1)` anlegen: gleiche Struktur wie die M2-Issues, Labels, „blocked by“-Beziehungen, plus ein Issue „M3-Abschluss & Planung M4“. Pakete in `beton-policy`, `beton-sandbox`, `beton-proxy`, `beton-secrets`, Auth/Sharing sowie Release-, Signing- und Update-Pipeline bekommen `security-review` und werden strikt per TDD umgesetzt.",
        "Übernahmen in die M3-Pakete: die „(ab M3)“-ACs früherer Features (`grep -rn \"(ab M3\" docs/spec/*.md`), #74 (M3 · Übernahmen aus M0), die offenen Issues im Milestone M3 und alle Übernahmen, die in den M2-Issues entstanden sind (z. B. Desktop-Badge der Inbox UX-001 AC3 und TUI-Anzeige USE-008 AC3, falls verschoben).",
        "Stand der `human-required`-Issues #54 (Signing-Accounts), #55 (Domain) und #56 (Markenrecht) prüfen – vor M3 fällig, Gate für das signierte Desktop-Paket und Release 0.1.",
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
