# 07 — Sessions, Collaboration & Git-Provider

Dieses Kapitel spezifiziert alles, was eine **Session** über ihren Lebenszyklus hinweg ausmacht (Erzeugen, Live-Streamen an mehrere Clients, Queue/Steer, Interrupt, Fork, Import/Export, Titel, Kontext, Projects, Worktrees, Workspace-Panel), die **Team-Collaboration** auf derselben Live-Session (Freigaben, Co-Drive, Presence, Inline-Kommentare, Side-Chats, Benachrichtigungen) sowie die **Git-Provider-Integration** (GitHub inkl. Enterprise, GitLab inkl. self-hosted) mit PR-/MR-Panel.

Scope-Grundlage ist ADR-0014 (Collaboration-Scope). Clients, die diese Funktionen darstellen, stehen in 08-clients.md (ADR-0015, ADR-0020); der eingebettete Browser in 09-browser.md (ADR-0016). Event-Modell und Transport sind in PROTO (Event-Katalog PROTO-002, siehe 06-data-sync-protocol.md) verbindlich definiert; die hier genannten Event-Namen sind dort übernommen. Meilensteine nach ADR-0030: Session-Basics M0/M1, Worktrees/Projects M3, Team-Collaboration und Git-Provider-Panel M4.

## Konzepte & Begriffe

| Begriff | Bedeutung |
| --- | --- |
| **Session** | Append-only-Log typisierter Events (ADR-0009) plus Metadaten (Titel, Projekt, Harness, Status). ID-Format `ses_<ulid>` *(Annahme)*. Gehört genau einem **Owner**. |
| **Runner** | Prozess, der eine Session ausführt und den Harness kapselt (RUN-003, siehe 10-runners-extensibility.md). Eine Session kann ohne laufenden Runner existieren (Status `stopped`). |
| **Client** | Jede Oberfläche (Web, Desktop, TUI, CLI, SDK), die per WebSocket an eine Session angeschlossen ist. Beliebig viele Clients pro Session. |
| **attach** | Client verbindet sich mit einer Session; erhält Replay ab `seq` und danach Live-Events. |
| **resume** | Runner für eine gestoppte Session neu starten und Harness-Zustand wiederherstellen. |
| **Turn** | Ein Durchlauf Agent-Loop vom Input bis `turn.completed` (oder Abbruch). |
| **Queue** | Serverseitige, geordnete Liste von Folge-Inputs, die nach Turn-Ende automatisch abgearbeitet wird. |
| **Steer** | Input, der in den **laufenden** Turn eingespeist wird, statt zu warten. |
| **Fork** | Neue Session, deren Historie bis Event `seq = X` aus einer Quell-Session übernommen wird; optional auf anderem Harness. |
| **Handover-Kontext** | Strukturierter Kontextblock, mit dem ein Ziel-Harness ohne native Fork-History-Fähigkeit die Vorgeschichte erhält. |
| **Side-Chat** | Versteckter Fork einer Session für Nebenfragen, sichtbar als Tab im Workspace-Rail der Eltern-Session. |
| **Project** | Gruppierung von Sessions mit Defaults und eigener Policy-Ebene (User → **Projekt** → Agent, ADR-0008). |
| **Worktree** | Eigenes `git worktree` + Branch pro Session, damit parallele Sessions sich nicht überschreiben. |
| **Share / Rolle** | Freigabe einer Session an User/Team mit Rolle `view`, `comment_approve` (Anzeige „comment+approve“) oder `drive` (ADR-0011). |
| **Co-Drive** | Mehrere Personen mit `drive` steuern dieselbe Session; ihre Eingaben werden im Modell-Kontext attribuiert. |
| **Anchor** | Ankerpunkt eines Inline-Kommentars: Nachricht, Datei-Zeilenbereich oder Diff-Zeile. |
| **Change Request (CR)** | Neutraler Oberbegriff für GitHub-Pull-Request und GitLab-Merge-Request. |
| **Git-Provider** | Implementierung des `GitProvider`-Traits (`beton-git`), eingebaut: `github`, `gitlab`; weitere als Plugin (PLG-002, siehe 10-runners-extensibility.md). |

## Design

### Session-Zustandsautomat

```
            create
              │
              ▼
  ┌──────► starting ──(runner ready)──► idle ◄──────────────┐
  │           │                         │  ▲                 │
  │       (fehler)                (input)  (turn.completed /  │
  │           ▼                         ▼  turn.interrupted)  │
  │        failed                    running ──(ask)──► waiting_approval
  │                                     │                     │ (kein Viewer + Timeout)
  │ resume                     (runner stop / idle-timeout)   ▼
  └──────── stopped ◄───────────────────┘                  paused
                │ archive / unarchive
                ▼
             archived ──(delete)──► deleted (Tombstone, Log gelöscht)
```

Status wird als Event `session.status` (PROTO-002) mit Feld `status ∈ {starting, idle, running, waiting_approval, paused, stopped, failed}` geloggt; `archived` ist ein orthogonales Flag (`session.archived`/`session.unarchived`).

### Datenfluss Input → Harness

```
Client A ─┐  input.submit {mode: queue|steer, text, attachments, author}
Client B ─┼──► Server (Home-Knoten, Single-Writer) ── Policy-Hook ──► Queue
SDK     ──┘                                                            │
                         idle? ─ ja ─► dequeue ──► Runner ──► Harness  │
                         steer + capability.steering ─────────────────┘
Runner ─► Events (message.delta, tool.call.*, fs.changed …) ─► Event-Log ─► alle Clients (WS, Resume ab seq)
```

### Rollenmatrix (COL)

| Aktion | view | comment_approve | drive | owner |
| --- | :-: | :-: | :-: | :-: |
| Stream & Historie lesen | ✔ | ✔ | ✔ | ✔ |
| Workspace-Dateien/Diffs sehen | Toggle¹ | ✔ | ✔ | ✔ |
| Inline-Kommentare schreiben | – | ✔ | ✔ | ✔ |
| Approvals/Fragen beantworten | – | ✔ | ✔ | ✔ |
| Input senden, Queue/Steer, Interrupt, Kommentar an Agent adressieren | – | – | ✔ | ✔ |
| Terminals öffnen, Browser-Input, Modell/Effort wechseln, auf anderen Harness forken | – | – | ✔ | ✔ |
| Fork (nur Historie, in eigene Session) | ✔ | ✔ | ✔ | ✔ |
| Freigaben verwalten, archivieren, löschen, Projekt ändern | – | – | – | ✔ |

¹ Owner-Schalter `share.workspace_files` (Default: aus) pro Session.

### Git-Provider-Trait (Skizze, `beton-git`)

```rust
#[async_trait::async_trait]
pub trait GitProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;                 // Github | Gitlab | Plugin(String)
    fn capabilities(&self) -> ProviderCaps;          // draft_cr, review_threads, checks, etag_polling …
    /// Ordnet eine Remote-URL einem Repo dieses Providers zu (Host-Matching inkl. Enterprise/self-hosted).
    fn match_remote(&self, remote: &GitRemote) -> Option<RepoRef>;
    async fn get_cr(&self, ctx: &ProviderCtx, repo: &RepoRef, id: CrNumber) -> Result<ChangeRequest>;
    async fn find_crs_for_branch(&self, ctx: &ProviderCtx, repo: &RepoRef, branch: &str) -> Result<Vec<CrSummary>>;
    async fn create_cr(&self, ctx: &ProviderCtx, repo: &RepoRef, req: NewChangeRequest) -> Result<ChangeRequest>;
    async fn list_checks(&self, ctx: &ProviderCtx, repo: &RepoRef, cr: &ChangeRequest) -> Result<Vec<Check>>;
    async fn check_log_tail(&self, ctx: &ProviderCtx, repo: &RepoRef, check: &CheckId, lines: u32) -> Result<String>;
    async fn list_review_threads(&self, ctx: &ProviderCtx, repo: &RepoRef, cr: &ChangeRequest) -> Result<Vec<ReviewThread>>;
    async fn reply_review_thread(&self, ctx: &ProviderCtx, repo: &RepoRef, thread: &ThreadId, body: &str) -> Result<()>;
    async fn get_diff(&self, ctx: &ProviderCtx, repo: &RepoRef, cr: &ChangeRequest) -> Result<UnifiedDiff>;
}
// ProviderCtx trägt: acting user, Credential-Handle (nie Klartext außerhalb beton-secrets), Conditional-Request-Cache (ETag).
```

## Features

### SES-001 — Session-Lebenszyklus (create / archive / delete)
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Sessions werden per API/CLI/UI erzeugt (Harness oder Agent, Arbeitsverzeichnis, optional Projekt), archiviert, wiederhergestellt und gelöscht. Archivieren stoppt einen laufenden Runner und blendet die Session aus der Standardliste aus; Löschen entfernt Event-Log und Blobs und hinterlässt einen Tombstone (ID, Owner, Löschzeitpunkt) für Sync-Konsistenz.
- **Details:** `POST /v1/sessions {target, cwd, project_id?, title?, harness_opts}` bzw. mit Agent `{agent, target?, params?, …}` (`target` ist dann ein Harness-Override, AGT-004, AGT-010); `POST …/archive|unarchive`; `DELETE …` (nur Owner). Löschen räumt den Worktree gemäß SES-016 auf.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given `beton run claude`, then existiert eine Session mit Status `starting`→`idle` und erstem Event `session.created` (seq 1), gefolgt von `session.started`.
  - [ ] AC2 — When eine laufende Session archiviert wird, then wird der Runner beendet, Status `stopped`, Flag `archived=true`, und sie fehlt in `GET /v1/sessions` ohne `?archived=true`.
  - [ ] AC3 — When eine Session gelöscht wird, then liefern `GET /v1/sessions/{id}` und Event-Abfragen `404`, Blobs sind entfernt, ein Tombstone ist vorhanden.
  - [ ] AC4 — Lifecycle-Aktionen erzeugen jeweils genau ein Event mit `actor` des Auslösers.
- **Abhängigkeiten:** PROTO-001, PROTO-002, DATA-002, DATA-008 (siehe 06-data-sync-protocol.md), RUN-003 (siehe 10-runners-extensibility.md)
- **Referenz:** Omnigent 3.6 (Archivieren/Löschen), ADR-0014

### SES-002 — Multi-Client-Live-Stream mit Resume ab seq
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Beliebig viele Clients hängen sich gleichzeitig an dieselbe Session und sehen denselben Event-Strom in Echtzeit. Reconnect setzt ab der zuletzt gesehenen `seq` fort, ohne Lücken oder Duplikate. Das gilt auch lokal (Desktop + Browser + TUI gegen localhost).
- **Details:** `attach {session_id, from_seq}` über den WS-Kanal (PROTO). Ephemere Daten (Terminal-Bytes, Browser-Frames) laufen über Binärkanäle und werden nicht replayed.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given drei Clients an einer Session, when der Harness 1 000 `message.delta` emittiert, then erhalten alle drei dieselbe Sequenz in identischer Reihenfolge.
  - [ ] AC2 — When ein Client bei seq 500 trennt und mit `from_seq=500` reconnectet, then erhält er exakt die Events 501…n, keine Duplikate.
  - [ ] AC3 — `beton attach <id>` aus einem zweiten Terminal zeigt den laufenden Turn live, während die Web-UI weiterläuft.
- **Abhängigkeiten:** PROTO-005, PROTO-009 (siehe 06-data-sync-protocol.md), SYNC-002 (ab M4)

### SES-003 — Resume gestoppter Sessions
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** `resume` startet einen neuen Runner für eine gestoppte Session und stellt den Harness-Kontext wieder her: nativ, wenn der Harness `capabilities.resume` meldet (z.B. Vendor-Session-ID), sonst per Handover-Kontext aus dem Event-Log (SES-007). Nach Daemon-Neustart sind alle Sessions resumebar.
- **Details:** Vendor-Session-IDs werden beim Start als Event-Feld `harness_session_ref` gespeichert. Ein Input an eine `stopped`-Session löst implizit `resume` aus. Mechanik je Harness (Warm-Resume, Rebuild, Präambel): HAR-004, HAR-018, HAR-019, HAR-020 (siehe 01-harnesses.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given eine Claude-Code-Session, when der Daemon neu startet und `beton resume <id>` läuft, then nutzt der Adapter die gespeicherte Vendor-Session-ID und das Modell kennt die vorherige Unterhaltung (Golden-Test mit Fake-Harness).
  - [ ] AC2 — (ab M1) Given ein Harness ohne Resume-Capability, when resumed wird, then erhält er einen Handover-Kontext und ein Event `session.resumed {mode: "handover"}`.
  - [ ] AC3 — Senden an eine `stopped`-Session startet den Runner automatisch und stellt den Input nach dem Resume zu.
- **Abhängigkeiten:** SES-001, HAR-004 (siehe 01-harnesses.md), HAR-018 (ab M1)

### SES-004 — Queue & Steer
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Inputs während eines laufenden Turns landen in einer **serverseitigen** Queue (sichtbar und editierbar für alle Clients mit `drive`), die bei `idle` automatisch abgearbeitet wird. „Steer“ speist einen Input in den laufenden Turn ein, sofern der Harness die Capability `steering: true` meldet; sonst bietet der Client „Unterbrechen und senden“ an.
- **Details:** Queue-Items: `{id, author, text, attachments, created_at}`; Operationen add / edit / delete / reorder / promote-to-steer. Events `queue.updated` (PROTO-002) mit vollständigem Queue-Snapshot. UI: WEB-005. Queue-Items durchlaufen beim Dequeue (nicht beim Einreihen) den Policy-Hook „vor Model-Request“ (mit der Policy-Engine, M2).
  API: `POST /v1/sessions/{id}/input {text, mode?: queue|steer}` → `202 {input_id, turn_id?, status: started|queued|steered}`; ohne laufenden Turn und mit leerer Queue startet die Eingabe sofort, sonst wird sie eingereiht. `mode: steer` während eines Turns speist ein (ohne Capability `steering`: `409 capability_unsupported`); ohne laufenden Turn verhält es sich wie `queue`. Queue: `GET …/queue`, `PATCH|DELETE …/queue/{item_id}` (`queue.edit|delete`), `POST …/queue/{item_id}/move {position}` (`queue.reorder`, 0 = als Nächstes), `POST …/queue/{item_id}/steer` (`queue.steer`), `POST …/queue/resume` (`queue.resume`); Änderungen antworten `204`, der neue Stand kommt als `queue.updated`. Der Stand überlebt einen Daemon-Neustart (letztes `queue.updated`). Neuer Input hebt eine Pause auf; ein Fehler beim Abarbeiten erscheint als `error`-Event im Verlauf.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given ein laufender Turn, when zwei Inputs gesendet werden, then erscheinen sie in Reihenfolge in der Queue aller Clients und werden nach `turn.completed` nacheinander als eigene Turns ausgeführt.
  - [ ] AC2 — Reorder und Delete eines Queue-Items durch Client A sind innerhalb von 500 ms bei Client B sichtbar.
  - [ ] AC3 — Given Codex (Steering nativ), when Steer gesendet wird, then wird der Input im laufenden Turn verarbeitet, ohne `turn.interrupted`.
  - [ ] AC4 — Given ein Harness ohne Steering, when Steer gewählt wird, then lehnt die API mit `409 capability_unsupported` ab (HAR-002).
- **Abhängigkeiten:** SES-002, HAR-001, HAR-002 (siehe 01-harnesses.md)
- **Referenz:** Omnigent Queue & Steer (dort clientseitig – beton bewusst serverseitig für Multi-Client)

### SES-005 — Interrupt
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Ein Interrupt bricht den laufenden Turn ab (Harness-nativ: stream-json-Interrupt bzw. Codex `turn/interrupt`; PTY-Modus: `Ctrl+C` an das PTY). Die Queue bleibt erhalten, wird aber nicht automatisch weiter abgearbeitet, bis ein Client „fortsetzen“ wählt oder neuen Input sendet.
- **Akzeptanzkriterien:**
  - [ ] AC1 — When während eines Tool-Calls interrupted wird, then folgt innerhalb von 2 s `turn.interrupted`, laufende Tool-Prozesse sind beendet, Status `idle`.
  - [ ] AC2 — (ab M1) Nach einem Interrupt bleibt eine nicht-leere Queue pausiert (`queue.paused=true`), bis sie explizit fortgesetzt wird.
  - [ ] AC3 — Interrupt ist idempotent: ein zweiter Interrupt ohne laufenden Turn liefert `200` ohne neues Event.
- **Abhängigkeiten:** SES-001, HAR-004 (siehe 01-harnesses.md), SES-004 (ab M1)

### SES-006 — Fork ab Event X
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Aus jeder Session lässt sich ab einem beliebigen Event (`at_seq`) eine neue Session abzweigen; die Historie bis einschließlich X wird übernommen, die Quell-Session bleibt unverändert. Der Dateisystemzustand wird **nicht** auf X zurückgesetzt; der Fork wählt den Workspace-Modus explizit.
- **Details:** `POST /v1/sessions/{id}/fork {at_seq?, harness?, model?, effort?, permission_mode?, workspace?: "shared" | "new_worktree" | "fresh", title?, harness_opts?}` → `201 {session, effective_seq, workspace}`; ohne `at_seq` ab dem Ende der Quelle, `at_seq` außerhalb der Session → `400 validation_failed`. `effort` und `permission_mode` prüft der Server vor dem Anlegen gegen den Ziel-Harness (`capability_unsupported`, `yolo` ohne Sandbox `sandbox_required`, HAR-017, HAR-027); ohne Angabe gelten die Werte aus dem `executor` des Agents der Quelle (AGT-004), sonst die Defaults des Harness, nicht die zuletzt gewechselten Werte der Quelle. Default `new_worktree` (SES-015; vom aktuellen HEAD der Quelle inkl. Commit eines WIP-Snapshots, falls Repo vorhanden: eigener Commit mit uncommitteten und neuen Dateien, ohne Index oder Branch der Quelle zu ändern; Base des Worktrees ist dieser Commit), sonst `shared` (Workspace der Quelle, also ihr Worktree bzw. Arbeitsverzeichnis). `fresh` legt ein leeres Verzeichnis `~/.beton/workspaces/<session>` an, das mit der Session gelöscht wird. Titel ohne Angabe: „<Quelle> (Fork)“ bzw. „<Quelle> (<Harness>)“. Übernommen werden die Inhalts-Events (`message.completed`, `reasoning.completed`, `tool.call.requested|started|completed`, `turn.*`, `fs.changed`) mit neuer ID und `seq`, sonst unverändert (inkl. `raw`); Lebenszyklus-, Runner-, Queue-, Freigabe- und Kosten-Events bleiben in der Quelle. Reihenfolge im Fork: `session.created`, `session.title_changed`, ggf. `git.worktree_created`, die übernommenen Events, dann `session.forked` (vom Runner, siehe HAR-018). Events: `session.forked {from_session, at_seq}` in der neuen, `session.fork_created {child}` in der Quell-Session.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given Session S mit 200 Events, when Fork bei seq 120, then enthält die neue Session die Inhalts-Events 1…120 von S (neu nummeriert) und ein `session.forked`-Event; S hat 201 Events.
  - [ ] AC2 — `at_seq` mitten in einem Turn wird auf das letzte vollständige Turn-Ende davor normalisiert; die Antwort meldet das tatsächliche `effective_seq`.
  - [ ] AC3 — With `workspace=new_worktree`, then existiert ein neuer Branch, und Änderungen im Fork sind im Worktree der Quelle nicht sichtbar.
  - [ ] AC4 — `beton run --fork <id>@<seq>` erzeugt denselben Fork wie die API.
- **Abhängigkeiten:** SES-001, SES-015, HAR-018 (siehe 01-harnesses.md), DATA-002 (siehe 06-data-sync-protocol.md)
- **Referenz:** Omnigent Fork `up_to_response_id`

### SES-007 — Fork auf anderen Harness mit Handover-Kontext
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Ein Fork kann auf einem anderen Harness weiterlaufen (Claude Code → Codex und umgekehrt). Dieses Feature regelt die Session-Seite: Auswahl des Ziel-Harness, Übergabe von `at_seq`, Worktree und Plan an den Adapter, Darstellung und Herkunftsverweis. Erzeugung, Budget und Kürzungsregeln des Handover-Kontexts sowie die Wahl `rebuild` vs. `preamble` anhand der Capability `fork_history` sind HAR-018/HAR-019 (siehe 01-harnesses.md).
- **Details:** Ein Harness-Wechsel innerhalb einer laufenden Session gibt es nicht: Eine Session bleibt auf ihrem Harness. Die UI bietet „Weiter mit <Harness>“ an (Composer-Picker, Command-Palette); das löst einen Fork ab dem letzten `seq` aus und öffnet die neue Session direkt. Der Fork-Request akzeptiert `harness`, `model`, `effort` und `permission_mode` (siehe SES-006); inkompatible Kombinationen werden vor dem Anlegen mit `422 harness_incompatible` abgelehnt: Ziel-Harness auf dem Host nicht verfügbar, `fork_history: none`, oder der Agent der Session braucht MCP-Server, System-Tools, Skills oder Sub-Agents und der Ziel-Harness hat kein `mcp_injection`. Das Event `session.forked` trägt `from_harness`, `harness` (Ziel), `history_mode: native|rebuild|preamble` (PROTO-002). `native`/`rebuild` gibt es nur bei gleichem Harness mit `fork_history: rebuild`; ein Harness mit `fork_history: preamble` bekommt auch beim Fork auf sich selbst die Präambel (AC2 gilt für Harnesses mit `rebuild`). „Weiter mit …“ listet die installierten, kompatiblen Harnesses des Katalogs außer dem eigenen, jeweils mit dem Hinweis „Übergabe“.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given eine Claude-Session, when mit `harness=codex` geforkt wird, then startet ein Codex-Runner, `session.forked.history_mode = preamble`, und der erste Turn erhält den Handover-Kontext (Fake-Harness-Test).
  - [ ] AC2 — Ein Fork auf denselben Harness nutzt `history_mode = native` bzw. `rebuild`, keinen Handover-Text.
  - [ ] AC3 — (ab M2) Der Handover enthält keine Secret-Werte, nur `bt_cred_*`-Platzhalter (Test mit präpariertem Log).
  - [ ] AC4 — UI zeigt in der neuen Session ein Banner „Fortgesetzt aus <Session> auf <Harness>“ mit Link.
  - [ ] AC5 — Wählt der User im Harness-Picker einer laufenden Session einen anderen Harness („Weiter mit Codex“), entsteht ein Fork ab dem letzten `seq`; die Ursprungs-Session behält ihren Harness unverändert.
- **Abhängigkeiten:** SES-006, HAR-018, HAR-019 (siehe 01-harnesses.md), SEC-013 (ab M2, siehe 05-security-identity.md)

### SES-008 — Import fremder Chats (Session-Seite)
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Bestehende Claude-Code- und Codex-Chats werden als beton-Sessions übernommen. Parsing der Vendor-Formate ist HAR-023/HAR-024 (siehe 01-harnesses.md); dieses Feature regelt Discovery, Deduplizierung, Metadaten und Resume-Fähigkeit der importierten Session.
- **Details:** `GET /v1/imports/candidates?harness=claude|codex` (läuft auf dem Host, auf dem die Dateien liegen; cursor-paginiert `{items: [{vendor_session_id, path, cwd?, title?, model?, updated_at?, size_bytes, imported_session_id?}], next_cursor}`, zuletzt geänderte zuerst); `POST /v1/imports {harness, refs[] | last_n, force}` → `{results: [{vendor_session_id, status: imported|skipped|failed, reason?, detail?, session_id?, events?, warnings[]}]}`. `refs` sind Vendor-Session-IDs, nie Pfade: welche Datei dazu gehört, bestimmt allein die Discovery des Adapters (HAR-023/HAR-024). Gelesen wird nur auf diese ausdrückliche Anfrage, nie im Hintergrund. Dedup-Schlüssel `(host_id, harness, vendor_session_id)` (Tabelle `session_imports`; `force` legt eine neue Session an und lässt den Schlüssel auf sie zeigen; Löschen der Session gibt ihn frei). Erstes Event nach `session.created` ist `session.imported {source, vendor_session_id, imported_at}`, danach Titel (`source: harness`), bei Parser-Warnungen ein `notice` (nur Zeilennummern und Zahlen, keine Inhalte), der Verlauf und `session.status: stopped`; Ursprungs-Zeitstempel bleiben in `ts`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein zweiter Import derselben Vendor-Session ohne `force` liefert `skipped: already_imported` mit der bestehenden Session-ID.
  - [ ] AC2 — Eine importierte Claude-Session ist per `resume` nativ fortsetzbar, solange die Vendor-Datei existiert (`--resume <id> --fork-session`, die Datei bleibt unverändert); sonst per Rebuild (HAR-019) und, wenn der scheitert, per Handover.
  - [ ] AC3 — (ab M3) Importierte Sessions werden dem Projekt zugeordnet, dessen Root ihr `cwd` enthält (falls vorhanden).
- **Abhängigkeiten:** HAR-023, HAR-024 (siehe 01-harnesses.md), SES-003, SES-013 (ab M3)

### SES-009 — Session-Export/-Import (JSONL)
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Sessions lassen sich verlustfrei als JSONL exportieren und auf einer anderen beton-Instanz importieren (Backup, Weitergabe, Bug-Reports).
- **Details:** Format und Import-/Export-Implementierung: Owner DATA-010 (Zeile 1 `beton.export`-Header, danach je Zeile ein Event im PROTO-001-Envelope). `--with-blobs` erzeugt ein `.tar.zst` mit `session.jsonl` + `blobs/<sha256>`. `raw`-Payloads (redigiert) sind standardmäßig **nicht** enthalten; `--with-raw` nimmt sie auf. CLI-Front: CLI-007. API: `GET /v1/sessions/{id}/export?with_raw=&with_blobs=` (Leserecht wie beim Attach; `application/x-ndjson` bzw. `application/zstd`, Header `beton-export-events`/`beton-export-blobs`) und `POST /v1/sessions/import?force=` mit der Datei als Body → `{status: imported|skipped, reason?, session_id, title, events, blobs, imported_from}`. Server und CLI lesen und schreiben nur die Datei, die der User nennt (ADR-0033); die CLI legt Exportdateien mit `0600` an. Eine importierte Session ist auf dem Zielhost nur lesbar (Verlauf, Suche, Export, Umbenennen, Archivieren, Löschen): Resume, Eingaben, Fork und Workspace-API lehnt der Server mit `409 imported_read_only` ab, weil Arbeitsverzeichnis und Startoptionen aus der Datei stammen. Fortsetzen in einem selbst gewählten Verzeichnis folgt mit #122.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Export → Import → Export ergibt (bis auf die in DATA-010 AC1 genannten Felder) identische Event-Zeilen.
  - [ ] AC2 — (ab M2) Der Export enthält keine Secret-Werte (Scan auf bekannte Test-Secrets schlägt nicht an).
  - [ ] AC3 — Ein Export mit `format_version` > unterstützt wird mit klarer Fehlermeldung abgelehnt.
- **Abhängigkeiten:** DATA-010 (siehe 06-data-sync-protocol.md), CLI-007 (siehe 08-clients.md)

### SES-010 — Automatische Session-Titel
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Nach dem ersten abgeschlossenen Turn erzeugt beton einen kurzen Titel (≤ 60 Zeichen). Ein vom User gesetzter Titel wird nie überschrieben; native Umbenennungen (z.B. Claude `/rename`) werden übernommen.
- **Details:** `titles.generator: auto | direct | harness | off`. `auto` (Default): Einmal-Aufruf über den Harness der Session – bei Vendor-CLIs deren nicht-interaktiver Modus (z. B. `claude -p`, `codex exec`, ACP: kurzlebige Session) mit deren eigener Anmeldung (Subscription, HAR-015) und kleinstem Modell –, bei Fehlschlag Heuristik (erste Zeile der ersten User-Nachricht, gekürzt). `harness`: wie `auto`, aber ohne Heuristik-Fallback. `direct`: Direkt-API-Harness mit konfiguriertem Kleinmodell (`titles.provider`), nur wenn ausdrücklich gewählt; ein API-Key ist nie Voraussetzung (ADR-0034). `off`: nur Heuristik. Optional `titles.instructions` (≤ 2 000 Zeichen). Event `session.title_changed {title, source: generated|user|harness}`. SES-010 ist Owner der Generierung; UI-Darstellung und Inline-Umbenennen: UX-009.
  Umsetzung (M1): Der Server erzeugt den Titel nach dem ersten `turn.completed` einer Session ohne Titel, einmal je Session und Daemon-Lauf; Sessions mit Titel (vom User, aus Fork, Import oder Sub-Agent) bleiben unverändert. Den Einmal-Aufruf führt der Runner der Session aus (Tunnel-Kommando `harness.one_shot`, Adapter-Methode `one_shot`), damit Vendor-CLIs wie bei Turns in dessen Umgebung laufen: `claude -p --output-format json --no-session-persistence --tools "" --strict-mcp-config --permission-mode dontAsk --model haiku --system-prompt <Anweisung>`; der Permission-Mode der Session gilt hier nicht: Claude läuft ausdrücklich in `dontAsk` (sonst gälte `permissions.defaultMode` aus `.claude/settings.json`, HAR-027), ohne eingebaute Tools und ohne MCP-Server aus Nutzer- oder Projekt-Konfiguration. **Codex hat keinen Einmal-Modus (fail closed, #146):** `codex exec` kennt keinen Schalter ohne Tools. Gegen codex-cli 0.153.2 ohne Modellaufruf geprüft (`codex exec --help`, Anfragen an einen lokalen Mock statt der Modell-API): `--ignore-user-config` lässt MCP-Server aus Nutzer- und Projekt-Konfiguration weg (`-c mcp_servers={}` nicht, Overrides werden zusammengeführt), `--disable unified_exec|shell_tool|multi_agent|view_image` und `-c web_search="disabled"` entfernen Shell, Bilder, Sub-Agents und Websuche, `apply_patch` (z. B. bei `gpt-5.5`, `gpt-5.6-luna`) und `request_user_input` bleiben aber, und neue CLI-Versionen können weitere Tools mitbringen. Der Adapter startet für Einmal-Aufrufe daher keine CLI und meldet `capability_unsupported`; `auto` nimmt die Heuristik, `harness` setzt keinen Titel. Auf den Claude-Einmal-Aufruf weicht beton bewusst nicht aus: Der Inhalt einer Codex-Session ginge sonst an einen anderen Anbieter als den, den der Nutzer gewählt hat. Der Inhalt (erste User-Nachricht, Anfang der ersten Antwort) geht über stdin. Die Antwort wird auf eine Zeile ohne Anführungszeichen und auf 60 Zeichen gekürzt. Die Projektion merkt sich die Herkunft (`title_source`); ein generierter Titel ersetzt nie einen Titel des Users. `titles.*` gilt nur aus der User-Konfiguration. Noch offen (eigene Issues): `direct` nutzt bis zur Anbindung des Direkt-API-Harness die Heuristik; ACP-Harnesses haben noch keinen Einmal-Modus (`auto` fällt auf die Heuristik zurück); native Umbenennungen (`source: harness`) werden noch nicht übernommen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given `titles.generator=off`, then entsteht der Titel per Heuristik ohne Modellaufruf.
  - [ ] AC2 — When der User umbenennt, then erfolgt keine weitere automatische Generierung für diese Session.
  - [ ] AC3 — Titelgenerierung verursacht ein `cost.delta` mit `purpose: "title"` bzw. zählt zur Subscription-Usage.
  - [ ] AC4 — Mit Default `auto`, ohne `*_API_KEY` in der Umgebung und ohne `providers`-Konfiguration entsteht der Titel einer Claude-Session über einen Einmal-Aufruf der `claude`-CLI (Fake-CLI-Test prüft Aufruf und bereinigte Umgebung); der Direkt-API-Harness wird nur mit `titles.generator: direct` genutzt.
- **Abhängigkeiten:** HAR-004, HAR-010, HAR-015 (siehe 01-harnesses.md), USE-001 (ab M2), UX-009 (siehe 11-platform-features.md)

### SES-011 — Compaction & Kontextanzeige (Session-Anbindung)
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Die Session führt pro Turn den Kontextfüllstand (genutzte vs. verfügbare Tokens) als Event, damit Clients eine Kontextanzeige rendern können. Compaction wird an den Harness durchgereicht (`/compact`), nur der Direkt-API-Harness kompaktiert selbst. Darstellung der Anzeige: USE-008, manueller/automatischer Trigger: USE-009 (siehe 11-platform-features.md); Adapter-Mechanik: HAR-021, HAR-022.
- **Details:** Event `context.usage {used_tokens, window_tokens, source: harness|estimated}`; `compaction.started|completed {before_tokens, after_tokens}`. `POST /v1/sessions/{id}/compact` (Kommando `session.compact`, PROTO-006) antwortet `202`; die Events folgen asynchron. Auto-Compaction für Direkt-API bei `compaction.threshold` (Default 0.8). *Präzisiert:* Meldet ein Harness den Kontext mehrfach je Turn (Codex je Usage-Update), bündelt der Runner auf genau ein `context.usage` je Turn, den letzten Stand direkt vor dem Turn-Ende; `context.usage` außerhalb eines Turns (nach einer Compaction) geht unverändert durch. `POST …/compact` antwortet `409 capability_unsupported` ohne Capability `compaction`, `409 conflict`, wenn die Session nicht läuft oder ein Turn läuft (Compaction erst danach). Die Konfiguration `compaction.threshold` gehört zu USE-009 (M2); bis dahin gilt für den Direkt-API-Harness fest 0,8.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach jedem Turn eines Harness mit Usage-Reporting existiert genau ein `context.usage`-Event.
  - [ ] AC2 — `POST …/compact` auf einer Claude-Session sendet `/compact` an den Harness und erzeugt `compaction.started` und `compaction.completed`.
  - [ ] AC3 — Harness ohne Compaction-Fähigkeit: API liefert `409 capability_unsupported` (HAR-002, HAR-022).
- **Abhängigkeiten:** HAR-021, HAR-022 (siehe 01-harnesses.md)

### SES-012 — Session-Liste, Suche & Unread-State
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Sessions sind filterbar (eigene / mit mir geteilt / archiviert / Projekt / Harness / Status), per Volltext über Titel und Nachrichten durchsuchbar und pinbar. Unread-State ist pro User geräteübergreifend: eine Session gilt als gelesen, sobald ein Client des Users sie bis zur aktuellen `seq` angezeigt hat.
- **Details:** `GET /v1/sessions?filter=…&q=…&cursor=…`; `PUT /v1/sessions/{id}/read-state {seq}`. Volltext: SQLite FTS5 bzw. Postgres `tsvector`.
  Präzisiert: `filter=own|shared|archived|all` (ohne Angabe alle nicht archivierten; `shared` ist bis COL-001 leer), dazu `harness=`, `status=`, `project_id=`. Suchsemantik in beiden Dialekten gleich: Dokumente sind Titel und Nachrichten (`message.completed`); jedes Wort der Anfrage als Wortanfang, alle Wörter im selben Dokument, ohne Groß-/Kleinschreibung, ohne Stammformen (SQLite `unicode61 remove_diacritics 0`, Postgres `to_tsvector('simple', …)` mit `wort:*`). Anpinnen je User: `PUT /v1/sessions/{id}/pin {pinned}`; angepinnte Sessions stehen auf der ersten Seite zusätzlich zur Seitengröße oben. Gelesen-Stand je User, steigt nur (höchstens bis `head_seq`); `SessionSummary` trägt `pinned`, `read_seq`, `unread`, `changed_at`. Andere Geräte erfahren Änderungen über `updated_after` (bezogen auf `changed_at`: Aktivität, Gelesen-Stand oder Pin).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Suche nach einem Wort aus einer Agent-Antwort findet die Session in < 300 ms bei 10 000 Sessions (lokal, SQLite).
  - [ ] AC2 — Liest der User eine Session auf Gerät A, ist sie auf Gerät B innerhalb von 2 s als gelesen markiert.
  - [ ] AC3 — Gepinnte Sessions erscheinen unabhängig von der Sortierung oben.
- **Abhängigkeiten:** DATA-005 (siehe 06-data-sync-protocol.md)

### SES-013 — Projects: Gruppierung & Defaults
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Ein Project gruppiert Sessions (flach, nicht verschachtelt) und liefert Defaults für neue Sessions: Repo-Root je Host, Harness/Agent, Modell, Effort, Permission-Mode, Worktree-Default und Base-Branch. Ein Project kann an ein Repo gebunden sein, dessen `.beton/config.yaml` als Projekt-Config mitgelesen wird.
- **Details:**
  ```yaml
  # Project-Entität (UI/API), Repo-Datei .beton/config.yaml wird gemergt (Repo < Project-UI-Overrides)
  name: beton
  roots: { host_local: /Users/ingo/Develop/ai/beton }
  defaults:
    harness: claude
    model: claude-opus-4
    effort: high
    permission_mode: default
    worktree: always          # always | ask | never
    base_branch: main
  ```
  Projekt löschen archiviert seine Sessions (kein Datenverlust). Manuelle Sortierung der Projekte pro User.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given Project mit `defaults.harness=codex`, when im Project eine Session ohne Angaben erstellt wird, then läuft sie auf Codex.
  - [ ] AC2 — `beton run` in einem Verzeichnis unterhalb eines Project-Roots ordnet die Session automatisch dem Project zu.
  - [ ] AC3 — Löschen eines Projects archiviert alle zugehörigen Sessions und löscht keine.
- **Abhängigkeiten:** SES-001, SES-015

### SES-014 — Projects als Policy-Ebene
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Ein Project trägt die Projekt-Ebene der Policy-Hierarchie (ADR-0008). Sie setzt sich zusammen aus `.beton/policies/*.yaml` im gebundenen Repo und im Project gespeicherten Policies; innerhalb der Ebene und über Ebenen hinweg gilt „strengere Regel gewinnt“. Auswertung selbst: POL-001, POL-007 (siehe 03-policies.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given eine Projekt-Policy `deny` für `git push` und eine User-Policy `allow`, then wird `git push` in Project-Sessions abgelehnt (`policy.decision` nennt die Projekt-Regel).
  - [ ] AC2 — Sessions außerhalb des Projects sind von dessen Policies nicht betroffen.
  - [ ] AC3 — Änderungen an Projekt-Policies wirken ab dem nächsten Hook-Aufruf laufender Sessions, ohne Neustart.
- **Abhängigkeiten:** SES-013, POL-001, POL-007 (siehe 03-policies.md)

### SES-015 — Worktree pro Session (Erstellung & Base-Branch)
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Auf Wunsch (oder per Project-Default) erhält jede Session ein eigenes `git worktree` mit eigenem Branch, damit parallele Sessions im selben Repo isoliert arbeiten. Die Sandbox der Session bekommt den Worktree als Schreib-Root (ab M2). Engine und API (`--worktree`, Fork mit `new_worktree`, Sub-Agents mit `worktree: new`) gibt es ab M1; die UI dazu (Branch-Auswahl im Composer, Project-Defaults, Worktree-Anzeige) folgt mit M3 (SES-013, WEB-004).
- **Details:** Ablage `~/.beton/worktrees/<repo-slug>-<hash8>/<branch-slug>/` *(Annahme: außerhalb des Repos, um es nicht zu verschmutzen)*. Branch-Name `beton/<titel-slug>-<id4>` (umbenennbar, solange nicht gepusht); ein explizit genannter, vorhandener Branch wird ausgecheckt. Base: explizit → Project-Default (ab M3) → `origin/HEAD` → aktueller Branch; der neue Branch startet bei der Commit-ID der Base ohne Upstream. Vor der Erstellung `git fetch <remote> <branch>`, wenn die Base ein Remote-Tracking-Branch (`<remote>/<branch>`) ist (abschaltbar, Timeout 15 s); ist das Remote nicht erreichbar (offline), wird vom lokalen Stand der Base erstellt und ein `notice` (`warn`) nennt den nicht ausgeführten Fetch, ohne Remote-URL – der Fetch ist nie Voraussetzung (ADR-0033). Sandbox-Grants: Schreibrecht auf Worktree **und** `<repo>/.git/worktrees/<name>` sowie Objekt-DB (SBX-003, siehe 04-sandbox.md).
  API (M1): `POST /v1/sessions {…, worktree: {branch?, base?, fetch?}}`; der Worktree entsteht vor der Session, scheitert er, gibt es keine Session (`422 base_not_found` mit den verfügbaren Branches, `409 not_a_git_repo`). `session.created.cwd` bleibt das angefragte Verzeichnis (für `beton run -c`); Workspace und Runner-`cwd` ist der Worktree (`git.worktree_created.path`, Projektion `worktree` der Session-Liste). CLI: `beton run --worktree[=BRANCH] [--base BRANCH]` (CLI-002).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given zwei Sessions mit `worktree=always` im selben Repo, then arbeiten sie in verschiedenen Verzeichnissen und Branches; `git worktree list` zeigt beide.
  - [ ] AC2 — (ab M2) Ein Tool-Call, der außerhalb des Worktrees in das Haupt-Checkout schreibt, scheitert an der Sandbox.
  - [ ] AC3 — Ist die Base nicht auflösbar, schlägt die Erstellung mit einer Fehlermeldung fehl, die verfügbare Branches nennt; die Session startet nicht stillschweigend ohne Worktree.
  - [ ] AC4 — Event `git.worktree_created {path, branch, base, base_sha}` (PROTO-002) wird geloggt.
  - [ ] AC5 — Ohne Netzwerk (Remote nicht erreichbar) wird der Worktree vom lokalen Stand der Base erstellt; die Session startet, und ein Hinweis nennt den nicht ausgeführten Fetch.
- **Abhängigkeiten:** SES-001, SBX-003 (ab M2, siehe 04-sandbox.md)
- **Referenz:** Omnigent Worktrees im Composer

### SES-016 — Worktree-Aufräumen
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Worktrees werden kontrolliert entfernt, ohne Arbeit zu verlieren. Archivieren behält den Worktree; Löschen entfernt ihn; der Branch wird nur gelöscht, wenn er gemergt ist bzw. keine eigenen Commits hat, sonst nach Rückfrage.
- **Details:** Prüfreihenfolge beim Löschen: (1) uncommitted Changes → Rückfrage (Optionen: WIP-Commit, verwerfen, abbrechen); (2) Branch mit eigenen, nicht gemergten Commits, die auf keinem Remote liegen → Rückfrage (behalten, löschen, abbrechen); (3) `git worktree remove`; (4) Branch löschen, wenn er keine eigenen Commits gegenüber der Base hat (sie also enthält), sonst nur nach Bestätigung. Alle Rückfragen werden geprüft, bevor etwas verändert wird. Weil `git branch -d` gegen den HEAD des Haupt-Checkouts prüft (und bei einem anderen ausgecheckten Branch fälschlich scheitert), prüft beton selbst gegen die Base und löscht erst danach mit `git branch -D`; ohne diese Prüfung nie `-D` ohne Bestätigung. API: `DELETE /v1/sessions/{id}?uncommitted=commit|discard&branch=keep|delete`; ohne nötige Entscheidung `409 worktree_dirty` bzw. `409 worktree_unpushed`, die Detailmeldung nennt die Optionen. CLI: `beton session delete <id> [--uncommitted commit|discard] [--branch keep|delete]`. Hintergrund-Job: `git worktree prune` beim Start des Daemons und danach täglich, Meldung verwaister Worktree-Verzeichnisse ohne Session im Log; `beton doctor` listet sie (Check `worktrees`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Löschen einer Session mit uncommitted Changes ohne Bestätigung entfernt nichts und liefert `409 worktree_dirty`.
  - [ ] AC2 — Nach Löschen einer Session mit gemergtem Branch existieren weder Verzeichnis noch Branch.
  - [ ] AC3 — `beton doctor` listet verwaiste Worktrees mit Pfad und Größe.
- **Abhängigkeiten:** SES-015

### SES-017 — Workspace-API: Files & Suche
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Server und Runner stellen den Workspace der Session für Clients bereit: Dateibaum, Datei lesen/schreiben, Dateinamen- und Inhaltssuche. Zugriffe sind auf Workspace-Roots begrenzt; User-Schreibzugriffe erzeugen `fs.changed` mit `actor=user`.
- **Details:** `GET …/workspace/tree?path=` (eine Ebene, nach Namen sortiert, ohne `.git`), `GET|PUT …/workspace/files/{path}` (PUT mit `If-Match: <sha256>` gegen Überschreiben; Body `{content}`), `GET …/workspace/search?q=&mode=name|content` (ripgrep-Semantik: `.gitignore` respektiert, versteckte und binäre Dateien übersprungen, Symlinks nicht verfolgt; Suchbegriff wörtlich mit Smart-Case). Listen sind cursor-paginiert (PROTO-010). Max. Dateigröße für Inline-Lesen 5 MiB, darüber Download (`?download=true`). `GET …/files/{path}` liefert `{path, size, sha256, binary, too_large, content?}` und `ETag: "<sha256>"`; `If-Match` akzeptiert das ETag mit oder ohne Anführungszeichen.
  Pfadbegrenzung (fail closed): absolute Pfade, `..`, `.git` (jede Schreibweise) und Pfade, die nach Auflösung aller Symlinks außerhalb des Workspace liegen – auch nicht existierende Ziele hinter einem Symlink –, liefern `403 path_outside_workspace`. `fs.changed` trägt `source: api` bei Schreibzugriffen über die API (`actor=user`) und `source: watcher` bei Änderungen, die der Runner während eines Turns feststellt (`actor=agent`, mit `turn_id`); der Runner vergleicht den Workspace dafür laufend mit dem letzten Snapshot (SES-018).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Pfade mit `..` oder Symlinks aus dem Workspace hinaus liefern `403`.
  - [ ] AC2 — PUT mit veraltetem `If-Match` liefert `412`, die Datei bleibt unverändert.
  - [ ] AC3 — Eine Agent-Schreiboperation erscheint ≤ 1 s später als `fs.changed` in allen Clients.
- **Abhängigkeiten:** PROTO-010 (siehe 06-data-sync-protocol.md), RUN-002 (siehe 10-runners-extensibility.md)

### SES-018 — Workspace-API: Changes & Diffs
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Die Session liefert eine Änderungsübersicht in drei Sichten: (a) uncommitted gegenüber HEAD, (b) Branch gegenüber Base (Worktree), (c) pro Turn (aus `fs.changed`-Events). Diffs sind zeilengenau adressierbar, damit Inline-Kommentare (COL-006) und „an Agent anhängen“ darauf verweisen können.
- **Details:** `GET …/workspace/changes?scope=uncommitted|branch|turn&turn=` → Liste `{path, old_path?, status, additions, deletions}` (cursor-paginiert) mit `base_sha`, `head_sha?`, `turn_id?`; `GET …/workspace/diff?path=&scope=&turn=` → Unified Diff (`patch`) + `base_sha`/`head_sha` + Hunks, deren Zeilen `old_line`/`new_line` tragen. `uncommitted` vergleicht das Arbeitsverzeichnis (inklusive unversionierter Dateien) mit `HEAD`; `head_sha` fehlt dort, weil das Arbeitsverzeichnis keine ID hat. `branch` entspricht `git diff <merge-base>...HEAD` gegen die Base des Worktrees (ohne Worktree `origin/HEAD` bzw. der aktuelle Branch). `turn` ohne `turn=` meint den letzten Turn. Turn-Sicht: Der Runner hält vor und nach jedem Turn einen Snapshot des Workspace als Baum in einem Schatten-Repository außerhalb des Workspace fest (`<data_dir>/snapshots/<session>.git`, Refs `refs/beton/turns/<turn>/{before,after}`; respektiert `.gitignore`, auch ohne Git im Workspace – das sind die Blob-Snapshots vor erster Änderung); gelistet werden die Dateien aus den `fs.changed`-Events des Turns mit Zeilenzahlen aus diesen Snapshots. Das Repository des Nutzers bleibt unberührt. Ohne Git-Repository liefern `uncommitted` und `branch` `409 not_a_git_repo`; `GET …/workspace` → `{git_repo}` sagt Clients vorab, welche Sichten es gibt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given ein Turn änderte 3 Dateien, then listet `scope=turn` genau diese 3 mit korrekten Zeilenzahlen.
  - [ ] AC2 — `scope=branch` entspricht `git diff <merge-base>...HEAD` (Vergleichstest).
  - [ ] AC3 — In einem Nicht-Git-Verzeichnis liefert `scope=turn` Diffs; `scope=branch` liefert `409 not_a_git_repo`.
- **Abhängigkeiten:** SES-017

### SES-019 — Workspace-Terminals
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** User können pro Session interaktive Shells öffnen (mehrere, benannt), die im Runner als PTY laufen und von allen berechtigten Clients gesehen werden; ebenso werden vom Agent gestartete Langläufer (Dev-Server) als Terminal angezeigt. Terminals überleben Client-Disconnects.
- **Details:** PTY-Multiplexing über `beton-pty` (TUI-002, siehe 08-clients.md). User-Terminals laufen per Default mit dem Sandbox-Profil der Tool-Ausführung (SBX-017); `terminals.user_sandbox: none` ist nur lokal und nur für den Owner erlaubt *(Annahme)*. Scrollback serverseitig 10 000 Zeilen; Snapshot beim Attach. Bytes über Binärkanal (`terminal.output` ephemer).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein in der Web-UI gestartetes `npm run dev` läuft weiter, wenn alle Clients trennen, und ist beim Re-Attach mit Scrollback sichtbar.
  - [ ] AC2 — Ein User mit Rolle `comment_approve` kann Terminals sehen, aber keine Eingaben senden (`403`).
  - [ ] AC3 — `cat ~/.ssh/id_rsa` in einem User-Terminal mit Default-Sandbox scheitert.
- **Abhängigkeiten:** SES-017, SBX-017 (siehe 04-sandbox.md), TUI-002, WEB-010 (siehe 08-clients.md)

### COL-001 — Session-Freigaben mit Rollen
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Der Owner gibt eine Session an User oder Teams frei, mit Rolle `view`, `comment_approve` oder `drive` gemäß Rollenmatrix. Jede API- und WS-Operation prüft die effektive Rolle serverseitig; die stärkste aus direkter und Team-Freigabe gilt. Freigaben sind jederzeit widerrufbar und wirken sofort auf verbundene Clients.
- **Details:** `GET|POST|PATCH|DELETE /v1/sessions/{id}/shares` `{principal: user:<id>|team:<id>, role}`. Server-Setting `sharing.mode: on | view_only | off`. Login-freie öffentliche Links sind v2. Events `share.granted|changed|revoked` (PROTO-002). COL-001 definiert Rollensemantik und Share-API; Owner der Durchsetzung (`authorize`, Routen, Kanäle) ist AUTH-015.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein `view`-User, der `POST …/input` aufruft, erhält `403`; derselbe Aufruf mit `drive` erzeugt einen Turn.
  - [ ] AC2 — Widerruf trennt den WS des Betroffenen innerhalb von 2 s (Close-Code `4403`).
  - [ ] AC3 — Bei `sharing.mode=view_only` lehnt der Server Freigaben mit `drive`/`comment_approve` ab.
  - [ ] AC4 — Workspace-Dateien sind für `view` nur sichtbar, wenn `share.workspace_files=true`.
- **Abhängigkeiten:** AUTH-014, AUTH-015 (siehe 05-security-identity.md), SES-002

### COL-002 — Share-Dialog, Einladungen & „Mit mir geteilt“
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Clients bieten einen Share-Dialog (Suche nach Usern/Teams der Org, Rollenwahl, Liste bestehender Freigaben, Link kopieren). Empfänger sehen geteilte Sessions in „Mit mir geteilt“ und können eine Freigabe verlassen. Ein Warnhinweis macht klar, dass `drive` Code-Ausführung auf dem Host des Owners bedeutet.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach Freigabe erscheint die Session beim Empfänger in „Mit mir geteilt“ und ein Inbox-Eintrag „Session geteilt“ entsteht.
  - [ ] AC2 — „Freigabe verlassen“ entfernt die Session aus der Liste des Empfängers und erzeugt `share.revoked {by: grantee}`.
  - [ ] AC3 — Beim Vergeben von `drive` muss der Owner den Warnhinweis bestätigen (E2E-Test).
- **Abhängigkeiten:** COL-001, COL-009

### COL-003 — Co-Drive mit Autor-Attribution im Modell-Kontext
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Mehrere `drive`-Berechtigte steuern dieselbe Session gleichzeitig (Input, Queue, Steer, Interrupt, Approvals). Sobald mehr als ein Mensch Input geliefert hat, werden User-Nachrichten im Modell-Kontext mit dem Autor präfixiert, damit der Agent Anweisungen unterscheiden kann. Die UI zeigt Avatar/Namen statt des Präfixes.
- **Details:** Format `[<Anzeigename> (@<handle>)]: <text>`. `collaboration.attribution: auto | always | off` (Default `auto`). Das Event speichert `actor.user_id` immer; das Präfix wird nur beim Rendern für den Harness erzeugt. Auch Approval-Entscheidungen tragen den Entscheider (`approval.resolved.actor`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given zwei Autoren, then enthält der an den Harness gesendete Input des zweiten Autors das Präfix (Golden-Test mit Fake-Harness); der erste Turn eines Single-Autors bleibt unpräfixiert.
  - [ ] AC2 — Gleichzeitige Inputs zweier Driver landen geordnet in der Queue, keiner geht verloren.
  - [ ] AC3 — Mit `attribution=off` werden keine Präfixe gesendet, die Events tragen dennoch `actor`.
- **Abhängigkeiten:** COL-001, SES-004

### COL-004 — Presence
- **Meilenstein:** M4 · **Priorität:** Should
- **Beschreibung:** Clients melden, wer eine Session gerade betrachtet, welches Panel/welche Datei offen ist und ob jemand im Composer tippt. Presence ist ephemer (nicht im Log) und dient auch der Notification-Unterdrückung (COL-010).
- **Details:** Ephemeres Event `presence.updated {users:[{user_id, devices:[{kind: desktop|web|pwa|tui|cli, focused, view: chat|file:<path>|diff:<path>|terminal:<id>|browser}], typing}]}`. Heartbeat 15 s, Ablauf nach 45 s.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Öffnet B die Session, sieht A innerhalb von 2 s Bs Avatar; schließt B den Tab, verschwindet er spätestens nach 45 s.
  - [ ] AC2 — Tippen von B zeigt bei A einen Indikator, ohne den Entwurfstext zu übertragen.
  - [ ] AC3 — Presence-Events werden nicht in den Event-Log geschrieben.
- **Abhängigkeiten:** SES-002

### COL-005 — Inline-Kommentare an Nachrichten
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Berechtigte (ab `comment_approve`) kommentieren Agent- oder User-Nachrichten, optional an einem markierten Textbereich. Kommentare bilden Threads mit Antworten, können aufgelöst werden und erzeugen Events im Session-Log.
- **Details:** Anchor `{kind: "message", seq, range?: {start, end}}` (UTF-16-Offsets im gerenderten Markdown-Quelltext). Events `comment.added|updated|resolved|deleted` (`comment.added` laut ADR-0019).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Kommentar an einem Textbereich wird bei allen Clients am selben Bereich hervorgehoben.
  - [ ] AC2 — Ein `view`-User kann Kommentare lesen, aber nicht anlegen (`403`).
  - [ ] AC3 — Aufgelöste Threads sind standardmäßig eingeklappt und per Filter wieder sichtbar.
- **Abhängigkeiten:** COL-001, PROTO-002 (siehe 06-data-sync-protocol.md)

### COL-006 — Inline-Kommentare an Dateien und Diff-Zeilen
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Kommentare können an Zeilenbereichen einer Datei oder an Diff-Zeilen (alte/neue Seite) hängen. Ändert sich die Datei, wird der Anker re-lokalisiert; gelingt das nicht, wird der Kommentar als `outdated` markiert statt zu verschwinden.
- **Details:** Datei-Anker `{kind: "file", path, blob_sha, lines: [from, to], context_hash}`; Diff-Anker `{kind: "diff", path, side: old|new, line, base_sha, head_sha}`. Re-Anchoring: exakte Zeileninhalte im Fenster ±50 Zeilen suchen, sonst `outdated`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Fügt der Agent oberhalb eines kommentierten Bereichs 10 Zeilen ein, zeigt der Kommentar danach auf die verschobenen Zeilen.
  - [ ] AC2 — Wird der kommentierte Bereich gelöscht, ist der Kommentar `outdated` und zeigt den ursprünglichen Ausschnitt.
  - [ ] AC3 — Diff-Kommentare sind in der Diff-Ansicht und in der Kommentar-Übersicht des Workspace-Rails sichtbar.
- **Abhängigkeiten:** COL-005, SES-018

### COL-007 — Kommentare an den Agent adressieren
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Ein oder mehrere Kommentare werden gebündelt „an den Agent adressiert“: beton erzeugt daraus einen strukturierten Input (Anker, zitierter Inhalt, Kommentartext, Autor) und reiht ihn in die Queue ein. Adressieren erfordert `drive`; `comment_approve` kann Kommentare als „für Agent vorgeschlagen“ markieren.
- **Details:** Input-Block pro Kommentar: `Datei src/app.ts Z. 40–52 (Kommentar von @anna): <text>` + Codeausschnitt ≤ 40 Zeilen. Nach dem Turn erhält der Thread eine Systemantwort „Adressiert in Turn #n“ mit Link; Status `addressed`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Drei ausgewählte Kommentare erzeugen genau einen Queue-Eintrag mit drei Blöcken (Snapshot-Test).
  - [ ] AC2 — Nach Abschluss des Turns sind die Threads `addressed` und verlinken den Turn.
  - [ ] AC3 — `comment_approve`-User erhalten beim Adressieren `403`, können aber `suggested_for_agent=true` setzen.
- **Abhängigkeiten:** COL-005, COL-006, SES-004

### COL-008 — Side-Chats
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Ein Side-Chat ist ein versteckter Fork der aktuellen Session für Rückfragen („Was macht diese Funktion?“), ohne die Haupt-Session zu stören. Er erscheint nicht in der Session-Liste, sondern als Tab im Workspace-Rail der Eltern-Session, läuft per Default mit **read-only**-Workspace *(Annahme)* und kann sein Ergebnis in die Haupt-Session übernehmen.
- **Details:** `POST …/fork {kind: "side_chat", at_seq: current, workspace: "shared_readonly"}`; Slash-Befehl `/side <frage>`. Sichtbarkeit: Default nur für den Ersteller, teilbar an Mitglieder der Eltern-Session *(Annahme)*. „In Hauptsession übernehmen“ reiht eine Zusammenfassung (oder die letzte Antwort) als Input in die Queue der Eltern-Session; die Zusammenfassung erzeugt der Harness des Side-Chats als zusätzlichen Turn (Subscription über die Vendor-CLI, kein separater API-Aufruf, ADR-0034). Kosten zählen zum Budget der Eltern-Session. Archivieren/Löschen der Eltern-Session erfasst Side-Chats.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Side-Chat fehlt in `GET /v1/sessions`, erscheint in `GET /v1/sessions/{parent}/side-chats`.
  - [ ] AC2 — Ein Schreib-Tool-Call im Side-Chat scheitert an der Sandbox (read-only).
  - [ ] AC3 — „Übernehmen“ erzeugt einen Queue-Eintrag in der Eltern-Session mit Herkunftsverweis.
  - [ ] AC4 — Löschen der Eltern-Session löscht ihre Side-Chats.
- **Abhängigkeiten:** SES-006, SBX-012 (siehe 04-sandbox.md)

### COL-009 — Inbox-Integration
- **Meilenstein:** M4 · **Priorität:** Should
- **Beschreibung:** Collaboration-Ereignisse erzeugen Inbox-Einträge zusätzlich zu den bestehenden (Approvals, Fragen, fertige Async-Agents; Inbox selbst: Owner UX-001 in 11-platform-features.md, Approvals: POL-010 in 03-policies.md): Freigabe erhalten, @-Erwähnung in Kommentar, Antwort in eigenem Thread, adressierter Kommentar erledigt, Approval in geteilter Session.
- **Details:** Inbox-Item gemäß UX-001 `{type, session_id, anchor?, actor, created_at, state: open|done|dismissed}`; ein Approval-Item verschwindet bei allen Berechtigten, sobald einer es entschieden hat.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `@anna` in einem Kommentar erzeugt genau ein Inbox-Item bei Anna mit Deep-Link zum Anker.
  - [ ] AC2 — Entscheidet A ein Approval, ist das entsprechende Item bei B innerhalb von 2 s `done` mit Vermerk „entschieden von A“.
- **Abhängigkeiten:** COL-005, UX-001 (siehe 11-platform-features.md), POL-010 (siehe 03-policies.md)

### COL-010 — Notification-Routing
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Der Server entscheidet zentral, wer bei welchem Ereignis auf welchem Kanal benachrichtigt wird (In-App, Desktop-Notification, Web-Push; Kanäle selbst: DESK-005, WEB-014, siehe 08-clients.md; User-Einstellungen: UX-006). Web-Push nutzt Push-Dienste der Browser-Hersteller und ist daher nur aktiv, wenn der User ihn eingeschaltet hat; Standard sind In-App/Inbox und lokale Desktop-Notifications (ADR-0033). Benachrichtigt wird nicht, wenn der Empfänger die Session gerade auf irgendeinem Gerät fokussiert betrachtet (Presence).
- **Details:** Default-Regeln: `approval.requested` → Owner + alle `comment_approve`/`drive`; `turn.completed` → Autor des letzten Inputs; `session.status=failed` / Runner-Disconnect → Owner; Erwähnung/Thread-Antwort → Betroffener; Share erhalten → Empfänger. User-Einstellungen pro Typ × Kanal; Benachrichtigungen werden pro (User, Session, Typ) innerhalb von 30 s zusammengefasst.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Betrachtet der Owner die Session fokussiert im Desktop, erhält er bei `turn.completed` weder Desktop- noch Push-Notification.
  - [ ] AC2 — Fünf Approvals binnen 10 s erzeugen eine zusammengefasste Notification („5 Freigaben ausstehend“).
  - [ ] AC3 — Deaktiviert ein User Push für `turn.completed`, erhält er dafür nur In-App-Hinweise.
- **Abhängigkeiten:** COL-004, DESK-005, WEB-014 (siehe 08-clients.md), UX-006 (siehe 11-platform-features.md)

### GIT-001 — Git-Provider-Trait & Registry
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** `beton-git` definiert den `GitProvider`-Trait (siehe Design) mit neutralen Typen (`ChangeRequest`, `Check`, `ReviewThread`, `UnifiedDiff`). Eine Registry ordnet Remote-URLs anhand konfigurierter Hosts einem Provider zu; Community-Provider kommen als Out-of-Process-Plugins mit demselben Vertrag (PLG-002, ab M5, siehe 10-runners-extensibility.md). Git-Provider sind optional: Ohne `git.providers`-Konfiguration bzw. ohne verbundenes Konto kontaktiert beton keine Provider-API; Sessions, Worktrees, Diffs und Commits funktionieren rein lokal (ADR-0033).
- **Details:**
  ```yaml
  # ~/.beton/config.yaml bzw. Server-Config
  git:
    providers:
      - { kind: github, host: github.com }
      - { kind: github, host: ghe.acme.corp, api_url: https://ghe.acme.corp/api/v3 }
      - { kind: gitlab, host: gitlab.acme.corp, api_url: https://gitlab.acme.corp/api/v4 }
  ```
- **Akzeptanzkriterien:**
  - [ ] AC1 — Die Remotes `git@ghe.acme.corp:team/x.git` und `https://gitlab.acme.corp/g/sub/x.git` werden dem jeweils richtigen Provider und `RepoRef` zugeordnet (Unit-Tests inkl. verschachtelter GitLab-Gruppen).
  - [ ] AC2 — Ein Remote ohne passenden Provider führt zu „kein Provider“ im Panel statt zu einem Fehler.
  - [ ] AC3 — Ein Fake-Provider im Test-Harness implementiert den Trait und besteht dieselbe Contract-Testsuite wie GitHub/GitLab (gegen Mock-Server).
  - [ ] AC4 — Ohne `git.providers`-Konfiguration baut der Server bei Session-Start, Worktree-Erstellung und Diff-Ansicht keine Verbindung zu einer Provider-API auf (Netz-Mock-Test).
- **Abhängigkeiten:** PLG-002 (ab M5, siehe 10-runners-extensibility.md)

### GIT-002 — GitHub-Provider (inkl. Enterprise Server)
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Implementierung für github.com und GitHub Enterprise Server über REST v3 und GraphQL v4 (Review-Threads mit Resolved-Status). Checks kombinieren Check-Runs und Commit-Statuses.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Contract-Tests gegen aufgezeichnete Antworten (github.com und GHES-API-Pfad `/api/v3`) bestehen.
  - [ ] AC2 — Check-Runs und Legacy-Statuses eines Commits erscheinen in einer gemeinsamen Liste mit Status `queued|running|success|failure|cancelled|skipped`.
  - [ ] AC3 — Conditional Requests (`If-None-Match`) werden genutzt; `304` verbraucht kein neues Parsing.
- **Abhängigkeiten:** GIT-001, GIT-004

### GIT-003 — GitLab-Provider (inkl. self-hosted)
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Implementierung für gitlab.com und self-hosted GitLab über REST v4: Merge-Requests, Discussions (Diff-Notes), Pipelines und Jobs. Self-signed-Zertifikate sind per `ca_file` konfigurierbar.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Contract-Tests gegen aufgezeichnete Antworten bestehen, inkl. Projekt-Pfaden mit URL-Encoding (`g%2Fsub%2Fx`).
  - [ ] AC2 — Pipeline-Jobs werden als `Check`-Liste abgebildet; ein fehlgeschlagener Job liefert `check_log_tail`.
  - [ ] AC3 — Ein Host mit eigener CA funktioniert mit `ca_file`; ohne schlägt die Verbindung mit klarer TLS-Fehlermeldung fehl.
- **Abhängigkeiten:** GIT-001, GIT-004

### GIT-004 — Provider-Verbindung & Credential-Nutzung
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Jeder User verbindet seine Provider-Konten per OAuth-App (GitHub inkl. Enterprise, GitLab inkl. self-hosted) oder PAT-Fallback; Tokens liegen in `beton-secrets`; OAuth-Flow, PAT-Fallback und Speicherung sind Owner SEC-009, SEC-010, SEC-011 (siehe 05-security-identity.md), GIT-004 regelt die Nutzung im Panel und die Proxy-Bindings pro Session. Panel-Abfragen laufen serverseitig im Namen des betrachtenden Users. Git- und CLI-Aufrufe im Sandbox (`git push`, `gh`, `glab`) sehen nur `bt_cred_*`-Platzhalter; der Egress-/Credential-Proxy injiziert den echten Wert (PRX-006, siehe 04-sandbox.md).
- **Details:** Credential-Bindung `(org, user, provider, account)`. Proxy-Bindings werden pro Session automatisch für die Provider-Hosts des Repos angelegt (`github.com`, `api.github.com`, GHES-/GitLab-Host). Jede Nutzung erzeugt einen Audit-Eintrag (ohne Wert).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `env | grep -i token` in einem Session-Terminal zeigt höchstens `bt_cred_*`-Werte; `git push` funktioniert dennoch.
  - [ ] AC2 — Ein Viewer ohne eigene Provider-Verbindung sieht im Panel den Hinweis „Konto verbinden“, nicht die Daten mit dem Token des Owners.
  - [ ] AC3 — Jede Token-Nutzung (Panel oder Proxy) erzeugt einen Audit-Eintrag mit User, Session, Host.
- **Abhängigkeiten:** SEC-009, SEC-010, SEC-011 (siehe 05-security-identity.md), PRX-006 (siehe 04-sandbox.md)

### GIT-005 — CR-Tracking pro Session (created / attached / inferred)
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Jede Session führt eine Liste verknüpfter Change Requests mit Herkunft: `created` (über GIT-009 oder erkannt in Tool-Ausgabe von `gh pr create`/`glab mr create` dieser Session), `attached` (vom User per URL verknüpft), `inferred` (Head-Branch entspricht dem Worktree-Branch oder CR-URL in der Konversation). Inferred-Einträge lassen sich verwerfen; mehrere CRs (auch Multi-Repo) pro Session sind erlaubt.
- **Details:** `GET|POST|DELETE /v1/sessions/{id}/change-requests`; Event `cr.linked {url, origin}` / `cr.unlinked` (PROTO-002).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Führt der Agent `gh pr create` aus und gibt die CLI eine PR-URL aus, erscheint die PR mit `origin=created` im Panel.
  - [ ] AC2 — Ein manuell per URL angehängter MR erscheint mit `origin=attached`; eine fremde, unzugängliche URL liefert eine verständliche Fehlermeldung.
  - [ ] AC3 — Ein verworfener `inferred`-Eintrag wird nicht erneut inferiert.
- **Abhängigkeiten:** GIT-001, SES-015

### GIT-006 — CR-Panel: Status & CI-Checks/Pipelines
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Das Panel (Tab im Workspace-Rail) zeigt je CR Titel, Status (draft/open/merged/closed), Mergeability, Review-Status und CI-Checks. Ein fehlgeschlagener Check kann mit einem Klick samt Log-Ausschnitt an den Agent gesendet werden. Da v1 keinen Webhook-Empfänger hat, wird gepollt.
- **Details:** Polling: 30 s bei mindestens einem fokussierten Viewer, sonst 5 min, nach Merge/Close 1×/h für 24 h, dann aus. Rate-Limit-Header werden respektiert (Backoff bei < 10 % Rest). „An Agent senden“ nutzt `check_log_tail(lines=200)`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Wechselt ein Check auf `failure`, zeigt das Panel eines fokussierten Viewers dies spätestens nach 35 s.
  - [ ] AC2 — „An Agent senden“ erzeugt einen Queue-Eintrag mit Check-Name, Status und ≤ 200 Log-Zeilen.
  - [ ] AC3 — Bei erschöpftem Rate-Limit stoppt das Polling bis zum Reset und das Panel zeigt den Zeitpunkt.
- **Abhängigkeiten:** GIT-005, SES-004

### GIT-007 — Review-Kommentare im Panel
- **Meilenstein:** M4 · **Priorität:** Should
- **Beschreibung:** Review-Threads des Providers werden im CR-Panel und als Anker in der Diff-Ansicht dargestellt. User können (mit eigenem Provider-Token) antworten und Threads per „an Agent adressieren“ in einen Input überführen, analog COL-007.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Review-Kommentar an Zeile 12 einer Datei erscheint in der Diff-Ansicht an derselben Zeile.
  - [ ] AC2 — Eine Antwort aus beton erscheint beim Provider unter dem Namen des antwortenden Users.
  - [ ] AC3 — Adressieren erzeugt einen Queue-Eintrag mit Thread-Inhalt und Codeausschnitt; resolved Threads sind ausgegraut.
- **Abhängigkeiten:** GIT-006, COL-007

### GIT-008 — CR-Diff
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Das Panel zeigt den Diff des CR, wie ihn der Provider berechnet (gegen die Ziel-Branch), pro Datei einklappbar und gestapelt. Abweichungen zum lokalen Worktree (ungepushte Commits, uncommitted Changes) werden angezeigt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Der angezeigte Diff entspricht dem Provider-Diff (Vergleich gegen aufgezeichnete Antwort).
  - [ ] AC2 — Hat der Worktree ungepushte Commits, zeigt das Panel „n Commits nicht gepusht“ mit Push-Aktion (sofern Policy erlaubt).
- **Abhängigkeiten:** GIT-005, SES-018

### GIT-009 — CR aus der Session erstellen
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Aus einer Session mit Worktree-Branch erstellt der User per Aktion einen PR/MR: Branch pushen (im Tool-Sandbox mit Credential-Proxy), Titel/Beschreibung vorschlagen (deterministisch aus Session-Titel und Commit-Liste, ohne Modellaufruf, editierbar), Ziel-Branch = Worktree-Base, optional Draft. Push und Erstellung durchlaufen Policies.
- **Details:** `POST /v1/sessions/{id}/change-requests {repo?, title, body, draft, base?}`. Policy-Kontext: `tool.name == "git.push"` bzw. `"cr.create"` (Variablen-Namen final in POL-004/POL-005, siehe 03-policies.md). Die Beschreibung erhält einen abschaltbaren Footer mit Session-Link.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given ein Worktree mit 2 Commits, when „PR erstellen“, then ist der Branch gepusht, der PR existiert und ist mit `origin=created` verknüpft.
  - [ ] AC2 — Eine Policy `ask` für `git.push` erzeugt vor dem Push eine Approval-Card; bei `deny` entsteht kein PR.
  - [ ] AC3 — Ohne Commits gegenüber Base ist die Aktion deaktiviert mit Begründung.
- **Abhängigkeiten:** GIT-004, GIT-005, SES-015, POL-017 (siehe 03-policies.md)

### GIT-010 — Merge-Konflikte erkennen
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** beton erkennt, wenn sich der Branch einer Session nicht konfliktfrei in seine Ziel-Branch mergen lässt, und zeigt das im CR-Panel, in der Session-Kopfzeile und in der Inbox an. Die Erkennung läuft **lokal** im Worktree per Probe-Merge ohne Änderungen am Arbeitsverzeichnis (`git merge-tree --write-tree <base> <head>`, Git ≥ 2.38) und braucht weder Provider-API noch Netzwerk (ADR-0033); ist ein Provider verbunden, wird zusätzlich dessen Mergeability-Status übernommen (GIT-006).
- **Details:** Prüfung bei Session-Start mit Worktree, nach jedem Commit der Session, nach `git fetch` und beim Öffnen des CR-Panels; Ziel-Branch aus dem verknüpften CR, sonst Worktree-Base (SES-015). Ohne Netz wird gegen den lokalen Stand der Ziel-Branch geprüft und das Alter dieses Stands angezeigt („Ziel-Branch zuletzt geholt vor 3 Std.“). Ergebnis als Event `git.conflicts_detected` (PROTO-002) mit Dateiliste und Konfliktart (`content`, `delete_modify`, `rename`, `binary`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given ein Worktree-Branch und eine Ziel-Branch, die dieselbe Zeile unterschiedlich ändern, when die Prüfung läuft, then entsteht `git.conflicts_detected` mit genau dieser Datei und `kind=content`, und das Arbeitsverzeichnis ist unverändert (`git status` sauber).
  - [ ] AC2 — Die Prüfung funktioniert ohne Netzwerk und ohne konfigurierten Git-Provider (Netz-Namespace-Test).
  - [ ] AC3 — Meldet ein verbundener Provider `mergeable=false`, zeigt das CR-Panel „Konflikte“ mit der Aktion „Konflikte lösen“ auch dann, wenn die lokale Prüfung (wegen veralteter Ziel-Branch) noch keinen Konflikt sieht; der Hinweis empfiehlt, die Ziel-Branch zu holen.
  - [ ] AC4 — Delete/Modify-, Rename- und Binärkonflikte werden mit ihrer Art erkannt, nicht als Inhaltskonflikt.
- **Abhängigkeiten:** SES-015, GIT-006, PROTO-002 (siehe 06-data-sync-protocol.md)

### GIT-011 — Konflikt-Resolver-Ansicht
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** „Konflikte lösen“ startet im Worktree der Session einen Merge der Ziel-Branch (Standard; kein Umschreiben der History, kein Force-Push nötig) oder auf Wunsch einen Rebase, und öffnet die Resolver-Ansicht: links die Liste der Konfliktdateien mit Status (offen, gelöst, automatisch gelöst), rechts je Konflikt-Hunk eine Drei-Wege-Darstellung **Basis · Dein Branch · Ziel-Branch** mit dem bearbeitbaren Ergebnis darunter (Monaco, WEB-009). Je Hunk: „Deine Seite“, „Ziel-Seite“, „Beide (deine zuerst / Ziel zuerst)“, „Bearbeiten“; je Datei: ganz übernehmen. Sonderfälle: Delete/Modify (behalten oder löschen), Rename, Binärdateien (eine Seite wählen).
- **Details:** Während des Resolvens pausiert der Agent der Session (Status `paused`), bis der Resolver abgeschlossen oder abgebrochen ist; andere Sessions laufen weiter. Schreibrechte und Befehle (`git merge`, `git rebase`, `git add`) laufen über den Exec-Broker in der Sandbox (SBX-002, siehe 04-sandbox.md) und durch Policies (POL-017 `git_guard`, siehe 03-policies.md). Fortschritt als `git.conflict_resolution` (PROTO-002). „Abbrechen“ führt `git merge --abort` bzw. `git rebase --abort` aus und stellt den Stand vor dem Start wieder her.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Given ein erkannter Inhaltskonflikt, when der User „Konflikte lösen“ wählt, then läuft `git merge <ziel>` im Worktree, und die Ansicht zeigt für jeden Hunk Basis, beide Seiten und das Ergebnis.
  - [ ] AC2 — „Beide (deine zuerst)“ ergibt im Ergebnis die Zeilen der eigenen Seite gefolgt von denen der Ziel-Seite, ohne Konfliktmarker.
  - [ ] AC3 — Eine Datei gilt erst als gelöst, wenn ihr Ergebnis keine Konfliktmarker (`<<<<<<<`, `=======`, `>>>>>>>`) mehr enthält; „Abschließen“ ist bis dahin deaktiviert und nennt die offenen Stellen.
  - [ ] AC4 — „Abbrechen“ stellt Branch und Arbeitsverzeichnis exakt auf den Stand vor dem Start zurück (Vergleich von `HEAD` und `git status`).
  - [ ] AC5 — Ein Rebase-Lauf zeigt die Konflikte Commit für Commit („Commit 2 von 5“); der anschließend nötige Force-Push ist eine eigene, per Policy freizugebende Aktion.
- **Abhängigkeiten:** GIT-010, WEB-009, SES-018, SBX-002 (siehe 04-sandbox.md), POL-017 (siehe 03-policies.md)

### GIT-012 — Konflikte vom Agent lösen lassen
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Im Resolver kann der User einzelne Hunks, eine Datei oder alle Konflikte an den Agent der Session geben. Der Agent erhält Basis, beide Seiten, die Commit-Nachrichten beider Seiten und den Session-Kontext und schreibt einen Lösungsvorschlag mit kurzer Begründung ins Ergebnis. Vorschläge sind als solche markiert und gelten erst nach Bestätigung durch den User als gelöst. Der Aufruf läuft über den Harness der Session und damit über die Subscription der Vendor-CLI (ADR-0034).
- **Akzeptanzkriterien:**
  - [ ] AC1 — „Agent lösen lassen“ für einen Hunk erzeugt einen Vorschlag mit Begründung; der Hunk bleibt „offen“, bis der User „Vorschlag übernehmen“ wählt (Fake-Harness-Test).
  - [ ] AC2 — Lehnt der User einen Vorschlag ab, wird der vorherige Ergebnisstand wiederhergestellt.
  - [ ] AC3 — Der Agent-Aufruf funktioniert ohne API-Key mit einer per CLI angemeldeten Subscription; die Kosten bzw. das Kontingent erscheinen in der Verbrauchsanzeige der Session.
  - [ ] AC4 — Die Policy-Engine prüft die Dateischreibungen des Agents wie jeden anderen Tool-Call (`policy.decision`-Event je Schreibvorgang).
- **Abhängigkeiten:** GIT-011, HAR-015 (siehe 01-harnesses.md), POL-025 (siehe 03-policies.md)

### GIT-013 — Konfliktlösung abschließen
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** „Abschließen“ markiert die Dateien als gelöst (`git add`), erstellt den Merge-Commit bzw. setzt den Rebase fort, bietet an, die Tests der Session laufen zu lassen, und danach den Branch zu pushen. Der Push läuft als Policy-geprüfte Aktion (Approval-Card bei `ask`); bei Rebase ist er ein Force-Push mit Lease (`--force-with-lease`). Danach aktualisiert das CR-Panel die Mergeability.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach „Abschließen“ existiert genau ein Merge-Commit mit beiden Eltern, und `git.conflict_resolution {phase: completed}` enthält Anzahl und Art der Lösungen je Datei.
  - [ ] AC2 — Ein Push nach einem Rebase nutzt `--force-with-lease`; ohne Freigabe durch `git_guard` wird nicht gepusht.
  - [ ] AC3 — Ohne Netzwerk endet der Abschluss lokal mit dem Commit; der Push wird als ausstehend angezeigt und lässt sich später auslösen.
  - [ ] AC4 — Nach erfolgreichem Push zeigt das CR-Panel spätestens beim nächsten Poll „konfliktfrei“ bzw. den neuen Provider-Status.
- **Abhängigkeiten:** GIT-011, GIT-009, POL-017 (siehe 03-policies.md)

## Nicht in v1

- Öffentliche, Login-freie Share-Links (v2, ADR-0014).
- Canvas-Ansicht (räumliches Board mit Session-Karten) (v2).
- Slack-Bot als Collaboration-Kanal (v2).
- Eingebauter Empfänger für GitHub-/GitLab-Webhooks (v1 pollt; Webhooks nur über die beton-API, siehe ASY-007 in 02-agents.md).
