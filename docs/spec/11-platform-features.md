# 11 — Plattform-Features

Dieses Kapitel bündelt querschnittliche Produktfunktionen, die auf allen Harnesses und Clients gleich funktionieren: **Usage & Kosten** (USE), **Spracheingabe** mit lokalem Whisper (VOI), **UX-Grundfunktionen** wie Inbox, Command-Palette, Themes und Onboarding (UX) sowie **Observability** für Betreiber und Nutzer (OBS).

Bezug: ADR-0021 (weitere v1-Features), ADR-0023 (Spracheingabe nur lokal), ADR-0025 (Telemetrie opt-in, OTel/Prometheus), ADR-0022 (Smart Routing → v2). Rohdaten für Usage und Kontext liefern die Adapter (`cost.delta`, `context.usage`, `usage.subscription`; HAR-021 in 01-harnesses.md); Session-Mechanik für Titel und Compaction liegt in SES-010/SES-011 (siehe 07-sessions-collaboration.md); Budgets/Spend-Caps in POL-011/POL-012 (siehe 03-policies.md); Benachrichtigungs-Routing in COL-010 (siehe 07-sessions-collaboration.md); Darstellung in Desktop/Web/CLI/TUI in 08-clients.md. Dieses Kapitel definiert Berechnung, Aggregation, UI-Verhalten und Betriebsaspekte.

## Konzepte & Begriffe

| Begriff | Bedeutung |
| --- | --- |
| **Usage-Record** | Normalisierter Verbrauchseintrag pro `cost.delta`: Tokens (input/output/cache_read/cache_write), Modell, Harness, `auth_source`, berechnete Kosten, Preisquelle. |
| **Preis-Katalog** | Versionierte, mit beton ausgelieferte Preistabelle (USD pro 1 Mio. Tokens) inkl. Kontextfenster je Modell. |
| **Preis-Override** | Vom User/Admin definierter Preis für Gateway-/Self-hosted-Modelle (inkl. Cache-Raten oder `free`). |
| **API-Äquivalent** | Rechnerischer Preis einer Subscription-Nutzung zu API-Preisen; nur informativ, nie Teil von Geld-Budgets. |
| **Subscription-Usage** | Token- und Rate-Limit-Nutzung bei `auth_source: vendor_cli` (Subscription über die offizielle CLI: Claude Pro/Max, ChatGPT Plus/Pro; HAR-015). |
| **Inbox-Item** | Offene Aufgabe für einen User: Approval, Frage, fertiger Async-Agent, Mention u.a. |
| **Diktat** | Sprachaufnahme → lokale Whisper-Transkription → Text im Composer. |
| **Feature-Flag** | Interner Schalter für unfertige Funktionen; kein Lizenz- oder Berechtigungsmechanismus. |
| **Produkt-Telemetrie** | Anonyme, opt-in Nutzungsstatistik an das beton-Projekt. Abzugrenzen von **Betreiber-Observability** (Logs, Traces, Metriken), die nur beim Betreiber bleibt. |

## Design

### Kostenberechnung (Reihenfolge der Preisquellen)

Für jedes `cost.delta` mit `auth_source ∈ {api_key, gateway}` bestimmt `beton-core::pricing` den Preis:
1. **Preis-Override** (USE-003), wenn `match` greift → `cost_source: override`.
2. **Harness-gemeldete Kosten** (z.B. Claude `total_cost_usd`) → `cost_source: harness`.
3. **Preis-Katalog** (USE-002) → `cost_source: catalog`, mit `pricing_version`.
4. Sonst `cost_usd: null`, `cost_source: none` → als „ohne Preis“ markiert.

Bei `auth_source: vendor_cli` (Subscription) fließt nichts in Geld-Summen; zusätzlich wird optional das API-Äquivalent (Katalog) als `equivalent_usd` gespeichert. Cache-Raten fehlen → Fallback `cache_read = 0,1 × input`, `cache_write = 1,25 × input`. Interne Rechnung in USD (Ganzzahlen in Mikro-USD, also 6 Nachkommastellen; Felder `*_micro`); alle Geldbeträge in Spec, Policies und Protokoll sind USD, angezeigt als „0,42 $“; Anzeige optional in EUR mit manuell konfiguriertem Kurs (`usage.display_currency: EUR`, `usage.fx_rate_usd_eur: 0.92`) *(Annahme: kein automatischer Kursabruf)*.

### Preis-Katalog (Ausschnitt, Werte illustrativ)

```toml
# crates/beton-core/pricing/catalog.toml — eingebettet per include_str!
catalog_version = "2026-10-01"
currency = "USD"

[[model]]
id = "example-sonnet"            # kanonische Modell-ID
aliases = ["example-sonnet-2026-08-01"]
provider = "anthropic"
context_window = 200000
input_per_mtok = 3.00
output_per_mtok = 15.00
cache_read_per_mtok = 0.30
cache_write_per_mtok = 3.75
valid_from = "2026-08-01"
```

### Preis-Overrides (User-, Projekt- oder Server-Config)

```yaml
pricing:
  overrides:
    - match: { provider: litellm-intern, model: "llama-3.3-70b*" }
      input_per_mtok: 0.40
      output_per_mtok: 0.80
      cache_read_per_mtok: 0.04      # optional
      cache_write_per_mtok: 0.50     # optional
      context_window: 131072
    - match: { provider: ollama-local }
      free: true
```

### Voice-Pipeline

```
Client (Mic) ─ AudioWorklet: 16 kHz mono PCM16 ─▶ WS-Binärkanal "voice"
   ▲                                                     │
   │  voice.partial / voice.final (JSON, Kontrollkanal)  ▼
   └──────────────────────────── beton-voice (Daemon/Server)
                                   ├─ Ring-Puffer (max. 120 s)
                                   ├─ Partial alle ~1 s: Re-Transkription der letzten ≤ 30 s
                                   ├─ Sprach-Erkennung auf {de, en} begrenzt
                                   └─ whisper-rs (Metal | CUDA | CPU), Modell lazy geladen
```

### Inbox-Item (Projektion aus Events)

```json
{ "id": "inb_01J…", "type": "approval", "session_id": "ses_01J…", "ref": { "approval_id": "apr_…" },
  "title": "bash: git push origin main", "state": "open", "created_at": "2026-10-03T09:12:00Z",
  "actions": ["approve", "deny"] }
```
Typen: `approval`, `question`, `async_done`, `async_failed`, `mention` (+ weitere aus COL-009, siehe 07-sessions-collaboration.md; Async-Einträge aus ASY-009). Zustände: `open → done | dismissed`. Die Inbox ist eine **Projektion** des Event-Logs, keine eigene Wahrheit.

## Features

### USE — Usage & Kosten

### USE-001 — Usage-Records & Normalisierung
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Jedes `cost.delta` wird in einen Usage-Record überführt und in einer Projektion (`usage_records`) mit den Dimensionen Session, Projekt, User, Team, Harness, Modell, `auth_source`, Kalendertag (UTC und Nutzer-Zeitzone) gespeichert. Die Policy-Variablen `session.cost_usd` und `user.daily_cost_usd` (POL-004, siehe 03-policies.md) werden aus derselben Projektion gespeist. USE-001 ist Owner der Usage-Projektion; DATA-005 liefert den Projektionsmechanismus.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach einem Fake-Harness-Turn mit `usage: {input_tokens: 1200, output_tokens: 80}` existiert genau ein Usage-Record mit diesen Werten und `cost_source` gemäß Design-Reihenfolge.
  - [ ] AC2 — Ein Rebuild der Projektion aus dem Event-Log ergibt identische Summen (Property-Test über zufällige Event-Folgen).
  - [ ] AC3 — Records mit `auth_source: vendor_cli` tragen nie zu Geld-Summen bei.
- **Abhängigkeiten:** HAR-021 (siehe 01-harnesses.md), DATA-005 (siehe 06-data-sync-protocol.md)
- **Referenz:** ADR-0021

### USE-002 — Versionierter Preis-Katalog
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** beton liefert einen Preis-Katalog (TOML, im Binary eingebettet) mit Preisen inkl. Cache-Raten, Kontextfenster und Gültigkeitsdatum je Modell. Jede Katalog-Änderung erhöht `catalog_version`; jeder kostenberechnete Record speichert die verwendete Version. Historische Kosten werden nie stillschweigend neu bewertet. Updates kommen ausschließlich mit beton-Releases (kein Netzabruf).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Record speichert `pricing_version`; nach Katalog-Update bleiben bestehende Records unverändert.
  - [ ] AC2 — Aliase (z.B. datierte Modell-IDs) werden auf das kanonische Modell aufgelöst (Unit-Tests je Katalogeintrag).
  - [ ] AC3 — Ein CI-Test validiert den Katalog (Schema, keine doppelten IDs/Aliase, Preise ≥ 0, `valid_from` gesetzt).
- **Abhängigkeiten:** USE-001
- **Referenz:** Omnigent `pricing` (Cache-Fallback 0,10×/1,25×)

### USE-003 — Eigene Preise für Gateway-Modelle
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Für Gateway-/Self-hosted-Modelle (Ollama, LiteLLM, OpenRouter, vLLM …) definieren User (lokal) bzw. Admins (zentral) Preis-Overrides mit Glob-Match auf `provider`/`model`, inkl. Cache-Raten oder `free: true`. Im zentralen Betrieb gelten ausschließlich serverseitige Overrides, damit Budgets konsistent bleiben. Preisangaben in der Provider-Konfiguration (HAR-011) werden als Overrides dieses Features behandelt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit dem Override-Beispiel aus dem Design kostet ein Turn mit 1 Mio. Input-Tokens auf `litellm-intern/llama-3.3-70b-instruct` 0,40 USD, `cost_source: override`.
  - [ ] AC2 — Fehlen Cache-Raten, wird `0,1 × input` bzw. `1,25 × input` verwendet.
  - [ ] AC3 — Mehrere passende Overrides: der erste in Deklarationsreihenfolge gewinnt; `beton usage pricing explain --model <m>` zeigt die Herleitung.
  - [ ] AC4 — Im zentralen Modus werden User-seitige Overrides ignoriert und `beton doctor` weist darauf hin.
- **Abhängigkeiten:** USE-002; HAR-011 (siehe 01-harnesses.md)
- **Referenz:** ADR-0021

### USE-004 — Subscription-Usage
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Bei Subscription-Nutzung zeigt beton Token-Verbrauch und — soweit die Vendor-CLI es meldet — Rate-Limit-Fenster (genutzter Anteil, Reset-Zeitpunkt) statt eines Geldbetrags. Quellen: Claude-`result`-Nachricht (Tokens je Modell), Codex-Notifications zu Token-Usage und Rate-Limits *(Annahme: gegen aktuelle CLI-Versionen verifizieren, siehe HAR-021 in 01-harnesses.md)*, `usage.subscription`-Events (PROTO-002). Liefert ein Harness nichts (z.B. ACP ohne Usage), zeigt beton „nicht gemeldet“ statt 0.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Claude-Subscription-Session zeigt Token-Summen pro Modell und kein Geld-Feld; optional das API-Äquivalent, klar als „informativ“ gekennzeichnet.
  - [ ] AC2 — Ein `usage.subscription {window: "5h", used_pct: 72, resets_at}` erscheint als Fortschrittsbalken mit Reset-Uhrzeit in Nutzer-Zeitzone.
  - [ ] AC3 — Bei `used_pct ≥ 90` wird ein Inbox-Hinweis erzeugt (abschaltbar, UX-006).
  - [ ] AC4 — Harnesses ohne Usage-Reporting werden in Usage-Ansichten als „nicht gemeldet“ geführt.
- **Abhängigkeiten:** USE-001; HAR-021 (siehe 01-harnesses.md)
- **Referenz:** ADR-0005, ADR-0021

### USE-005 — Usage-Aggregation & API
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** `GET /v1/usage` liefert aggregierte Zeitreihen und Summen mit `group_by` (`day`, `session`, `project`, `harness`, `model`, `auth_source`, ab M4 `user`, `team`), Filtern (`from`, `to`, `project`, `harness`, `user`, `team`) und getrennten Summen für Geld und Tokens. Sichtbarkeit nach Rolle: Member eigene Daten, Team-Admins ihr Team, Owner die Org (AUTH-014, siehe 05-security-identity.md). Tägliche Rollups halten Abfragen über 90 Tage unter 200 ms.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `GET /v1/usage?group_by=day,harness&from=…&to=…` liefert pro Tag/Harness `cost_usd`, `equivalent_usd`, Token-Summen und `unpriced_count`.
  - [ ] AC2 — Ein Member erhält für `user=<anderer>` HTTP 403.
  - [ ] AC3 — Abfrage über 90 Tage mit 1 Mio. Records antwortet in < 200 ms (Postgres, Benchmark in CI-Nightly).
- **Abhängigkeiten:** USE-001, AUTH-014 (ab M4, siehe 05-security-identity.md)
- **Referenz:** Omnigent `GET /v1/usage`

### USE-006 — Usage-Seite
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Web/Desktop-Seite „Usage“ mit Zeitraumwahl, Kennzahlen (heute, 7 Tage, Monat), Zeitreihe gestapelt nach Harness oder Modell, Tabellen nach Session/Projekt/Modell und separatem Bereich „Subscriptions“ (Tokens, Rate-Limit-Fenster). Ab M4 zusätzlich Dimensionen User/Team für berechtigte Rollen. CSV-Export der aktuellen Ansicht.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Die Seite zeigt für den Fake-Harness-Datensatz dieselben Summen wie `GET /v1/usage` (E2E-Test).
  - [ ] AC2 — Klick auf eine Session in der Tabelle öffnet die Session.
  - [ ] AC3 — CSV-Export enthält die sichtbaren Spalten und Filter im Dateinamen.
  - [ ] AC4 — Records ohne Preis werden mit Hinweis „ohne Preis – Override anlegen“ und Link zu den Pricing-Einstellungen dargestellt.
- **Abhängigkeiten:** USE-005; WEB-001 (siehe 08-clients.md)

### USE-007 — CLI `beton usage`
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** `beton usage [--since 7d|--from DATE --to DATE] [--by day|session|project|harness|model|user|team] [--session ID] [--json|--csv]` (Kommandobaum in 08-clients.md). USE-007 ist Owner der Implementierung; CLI-010 regelt die Konsistenz der CLI-Oberfläche. Ohne Argumente: Tabelle „heute“ und „letzte 7 Tage“ je Harness. stdout enthält nur Daten, Hinweise auf stderr.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `beton usage --json --by model --since 1d` liefert valides JSON gemäß veröffentlichtem Schema.
  - [ ] AC2 — `beton usage --session <id>` zeigt Tokens, Kosten und Preisquelle pro Turn.
  - [ ] AC3 — Ohne laufenden Daemon startet der Befehl diesen nicht, sondern liest direkt aus der lokalen DB (read-only).
- **Abhängigkeiten:** USE-005; CLI-001, CLI-010 (siehe 08-clients.md)

### USE-008 — Kontextfenster-Anzeige
- **Meilenstein:** M2 · **Priorität:** Should
- **Beschreibung:** Der Composer zeigt den Kontextfüllstand aus `context.usage` (SES-011, siehe 07-sessions-collaboration.md; USE-008 ist Owner der Darstellung) als Ring mit Prozentwert; Tooltip mit `used_tokens / window_tokens` und Quelle (`harness` oder `estimated`). Fehlt `window_tokens`, wird es aus Katalog/Override ergänzt; ist auch das unbekannt, zeigt die Anzeige nur absolute Tokens. Farbschwellen 70 % / 85 % / 95 %.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach einem `context.usage {used_tokens: 150000, window_tokens: 200000}` zeigt der Ring 75 % im Warnstil.
  - [ ] AC2 — Ohne bekanntes Fenster erscheint „150 k Tokens“ ohne Prozentangabe.
  - [ ] AC3 — Die TUI zeigt denselben Wert in der Statuszeile.
- **Abhängigkeiten:** USE-002; SES-011 (siehe 07-sessions-collaboration.md)
- **Referenz:** ADR-0021

### USE-009 — Compaction-Trigger
- **Meilenstein:** M2 · **Priorität:** Should
- **Beschreibung:** Compaction wird manuell über Composer-Button, Slash-Befehl `/compact` oder Command-Palette ausgelöst (API `POST /v1/sessions/{id}/compact`, SES-011 in 07-sessions-collaboration.md; Adapter-Mechanik HAR-022). Automatik: `compaction.auto: harness | beton | off` (Default `harness`: der Harness entscheidet selbst; `beton`: beton löst bei `compaction.threshold`, Default 0,8, die native Compaction aus *(Annahme)*). Bei Harnesses ohne Compaction-Fähigkeit ist der Trigger deaktiviert mit Erklärung.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Klick auf „Kompaktieren“ erzeugt `compaction.started`/`compaction.completed`; die Kontextanzeige aktualisiert sich ohne Reload.
  - [ ] AC2 — Mit `compaction.auto: beton` und Fake-Harness (`compaction: native`) wird bei Überschreiten von 80 % genau einmal kompaktiert, bevor der nächste Turn startet.
  - [ ] AC3 — Bei `compaction: none` ist der Button deaktiviert und zeigt einen Tooltip mit Grund.
- **Abhängigkeiten:** USE-008; HAR-022 (siehe 01-harnesses.md), SES-011 (siehe 07-sessions-collaboration.md)

### VOI — Spracheingabe (lokales Whisper)

### VOI-001 — Transkriptions-Engine `beton-voice`
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Crate `beton-voice` kapselt `whisper-rs` (whisper.cpp) und läuft im lokalen Daemon bzw. zentralen Server. Modelle werden lazy beim ersten Diktat geladen und nach `voice.idle_unload` (Default 10 min) entladen. Gleichzeitige Transkriptionen sind begrenzt (`voice.max_concurrent`, Default 1 lokal, 2 zentral); weitere Anfragen warten mit Status `queued`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine 10-s-WAV-Testdatei (Deutsch, Fixture) wird mit Modell `small` mit Wortfehlerrate ≤ 15 % transkribiert (Referenz-Transkript im Repo).
  - [ ] AC2 — Nach `idle_unload` ist der Modellspeicher freigegeben (RSS sinkt um ≥ 80 % der Modellgröße).
  - [ ] AC3 — Bei `max_concurrent` erreichten Sessions erhält der dritte Client `voice.status {state: "queued", position}`.
- **Abhängigkeiten:** —
- **Referenz:** ADR-0023

### VOI-002 — Modellverwaltung on demand mit Checksum
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Modelle (`small`, `medium`, `large-v3-turbo`, jeweils optional quantisiert) werden erst auf Anforderung (Onboarding, Einstellungen oder erstes Diktat mit Rückfrage) aus einer im Binary gepinnten URL-Liste geladen und gegen gepinnte SHA-256 geprüft. Ablage `~/.beton/models/whisper/`. Default-Modell abhängig von Hardware: `large-v3-turbo` bei Apple Silicon/CUDA, sonst `small` *(Annahme)*. `beton voice models list|pull|import|remove`. Offline-Alternative (ADR-0033): `beton voice models import <datei>` bzw. „Aus Datei importieren“ in den Einstellungen übernimmt eine lokal vorliegende Modelldatei, sofern ihre SHA-256 zu einem Eintrag der gepinnten Liste passt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Download mit falscher Checksumme wird verworfen; die Datei existiert danach nicht im Modellverzeichnis.
  - [ ] AC2 — Abgebrochene Downloads werden per HTTP-Range fortgesetzt.
  - [ ] AC3 — Ohne Modell und ohne Zustimmung findet kein Download statt; der Mikrofon-Button zeigt „Modell laden (≈ X MB)“.
  - [ ] AC4 — `beton voice models list --json` liefert Name, Größe, installiert ja/nein, Checksumme.
  - [ ] AC5 — `beton voice models import <datei>` installiert eine Modelldatei mit passender Prüfsumme ohne Netzwerk; eine Datei ohne passenden Eintrag in der gepinnten Liste wird abgelehnt.
- **Abhängigkeiten:** VOI-001

### VOI-003 — Hardware-Beschleunigung
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Build-Varianten: macOS mit Metal (Apple Silicon), Linux/Windows CPU (AVX2/NEON), optional CUDA-Variante für Linux x64 (DIST-001, siehe 12-distribution-quality.md). Zur Laufzeit wählt beton das beste verfügbare Backend und fällt bei Initialisierungsfehlern auf CPU zurück. Gewähltes Backend und Real-Time-Factor erscheinen in `beton doctor`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Auf Apple Silicon meldet `beton doctor` `voice.backend = metal`.
  - [ ] AC2 — Schlägt die GPU-Initialisierung fehl (simuliert), läuft die Transkription auf CPU und ein Warn-Log wird geschrieben.
  - [ ] AC3 — Benchmark: `large-v3-turbo` auf Apple M-Serie erreicht Real-Time-Factor ≤ 0,3 für die 10-s-Fixture (Nightly-Bench, kein PR-Gate).
- **Abhängigkeiten:** VOI-001

### VOI-004 — Audio-Streaming über WS-Binärkanal
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Clients (Desktop, Web, PWA) nehmen über `getUserMedia` + AudioWorklet auf, resamplen auf 16 kHz mono PCM16 LE und streamen in 100-ms-Frames über einen eigenen Binärkanal der bestehenden WebSocket-Verbindung (Kanal-ID-Multiplexing, PROTO-007 in 06-data-sync-protocol.md). Steuerung per JSON: `voice.start {lang_hint?}`, `voice.stop`, `voice.cancel`; Antworten `voice.partial`, `voice.final`, `voice.error`. Max. Aufnahmedauer 120 s *(Annahme)*. Audio wird nicht persistiert und nicht ins Event-Log geschrieben.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Client sendet die Fixture als Frames; nach `voice.stop` folgt innerhalb von 2 s (Modell `small`, CPU-Referenzmaschine) `voice.final` mit Text.
  - [ ] AC2 — Nach Abschluss existiert weder im Event-Log noch im Blob-Store noch im Dateisystem eine Audiodatei (Test: Verzeichnis-Scan).
  - [ ] AC3 — Nach 120 s wird die Aufnahme serverseitig beendet und `voice.final` mit `truncated: true` gesendet.
  - [ ] AC4 — Nur Clients mit Schreibrecht auf die Ziel-Session (bzw. lokal authentifiziert) dürfen den Kanal öffnen.
- **Abhängigkeiten:** VOI-001; PROTO-007 (siehe 06-data-sync-protocol.md)
- **Referenz:** ADR-0019, ADR-0023

### VOI-005 — Teil-Ergebnisse
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** Während der Aufnahme sendet beton etwa jede Sekunde `voice.partial {text, stable_prefix_len}`, berechnet durch Re-Transkription der letzten ≤ 30 s. Der Composer zeigt den stabilen Präfix normal und den Rest abgeschwächt; `voice.final` ersetzt das Partial vollständig.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Für eine 10-s-Aufnahme treffen mindestens 5 Partials vor `voice.final` ein.
  - [ ] AC2 — `stable_prefix_len` ist monoton nicht fallend über aufeinanderfolgende Partials.
  - [ ] AC3 — Ist die Engine ausgelastet, werden Partials übersprungen, nicht gepuffert (keine wachsende Latenz).
- **Abhängigkeiten:** VOI-004

### VOI-006 — Sprache DE/EN mit Autoerkennung
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Standard ist automatische Erkennung, beschränkt auf Deutsch und Englisch: Whisper-Spracherkennung liefert Wahrscheinlichkeiten, beton wählt die wahrscheinlichste aus `voice.languages` (Default `[de, en]`). Manuelle Festlegung pro User möglich (`voice.language: de|en|auto`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Deutsche und englische Fixture werden ohne Vorgabe korrekt als `de` bzw. `en` erkannt (`voice.final.lang`).
  - [ ] AC2 — Eine französische Fixture wird als `de` oder `en` transkribiert, nie als `fr` (Beschränkung greift).
  - [ ] AC3 — Mit `voice.language: en` wird keine Erkennung durchgeführt.
- **Abhängigkeiten:** VOI-001

### VOI-007 — Push-to-Talk (globaler Shortcut im Desktop)
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Die Desktop-App registriert einen globalen Shortcut (Default `CmdOrCtrl+Shift+Space` *(Annahme)*, konfigurierbar). Modus `hold` (Aufnahme solange gedrückt) oder `toggle`. Das Ergebnis landet im Composer der zuletzt fokussierten Session; optional `voice.auto_send: true`. Ein Tray-/Overlay-Indikator zeigt aktive Aufnahme. In Web/PWA steht ein Mikrofon-Button mit gleichem Verhalten im Composer zur Verfügung (kein globaler Shortcut). VOI-007 ist Owner von Verhalten und Default-Shortcut; Registrierung des globalen Shortcuts und HUD in der Desktop-App beschreibt DESK-008.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Bei unfokussierter App startet der Shortcut die Aufnahme; der Indikator ist sichtbar, solange aufgenommen wird.
  - [ ] AC2 — Ist der Shortcut systemweit belegt, zeigt die App beim Start einen Hinweis mit Link zu den Einstellungen.
  - [ ] AC3 — Ohne Mikrofon-Berechtigung erscheint die OS-Berechtigungsanfrage bzw. eine Anleitung; es wird keine leere Aufnahme gesendet.
- **Abhängigkeiten:** VOI-004; DESK-008, WEB-004 (siehe 08-clients.md)

### VOI-008 — Keine Cloud-Transkription (Garantie)
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Der gesamte Voice-Pfad ist lokal: keine Browser-Web-Speech-API, keine Cloud-STT, keine Netzverbindungen von `beton-voice` außer dem expliziten, vom User ausgelösten Modell-Download von gepinnten URLs (Alternative: Import einer lokalen Datei, VOI-002). Im zentralen Betrieb läuft Whisper auf dem eigenen Server des Betreibers.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Das Frontend referenziert weder `SpeechRecognition` noch `webkitSpeechRecognition` (Lint-Regel in CI).
  - [ ] AC2 — Während einer Transkription baut der Prozess keine ausgehenden Verbindungen auf (Integrationstest mit Netz-Monitoring/Proxy-Assertion).
  - [ ] AC3 — Modell-Downloads erfolgen nur zu URLs aus der eingebetteten, gepinnten Liste.
- **Abhängigkeiten:** VOI-002
- **Referenz:** ADR-0023

### UX — Bedienung

### UX-001 — Inbox
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Zentrale Inbox (Sidebar-Eintrag mit Zähler) für offene Approvals und Fragen (M2), fertige/fehlgeschlagene Async-Agents (M5, ASY-009 in 02-agents.md) und Mentions/Collaboration-Ereignisse (M4, COL-009 in 07-sessions-collaboration.md). Approvals und Fragen sind direkt in der Inbox beantwortbar (Approve/Deny mit optionalem Grund, Antwortfeld) — inklusive pausierter Sessions ohne Zuschauer. Filter nach Typ/Projekt, Tastatursteuerung (`j/k`, `a` approve, `d` deny, `Enter` öffnen). UX-001 ist Owner von Inbox-Projektion, Item-Schema und UI; ASY-009 und COL-009 erzeugen nur Einträge.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein `approval.requested` erscheint innerhalb von 1 s als Inbox-Item; nach `approval.resolved` (egal von welchem Client) ist es `done`.
  - [ ] AC2 — Approve aus der Inbox setzt eine pausierte Session fort, ohne dass die Session geöffnet wurde.
  - [ ] AC3 — Der Zähler entspricht der Anzahl `open`-Items und ist in Tray/Dock-Badge gespiegelt (Desktop).
  - [ ] AC4 — Nach Neuaufbau der Projektion aus dem Event-Log ist der Inbox-Zustand identisch.
- **Abhängigkeiten:** POL-010 (siehe 03-policies.md), DATA-005 (siehe 06-data-sync-protocol.md)
- **Referenz:** ADR-0021; Omnigent Inbox

### UX-002 — Command-Palette (⌘K)
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** `⌘K` / `Ctrl+K` öffnet eine fuzzy-durchsuchbare Palette mit Befehlen (neue Session, Modell wechseln, „Weiter mit <Harness>“ (Fork), Fork, Interrupt, Compact, Theme, Einstellungen, Agent starten), Navigation (Sessions, Projekte, Seiten) und kontextabhängigen Aktionen der aktuellen Session. Befehle stammen aus einer zentralen Registry (`registerCommand({id, title, keywords, shortcut, when, run})`), die auch Desktop-Menü und Shortcuts speist.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eingabe „fork“ in einer Session zeigt „Session forken“ als ersten Treffer; Enter führt den Fork aus.
  - [ ] AC2 — Befehle mit `when`-Bedingung (z.B. Interrupt nur bei laufendem Turn) erscheinen nur, wenn die Bedingung erfüllt ist.
  - [ ] AC3 — Die Palette ist vollständig per Tastatur bedienbar und erfüllt axe-core ohne kritische Verstöße.
- **Abhängigkeiten:** WEB-001 (siehe 08-clients.md)

### UX-003 — Session-Switcher
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Schneller Wechsel zwischen Sessions: `⌘⌥S` / `Ctrl+Alt+S` öffnet eine MRU-Liste (zuletzt genutzt) mit Status (läuft, wartet auf Approval, ungelesen) und Suche; im Desktop zusätzlich `Ctrl+Tab` für MRU-Durchschalten. Der Wechsel lädt die Session aus dem lokalen Cache sofort und synchronisiert ab der letzten `seq`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Wechsel zu einer zuvor geöffneten Session zeigt den letzten Stand in < 150 ms (gemessen im E2E-Test), Live-Events folgen.
  - [ ] AC2 — Sessions mit offenem Approval sind in der Liste markiert.
  - [ ] AC3 — Die MRU-Reihenfolge ist pro Gerät *(Annahme)* und überlebt Neustarts.
- **Abhängigkeiten:** SES-012 (siehe 07-sessions-collaboration.md)

### UX-004 — Tastenkürzel & Shortcuts-Overlay
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Konsistente, plattformabhängige Shortcuts (⌘ auf macOS, Ctrl sonst), definiert in der Befehls-Registry; `⌘/` zeigt ein Overlay aller Shortcuts, gruppiert und durchsuchbar. Defaults: `⌘K` Palette, `⌘N` neue Session, `⌘⌥S` Switcher, `⌘↵` Approve fokussiertes Approval, `⌘⌫` Deny, `Esc Esc` Interrupt, `⌘⇧M` Modell wechseln, `⌘⇧I` Inbox, `⌘,` Einstellungen. User können Shortcuts umbelegen; Konflikte werden angezeigt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `⌘/` öffnet das Overlay; alle in der Registry definierten Shortcuts sind gelistet (Test vergleicht Registry ↔ Overlay).
  - [ ] AC2 — Eine Umbelegung, die mit einem bestehenden Shortcut kollidiert, wird mit Hinweis abgelehnt oder nach Bestätigung getauscht.
  - [ ] AC3 — Shortcuts greifen nicht, solange ein Texteingabefeld Fokus hat, außer explizit als `global_in_input` markierte (z.B. `⌘K`).
- **Abhängigkeiten:** UX-002

### UX-005 — Themes
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** Light und Dark (Default: folgt System) plus drei weitere Themes: „Concrete“ (warmes Betongrau, Markenthema), „Nord“ und „High Contrast“. Themes sind Token-Sets (CSS-Variablen im shadcn/ui-Schema) mit zugeordneten Varianten für Monaco, xterm.js und Shiki, damit Editor, Terminal und Code-Blöcke konsistent sind. Auswahl pro Gerät, optional synchronisiert *(Annahme)*.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Theme-Wechsel wirkt ohne Reload auf UI, Monaco, xterm und Code-Blöcke.
  - [ ] AC2 — Alle Themes erfüllen WCAG-AA-Kontrast für Fließtext; „High Contrast“ erfüllt AAA (automatisierter Kontrasttest über Token-Paare).
  - [ ] AC3 — „System“ folgt einer Änderung der OS-Einstellung live.
- **Abhängigkeiten:** WEB-001, DESK-001 (siehe 08-clients.md)

### UX-006 — Benachrichtigungs-Einstellungen
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Einstellungsseite für Benachrichtigungen als Matrix Ereignistyp × Kanal: Typen `approval_requested`, `question`, `turn_completed`, `async_done` (M5), `mention` (M4), `runner_lost`, `budget_threshold`, `rate_limit_high`; Kanäle In-App/Inbox, Desktop-Notification, Ton, Dock-/Tray-Badge, Web-Push (M4; standardmäßig aus, nur nach ausdrücklicher Aktivierung, WEB-014, ADR-0033). Optional Ruhezeiten. Einstellungen werden pro User serverseitig gespeichert; das Routing selbst ist in COL-010 (ab M4, siehe 07-sessions-collaboration.md) definiert.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Deaktivierter Desktop-Kanal für `turn_completed` unterdrückt die OS-Notification, das Inbox-/In-App-Signal bleibt.
  - [ ] AC2 — Während Ruhezeiten werden nur `approval_requested`-Benachrichtigungen zugestellt, sofern der User dies ausgewählt hat.
  - [ ] AC3 — Einstellungen gelten auf allen Geräten des Users (zweiter Client sieht Änderung nach Reload).
- **Abhängigkeiten:** UX-001; COL-010 (ab M4, siehe 07-sessions-collaboration.md), DESK-005 (siehe 08-clients.md)

### UX-007 — Interne Feature-Flags
- **Meilenstein:** M1 · **Priorität:** Must
- **Beschreibung:** Ein `FeatureFlag`-Enum in `beton-core` mit Status `experimental | beta | stable | removed`. Aktivierung über Config (`features: [voice, browser]`) oder Env `BETON_FEATURES=voice,browser`; im zentralen Betrieb server-autoritativ. Aktive Flags werden in `GET /v1/info` ausgeliefert; die UI blendet nicht aktivierte Funktionen aus. Unbekannte Flags erzeugen eine Warnung, keinen Startabbruch.
- **Details:** Konvention: ADR-0035. Katalog `beton_core::feature::CATALOG`; `stable` ist immer an, `experimental`/`beta` nur nach Aktivierung. `features:` gilt nur aus der User-Konfiguration (das Projekt darf keine Flags setzen); `BETON_FEATURES` ergänzt sie. `GET /v1/info` liefert `features: [<aktive Namen>]`; eine Funktion hinter einem nicht aktiven Flag antwortet `404 feature_disabled`. `beton doctor` meldet unter `features` aktive Flags bzw. als Warnung unbekannte (mit Vorschlag bei Tippfehlern, „Meintest du …?“) und entfernte Flags; dieselben Warnungen schreibt der Daemon beim Start ins Log. Erstes Flag: `fake_harness` (experimentell, HAR-026 AC3), `--dev` aktiviert es ebenfalls.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein `experimental`-Flag ist ohne explizite Aktivierung in API und UI nicht erreichbar (API antwortet 404 `feature_disabled`).
  - [ ] AC2 — `BETON_FEATURES=unknown_flag` startet mit Warnung im Log und in `beton doctor`.
  - [ ] AC3 — Flags mit Status `removed` werden ignoriert und als „entfernt“ gemeldet.
- **Abhängigkeiten:** —
- **Referenz:** ADR-0021; Omnigent `OMNIGENT_FEATURES`

### UX-008 — Onboarding-Wizard (UI-Variante von `beton setup`)
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Beim ersten Start von Desktop/Web führt ein Wizard durch dieselben Schritte wie `beton setup` (gemeinsame Zustandsmaschine im Daemon, API `/v1/setup/*` *(Annahme)*): (1) Modus: lokal oder mit Server verbinden, (2) Harness-Erkennung (`claude`, `codex`, ACP-Agents) mit **Angebot** zur Installation fehlender CLIs — nie still (HAR-016, DIST-015), (3) Login-Status je CLI; Login erfolgt durch die CLI selbst in einem eingebetteten Terminal (`claude auth login`, `codex login`) – das ist der Standardweg, ein API-Key wird nie verlangt (ADR-0034), (4) optionale API-Keys/Gateways als zusätzliche Option (Keychain, SEC-002 in 05-security-identity.md), (5) Sandbox-/Proxy-Check (Ausschnitt aus `beton doctor`), (6) Telemetrie-Consent (OBS-007, Default „Nein“), (7) optional Whisper-Modell, (8) erste Session. Jeder Schritt ist überspringbar; der Wizard ist später unter Einstellungen erneut startbar.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Abbruch nach Schritt 3 und Neustart der App setzt den Wizard bei Schritt 4 fort.
  - [ ] AC2 — Ohne explizite Bestätigung wird keine CLI installiert (E2E-Test prüft, dass kein Installationsprozess startet).
  - [ ] AC3 — beton zeigt im Login-Schritt nur das Terminal der Vendor-CLI; es existiert kein beton-eigener OAuth-Dialog.
  - [ ] AC4 — Der Telemetrie-Schritt hat keine Vorauswahl; „Überspringen“ bedeutet „aus“.
- **Abhängigkeiten:** OBS-005, OBS-007; HAR-016 (siehe 01-harnesses.md), CLI-005 (siehe 08-clients.md), DIST-015 (siehe 12-distribution-quality.md)
- **Referenz:** ADR-0005, ADR-0026

### UX-009 — Automatische Session-Titel (UI)
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** UI-Seite von SES-010 (Owner der Generierung, siehe 07-sessions-collaboration.md): Neue Sessions zeigen „Neue Session“ als Platzhalter; trifft `session.title_changed` ein, wird der Titel ohne Layout-Sprung eingeblendet (Sidebar, Fenstertitel, Tab). Inline-Umbenennen per Doppelklick bzw. `F2`; danach wird nicht mehr automatisch generiert. Einstellung „Titel automatisch erzeugen“ pro User.
- **Details:** Der Kopf zeigt die Herkunft („automatisch“ bzw. „von dir benannt“, aus `title_source` der Session-Liste); generierte Titel blenden ein (ohne Bewegung bei `prefers-reduced-motion`). Fenstertitel: `<Titel> · beton`. Die Einstellung „Titel automatisch erzeugen“ entspricht im lokalen Modus `titles.generator` (aus = `off`) in der User-Konfiguration; der Schalter in der Web-UI folgt als eigenes Issue.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein `session.title_changed {source: generated}` aktualisiert Sidebar und Fenstertitel aller verbundenen Clients innerhalb von 1 s.
  - [ ] AC2 — Nach Inline-Umbenennung zeigt die Session den User-Titel; nachfolgende generierte Titel werden nicht angewendet.
- **Abhängigkeiten:** SES-010 (siehe 07-sessions-collaboration.md)

### UX-010 — MCP-Server-Verwaltung (UI & CLI)
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Oberfläche zur Verwaltung der MCP-Server auf User- und Projekt-Ebene (Agent-Ebene bleibt im Agent-YAML). Owner von Konfigurationsformat, Merge-Regeln und Injektion ist AGT-006 (siehe 02-agents.md), der Transport an die Harnesses ist HAR-009 (siehe 01-harnesses.md). Diese UI zeigt nur die Konfiguration an, bearbeitet sie und testet die Verbindung: Server hinzufügen (stdio-Kommando oder HTTP-URL), Env- bzw. Header-Werte als SecretRef (SEC-001, siehe 05-security-identity.md) statt im Klartext, Server pro Projekt aktivieren und deaktivieren, verfügbare Tools des Servers auflisten. Das CLI-Pendant ist `beton mcp list|add|remove|test` (in M1 schon als CLI verfügbar, die UI folgt mit M3).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein in der UI hinzugefügter stdio-MCP-Server erscheint in `~/.beton/` bzw. `.beton/` in exakt dem Format von AGT-006 und wird bei der nächsten Session an Claude Code und Codex durchgereicht.
  - [ ] AC2 — „Verbindung testen“ startet den Server, führt `tools/list` aus und zeigt die Tool-Namen an. Schlägt das fehl, erscheint die stderr-Ausgabe des Servers (ohne Secret-Werte).
  - [ ] AC3 — Ein als Secret markierter Env-Wert wird als SecretRef gespeichert; Config-Datei, Events und Logs enthalten den Klartext nicht.
  - [ ] AC4 — `beton mcp list` zeigt Server mit Ebene (user/project) und Status; `beton mcp remove <name>` entfernt sie aus der richtigen Ebene.
- **Abhängigkeiten:** AGT-006, HAR-009, SEC-001, WEB-008 (siehe 08-clients.md)
- **Referenz:** ADR-0021

### OBS — Observability & Telemetrie

### OBS-001 — Strukturierte Logs (`tracing`)
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Alle Komponenten loggen über `tracing`: Daemon/Server/Host/Runner als JSON-Lines in `~/.beton/logs/{daemon,host,runner,cli}.log` (zentral zusätzlich stdout für Container), CLI menschenlesbar auf stderr. Pflichtfelder: `ts`, `level`, `target`, `component`, `version`, Span-Kontext mit `session_id`, `runner_id`, `request_id`, `seq` wo vorhanden. Filter über `BETON_LOG` (EnvFilter-Syntax), Rotation täglich, Aufbewahrung 7 Tage bzw. 100 MB. Prompts und Tool-Inhalte werden nie auf `info` oder höher geloggt.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Jede Zeile in `daemon.log` ist valides JSON mit den Pflichtfeldern (Test parst Log eines E2E-Laufs).
  - [ ] AC2 — `BETON_LOG=beton_policy=debug` erhöht nur das Level dieses Targets.
  - [ ] AC3 — Nach Überschreiten von 100 MB bzw. 7 Tagen werden alte Dateien entfernt.
  - [ ] AC4 — Ein E2E-Lauf mit Marker-Prompt enthält den Marker in keinem Log auf Level `info` oder höher.
- **Abhängigkeiten:** —
- **Referenz:** ADR-0025

### OBS-002 — Secret-Redaction
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Secrets werden typseitig als `SecretValue` (SEC-001; Debug/Display → `***`) geführt. Zusätzlich filtert ein Redaction-Layer Logs, Traces, Fehlermeldungen und Diagnose-Bundles: bekannte Muster (Bearer-/Basic-Header, `sk-…`, `ghp_…`/`glpat-…`, JWTs, PEM-Private-Keys) sowie exakte Werte, die `beton-secrets` bekannt sind (Abgleich über Hash-Set). `bt_cred_*`-Platzhalter bleiben sichtbar. Implementierung der Pipeline (Detektoren, exakter Abgleich, Ersetzung `[REDACTED:<kind>]`): Owner SEC-013; OBS-002 bindet sie als `tracing`-Layer ein.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein in der Keychain hinterlegter Test-Key, der in einer Fehlermeldung auftaucht, erscheint im Log als `[REDACTED:<kind>]` (SEC-013).
  - [ ] AC2 — Property-Test: für zufällig generierte Strings mit eingebetteten Mustern enthält die Ausgabe keines der Muster.
  - [ ] AC3 — `format!("{:?}", secret)` liefert `***` (SEC-001) (Compile-/Unit-Test).
- **Abhängigkeiten:** OBS-001; SEC-001, SEC-013 (siehe 05-security-identity.md)
- **Referenz:** ADR-0024

### OBS-003 — OpenTelemetry-Traces (Trace pro Session)
- **Meilenstein:** M2 · **Priorität:** Should
- **Beschreibung:** Export über `tracing-opentelemetry` + OTLP (gRPC/HTTP), aktiviert durch `observability.otlp.endpoint` oder `OTEL_EXPORTER_OTLP_ENDPOINT`; ohne Endpoint kein Export. Jede Session hat eine stabile Trace-ID; Spans: `turn` → `policy.evaluate` (Hook, Entscheidung, Regel-ID) → `model.request` (`gen_ai.request.model`, `gen_ai.usage.*`) → `tool.call` (Name, Status, Dauer) → `approval.wait` → `cost` (USD, Tokens). Inhalte (Prompts, Tool-Args) nur mit `observability.capture_content: true`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Mit einem lokalen OTLP-Collector (Testcontainer) erscheint für eine Fake-Harness-Session ein Trace mit `turn`-, `policy.evaluate`- und `tool.call`-Spans in korrekter Hierarchie.
  - [ ] AC2 — Alle Turns derselben Session teilen die Trace-ID.
  - [ ] AC3 — Ohne `capture_content` enthält kein Span-Attribut den Marker-Prompt.
- **Abhängigkeiten:** OBS-001
- **Referenz:** ADR-0025 (Prompt → Policy-Entscheidungen → Tool-Calls → Kosten)

### OBS-004 — Metriken & Prometheus `/metrics`
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Server und Hosts exponieren Prometheus-Metriken unter `/metrics` (zentral auf eigenem Port/Pfad, per Token oder Netzbindung geschützt) und optional per OTLP. Labels mit niedriger Kardinalität (keine `session_id`/`user_id`).
- **Details:** Vorgeschlagene Metriken:
  | Metrik | Typ | Labels |
  | --- | --- | --- |
  | `beton_sessions_active` | Gauge | `harness` |
  | `beton_sessions_started_total` | Counter | `harness`, `origin` (interactive/async/schedule/api) |
  | `beton_events_appended_total` | Counter | `type_group` |
  | `beton_event_append_seconds` | Histogram | `backend` (sqlite/postgres) |
  | `beton_ws_clients_connected` | Gauge | `client_kind` |
  | `beton_hosts_connected` / `beton_runners_active` | Gauge | `provider` |
  | `beton_runner_provision_seconds` | Histogram | `provider`, `result` |
  | `beton_policy_evaluations_total` | Counter | `hook`, `decision` |
  | `beton_policy_evaluation_seconds` | Histogram | `hook` |
  | `beton_approvals_pending` / `beton_approval_wait_seconds` | Gauge / Histogram | — / `result` |
  | `beton_tool_calls_total` | Counter | `harness`, `status` |
  | `beton_tokens_total` / `beton_cost_usd_total` | Counter | `harness`, `model`, `kind` / `harness`, `model` |
  | `beton_proxy_requests_total` | Counter | `decision`, `credential_injected` |
  | `beton_sandbox_denials_total` | Counter | `os`, `kind` (fs/net/syscall) |
  | `beton_voice_transcriptions_total` / `beton_voice_rtf` | Counter / Histogram | `model`, `backend` |
  | `beton_scheduler_runs_total` | Counter | `result` |
  | `beton_db_query_seconds` | Histogram | `op` |
- **Akzeptanzkriterien:**
  - [ ] AC1 — `/metrics` liefert gültiges Prometheus-Textformat (Parse-Test mit `promtool check metrics`).
  - [ ] AC2 — Nach einer Fake-Session mit einem `deny` steigt `beton_policy_evaluations_total{decision="deny"}` um genau 1.
  - [ ] AC3 — Kein Metrik-Label enthält Session-, User- oder Pfadwerte (Test prüft Label-Werte gegen Allowlist).
  - [ ] AC4 — Ohne Konfiguration ist `/metrics` im zentralen Betrieb nicht öffentlich erreichbar.
- **Abhängigkeiten:** OBS-001
- **Referenz:** ADR-0025

### OBS-005 — `beton doctor`
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Prüft die Umgebung und gibt eine Tabelle (`ok`/`warn`/`fail`, Hinweis zur Behebung) oder `--json` aus; Exit-Code 0 (ok), 1 (Warnungen), 2 (Fehler). `doctor` kontaktiert keine externen Dienste (keine Online-Update-Prüfung); Netzprüfungen laufen nur gegen ausdrücklich konfigurierte Server (ADR-0033). Prüfungen werden mit den Meilensteinen ergänzt:
  - M0: Version/Update-Kanal, Daemon-Status und Port, Config-Validität, Rechte von `~/.beton` und Token-Datei (0600), SQLite-Integrität (`quick_check`), Harness-CLIs (Pfad, Version, unterstützter Bereich), Login-Status über CLI-eigene Statusbefehle, sofern vorhanden.
  - M1: verwaiste Session-Worktrees ohne Session mit Pfad und Größe (Check `worktrees`, `warn`, SES-016).
  - M2: Sandbox-Fähigkeiten (macOS `sandbox-exec`; Linux Landlock-ABI, seccomp, User-Namespaces, optional bubblewrap; Windows Beta), Egress-Proxy-CA, Keychain-Zugriff, Policy-Ladefehler.
  - M3: Chromium/Chrome for Testing, Whisper-Modelle + Backend, Desktop-Updater-Kanal.
  - M4/M5: Server-Erreichbarkeit, TLS, Uhrzeitabweichung, Host-Pairing, Docker/Podman, Kubernetes-Kontext, Plugins (PLG-011, siehe 10-runners-extensibility.md), Telemetrie-Status. OBS-005 ist Owner der Prüfungen, des `--json`-Schemas und der Exit-Codes; CLI-005 ist die CLI-Front.
- **Details:** M0-Prüfungen mit stabilen IDs: `version`, `daemon` (`warn`, wenn keiner läuft), `config`, `home.permissions`, `database` (`PRAGMA quick_check` schreibgeschützt, ohne Migration), je Harness `harness.<id>` (`details`: `path`, `version`, `version_range`, `compatible`, `source`, `auth_status`) und `auth.<id>` (`warn` nur bei `logged_out`). Fehlende CLI → `warn` mit Installationsbefehl, inkompatible Version → `fail`. `--json` liefert `{version, status, checks: [{id, status, message, hint, details?}]}` gemäß `schemas/v1/doctor.schema.json`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Fehlt `claude` im PATH, meldet `doctor` `warn` mit Installationshinweis und Exit-Code 1.
  - [ ] AC2 — `--json` folgt einem veröffentlichten Schema (Snapshot-Test) und enthält pro Check `id`, `status`, `message`, `hint`.
  - [ ] AC3 — (ab M2) Auf Linux ohne Landlock meldet `doctor` `fail` für `sandbox.linux` mit Verweis auf Kernel-Anforderung.
  - [ ] AC4 — `doctor` ändert nichts am System (läuft unter einer Read-only-Testumgebung erfolgreich durch) und läuft ohne Netzwerk (nur Loopback) ohne Verbindungsversuch nach außen durch.
- **Abhängigkeiten:** —
- **Referenz:** ADR-0025

### OBS-006 — `beton diagnose` (secret-freies Bundle)
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Erstellt ein `.tar.gz` mit `doctor --json`, Versions- und Systeminfo, redigierter effektiver Config, den letzten 2 000 Log-Zeilen je Komponente, Schema-Version der DB, Plugin-Liste und Event-Zählern (ohne Inhalte). Vor dem Schreiben wird ein Inhaltsverzeichnis angezeigt; `--anonymize` ersetzt zusätzlich Hostnamen, Usernamen und Pfade. Kein automatischer Upload. Session-Inhalte nur mit `--include-session <id>` nach Bestätigung, ebenfalls redigiert.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Bundle aus einer Testumgebung mit hinterlegten Fake-Secrets enthält keinen dieser Werte (Scan aller Dateien).
  - [ ] AC2 — Ohne `--include-session` enthält das Bundle keine Nachrichten- oder Tool-Inhalte (Marker-Test).
  - [ ] AC3 — Mit `--anonymize` kommen Home-Pfad und Hostname nicht vor.
  - [ ] AC4 — Als `sensitive` markierte Terminal-Kanäle (z.B. CLI-Login, RUN-015 in 10-runners-extensibility.md) sind nie enthalten.
- **Abhängigkeiten:** OBS-002, OBS-005
- **Referenz:** ADR-0025; Omnigent `omnigent diagnose`

### OBS-007 — Opt-in-Produkt-Telemetrie
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Anonyme Nutzungsstatistik, **standardmäßig aus**. Consent wird im Onboarding (UX-008) bzw. bei `beton setup` ohne Vorauswahl erfragt; ohne Antwort bleibt sie aus. Steuerung: `beton telemetry status|enable|disable|show`, Config `telemetry.enabled`, Env `BETON_TELEMETRY=0`; `DO_NOT_TRACK=1` und erkannte CI-Umgebungen (`CI=true`) erzwingen „aus“, unabhängig von der Config. Im zentralen Betrieb entscheidet der Admin für die Server-Instanz.
- **Details:** Übertragene Daten (abschließende Liste): zufällige `installation_id` (UUIDv4, rotierbar via `beton telemetry reset-id`), beton-Version, Kanal, Installationsart, OS, OS-Version (Major), CPU-Architektur, Modus (lokal/zentral), Client-Typ; Tageszähler: gestartete Sessions je Harness-Typ (`claude|codex|acp|direct|plugin`), genutzte Features (Voice, Browser, Fork, Worktree, Policies aktiv ja/nein, Sandbox-Backend), Anzahl Approvals (nur Zahl). **Nie:** Prompts, Antworten, Tool-Argumente, Dateinamen, Pfade, Repo-Namen, Modellnamen von Gateways, Hostnamen, E-Mail, IP-Speicherung beim Empfänger. Versand gebündelt 1× pro 24 h per HTTPS; Endpoint ist offen (Domain, siehe Offene Punkte in 00-overview.md). `beton telemetry show` zeigt das nächste Paket vollständig vor dem Versand.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Frische Installation ohne Consent: innerhalb von 48 h Laufzeit keine Verbindung zum Telemetrie-Endpoint (Netz-Mock-Test).
  - [ ] AC2 — Mit `telemetry.enabled: true` und `DO_NOT_TRACK=1` wird nichts gesendet; `status` nennt `DO_NOT_TRACK` als Grund.
  - [ ] AC3 — Das gesendete JSON validiert gegen ein Schema mit `additionalProperties: false`, das exakt die Datenliste abbildet.
  - [ ] AC4 — `beton telemetry disable` löscht ungesendete Pakete und die `installation_id`.
- **Abhängigkeiten:** UX-008
- **Referenz:** ADR-0025 (Opt-in, nie Inhalte); Gegenbeispiel Omnigent (Default an)

### OBS-008 — Opt-in-Crash-Reports
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** Ein Panic-Hook schreibt Crash-Berichte lokal nach `~/.beton/crashes/` (Panic-Nachricht redigiert, symbolisierter Backtrace, Version, OS/Arch, Komponente). Versand nur, wenn der User Crash-Reports separat aktiviert hat (`crash_reports.enabled`, eigener Consent im Onboarding) oder einen einzelnen Bericht bestätigt; Ziel ist ein konfigurierbarer, Sentry-kompatibler Endpoint *(Annahme)*. Prompts, Env-Variablen, Kommandozeilen-Argumente und Pfade sind nie enthalten.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein provozierter Panic (Test-Build-Flag) erzeugt eine lokale Crash-Datei ohne Env-Variablen und ohne Marker-Prompt.
  - [ ] AC2 — Ohne Consent erfolgt kein Versand; die Desktop-App bietet beim nächsten Start „Bericht ansehen / senden / verwerfen“ an.
  - [ ] AC3 — `DO_NOT_TRACK=1` unterdrückt auch Crash-Report-Versand und -Rückfrage.
- **Abhängigkeiten:** OBS-002, OBS-007
- **Referenz:** ADR-0025

### OBS-009 — Health- & Readiness-Endpunkte
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** `GET /healthz` (Prozess lebt) und `GET /readyz` (DB erreichbar, Migrationen angewendet, Blob-Store erreichbar) für Container-Orchestrierung; `GET /v1/info` mit Version, Protokollversion und aktiven Feature-Flags. Health-Endpunkte sind ohne Auth erreichbar und geben keine internen Details preis.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Bei gestoppter Postgres-Instanz liefert `/readyz` 503, `/healthz` weiterhin 200.
  - [ ] AC2 — Die Antworten enthalten keine Hostnamen, Pfade oder Fehlermeldungen der DB.
  - [ ] AC3 — Helm-Chart und docker-compose nutzen die Endpunkte als Probes (DIST-011, DIST-012 in 12-distribution-quality.md).
- **Abhängigkeiten:** —

## Nicht in v1

- **Smart Routing** („Auto“-Harness/Modell-Wahl), lernender Router — v2 (ADR-0022); subscription-first gemäß ADR-0034.
- **Prompt-Cleanup nach Diktat** (LLM-Nachbearbeitung des Transkripts) — v2 (ADR-0023); dann über eine eingeloggte Vendor-CLI im Einmal-Modus, API-Key nur optional (ADR-0034).
- **Cloud-Transkription** jeglicher Art — dauerhaft ausgeschlossen (ADR-0023).
- **Branding/White-Label** von Themes und Oberfläche — v2.
- **UI-Extensions** (eigene Seiten/Panels von Drittanbietern) — v2 (siehe 10-runners-extensibility.md).
