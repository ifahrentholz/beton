# 12 — Distribution & Qualität

Dieses Kapitel legt fest, wie beton gebaut, signiert, ausgeliefert, aktualisiert und entfernt wird (**DIST**) und wie seine Qualität abgesichert wird — Teststrategie, Test-Infrastruktur, CI-Gates und Arbeitsweise mit Coding-Agents (**QA**).

Bezug: ADR-0026 (Distribution, Signing, Updates), ADR-0031 (Qualitätsstrategie), ADR-0027 (Apache-2.0 + DCO), ADR-0030 (erstes öffentliches Release 0.1 nach M3, v1.0 nach M5). Adapter-spezifische Golden-Tests und der In-Process-Fake-Harness sind in HAR-025 bzw. HAR-026 (siehe 01-harnesses.md) spezifiziert; hier stehen die querschnittlichen Regeln, Werkzeuge und Gates. Offener Punkt aus 00-overview.md: Wer die Signing-Accounts (Apple Developer, Azure Trusted Signing) hält, ist vor M3 zu klären.

## Konzepte & Begriffe

| Begriff | Bedeutung |
| --- | --- |
| **Target** | Rust-Target-Triple, für das ein Artefakt gebaut wird. |
| **Installationsart** | Wie beton installiert wurde (`script`, `homebrew`, `winget`, `scoop`, `cargo`, `deb`, `rpm`, `desktop`, `container`); bestimmt das Update-Verhalten. |
| **Install-Receipt** | `~/.beton/install.json` mit Installationsart, Version, Kanal, Pfad; geschrieben vom Installer. |
| **Kanal** | `stable`, `beta`, `nightly` — Update-Strom für CLI und Desktop. |
| **Kompatibilitätsfenster** | Bereich, in dem verschiedene Versionen von Client, Server, Host/Runner miteinander sprechen dürfen. |
| **Golden Transcript** | Aufgezeichnete echte Vendor-CLI-Ausgabe plus erwartete normalisierte Events (HAR-025, siehe 01-harnesses.md). |
| **Protokoll-Fake-CLI** | Test-Binary, das das Wire-Protokoll einer Vendor-CLI (stream-json, app-server, ACP) deterministisch simuliert. |
| **Escape-Probe** | Test-Programm, das in der Sandbox einen verbotenen Zugriff versucht; der Test besteht nur, wenn der Zugriff scheitert. |
| **Sicherheitskritische Crates** | `beton-policy`, `beton-sandbox`, `beton-proxy`, `beton-secrets` und das Auth-Modul in `beton-server`. |
| **Feature-ID** | Stabile ID aus dieser Spec (`POL-003`), referenziert in Commits, PRs und Tests. |

## Design

### Build-Matrix

| Artefakt | Targets | Hinweise |
| --- | --- | --- |
| CLI/Server-Binary `beton` | `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc` | Linux statisch (musl); Whisper CPU-only auf Linux/Windows, Metal auf macOS |
| CLI CUDA-Variante | `x86_64-unknown-linux-gnu` | optionales Artefakt `beton-cuda` *(Annahme)* |
| Desktop | macOS Universal (arm64+x64), `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` (Should), `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc` (Should) | Linux glibc wegen WebKitGTK |
| Container | `linux/amd64`, `linux/arm64` | `beton-server`, `beton-runner` |

CI-Runner: GitHub-hosted `macos-14` (arm64; x64 per Cross-Compile), `ubuntu-24.04`, `ubuntu-24.04-arm`, `windows-2025`, `windows-11-arm`.

### Release-Pipeline (Tag `vX.Y.Z`)

```
release-please PR gemergt → Tag
  → build matrix (cargo build --release --locked, Tauri bundle)
  → sign: macOS codesign + notarytool + staple | Windows Azure Trusted Signing | cosign (keyless, GitHub OIDC)
  → SBOM (CycloneDX: cargo-cyclonedx; Images: syft) + SLSA-Provenance (GitHub Attestations)
  → GitHub Release (Draft) mit SHA256SUMS, *.sigstore.json, SBOMs
  → Smoke-Tests: Installer auf macOS/Linux/Windows, `beton --version`, `beton doctor --json`
  → Release veröffentlichen → Container push (ghcr) → Tauri latest.json je Kanal
  → PRs/Commits an homebrew-tap, scoop-bucket, winget-pkgs
```

### Versionierung & Kompatibilität

- Produkt: **SemVer** `MAJOR.MINOR.PATCH`, eine Version für den gesamten Workspace (alle `beton-*`-Crates, Desktop, Container, `@ifahrentholz/beton-sdk`). Vor 1.0 dürfen Minor-Versionen brechen; ab 1.0 nur Major.
- Wire-Protokoll: `v1` (PROTO-004, siehe 06-data-sync-protocol.md). Client ↔ Server und Host/Runner ↔ Server sind kompatibel, wenn die Minor-Differenz ≤ 1 ist (neuer Client gegen Server bis eine Minor älter und umgekehrt).
- Plugin-Protokoll: eigene Major-Version `plugin_api` (PLG-010, siehe 10-runners-extensibility.md).
- Datenbank: nur Vorwärts-Migrationen; vor Migration automatisches Backup (SQLite-Datei-Kopie) bzw. Hinweis (Postgres).

## Features

### DIST — Distribution

### DIST-001 — Build-Matrix & reproduzierbare Builds
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Release-Artefakte werden ausschließlich in CI für die Targets der Build-Matrix gebaut (`--locked`, gepinnte Toolchain in `rust-toolchain.toml`, gepinnte Frontend-Lockfile). Windows-Targets, Linux-arm64-Desktop und CUDA-Variante werden bis M5 vervollständigt. Ein PR-Workflow baut die Matrix im Check-Modus (`cargo check`) mit, damit Plattformbrüche früh auffallen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Release-Lauf erzeugt für jedes Matrix-Target ein Artefakt; fehlt eines, schlägt der Release fehl.
  - [ ] AC2 — `beton --version` gibt Version, Git-Commit, Build-Datum, Target und Kanal aus.
  - [ ] AC3 — Die Linux-musl-Binaries sind statisch gelinkt (`ldd` meldet „not a dynamic executable“).
  - [ ] AC4 — Zwei Builds desselben Commits auf Linux erzeugen identische SHA-256 *(Should-Ziel; Abweichungen werden als Warnung berichtet)*.
- **Abhängigkeiten:** —
- **Referenz:** ADR-0026

### DIST-002 — GitHub Releases & Artefakt-Layout
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Jedes Release auf `github.com/ifahrentholz/beton/releases` enthält pro Target `beton-<version>-<target>.tar.gz` (Windows `.zip`), Desktop-Bundles, `SHA256SUMS`, Sigstore-Bundles (`*.sigstore.json`), SBOMs und Installer-Skripte. Die URL `…/releases/latest/download/<name>` ist stabil nutzbar (versionsloser Alias `beton-<target>.tar.gz`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `sha256sum -c SHA256SUMS` validiert alle Artefakte eines Releases.
  - [ ] AC2 — `cosign verify-blob --bundle <f>.sigstore.json --certificate-identity-regexp '^https://github.com/ifahrentholz/beton/' --certificate-oidc-issuer https://token.actions.githubusercontent.com <f>` gelingt für jedes Artefakt.
  - [ ] AC3 — Der versionslose Alias zeigt nach Veröffentlichung auf das neueste `stable`-Artefakt.
- **Abhängigkeiten:** DIST-001, DIST-013

### DIST-003 — Installer-Skripte (curl|sh, PowerShell)
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** `curl -fsSL https://github.com/ifahrentholz/beton/releases/latest/download/install.sh | sh` bzw. `irm https://github.com/ifahrentholz/beton/releases/latest/download/install.ps1 | iex`. Die Skripte erkennen OS/Arch, laden das passende Archiv, prüfen SHA-256 (und cosign, falls installiert), installieren nach `~/.beton/bin` (Windows `%LOCALAPPDATA%\beton\bin`), ergänzen PATH nach Rückfrage bzw. mit `--no-modify-path`, und schreiben das Install-Receipt. Optionen `--version`, `--channel`, `--prefix`, `--yes`. Keine Root-Rechte erforderlich.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Auf frischen CI-VMs (macOS, Ubuntu, Windows) installiert das Skript beton; `beton --version` läuft in einer neuen Shell.
  - [ ] AC2 — Eine manipulierte Archiv-Checksumme bricht die Installation mit Exit-Code ≠ 0 ab, ohne Dateien zu hinterlassen.
  - [ ] AC3 — Das Install-Receipt enthält `method: "script"`, Version, Kanal und Binary-Pfad.
  - [ ] AC4 — `install.sh` ist POSIX-sh-kompatibel (`shellcheck -s sh` ohne Fehler).
- **Abhängigkeiten:** DIST-002

### DIST-004 — Homebrew-Tap
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Tap `ifahrentholz/homebrew-tap` mit Formel `beton` (vorgebaute Binaries für macOS/Linux) und Cask `beton-desktop`. Der Release-Workflow aktualisiert Formel und Cask automatisch (Version, URLs, SHA-256).
- **Akzeptanzkriterien:**
  - [ ] AC1 — `brew install ifahrentholz/tap/beton` installiert die aktuelle Version; `brew test beton` führt `beton --version` aus.
  - [ ] AC2 — `brew install --cask ifahrentholz/tap/beton-desktop` installiert die signierte, notarisierte App ohne Gatekeeper-Warnung.
  - [ ] AC3 — Nach einem Release ist die Formel innerhalb der Pipeline aktualisiert (CI prüft `brew audit --strict`).
- **Abhängigkeiten:** DIST-002, DIST-008, DIST-013

### DIST-005 — `cargo binstall`
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** Das Binary-Crate `beton-cli` enthält `[package.metadata.binstall]`, das auf die GitHub-Release-Artefakte zeigt; `cargo binstall beton-cli` installiert das Binary `beton` ohne Kompilierung. `cargo install beton-cli` (Source-Build) bleibt möglich.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `cargo binstall beton-cli --no-confirm` lädt das vorgebaute Artefakt (Log zeigt keinen Compile-Schritt).
  - [ ] AC2 — `beton upgrade` erkennt diese Installation als `cargo` (DIST-016).
- **Abhängigkeiten:** DIST-002

### DIST-006 — winget & Scoop
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Windows-Pakete: winget-Manifest `ifahrentholz.beton` (CLI, portable) und `ifahrentholz.beton.Desktop` (MSI) sowie Scoop-Bucket `ifahrentholz/scoop-bucket`. Der Release-Workflow erzeugt Manifeste und öffnet PRs gegen `microsoft/winget-pkgs` bzw. committet in den Bucket.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `winget install ifahrentholz.beton` und `scoop install beton` (nach `scoop bucket add`) installieren eine lauffähige CLI auf `windows-2025`.
  - [ ] AC2 — Manifeste bestehen `winget validate` bzw. Scoops `checkver`/`autoupdate`-Prüfung in CI.
- **Abhängigkeiten:** DIST-002, DIST-013

### DIST-007 — deb- & rpm-Pakete
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** `.deb` und `.rpm` für CLI (amd64/arm64) als Release-Assets, inklusive systemd-User-Units für `beton serve` und `beton host` (nicht automatisch aktiviert), Shell-Completions und Manpage. Eigene apt/yum-Repositories sind *Could* (später); AUR/Nix sind nicht v1.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `apt install ./beton_<v>_amd64.deb` auf Ubuntu 24.04 und `dnf install ./beton-<v>.x86_64.rpm` auf Fedora installieren `/usr/bin/beton`; `lintian`/`rpmlint` ohne Fehler.
  - [ ] AC2 — Nach Installation sind die systemd-User-Units vorhanden, aber nicht aktiviert.
  - [ ] AC3 — Deinstallation über den Paketmanager entfernt Binary und Units, nicht aber `~/.beton`.
- **Abhängigkeiten:** DIST-001

### DIST-008 — Desktop-Bundles
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Tauri-Bundles: macOS `.dmg` (Universal), Linux `.AppImage`, `.deb`, `.rpm`; Windows `.msi` und NSIS-`.exe` (Windows Beta, bis M5 vollständig). Die App bündelt den Rust-Core und registriert den Deep-Link `beton://`. Flatpak ist v2.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Die `.dmg` startet auf macOS arm64 und x64 ohne Gatekeeper-Warnung (notarisiert, gestapelt).
  - [ ] AC2 — `beton://local/s/<id>` (Format DESK-006) öffnet nach Installation die App auf allen drei Plattformen (E2E-Smoke).
  - [ ] AC3 — Das AppImage startet auf Ubuntu 22.04 und 24.04 ohne zusätzliche Pakete außer WebKitGTK-Laufzeit.
- **Abhängigkeiten:** DIST-001, DIST-013; DESK-001, DESK-006 (siehe 08-clients.md)

### DIST-009 — Container-Image `beton-server`
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** `ghcr.io/ifahrentholz/beton-server:<version>` (multi-arch, distroless/`static`-Basis), enthält nur das `beton`-Binary und die Web-UI-Assets; kein Harness, keine CLIs. Läuft als Non-root, Konfiguration über Env/Datei, Volumes für SQLite/Blob-Store optional. Tags: `<version>`, `<major>.<minor>`, `latest` (stable), `beta`, `nightly`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `docker run ghcr.io/ifahrentholz/beton-server:<v>` startet und `/readyz` (OBS-009, siehe 11-platform-features.md) liefert 200 mit SQLite-Default.
  - [ ] AC2 — Image ist cosign-signiert, hat SBOM- und SLSA-Attestierung (`cosign verify-attestation`).
  - [ ] AC3 — Trivy-Scan ohne `CRITICAL`/`HIGH` mit verfügbarem Fix (CI-Gate für Release).
- **Abhängigkeiten:** DIST-013

### DIST-010 — Container-Image `beton-runner`
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** Build und Veröffentlichung des Runner-Images (Inhalt: Owner RUN-014 in 10-runners-extensibility.md) in derselben Pipeline wie `beton-server`, multi-arch, gleiche Tags, Signatur und SBOM. Zusätzlich wöchentlicher Rebuild der aktuellen Version für Sicherheitsupdates der Basis und gepinnten CLIs (Tag-Suffix `-r<n>` *(Annahme)*).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Jedes Release enthält `ghcr.io/ifahrentholz/beton-runner:<v>` für amd64 und arm64.
  - [ ] AC2 — Der wöchentliche Rebuild ändert die beton-Version nicht und ist separat signiert.
- **Abhängigkeiten:** DIST-009; RUN-014 (siehe 10-runners-extensibility.md)

### DIST-011 — docker-compose
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** `deploy/docker-compose/` mit `compose.yaml` für den zentralen Server: `beton-server`, Postgres, optional MinIO (S3-Blob-Store) und ein Reverse-Proxy-Beispiel (Caddy) mit TLS. `.env.example` dokumentiert OIDC-, DB- und Master-Key-Variablen (SEC-005, siehe 05-security-identity.md). Healthchecks nutzen `/healthz` und `/readyz`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `docker compose up -d` mit `.env.example`-Werten führt zu einem erreichbaren Server; ein Client kann sich per Device-Pairing verbinden (CI-E2E mit Fake-OIDC).
  - [ ] AC2 — Neustart des Stacks verliert keine Sessions (Postgres-Volume).
- **Abhängigkeiten:** DIST-009, OBS-009 (siehe 11-platform-features.md)

### DIST-012 — Helm-Chart
- **Meilenstein:** M5 · **Priorität:** Must
- **Beschreibung:** `deploy/helm/beton` mit Server-Deployment (Probes, HPA optional, Ingress, Secrets-Referenzen), optional Postgres-Abhängigkeit, und optionalem `beton host`-Deployment mit Kubernetes-RunnerProvider inkl. namespace-beschränkter RBAC und optionaler Egress-NetworkPolicy für Runner-Pods (RUN-012, siehe 10-runners-extensibility.md). Veröffentlichung als OCI-Chart `oci://ghcr.io/ifahrentholz/charts/beton`, cosign-signiert.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `helm install` in einem kind-Cluster (CI) bringt Server und Host auf `Ready`; eine Fake-Harness-Session läuft als Runner-Pod.
  - [ ] AC2 — `helm lint` und `kubeconform` laufen ohne Fehler.
  - [ ] AC3 — Mit `host.enabled=false` werden keine RBAC-Objekte für Runner erzeugt.
- **Abhängigkeiten:** DIST-009, DIST-010, RUN-012 (siehe 10-runners-extensibility.md)

### DIST-013 — Signing: macOS, Windows, Sigstore, SBOM, SLSA
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** macOS: Developer-ID-Signatur mit Hardened Runtime für App **und** CLI-Binary, Notarisierung per `notarytool`, Stapling für `.dmg`/`.app`. Windows (bis M5): Azure Trusted Signing für `.exe`, `.msi`, NSIS und CLI. Linux/Container/alle Artefakte: cosign keyless (GitHub OIDC), Sigstore-Bundles; SBOM (CycloneDX) für Binaries und Images; SLSA-Build-Provenance über GitHub Artifact Attestations. Signing-Secrets nur in einer geschützten GitHub-Environment `release` mit manueller Freigabe.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `spctl --assess --type execute` akzeptiert App und CLI; `stapler validate` gelingt für die `.dmg`.
  - [ ] AC2 — `Get-AuthenticodeSignature` meldet `Valid` für alle Windows-Artefakte (ab M5).
  - [ ] AC3 — `gh attestation verify <artefakt> --repo ifahrentholz/beton` gelingt für jedes Release-Artefakt.
  - [ ] AC4 — Ein Workflow außerhalb der `release`-Environment hat keinen Zugriff auf Signing-Secrets (Konfigurationstest via GitHub-API).
- **Abhängigkeiten:** DIST-001
- **Referenz:** ADR-0026; offener Punkt Signing-Accounts

### DIST-014 — Desktop-Auto-Update (Tauri-Updater)
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Die Desktop-App nutzt den Tauri-Updater mit signierten Update-Manifesten (Ed25519-Schlüssel getrennt von Code-Signing) je Kanal (`latest-stable.json`, `latest-beta.json`, `latest-nightly.json` als Release-Assets). Prüfung beim Start und alle 6 h; Installation erst nach Bestätigung oder beim nächsten Neustart; laufende Sessions werden nicht unterbrochen (Daemon-Neustart nach Drain). DIST-014 ist Owner der Update-Implementierung; das App-Verhalten beschreibt DESK-007.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine App auf Version N erkennt N+1 im eigenen Kanal und installiert sie nach Bestätigung; danach meldet sie N+1.
  - [ ] AC2 — Ein Manifest mit ungültiger Signatur wird verworfen und geloggt; keine Installation.
  - [ ] AC3 — Mit laufendem Turn wird der Daemon-Neustart verschoben, bis alle Sessions idle sind oder der User „jetzt neu starten“ wählt.
- **Abhängigkeiten:** DIST-008, DIST-017

### DIST-015 — Harness-CLI-Installationsangebot
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** `beton setup` (und der UI-Wizard, UX-008 in 11-platform-features.md) erkennt fehlende `claude`/`codex`-CLIs und **bietet** die offizielle Installationsmethode an (z.B. npm-Paket des Herstellers oder Hersteller-Installer), zeigt den exakten Befehl und führt ihn erst nach expliziter Zustimmung aus. Nie still, nie mit Root ohne Rückfrage. Anschließend Versionsprüfung gegen den unterstützten Bereich (HAR-002/HAR-003, siehe 01-harnesses.md). Erkennung und Installationsangebot an sich sind Owner HAR-016 (M0); DIST-015 ergänzt die gepflegten Installationsbefehle je Plattform, `--install-clis` und die Wizard-Anbindung.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ohne Zustimmung (`n` oder nicht-interaktiv ohne `--install-clis`) wird kein Installationsprozess gestartet.
  - [ ] AC2 — Der angezeigte Befehl entspricht exakt dem ausgeführten (Test vergleicht Log).
  - [ ] AC3 — Schlägt die Installation fehl, endet `setup` nicht mit Fehler, sondern markiert den Harness als „nicht verfügbar“ mit Hinweis.
- **Abhängigkeiten:** HAR-016 (siehe 01-harnesses.md), CLI-005 (siehe 08-clients.md)
- **Referenz:** ADR-0026

### DIST-016 — `beton upgrade` mit Installationsart-Erkennung
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** `beton upgrade [--check] [--channel stable|beta|nightly] [--version X] [--dry-run] [--force]` bestimmt die Installationsart (Receipt → Pfad-Heuristik: Homebrew-Cellar, `~/.cargo/bin`, Scoop-/winget-Pfade, `dpkg -S`/`rpm -qf`, Desktop-Bundle, Container-Marker). Bei `script` aktualisiert beton sich selbst (Download, Checksumme + Signatur prüfen, atomarer Austausch, vorherige Version als `beton.old`). Bei Paketmanagern wird nur der passende Befehl angezeigt (z.B. `brew upgrade beton`), im Container ein Hinweis auf das neue Image. Vor dem Austausch werden laufende Sessions gedraint (Daemon-Neustart nach Idle, `--force` überspringt). DIST-016 ist Owner der Implementierung; CLI-Oberfläche: CLI-013.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Script-Installation auf Version N wird mit `beton upgrade` auf N+1 aktualisiert; `beton.old` ist N.
  - [ ] AC2 — Bei Homebrew-Installation verändert `beton upgrade` keine Dateien und gibt `brew upgrade beton` aus (Exit-Code 0).
  - [ ] AC3 — Ein Download mit falscher Signatur bricht ab; die installierte Version bleibt unverändert.
  - [ ] AC4 — `--check` gibt maschinenlesbar (mit `--json`) aktuelle/neueste Version und Installationsart aus.
- **Abhängigkeiten:** DIST-003, DIST-017
- **Referenz:** Omnigent `omni upgrade`

### DIST-017 — Release-Kanäle stable/beta/nightly
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** `stable` = getaggte Releases; `beta` = Pre-Releases `X.Y.Z-beta.N`; `nightly` = täglicher Build von `main` als `X.Y.Z-nightly.YYYYMMDD` (nur bei Änderungen), als GitHub-Pre-Release mit begrenzter Aufbewahrung (letzte 14). Kanalwahl über `update.channel` (CLI und Desktop gemeinsam) bzw. Installer-Option `--channel`. Ein Wechsel auf einen „älteren“ Kanal installiert erst die nächste höhere Version dieses Kanals (kein Downgrade ohne `--version`).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach `beton config set update.channel nightly` findet `beton upgrade --check` den neuesten Nightly.
  - [ ] AC2 — Wechsel von nightly zurück nach stable führt zu keinem Downgrade; `--check` meldet „aktuellste stable < installiert“.
  - [ ] AC3 — Nightlies älter als die letzten 14 werden automatisch entfernt.
- **Abhängigkeiten:** DIST-002

### DIST-018 — Versionierung & Kompatibilitätsfenster
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Eine SemVer-Version für das gesamte Produkt (siehe Design). Server, Host/Runner und Clients tauschen beim Verbindungsaufbau Produkt- und Protokollversion aus; erlaubt ist eine Minor-Differenz von höchstens 1 in beide Richtungen. Außerhalb des Fensters wird die Verbindung mit klarer Meldung und Upgrade-Hinweis abgelehnt. Breaking Changes am Protokoll erfordern eine Protokoll-Versionserhöhung und einen `!`-Commit.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Integrationstest-Matrix: Client N gegen Server N-1, N, N+1 funktioniert; gegen N-2 wird abgelehnt (WS-Close `4400` gemäß PROTO-004, Problem-Code `version_incompatible`).
  - [ ] AC2 — Alle Workspace-Crates, Desktop und `@ifahrentholz/beton-sdk` tragen dieselbe Version (CI-Check).
  - [ ] AC3 — Eine DB-Migration legt vor Ausführung ein Backup der SQLite-Datei an; Downgrade auf eine ältere Binary mit neuerem Schema wird mit Fehler verweigert.
- **Abhängigkeiten:** PROTO-004, PROTO-014 (siehe 06-data-sync-protocol.md)
- **Referenz:** ADR-0019, ADR-0026

### DIST-019 — Release-Prozess & Changelog
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Conventional Commits (ab M0 erzwungen, QA-014) speisen `release-please` (Single-Version-Workspace), das einen Release-PR mit Versionserhöhung und `CHANGELOG.md` pflegt. Changelog-Einträge enthalten Feature-IDs. Merge des Release-PR erzeugt Tag und startet die Release-Pipeline (siehe Design). Breaking Changes erscheinen in eigener Sektion mit Migrationshinweis.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein `feat(policy): POL-003 …`-Commit erscheint im nächsten Release-PR unter „Features“ mit Feature-ID.
  - [ ] AC2 — Ein `feat!:`-Commit führt (ab 1.0) zu einer Major-, davor zu einer Minor-Erhöhung und zu einem Eintrag unter „Breaking Changes“.
  - [ ] AC3 — Ein Release ohne erfolgreiche Smoke-Tests bleibt Draft und wird nicht veröffentlicht.
- **Abhängigkeiten:** DIST-002, QA-014

### DIST-020 — Deinstallation
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** `beton uninstall [--purge] [--yes]` stoppt Daemon/Host, entfernt Dienste (launchd/systemd/geplante Tasks), bei `script`-Installation das Binary und PATH-Einträge; bei Paketmanagern wird der passende Deinstallationsbefehl ausgegeben. `--purge` entfernt nach Bestätigung zusätzlich `~/.beton` (Daten, Logs, Modelle, Plugins), beton-eigene Keychain-Einträge und die Egress-Proxy-CA. Die Desktop-App wird über das OS deinstalliert; Daten bleiben ohne `--purge` erhalten. CLI-Oberfläche: CLI-013.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Nach `beton uninstall` (Script-Installation) existieren Binary und Service-Definitionen nicht mehr; `~/.beton` ist unverändert.
  - [ ] AC2 — Nach `beton uninstall --purge --yes` existieren weder `~/.beton` noch beton-Keychain-Einträge (Test auf macOS/Linux mit Test-Keychain).
  - [ ] AC3 — Fremde Keychain-Einträge, insbesondere die der Vendor-CLIs, bleiben unberührt.
- **Abhängigkeiten:** DIST-003

### QA — Qualität

### QA-001 — Teststrategie & Testpyramide
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Verbindliche Ebenen und Orte: (1) **Unit** — im Crate (`#[cfg(test)]`), schnell, ohne IO; (2) **Property/Fuzz** — `proptest` im Crate, `cargo-fuzz` unter `fuzz/`; (3) **Contract** — Trait-Suiten für Harness-Adapter, RunnerProvider, Git-Provider, Plugins; (4) **Golden** — Adapter gegen Aufnahmen (HAR-025, siehe 01-harnesses.md); (5) **Integration** — `crates/*/tests/`, echter Daemon in-process mit SQLite, Fake-Harness; (6) **Sicherheit** — Sandbox-Escape-Suite, Policy-Tests; (7) **E2E** — Playwright/WebDriver gegen Web und Desktop; (8) **Benchmarks**. Zielverhältnis grob 70 % Unit/Property, 20 % Contract/Golden/Integration, 10 % E2E. Kein Test darf echte Vendor-Accounts, Subscriptions oder das Internet benötigen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `AGENTS.md` beschreibt Ebenen, Orte, Befehle (`cargo nextest run`, `pnpm test`, `pnpm e2e`) und Namenskonventionen.
  - [ ] AC2 — CI läuft mit blockiertem ausgehendem Netz für Testjobs (außer Paket-Registries in Setup-Schritten); ein Test mit Netzzugriff schlägt fehl.
  - [ ] AC3 — Die PR-Pipeline (ohne E2E-Desktop und Benchmarks) läuft in ≤ 15 min.
- **Abhängigkeiten:** —
- **Referenz:** ADR-0031

### QA-002 — Protokoll-Fake-CLIs
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Test-Binary `beton-fake-cli` *(Annahme: Crate `crates/beton-fake-cli`, `publish = false`)* simuliert die Wire-Protokolle der Vendor-CLIs: `--protocol stream-json` (Claude Code), `app-server` (Codex, ab M1), `acp` (ab M1). Es spielt Szenarien im YAML-Format des Fake-Harness (Owner des Formats: HAR-026, siehe 01-harnesses.md) ab, erwartet Eingaben, sendet Permission-/Approval-Requests und reagiert auf Entscheidungen, Interrupts und Fehlerinjektion (Crash, Hänger, ungültiges JSON). So werden die **echten** Adapter über ihre Prozessgrenze getestet, deterministisch und ohne Subscription.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Der Claude-Adapter läuft mit `BETON_CLAUDE_PATH=beton-fake-cli --protocol stream-json --scenario …` ein Tool-Approval-Szenario vollständig durch (Integrationstest).
  - [ ] AC2 — Identisches Szenario + Eingabe ergeben byte-identische stdout-Ausgaben des Fake-CLIs über 100 Läufe.
  - [ ] AC3 — Fehlerinjektionen `crash_after`, `hang`, `malformed_line` sind konfigurierbar; der Adapter behandelt jede ohne Panic.
  - [ ] AC4 — Die Fake-CLI ist nicht Teil von Release-Artefakten (Prüfung der Release-Archive).
- **Abhängigkeiten:** HAR-026 (siehe 01-harnesses.md)
- **Referenz:** ADR-0031

### QA-003 — Golden-Transcript-Gate & Drift-Prozess
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Golden-Tests (Aufbau, Normalisierung, Aufnahme mit `beton dev record-golden`: Owner HAR-025 in 01-harnesses.md; QA-003 regelt nur Gate und Drift-Prozess) sind ein CI-Gate. Prozess bei neuer Vendor-CLI-Version: lokal neu aufnehmen, Diff-Report im PR, Update des unterstützten Versionsbereichs; ohne Golden-Satz für eine Version wird sie nicht als „unterstützt“ deklariert. Aufnahmen durchlaufen vor dem Commit einen Secret-Scan (gitleaks) zusätzlich zum Scrubbing.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Änderung am Normalisierer, die ein Golden-Ergebnis verändert, lässt CI fehlschlagen, bis die Erwartung explizit aktualisiert ist (`--bless` erzeugt Diff im PR).
  - [ ] AC2 — Ein Pre-Commit- und CI-Secret-Scan über `**/golden/**` schlägt bei eingebetteten Test-Keys fehl.
  - [ ] AC3 — Für jede im Harness-Katalog als unterstützt deklarierte CLI-Version existiert mindestens ein Golden-Satz (CI-Check).
- **Abhängigkeiten:** QA-002; HAR-025 (siehe 01-harnesses.md)
- **Referenz:** ADR-0031

### QA-004 — Deklarative Policy-Tests
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Policies werden mit YAML-Testfällen getestet: Event + Kontext rein, erwartete Entscheidung raus (inkl. Regel-ID und `modify`-Ergebnis). Ausführung über `beton-policy` (Testformat und `beton policy test`: Owner POL-026 in 03-policies.md; QA-004 regelt nur das CI-Gate) und in CI für alle mitgelieferten Policies und Built-in-Agents. Hierarchie-Fälle (User/Projekt/Agent, „strengere gewinnt“) sind Pflichtbestandteil.
- **Details:**
  ```yaml
  # policies/tests/dangerous-shell.policytest.yaml   (Format POL-026)
  spec_version: 1
  layers: { project: [../dangerous-shell.yaml] }
  cases:
    - name: rm -rf fragt nach
      phase: tool_call
      given: { session: { cost_usd: 0.4 } }
      input: { tool: { name: Bash, kind: shell, args: { command: "rm -rf build" } } }
      expect: { outcome: ask, rules: [rm-rf] }
  ---
  # policies/tests/hierarchy.policytest.yaml
  spec_version: 1
  layers: { project: [../deny-push.yaml], agent: [../allow-all.yaml] }
  cases:
    - name: Projekt-deny schlägt Agent-allow
      phase: tool_call
      input: { tool: { name: Bash, kind: shell, args: { command: "git push" } } }
      expect: { outcome: deny }
  ```
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein fehlschlagender Fall zeigt erwartete vs. tatsächliche Entscheidung samt auslösender Regel.
  - [ ] AC2 — Jede mitgelieferte Policy-Datei hat mindestens eine Testdatei (CI-Check).
  - [ ] AC3 — Property-Test: Hinzufügen einer `deny`-Regel macht keine Entscheidung weniger streng (Monotonie).
- **Abhängigkeiten:** POL-026 (siehe 03-policies.md), QA-010
- **Referenz:** ADR-0008, ADR-0031

### QA-005 — Sandbox-Escape-Suite pro OS
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Das Probe-Binary `bt-escape-probe` versucht innerhalb der Sandbox (Stufe 1 und 2, siehe 04-sandbox.md) verbotene Aktionen; Suite, Fälle und Plattform-Matrix: Owner SBX-011, QA-005 regelt Gate und Reporting; jeder Fall muss scheitern und den erwarteten Mechanismus zeigen. CI läuft auf macOS, Linux und (ab M5) Windows. macOS/Linux: jeder Fehlschlag blockiert den Merge. Windows Beta: Regressionen blockieren; bekannte Lücken stehen in `tests/sandbox/known_gaps.windows.yaml` und werden im Release-Text genannt.
- **Details:** Fälle (Auszug): `~/.ssh/id_*` und `~/.aws/credentials` lesen; Keychain/Secret-Service abfragen; `/etc/hosts` bzw. außerhalb des Workspace schreiben; Shell-RC-Dateien ändern; direkter TCP-Connect an Internet-IP (Proxy-Bypass); HTTPS über Proxy an nicht freigegebene Domain; `169.254.169.254` und `127.0.0.1:<Daemon-Port>`; DNS-Rebinding-Fixture; UDP/Raw-Sockets; `/var/run/docker.sock`; Symlink- und Hardlink-Ausbruch; `ptrace` des Elternprozesses; Lesen der Env anderer Prozesse via `/proc`; Env-Leak (keine Secrets in `env`); Fork-Bomb gegen Limits; verschachteltes `sandbox-exec`/`unshare`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Alle Fälle für macOS und Linux sind implementiert und schlagen in der Sandbox fehl; derselbe Probe **ohne** Sandbox gelingt (Kontrollgruppe, beweist Testwirksamkeit).
  - [ ] AC2 — Ein absichtlich gelockertes Profil (Test-Fixture) lässt die Suite fehlschlagen.
  - [ ] AC3 — Jeder Linux-Job der Matrix (SBX-011) prüft die für sein Image erwartete Landlock-ABI (mindestens ein Job mit ABI ≥ 4); weicht sie ab, schlägt der Job fehl statt Fälle zu überspringen.
  - [ ] AC4 — Ergebnisse werden als JUnit-Report mit Fall-ID veröffentlicht.
- **Abhängigkeiten:** SBX-011 (siehe 04-sandbox.md), QA-010
- **Referenz:** ADR-0007, ADR-0031

### QA-006 — Schema- & OpenAPI-Snapshot-Tests
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Generierte Artefakte — JSON-Schemas (`schemars`) für Events, Agent-YAML, Config, Plugin-Protokoll/-Manifest, `doctor`-Ausgabe; TS-Typen (`ts-rs`/`specta`); OpenAPI 3.1 (`utoipa`) — werden committet (Pfade gemäß PROTO-013: `schemas/`, `openapi/v1.json`, `packages/sdk-ts/src/gen/`). CI regeneriert sie und schlägt bei Abweichung fehl. Für OpenAPI erkennt `oasdiff` Breaking Changes; diese verlangen `!`-Commit bzw. Label `breaking-change`.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine Feldänderung in einem Event-Typ ohne regenerierte Schemas lässt CI mit Diff-Ausgabe fehlschlagen.
  - [ ] AC2 — Ein entferntes OpenAPI-Feld ohne `!`/Label wird als Breaking Change blockiert.
  - [ ] AC3 — Frontend-`tsc` läuft gegen die generierten Typen; Drift zwischen Rust und TS ist damit ausgeschlossen.
- **Abhängigkeiten:** PROTO-013 (siehe 06-data-sync-protocol.md), API-001 (siehe 08-clients.md)
- **Referenz:** ADR-0019

### QA-007 — E2E-Tests Web (Playwright)
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Playwright-Tests (`apps/web/e2e/`) starten einen echten Daemon mit temporärem Datenverzeichnis und Fake-Harness bzw. Fake-CLI und prüfen Kernflüsse: Session starten und streamen, zweiter Client sieht Live-Events, Reload mit Resume ab `seq`; ab M2 Approval über Card und Inbox; ab M3 Command-Palette, Themes, Voice mit Audio-Fixture; ab M4 Sharing/Co-Drive mit zwei Usern. Browser: Chromium, WebKit (für Safari/PWA-Nähe), Firefox.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Das M0-Szenario „`beton run fake` → Browser zeigt gestreamte Antwort → Reload → vollständiger Verlauf“ läuft in CI auf allen drei Browser-Engines.
  - [ ] AC2 — Tests sind deterministisch: 20 aufeinanderfolgende Läufe ohne Flake (Nightly-Job mit `--repeat-each`).
  - [ ] AC3 — Fehlgeschlagene Tests liefern Trace, Screenshot und Daemon-Logs als CI-Artefakt.
- **Abhängigkeiten:** QA-002; HAR-026 (siehe 01-harnesses.md), WEB-001 (siehe 08-clients.md)
- **Referenz:** ADR-0031

### QA-008 — E2E-Tests Desktop (tauri-driver)
- **Meilenstein:** M3 · **Priorität:** Must
- **Beschreibung:** Desktop-E2E über `tauri-driver` (WebDriver) auf Linux (WebKitWebDriver) und Windows (Edge WebDriver), mit einem WebDriver-Client (WebdriverIO *(Annahme)*) und denselben Szenario-Beschreibungen/Page-Objects wie die Playwright-Suite. Da `tauri-driver` macOS nicht unterstützt, wird macOS-Desktop durch die Web-Suite (WebKit) plus eine Smoke-Checkliste vor jedem Release abgedeckt. Zusätzlich desktop-spezifische Fälle: Deep-Link, Tray-Badge-Zähler (via Test-Hook), Daemon-Autostart, Updater mit Test-Manifest.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Die Kernszenarien aus QA-007 laufen gegen die gebaute Desktop-App auf Linux und Windows grün.
  - [ ] AC2 — `beton://local/s/<id>` (DESK-006) öffnet im Test die richtige Session.
  - [ ] AC3 — Der Updater-Test installiert ein signiertes Test-Update aus lokalem Manifest und lehnt ein unsigniertes ab.
- **Abhängigkeiten:** QA-007; DESK-001, DESK-006 (siehe 08-clients.md)

### QA-009 — Property- & Fuzz-Tests (Proxy-Parser, CEL)
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** `cargo-fuzz`-Targets für sicherheitskritische Parser: HTTP/1.1-Request- und CONNECT-Parser des Egress-Proxys, TLS-ClientHello/SNI-Extraktion, Egress-Regel-Parser und -Matcher, Credential-Platzhalter-Ersetzung, CEL-Parse+Eval mit zufälligem Kontext, Policy-YAML-Loader, WS-Frame-Decoder, SBPL-Pfad-Escaping. `proptest`-Eigenschaften u.a.: Regel-Matcher ≡ Referenzimplementierung, CEL-Auswertung deterministisch und zeitbegrenzt, Pfad-Kanonisierung idempotent, Platzhalter nie an fremde Hosts. PR-Läufe 60 s pro betroffenem Target, Nightly 30 min; Korpora im Repo, jeder Fund wird Regressionstest.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Für jedes genannte Target existiert ein Fuzz-Harness mit Seed-Korpus; Nightly-Läufe laufen ohne Crash/Timeout.
  - [ ] AC2 — Ein PR, der `beton-proxy` ändert, startet automatisch die Proxy-Fuzz-Targets (60 s je Target).
  - [ ] AC3 — CEL-Auswertung eines Fuzz-Ausdrucks überschreitet nie 10 ms bzw. das Kosten-Limit (Panic oder Hänger = Fehlschlag).
- **Abhängigkeiten:** PRX-003, PRX-006 (siehe 04-sandbox.md), POL-002 (siehe 03-policies.md)
- **Referenz:** ADR-0031

### QA-010 — CI-Gates
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Pflicht-Checks für jeden PR (Branch-Protection auf `main`): `cargo fmt --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo deny check` (Lizenzen: Apache-2.0-kompatible Allowlist, Advisories, Bans, Quellen); `cargo nextest run --workspace` plus `cargo test --doc`; MSRV-Build; Frontend `pnpm tsc --noEmit`, `pnpm lint` (ESLint *(Annahme)*), `pnpm vitest run`; Schema-Snapshots (QA-006); Golden-Tests (QA-003); Commit-Lint und DCO (QA-014, QA-015). Ab M2 zusätzlich Coverage-Floor (QA-011), Sandbox-Escape (QA-005), Policy-Tests (QA-004).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Eine neue Clippy-Warnung lässt den PR fehlschlagen.
  - [ ] AC2 — Eine Abhängigkeit mit GPL-3.0-Lizenz oder bekannter RUSTSEC-Advisory wird von `cargo deny` blockiert.
  - [ ] AC3 — Alle Gates sind als Required Checks in der Branch-Protection hinterlegt (CI prüft per GitHub-API).
- **Abhängigkeiten:** —
- **Referenz:** ADR-0031

### QA-011 — Coverage-Floor für `beton-policy` und `beton-sandbox`
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** `cargo llvm-cov` misst Zeilen- und Branch-Coverage; plattformspezifischer Sandbox-Code wird auf dem jeweiligen OS gemessen und zusammengeführt. Floor *(Annahme)*: `beton-policy` ≥ 90 % Zeilen, `beton-sandbox` ≥ 80 % Zeilen; Ratchet — der Wert darf pro PR nicht sinken (Toleranz 0,2 Prozentpunkte). `beton-proxy`, `beton-secrets` werden berichtet, ohne Gate (Should: Gate ab M4).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein PR, der die Coverage von `beton-policy` unter 90 % senkt, schlägt fehl.
  - [ ] AC2 — Der Coverage-Report pro Crate wird als PR-Kommentar bzw. Job-Summary angezeigt.
  - [ ] AC3 — Sinkt die Coverage um mehr als 0,2 Punkte gegenüber `main`, schlägt der Ratchet-Check fehl.
- **Abhängigkeiten:** QA-010

### QA-012 — Performance-Benchmarks
- **Meilenstein:** M3 · **Priorität:** Should
- **Beschreibung:** `criterion`-Benchmarks und ein Last-Harness (Fake-Harness mit hoher Event-Rate) messen Budgets *(Annahme, Erstwerte)*: Event-Append SQLite ≥ 5 000 Events/s pro Session, p99 < 5 ms; WS-Fan-out an 10 Clients ≥ 2 000 Events/s je Client; `beton --version` < 50 ms Kaltstart; `beton run fake` bis Session bereit < 300 ms (ohne Harness-Start); Daemon-RSS idle < 60 MB, mit 10 idle Sessions < 150 MB (ohne Harness-Prozesse und Whisper-Modell); Desktop bis interaktiv < 1,5 s. Nightly auf festem Runner-Typ; > 10 % Regression erzeugt Issue, ab v1.0 PR-Gate für Event-Pfad.
- **Akzeptanzkriterien:**
  - [ ] AC1 — `cargo bench -p beton-store` und `xtask bench e2e` laufen reproduzierbar und schreiben JSON-Ergebnisse.
  - [ ] AC2 — Der Nightly-Job vergleicht mit der Baseline und öffnet bei > 10 % Regression automatisch ein Issue mit Diff.
  - [ ] AC3 — Alle genannten Budgets sind als Assertions im Bench-Harness hinterlegt und werden im Release-Check von 0.1 eingehalten.
- **Abhängigkeiten:** QA-002
- **Referenz:** ADR-0031

### QA-013 — TDD- & Review-Pflicht für sicherheitskritische Crates
- **Meilenstein:** M2 · **Priorität:** Must
- **Beschreibung:** Änderungen an sicherheitskritischen Crates folgen TDD (zuerst fehlschlagender Test, dann Implementierung) und benötigen ein **menschliches** Review: `CODEOWNERS` weist diese Pfade dem Projektinhaber zu; Branch-Protection verlangt dessen Approval, Agents können nicht freigeben. Ein CI-Check verlangt bei Codeänderungen in diesen Crates mindestens eine geänderte oder neue Testdatei, sonst ist das Label `no-test-needed` nötig, das nur Menschen setzen dürfen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein PR, der `crates/beton-sandbox/src/**` ändert, ist ohne Approval des Code-Owners nicht mergebar.
  - [ ] AC2 — Ein solcher PR ohne Teständerung schlägt im Check `security-tests-touched` fehl.
  - [ ] AC3 — Die PR-Vorlage enthält für diese Pfade die Checkliste „Test zuerst (rot) — Implementierung (grün) — Escape-/Policy-Suite grün“.
- **Abhängigkeiten:** QA-010
- **Referenz:** ADR-0031

### QA-014 — Agent-Workflow & Feature-IDs
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Commits folgen Conventional Commits mit Feature-ID: `<type>(<scope>): <ID> <Beschreibung>`, z.B. `feat(policy): POL-003 hierarchische Auswertung`; PR-Titel ebenso, PR-Body listet umgesetzte ACs (`Implements: POL-003 AC1–AC3`). Tests referenzieren ACs im Namen (`pol_003_ac2_stricter_rule_wins`). `AGENTS.md` und `CLAUDE.md` beschreiben Workflow, Befehle, Spec-Pfade und Grenzen (keine Änderung an Spec-Entscheidungen ohne ADR). `cargo xtask spec-coverage` listet ACs ohne zugeordneten Test.
- **Details:** Commit-Lint-Regex: `^(feat|fix|perf|refactor|test|docs|build|ci|chore)(\([a-z0-9-]+\))?!?: ((HAR|AGT|ASY|POL|SBX|PRX|AUTH|SEC|PROTO|DATA|SYNC|SES|COL|GIT|DESK|WEB|CLI|TUI|API|BRW|RUN|PLG|USE|VOI|UX|OBS|DIST|QA)-\d{3}(,\s?[A-Z]+-\d{3})* )?.+` — Feature-ID Pflicht für `feat`, optional sonst.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein `feat`-Commit ohne Feature-ID wird vom Commit-Lint abgelehnt.
  - [ ] AC2 — Eine referenzierte, in der Spec nicht existierende ID (z.B. `POL-999`) lässt den Check fehlschlagen (Abgleich gegen `docs/spec/*.md`).
  - [ ] AC3 — `cargo xtask spec-coverage --milestone M0` gibt die Liste der M0-ACs ohne Test aus (Exit-Code 1, wenn Must-ACs fehlen).
- **Abhängigkeiten:** —
- **Referenz:** ADR-0031; Konventionen in 00-overview.md

### QA-015 — DCO-Check
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** Jeder Commit benötigt `Signed-off-by:` mit Name/E-Mail passend zum Autor (Developer Certificate of Origin); kein CLA. Agent-Commits werden vom verantwortlichen Menschen abgezeichnet (`git commit -s`), der damit die DCO-Erklärung abgibt *(Annahme)*; `Co-Authored-By`-Zeilen für Agents sind erlaubt. `CONTRIBUTING.md` erklärt das Vorgehen.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein PR mit einem Commit ohne `Signed-off-by` schlägt im DCO-Check fehl.
  - [ ] AC2 — Abweichende Sign-off-E-Mail gegenüber Commit-Autor wird abgelehnt.
  - [ ] AC3 — `CONTRIBUTING.md` verlinkt den DCO-Text und zeigt `git commit -s` sowie das Nachholen per `git rebase --signoff`.
- **Abhängigkeiten:** —
- **Referenz:** ADR-0027

### QA-016 — Harness-Contract-Suite
- **Meilenstein:** M1 · **Priorität:** Should
- **Beschreibung:** Eine gemeinsame Contract-Suite prüft jeden Harness-Adapter (über Fake-CLI) gegen seine deklarierten Capabilities: Streaming, Interrupt, Approval-Roundtrip, Resume, Modellwechsel, Usage-Reporting, Fehlerpfade. Dieselbe Suite wird für Harness-Plugins wiederverwendet (PLG-013, siehe 10-runners-extensibility.md).
- **Akzeptanzkriterien:**
  - [ ] AC1 — Ein Adapter, der `interrupt: true` deklariert, aber nach Interrupt weiter Deltas sendet, fällt durch.
  - [ ] AC2 — Claude-, Codex- und ACP-Adapter bestehen die Suite in CI.
- **Abhängigkeiten:** QA-002; HAR-002 (siehe 01-harnesses.md)

### QA-017 — Datenbank-Matrix-Tests
- **Meilenstein:** M4 · **Priorität:** Must
- **Beschreibung:** Repository- und Integrationstests von `beton-store`/`beton-server` laufen gegen SQLite **und** Postgres (CI-Service-Container, unterstützte Major-Versionen *(Annahme: Postgres 15–17)*). Migrationen werden auf beiden Backends von leer und von der letzten Release-Version aus getestet.
- **Akzeptanzkriterien:**
  - [ ] AC1 — Die Repository-Test-Suite läuft mit `BETON_TEST_DB=sqlite` und `=postgres` identisch grün.
  - [ ] AC2 — Ein Upgrade-Test migriert einen mit der Vorgängerversion erzeugten Datenbestand ohne Datenverlust (Event-Zähler und Hashes vorher = nachher).
- **Abhängigkeiten:** DATA-003, DATA-004 (siehe 06-data-sync-protocol.md)

## Nicht in v1

- **Flatpak** für die Desktop-App — v2 (ADR-0026).
- **AUR- und Nix-Pakete** — später, nicht v1.
- **Eigene apt/yum-Repositories** — nicht v1 (Pakete nur als Release-Assets).
- **Native Mobile-Apps** (Tauri Mobile) inkl. Store-Distribution — v2.
- **Python-SDK** und dessen PyPI-Distribution — v2.
- **VS-Code-Extension** (Marketplace) — v2.
