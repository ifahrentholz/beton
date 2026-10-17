#!/usr/bin/env bash
# Sprach-Fixtures für die nur lokal laufenden Whisper-Tests (VOI-001 AC1, VOI-003 AC3, VOI-006).
#
# Erzeugt aus den Referenz-Transkripten in crates/beton-voice/tests/fixtures/{de,en,fr}.txt
# WAV-Dateien (16 kHz, mono, PCM16) per Sprachsynthese: macOS `say`, sonst `espeak-ng` und
# `sox`. Die Audiodateien gehören nicht ins Repo (Nutzungsbedingungen der Systemstimmen); nur
# die Transkripte sind eingecheckt.
#
#   scripts/voice_fixtures.sh [ZIELVERZEICHNIS]   # Default: target/voice-fixtures
#   BETON_TEST_VOICE_FIXTURES=target/voice-fixtures BETON_TEST_VOICE_MODELS=~/.beton/models/whisper \
#     cargo test -p beton-voice --release --test real_models -- --ignored
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
src="$root/crates/beton-voice/tests/fixtures"
out="${1:-$root/target/voice-fixtures}"
mkdir -p "$out"

if command -v say >/dev/null 2>&1; then
  for spec in de:Anna en:Samantha fr:Thomas; do
    lang="${spec%%:*}"
    voice="${spec##*:}"
    say -v "$voice" -o "$out/$lang.wav" --file-format=WAVE --data-format=LEI16@16000 -f "$src/$lang.txt"
  done
elif command -v espeak-ng >/dev/null 2>&1 && command -v sox >/dev/null 2>&1; then
  for lang in de en fr; do
    espeak-ng -v "$lang" -f "$src/$lang.txt" -w "$out/$lang.raw.wav"
    sox "$out/$lang.raw.wav" -r 16000 -c 1 -b 16 "$out/$lang.wav"
    rm -f "$out/$lang.raw.wav"
  done
else
  echo "Weder macOS \`say\` noch \`espeak-ng\` + \`sox\` gefunden." >&2
  exit 1
fi
echo "Fixtures in $out"
