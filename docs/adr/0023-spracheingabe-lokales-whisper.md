# ADR-0023: Spracheingabe – nur lokales Whisper im Server/Daemon, keine Cloud

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Prompts per Sprache zu diktieren ist ein gefragtes Feature. Omnigent nutzt die Web Speech API im Browser und eine serverseitige Transkription (sherpa-onnx) mit Streaming über WebSocket, weil Web Speech in Electron nicht transkribiert. In Tauri-WebViews ist Web Speech ebenfalls unzuverlässig bzw. je OS unterschiedlich. Prompts können vertrauliche Inhalte (Code, Kundennamen) enthalten.

## Betrachtete Optionen
1. **A — OS-native Spracherkennung (macOS Speech, Windows Speech, Web Speech API)** — keine Modelle nötig; je Plattform unterschiedlich, teilweise Cloud-gestützt, Linux schwach.
2. **B — Lokales Whisper (whisper.cpp) im Server/Daemon** — plattformübergreifend identisch, offline, datenschutzfreundlich; Modell-Download und Rechenlast.
3. **C — Cloud-Transkription (z. B. OpenAI, Deepgram)** — beste Qualität ohne lokale Last; Datenabfluss, API-Keys, Kosten.

## Entscheidung
Option **B**: **nur lokales Whisper** (`whisper-rs`/whisper.cpp) in Crate `beton-voice`.
- Modelle `small` bis `large-v3-turbo`, **on demand** geladen; Metal-Beschleunigung auf Apple Silicon.
- Läuft **im Server/Daemon**; Audio kommt per WebSocket vom Client (auch PWA).
- Push-to-talk via globalem Shortcut in der Desktop-App.
- **Keine Cloud-Transkription.** Prompt-Cleanup nach Diktat ist v2.

## Konsequenzen
- Positiv: Kein Datenabfluss; identisches Verhalten in Desktop, Web und PWA.
- Negativ / Risiken: Modell-Downloads (Hunderte MB bis > 1 GB); CPU-Last auf Servern ohne GPU; Latenz bei großen Modellen.
- Negativ / Risiken: Zentraler Server transkribiert für alle Nutzer → Ressourcenplanung.
- Folgearbeiten: Modellverwaltung (Download nach Zustimmung, Prüfsummen), Audio-Binärkanal im WS (ADR-0019), Mikrofon-Berechtigungen in Tauri/PWA.

## Bezug
- Spec: docs/spec/11-platform-features.md (Prefix VOI)
