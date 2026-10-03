# ADR-0026: Distribution, Signing und Updates

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
beton wird als CLI/Server-Binary, Desktop-App und Container-Images ausgeliefert, für macOS, Linux und Windows. Sicherheitsrelevante Software (Sandbox, Proxy mit eigener CA) muss vertrauenswürdig verteilt werden. Omnigent verteilt über PyPI/uv, Homebrew, curl-Installer und Electron-Auto-Update; Windows-Signing ist nicht ersichtlich.

## Betrachtete Optionen
1. **A — Nur GitHub Releases + `cargo install`** — minimaler Aufwand; schlechte UX, keine Signaturen, keine Updates.
2. **B — Breite Paketkanäle mit Signing pro Plattform und Update-Kanälen** — professionelle UX und Vertrauen; Accounts/Kosten und CI-Aufwand.
3. **C — Nur Container** — einheitlich; ungeeignet für Desktop und lokale Sandbox.

## Entscheidung
Option **B**:
- **CLI/Server-Binary:** GitHub Releases (macOS arm64/x64, Linux x64/arm64 musl, Windows x64/arm64), `curl | sh`- und PowerShell-Installer, Homebrew-Tap, winget, Scoop, `cargo binstall`, deb/rpm (AUR/Nix später).
- **Desktop:** `.dmg` (Universal), `.msi`/NSIS, `.AppImage` + `.deb` + `.rpm` (Flatpak v2).
- **Container:** `ghcr.io/ifahrentholz/beton-server`, `ghcr.io/ifahrentholz/beton-runner` (multi-arch); Docker-Compose + Helm-Chart.
- **Signing:** macOS Developer-ID + **Notarisierung**; Windows **Azure Trusted Signing**; Linux/Container **Sigstore/cosign** + SBOM + SLSA-Provenance.
- **Updates:** Tauri-Updater (signierte Manifeste), `beton upgrade` (erkennt Installationsart), Kanäle **stable / beta / nightly**.
- `beton setup` erkennt fehlende `claude`/`codex`-CLIs und **bietet** die Installation an – nie still.

## Konsequenzen
- Positiv: Vertrauenswürdige, verifizierbare Artefakte; einfache Installation auf allen Plattformen.
- Negativ / Risiken: Kosten und Verwaltung der Signing-Accounts (Apple Developer, Azure); umfangreiche Release-Pipeline.
- Folgearbeiten / offene Punkte: **Wer hält die Signing-Accounts** (Apple Developer, Azure Trusted Signing)? Release-Automatisierung (z. B. cargo-dist), Paketpflege in Fremd-Repos (winget, Scoop, Homebrew-Tap).

## Bezug
- Spec: docs/spec/12-distribution-quality.md (Prefix DIST)
