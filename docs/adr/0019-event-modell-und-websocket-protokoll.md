# ADR-0019: Eigenes Event-Modell + WebSocket mit Resume ab `seq`, Binärkanäle, Typen aus Rust generiert

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Clients, Server, Hosts und Runner tauschen Session-Events aus. Omnigent lehnt sein Schema an die OpenAI-Responses-API an, streamt per SSE **ohne Replay und ohne Sequenznummern** (Clients müssen Snapshot + Stream deduplizieren) und tunnelt intern HTTP über WebSocket. beton braucht ein harness-neutrales Modell, das zum Event-Log (ADR-0009) passt und Wiederaufnahme nach Verbindungsabbruch erlaubt; Terminal-Bytes und Browser-Frames (ADR-0016) sind binär.

## Betrachtete Optionen
1. **A — JSON über WebSocket (+ REST, SSE read-only), eigenes Event-Modell** — browser-nativ, leicht debugbar, TS-freundlich; größere Payloads.
2. **B — Protobuf/gRPC (gRPC-Web im Browser)** — kompakt, stark typisiert; Browser-Unterstützung nur über Proxy, schlechter debugbar.
3. **C — OpenAI-Responses-API übernehmen (wie Omnigent)** — bekanntes Schema; vendor-geprägt, passt nicht zu Approvals/Policies/Collaboration.

## Entscheidung
Option **A**:
- **Eigenes, harness-neutrales Event-Modell**, z. B. `session.started`, `message.delta/completed`, `reasoning.delta`, `tool.call.requested/started/completed`, `approval.requested/resolved`, `policy.decision`, `cost.delta`, `fs.changed`, `terminal.output`, `browser.frame`, `comment.added`, `session.forked`.
- Jedes Event: `session_id`, monotone `seq`, `ts`, `actor` (user/agent/system), optional `raw` (Original-Harness-Payload).
- **Transport:** WebSocket bidirektional mit **Resume ab `seq`**; REST für CRUD; SSE read-only für Skripte.
- JSON, versioniert (`v1`). **Binärdaten** (Browser-Frames, Terminal-Bytes) über separate Binärkanäle im selben WS (Kanal-ID-Multiplexing). Terminal-Output und Browser-Frames sind ephemer bzw. nur Snapshots im Log.
- **Single Source of Truth = Rust-Typen** → JSON-Schema (`schemars`), TS-Typen (`ts-rs`/`specta`), OpenAPI (`utoipa`).
- **Versionsaushandlung:** Client ↔ Server kompatibel bis eine Minor-Version älter, in beide Richtungen.

## Konsequenzen
- Positiv: Lückenlose Wiederaufnahme ohne Dedup-Logik im Client; ein Typsystem für Server, Web, SDKs.
- Positiv: `raw` erlaubt Debugging und Golden-Transcript-Tests.
- Negativ / Risiken: JSON-Overhead bei hochfrequenten Deltas → Event-Batching nötig; Schema-Evolution diszipliniert pflegen.
- Folgearbeiten: Framing-Spezifikation (Kanäle, Heartbeat, Backpressure), Snapshot-Tests der generierten Schemas (ADR-0031), Tunnel-Protokoll Host/Runner ↔ Server.

## Bezug
- Spec: docs/spec/06-data-sync-protocol.md (Prefix PROTO)
