---
name: cross-review
description: Lässt jede Implementierung von einem Agent eines anderen Vendors reviewen, beauftragt bei blockierenden Punkten nach, höchstens 3 Runden (maestra).
user-invocable: true
---

# Cross-Review

Für jede fertige Implementierung:

1. Wähle den Reviewer des **anderen** Vendors: `impl-claude` → `review-codex`,
   `impl-codex` → `review-claude`. Ist nur ein Vendor verfügbar, nimm den Reviewer desselben
   Vendors und vermerke „same-vendor review“.
2. Starte ihn (parallel für alle Teilaufgaben):

   ```
   session_spawn(agent: <Reviewer>, async: true, worktree: "new",
                 prompt: "Reviewe Branch <branch> (Base <base_sha>) für: <Titel>.
                          Akzeptanzkriterien: …  Runde <n> von 3.")
   ```
3. Warte mit `session_wait(mode: all, timeout: "50s")`, bis `done: true` ist.
4. Werte die erste Zeile der Antwort aus:
   - `VERDICT: approve` → Teilaufgabe `approved`.
   - `VERDICT: changes_requested` → gib die Punkte unter `BLOCKING:` mit
     `session_send(session_id: <Implementer>, text: …)` an **denselben** Implementer zurück
     („Behebe diese blockierenden Review-Punkte, committe, melde dich“), warte, und lass
     erneut reviewen (neue Runde, gleicher Reviewer-Agent).
   - Fehlt ein Verdict, frage den Reviewer einmal mit `session_send` nach dem Format.
5. Nach **3 Runden** mit weiterhin blockierenden Punkten endet die Teilaufgabe mit Status
   `needs_human`. Beauftrage nicht weiter; nenne die offenen Punkte in der Zusammenfassung.

Nicht-blockierende Punkte gibst du nicht zurück; sie stehen nur in der Zusammenfassung.
