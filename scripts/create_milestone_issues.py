#!/usr/bin/env python3
"""Legt Milestones, Labels und die Arbeitspaket-Issues eines Meilensteins auf GitHub an.

Rollierende Planung (siehe Issue #17 und AGENTS.md): Pro Meilenstein werden die
Arbeitspakete (WPS), ggf. Sonder-Issues (LEGAL) und das Abschluss-/Planungs-Issue
(PLAN) unten definiert. Für den nächsten Meilenstein `MS` umstellen und WPS/LEGAL/PLAN
ersetzen. Die Definitionen von M0 bleiben als Vorlage in der Git-History.

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
MS = "M0"  # Meilenstein, für den Issues angelegt werden
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
    "area:legal": ("c5def5", "Lizenzen, Nutzungsbedingungen"),
}

# ---------- Arbeitspakete M0 ----------
WPS = [
 dict(key="WP-01", title="Repo-Bootstrap, Workspace & CI-Gates", labels=["area:core", "area:qa"], deps=[],
      features=["QA-001", "QA-010", "QA-014", "QA-015", "PLG-001", "OBS-001"],
      goal="Lauffähiges Gerüst, auf dem alle weiteren Pakete aufsetzen: Cargo-Workspace, CI, Lint/Test-Gates, Logging, Agent-Workflow.",
      notes=[
        "Cargo-Workspace gemäß `docs/spec/00-overview.md` §4.4 anlegen – **nur die in M0 benötigten Crates** als Gerüst: `beton-cli`, `beton-core`, `beton-proto`, `beton-store`, `beton-server`, `beton-host`, `beton-runner`, `beton-harness`, `beton-harness-claude`, `beton-sdk`, `beton-fake-cli`. Weitere Crates erst mit ihrem Meilenstein.",
        "`LICENSE` mit dem vollständigen Apache-2.0-Text anlegen (README verweist darauf).",
        "`rust-toolchain.toml` (stable), `cargo-deny`-Konfiguration (Apache-2.0-kompatible Lizenzen), `nextest`, `clippy -D warnings`, `rustfmt`.",
        "GitHub Actions: Linux + macOS (Build, Lint, Tests), Windows nur Build-Check; DCO-Check (QA-015); PR-Template mit Feld für Feature-IDs und `Closes #…`.",
        "`cargo xtask`-Gerüst (Ziel: `codegen`, ab WP-02 genutzt).",
        "`AGENTS.md` §3 (Befehle) mit den echten Kommandos füllen und einen Abschnitt **Arbeitsablauf mit GitHub-Issues** ergänzen: Issue nur starten, wenn alle Blocker geschlossen sind → Branch `wp-NN-kurzname` → PR mit `Closes #…` und Feature-IDs in Commits → Label `security-review` = Merge erst nach Maintainer-Freigabe → Label `human-required` = nicht bearbeiten.",
      ]),
 dict(key="WP-02", title="Event-Modell, IDs & Schema-Generierung", labels=["area:protocol"], deps=["WP-01"],
      features=["PROTO-001", "PROTO-002", "PROTO-003", "PROTO-013", "QA-006"],
      goal="Das harness-neutrale Event-Modell als Rust-Typen (Single Source of Truth) inklusive generierter JSON-Schemas und TypeScript-Typen.",
      notes=[
        "Alle Event-Typen des Katalogs PROTO-002 existieren ab M0 als Rust-Varianten mit Schema – auch die, deren Producer erst später kommen.",
        "Generierung über `cargo xtask codegen`: JSON-Schema (`schemars`), TS-Typen (`ts-rs` oder `specta`) nach `packages/sdk-ts/src/generated/`. Generierte Dateien nie von Hand ändern.",
        "Snapshot-Tests der Schemas (z. B. `insta`), damit jede Protokolländerung im PR sichtbar wird.",
      ]),
 dict(key="WP-03", title="Persistenz: SQLite-Event-Log, Projektionen & Blob-Store", labels=["area:store"], deps=["WP-02"],
      features=["DATA-001", "DATA-002", "DATA-003", "DATA-005", "DATA-006", "DATA-008"],
      goal="Append-only-Event-Log mit lückenloser `seq`, Read-Modellen und Dateisystem-Blob-Store auf SQLite.",
      notes=[
        "`sqlx` mit Migrationen; Repository-Schicht so schneiden, dass Postgres (DATA-004, M4) später ohne Umbau dazukommt.",
        "Optimistisches Anhängen über `head_seq`/`epoch` wie in DATA-002 beschrieben; Nebenläufigkeitstest mit parallelen Writern.",
      ]),
 dict(key="WP-04", title="Server-Grundgerüst: REST, OpenAPI & lokale Sicherheit", labels=["area:server", "area:security", "security-review"], deps=["WP-02", "WP-03"],
      features=["PROTO-010", "PROTO-011", "API-001", "API-002", "AUTH-001", "AUTH-002", "AUTH-003", "AUTH-004"],
      goal="`beton-server` (axum) mit REST-Konventionen, RFC-9457-Fehlern, generierter OpenAPI 3.1 und abgesichertem lokalen Modus.",
      notes=[
        "Sicherheitskritisch (AGENTS.md §4/§5): **strikt TDD**, Merge erst nach Maintainer-Freigabe.",
        "Lokaler Modus: Bindung nur an Loopback, Token-Datei `0600`, Host-Header-Prüfung gegen DNS-Rebinding, CSRF-Schutz, WS-Origin-Allowlist, Einmal-Link für den Browser-Login.",
        "OpenAPI wird aus dem Code erzeugt (`utoipa`) und per Snapshot-Test (QA-006) abgesichert.",
      ]),
 dict(key="WP-05", title="WebSocket-Protokoll: Handshake, Resume, Backpressure", labels=["area:protocol", "area:server"], deps=["WP-04"],
      features=["PROTO-004", "PROTO-005", "PROTO-006", "PROTO-008", "PROTO-009"],
      goal="Bidirektionaler WebSocket mit Versionsaushandlung, Attach/Resume ab `seq`, Client-Kommandos, Batching und Heartbeats.",
      notes=[
        "Resume ab `seq` ist ein Kernversprechen gegenüber Omnigent (SSE ohne Replay) – Tests mit erzwungenen Verbindungsabbrüchen.",
        "Binärkanäle (PROTO-007) gehören zu M3, hier nur so vorbereiten, dass das Framing sie später aufnehmen kann.",
      ]),
 dict(key="WP-06", title="Runner, lokaler RunnerProvider & Tunnel", labels=["area:runner"], deps=["WP-02", "WP-04"],
      features=["RUN-001", "RUN-002", "RUN-003", "PROTO-015"],
      goal="`beton-host`/`beton-runner`: RunnerProvider-Trait, lokaler Provider, Runner-Lebenszyklus und das Tunnel-Protokoll Host/Runner → Server (lokal wie später remote).",
      notes=[
        "Auch lokal läuft die Kommunikation über das Tunnel-Protokoll – damit ist Remote (M4) nur noch Konfiguration (ADR-0003).",
        "Supervision: Absturz des Harness-Prozesses → `harness.exited`-Event, Session-Status `failed`.",
      ]),
 dict(key="WP-07", title="Harness-Abstraktion, Fake-Harness & Golden Transcripts", labels=["area:harness", "area:qa"], deps=["WP-02"],
      features=["HAR-001", "HAR-002", "HAR-003", "HAR-025", "HAR-026", "QA-002", "QA-003"],
      goal="Adapter-Trait mit Capabilities und Registry sowie die Test-Infrastruktur, die echte Vendor-CLIs in CI überflüssig macht.",
      notes=[
        "`beton-fake-cli`: deterministisches Binary, das das stream-json-Protokoll von Claude Code simuliert (Szenario-Dateien). Codex/ACP-Simulation folgt in M1.",
        "Golden-Transcript-Framework inkl. `beton dev record-golden` und Drift-Prozess (QA-003). Aufnahmen echter CLIs passieren in WP-08.",
        "Fake-Harness ist auch der Harness `fake` für `beton run --scenario` (E2E in WP-15).",
      ]),
 dict(key="WP-08", title="Claude-Code-Adapter (stream-json) inkl. Permission-Bridge", labels=["area:harness", "needs-vendor-cli"], deps=["WP-06", "WP-07"],
      features=["HAR-004", "HAR-005", "HAR-015", "HAR-021"],
      goal="Claude Code über das native stream-json-Protokoll anbinden – mit Subscription über die offizielle CLI, Approval-Rückfragen und Usage-Reporting.",
      notes=[
        "**Erster Schritt:** alle in 01-harnesses.md als *(Annahme)* markierten Flags/Nachrichtenformate gegen die aktuelle `claude`-CLI verifizieren, Abweichungen in der Spec korrigieren (im selben PR, klar markiert) und Golden Transcripts aufnehmen.",
        "**Subscription-Regel (ADR-0005, HAR-015):** beton liest, speichert oder verwendet niemals OAuth-/Subscription-Tokens der CLI. Kein eigener Login.",
        "Approval-Rückfragen gehen in M0 an die minimale Approval-Karte (WEB-018) bzw. den CLI-Prompt; die Policy-Engine kommt erst in M2.",
      ]),
 dict(key="WP-09", title="Session-Lebenszyklus, Live-Stream & Interrupt", labels=["area:server", "area:runner"], deps=["WP-03", "WP-05", "WP-06", "WP-07"],
      features=["SES-001", "SES-002", "SES-003", "SES-005"],
      goal="Sessions anlegen, mehreren Clients live streamen, nach Stopp/Neustart fortsetzen und unterbrechen.",
      notes=["Tests durchgängig mit dem Fake-Harness; Resume über Daemon-Neustart hinweg ist Teil des M0-Demo-Szenarios."]),
 dict(key="WP-10", title="CLI-Grundgerüst, `serve`/`host` & `config`", labels=["area:cli"], deps=["WP-04", "WP-06"],
      features=["CLI-001", "CLI-004", "CLI-008"],
      goal="Binary `beton` mit Kommandobaum, Ausgabekonventionen und Exit-Codes; lokaler Daemon über `serve`/`host`; Konfigurationsverwaltung.",
      notes=["Kommandobaum verbindlich laut 08-clients.md (Design-Abschnitt); Kommandos späterer Meilensteine noch nicht anlegen."]),
 dict(key="WP-11", title="CLI: `run`, `resume`, `attach` & Skript-Modus", labels=["area:cli"], deps=["WP-08", "WP-09", "WP-10"],
      features=["CLI-002", "CLI-003", "API-006"],
      goal="Sessions aus dem Terminal starten, fortsetzen, anhängen und nicht-interaktiv für Skripte nutzen.",
      notes=["`beton run` auf TTY beantwortet Approvals per `[y/N]` (Enter = Nein); ohne TTY gilt `--on-ask` (Default `wait`)."]),
 dict(key="WP-12", title="`beton setup` & `beton doctor`", labels=["area:cli", "area:harness"], deps=["WP-07", "WP-10"],
      features=["CLI-005", "HAR-016", "OBS-005"],
      goal="Erstkonfiguration mit Erkennung von CLI-Logins und Umgebungsprüfung.",
      notes=["`setup` **bietet** die Installation fehlender CLIs an, installiert nie still; `--non-interactive` installiert nie etwas.", "Login-Erkennung ohne Token-Inhalte zu lesen (nur Existenz/Status über die CLI selbst)."]),
 dict(key="WP-13", title="SDKs: TypeScript & Rust", labels=["area:sdk"], deps=["WP-04", "WP-05"],
      features=["API-004", "API-005"],
      goal="Client-SDKs auf Basis der generierten Typen, die Web-UI, CLI und Tests nutzen.",
      notes=["TS-SDK `@ifahrentholz/beton-sdk` unter `packages/sdk-ts`, Typen ausschließlich generiert (WP-02)."]),
 dict(key="WP-14", title="Web-UI M0: Shell, Session-Liste, Chat-Stream, Composer, Approval-Karte", labels=["area:web"], deps=["WP-09", "WP-13"],
      features=["WEB-001", "WEB-002", "WEB-003", "WEB-004", "WEB-015", "WEB-018"],
      goal="Minimale, schnelle Web-UI, in der eine Claude-Session live mitläuft und Approvals beantwortet werden.",
      notes=["Stack laut ADR-0020: React 19, TypeScript, Vite, TanStack Router/Query, Tailwind, shadcn/ui, Zustand.", "Performance-Budgets (WEB-015) werden in CI mit Playwright + Fake-Harness gemessen."]),
 dict(key="WP-15", title="E2E-Tests & M0-Demo-Szenario", labels=["area:qa", "area:web"], deps=["WP-08", "WP-11", "WP-14"],
      features=["QA-007"],
      goal="Das M0-Demo-Szenario als automatisierter E2E-Test (mit Fake-Harness) plus dokumentierte manuelle Verifikation mit echter Claude-CLI.",
      notes=[
        "Demo: `beton run claude` → Session läuft im Browser live mit → Verbindungsabbruch → Resume ab `seq` → Daemon-Neustart → Session ist noch da und lässt sich fortsetzen.",
        "Manuelle Verifikation mit echter, eingeloggter `claude`-CLI als Checkliste im PR dokumentieren.",
      ]),
]

LEGAL = dict(key="LEGAL", title="Nutzungsbedingungen zur Subscription-Nutzung durch Drittwerkzeuge prüfen", labels=["human-required", "area:legal"], deps=[], features=[],
      goal="Offener Punkt 4 aus `docs/spec/00-overview.md` §9 und ADR-0005: Klären, ob das Starten der offiziellen `claude`- bzw. `codex`-CLI mit deren eigener Anmeldung durch beton mit den aktuellen Nutzungsbedingungen von Anthropic und OpenAI vereinbar ist (Hinweis in der Claude-Agent-SDK-Doku zu Third-Party-claude.ai-Logins).",
      notes=[
        "Aktuellen Wortlaut der relevanten Bedingungen/Doku sichten und Ergebnis als Kommentar bzw. ADR-Ergänzung festhalten.",
        "Bis zur Klärung: keine öffentliche Werbung mit Subscription-Nutzung; Repo bleibt privat.",
        "Gate für den M0-Abschluss (WP-16) und Voraussetzung für jede Veröffentlichung.",
      ])

MAINTAINER_RULE = """## Vorgabe des Maintainers: Arbeitsstand vollständig in GitHub
Sobald die Planung eines Meilensteins steht, muss **seine gesamte Arbeit als Issues in GitHub sichtbar** sein, damit der Arbeitsstand jederzeit nachvollziehbar ist:
- Jedes Feature des Meilensteins ist genau einem Arbeitspaket-Issue zugeordnet (Checkliste mit Feature-ID und Spec-Link); vor dem Anlegen per Skript prüfen, dass nichts fehlt und nichts doppelt ist.
- Zusätzliche Arbeit, die beim Umsetzen entsteht (Nacharbeiten, Spec-Korrekturen, Bugs, Folge-Aufgaben), wird ebenfalls als Issue im passenden Milestone angelegt – nicht nur im PR-Text erwähnt.
- Checklisten in den Issues werden beim Umsetzen abgehakt, Issues per PR (`Closes #…`) geschlossen; der Milestone-Fortschritt spiegelt so den echten Stand.
- Diese Vorgabe wird in jedes folgende Planungs-Issue („Mx-Abschluss & Planung Mx+1“) wörtlich übernommen.
"""

PLAN = dict(key="WP-16", title="M0-Abschluss & Planung M1", labels=["type:planning"], deps=["WP-12", "WP-15", "LEGAL"], features=[],
      goal="M0 formal abschließen und die Arbeitspakete für M1 schneiden (rollierende Planung).",
      notes=[
        "Prüfen: alle M0-Issues geschlossen, Demo-Szenario (Milestone-Beschreibung) erfüllt, `docs/spec/roadmap.md` mit `python3 scripts/gen_roadmap.py` aktuell.",
        "Erkenntnisse aus M0 in Spec/ADRs zurückführen (neue ADRs statt stiller Änderungen).",
        "M1-Features (`docs/spec/roadmap.md` → M1) in Arbeitspakete mit 3–8 Features schneiden – jedes M1-Feature genau einem Paket zugeordnet – und mit `scripts/create_milestone_issues.py` (MS und Paketdefinitionen anpassen) als Issues im Milestone `M1 — Meta-Harness` anlegen: gleiche Struktur wie die M0-Issues, Labels, „blocked by“-Beziehungen, plus ein Issue „M1-Abschluss & Planung M2“.",
        "Offene Punkte aus 00-overview §9 prüfen, die vor M3 fällig werden (Signing-Accounts, Domain, Markenrecht) und dafür `human-required`-Issues anlegen.",
      ],
      extra=MAINTAINER_RULE)

ALL = WPS + [LEGAL, PLAN]

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
m0 = {k for k, v in feats.items() if v.get("ms") == MS}
assigned = [f for wp in ALL for f in wp["features"]]
dups = {f for f in assigned if assigned.count(f) > 1}
missing = m0 - set(assigned)
wrong = set(assigned) - m0
print(f"{MS} features: {len(m0)} · assigned: {len(assigned)} · missing: {sorted(missing)} · dup: {sorted(dups)} · not-M0: {sorted(wrong)}")
if missing or dups or wrong:
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
order = [LEGAL] + WPS + [PLAN]
num_of, id_of = {}, {}
existing_titles = {} if DRY else {i["title"]: i["number"] for i in json.loads(
    gh(["issue", "list", "--repo", REPO, "--state", "all", "--limit", "1000", "--json", "title,number"]))}
for wp in order:
    title = f"{MS} · {wp['key']} · {wp['title']}" if wp["key"].startswith("WP-") else f"{MS} · {wp['title']}"
    if title in existing_titles:
        print("skip (exists):", title)
        num_of[wp["key"]] = existing_titles[title]
        id_of[wp["key"]] = json.loads(gh(["api", f"repos/{REPO}/issues/{existing_titles[title]}"]))["id"] if not DRY else 0
        continue
    labels = list(wp["labels"])
    if wp["key"].startswith("WP-") and "type:planning" not in labels:
        labels.insert(0, "type:work-package")
    ms_title = next(t for t, _ in MILESTONES if t.startswith(MS + " "))
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
