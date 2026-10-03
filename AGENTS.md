# AGENTS.md: Arbeitsanweisungen für Coding-Agents

Dieses Repo wird überwiegend von Coding-Agents gebaut und von einem Menschen (Ingo Fahrentholz) verantwortet. Lies diese Datei vollständig, bevor du etwas änderst.

## 1. Orientierung

1. **Spec zuerst:** [`docs/spec/00-overview.md`](docs/spec/00-overview.md), danach das Kapitel zu deinem Auftrag.
2. **Entscheidungen sind bindend:** [`docs/adr/`](docs/adr/README.md). Widerspricht dein Ansatz einem ADR, **stopp und frag**. Ein ADR wird nur über ein neues ADR geändert.
3. **Aufträge referenzieren Feature-IDs** (z. B. `POL-003`). Umgesetzt ist ein Feature erst, wenn **alle** seine Akzeptanzkriterien durch Tests abgedeckt sind.
4. **Meilenstein beachten:** Baue nichts aus einem späteren Meilenstein oder aus v2 "auf Vorrat" ([roadmap.md](docs/spec/roadmap.md)).

## 2. Repo-Layout

Siehe [00-overview § 4.4](docs/spec/00-overview.md#44-crate--und-repo-layout). Kurz:
- `crates/beton-*`: Rust-Workspace; das Binary `beton` kommt aus `crates/beton-cli`.
- `apps/web`: React-Frontend; `apps/desktop`: Tauri-2-Hülle.
- `packages/sdk-ts`: TypeScript-SDK (Typen werden **generiert**).
- `agents/`: mitgelieferte Agents (YAML). `deploy/`: Compose und Helm.

## 3. Befehle

> Noch nicht vorhanden. Dieser Abschnitt wird mit M0 gefüllt. Zielzustand:

| Zweck | Befehl |
|---|---|
| Rust bauen / testen | `cargo build --workspace` · `cargo nextest run --workspace` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` · `cargo fmt --check` |
| Abhängigkeiten prüfen | `cargo deny check` |
| Typen/Schemas generieren | `cargo xtask codegen` (JSON-Schema, TS-Typen, OpenAPI) |
| Frontend | `pnpm -C apps/web typecheck lint test` |
| E2E | `pnpm -C apps/web e2e` (Playwright gegen Fake-Harness) |

## 4. Regeln

**Immer**
- Akzeptanzkriterien → Tests → Implementierung. In `beton-policy`, `beton-sandbox`, `beton-proxy`, `beton-secrets` und Auth-Code von `beton-server` gilt **strikt TDD**.
- Rust-Typen sind die Single Source of Truth. Nach Änderungen an Protokoll- oder API-Typen `codegen` laufen lassen und die Snapshot-Tests aktualisieren.
- Fehler **fail closed** behandeln: Sandbox, Proxy oder Policy nicht verfügbar → ablehnen, nie stillschweigend erlauben.
- Commits nach Conventional Commits mit Feature-ID und DCO: `git commit -s -m "feat(policy): POL-003 spend_cap rule"`.
- Neue Dependencies: Lizenz muss mit Apache-2.0 kompatibel sein (`cargo deny`).

**Nie**
- Subscription- bzw. OAuth-Tokens von Vendor-CLIs lesen, speichern oder selbst gegen APIs verwenden (ADR-0005).
- Secrets in Logs, Events, Transcripts oder Modell-Kontext schreiben. Agents sehen nur `bt_cred_*`-Platzhalter.
- Telemetrie ohne Opt-in senden (ADR-0025).
- Echte Vendor-CLIs oder Subscriptions in CI-Tests voraussetzen. Stattdessen den **Fake-Harness** und **Golden Transcripts** nutzen.
- Sandbox-Escape-Tests abschwächen oder überspringen, damit die CI grün wird.
- Generierte Dateien (TS-Typen, OpenAPI, JSON-Schemas) von Hand ändern.

## 5. Menschliches Review erforderlich

PRs, die eines der folgenden Themen berühren, brauchen eine explizite Freigabe durch den Maintainer, auch wenn alle Checks grün sind:
- `beton-sandbox`, `beton-proxy`, `beton-secrets`, `beton-policy`, Auth/Sharing in `beton-server`
- Wire-Protokoll-Breaking-Changes (`beton-proto`)
- neue ADRs oder Änderungen an `docs/adr/`
- Release-, Signing- und Update-Pipeline
