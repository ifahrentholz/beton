---
name: plan
description: Zerlegt ein Ziel in unabhängige, prüfbare Teilaufgaben für parallele Implementierung (maestra).
user-invocable: true
---

# Plan

1. Verstehe das Ziel. Lies bei Bedarf die relevanten Dateien (nur lesen). Kläre nichts, was
   du selbst im Projekt nachsehen kannst.
2. Zerlege es in **1 bis 4 Teilaufgaben**, die unabhängig voneinander in eigenen Worktrees
   umgesetzt werden können (keine zwei Teilaufgaben ändern dieselben Stellen). Teile nicht
   künstlich: Ist das Ziel klein, ist eine Teilaufgabe richtig.
3. Gib jeder Teilaufgabe:
   - einen kurzen Titel,
   - den Auftrag in 2–6 Sätzen (was, wo, warum),
   - **Akzeptanzkriterien** als prüfbare Liste (inkl. Tests, die grün sein müssen),
   - einen Implementer: abwechselnd `impl-claude` und `impl-codex`, damit beide Vendors
     arbeiten; ist nur ein Vendor verfügbar, nur dieser.
4. Zeige den Plan als nummerierte Liste in einer normalen Antwort (nie über `ExitPlanMode`)
   und fahre dann ohne Rückfrage mit `fanout` fort,
   außer das Ziel ist mehrdeutig und eine falsche Annahme wäre teuer – dann frage einmal
   knapp nach.
