# 06 — Daten, Sync & Protokoll

Dieses Kapitel spezifiziert das **Wire-Protokoll** (Prefix `PROTO`: Event-Envelope, Event-Katalog v1, WebSocket, REST, SSE, Schema-Generierung, Tunnel Host/Runner → Server), das **Datenmodell und die Persistenz** (Prefix `DATA`: Entitäten, Event-Log, Blob-Store, Migrationen, Retention, Export/Import, ephemere Daten) und die **Synchronisation** zwischen lokalen Knoten und Team-Server (Prefix `SYNC`: Home-Knoten/Single-Writer, Replikation, Input-Forwarding, Ownership, Divergenz → Fork, Policy-Cache, Budget-Leases).

Grundlage sind ADR-0019 (eigenes Event-Modell, WebSocket mit Resume ab `seq`, generierte Typen), ADR-0009 (SQLite lokal, Postgres zentral, event-sourced, Blob-Store) und ADR-0010 (Single-Writer, Fork bei Divergenz, server-autoritative Policies, Budget-Leases), ergänzt durch ADR-0003 (Runner verbinden sich ausgehend) und ADR-0011 (Auth, siehe AUTH in 05-security-identity.md). Event-Namen, die andere Kapitel als *Vorschlag* führen (u. a. SES/COL in 07-sessions-collaboration.md, SBX/PRX in 04-sandbox.md, HAR in 01-harnesses.md, AGT/ASY in 02-agents.md, BRW in 09-browser.md), sind hier verbindlich übernommen. Meilensteine: Protokoll, Event-Log und SQLite in **M0**; Export-Format in **M1**; Binärkanäle und Snapshots in **M3**; Postgres, S3, Remote-Tunnel und Sync in **M4**.

## Konzepte & Begriffe

| Begriff | Bedeutung |
| --- | --- |
| **Event** | Unveränderlicher, typisierter Eintrag im Log einer Session (Envelope + Payload). |
| **Dauerhaftes Event** | Wird persistiert und erhält eine lückenlose `seq`. |
| **Transientes Event** | Hochfrequente Zwischenstände (`*.delta`, Presence, Frames); nicht im Log, nur kurz im Ringpuffer des Home-Knotens; trägt `tseq`. |
| **`seq`** | Monoton steigende, lückenlose Sequenznummer pro Session, vergeben ausschließlich vom Home-Knoten. |
| **Home-Knoten** | Einziger Knoten (lokaler Daemon oder Team-Server), der das Log einer Session schreibt (Single-Writer). |
| **Replica** | Knoten mit Lesekopie des Logs; leitet Kommandos an den Home-Knoten weiter. |
| **Epoch** | Zähler der Ownership einer Session; jede Übernahme erhöht ihn; dient als Fencing-Token. |
| **Knoten (Node)** | Eine `beton serve`-Instanz: lokaler Daemon oder Team-Server. Hosts und Runner sind keine Knoten, sondern hängen an genau einem Knoten. |
| **Tunnel** | Ausgehende WebSocket-Verbindung von Host, Runner oder Knoten zum (Team-)Server. |
| **Kanal** | Multiplexter Binärstrom im selben WebSocket (Terminal, Browser, Audio), adressiert über `channel_id`. |
| **Projektion** | Aus Events abgeleitete, neu aufbaubare Lese-Tabelle (Session-Liste, offene Approvals, Usage). |
| **Blob** | Inhaltsadressiertes Artefakt (SHA-256) im Blob-Store; Events referenzieren Blobs per `blob_ref`. |
| **Policy-Bundle** | Signierter, versionierter Satz server-autoritativer Policies, den Knoten cachen. |
| **Budget-Lease** | Vom Server reserviertes Offline-Budget für einen Knoten (Betrag, Ablauf, Abrechnung). |
| **Problem** | Fehlerobjekt nach RFC 9457 (`application/problem+json`), auch in WS-Antworten. |

## Design

### Event-Envelope

```json
{
  "v": 1,
  "id": "evt_01JB8Y3K6V9Q7W2N4R5T6Y7Z8A",
  "session_id": "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C",
  "seq": 42,
  "ts": "2026-10-03T12:00:00.123Z",
  "actor": { "kind": "agent", "id": "agt_01JB…", "harness": "claude" },
  "type": "tool.call.requested",
  "payload": { "call_id": "call_7", "tool": "bash", "args": { "command": "cargo test" } },
  "turn_id": "trn_01JB…",
  "causation_id": "evt_01JB…",
  "raw": { "…": "Original-Harness-Payload (optional, redigiert)" }
}
```

- `actor.kind ∈ {user, agent, system}`; `user` mit `id` (`usr_`/`sa_`) und optional `device_id`; `agent` mit `harness`, optional `agent_ref`; `system` mit `component ∈ {policy, sandbox, proxy, scheduler, sync, runner, server}`.
- IDs: Präfix + ULID (`ses_`, `evt_`, `trn_`, `usr_`, `sa_`, `org_`, `team_`, `prj_`, `agt_`, `pol_`, `sec_`, `hst_`, `run_`, `dev_`, `sch_`, `tmr_`, `cmt_`, `shr_`, `apr_`, `bls_`, `nod_`).
- Transiente Events: `seq` = letzte dauerhafte `seq`, zusätzlich `"transient": true, "tseq": <n>` (pro Session und Epoch monoton).
- Payloads > 64 KiB (serialisiert) werden als Blob ausgelagert: Das Event trägt dann `"payload_ref": "sha256:<hex>"` **statt** `payload` (Typ `OffloadedPayload` in den generierten Schemas); den Inhalt holen Clients über `GET /v1/sessions/{id}/blobs/{hash}`. Große Tool-Ergebnisse als `result_ref`.

### Event-Katalog v1

D = dauerhaft, T = transient. Felder knapp; optionale Felder mit `?`.

| Gruppe | Typ | D/T | Payload |
| --- | --- | :-: | --- |
| Session | `session.created` | D | `owner, kind: main\|side_chat\|subagent\|async, harness, agent_ref?, model?, effort?, permission_mode?, cwd, project_id?, parent_session_id?, trigger: user\|api\|schedule\|timer\|spawn\|webhook, harness_opts?` |
| | `session.started` | D | `runner_id, host_id, harness, harness_version, capabilities, harness_session_ref?` |
| | `session.status` | D | `status: starting\|idle\|running\|waiting_approval\|paused\|stopped\|failed, reason?` |
| | `session.resumed` | D | `mode: native\|handover` |
| | `session.settings_changed` | D | `model?, effort?, requested_effort?, permission_mode?, mechanism?: live\|restart, effective_from_turn?` (kein `harness`: Harness-Wechsel nur per Fork, SES-007) |
| | `session.title_changed` | D | `title, source: generated\|user\|harness` |
| | `session.archived` / `session.unarchived` | D | — |
| | `session.forked` | D | `from_session, at_seq, reason: user\|side_chat\|divergence, harness?, from_harness?, history_mode?: native\|rebuild\|preamble, fallback_reason?` |
| | `session.fork_created` | D | `child, at_seq, reason` |
| | `session.imported` | D | `source, vendor_session_id, imported_at` |
| | `session.ownership_changed` | D | `home_node, epoch, mode: planned\|forced` |
| | `session.start_denied` | D | `reasons, rules?` (POL-003, POL-024) |
| Harness & Runner | `harness.ready` | D | `harness_session_ref?, tools, mcp_servers` (HAR-004) |
| | `harness.exited` | D | `code?, signal?, stderr_tail` (HAR-001) |
| | `harness.auth_required` | D | `harness, hint` (HAR-015) |
| | `harness.incompatible` | D | `detected_version, expected_range` (HAR-002) |
| | `mcp.server_failed` | D | `name, error` (HAR-009) |
| | `runner.status` | D | `runner_id, from, to, reason?` (RUN-003) |
| Input & Turn | `queue.updated` | D | `items: [{id, author, text, attachments, created_at}], paused` (vollständiger Stand, SES-004) |
| | `turn.started` | D | `turn_id, input_id?, author` |
| | `turn.completed` | D | `turn_id, stop_reason, usage_summary` |
| | `turn.failed` | D | `turn_id, problem` |
| | `turn.interrupted` | D | `turn_id, by, reason?` (`timed_out`: `executor.timeout`, AGT-004) |
| Nachrichten | `message.delta` | T | `message_id, text` |
| | `message.completed` | D | `message_id, role: user\|assistant, content: [text\|image\|file_ref\|code], author?` |
| | `reasoning.delta` | T | `message_id, text` |
| | `reasoning.completed` | D | `message_id, summary?, redacted` |
| Tools | `tool.call.requested` | D | `call_id, tool, mcp_server?, args, source: harness\|beton_mcp\|acp, parent_call_id?` (`parent_call_id`: auslösender Tool-Call eines Vendor-Sub-Agents, HAR-023) |
| | `tool.call.started` | D | `call_id, sandbox_stage?, args?` (ausgeführte Argumente, wenn das Gate sie geändert hat) |
| | `tool.call.output.delta` | T | `call_id, stream: stdout\|stderr, text` |
| | `tool.call.completed` | D | `call_id, status: ok\|error\|denied\|cancelled, result?\|result_ref?, duration_ms` |
| Kontrolle | `approval.requested` | D | `approval_id, kind: tool\|policy\|browser\|budget\|question, subject, options, expires_at, on_timeout: deny\|allow\|abort` |
| | `approval.resolved` | D | `approval_id, decision: allow\|deny\|abort\|answer, answer?, actor, via: user\|timeout\|policy\|system, remember?: none\|session, comment?, on_timeout_applied?` |
| | `policy.decision` | D | `decision_id, phase, outcome: allow\|deny\|ask, modified?, approval_id?, matched[], defaults_applied[], conflicts[], errors[], grants_used[], enforcement, policy_set_hash, eval_us` (Details POL-025) |
| | `policy.set_changed` / `policy.set_invalid` | D | `scope, set, version` bzw. `file, line, error` (POL-001) |
| | `policy.state_changed` | D | `scope, key, op, value` (POL-009) |
| | `policy.enforcement_degraded` / `policy.rule_inapplicable` | D | `rule_ids, reason` (POL-024, POL-011) |
| | `budget.exhausted` | D | `scope, lease_id?, action: ask\|deny` |
| | `budget.lease_overdrawn` | D | `lease_id, overrun_micro` (POL-021, SYNC-007) |
| Kosten & Kontext | `cost.delta` | D | `harness, model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cost_micro?, currency, source: reported\|estimated\|subscription, auth_source: vendor_cli\|api_key\|gateway\|none, purpose?` |
| | `usage.subscription` | D | `vendor, window, used_pct, resets_at` |
| | `context.usage` | D | `used_tokens, window_tokens, source: harness\|estimated` |
| | `compaction.started` / `compaction.completed` | D | `before_tokens, after_tokens?` |
| Workspace & Git | `fs.changed` | D | `changes: [{path, change: added\|modified\|deleted\|renamed, from?}], source: watcher\|api\|import` |
| | `git.worktree_created` | D | `path, branch, base, base_sha` |
| | `git.commit_created` | D | `sha, branch, subject` |
| | `cr.linked` / `cr.unlinked` | D | `url, provider, number, origin: created\|attached\|inferred` |
| | `git.conflicts_detected` | D | `base, base_sha, head_sha, files[{path, kind: content\|delete_modify\|rename\|binary}], source: local\|provider` |
| | `git.conflict_resolution` | D | `phase: started\|hunk_resolved\|completed\|aborted, strategy?: merge\|rebase, path?, hunk?, resolution?: ours\|theirs\|both\|edited\|agent, actor` |
| Terminal | `terminal.opened` | D | `terminal_id, channel_id, kind: harness_pty\|user\|tool, cmd, cols, rows` |
| | `terminal.output` | T | Binärkanal (Bytes) |
| | `terminal.snapshot` | D | `terminal_id, blob_ref, cols, rows, cursor` |
| | `terminal.closed` | D | `terminal_id, exit_code?` |
| Browser | `browser.opened` | D | `browser_id, channel_id, mode: embedded\|window` |
| | `browser.frame` | T | Binärkanal (Bild + Metadaten) |
| | `browser.navigated` | D | `browser_id, url, title` |
| | `browser.action` | D | `browser_id, action, target_ref?, decision` |
| | `browser.snapshot` | D | `browser_id, screenshot_ref, a11y_ref?` |
| | `browser.picked` | D | `browser_id, selector, outer_html_ref, styles, bbox, screenshot_ref, comment?, source_location?` |
| | `browser.closed` | D | `browser_id, reason?: stopped\|idle\|crashed` |
| | `browser.devserver.detected` | D | `url, pid, source` (BRW-017) |
| Sandbox & Egress | `sandbox.started` | D | `stage: harness\|tools, backend, caps, degraded` |
| | `sandbox.violation` | D | `stage, kind: fs\|net\|syscall\|limit\|proxy_down, target?, suppressed_count` |
| | `egress.blocked` | D | `method, host, path, reason, rule_hint?, source: tools\|harness\|browser, tool_call_id?` |
| | `egress.summary` | D | `window_s, hosts: [{host, requests, bytes}]` |
| Agents & Async | `agent.spawned` | D | `child_session_id, agent_ref, harness, async` |
| | `agent.completed` | D | `child_session_id, status, summary?, cost_micro?` |
| | `agent.resolved` | D | `name, version, source, hash, blob_ref, overrides?, params?` (AGT-004) |
| | `agent.message` | D | `from_session, to_session, text` |
| | `timer.set` / `timer.fired` / `timer.cancelled` | D | `timer_id, fire_at?, note?, late_by_s?` |
| | `async.run.queued` / `.started` / `.paused` / `.resumed` / `.finished` | D | `run_id, trigger?, reason?, status?, cost_micro?, duration_s?, summary?` (ASY-001) |
| | `schedule.created` / `.updated` / `.deleted` / `.fired` / `.skipped` | D | `schedule_id, run_id?, scheduled_for?, reason?, count?` (ASY-004) |
| Collaboration | `comment.added` / `.updated` / `.resolved` / `.deleted` | D | `comment_id, thread_id, anchor, body?, author` |
| | `comment.addressed` | D | `comment_ids, turn_id` |
| | `share.granted` / `.changed` / `.revoked` | D | `principal, role, by` |
| | `presence.updated` | T | `users: [{user_id, devices, typing}]` (COL-004, siehe 07-sessions-collaboration.md) |
| System | `event.redacted` | D | `target_seq, by, reason` |
| | `harness.unmapped` | D | `raw` (unbekanntes Vendor-Event, für Golden-Tests) |
| | `notice` | D | `level: info\|warn, text` |
| | `error` | D | `problem` |

### WebSocket-Protokoll

Endpunkt `GET /v1/ws`, Subprotokoll `beton.v1` (Major). Textframes = JSON-Steuernachrichten mit Feld `t`; Binärframes = Kanaldaten.

```
Client                                         Server
  │── hello {protocol:"1.4", client:{kind,version}} ─►│  Version: min(client, server), beide ≥ max(eigene)−1
  │◄─ welcome {protocol:"1.4", server_version, session_limits} ─│  sonst Close 4400 {supported:{min,max}}
  │── attach {id:"r1", session_id, from_seq:120, transient:true} ─►│
  │◄─ events {session_id, events:[seq 121…184]}  (Batches ≤ 64 Events / 256 KiB) ─│
  │◄─ live {session_id, head_seq:184}  + Snapshot laufender Deltas (snapshot:true) ─│
  │◄─ events {…live…} ─│
  │── cmd {id:"c7", session_id, name:"input.submit", args:{text, mode:"queue"}, idempotency_key} ─►│
  │◄─ ack {id:"c7", result:{input_id}}   |   nack {id:"c7", problem:{…RFC 9457…}} ─│
  │── open_channel {id:"r2", session_id, kind:"terminal", ref:"term_3"} ─►│
  │◄─ channel_opened {id:"r2", channel_id:5, window:262144} ─│
  │◄══ Binär [ver u8][channel_id u32 BE][flags u8][payload] ══►│  flags: FIN, KEYFRAME, INPUT
  │── credit {channel_id:5, bytes:262144} ─►│
```

Die erste Protokollversion ist `1.0`; die Beispiele oben zeigen eine spätere Minor. Kommandos (gleichwertig zu REST): `input.submit` (`mode: queue|steer`), `queue.edit|delete|reorder|steer|resume`, `turn.interrupt`, `approval.resolve`, `comment.add|update|resolve|delete|address`, `session.set`, `session.compact`, `terminal.resize`, `browser.input`, `presence.update`, `voice.start|stop` (Audio-Kanal; Transkripte kommen als `voice.partial|final` direkt an den Client, nicht ins Session-Log). Close-Codes: `4400` Protokoll/Version, `4401` nicht (mehr) authentisiert, `4403` verboten/Share widerrufen, `4404` Session unbekannt, `4408` Heartbeat-Timeout, `4429` Rate-Limit, `4500` intern, `4503` Server fährt herunter (Reconnect).

### Tunnel-Protokoll (Host/Runner/Knoten → Server)

Endpunkt `GET /v1/tunnel` (lokal ausschließlich auf dem Tunnel-Socket, nicht auf dem TCP-Port und nicht in der öffentlichen OpenAPI), Subprotokoll `beton.tunnel.v1`, `Authorization: Bearer` (Host: Device-Token; Runner: Runner-Token; Knoten: Device-Token mit Scope `nodes:sync`); lokal über Unix-Socket `~/.beton/run/tunnel.sock` mit identischem Framing. `hello {kind: host|runner|node, node_id, version, protocol, labels, capabilities, harnesses, sandbox_probe}` → `welcome {server_time, policy_bundle_version}`.

| Richtung | Nachricht | Zweck |
| --- | --- | --- |
| S → Host | `runner.launch {request_id, session_id, spec}` / `runner.stop {session_id, grace_s}` | Runner starten/stoppen; Runner-Token geht per FD an den Runner (AUTH-011, siehe 05-security-identity.md) |
| Host → S | `runner.launched`, `runner.exited {code, reason}`, `host.status {runners, load}` | Lebenszyklus |
| S ↔ Host | `rpc.request {id, method, params}` / `rpc.response` | Host-Operationen (Worktrees, Harness-Erkennung, Import-Listen); Methoden in den Fachkapiteln |
| Runner → S | `session.bind {session_id, epoch, last_acked_rseq}` | Anmeldung / Wiederaufnahme |
| Runner → S | `events.push {session_id, epoch, batch: [{rseq, event}]}` | Events ohne `seq`; Home vergibt `seq` |
| S → Runner | `events.ack {session_id, upto_rseq, seq_range}` | Runner verwirft bis `upto_rseq`; ungeackte werden nach Reconnect erneut gesendet |
| Runner → S | `transient.push {session_id, events}` | Deltas, ohne Ack |
| S → Runner | `cmd.deliver {cmd_id, name, args, actor}` → `cmd.result` | Inputs, Approvals, Interrupts |
| Runner → S | `system.call {call_id, tool, args}` → `system.result {call_id, result?, problem?}` | System-Tools, die der Server ausführt, z. B. `session.spawn` (AGT-007); gilt immer für die gebundene Session, Arbeitsverzeichnis und Owner leitet der Server daraus ab |
| S → Runner | `policy.bundle {version, hash, sig, rules}` | Policy-Cache (SYNC-006) |
| beide | `secret.lease.*` (SEC-008, siehe 05-security-identity.md), `budget.lease.*` (SYNC-007) | Leases |
| beide | Binärframes wie Client-WS | Terminal-/Browser-Kanäle (Server mappt auf Client-Kanäle) |

Keepalive: WS-Ping alle 20 s, Abbruch nach 60 s ohne Pong. Reconnect: Backoff 0,5 s × 2ⁿ bis 30 s, ±20 % Jitter.

### Sync-Topologie

```
 Laptop: beton serve (Knoten L)  ══ node-Tunnel ══►  Team-Server (Knoten S)
   ses_A: Home = L (lokal gestartet)  ──repl.push──►   ses_A: Replica
   ses_B: Replica  ◄──repl.push──                     ses_B: Home = S (Runner auf K8s)
   Client am Handy → S → input.forward → L (Home von ses_A)
```

Verzeichnis der Home-Knoten (`sessions.home_node_id`, `epoch`) ist auf dem Team-Server autoritativ. Nur Sessions gemäß `sync.sessions: all | projects: […] | manual` werden repliziert; lokale Sessions verlassen den Rechner sonst nie.

### Budget-Lease-Algorithmus

```
Anfordern:   Knoten → S: budget.lease.request {scope: "user:usr_x/daily", requested_micro, unit}
Gewähren:    granted = min(requested, remaining(scope) × lease_fraction (0,5), max_lease (Default 10 USD))
             remaining(scope) -= granted   (Reservierung)
             → budget.lease.grant {lease_id, granted_micro, expires_at = min(now+24h, Periodenende)}
Verbrauchen: lokal je cost.delta: balance -= cost_micro
             online & balance < 20 % granted → Aufstockung (neue Anforderung, gleicher Scope)
             balance ≤ 0 → on_lease_exhausted: ask | deny (Event budget.exhausted) bis Reconnect/Aufstockung
             now > expires_at → Lease ungültig, Verhalten wie erschöpft
Melden:      online alle 60 s oder je 1 USD: budget.lease.report {lease_id, consumed_total_micro}  (kumulativ → idempotent)
             S bucht delta = consumed_total − zuletzt_gemeldet
Abrechnen:   bei Reconnect/Ablauf: budget.lease.settle {lease_id, consumed_total_micro, cost_event_refs}
             S bucht Rest, gibt unverbrauchte Reservierung frei, schließt Lease
             consumed > granted → Overrun voll gebucht + notify an Owner/Admin
             keine Meldung bis expires_at + settle_timeout (72 h) → Reservierung freigegeben, Lease „unsettled“;
             späte Meldung wird trotzdem gebucht (ggf. Overrun)
```

Beträge sind Ganzzahlen in Mikro-Einheiten (`1 USD = 1 000 000`; Geldbeträge sind immer USD, siehe USE-001); `unit ∈ {currency, tokens}` (Subscription-Budgets zählen Tokens).

### Konfliktfälle

| Fall | Verhalten |
| --- | --- |
| Zwei Clients senden gleichzeitig Input | Home ordnet nach Eingang in die Queue; beide erhalten `ack` mit `input_id`; keine Eingabe geht verloren |
| Zwei Clients lösen dieselbe Approval auf | Erste Auflösung gewinnt; die zweite erhält `nack 409 already_resolved` inkl. Entscheider |
| Duplikat nach Reconnect (Runner/Replica sendet erneut) | Dedup über `rseq` bzw. `(session_id, seq, id)`; identisches Event → ignoriert |
| Gleiche `seq`, anderes Event bei Replikation | Divergenz → SYNC-005 (Fork) |
| Replica erhält Kommando, Home offline | `nack 503 home_unreachable`; mit `queue_if_offline` Outbox (SYNC-008) |
| Erzwungene Übernahme, alter Home schrieb nicht weiter | Übernahme wird beim Reconnect bestätigt, kein Fork |
| Erzwungene Übernahme, beide schrieben | Zweig des im Verzeichnis gültigen Homes behält die Session-ID; der andere wird Fork `reason: divergence` |
| Übernahme angefragt während laufendem Turn | Wartet auf `idle` (max. 10 min) oder interrupt mit `force_interrupt` |
| Policy-Bundle während Offline-Phase geändert | Knoten nutzt Cache; neues Bundle greift ab der nächsten Hook-Auswertung nach Reconnect; `policy.decision` enthält `policy_set_hash` |
| Policy-Cache älter als `max_age` | „stale“: Tool-Calls und Model-Requests → `ask` (ohne Zuschauer `deny`) |
| Lease überzogen (teurer Einzel-Request) | Overrun gebucht, Notify; nächste Requests `ask`/`deny` |
| Session zentral gelöscht, lokaler Home offline | Beim Reconnect `gone`: lokale Kopie wird lokal-only (entkoppelt) + `notice`; Tombstone verhindert erneute Replikation |
| Share widerrufen während Client verbunden | Subscription geschlossen mit `4403` binnen 2 s |
| Uhrzeit-Abweichung zwischen Knoten | Ordnung nur über `seq`; `ts` vom Home vergeben; Lease-Ablauf mit 5 min Toleranz |
| Unbekannter Event-Typ (neuerer Knoten) | Wird opak gespeichert und weitergereicht; Clients ignorieren unbekannte Typen |

## Features

### PROTO-001 — Event-Envelope & ID-Schema
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Alle Session-Ereignisse verwenden die Envelope aus dem Design (`v, id, session_id, seq, ts, actor, type, payload, turn_id?, causation_id?, raw?`). IDs sind Präfix + ULID; `ts` ist RFC 3339 UTC mit Millisekunden; Payloads sind als Rust-Enum `EventPayload` (`#[serde(tag = "type", content = "payload")]`) in `beton-core` definiert.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Roundtrip-Property-Test: Jedes `EventPayload` serialisiert und deserialisiert verlustfrei.
  - [ ] AC2 — Eine ID mit falschem Präfix (z. B. `usr_` als `session_id`) wird beim Deserialisieren abgelehnt.
  - [ ] AC3 — `raw` ist optional, wird vor Persistenz redigiert (SEC-013, siehe 05-security-identity.md) und ist per Konfiguration (`events.store_raw: false`) abschaltbar.
  - [ ] AC4 — Payloads > 64 KiB werden automatisch als Blob gespeichert und als `payload_ref` referenziert; Clients erhalten den Inhalt über die Blob-API.
- **Abhängigkeiten:** —
- **Referenz:** ADR-0019

### PROTO-002 — Event-Katalog v1
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Der Katalog im Design ist die verbindliche Liste der Event-Typen v1. Alle Typen existieren ab M0 als Rust-Varianten mit Schema; Producer werden mit den jeweiligen Features (M0–M5) implementiert. Harness-Adapter bilden Vendor-Ereignisse auf diese Typen ab; nicht abbildbare Ereignisse werden als `harness.unmapped` mit `raw` geloggt statt verworfen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Test vergleicht die Typliste aus dem generierten Schema mit der Katalogtabelle dieses Kapitels (Markdown-Parser) und schlägt bei Abweichung fehl.
  - [ ] AC2 — Golden-Transcript-Tests (Claude, Codex) erzeugen für bekannte Vendor-Events ausschließlich Katalog-Typen; unbekannte erscheinen als `harness.unmapped`.
  - [ ] AC3 — Jedes dauerhafte Event hat genau ein `actor`-Feld gemäß Envelope; System-Events tragen eine `component`.
- **Abhängigkeiten:** PROTO-001, HAR-001 (siehe 01-harnesses.md)

### PROTO-003 — Transiente Events & Delta-Ringpuffer
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** `*.delta`, `presence.updated` und Kanal-Frames werden nicht ins Log geschrieben. Der Home-Knoten hält pro Session einen Ringpuffer transienter Events (10 000 Einträge oder 5 min) und den akkumulierten Zustand laufender Nachrichten/Tool-Outputs. Reconnectende Clients erhalten nach den dauerhaften Events einen Snapshot (`message.delta` mit vollem bisherigen Text und `snapshot: true`); mit dem zugehörigen `*.completed` ist der Zustand vollständig im Log.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach 1 000 `message.delta` und `message.completed` enthält die DB genau ein Event für diese Nachricht.
  - [ ] AC2 — Ein Client, der mitten in einer Nachricht reconnectet, sieht danach denselben Text wie ein durchgehend verbundener Client (E2E mit Fake-Harness).
  - [ ] AC3 — `tseq` ist pro Session und Epoch monoton; Clients verwerfen transiente Events mit `tseq` ≤ zuletzt gesehenem.
- **Abhängigkeiten:** PROTO-001

### PROTO-004 — WebSocket-Handshake & Versionsaushandlung
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Clients verbinden sich mit `/v1/ws` (Subprotokoll `beton.v1`) und senden `hello` mit ihrer Protokollversion `1.<minor>`. Jede Seite unterstützt die eigene und die vorherige Minor; ausgehandelt wird `min(client, server)`, sofern das ≥ `max(client, server) − 1` ist. Andernfalls schließt der Server mit `4400` und nennt die unterstützte Spanne. Authentisierung und Origin-Prüfung erfolgen vor dem Upgrade (AUTH-001, AUTH-003, siehe 05-security-identity.md). Das Kompatibilitätsfenster auf Produktebene beschreibt DIST-018.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Client 1.4 ↔ Server 1.3 und Client 1.3 ↔ Server 1.4 verbinden sich mit 1.3; Client 1.2 ↔ Server 1.4 erhält `4400` mit `{supported:{min:"1.3",max:"1.4"}}`.
  - [ ] AC2 — Ohne `hello` innerhalb von 10 s schließt der Server mit `4400`.
  - [ ] AC3 — (ab M3, sobald es ein erstes Release gibt) Kompatibilitätstests in CI fahren den aktuellen Server gegen den Client der Vorversion (und umgekehrt) mit derselben E2E-Suite.
- **Abhängigkeiten:** PROTO-001, AUTH-001, AUTH-003 (siehe 05-security-identity.md)
- **Referenz:** ADR-0019 (Versionsaushandlung)

### PROTO-005 — Attach & Resume ab seq
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** `attach {session_id, from_seq, transient?, tail?}` liefert alle dauerhaften Events mit `seq > from_seq` in Reihenfolge, dann `live {head_seq}`, dann Live-Events — lückenlos und ohne Duplikate über Replay und Live hinweg. `tail: N` liefert nur die letzten N Events plus `has_more` für schnelle UIs. Mehrere Sessions pro Verbindung sind möglich.
- **Details:** Während des Replays eintreffende Live-Events werden serverseitig gepuffert und nach dem Replay in Reihenfolge gesendet. `from_seq > head_seq` → `nack seq_ahead` (Client verwirft lokalen Zustand und attached neu ab 0, z. B. nach Divergenz-Fork). Eingaben des Nutzers stehen als `message.completed` mit `role: user` und `author` im Log, vor dem zugehörigen Turn.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Bei laufender Produktion von 100 Events/s liefert ein Attach ab `from_seq=0` eine lückenlose, duplikatfreie Folge bis zum Live-Betrieb (Test prüft `seq` streng +1).
  - [ ] AC2 — `from_seq=head_seq+5` liefert `nack` mit Code `seq_ahead`.
  - [ ] AC3 — Ein Client mit fehlender Leseberechtigung erhält `nack 403` und keine Events.
- **Abhängigkeiten:** PROTO-004, DATA-002

### PROTO-006 — Client-Kommandos über WebSocket
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Clients senden Aktionen als `cmd {id, session_id, name, args, idempotency_key?}`; der Server antwortet mit `ack {id, result}` oder `nack {id, problem}`. Jedes Kommando hat einen REST-Zwilling mit identischer Semantik und Autorisierung. Wiederholte `idempotency_key`s innerhalb von 24 h liefern das ursprüngliche Ergebnis.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein doppelt gesendetes `input.submit` mit gleichem `idempotency_key` erzeugt genau einen Queue-Eintrag; beide Antworten tragen dieselbe `input_id`.
  - [ ] AC2 — Ein unbekannter Kommandoname liefert `nack` mit `unknown_command`, die Verbindung bleibt offen.
  - [ ] AC3 — Ein contract-Test stellt sicher, dass zu jedem WS-Kommando ein REST-Endpunkt mit gleichem Args-Schema existiert.
- **Abhängigkeiten:** PROTO-004, PROTO-011

### PROTO-007 — Binärkanäle & Flusskontrolle
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Terminal-Bytes, Browser-Frames und Audio laufen als Binärframes `[ver u8][channel_id u32 BE][flags u8][payload]` im selben WebSocket. Kanäle werden per `open_channel` geöffnet (Autorisierung je Kind, z. B. Terminal-Input nur mit `drive`) und per `close_channel`/`channel_closed` beendet. Flusskontrolle ist kreditbasiert: Der Empfänger gewährt `credit {channel_id, bytes|frames}`; der Sender überschreitet das Fenster nie. Browser-Frames: max. 2 Frames unbestätigt, ältere werden verworfen (nur neuester zählt).
- **Details:** Browser-Payload: `u16 meta_len ‖ JSON {w, h, ts, format: jpeg|webp} ‖ Bilddaten`; Audio Client → Server: PCM s16le 16 kHz mono oder Opus. Browser-/Terminal-Eingaben gehen als Textkommandos (`browser.input`, Terminal-Input als Binärframe mit Flag `INPUT`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Terminal mit 50 MB Ausgabe an einen langsamen Client (gedrosselt auf 100 KB/s) erhöht den Server-Speicher um < 2 MB (Kredit stoppt den Sender, PTY-Lesen pausiert).
  - [ ] AC2 — Ein `view`-Client kann einen Terminal-Kanal lesend öffnen, Frames mit `INPUT`-Flag werden verworfen und mit `nack 403` beantwortet.
  - [ ] AC3 — Bei einem Client, der keine Browser-Frames bestätigt, sendet der Server nie mehr als 2 ausstehende Frames.
- **Abhängigkeiten:** PROTO-004, BRW-005 (siehe 09-browser.md), TUI-002, WEB-010 (siehe 08-clients.md), VOI-004 (siehe 11-platform-features.md)

### PROTO-008 — Backpressure & Batching
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Der Server bündelt Events (spätestens alle 16 ms bzw. 64 Events/256 KiB). Pro Verbindung ist die Ausgangswarteschlange begrenzt: Ab 1 MiB werden transiente Deltas zurückgehalten und, sobald die Warteschlange wieder unter 1 MiB liegt, als ein Snapshot je Strom (`snapshot: true`) nachgereicht; überschreiten dauerhafte Events 4 MiB, sendet der Server `overflow {session_id, resume_from}` und stoppt das Senden für diese Session, bis der Client neu attached. Eingehend: max. 100 Kommandos/s und 1 MiB pro Textframe.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein künstlich blockierter Client erhält `overflow`; nach erneutem `attach` ab `resume_from` ist sein Zustand vollständig.
  - [ ] AC2 — Ein langsamer Client verzögert andere Clients derselben Session nicht (p99-Latenz der schnellen Clients < 100 ms im Lasttest).
  - [ ] AC3 — Mehr als 100 Kommandos/s führen zu `nack 429`; dauerhaft (> 10 s) zu Close `4429`.
- **Abhängigkeiten:** PROTO-005

### PROTO-009 — Heartbeats & Reconnect
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Der Server sendet alle 20 s einen WS-Ping und schließt nach 60 s ohne Pong mit `4408`. Clients bauen Verbindungen bei Abbruch automatisch mit exponentiellem Backoff (0,5 s → 30 s, ±20 % Jitter) neu auf und attachen ab der zuletzt gesehenen `seq`. Beim geordneten Herunterfahren sendet der Server `4503`, worauf Clients sofort reconnecten, gleichverteilt über 2 s (so bleiben es bei 100 Clients höchstens 20 je 100 ms). Nach `4401`, `4403` oder `4404` verbinden Clients nicht neu.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Client ohne Pong wird nach spätestens 60 s getrennt; serverseitige Ressourcen der Verbindung sind danach freigegeben.
  - [ ] AC2 — Nach Server-Neustart sind 100 Clients binnen 35 s wieder verbunden, ohne dass mehr als 20 gleichzeitig im selben 100-ms-Fenster reconnecten.
  - [ ] AC3 — Nach Reconnect zeigt die Web-UI keine doppelten oder fehlenden Nachrichten (E2E mit erzwungenem Netzabbruch).
- **Abhängigkeiten:** PROTO-005

### PROTO-010 — REST-Konventionen
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** REST unter `/v1` ist ressourcenorientiert mit JSON in `snake_case`. Listen nutzen Cursor-Pagination (`?limit=1..200`, Default 50, `&cursor=<opak>` → `{items, next_cursor}`) über einen unveränderlichen Schlüssel (z. B. die zeitlich sortierte ID), damit neue Einträge keine Seiten verschieben; Event-Abfragen `GET /v1/sessions/{id}/events?after_seq=N&limit≤1000`. `POST` akzeptiert `Idempotency-Key` (24 h); veränderliche Konfigurationsobjekte (Agents, Policies, Projekte) nutzen `ETag`/`If-Match` (412 bei Konflikt). Geld als Ganzzahl-Mikro-Einheiten + Währung; Zeitpunkte RFC 3339 UTC; lange Operationen `202` + `Location: /v1/operations/{id}`; Rate-Limits mit `429`, `Retry-After` und `RateLimit-*`-Headern.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Paginieren über 1 000 Sessions mit `limit=50` liefert jede genau einmal, auch wenn währenddessen neue entstehen.
  - [ ] AC2 — Gleicher `Idempotency-Key` mit anderem Body liefert 422 `idempotency_key_reused`.
  - [ ] AC3 — `PUT` einer Policy mit veraltetem `If-Match` liefert 412 und verändert nichts.
  - [ ] AC4 — Ein Lint über das OpenAPI-Dokument erzwingt `snake_case`, Cursor-Pagination auf allen Listen-Endpunkten und Problem-Responses für 4xx/5xx.
- **Abhängigkeiten:** PROTO-011, PROTO-013

### PROTO-011 — Fehlerformat RFC 9457
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Alle Fehler (REST, WS-`nack`, Tunnel) sind Problem-Objekte: `type` (`urn:beton:problem:<code>`), `title`, `status`, `detail`, `instance`, `code`, `trace_id`, optional `errors: [{pointer, detail}]` für Validierung. Codes stammen aus einem Rust-Enum `ProblemCode`, aus dem eine Code-Referenz generiert wird ([`docs/generated/problem-codes.md`](../generated/problem-codes.md)). Problem-Details enthalten nie Secrets, Stacktraces oder interne Pfade.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Jede 4xx/5xx-Antwort hat `Content-Type: application/problem+json` (Contract-Test über alle Routen mit provozierten Fehlern).
  - [ ] AC2 — Validierungsfehler in einem Agent-YAML-Upload liefern JSON-Pointer auf das fehlerhafte Feld.
  - [ ] AC3 — Ein Panic in einem Handler liefert 500 mit `trace_id` und ohne Stacktrace im Body.
- **Abhängigkeiten:** PROTO-013

### PROTO-012 — SSE read-only
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Für Skripte bietet `GET /v1/sessions/{id}/events/stream` (`text/event-stream`) einen read-only Strom: `id: <seq>`, `event: <type>`, `data: <Envelope>`; Resume über `Last-Event-ID` oder `?from_seq=`. Transiente Events nur mit `?transient=true` (ohne `id:`-Zeile). Heartbeat-Kommentar `: hb` alle 15 s. Authentisierung per `Authorization`-Header (PAT) oder Same-Origin-Cookie. PROTO-012 ist Owner des SSE-Endpunkts; API-003 beschreibt nur die Skript-Sicht.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `curl -N -H "Last-Event-ID: 50" …/events/stream` liefert ab `seq 51` lückenlos.
  - [ ] AC2 — Über SSE ist keine Aktion möglich; der Endpunkt akzeptiert nur `GET`.
  - [ ] AC3 — Ohne Ereignisse kommt alle 15 s ein Heartbeat-Kommentar.
- **Abhängigkeiten:** PROTO-005

### PROTO-013 — Schema-Generierung aus Rust & Snapshot-Tests
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Rust-Typen in `beton-core`/`beton-proto` sind die einzige Quelle: `schemars` → JSON-Schema (`schemas/v1/{events,ws,tunnel}.schema.json`), `ts-rs`/`specta` → TypeScript (`packages/sdk-ts/src/gen/`, genutzt von `apps/web`), `utoipa` → OpenAPI 3.1 (`openapi/v1.json`). `cargo xtask codegen` erzeugt alles; generierte Dateien sind eingecheckt, `cargo xtask codegen --check` und `insta`-Snapshots laufen in CI.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Feldänderung in einem Payload ohne Neugenerierung lässt CI fehlschlagen (`--check` meldet Diff).
  - [ ] AC2 — `tsc` gegen die Web-UI schlägt fehl, wenn ein von der UI genutztes Feld im Rust-Typ entfernt wird.
  - [ ] AC3 — Die OpenAPI enthält für jede Route Request-/Response-Schemas und Problem-Responses; Validierung mit einem OpenAPI-3.1-Linter in CI.
- **Abhängigkeiten:** PROTO-001
- **Referenz:** ADR-0019, ADR-0031

### PROTO-014 — Kompatibilitätsprüfung der Schemas
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Ab dem ersten öffentlichen Release vergleicht CI die aktuellen Schemas mit denen des letzten Release-Tags. In einer Minor-Version erlaubt: neue optionale Felder, neue Event-Typen, neue Kommandos, neue Varianten offener Enums. Verboten (nur mit Major): Entfernen/Umbenennen von Feldern, Typänderungen, optional → required, Varianten geschlossener Enums entfernen. Clients müssen unbekannte Event-Typen und Felder ignorieren.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Das Entfernen eines Feldes aus `tool.call.completed` lässt den Kompatibilitäts-Check mit Hinweis „breaking: requires major“ fehlschlagen.
  - [ ] AC2 — Fuzz-Test: Web-UI-Reducer und Rust-SDK verarbeiten Streams mit eingestreuten unbekannten Event-Typen und Feldern ohne Fehler.
  - [ ] AC3 — Die Protokoll-Minor wird bei jeder additiven Änderung erhöht (Check vergleicht Schema-Hash und Versionskonstante).
- **Abhängigkeiten:** PROTO-013, QA-006 (siehe 12-distribution-quality.md)

### PROTO-015 — Tunnel-Protokoll Host/Runner → Server
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Hosts und Runner verbinden sich ausschließlich ausgehend über `/v1/tunnel` (lokal Unix-Socket, ab M4 auch remote über WSS) gemäß Design-Tabelle. Runner liefern Events ohne `seq` mit Runner-Sequenz `rseq`; der Home-Knoten vergibt `seq`, persistiert und bestätigt per `events.ack`. Nach einem Reconnect meldet `bound.acked_rseq` den gespeicherten Stand; erneut gesendete Events mit `rseq ≤ acked_rseq` verwirft der Server (genau einmal). Ein neuer Runner-Prozess (neues Runner-Token, z. B. nach `resume`) beginnt wieder bei `rseq` 1; der Server setzt den gespeicherten Stand dafür zurück. Tunnel- und WS-Nachrichten mit Events lesen Server und SDK so, dass `raw` byte-genau erhalten bleibt (intern getaggte Enums puffern sonst und verlieren `RawValue`); scheitert das Speichern eines Pushs, antwortet der Server mit `problem` statt ihn still zu verwerfen. Unbestätigte Events hält der Runner (begrenzt auf 64 MiB, darüber pausiert er den Harness) und sendet sie nach Reconnect erneut.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Wird der Tunnel während eines Turns 30 s getrennt, enthält das Log nach Reconnect alle Events genau einmal und in Runner-Reihenfolge.
  - [ ] AC2 — Ein Runner, der `events.push` mit veralteter `epoch` sendet, erhält ein Problem `stale_epoch` und stoppt das Schreiben.
  - [ ] AC3 — Der Host öffnet keinen lauschenden Port (Test: `ss -ltn`/`lsof` zeigt keine Listener des Host-/Runner-Prozesses außer Loopback-Proxy).
  - [ ] AC4 — (ab M4) Ein Remote-Host ohne gültiges Device-Token wird vor dem Upgrade mit 401 abgewiesen.
- **Abhängigkeiten:** PROTO-001, DATA-002, AUTH-001, AUTH-011 (ab M4, siehe 05-security-identity.md), RUN-002, RUN-004 (ab M4, siehe 10-runners-extensibility.md)
- **Referenz:** ADR-0003; Omnigent Runner-/Host-Tunnel (1.6)

### DATA-001 — Datenmodell & Entitäten
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Das relationale Modell umfasst: `Org`, `Team`, `User`, `Identity (issuer, sub)`, `Membership`, `ServiceAccount`, `Token`, `Device`, `Node`, `Host`, `Runner`, `Project`, `Session`, `Event`, `Blob`, `Agent` (versioniert), `Policy` (Scope org/team/user/project/agent, versioniert, ETag), `Secret`/`SecretRef` (Bindung + Krypto-Felder, SEC-005/SEC-007 in 05-security-identity.md), `Schedule`, `ScheduleRun`, `Timer`, `Comment`, `Share`, `Approval`, `BudgetLease`, `AuditEntry`, `Tombstone`. Alle Tabellen tragen `org_id` (außer `orgs` selbst); lokal existiert genau `org_local` mit `usr_local` (intern die Null-ULID, nach außen immer als `org_local`/`usr_local`). Das ER-Diagramm wird aus den Migrationen erzeugt: [`docs/generated/er-diagram.md`](../generated/er-diagram.md). In M0 sind die lokal benötigten Entitäten implementiert, Team-Entitäten ab M4.
- **Details:** `Session`: `id, org_id, owner (usr_|sa_), project_id?, parent_id?, kind, title, status, archived, harness, home_node_id, epoch, head_seq, created_at, updated_at`. `Comment`, `Approval`, `Share` sind Projektionen ihrer Events plus Indizes.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein ER-Diagramm wird aus den Migrationen generiert und im Repo eingecheckt (CI prüft Aktualität).
  - [ ] AC2 — Jede Abfrage in `beton-store` filtert nach `org_id` (Lint/Review-Regel; Test: Daten einer zweiten Org sind über keine Repository-Funktion erreichbar).
  - [ ] AC3 — Fremdschlüssel sind aktiv (SQLite `foreign_keys=ON`); Löschen eines Users mit Sessions ohne Übertragung schlägt fehl.
- **Abhängigkeiten:** —

### DATA-002 — Event-Log-Speicherung (Append-only)
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Events liegen in `events(org_id, session_id, seq, id, ts, actor_kind, actor_id, actor, type, payload, payload_ref, turn_id, causation_id, epoch, redacted, PRIMARY KEY (session_id, seq))` (`actor` ist das vollständige Actor-Objekt; genau eines von `payload`/`payload_ref` ist gesetzt); `raw` separat in `event_raw` mit eigener Retention. Anhängen geschieht in einer Transaktion mit optimistischer Prüfung `UPDATE sessions SET head_seq = head_seq + 1 WHERE id = ? AND head_seq = ? AND epoch = ?` — so bleibt `seq` lückenlos und nur ein Schreiber erfolgreich. Updates/Deletes einzelner Events sind außer Redaktion (DATA-012) und Session-Löschung nicht vorgesehen. Schreibtransaktionen beginnen mit `BEGIN IMMEDIATE` (SQLite), damit gleichzeitige Schreiber nicht mit `SQLITE_BUSY` scheitern.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Zwei konkurrierende Appends mit gleicher erwarteter `head_seq` → genau einer gelingt, der andere erhält `seq_conflict`.
  - [ ] AC2 — Nach Kill des Prozesses während Appends (Crash-Test, 1 000 Iterationen) ist das Log lückenlos und `head_seq` = max(`seq`).
  - [ ] AC3 — Durchsatz SQLite lokal ≥ 5 000 Events/s (Batch-Append), gemessen im Benchmark in CI.
- **Abhängigkeiten:** DATA-001

### DATA-003 — SQLite-Backend & Migrationen
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Lokal nutzt `beton-store` SQLite via `sqlx` (`~/.beton/beton.db`, WAL, `synchronous=FULL`, `busy_timeout=5000`, `foreign_keys=ON`). Migrationen liegen in `crates/beton-store/migrations/{sqlite,postgres}/NNNN_<name>.sql` mit identischer Nummerierung, sind ins Binary eingebettet und laufen beim Start unter Lock. Vor jeder Migration wird `beton.db.bak-<version>` angelegt; Down-Migrationen gibt es nicht; ein älteres Binary verweigert eine DB mit neuerer Schema-Version.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Paritätstest stellt sicher, dass jede Migrationsnummer in beiden Dialekten existiert und die resultierenden Schemas (Tabellen, Spalten, Indizes) äquivalent sind.
  - [ ] AC2 — Ein Binary der Vorversion startet gegen eine migrierte DB nicht und meldet `schema_too_new`.
  - [ ] AC3 — Zwei gleichzeitig startende Daemons migrieren nicht doppelt (Lock-Test).
- **Abhängigkeiten:** DATA-001
- **Referenz:** ADR-0009

### DATA-004 — Postgres-Backend
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Zentral nutzt dieselbe Repository-Schicht Postgres (≥ 15) via `sqlx` mit `JSONB`-Payloads, Connection-Pool und Advisory-Lock für Migrationen. Alle Repository-Tests laufen in CI gegen beide Dialekte. v1 garantiert den Betrieb mit einer aktiven Server-Instanz pro Datenbank *(Annahme)*.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Die gesamte `beton-store`-Testsuite läuft grün gegen SQLite und Postgres (CI-Matrix).
  - [ ] AC2 — Ein Start mit erreichbarem, aber migrationsgesperrtem Postgres wartet bis 60 s auf den Lock und bricht danach mit klarer Meldung ab.
  - [ ] AC3 — Event-Abfragen `after_seq` nutzen den Primärschlüssel-Index (EXPLAIN-Test, keine Sequenzscans bei 10 Mio. Events).
- **Abhängigkeiten:** DATA-003

### DATA-005 — Projektionen & Read-Modelle
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Session-Liste (Titel, Status, `head_seq`, Kostensumme, letzte Aktivität), offene Approvals, Usage-Aggregate (Tag × Harness × Modell), Kommentare und Inbox werden aus Events abgeleitet; Kernprojektionen werden in derselben Transaktion wie der Event-Append aktualisiert. `beton admin projections rebuild [--session]` baut sie aus dem Log neu auf.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach `projections rebuild` sind alle Projektionstabellen identisch mit dem Zustand vor dem Rebuild (Test über Dump-Vergleich).
  - [ ] AC2 — (ab M2, USE-001) Ein `cost.delta` ist unmittelbar nach dem Append in der Usage-Projektion sichtbar (gleiche Transaktion).
- **Abhängigkeiten:** DATA-002, USE-001 (ab M2, siehe 11-platform-features.md)

### DATA-006 — Blob-Store: Dateisystem
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Große Artefakte (Anhänge, ausgelagerte Payloads/Tool-Ergebnisse, Screenshots, Terminal-Snapshots, Agent-Bundles, Exporte) liegen inhaltsadressiert unter `~/.beton/blobs/sha256/<ab>/<cd>/<hash>` (0600). Der Zugriff über die API erfolgt nur über die referenzierende Ressource (z. B. `GET /v1/sessions/{id}/blobs/{hash}`) mit deren Autorisierung — nie über den Hash allein. Garbage Collection per Mark-and-Sweep (täglich; unreferenziert und älter als 24 h → gelöscht). Max. Blob-Größe Default 512 MiB.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein User ohne Zugriff auf Session A erhält für `GET /v1/sessions/A/blobs/{hash}` 403, auch wenn derselbe Inhalt in einer eigenen Session existiert; es gibt keinen Endpunkt, der Blobs allein per Hash ausliefert.
  - [ ] AC2 — Nach Löschen der einzigen referenzierenden Session ist der Blob nach dem nächsten GC-Lauf (+24 h) entfernt.
  - [ ] AC3 — Eine beschädigte Blob-Datei (Hash stimmt nicht) wird beim Lesen erkannt und mit 500 `blob_corrupt` gemeldet.
- **Abhängigkeiten:** DATA-001
- **Referenz:** ADR-0009

### DATA-007 — Blob-Store: S3-kompatibel
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Zentral liegen Blobs in einem S3-kompatiblen Bucket (`blobs.uri: s3://bucket/prefix`, AWS S3, MinIO, R2), optional mit SSE (`sse: s3|kms`). Downloads an Clients erfolgen über kurzlebige Presigned-URLs (5 min) nach Autorisierung durch den Server oder gestreamt durch den Server (`blobs.delivery: presign|proxy`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Die Blob-Store-Testsuite läuft gegen FS und MinIO (CI-Container) mit identischen Ergebnissen.
  - [ ] AC2 — Eine Presigned-URL ist nach 5 min ungültig; die URL wird nicht geloggt.
  - [ ] AC3 — `beton admin blobs migrate --from fs --to s3` überträgt alle Blobs idempotent und verifiziert Hashes.
- **Abhängigkeiten:** DATA-006, DATA-004

### DATA-008 — Archivieren & Löschen
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Archivieren ist reversibel (Flag + Event). Löschen einer Session entfernt Events, `event_raw`, Projektionseinträge und Blob-Referenzen in einer Transaktion und hinterlässt einen `Tombstone(kind, id, owner, deleted_at, deleted_by)`, der über Sync propagiert wird und erneute Replikation verhindert; Blobs verschwinden mit dem nächsten GC. Side-Chats und Sub-Sessions werden mitgelöscht (SES-001, COL-008, siehe 07-sessions-collaboration.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach Löschen liefern alle Event-, Blob- und Session-Endpunkte 404; der Tombstone ist über `GET /v1/tombstones?since=` (Admin/Sync) sichtbar.
  - [ ] AC2 — Eine Replica, die einen Tombstone erhält, löscht ihre Kopie ebenfalls.
  - [ ] AC3 — Löschen ist nur dem Owner (bzw. Org-Admin für Offboarding) erlaubt und wird auditiert.
- **Abhängigkeiten:** DATA-002, DATA-006

### DATA-009 — Retention-Regeln
- **Meilenstein:** M4 · **Priorität:** Should
- **Beschreibung:** Pro Org (und überschreibbar pro Projekt, nur kürzer) sind Aufbewahrungsfristen konfigurierbar: `retention.sessions` (Default unbegrenzt), `retention.archived_sessions`, `retention.raw` (Default 30 Tage), `retention.snapshots` (Terminal-/Browser-Snapshots, Default 90 Tage). Ein täglicher Job löscht gemäß DATA-008 bzw. entfernt Teil-Daten (raw, Snapshot-Blobs) unter Erhalt der Events.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit `retention.raw: 1d` ist `event_raw` für Events älter als 1 Tag nach dem Job-Lauf leer, Events selbst bleiben.
  - [ ] AC2 — Eine Projekt-Retention länger als die Org-Retention wird abgelehnt.
  - [ ] AC3 — Der Job ist idempotent und meldet gelöschte Mengen als Metrik (OBS-004, siehe 11-platform-features.md).
- **Abhängigkeiten:** DATA-008

### DATA-010 — Export-/Import-Format (JSONL)
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Das Austauschformat für den Session-Export/-Import (SES-009, siehe 07-sessions-collaboration.md); DATA-010 ist Owner von Format und Import-/Export-Implementierung, die CLI-Front ist CLI-007: Zeile 1 `{"type":"beton.export","format_version":1,"exported_at","session":{…Metadaten}}`, danach je Zeile ein Event als Envelope in `seq`-Reihenfolge. Mit Blobs als `.tar.zst` (`session.jsonl` + `blobs/<sha256>`). Import validiert jede Zeile gegen das Schema, vergibt eine neue Session-ID (`imported_from` in Metadaten), behält `seq`, mappt unbekannte Actors auf `system`/`import` und ist anhand des Export-Hashes idempotent.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Export → Import → Export ergibt (bis auf Session-ID, Zeitstempel des Exports und Actor-Mapping) identische Event-Zeilen.
  - [ ] AC2 — Eine Datei mit Lücke in `seq` oder ungültiger Zeile wird vollständig abgelehnt, mit Zeilennummer im Problem.
  - [ ] AC3 — Exporte enthalten nur redigierte Payloads; `raw` (ebenfalls redigiert) ist nur mit `--with-raw` enthalten (Default: weggelassen).
  - [ ] AC4 — Ein Export-Format `format_version: 2` wird von einer v1-Implementierung mit `unsupported_format_version` abgelehnt.
- **Abhängigkeiten:** DATA-002, DATA-006, SES-009 (siehe 07-sessions-collaboration.md), CLI-007 (siehe 08-clients.md)

### DATA-011 — Ephemere Daten & Snapshots
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Terminal-Bytes, Browser-Frames, Audio und Presence werden nie dauerhaft gespeichert. Stattdessen entstehen Snapshots: `terminal.snapshot` (serialisierter Bildschirmzustand des Terminal-Emulators als Blob) beim Schließen und bei Aktivität höchstens alle 10 s; `browser.snapshot` (Screenshot + optional A11y-Baum) nach Navigation, nach Agent-Aktionen und auf Anforderung. Audio wird nach Transkription verworfen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach einer Sitzung mit 100 MB Terminal-Ausgabe enthält das Log keine `terminal.output`-Events und höchstens 6 Snapshots pro Minute Aktivität.
  - [ ] AC2 — Ein später attachender Client sieht für ein geschlossenes Terminal den letzten Bildschirmzustand.
  - [ ] AC3 — Weder DB noch Blob-Store enthalten Audio-Daten nach Abschluss einer Spracheingabe (Test durchsucht nach WAV/Opus-Signaturen).
- **Abhängigkeiten:** PROTO-007, DATA-006

### DATA-012 — Payload-Redaktion
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Die einzige erlaubte Änderung bestehender Events: Owner oder Admin ersetzen einen Payload (ganz oder per JSON-Pointer-Pfade) durch `[REDACTED:manual]`, z. B. nach einem entdeckten Secret-Leak. `seq`, `type`, `actor`, `ts` bleiben; `redacted=true`; `raw` und ausgelagerte Blobs werden gelöscht; ein dauerhaftes `event.redacted` dokumentiert den Vorgang und repliziert ihn. Abgrenzung: Die automatische Redaction vor Persistenz ist SEC-013; DATA-012 ist die manuelle Nachredaktion.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `POST /v1/sessions/{id}/events/{seq}/redact` ersetzt den Inhalt; neue Attaches und Exporte zeigen die redigierte Fassung.
  - [ ] AC2 — Verbundene Clients erhalten `event.redacted` und ersetzen die Anzeige ohne Reload.
  - [ ] AC3 — Die Aktion erzeugt einen Audit-Eintrag (SEC-012, siehe 05-security-identity.md); ein `view`-User erhält 403.
- **Abhängigkeiten:** DATA-002, SEC-012, SEC-013 (siehe 05-security-identity.md)

### SYNC-001 — Home-Knoten & Single-Writer mit Epochen
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Jede Session hat genau einen Home-Knoten, der `seq` vergibt; Default ist der Knoten, auf dem sie erzeugt wurde. Home und `epoch` stehen im Verzeichnis des Team-Servers. Jede Schreiboperation trägt die `epoch`; Knoten und Server lehnen Schreibvorgänge mit veralteter `epoch` ab (Fencing). Lokal-only-Betrieb kennt nur einen Knoten und ist davon unberührt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Knoten mit veralteter `epoch` kann kein Event mehr in die Session schreiben (`stale_epoch`), auch wenn er die höchste `seq` kennt.
  - [ ] AC2 — `GET /v1/sessions/{id}` zeigt `home_node`, `epoch` und `sync_state: synced|behind|offline`.
  - [ ] AC3 — Ohne `sync.server`-Konfiguration verlässt kein Event den lokalen Knoten (Netzwerk-Mitschnitt im Test).
- **Abhängigkeiten:** DATA-002, PROTO-015
- **Referenz:** ADR-0010

### SYNC-002 — Replikation & Read-Replicas
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Der Home-Knoten repliziert dauerhafte Events per `repl.push {session_id, epoch, events}` (mit `seq`) an den Team-Server, der sie an weitere Interessenten (andere Knoten via `repl.subscribe {from_seq}`, Clients) verteilt; transiente Events werden live weitergereicht, nicht gespeichert. Replicas bestätigen mit `repl.ack {upto_seq}`; der Home-Knoten sendet nach Reconnect ab der letzten bestätigten `seq`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine auf dem Laptop laufende Session ist für einen Team-Kollegen mit `view`-Share in der Web-UI des Servers live sichtbar (p95-Latenz < 500 ms im LAN-Test).
  - [ ] AC2 — Nach 2 h Offline-Betrieb und Reconnect enthält die Server-Replica alle Events in identischer Reihenfolge und mit identischen `seq`/`id`.
  - [ ] AC3 — Eine Replica mit abweichendem Event bei gleicher `seq` löst SYNC-005 aus, statt still zu überschreiben.
- **Abhängigkeiten:** SYNC-001

### SYNC-003 — Input-Forwarding an den Home-Knoten
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Kommandos (Input, Queue-Operationen, Approvals, Interrupt, Kommentare), die an einer Replica eingehen, werden autorisiert und per `input.forward {cmd_id, session_id, actor, cmd, idempotency_key}` an den Home-Knoten weitergeleitet; das Ergebnis geht als `ack`/`nack` an den Client zurück, die Wirkung erscheint als Events. Der Home-Knoten prüft die Rechte erneut anhand der replizierten Shares.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Approval vom Handy (verbunden mit dem Team-Server) für eine auf dem Laptop gehostete Session wird binnen 2 s auf dem Laptop wirksam.
  - [ ] AC2 — Ein vom Server weitergeleitetes Kommando eines Users ohne `drive`-Share wird vom Home-Knoten mit 403 abgelehnt, auch wenn der Server es durchließe (Fehlerinjektion im Test).
  - [ ] AC3 — Ist der Home-Knoten offline, erhält der Client `nack 503 home_unreachable` mit Angabe des Home-Knotens.
- **Abhängigkeiten:** SYNC-002, AUTH-015 (siehe 05-security-identity.md)

### SYNC-004 — Geplante Ownership-Übernahme
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Ein Knoten fordert den Home einer Session an (`beton session take <id>` bzw. UI „Hierher übernehmen“, z. B. vor einer Zugfahrt). Der bisherige Home wartet auf `idle` (max. 10 min, oder interrupt mit `force_interrupt`), sendet alle Events, gibt die Ownership mit `epoch+1` ab; der Server aktualisiert das Verzeichnis; der neue Home übernimmt erst, wenn er `head_seq` vollständig besitzt. Danach kann der neue Home offline weiterarbeiten. Laufende Runner werden auf dem neuen Home-Knoten neu gestartet (Resume, SES-003 in 07-sessions-collaboration.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach Übernahme schreibt nur noch der neue Home; ein Schreibversuch des alten liefert `stale_epoch`.
  - [ ] AC2 — Eine Übernahme während eines Turns ohne `force_interrupt` wartet bis `turn.completed`; mit `force_interrupt` folgt `turn.interrupted` vor der Übergabe.
  - [ ] AC3 — Das Log enthält `session.ownership_changed {mode: planned}` mit neuer `epoch`.
  - [ ] AC4 — Nur Owner (oder Org-Admin) dürfen die Übernahme anfordern.
- **Abhängigkeiten:** SYNC-001, SYNC-002, SES-003 (siehe 07-sessions-collaboration.md)

### SYNC-005 — Erzwungene Übernahme & Divergenz → automatischer Fork
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Ist der Home nicht erreichbar, kann der Owner die Übernahme erzwingen (`--force`); der Knoten schreibt dann vorläufig mit `epoch+1, forced=true, base_seq`. Beim Reconnect gilt: Hat der bisherige Home seit `base_seq` nichts geschrieben, wird die Übernahme bestätigt. Andernfalls behält der im Server-Verzeichnis gültige Zweig die Session-ID, und die Events des anderen Zweigs nach `base_seq` werden automatisch in eine neue Session übertragen (`session.forked {reason: divergence, at_seq: base_seq}`); der Owner erhält einen Inbox-Hinweis. Es wird nie gemergt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Szenario „beide schrieben“: Nach Reconnect existieren zwei Sessions; die ursprüngliche ID trägt den Server-Zweig, der Fork enthält Events 1…`base_seq` plus die lokalen Events; kein Event ist verloren.
  - [ ] AC2 — Szenario „nur lokal geschrieben“: Kein Fork, `session.ownership_changed {mode: forced}` wird bestätigt.
  - [ ] AC3 — Clients mit lokalem Zustand jenseits der gültigen `head_seq` erhalten `seq_ahead` und laden neu.
  - [ ] AC4 — Deterministischer Simulationstest (zufällige Partitionen, 10 000 Läufe) findet keinen Fall mit zwei Schreibern derselben `epoch` oder verlorenen Events.
- **Abhängigkeiten:** SYNC-004, SES-006 (siehe 07-sessions-collaboration.md)
- **Referenz:** ADR-0010 (Fork als Notfallpfad)

### SYNC-006 — Policy-Cache (nur verschärfbar)
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Der Server verteilt server-autoritative Policies (Org/Team/User) als signiertes Bundle `policy.bundle {org_id, version, hash, issued_at, max_age, sig (Ed25519), rules}` an Knoten und Runner; diese cachen es persistent. Die lokale Auswertung kombiniert Bundle und lokale Ebenen (User/Projekt/Agent) mit „strengere Regel gewinnt“ (POL-007, POL-008, siehe 03-policies.md), sodass lokale Regeln nur verschärfen können. Ältere Bundle-Versionen werden nicht akzeptiert (Rollback-Schutz). Nach `max_age` (Default 7 Tage) ohne Aktualisierung gilt der Modus „stale“ (siehe Konfliktfälle).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine lokale Projekt-Policy `allow` für ein vom Org-Bundle mit `deny` belegtes Kommando führt zu `deny` (deklarativer Policy-Test).
  - [ ] AC2 — Ein Bundle mit ungültiger Signatur oder niedrigerer Version als die gecachte wird verworfen und geloggt.
  - [ ] AC3 — Offline mit gültigem Cache werden Policies weiter durchgesetzt; nach Überschreiten von `max_age` erzeugt jeder Tool-Call eine Approval (`ask`).
  - [ ] AC4 — Die UI zeigt Server-Policies lokal nur lesend an.
- **Abhängigkeiten:** SYNC-001, POL-007, POL-008 (siehe 03-policies.md)

### SYNC-007 — Budget-Leases
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Server-seitige Budgets (Org/Team/User, POL-012 in 03-policies.md) werden für Knoten, die Kosten lokal verursachen, über Leases gemäß Algorithmus im Design durchgesetzt: Anforderung, Reservierung, lokaler Verbrauch, periodische kumulative Meldung, Abrechnung bei Reconnect oder Ablauf, Overrun-Buchung. Ist die Lease erschöpft oder abgelaufen und keine Aufstockung möglich, greift `on_lease_exhausted: ask | deny` bis zum Reconnect. SYNC-007 ist Owner von Lease-Protokoll und Abrechnung; die Policy-Auswertung offline (Variablen `budget.*`, `on_lease_exhausted`) beschreibt POL-021.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Bei Restbudget 30 USD und `lease_fraction 0,5`, `max_lease 10 USD` erhält ein Knoten 10 USD; das Server-Restbudget zeigt 20 USD verfügbar + 10 USD reserviert.
  - [ ] AC2 — Offline: Nach Verbrauch von 10 USD erzeugt der nächste Model-Request `budget.exhausted` und eine Approval (bzw. `deny` bei `on_lease_exhausted: deny`).
  - [ ] AC3 — Doppelt gesendete `budget.lease.report` mit gleichem `consumed_total` buchen nichts doppelt.
  - [ ] AC4 — Reconnect nach Verbrauch von 7 USD einer 10-USD-Lease: Server bucht 7 USD, gibt 3 USD frei, Lease ist `settled`.
  - [ ] AC5 — Ein Knoten, der bis `expires_at + 72 h` nicht meldet, verliert die Reservierung (Lease `unsettled`); eine spätere Meldung wird gebucht und ein Overrun gemeldet, falls das Budget dadurch überschritten wird.
- **Abhängigkeiten:** SYNC-001, POL-012, POL-021 (siehe 03-policies.md), USE-001 (siehe 11-platform-features.md)
- **Referenz:** ADR-0010

### SYNC-008 — Offline-Outbox & Reconnect-Verhalten
- **Meilenstein:** M4 · **Priorität:** Should
- **Beschreibung:** Clients können Kommandos mit `queue_if_offline: true` senden; ist der Home-Knoten nicht erreichbar, speichert die Replica sie in einer Outbox (TTL 24 h, Reihenfolge erhalten) und liefert sie nach Reconnect an den Home. Approvals werden dabei nur zugestellt, wenn sie noch offen und nicht abgelaufen sind; abgelaufene Einträge werden mit Hinweis an den Absender verworfen. Die UI zeigt Outbox-Einträge als „ausstehend“.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Input an eine offline gehostete Session mit `queue_if_offline` erscheint nach Reconnect des Homes genau einmal in der Queue.
  - [ ] AC2 — Eine Approval-Entscheidung aus der Outbox für eine inzwischen per Timeout entschiedene Approval wird verworfen; der Absender erhält eine Inbox-Notiz.
  - [ ] AC3 — Outbox-Einträge älter als 24 h werden verworfen und gemeldet.
- **Abhängigkeiten:** SYNC-003

## Nicht in v1

- **Öffentliche (login-freie) Share-Links** und damit anonymer Lesezugriff auf Event-Streams — v2.
- **Python-SDK** für das Protokoll — v2 (v1: TypeScript- und Rust-SDK, API-004/API-005 in 08-clients.md).
