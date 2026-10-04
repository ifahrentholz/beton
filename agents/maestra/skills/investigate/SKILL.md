---
name: investigate
description: Untersucht eine Frage oder einen Fehler ohne Änderungen, mit zwei unabhängigen Analysen verschiedener Vendors (maestra).
user-invocable: true
---

# Investigate

Für Fragen wie „Warum schlägt X fehl?“ oder „Wo wird Y entschieden?“ – ohne Code-Änderungen.

1. Starte `review-claude` und `review-codex` parallel (`async: true`, `worktree: "none"`)
   mit derselben Frage und der Bitte um eine Analyse mit Belegen (Dateien, Zeilen, Befehle).
   Ist nur ein Vendor verfügbar, nur diesen und vermerke „same-vendor review“.
2. Warte mit `session_wait(mode: all, timeout: "50s")`, bis `done: true` ist.
3. Vergleiche die Analysen: Was stimmt überein, wo widersprechen sie sich, was ist belegt?
   Antworte mit dem Befund, den Belegen und einem Vorschlag für das weitere Vorgehen. Ändere
   nichts.
