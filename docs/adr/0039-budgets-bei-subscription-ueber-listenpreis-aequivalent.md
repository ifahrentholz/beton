# ADR-0039: Budgets bei Subscription-Sessions über das Listenpreis-Äquivalent

- **Status:** Akzeptiert
- **Datum:** 2026-10-16
- **Entscheider:** Coding-Agent (delegiert, AGENTS.md § 5); umgesetzt und security-reviewed in WP-43 (#166, PR #284)

## Kontext
ADR-0034 legt fest: Bei Subscription-Nutzung über die Vendor-CLI (`auth_source: vendor_cli`) entsteht nie ein Geldbetrag; Subscription-Records zählen nie zu Geld-Summen (USE-001). Das Demo-Szenario von M2 verlangt aber, dass ein Agent „bei Überschreitung des Budgets gestoppt“ wird, und Subscription ist der Default-Weg. POL-011 nahm an, dass eine Budget-Regel bei Subscription-Sessions den Katalogpreis nur nutzt, „wenn konfiguriert“, und sonst nicht greift *(Annahme)*. Das Security-Review von #284 zeigte außerdem: Über ein Modell ohne Katalogpreis ließe sich jedes Budget umgehen.

## Betrachtete Optionen
1. **A — Budgets gelten nicht für Subscription-Sessions** — konsequent zu ADR-0034, aber das Demo scheitert, und gegen außer Kontrolle geratene Agents auf dem Abo-Kontingent gibt es keine Bremse.
2. **B — Budgets in Tokens statt USD** — kein Geld nötig; neue Parameter, zwischen Modellen nicht vergleichbar, keine gemeinsame Grenze mit API-Sessions.
3. **C — Geld plus Listenpreis-Äquivalent aus dem eingebetteten Katalog** — eine Grenze für beide Arten; das Äquivalent ist nur eine Näherung des Kontingentverbrauchs.

## Entscheidung
Option **C**:
- Die Kosten einer Budget-Regel (`spend_cap`, `daily_budget`) sind Geld plus Listenpreis-Äquivalent des Bereichs (`session.cost_usd + session.equivalent_usd`, entsprechend für `run`, `subtree` und den Tag des Users). Das Äquivalent berechnet der Server beim Append aus dem im Binary eingebetteten, versionierten Katalog (USE-002, ADR-0033); Preis-Overrides (USE-003) zählen dafür nicht.
- Das Äquivalent ist in Usage-Ansichten nur informativ, wird nie als Kosten angezeigt und fließt nie in Geld-Summen (USE-001 AC3, ADR-0034).
- Modelle ohne Katalogpreis: Parameter `on_unpriced: ask|deny|ignore` (Default `ask`). Solange ein Budget gilt, dürfen Projekt- und Agent-Ebene sowie Spawns nicht auf ein Modell ohne Katalogpreis wechseln. Fehlen die Verbrauchswerte ganz, lehnt die Regel ab (fail closed).

## Konsequenzen
- Positiv: Auch eine Subscription-Session ist stoppbar, ohne dass Geld entsteht; API- und Subscription-Sessions teilen eine Grenze.
- Negativ / Risiken: Das Äquivalent bildet das Abo-Kontingent nur näherungsweise ab; die UI muss es klar als „zu API-Preisen, informativ“ kennzeichnen (#267, #281). Modelle mit gestaffelten Preisen fehlen im Katalog und gelten als „ohne Preis“ (#180). Handgeschriebene CEL-Budgets sind nicht automatisch gegen preislose Modelle geschützt (#297).
- Folgearbeiten: #180, #281, #282, #297.

## Bezug
- Spec: docs/spec/03-policies.md (POL-011, POL-012, POL-013), docs/spec/11-platform-features.md (USE-001, USE-002, USE-003)
- ADR-0033, ADR-0034
- Issues: #166, #284, #297
