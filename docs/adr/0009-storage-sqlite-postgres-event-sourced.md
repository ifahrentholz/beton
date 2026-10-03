# ADR-0009: Storage – SQLite lokal + Postgres zentral, event-sourced, Blob-Store

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
beton muss Sessions, Events, Policies, Nutzer, Budgets und Artefakte persistieren – lokal ohne Zusatzdienste, zentral mehrinstanzfähig. Lokal und zentral sollen derselbe Code sein (ADR-0003). Omnigent nutzt SQLAlchemy mit SQLite/Postgres/CockroachDB, speichert Konversations-Items als komprimiertes JSON und hat einen Artifact-Store (lokal/S3). Sessions sollen geforkt, resumed, synchronisiert und ab beliebigem `seq` wiedergegeben werden können (ADR-0010, ADR-0019).

## Betrachtete Optionen
1. **A — Nur SQLite (auch zentral)** — einfach; keine horizontale Skalierung, schwache Nebenläufigkeit im Team-Betrieb.
2. **B — Nur Postgres (auch lokal)** — eine Datenbank; lokal ein Dienst zu installieren/betreiben – widerspricht "Lokal-only voll funktionsfähig ohne Setup".
3. **C — SQLite lokal + Postgres zentral mit gemeinsamer Repository-Schicht** — beste Passform je Umgebung; zwei Dialekte testen.

## Entscheidung
Option **C**:
- **SQLite lokal, Postgres zentral**, gemeinsame Repository-Schicht in `beton-store` auf Basis von `sqlx`.
- **Event-sourced:** Jede Session ist ein **Append-only-Log typisierter Events** (monotone `seq` pro Session); Projektionen/Views (Session-Liste, Usage) werden daraus abgeleitet.
- **Blob-Store** für große Artefakte (Anhänge, Screenshots, Exporte): lokal Dateisystem, zentral S3-kompatibel.
- Terminal-Output und Browser-Frames sind ephemer bzw. nur als Snapshots im Log (ADR-0019).

## Konsequenzen
- Positiv: Fork ab Event X, Resume ab `seq`, Export/Import (JSONL) und Sync fallen natürlich aus dem Log-Modell heraus.
- Positiv: Kein Setup lokal; zentral skalierbar.
- Negativ / Risiken: Migrationen und Queries müssen auf beiden Dialekten laufen → CI-Matrix mit SQLite und Postgres.
- Negativ / Risiken: Event-Log-Wachstum; Snapshot-/Kompaktierungsstrategie für Projektionen nötig.
- Folgearbeiten: Event-Schema-Versionierung, Retention-Regeln, Blob-Referenzen (Content-Hash) im Log.

## Bezug
- Spec: docs/spec/06-data-sync-protocol.md (Prefix DATA)
