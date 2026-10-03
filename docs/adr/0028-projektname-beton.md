# ADR-0028: Projektname "beton"

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Das Projekt braucht einen kurzen, merkfähigen Namen für Binary, Crates, Pakete, Config-Pfade und Deep-Links. Ein Meta-Harness "dirigiert" mehrere Agents und soll ein solides Fundament bilden. Der Name `beton` auf crates.io ist bereits vergeben.

## Betrachtete Optionen
1. **A — "beton"** — Wortspiel: *baton* (Taktstock des Dirigenten) und *Beton* (solides Fundament); kurz, gut tippbar; Crate-Name belegt.
2. **B — Ein Name mit freiem Crate-Namen** — keine Präfix-Umwege; Namenssuche ohne überzeugenden Kandidaten.
3. **C — Generischer Name ("meta-harness-rs" o. Ä.)** — beschreibend; nicht markenfähig.

## Entscheidung
Option **A – "beton"**:
- Binary `beton`.
- Crates mit Präfix `beton-*`; da `beton` auf crates.io belegt ist, heißt die Binary-Crate **`beton-cli`** und installiert das Binary `beton`.
- Pfade: User-Config `~/.beton/`, Projekt-Config `.beton/`, Deep-Links `beton://`, Credential-Platzhalter `bt_cred_*`.

## Konsequenzen
- Positiv: Einprägsamer Name mit doppelter Bedeutung; konsistentes Präfix für alle Artefakte.
- Negativ / Risiken: `cargo install beton` funktioniert nicht (→ `cargo install beton-cli` / `cargo binstall beton-cli`); Verwechslungsgefahr mit dem bestehenden Crate.
- Folgearbeiten / offene Punkte: **Markenrecht "beton" ungeprüft**; **Domain** (z. B. beton.dev / getbeton.dev) ungeprüft.

## Bezug
- Spec: docs/spec/00-overview.md
