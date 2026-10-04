---
name: fanout
description: Startet die Implementierungen eines Plans parallel, jede in eigenem Worktree, und sammelt die Ergebnisse ein (maestra).
user-invocable: true
---

# Fanout

1. Starte für jede Teilaufgabe den geplanten Implementer:

   ```
   session_spawn(agent: "impl-claude" | "impl-codex",
                 prompt: <Auftrag>,
                 async: true,
                 worktree: "new")
   ```

   Der Auftrag enthält: Titel, Auftrag, Akzeptanzkriterien und diese Regeln:
   „Arbeite nur in deinem Worktree. Committe auf den ausgecheckten Branch. Kein push, kein
   merge. Melde am Ende Branch, Commits, Tests und offene Punkte.“
2. Merke dir je Teilaufgabe `session_id`, `agent`, `harness`, `worktree.branch` und
   `worktree.base_sha` aus der Antwort.
3. Lehnt beton einen Start ab (`spawn_denied: max_concurrent`), warte mit `session_wait`
   (`mode: any`) und starte dann die nächste.
4. Warte mit `session_wait(mode: all, timeout: "50s")`, bis `done: true` ist.
5. `status: failed` oder `cancelled`: Teilaufgabe als `failed` bzw. `cancelled` vermerken,
   nicht reviewen. `status: completed`: weiter mit `cross-review`.
