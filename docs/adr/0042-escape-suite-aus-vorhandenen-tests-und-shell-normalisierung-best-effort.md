# ADR-0042: Escape-Suite aus den vorhandenen Tests mit Kontrollgruppe, Shell-Normalisierung best effort

- **Status:** Akzeptiert
- **Datum:** 2026-10-17
- **Entscheider:** Ingo Fahrentholz (Entscheidungen vom 2026-10-17 in #171 und #235, jeweils Option b); festgehalten von einem Coding-Agent

## Kontext
SBX-011 und QA-005 sahen eine eigene, deklarative Escape-Suite vor (`crates/beton-sandbox/tests/escape/*.yaml` mit einem Probe-Binary `bt-escape-probe`, Mutationstest mit gelockertem Profil). Für das Shell-Parsing (POL-005) sah #235 einen Korpus von Umgehungsvarianten als Property-Tests vor. Beide Arbeiten wurden vom Sicherheitsfilter des Modells angehalten, als Agents die Ausbruchs- bzw. Umgehungsvarianten ausformulieren sollten; nach ADR-0040 lagen sie beim Maintainer.

Gleichzeitig gibt es seit M2 in `beton-sandbox`, `beton-runner` und `beton-cli` Escape-Tests mit Kontrollgruppe: Jeder Angriff läuft auch ohne Sandbox und muss dort gelingen, Homes und Secrets sind Fixtures, Linux-Backends sind über `BETON_TEST_LINUX_BACKENDS` erzwungen statt still übersprungen. Sie laufen in den Required Checks `test (macos)` und `test (linux, ohne Netzwerk)`.

## Betrachtete Optionen
1. **A — Deklarative Suite und Umgehungskorpus wie geplant** — einheitliches Format, aber genau die Arbeit, die der Sicherheitsfilter anhält; M2 bliebe offen.
2. **B — Die vorhandenen Tests sind die Suite; die Shell-Normalisierung bleibt best effort** — keine neuen Ausbruchs- oder Umgehungsvarianten, die Spec wird an das Vorhandene angepasst; die Sicherheit hängt an Sandbox, Proxy und Git-Shim, nicht am Parsen.
3. **C — Beides ersatzlos streichen** — schließt M2, verliert aber die Gates und die Plattform-Matrix.

## Entscheidung
Option **B**:
- **Escape-Suite (#171):** Die Escape-Suite sind die Escape-Tests mit Kontrollgruppe in `crates/beton-sandbox/tests/seatbelt.rs`, `tests/linux.rs`, `tests/broker.rs`, `crates/beton-runner/tests/stages.rs` und `crates/beton-cli/tests/m2_demo.rs`. Es gibt kein eigenes Probe-Binary und kein YAML-Format. Gate sind die Jobs `test (macos)` und `test (linux, ohne Netzwerk)` sowie die Plattform-Matrix (`escape (…)`, SBX-011); der JUnit-Bericht entsteht über das nextest-Profil `ci` aus diesen Tests. Die Wirksamkeit belegt die Kontrollgruppe jedes Falls; ein Mutationstest mit gelockertem Profil entfällt. Neue Fälle entstehen als Tests in diesen Dateien zusammen mit den Features, die sie absichern. `beton sandbox test` führt auf dem lokalen System die Selbsttests der Probe aus.
- **Shell-Normalisierung (#235):** Die Normalisierung von Shell-Kommandos ist best effort und keine Sicherheitsgrenze. Verbindlich setzen Sandbox, Egress-Proxy und Git-Shim durch; unsichere Fälle ergeben `parse_uncertain` (eingebaute Regeltypen: `ask`). Die Grenzen stehen in POL-005. Defensive Verbesserungen sind weiter möglich, ein Korpus von Umgehungsvarianten wird nicht gepflegt.

## Konsequenzen
- Positiv: SBX-011, QA-005, SBX-010 AC3 und #235 lassen sich ohne neue Ausbruchs- oder Umgehungsvarianten abschließen; Gates und Matrix bleiben.
- Negativ / Risiken: Die Fälle sind über mehrere Testdateien verteilt statt in einem Katalog; ein fehlender Fall fällt nur im Review auf. Regeln auf `tool.argv` schützen vor Versehen, nicht vor einem gezielt verschleierten Kommando.
- Folgearbeiten: keine aus diesem ADR. Escape-Tests werden weiterhin nie abgeschwächt oder übersprungen (AGENTS.md §4).

## Bezug
- Spec: docs/spec/04-sandbox.md (SBX-010, SBX-011), docs/spec/12-distribution-quality.md (QA-005), docs/spec/03-policies.md (POL-005)
- ADR-0031, ADR-0040
- Issues: #171, #235, #301
