# ADR-0030: Roadmap M0–M5, erstes öffentliches Release nach M3, Solo-Entwicklung mit Coding-Agents

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Der Gesamtumfang (Harnesses, Sandbox, Policies, Desktop, Team-Server, Autonomie) ist für einen Solo-Entwickler nur in klar geschnittenen, jeweils nutzbaren Inkrementen umsetzbar. Die Entwicklung erfolgt mit Coding-Agents, die präzise, meilensteinbezogene Arbeitspakete brauchen. Ein zu frühes öffentliches Release erzeugt Support-Last; ein zu spätes verschenkt Feedback.

## Betrachtete Optionen
1. **A — Früh veröffentlichen (nach M0/M1)** — schnelles Feedback; unfertige Sicherheit (keine Sandbox), hohe Support-Last.
2. **B — Release nach vollständigem Single-User-Produkt (M3), Team und Autonomie danach** — sicheres, rundes Erstprodukt; späteres Feedback.
3. **C — Release erst mit v1.0 (M5)** — vollständig; sehr spät, Motivations- und Relevanzrisiko.

## Entscheidung
Option **B**. Meilensteine:
- **M0 Fundament:** Cargo-Workspace, Event-Modell & Protokoll, SQLite-Event-Log, lokaler Daemon, CLI, Claude-Code-Adapter (stream-json), minimale Web-UI → `beton run claude` mit persistenter Session im Browser.
- **M1 Meta-Harness:** Codex-Adapter, generisches ACP, Direkt-API-Harness, Fork über Harness-Grenzen, Import, Agent-YAML + MCP + Skills, Sub-Agents → Claude ⇄ Codex im Wechsel, Cross-Vendor-Review.
- **M2 Kontrolle:** CEL-Policies, Kosten/Usage, Approvals + Inbox, Sandbox macOS/Linux, Egress-/Credential-Proxy, Secrets → sicherer YOLO-Mode lokal.
- **M3 Desktop & TUI:** Tauri-App, ratatui-TUI, PTY/Native-TUI-Modus, Worktrees, Projects, eingebetteter Browser + Inspect-Mode, Whisper → **erstes öffentliches Release 0.1**.
- **M4 Team:** Zentraler Server (Postgres), OIDC, Device-Pairing, Rollen, Sharing, Co-Drive, Inline-Kommentare, Side-Chats, Sync + Budget-Leases, PWA + Web-Push, GitHub-/GitLab-Panel.
- **M5 Autonomie & Breite:** Async-Agents, Timer, Schedules, Webhook-API, Docker-/K8s-Runner, Runner-Image, Plugin-System, Windows-Beta-Sandbox, Signing & Distribution aller Kanäle → **v1.0**.
- **v2** (explizit nicht v1): u. a. Smart Routing, White-Label, Slack, VS Code, native Mobile, UI-Extensions, öffentliche Links, Canvas, SaaS-Sandbox-Provider, MicroVMs, KMS/Vault-Plugins, Python-SDK, Flatpak, SCIM.
- **Entwicklungsmodell:** Solo-Entwickler mit Coding-Agents; Spec agent-tauglich (ADR-0031).

## Konsequenzen
- Positiv: Jeder Meilenstein liefert ein nutzbares Ergebnis; erstes Release ist sicher (Sandbox/Policies enthalten).
- Negativ / Risiken: Team-Features erst nach dem ersten Release; Gesamtumfang bleibt groß – Scope-Disziplin nötig.
- Folgearbeiten: Feature-IDs je Spec-Kapitel mit Meilenstein-Zuordnung; Roadmap-Dokument pflegen.

## Bezug
- Spec: docs/roadmap.md, docs/spec/00-overview.md
