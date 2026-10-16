# Triage der offenen M2-Issues (WP-50, #173)

Stand: 2026-10-16, 85 offene Issues im Milestone `M2 — Kontrolle`. Die Issues werden **nicht** von diesem PR verschoben; der Maintainer verschiebt sie nach dem Merge laut der Tabelle.

**Ergebnis:** 35 bleiben in M2, 50 wandern nach M3.

**M2 wird nicht geschlossen.** Offen sind die Must-ACs der Escape-Suite (SBX-010 AC3, SBX-011 AC1–AC4, QA-005 AC2 und AC4; `cargo xtask spec-coverage --milestone M2`). Sie sind wie #235 beim Maintainer zurückgestellt (ADR-0040). Dazu kommen die `human-required`-Issues #260 und #301. Die M3-Pakete hängen nicht an M2; das Release 0.1 schon (Hinweis „Kein Release 0.1 vor dem M2-Abschluss“ in WP-65 und WP-68).

## Regeln

- **Bleibt in M2:** ein offenes Must-AC von M2, ein sicherheitsrelevanter Befund vor dem Release (Label `security-review` oder eine konkrete Schwäche: Umgehung, Ausbruch, Leck, fail open), Demo-relevant oder `human-required` für M2. Im Zweifel bleibt ein `security-review`-Befund in M2.
- **Wandert nach M3:** UI-Ansichten ohne UI-AC (Design-first, ADR-0032), Verbesserungen, Nice-to-have und Could-Features, Golden-Aufnahmen und Verifikationen mit echten CLIs (gebündelt in der Subscription-Checkliste QA-019, WP-65), Stabilität von Tests.
- In der Spalte **M3-Paket** steht das Arbeitspaket, in dessen Hinweisen das Issue steht (`scripts/create_milestone_issues.py`). Das Issue bleibt ein eigenes Issue und wird separat geschlossen.

## Bleibt in M2 (35)

| Issue | Thema | Begründung |
|---|---|---|
| #73 | Übernahmen aus M0 | Rest (HAR-005 `PreToolUse`-Hook, `tool_call_gate: full`) ist mit WP-35 umgesetzt; abhaken und schließen |
| #92 | Workspace-API race-frei (TOCTOU) | Sicherheit: seit der Sandbox ist die Workspace-API die relevante Grenze; im Planungs-Issue ausdrücklich genannt |
| #150 | Vendor-CLIs schreiben bzw. lesen außerhalb des Worktrees | Isolation (Sicherheit); mit dem Sandbox-Stand aus WP-45/WP-49 neu bewerten, ggf. schließen |
| #171 | WP-48 Escape-Suite, Fuzzing | offene Must-ACs (SBX-010 AC3, SBX-011, QA-005); beim Maintainer zurückgestellt |
| #173 | WP-50 M2-Abschluss & Planung M3 | dieses Paket; bleibt offen, bis M2 abgeschlossen ist |
| #196 | POL-004 restliche Kontext-Variablen | Variablen mit Default-Werten werten Regeln still falsch aus (nicht fail closed); die `session.*`-Werte vor dem Release füllen oder als Ladefehler melden (`run.*`, `user.roles` folgen mit M4/M5) |
| #200 | SBX-003 Projekt- und Agent-Ebene der Sandbox | umgesetzt in WP-49, Entscheidung jetzt als ADR-0041; nach dem Merge schließen (#324 wandert nach M3) |
| #203 | SBX-004 Hardlinks auf maskierte Dateien (macOS) | konkrete Schwäche; Entscheidung (Prüfung `st_nlink` oder Restrisiko) vor dem Release |
| #211 | SBX-008 Unix-Sockets vor Landlock-ABI 9 | konkrete Schwäche, auch Sockets paralleler Sessions; vor dem Release schließen oder als Restrisiko festhalten |
| #216 | SEC-013 `env.passthrough`-Werte und Daemon-Tokens | Leck-Risiko aus dem Security-Review von WP-38 |
| #223 | Submodule/Gitlinks bringen Host-git zur Code-Ausführung | konkrete Schwäche; beton-eigene git-Aufrufe in Stufe 0 absichern |
| #234 | Policy-Härtung (Major), Sammel-Issue | `security-review`; schließen, wenn #247, #248, #252 erledigt sind |
| #235 | Shell-Parsing umgehbar ohne `parse_uncertain` | `security-review`, Demo-relevant (`git push --force`); beim Maintainer zurückgestellt (Sicherheitsfilter, ADR-0040) |
| #242 | Grant-Fingerprint, `remember_key`, Codex-`workdir`, `sanitize` | `security-review` |
| #243 | Runner-Tokens mit Ablauf, nur SHA-256 | `security-review`, Auth-Code in `beton-server` |
| #244 | SBX-013 Prozessbaum per SIGSTOP einfrieren | Schwäche aus dem Security-Review von #237 (Prozess entkommt dem Beenden), `beton-sandbox` |
| #247 | Hook-Socket und Token-Datei für Tools derselben UID | `security-review` |
| #248 | `glob()`/`startsWith` ohne Symlink-/Case-Auflösung | `security-review` |
| #250 | MCP über den Relay-Hub erzeugt zweites `policy.decision` | verletzt das Must-AC POL-025 AC1 |
| #252 | MCP-Server `beton` aus der Vendor-Konfiguration | `security-review` |
| #258 | User-Hooks in `~/.claude`, Managed Settings, `accept_edits` | `security-review` |
| #260 | Manuelle Nachprüfung der Stufen mit echter CLI | `human-required`, Demo-Verifikation von M2 |
| #263 | Snapshot-Integrität, Shadow-Sichtbarkeit, Trace | `security-review` (Sichtbarkeit für M4 vormerken) |
| #268 | Effect-Keys je Ebene (Projekt-Ebene offen) | `security-review` |
| #270 | SBX-002 Integritätsprüfung je Tool-Call | konkrete Schwäche (globaler Zähler, ADR-0037), `beton-sandbox` |
| #276 | `policy_query`-Limit je Session-Baum | `security-review` |
| #283 | POL-013 Modell-Modifikation beim Start für ACP | Punkt 1 ist fail open (Route bzw. `allowed_models` wirkt bei ACP nicht); Punkte 2–3 (Verbesserungen) bei Bedarf als eigenes Issue nach M3 |
| #286 | Projekt-Routen dürfen nicht verteuern | `security-review` |
| #293 | `git_guard`: cwd und Push-Konfiguration | `security-review`, Demo-nah (Force-Push) |
| #295 | Credentials: Restpunkte aus #291 | `security-review` |
| #297 | Budgets: Restpunkte aus #284 | `security-review`, Demo-nah (Budget-Stopp) |
| #301 | CI-Matrix der Sandbox als Required Checks | `human-required`, Must-AC QA-005 AC3 bzw. SBX-011 AC1 |
| #307 | SEC-013 AC3 Benchmark-Toleranz | Gate eines Must-AC; Entscheidung (Messung im selben Job oder Toleranz) beim Maintainer |
| #329 | ACP `fs/write_text_file` über den Exec-Broker | Restrisiko (TOCTOU) aus dem Security-Review von #327 |
| #330 | Zweites `policy.decision` nach `sandbox-halted` | verletzt das Must-AC POL-025 AC1 |

## Wandert nach M3 (50)

| Issue | Thema | Begründung | M3-Paket |
|---|---|---|---|
| #72 | Nacharbeiten aus M0 (Config, Tunnel-Retry, Idempotency) | Verbesserung | WP-51 |
| #80 | HAR-008 Preset-Flags gemini/goose/qwen | Verifikation mit echten CLIs | WP-65 (QA-019) |
| #81 | PROTO-002 `plan.updated` | Verbesserung; vor dem ersten Schema-Stand | WP-52 |
| #93 | `fs.changed`: User-Schreibzugriffe | Verbesserung, betrifft Terminals | WP-51 |
| #97 | HAR-009 System-Tools mit echten CLIs | Golden-Aufnahme | WP-65 (QA-019) |
| #112 | HAR-022 `/compact` mit echter CLI | Golden-Aufnahme | WP-65 (QA-019) |
| #122 | SES-009 Import fortsetzen im eigenen Verzeichnis | Verbesserung; heute fail closed (nur lesbar), braucht dann `security-review` | WP-61 |
| #123 | RUN-003 AC4 Grace-Test zeitabhängig | Teststabilität | WP-67 |
| #128 | SES-010 Einmal-Modus mit echten CLIs | Golden-Aufnahme | WP-65 (QA-019) |
| #136 | HAR-017/HAR-027 Claude-Golden-Fälle neu | Golden-Aufnahme | WP-65 (QA-019) |
| #143 | HAR-009 Tool-Timeouts der Vendor-CLIs | Verbesserung, echte CLI | WP-65 (QA-019) |
| #147 | Default-Modell je Harness | Verbesserung; zusammen mit Project-Defaults | WP-61 |
| #179 | Preise-Seite ohne UI-AC | UI-Ansicht ohne UI-AC (Rest nach #269) | WP-62 |
| #180 | Gestaffelte Preise im Katalog | Verbesserung | WP-62 |
| #183 | Policy-Übersicht und Ladefehler | UI-Ansicht ohne UI-AC | WP-55 |
| #184 | Secrets und Audit-Log in der UI | UI-Ansicht ohne UI-AC | WP-64 |
| #185 | SEC-012 Retention des Audit-Logs | Verbesserung (*Annahme* der Spec, kein AC) | WP-64 |
| #190 | Proxy-Ansicht und Egress-Log | UI-Ansicht ohne UI-AC | WP-55 |
| #194 | Durchsetzung je Harness, Policy-Audit | UI-Ansicht ohne UI-AC | WP-53 |
| #201 | Sandbox-Einstellungen (`sandbox-settings`) | UI-Ansicht ohne UI-AC; doppelt mit #321, zusammenführen | WP-63 |
| #202 | SBX-001 AC4 `sandbox.started` außerhalb des Runners | AC ist abgedeckt; Rest betrifft Terminals und Audit | WP-51 |
| #212 | SBX-004 Muster-Masken bei jedem Start (Linux) | Performance; doppelt mit #240, zusammenführen | WP-53 |
| #213 | Linux-Probe und Verstoß `limit` in der UI | UI-Ansicht ohne UI-AC | WP-64 |
| #214 | SEC-013 Entropie-Heuristik | Verbesserung (Default aus) | WP-64 |
| #215 | Event-Log-UI zum Schwärzen | UI-Ansicht ohne UI-AC | WP-64 |
| #236 | POL-010 `allow_on_timeout: false` | Spec-Klärung, heute fail-safe; relevant für die Projekt-Ebene | WP-61 |
| #238 | Stufen der Sandbox (`sandbox-stages`) | UI-Ansicht ohne UI-AC | WP-53 |
| #239 | Codex-Tools in Stufe 2 | Verbesserung; heute fail closed (Codex nur mit Opt-out, ADR-0037) | WP-53 |
| #240 | Muster-Masken bei großen Lese-Grants (Linux) | Performance; doppelt mit #212 | WP-53 |
| #241 | Toolchains außerhalb der Systempfade in Stufe 2 | Benutzbarkeit, fail closed; vor dem Release 0.1 nötig | WP-51 |
| #249 | Claude lädt Projekt-`.mcp.json` nicht mehr | Verbesserung, gehört zur MCP-Verwaltung | WP-61 |
| #251 | Flaky `ses_011_ac1_…` | Teststabilität | WP-67 |
| #254 | Explain, Was-wäre-wenn, Policy-Tests in der UI | UI-Ansicht ohne UI-AC | WP-64 |
| #266 | Eigene Preise und Compaction-Automatik in der UI | UI, braucht schreibende Einstellungs-API | WP-62 |
| #267 | Zustände aus Design-PR #262 | UI, wartet auf Design-Abnahme | WP-62 |
| #279 | POL-019 `pii_redaction` | Could; Feature per Spec-Änderung nach M3 verschoben | WP-64 (Feature) |
| #280 | Approval-Card „Agent wiederholt sich“ | UI-Ansicht ohne UI-AC | WP-64 |
| #281 | Budgets und Modell-Routing in der UI | UI-Ansicht ohne UI-AC | WP-64 |
| #282 | POL-012 Zeitzone je Regel | Verbesserung; heute fail closed (Parameter abgelehnt) | WP-64 |
| #290 | Credential-Bindings und Git-Verbindungen | UI-Ansicht ohne UI-AC | WP-64 |
| #292 | Flaky `har_020_ac1_…` | Teststabilität | WP-67 |
| #299 | SBX-014 Verstoß-Reporting | Could; Feature per Spec-Änderung nach M3 verschoben, AC1 braucht ein ADR | WP-64 (Feature) |
| #300 | Sandbox-Probe und `suggest` in der UI | UI-Ansicht ohne UI-AC | WP-64 |
| #320 | `sandbox-unavailable` in der UI | UI-Ansicht ohne UI-AC | WP-63 |
| #321 | `sandbox-settings` (Ziel von „Sandbox einrichten“) | UI-Ansicht ohne UI-AC; doppelt mit #201 | WP-63 |
| #322 | Agent-Detail, Tab „Policies & Sandbox“ | UI-Ansicht ohne UI-AC | WP-61 |
| #323 | Agent-Ebene in `GET /v1/policies?session` | Transparenz, Durchsetzung nicht betroffen | WP-61 |
| #324 | `beton agent validate` prüft Policies und Sandbox | Verbesserung; der Start prüft schon (fail closed) | WP-61 |
| #325 | Fortsetzen nach Proxy-Ausfall | Verbesserung; heute fail closed (`paused`) | WP-63 |
| #326 | ACP `fs/read_text_file` und `terminal/*` in Stufe 2 | Verbesserung; ACP heute `inherited` nur mit Opt-out (ADR-0037) | WP-51 |

## Schon offene Issues im Milestone M3

| Issue | Thema | M3-Paket |
|---|---|---|
| #54 | Signing-Accounts (`human-required`) | blockiert WP-65 |
| #55 | Domain (`human-required`) | blockiert WP-66 |
| #56 | Markenrecht (`human-required`) | blockiert WP-65 |
| #74 | Übernahmen aus M0 | WP-51 (PROTO-007), WP-61 (SES-013), WP-65 (QA-002 AC4, Pflichten aus dem Kommentar), WP-67 (PROTO-004 AC3) |
| #177 | Eigene GitHub-Identität für Agents (`human-required`) | blockiert WP-68 (Release 0.1) |
| #197 | POL-022 `tool_result` bei Claude | WP-53 |
| #217 | UX-001 AC5 Badge der Desktop-App | WP-58 |
| #265 | USE-008 AC3 TUI-Statuszeile | WP-52 |
| #82, #113 | Harness-Katalog, ACP-Registrierung, Provider-Ansicht | WP-63 |
| #91, #96, #119, #121 | Worktree-Ansicht, MCP-Fehler, Import/Export | WP-61 |
| #104 | Design der Workspace-Rail | WP-56 |
| #108, #111, #127, #137 | Codex-Fork, Codex-Compaction, Einmal-Modus ACP, Effort je Modell (echte CLIs) | WP-65 (QA-019) |
| #130 | SSE-Befehl im Event-Log | WP-64 |
| #135, #153 | Hinweise nach Runner-Neustart, automatisch abgelehnte Freigaben | WP-58 |
| #78, #86, #98, #99, #100, #105, #109, #125, #126, #129, #131, #139, #140, #141, #142, #145 | Web-UI-Ansichten aus M1 (Agents, Sessions, Fork, Titel, Queue, Diff, Prototyp-Kit) | WP-62 |

## Spec-Änderungen aus der Triage

- POL-019 und SBX-014 (beide *Could*) stehen jetzt in M3 (`docs/spec/03-policies.md`, `docs/spec/04-sandbox.md`). Damit blockieren sie M2 auch formal nicht und sind in M3 genau einem Paket zugeordnet (WP-64).
