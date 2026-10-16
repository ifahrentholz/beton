# ADR-0037: Tool-Isolation der Vendor-CLIs – Claude über `CLAUDE_CODE_SHELL`, Codex nur als bestätigtes, degradiertes Opt-out

- **Status:** Akzeptiert
- **Datum:** 2026-10-16
- **Entscheider:** Coding-Agent (delegiert, AGENTS.md § 5); umgesetzt und security-reviewed in WP-45 (#168, PR #228)

## Kontext
ADR-0007 legt das zweistufige Modell fest: Die Vendor-CLI läuft in Stufe 1 und darf ihre eigenen Credentials lesen, jedes Tool läuft in einer frischen Stufe-2-Sandbox ohne diesen Zugriff. Die Spec nahm an, dass sich das für Claude Code über `CLAUDE_CODE_SHELL_PREFIX` bzw. einen Shell-Override und für Codex über `bt-exec` als `bash`/`sh`-Shim vorn im `PATH` erzwingen lässt *(Annahme)*. WP-45 hat das gegen claude 2.1.296 und codex-cli 0.161.0 geprüft, ohne Dateien der CLIs zu lesen (Doku, `--help`, Verhalten, Logging-Shims, CONNECT-Mitschnitt):

- `CLAUDE_CODE_SHELL_PREFIX` umhüllt nicht nur das Bash-Tool, sondern auch Hooks in Shell-Form und alle MCP-stdio-Server, darunter die Relays von beton. In Stufe 2 erreichen diese ihre Sockets nicht; der `PreToolUse`-Hook und die System-Tools fielen aus.
- `CLAUDE_CODE_SHELL` wählt dagegen genau die Shell für Bash-Tool, Env-Probe und Shell-Snapshot.
- Codex startet Kommandos mit der Login-Shell aus der Benutzerdatenbank (absoluter Pfad, `-lc`). Shims im `PATH` und `SHELL` wirken nicht. `shell_zsh_fork` (gepatchte zsh mit `EXEC_WRAPPER`) und `codex exec-server` sind experimentell.

## Betrachtete Optionen
1. **A — `CLAUDE_CODE_SHELL_PREFIX` für Claude** — ein Mechanismus für alles; bricht Hook und Relays, damit fiele das Policy-Gate aus (nicht fail closed).
2. **B — `CLAUDE_CODE_SHELL` plus Integritätsprüfung** — trifft genau die Tool-Ausführung; Hooks und von Claude selbst gestartete MCP-Server bleiben in Stufe 1.
3. **C — Codex in der Sandbox ganz sperren** — sicher, aber Codex ist dann nur ohne Sandbox nutzbar.
4. **D — Codex mit `tool_isolation: inherited` als ausdrückliches, degradiertes Opt-out** — Codex bleibt nutzbar, Tools laufen aber in Stufe 1 mit Zugriff auf `~/.codex/auth.json`.
5. **E — experimentelle Codex-Wege (`shell_zsh_fork`, `exec-server`) oder Bind-Mount über die Login-Shell (Linux)** — würde Stufe 2 erzwingen; nicht stabil bzw. nur auf einer Plattform.

## Entscheidung
- **Claude Code:** Option **B**. `tool_isolation: shell_override` über `CLAUDE_CODE_SHELL=<shim>/bt-exec-zsh` bzw. `bt-exec-bash`, zusätzlich per `--settings` (`env`); `CLAUDE_CODE_TMPDIR` zeigt auf den Austausch. Der Runner prüft jeden abgeschlossenen `Bash`-Aufruf gegen die Starts des Exec-Brokers; fehlt einer, endet die Session mit `sandbox_integrity` (fail closed). Hooks und von Claude gestartete MCP-Server laufen in Stufe 1 (Restrisiko im Bedrohungsmodell).
- **Codex:** Option **D**. Der Adapter deklariert `tool_isolation: inherited`. Mit dem Default „Sandbox erforderlich“ (SBX-006) startet Codex nicht (`sandbox_unavailable` mit Hinweis). Nur mit `sandbox.require_tool_isolation: false` startet er, markiert als *degraded* (`sandbox.started.degraded`) und ohne `yolo`. Option **E** wird je Codex-Release neu geprüft (#239); bei Erfolg wird der Adapter `shell_override`.
- **ACP:** `inherited`, bis beton `terminal/*` und `fs/*` in Stufe 2 anbietet (#326, #329); dann `beton`.
- **Direkt-API-Harness:** `beton`, Tools immer in Stufe 2.
- Jede Annahme über Vendor-Mechanismen wird pro Vendor-Release gegen die echte CLI geprüft (QA-019, Golden Transcripts HAR-025).

## Konsequenzen
- Positiv: Für Claude ist Stufe 2 erzwungen und überprüft; Hook und System-Tools funktionieren in der Sandbox.
- Negativ / Risiken: Codex ist in der Default-Konfiguration nicht nutzbar, das betrifft das Demo-Szenario von M3 und Release 0.1. Der Integritätszähler ist global: Code, der schon in Stufe 1 läuft, kann ihn erhöhen (#270). Hooks und MCP-Server des Nutzers laufen in Stufe 1.
- Folgearbeiten: #239 (Codex in Stufe 2), #270 (Zuordnung je Tool-Call, Shell-Starts in Stufe 1 sperren), #326/#329 (ACP über beton), manuelle Nachprüfung mit der echten CLI (#260).

## Bezug
- Spec: docs/spec/04-sandbox.md (SBX-002, Stufenmodell), docs/spec/01-harnesses.md (HAR-002, HAR-005, HAR-006, HAR-007)
- ADR-0005, ADR-0007
- Issues: #168, #239, #260, #270, #326, #329
