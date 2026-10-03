# ADR-0010: Sync – Single-Writer + Fork bei Divergenz, server-autoritative Policies, Budget-Leases

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Im Team-Betrieb (M4) existieren Sessions sowohl auf lokalen Knoten (Laptop mit eigenem Daemon) als auch auf dem zentralen Server. Sessions sind Append-only-Event-Logs (ADR-0009). Laptops können offline gehen und weiterarbeiten. Policies und Budgets müssen zentral durchsetzbar bleiben, ohne den Offline-Betrieb vollständig zu blockieren.

## Betrachtete Optionen
1. **A — Single-Writer pro Session (Home-Knoten), andere sind Read-Replicas** — einfache, deterministische Semantik; Offline-Weiterarbeit nur nach Ownership-Übernahme.
2. **B — Multi-Writer mit CRDT/Merge** — beliebige gleichzeitige Schreiber; Agent-Transkripte sind semantisch nicht sinnvoll mergebar, hohe Komplexität.
3. **C — Nur zentral (kein lokaler Schreiber im Team-Modus)** — einfach; kein Offline-Betrieb.

## Entscheidung
Option **A** mit Fork als Notfallpfad:
- **Single-Writer pro Session:** Der Home-Knoten schreibt das Log; andere Knoten sind Read-Replicas und senden Inputs (Nachrichten, Approvals) an den Home-Knoten.
- **Explizite Ownership-Übernahme** ermöglicht Offline-Weiterarbeit auf einem anderen Knoten.
- Bei dennoch auftretender **Divergenz** wird automatisch ein **Fork** erzeugt (kein Merge).
- **Policies sind server-autoritativ**, werden lokal gecacht und dürfen lokal **nur verschärft** werden.
- **Budget-Leases:** Der Server vergibt Offline-Budget (z. B. 10 €) an einen Knoten; ist es aufgebraucht, gilt `ask`/`deny` bis zum Reconnect.

## Konsequenzen
- Positiv: Deterministische Logs, kein Merge-Konflikt-Code; Konflikte werden sichtbar (Fork) statt still verschluckt.
- Positiv: Zentrale Kostenkontrolle auch offline mit begrenztem Risiko.
- Negativ / Risiken: Nutzer müssen Ownership verstehen; Forks bei Divergenz können überraschen → klare UI-Hinweise.
- Negativ / Risiken: Lease-Abrechnung beim Reconnect (Über-/Unterverbrauch) braucht saubere Abgleichlogik.
- Folgearbeiten: Ownership-Protokoll, Replikationsformat (Events ab `seq`), Lease-Erneuerung und -Widerruf.

## Bezug
- Spec: docs/spec/06-data-sync-protocol.md (Prefix SYNC), docs/spec/03-policies.md (Prefix POL)
