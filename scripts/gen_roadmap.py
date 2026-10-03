#!/usr/bin/env python3
"""Generiert docs/spec/roadmap.md aus den Feature-Einträgen der Spec."""
import re, glob, os, collections

SPEC = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "docs", "spec")
os.chdir(SPEC)

feats = []  # (id, title, ms, prio, file)
for f in sorted(glob.glob("[0-9][0-9]-*.md")):
    cur = None
    for line in open(f, encoding="utf-8"):
        m = re.match(r"###\s+([A-Z]+-\d{3})\s+—\s+(.*)", line)
        if m:
            cur = [m.group(1), m.group(2).strip(), None, None, f]
            feats.append(cur)
            continue
        if cur and cur[2] is None:
            m2 = re.search(r"\*\*Meilenstein:\*\*\s*(M\d|v2)", line)
            if m2:
                cur[2] = m2.group(1)
                p = re.search(r"\*\*Priorität:\*\*\s*(Must|Should|Could)", line)
                cur[3] = p.group(1) if p else "?"

chapter_titles = {}
for f in sorted(glob.glob("[0-9][0-9]-*.md")):
    first = open(f, encoding="utf-8").readline().strip().lstrip("# ").strip()
    chapter_titles[f] = first

MS = {
    "M0": ("Fundament", "`beton run claude` startet eine persistente Claude-Code-Session (Subscription über die offizielle CLI), die im Browser live mitläuft, nach einem Verbindungsabbruch ab `seq` fortgesetzt wird und einen Neustart des Daemons überlebt.",
           ["Cargo-Workspace, Event-Modell und Wire-Protokoll, SQLite-Event-Log", "Lokaler Daemon (`serve` + lokaler Runner), CLI-Grundgerüst", "Claude-Code-Adapter (stream-json) inkl. Permission-Bridge", "Minimale Web-UI (Session-Liste, Chat-Stream, Composer)", "Test-Fundament: Fake-Harness, Golden Transcripts, Schema-Snapshots, CI-Gates, Offline-E2E ohne Netzwerk (QA-018)"]),
    "M1": ("Meta-Harness", "Eine Session startet auf Claude Code, wird auf Codex geforkt und weitergeführt; `maestra` lässt Claude implementieren und Codex reviewen; vorhandene Claude- und Codex-Chats lassen sich importieren.",
           ["Codex-Adapter (app-server), generisches ACP, Direkt-API-/Gateway-Harness", "Fork über Harness-Grenzen mit Handover-Kontext, Import, Export", "Agent-YAML + JSON-Schema, MCP als Tool-Mechanismus, System-Tools, Skills, Sub-Agents", "Built-in-Agents `maestra` und `duetto`"]),
    "M2": ("Kontrolle", "Ein Agent läuft im YOLO-Mode in der Sandbox: Er kann `~/.ssh` nicht lesen und nur erlaubte Hosts erreichen, er sieht Secrets nur als `bt_cred_*`, `git push --force` erzeugt eine Approval-Card, und bei Überschreitung des Budgets wird gestoppt.",
           ["CEL-Policies mit Hierarchie (User → Projekt → Agent), Regeltypen, Approvals, Policy-Tests, Explain", "Usage/Kosten inkl. Subscription-Usage, Inbox", "Sandbox macOS (Seatbelt) + Linux (Landlock/seccomp), zweistufig, Escape-Suite in der CI", "Egress- und Credential-Proxy, lokaler Secret-Store, Audit, Redaction"]),
    "M3": ("Desktop & TUI → öffentliches Release 0.1", "Ein neuer Nutzer installiert die signierte Desktop-App, `beton setup` erkennt die CLI-Logins, und er arbeitet mit mehreren Sessions in Worktrees, nutzt den Inspect-Mode im eingebetteten Browser und diktiert Prompts per Push-to-Talk – vollständig lokal.",
           ["Tauri-2-Desktop-App, ratatui-TUI, Native-TUI/PTY-Modus mit Hooks", "Projects, Worktrees, Terminals", "Eingebetteter Browser (CDP-Screencast), Agent-Tools, Inspect-Mode", "Lokales Whisper, Command-Palette, Themes, Onboarding", "Release-Pipeline: Signing, Notarisierung, Homebrew, Installer, Auto-Update"]),
    "M4": ("Team", "Zwei Personen melden sich per OIDC an einem zentralen Server an, teilen eine Session, steuern sie gemeinsam, kommentieren inline, verfolgen den PR/MR im Panel; ein Laptop arbeitet offline mit einer Budget-Lease weiter und synchronisiert danach.",
           ["Zentraler Server mit Postgres + S3, OIDC, Device-Pairing, PATs, Rollen", "Sharing, Co-Drive, Presence, Inline-Kommentare, Side-Chats", "Sync (Single-Writer, Ownership, Fork bei Divergenz), Policy-Cache, Budget-Leases", "Remote-Hosts, PWA + Web-Push, GitHub-/GitLab-Panel, Envelope-Encryption"]),
    "M5": ("Autonomie & Breite → v1.0", "Ein nächtlicher Schedule startet auf einem Kubernetes-Runner einen Async-Agent, der Dependencies aktualisiert und einen PR öffnet; eine `ask`-Policy pausiert ihn bis zur Freigabe vom Handy; ein Community-Plugin wird signiert installiert; Windows-Nutzer arbeiten mit der Beta-Sandbox.",
           ["Async-Agents, Timer, Schedules, Webhook-API, Approval ohne Zuschauer", "Docker/Podman- und Kubernetes-Runner, Runner-Image, Reaper", "Out-of-Process-Plugins mit Registry und Signaturen", "Windows-Beta (Sandbox, Desktop), winget/Scoop/deb/rpm, Helm"]),
}

by_ms = collections.defaultdict(list)
for fid, title, ms, prio, f in feats:
    by_ms[ms].append((fid, title, prio, f))

out = []
out.append("# Roadmap\n")
out.append("> Generiert aus den Feature-Einträgen in `docs/spec/*.md`. Maßgeblich ist immer der Meilenstein **im Feature-Eintrag**; diese Datei ist eine Sicht darauf. Neu generieren: `python3 scripts/gen_roadmap.py`.\n")
out.append("Grundlage: ADR-0030 (Meilensteine M0–M5, erstes öffentliches Release nach M3, Solo-Entwicklung mit Coding-Agents).\n")
out.append("## Übersicht\n")
out.append("| Meilenstein | Thema | Must | Should | Could | Gesamt |")
out.append("|---|---|---:|---:|---:|---:|")
tot = collections.Counter()
for ms in ["M0", "M1", "M2", "M3", "M4", "M5"]:
    c = collections.Counter(p for _, _, p, _ in by_ms[ms])
    n = len(by_ms[ms]); tot.update(c); tot["all"] += n
    out.append(f"| [{ms}](#{ms.lower()}) | {MS[ms][0]} | {c['Must']} | {c['Should']} | {c['Could']} | {n} |")
out.append(f"| **Summe v1.0** | | **{tot['Must']}** | **{tot['Should']}** | **{tot['Could']}** | **{tot['all']}** |\n")
out.append("**Arbeitsweise:** Innerhalb eines Meilensteins zuerst alle *Must*-Features (in Abhängigkeitsreihenfolge, siehe Feld *Abhängigkeiten*), dann *Should*. *Could*-Features sind Stretch-Goals und blockieren den Meilenstein nicht. Ein Meilenstein gilt als erreicht, wenn alle *Must*-Features grün sind **und** das Demo-Szenario manuell (und so weit möglich als E2E-Test) funktioniert.\n")

for ms in ["M0", "M1", "M2", "M3", "M4", "M5"]:
    title, demo, scope = MS[ms]
    out.append(f"## {ms}\n")
    out.append(f"### {ms} — {title}\n")
    out.append("**Umfang**\n")
    for s in scope:
        out.append(f"- {s}")
    out.append("")
    out.append(f"**Demo-Szenario (Exit-Kriterium):** {demo}\n")
    out.append("<details><summary>Features ({})</summary>\n".format(len(by_ms[ms])))
    grouped = collections.defaultdict(list)
    for fid, t, p, f in by_ms[ms]:
        grouped[f].append((fid, t, p))
    for f in sorted(grouped):
        out.append(f"\n**[{chapter_titles[f]}]({f})**\n")
        out.append("| ID | Feature | Priorität |")
        out.append("|---|---|---|")
        for fid, t, p in grouped[f]:
            out.append(f"| {fid} | {t} | {p} |")
    out.append("\n</details>\n")

out.append("## v2\n")
out.append("Bewusst **nicht** in v1.0 (siehe ADRs und die Abschnitte \"Nicht in v1\" der Kapitel):\n")
for item in [
    "Smart Routing („Auto“-Harness- und Modellwahl, lernender Router) — ADR-0022",
    "Native Mobile-Apps (Tauri Mobile) — ADR-0015",
    "Slack-Bot, VS-Code-Extension — ADR-0015",
    "UI-Extensions (sandboxed iframe + Message-Bridge) — ADR-0018",
    "Öffentliche (login-freie) Share-Links, Canvas-Ansicht — ADR-0014",
    "SaaS-Sandbox-Provider (E2B, Daytona, Modal, Fly) und MicroVMs als Community-Provider — ADR-0017",
    "Vault-/KMS-Backends für den Master-Key (als Plugin) — ADR-0024",
    "Branding/White-Labeling — ADR-0021",
    "Prompt-Cleanup nach Diktat — ADR-0023",
    "Python-SDK, Flatpak, AUR/Nix, SCIM",
]:
    out.append(f"- {item}")
if by_ms.get("v2"):
    out.append("\nAls Feature-Einträge mit Meilenstein `v2` spezifiziert:\n")
    for fid, t, p, f in by_ms["v2"]:
        out.append(f"- {fid} — {t} ([{f}]({f}))")
out.append("")
open("roadmap.md", "w", encoding="utf-8").write("\n".join(out))
print("features:", len(feats), {k: len(v) for k, v in by_ms.items()})
missing = [x for x in feats if x[2] is None]
print("missing ms:", missing)
