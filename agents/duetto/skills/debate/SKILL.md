---
name: debate
description: "Debatte: /debate [rounds=N] <Frage> – zwei Stimmen antworten, kritisieren sich N Runden lang gegenseitig (Default 2, höchstens 5), dann Synthese mit Konsens und Dissens (duetto)."
user-invocable: true
---

# Debatte

Aufruf: `/debate rounds=N <Frage>`. Ohne `rounds` gilt die Standardzahl aus deinen
Anweisungen (2). Werte über 5 setzt du auf 5, Werte unter 1 auf 1, und sagst das.

1. **Vorab-Prüfung** wie in deinen Anweisungen: ohne beide Stimmen kein Start.
2. **Antworten:** Beide Stimmen bekommen die Frage parallel (`session_spawn`, `async: true`);
   warte mit `session_wait(mode: all, timeout: "50s")` bis `done: true`.
3. **Kritikrunden r = 1 … N.** In jeder Runde, für beide Stimmen gleichzeitig:
   `session_send(session_id: <Stimme>, text: …)` mit der **letzten Antwort der jeweils
   anderen Stimme** und dem Auftrag: „Runde r von N: Kritisiere diese Antwort sachlich (was
   stimmt, was fehlt, was ist falsch – mit Begründung) und gib danach deine überarbeitete
   Position an.“ Dann `session_wait(mode: all, timeout: "50s")` bis `done: true`.
   Genau N Runden, nicht mehr und nicht weniger; jede Stimme kritisiert in jeder Runde einmal.
4. **Synthese** (von dir, ohne weitere Stimme):

   ```
   ## Konsens
   - Punkte, die beide Stimmen am Ende vertreten.

   ## Dissens
   - Punkte, bei denen sie uneins bleiben, je mit beiden Positionen und deiner Einschätzung,
     woran es hängt.

   ## Urteil
   Deine Empfehlung in 2–5 Sätzen.
   ```

Vermerke unter der Synthese: „N Kritikrunden je Stimme · Claude Code und Codex“.
