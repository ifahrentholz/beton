# Beitragen zu beton

Danke für dein Interesse! beton wird überwiegend von Coding-Agents gebaut und von Ingo Fahrentholz verantwortet. Die Arbeitsregeln für Menschen und Agents stehen in [AGENTS.md](AGENTS.md); dieses Dokument fasst das Wichtigste zusammen.

## Developer Certificate of Origin (DCO)

beton nutzt das [Developer Certificate of Origin 1.1](https://developercertificate.org/) statt eines CLA (ADR-0027). Mit einem `Signed-off-by` in jedem Commit bestätigst du, dass du den Beitrag unter der Projektlizenz (Apache-2.0) einreichen darfst.

```bash
git commit -s -m "fix(store): DATA-002 seq bleibt lückenlos"
```

`-s` fügt `Signed-off-by: Dein Name <deine@email>` hinzu. Name und E-Mail müssen zum Commit-Autor passen; die CI prüft das (`cargo xtask dco`). Commits von Agents zeichnet der verantwortliche Mensch ab; `Co-Authored-By`-Zeilen für Agents sind erlaubt.

Sign-off vergessen? Für die Commits deines Branches nachholen:

```bash
git rebase --signoff origin/main
git push --force-with-lease
```

## Commits und Pull Requests

- Format: `<type>(<scope>): <Feature-ID> <Beschreibung>`, z. B. `feat(policy): POL-003 hierarchische Auswertung`. `feat` braucht eine Feature-ID aus [`docs/spec/`](docs/spec/00-overview.md); andere Typen optional.
- Ein PR schließt sein Issue mit `Closes #<Nummer>` und listet die umgesetzten Akzeptanzkriterien.
- Vor dem PR lokal: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, `cargo xtask spec-check`.

## Entscheidungen

Grundsätzliche Änderungen laufen über ein neues ADR in [`docs/adr/`](docs/adr/README.md), nicht über stilles Umschreiben der Spec.
