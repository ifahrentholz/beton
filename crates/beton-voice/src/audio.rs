//! Audio-Format der Engine: 16 kHz mono, PCM16 LE (VOI-004) bzw. `f32` in [-1, 1].
//!
//! Audio wird nur im Arbeitsspeicher verarbeitet; dieses Modul kann WAV lesen (Fixtures), aber
//! nichts schreiben (DATA-011 AC3, VOI-004 AC2).

/// Abtastrate, die Whisper erwartet.
pub const SAMPLE_RATE: u32 = 16_000;

/// PCM16 LE → `f32`.
pub fn pcm16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|&s| f32::from(s) / 32_768.0).collect()
}

/// Dauer in Sekunden.
pub fn duration_secs(samples: &[f32]) -> f64 {
    samples.len() as f64 / f64::from(SAMPLE_RATE)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WavError {
    #[error("keine RIFF/WAVE-Datei")]
    NotWav,
    #[error("Format {0} wird nicht unterstützt (erwartet: PCM16, mono, 16 kHz)")]
    Unsupported(String),
    #[error("WAV-Datei ist abgeschnitten")]
    Truncated,
}

/// Liest eine WAV-Datei mit 16 kHz, mono, PCM16 LE.
pub fn read_wav(bytes: &[u8]) -> Result<Vec<f32>, WavError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotWav);
    }
    let mut pos = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        let body = pos + 8;
        let end = body.checked_add(len).ok_or(WavError::Truncated)?;
        if id == b"fmt " {
            if len < 16 || end > bytes.len() {
                return Err(WavError::Truncated);
            }
            let u16_at = |o: usize| u16::from_le_bytes([bytes[body + o], bytes[body + o + 1]]);
            let rate = u32::from_le_bytes([
                bytes[body + 4],
                bytes[body + 5],
                bytes[body + 6],
                bytes[body + 7],
            ]);
            format = Some((u16_at(0), u16_at(2), rate, u16_at(14)));
        } else if id == b"data" {
            let (tag, channels, rate, bits) = format.ok_or(WavError::NotWav)?;
            if tag != 1 || channels != 1 || rate != SAMPLE_RATE || bits != 16 {
                return Err(WavError::Unsupported(format!(
                    "tag {tag}, {channels} Kanäle, {rate} Hz, {bits} bit"
                )));
            }
            let data = bytes
                .get(body..end.min(bytes.len()))
                .ok_or(WavError::Truncated)?;
            let samples: Vec<i16> = data
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();
            return Ok(pcm16_to_f32(&samples));
        }
        // Chunks sind auf gerade Längen aufgefüllt.
        pos = end + (len & 1);
    }
    Err(WavError::Truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2 * u32::from(channels)).to_le_bytes());
        out.extend_from_slice(&(2 * channels).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        // Fremder Chunk dazwischen (z. B. `LIST` von `say`/`ffmpeg`), ungerade Länge.
        out.extend_from_slice(b"LIST");
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(b"abc\0");
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn reads_16k_mono_pcm16() {
        let s = read_wav(&wav(16_000, 1, &[0, 16_384, -32_768])).unwrap();
        assert_eq!(s, vec![0.0, 0.5, -1.0]);
        assert!((duration_secs(&vec![0.0; 16_000]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_other_formats() {
        assert!(matches!(
            read_wav(&wav(44_100, 1, &[0])),
            Err(WavError::Unsupported(_))
        ));
        assert!(matches!(
            read_wav(&wav(16_000, 2, &[0, 0])),
            Err(WavError::Unsupported(_))
        ));
        assert_eq!(read_wav(b"nope"), Err(WavError::NotWav));
    }
}
