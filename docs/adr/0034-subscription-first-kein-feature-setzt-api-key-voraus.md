# ADR-0034: Subscription-first – kein Feature setzt einen API-Key voraus

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
ADR-0005 legt fest, dass Subscriptions (Claude Pro/Max, ChatGPT Plus/Pro) über die offizielle Vendor-CLI funktionieren. Offen blieb, was für Features gilt, die **selbst** ein Modell brauchen – automatische Session-Titel, Compaction, Zusammenfassungen, Built-in-Agents (`maestra`, `duetto`) und künftig der Router (v2, ADR-0022) bzw. LLM-basierte Policies (v2, ADR-0008). Die Spec sah für Titel bevorzugt den Direkt-API-Harness vor, falls ein API-Key vorhanden ist; ACP-Presets reichten `GEMINI_API_KEY` durch, obwohl die Gemini CLI einen Google-Login hat. Viele Nutzer haben nur eine Subscription und keinen API-Key. CI kann keine Subscriptions nutzen (ADR-0031), also muss der Subscription-Pfad anders abgesichert werden.

## Betrachtete Optionen
1. **A — API-Key für Hilfsfunktionen (Titel, Zusammenfassungen) voraussetzen** — einfach, deterministische Kosten; Subscription-Nutzer verlieren Funktionen.
2. **B — Subscription-first: Hilfsfunktionen über die eingeloggte Vendor-CLI bzw. den Harness der Session; API-Key nur als zusätzliche Option** — alles funktioniert mit Subscription; Verhalten hängt von CLI-Einmal-Modi ab.
3. **C — Hilfsfunktionen ganz ohne Modell (nur Heuristik)** — kein Modellzugang nötig; deutlich schlechtere Ergebnisse, Built-in-Agents unmöglich.

## Entscheidung
Option **B**.
- **Alles muss mit Subscription-Modellen funktionieren:** Claude Pro/Max über `claude`, ChatGPT Plus/Pro über `codex`, Google-Login über die Gemini CLI via ACP und entsprechend weitere ACP-Agents mit eigenem Login.
- **Kein Feature darf einen API-Key voraussetzen.** Default-Auth jedes Vendor-CLI-Harness ist der Login der CLI (`auth: subscription`); API-Keys und der Direkt-API-Harness sind **nur eine zusätzliche Option**.
- **Features, die selbst ein Modell brauchen** (z. B. automatische Session-Titel, Compaction, Zusammenfassungen, Side-Chat-Übernahme, Built-in-Agents `maestra`/`duetto`, Router in v2, LLM-basierte Policies in v2), laufen über eine **eingeloggte Vendor-CLI im Einmal-Modus** (z. B. `claude -p`, `codex exec`) **bzw. über den Harness der Session**. Der Direkt-API-Harness wird dafür nur genutzt, wenn der User ihn ausdrücklich wählt.
- Subscription-Regel aus ADR-0005 gilt unverändert: beton liest, speichert oder nutzt keine Subscription-Tokens selbst.
- **Verifikation:** CI testet die Pfade mit Fake-CLIs (Aufruf ohne `*_API_KEY` im Env). **Pro Release** wird zusätzlich eine **manuelle Verifikations-Checkliste mit echten Subscription-Logins** durchlaufen und protokolliert (QA-019); ohne Protokoll bleibt der Release Draft.
- Die **rechtliche Klärung** der Nutzungsbedingungen bleibt Issue #1 bzw. offener Punkt 4 beim Maintainer (ADR-0005).

## Konsequenzen
- Positiv: Nutzer mit reiner Subscription erhalten den vollen Funktionsumfang; keine versteckten API-Kosten für Hilfsfunktionen.
- Positiv: Einheitliche Regel für künftige Modell-Features (Router, LLM-Policies, Prompt-Cleanup in v2).
- Negativ / Risiken: Hilfsfunktionen verbrauchen Subscription-Kontingent (Rate-Limits) und hängen von Einmal-Modi der Vendor-CLIs ab, die sich ändern können → Golden-Transcripts und Release-Checkliste.
- Negativ / Risiken: Manuelle Release-Verifikation kostet Zeit und braucht aktive Subscriptions beim Maintainer.
- Negativ / Risiken (rechtlich, offen): Die Zulässigkeit hängt weiter am Ergebnis von offenem Punkt 4 (ADR-0005).
- Folgearbeiten: Einmal-Modus je Adapter (Capability) definieren; Checkliste `docs/release/subscription-checklist.md` anlegen; Presets ohne API-Key-Pflicht.

## Bezug
- Spec: docs/spec/01-harnesses.md (HAR-008, HAR-015), docs/spec/02-agents.md (AGT-011, AGT-012), docs/spec/07-sessions-collaboration.md (SES-010, COL-008), docs/spec/12-distribution-quality.md (QA-019)
- Verschärft ADR-0005, ADR-0008, ADR-0021, ADR-0022, ADR-0031
