# ADR-0014: Collaboration-Scope v1

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Echtzeit-Collaboration auf derselben Live-Session ist ein Kernversprechen. Omnigent bietet Live-Sync, Sharing (Read/Edit), Co-Drive, Fork (auch auf anderen Harness), Side-Chats, Queue/Steer, Inline-Kommentare, Projects, Worktrees, Import fremder Chats, ein GitHub-PR-Panel sowie öffentliche Links und eine Canvas-Ansicht. Für v1 muss der Umfang festgelegt werden – insbesondere welche Teile Team-Features (M4) sind.

## Betrachtete Optionen
1. **A — Minimal (Live-Stream + Teilen)** — schnell; verfehlt das Team-Versprechen.
2. **B — Breiter Kern inkl. Inline-Kommentare, Side-Chats, GitHub- + GitLab-Panel; öffentliche Links und Canvas in v2** — Team-tauglich, Fokus auf Arbeitsfluss.
3. **C — Volle Omnigent-Parität inkl. öffentlicher Links und Canvas** — maximaler Umfang; öffentliche Links erfordern zusätzliches Sicherheitsmodell, Canvas ist "nice to have".

## Entscheidung
Option **B**. v1 umfasst:
- Live-Stream an mehrere Clients; Teilen mit *view* / *approve* / *drive*; **Co-Drive** + Queue/Steer.
- **Fork** ab Event X, auch auf einen anderen Harness; **Resume/Import** von Claude-Code-/Codex-Chats.
- **Git-Worktrees pro Session**; **Projects** (Gruppierung + Policy-Ebene).
- **Inline-Kommentare** und **Side-Chats**.
- **GitHub-PR-Panel und GitLab-MR-Panel** über ein Git-Provider-Interface, inkl. GitHub Enterprise und self-hosted GitLab.

v2: **öffentliche (login-freie) Links**, **Canvas-Ansicht**.

## Konsequenzen
- Positiv: Vollwertige Team-Zusammenarbeit inkl. Code-Review-Workflow; GitLab-Nutzer nicht ausgeschlossen.
- Positiv: Kein login-freier Zugriff in v1 → kleinere Angriffsfläche.
- Negativ / Risiken: Zwei Git-Provider von Anfang an verdoppeln Integrations- und Testaufwand.
- Folgearbeiten: Git-Provider-Trait in `beton-git`, Kommentar-Events (`comment.added`), Side-Chat als Fork mit Markierung, Freigabe-Rechte in Policies abbilden.

## Bezug
- Spec: docs/spec/07-sessions-collaboration.md (Prefix SES/COL/GIT)
