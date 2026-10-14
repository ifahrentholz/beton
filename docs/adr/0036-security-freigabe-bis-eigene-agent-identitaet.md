# ADR-0036: Security-Freigabe ohne GitHub-Code-Owner-Approval bis zur eigenen Agent-Identität

- **Status:** Akzeptiert
- **Datum:** 2026-10-14
- **Entscheider:** Ingo Fahrentholz

## Kontext
QA-013 (ADR-0031) verlangt, dass Änderungen an sicherheitskritischen Crates (`beton-policy`, `beton-sandbox`, `beton-proxy`, `beton-secrets`, Auth-Code in `beton-server`) nur mit Approval des Maintainers als Code-Owner mergebar sind. Die Coding-Agents arbeiten derzeit unter dem GitHub-Account des Maintainers: Sie committen, öffnen PRs und kommentieren mit dessen Identität. GitHub zählt das Approval des PR-Autors nicht. Eine Branch-Protection mit „Require review from Code Owners“ würde deshalb jeden sicherheitskritischen PR dauerhaft blockieren. Auch ein Kommentar oder Label als Freigabe ist technisch nicht vom Agent unterscheidbar, solange beide denselben Account nutzen (#154).

## Betrachtete Optionen
1. **A — Eigene Identität für Agents (Bot-Account oder GitHub App) sofort** — echtes Code-Owner-Approval in GitHub, technisch durchgesetzt; braucht eine Account-Entscheidung, Einrichtung und Token-Verwaltung, bevor M2 weiterlaufen kann.
2. **B — Übergangsregel bis M3, danach A** — M2 läuft ohne Blockade weiter; die Freigabe ist bis dahin ein Prozess und kein technischer Zwang. Ein CI-Check stellt sicher, dass sicherheitskritische PRs immer als solche markiert sind.
3. **C — Nur der Maintainer darf mergen (Ruleset-Bypass-Liste)** — scheitert am selben Problem: Agents mergen mit dem Account des Maintainers.

## Entscheidung
Option **B**:
- **Bis zum Release 0.1 (M3)** gilt eine ausdrückliche Freigabe des Maintainers im Chat mit dem Agent oder als PR-Kommentar als Code-Owner-Approval. Agents mergen PRs mit Label `security-review` erst nach dieser Freigabe (AGENTS.md §5).
- **Technisch abgesichert** wird, dass kein sicherheitskritischer PR unbemerkt bleibt: Ein Schritt in einem Required-Check-Job lässt die CI scheitern, wenn ein PR Pfade aus `.github/CODEOWNERS` für die sicherheitskritischen Crates ändert und das Label `security-review` fehlt. `CODEOWNERS` bleibt gepflegt.
- Das Label `no-test-needed` (QA-013 AC2) setzen Agents ebenfalls nur nach ausdrücklicher Freigabe des Maintainers.
- **Vor dem Release 0.1** bekommen die Agents eine eigene Identität (Bot-Account oder GitHub App, Option A; #177). Danach verlangt die Branch-Protection das Code-Owner-Approval, und diese Übergangsregel entfällt.

## Konsequenzen
- Positiv: M2 läuft ohne Account-Einrichtung weiter; sicherheitskritische PRs sind durch Label und CI-Check immer sichtbar markiert.
- Negativ / Risiken: Bis M3 hängt die Freigabe an der Disziplin der Agents und an AGENTS.md, nicht an GitHub. Ein fehlerhafter Agent könnte einen markierten PR ohne Freigabe mergen. Das Repo ist bis dahin nicht als Release veröffentlicht.
- Folgearbeiten: CI-Check auf das Label (QA-013 AC1, #176); eigene Agent-Identität und Code-Owner-Approval in der Branch-Protection (#177, M3); danach dieses ADR auf „Abgelöst“ setzen.

## Bezug
- Spec: docs/spec/12-distribution-quality.md (QA-013)
- ADR-0031 (Qualitätsstrategie), ADR-0030 (agent-getriebene Entwicklung)
- Issues: #154, #176, #177
