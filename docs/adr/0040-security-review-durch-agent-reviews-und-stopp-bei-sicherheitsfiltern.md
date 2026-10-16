# ADR-0040: Security-Review durch unabhängige Agent-Reviews und Stopp bei Sicherheitsfiltern (Übergang bis zur eigenen Agent-Identität)

- **Status:** Vorgeschlagen
- **Datum:** 2026-10-16
- **Entscheider:** Ingo Fahrentholz (Freigabe ausstehend); Entwurf eines Coding-Agents aus der Praxis von M2

## Kontext
ADR-0036 macht bis zur eigenen Agent-Identität (#177) die ausdrückliche Zustimmung des Maintainers im Chat oder als PR-Kommentar zur Freigabe von PRs mit Label `security-review`. In M2 hat der Maintainer diese Freigabe ab dem 2026-10-14 jeweils im Chat an **unabhängige Agent-Reviews** delegiert. Die PR-Kommentare lauten dann „Unabhängiges Security-Review (Agent, stellvertretend für den Maintainer laut Freigabe im Chat vom …, ADR-0036): MERGE“ bzw. „NICHT MERGEN“ (u. a. #220, #228, #237, #253, #257, #284, #285, #291, #302, #303, #306, #308, #313, #315, #317, #327). Die Reviews fanden mehrfach Blocker, die vor dem Merge behoben wurden. Kleinere Befunde wurden als Issues angelegt (#242, #243, #247, #248, #252, #258, #263, #268, #276, #286, #293, #295, #297, #329, #330).

Außerdem hat der Sicherheitsfilter des Modells Agents mehrfach angehalten, wenn sie Umgehungs- oder Ausbruchsvarianten ausformulieren sollten: die konsolidierte Escape-Suite (SBX-011, QA-005, #171), den Umgehungskorpus für das Shell-Parsing (#235) und die ersten Reviewer von #291 und #327. Die Agents haben jeweils gestoppt und nichts umformuliert. Beide Praktiken sind bisher nirgends festgehalten.

## Betrachtete Optionen
1. **A — Strikt nach ADR-0036** — jede Freigabe durch den Maintainer selbst; sicher, aber ein Engpass für jeden sicherheitskritischen PR.
2. **B — Delegation an unabhängige Agent-Reviews mit festen Regeln, bis #177 erledigt ist** — entspricht der Praxis von M2; die Freigabe bleibt ein Prozess, kein technischer Zwang.
3. **C — Bis #177 keine sicherheitskritischen Merges** — blockiert M3.

## Entscheidung (Vorschlag)
Option **B**, bis die eigene Agent-Identität (#177) eingerichtet ist:
- **Delegation:** Nur der Maintainer delegiert, im Chat und für einen genannten Zeitraum oder Umfang. Der Review-Kommentar nennt das Datum der Delegation.
- **Unabhängigkeit:** Der Reviewer ist eine eigene Agent-Instanz, die an der Umsetzung nicht beteiligt war. Er prüft gegen Spec, Bedrohungsmodell und AGENTS.md §4 und schließt mit „MERGE“ oder „NICHT MERGEN“.
- **Befunde:** Bei Blocker- oder Major-Befunden folgt eine Fix-Runde: erst ein Commit mit roten Tests, dann der Fix, danach ein erneutes Review. Minor-Befunde, die nicht im PR behoben werden, werden Issues im passenden Milestone.
- **Nicht delegierbar:** Änderungen an ADR-0036 und an diesem ADR, an Branch-Protection und Required Checks, an Signing-Schlüsseln und Release-Secrets, die Veröffentlichung eines Releases (Release-, Signing- und Update-Pipeline brauchen die ausdrückliche Zustimmung des Maintainers selbst), das Abschwächen von Sandbox-Escape-Tests und das Label `no-test-needed`.
- **Sicherheitsfilter:** Hält der Sicherheitsfilter einen Agent an (Umsetzung oder Review), stoppt er diese Teilaufgabe. Er formuliert nicht um, zerlegt die Aufgabe nicht und verschleiert nichts, um den Filter zu umgehen. Er hält den Stopp ohne den angehaltenen Inhalt als Issue-Kommentar fest und übergibt an den Maintainer; lokale, nicht gepushte Stände bleiben erhalten. Ein Review darf einmal von einem anderen Reviewer mit unveränderter Aufgabe wiederholt werden; wird auch er angehalten, entscheidet der Maintainer. Die betroffene Arbeit bleibt im Milestone als „zurückgestellt beim Maintainer“ markiert. Der Milestone wird nicht geschlossen, solange Must-ACs daran hängen.

## Konsequenzen
- Positiv: Sicherheitskritische PRs bekommen ohne Engpass ein dokumentiertes Review; die Befunde sind in GitHub nachvollziehbar.
- Negativ / Risiken: Die Delegation steht im Chat und ist in GitHub nur über die PR-Kommentare belegt; technisch erzwungen ist sie nicht (alle Aktionen laufen über den Account des Maintainers). Agent-Reviews können systematisch dieselben blinden Flecken haben wie die Umsetzung. Arbeit an Escape-Fällen und Umgehungskorpora hängt am Maintainer: M2 bleibt offen (#171, #235).
- Folgearbeiten: Nach Annahme AGENTS.md §5 auf dieses ADR verweisen lassen. Mit #177 werden ADR-0036 und dieses ADR durch ein neues ADR ersetzt.

## Bezug
- Spec: docs/spec/12-distribution-quality.md (QA-005, QA-013), docs/spec/04-sandbox.md (SBX-011)
- ADR-0031, ADR-0036
- Issues: #171, #177, #235
