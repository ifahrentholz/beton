# ADR-0022: Smart Routing auf v2 verschoben

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Omnigent bietet "Smart Routing": Die Option "Auto" im Harness-Picker lässt einen LLM-Judge (oder eine externe Routing-API) anhand des ersten Prompts Harness und Modell wählen. Das erfordert einen serverseitigen LLM-Zugang, Evaluationsdaten und Fallback-Logik und ist schwer zu testen. beton hat mit Policies (ADR-0008) bereits einen Mechanismus, Modelle regelbasiert umzurouten (`modify: model`).

## Betrachtete Optionen
1. **A — Smart Routing in v1 (LLM-Judge)** — Komfortfeature; zusätzlicher LLM-Zugang am Server, nicht deterministisch, Testaufwand.
2. **B — Nur regelbasiertes Routing per Policy in v1, Smart Routing in v2** — deterministisch, testbar; kein "Auto"-Komfort.
3. **C — Gar kein Routing** — einfachster Weg; Budget-Szenarien (Downgrade bei Überschreitung) nicht abbildbar.

## Entscheidung
Option **B**. Smart Routing ("Auto"-Harness-Wahl, lernender Router) wird **bewusst gestrichen und auf v2 verschoben**. In v1 gibt es nur **manuelles Routing per Policy** (`modify: model`, z. B. bei Budget-Überschreitung auf ein günstigeres Modell).

## Konsequenzen
- Positiv: Weniger Komplexität und keine nicht-deterministische Komponente in v1; Routing ist über Policy-Tests prüfbar.
- Negativ / Risiken: Nutzer müssen Harness/Modell selbst wählen; Wettbewerbsnachteil gegenüber Omnigent in diesem Punkt.
- Folgearbeiten: In v2 Router als Policy-Erweiterung (WASM, ADR-0018) oder eigener Dienst evaluieren; Telemetrie-freie Evaluationsdaten klären.
- Verschärft durch ADR-0034: Ein Router in v2 muss über eine eingeloggte Vendor-CLI bzw. den Harness der Session laufen; ein API-Key ist nur zusätzliche Option.

## Bezug
- Spec: docs/spec/03-policies.md (Prefix POL), docs/roadmap.md
