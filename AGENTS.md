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

| Zweck | Befehl |
|---|---|
| Bauen | `cargo build --workspace` |
| Tests (CI-Standard) | `cargo nextest run --workspace` und `cargo test --workspace --doc` |
| Tests ohne nextest | `cargo test --workspace` |
| Format / Lint | `cargo fmt --all --check` · `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| Abhängigkeiten prüfen | `cargo deny check` (Lizenzen, Advisories, Bans, Quellen; Konfiguration in `deny.toml`) |
| Spec-Konsistenz | `cargo xtask spec-check` |
| Spec-Abdeckung | `cargo xtask spec-coverage --milestone M0` (Exit 1, wenn Must-ACs ohne Test sind; `--report` nur zur Anzeige) |
| Commit-Lint / DCO | `cargo xtask commit-lint --range origin/main..HEAD` · `cargo xtask dco --range origin/main..HEAD` |
| Roadmap neu erzeugen | `python3 scripts/gen_roadmap.py` (CI prüft, dass `docs/spec/roadmap.md` aktuell ist) |
| Typen/Schemas generieren | `cargo xtask codegen` (JSON-Schema nach `schemas/v1/`, TypeScript nach `packages/sdk-ts/src/gen/`); `--check` prüft, ob alles aktuell ist |
| Frontend | `pnpm test` · `pnpm lint` · `pnpm typecheck` in `apps/web` *(folgt mit WP-14)* |
| E2E | `pnpm e2e` in `apps/web` (Playwright gegen Fake-Harness) *(folgt mit WP-15)* |

`cargo-nextest` und `cargo-deny` sind optionale lokale Werkzeuge (`cargo install cargo-nextest cargo-deny`); die CI installiert sie selbst.

## 3a. Teststrategie (QA-001)

| Ebene | Ort | Zweck |
|---|---|---|
| Unit | im Crate, `#[cfg(test)]` | schnell, ohne IO |
| Property / Fuzz | `proptest` im Crate, `cargo-fuzz` unter `fuzz/` | Parser, CEL, Proxy-Regeln |
| Contract | gemeinsame Trait-Suiten | Harness-Adapter, RunnerProvider, Git-Provider, Plugins |
| Golden | Adapter gegen aufgezeichnete Transcripts | Drift der Vendor-CLIs erkennen (HAR-025) |
| Integration | `crates/*/tests/` | echter Daemon in-process mit SQLite und Fake-Harness |
| Sicherheit | eigene Suiten | Sandbox-Escape (QA-005), Policy-Tests (QA-004) |
| E2E | `apps/web` (Playwright), Desktop (WebDriver) | Nutzerflüsse, Demo-Szenarien |
| Benchmarks | `benches/` | Durchsatz, Startzeit, Speicher |

Zielverhältnis grob 70 % Unit/Property, 20 % Contract/Golden/Integration, 10 % E2E. **Kein Test darf echte Vendor-Accounts, Subscriptions oder das Internet brauchen**; in der Linux-CI laufen die Tests ohne Netzwerk (nur Loopback).

**Namenskonvention:** Tests, die ein Akzeptanzkriterium abdecken, heißen `<prefix>_<nnn>_ac<k>_<beschreibung>`, z. B. `pol_003_ac2_stricter_rule_wins`. In TypeScript-Tests und CI-Workflows gilt die Schreibweise `POL-003 AC2` (im Testtitel bzw. als Kommentar `# covers: QA-010 AC1`). `cargo xtask spec-coverage` wertet beides aus.

## 3b. Logging (OBS-001)

Logs gehen über `tracing` (Modul `beton_cli::logging`). Prompts, Tool-Inhalte und Nutzerdaten werden **nie auf `info` oder höher** geloggt, Secrets nie. Kontextfelder wie `session_id`, `runner_id`, `request_id` und `seq` gehören an den Span, nicht in die Nachricht.

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

## 5. Entscheidungen und menschliches Review

Der Maintainer hat die Entscheidungen delegiert: Agents treffen Architektur- und Umsetzungsentscheidungen selbst und halten grundsätzliche Entscheidungen als ADR fest (`docs/adr/`). Verbindlich bleiben die Prämissen des Maintainers (ADR-0032 Design-first, ADR-0033 lokal ohne externe Server, ADR-0034 Subscription-first). PRs ohne Label `security-review` dürfen nach grüner CI gemergt werden.

Dem Maintainer vorgelegt werden nur:
- **Security-Review vor dem Merge** (Label `security-review`): `beton-sandbox`, `beton-proxy`, `beton-secrets`, `beton-policy`, Auth/Sharing in `beton-server`, Release-, Signing- und Update-Pipeline.
- **Designs zur Abnahme** (Design-Prototyp, ADR-0032).
- **Recht, Geld und Accounts** (Label `human-required`), z. B. Nutzungsbedingungen, Signing-Zertifikate, kostenpflichtige Dienste.
- **Unumkehrbares**, z. B. das Repo öffentlich machen, Releases veröffentlichen, Daten löschen.

## 6. Arbeitsablauf mit GitHub-Issues

1. **Arbeitspaket wählen:** offene Issues im aktuellen Milestone mit Label `type:work-package`, deren „blocked by“-Issues alle geschlossen sind. Issues mit `human-required` nicht bearbeiten.
2. **Branch:** `wp-NN-kurzname` (z. B. `wp-03-persistence`) von `main`.
3. **Umsetzen:** pro Feature-ID im Issue alle Akzeptanzkriterien testen (Namenskonvention siehe §3a) und implementieren; Checklisten im Issue beim Erledigen abhaken.
4. **Commits:** `<type>(<scope>): <ID> <Beschreibung>` mit `git commit -s`; `feat` braucht eine Feature-ID. Optional im Body: `Implements: POL-003 AC1–AC3`.
5. **PR:** Titel im selben Format, Body nach `.github/pull_request_template.md` mit `Closes #<Issue>`. Label `security-review` ⇒ Merge erst nach expliziter Freigabe durch den Maintainer.
6. **Abweichungen:** Widerspricht die Umsetzung der Spec oder einem ADR, Spec-Änderung bzw. neues ADR im selben PR und im PR-Text hervorheben. Lässt sich ein AC in diesem Meilenstein nicht erfüllen, nicht stillschweigend weglassen, sondern als Issue anlegen.

**Vorgabe des Maintainers: Der Arbeitsstand ist vollständig in GitHub sichtbar.** Sobald ein Meilenstein geplant ist, existiert jede seiner Aufgaben als Issue im passenden Milestone: jedes Feature in genau einem Arbeitspaket, dazu alle beim Umsetzen entstehenden Nacharbeiten, Spec-Korrekturen, Bugs und Folgeaufgaben. Erwähnungen nur im PR-Text reichen nicht. Neue Meilensteine werden mit `scripts/create_milestone_issues.py` angelegt (rollierend, jeweils im Planungs-Issue des Vormeilensteins).

