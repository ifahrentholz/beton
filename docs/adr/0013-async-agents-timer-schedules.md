# ADR-0013: Async-Agents, Timer und Schedules in v1; Webhooks nur via API; Approval ohne Zuschauer

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Agents sollen auch ohne angeschlossenen Client arbeiten: vom Parent asynchron gestartet, zeitgesteuert (Timer, Cron) oder extern ausgelöst. Omnigent bietet Async-Dispatch-Tools, Timer und Scheduled Tasks (RRULE, serverseitiger Scheduler, keine Backfills). Offene Frage ist, was passiert, wenn eine Policy `ask` verlangt, aber niemand zusieht – und wie Schedules im Lokal-only-Betrieb laufen.

## Betrachtete Optionen
1. **A — Autonomie erst in v2** — kleinerer v1-Umfang; Kernnutzen (unbeaufsichtigte Agents) fehlt.
2. **B — Async-Agents, Timer, Schedules in v1; Webhooks nur über die API** — volle Autonomie ohne Integrationsbreite.
3. **C — Zusätzlich eingebaute Event-Empfänger (GitHub-Webhooks etc.)** — komfortabel; viele Integrationen, Angriffsfläche.
4. Approval ohne Zuschauer: **auto-deny sofort** vs. **auto-allow** vs. **Pause + Timeout mit konfigurierbarem Verhalten**.

## Entscheidung
Option **B** (Meilenstein M5) mit **Pause + Timeout, Default deny**:
- **Async-Agent** = Session ohne angeschlossenen Client. Start via `spawn(async: true)` durch einen Parent (Ergebnis kommt als Event zurück), per Schedule (Cron), Timer ("in 2h") oder Webhook-/API-Aufruf.
- **Webhooks in v1 nur über die API** – kein eingebauter GitHub-Event-Empfänger.
- **Lokal-only:** Der Daemon führt Schedules aus, solange der Rechner läuft; verpasste Läufe per `catch_up: run_once | skip`; optional `keep_awake`.
- **Zentral:** Der Server dispatcht an online Runner mit passenden Labels (`os=linux`, `repo=x`); später Cloud-Sandbox.
- **`ask` ohne Zuschauer:** Push-Notification (Desktop/Mobile) + Approval-Card in der Inbox; die Session **pausiert** mit Timeout; `on_timeout: deny | allow | abort`, **Default `deny`**.

## Konsequenzen
- Positiv: Unbeaufsichtigte Agents mit sicherem Default; Lokal-only bleibt voll funktionsfähig.
- Positiv: Webhook-Integration bleibt generisch (jeder Dienst kann die API aufrufen).
- Negativ / Risiken: Lokale Schedules hängen am laufenden Rechner; `keep_awake` plattformabhängig.
- Negativ / Risiken: Pausierte Sessions binden Ressourcen (Runner, Worktree) bis zum Timeout.
- Folgearbeiten: Scheduler-Leader-Election bei mehreren Server-Instanzen, Kosten-Caps pro Schedule, Inbox-Modell (ADR-0021).
- Verschärft durch ADR-0033: Benachrichtigung ohne Zuschauer standardmäßig lokal (Desktop-Notification, Inbox); Web-Push nur opt-in.

## Bezug
- Spec: docs/spec/02-agents.md (Prefix ASY), docs/spec/03-policies.md (Prefix POL)
