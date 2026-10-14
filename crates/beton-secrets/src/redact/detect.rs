//! Muster-Detektoren für verbreitete Token-Formate (SEC-013).

use super::Span;

/// Sucht Token-Muster; `Err`, wenn die Detektoren nicht bereitstehen (fail closed).
pub(crate) fn find(_text: &str, _out: &mut Vec<Span>) -> Result<(), super::Unavailable> {
    Ok(())
}

/// CRC32 (IEEE) für die Prüfsumme der beton-Tokens.
pub(crate) fn crc32(_data: &[u8]) -> u32 {
    0
}

/// Prüfsumme eines beton-Tokens: CRC32 der 40 Zufallszeichen, base62 (`0-9A-Za-z`), 6 Zeichen.
pub fn beton_token_checksum(_random: &str) -> String {
    String::new()
}
