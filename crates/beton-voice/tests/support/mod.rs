//! Hilfen der Integrationstests: synthetisches Whisper-Modell und Messwerkzeuge.
//!
//! Das synthetische Modell hat das ggml-Format von whisper.cpp mit echten Dimensionen
//! (mehrsprachiges Vokabular, 1 500 Audio-Frames), aber nur je einer Schicht und Null-Gewichten.
//! whisper.cpp lädt und rechnet damit wie mit einem echten Modell; der Text ist bedeutungslos.
//! So prüfen die Tests Laden, Entladen, Speicher, Backend-Wahl und Netzverhalten der echten
//! Engine, ohne ein Modell herunterzuladen (ADR-0033).

#![allow(dead_code, clippy::unwrap_used)]

use std::io::Write;
use std::path::{Path, PathBuf};

/// `ggml` als Magic von whisper.cpp.
const GGML_FILE_MAGIC: u32 = 0x6767_6d6c;

/// Dimensionen des synthetischen Modells.
#[derive(Debug, Clone, Copy)]
pub struct Dims {
    /// Breite von Encoder und Decoder (`n_audio_state` = `n_text_state`).
    pub state: i32,
    pub heads: i32,
}

impl Dims {
    /// Rund 8 MB.
    pub const SMALL: Dims = Dims {
        state: 64,
        heads: 1,
    };
    /// Rund 65 MB: groß genug, um die Freigabe im RSS zu sehen (VOI-001 AC2).
    pub const LARGE: Dims = Dims {
        state: 512,
        heads: 8,
    };
}

const N_VOCAB: i32 = 51_865;
const N_AUDIO_CTX: i32 = 1_500;
const N_TEXT_CTX: i32 = 448;
const N_MELS: i32 = 80;
const N_FFT: i32 = 201;
/// EOT im mehrsprachigen Vokabular.
const TOKEN_EOT: usize = 50_257;

/// Gewichtsmatrizen und Faltungen sind f16 (`ftype` 1), Biases, Normen und Positionen f32 –
/// wie in den echten Modellen.
fn tensor(out: &mut impl Write, name: &str, ne: &[i32]) -> u64 {
    let f16 = name.ends_with(".weight") && ne.len() >= 2;
    out.write_all(&(ne.len() as i32).to_le_bytes()).unwrap();
    out.write_all(&(name.len() as i32).to_le_bytes()).unwrap();
    // ttype 0 = f32, 1 = f16
    out.write_all(&i32::from(f16).to_le_bytes()).unwrap();
    for n in ne {
        out.write_all(&n.to_le_bytes()).unwrap();
    }
    out.write_all(name.as_bytes()).unwrap();
    let count: i64 = ne.iter().map(|&n| i64::from(n)).product();
    let mut bytes = vec![0u8; usize::try_from(count).unwrap() * if f16 { 2 } else { 4 }];
    // Damit der Decoder sofort endet: Die letzte Norm liefert einen Einsvektor, und nur die
    // Zeile von EOT der Token-Einbettung ist ungleich null – EOT gewinnt ab dem zweiten Token.
    if name == "decoder.ln.bias" {
        for c in bytes.as_chunks_mut::<4>().0 {
            c.copy_from_slice(&1.0f32.to_le_bytes());
        }
    }
    if name == "decoder.token_embedding.weight" {
        let row = usize::try_from(ne[0]).unwrap() * 2;
        let start = TOKEN_EOT * row;
        for c in bytes[start..start + row].as_chunks_mut::<2>().0 {
            // 1.0 als f16
            c.copy_from_slice(&0x3c00u16.to_le_bytes());
        }
    }
    out.write_all(&bytes).unwrap();
    bytes.len() as u64
}

/// Schreibt ein synthetisches Modell nach `path` und liefert die Größe der Gewichte in Bytes.
pub fn write_model(path: &Path, dims: Dims) -> u64 {
    let f = std::fs::File::create(path).unwrap();
    let mut out = std::io::BufWriter::new(f);
    let s = dims.state;
    out.write_all(&GGML_FILE_MAGIC.to_le_bytes()).unwrap();
    // n_vocab, n_audio_ctx, n_audio_state, n_audio_head, n_audio_layer, n_text_ctx,
    // n_text_state, n_text_head, n_text_layer, n_mels, ftype (1 = f16)
    for v in [
        N_VOCAB,
        N_AUDIO_CTX,
        s,
        dims.heads,
        1,
        N_TEXT_CTX,
        s,
        dims.heads,
        1,
        N_MELS,
        1,
    ] {
        out.write_all(&v.to_le_bytes()).unwrap();
    }
    // Mel-Filter: n_mel × n_fft
    out.write_all(&N_MELS.to_le_bytes()).unwrap();
    out.write_all(&N_FFT.to_le_bytes()).unwrap();
    let filters = vec![0u8; (N_MELS * N_FFT) as usize * 4];
    out.write_all(&filters).unwrap();
    // Vokabular: nur das Leerzeichen (braucht `suppress_blank`) und ein Wort; die übrigen
    // Tokens (Sprachen, Timestamps, Steuer-Tokens) ergänzt whisper.cpp selbst.
    let vocab: [&[u8]; 2] = [b" ", b" beton"];
    out.write_all(&(vocab.len() as i32).to_le_bytes()).unwrap();
    for token in vocab {
        out.write_all(&(token.len() as u32).to_le_bytes()).unwrap();
        out.write_all(token).unwrap();
    }

    let mut weights = 0;
    weights += tensor(&mut out, "encoder.positional_embedding", &[s, N_AUDIO_CTX]);
    weights += tensor(&mut out, "encoder.conv1.weight", &[3, N_MELS, s]);
    weights += tensor(&mut out, "encoder.conv1.bias", &[1, s]);
    weights += tensor(&mut out, "encoder.conv2.weight", &[3, s, s]);
    weights += tensor(&mut out, "encoder.conv2.bias", &[1, s]);
    weights += tensor(&mut out, "encoder.ln_post.weight", &[s]);
    weights += tensor(&mut out, "encoder.ln_post.bias", &[s]);
    let block = |out: &mut std::io::BufWriter<std::fs::File>, prefix: &str| {
        let mut w = 0;
        w += tensor(out, &format!("{prefix}.mlp_ln.weight"), &[s]);
        w += tensor(out, &format!("{prefix}.mlp_ln.bias"), &[s]);
        w += tensor(out, &format!("{prefix}.mlp.0.weight"), &[s, 4 * s]);
        w += tensor(out, &format!("{prefix}.mlp.0.bias"), &[4 * s]);
        w += tensor(out, &format!("{prefix}.mlp.2.weight"), &[4 * s, s]);
        w += tensor(out, &format!("{prefix}.mlp.2.bias"), &[s]);
        w += tensor(out, &format!("{prefix}.attn_ln.weight"), &[s]);
        w += tensor(out, &format!("{prefix}.attn_ln.bias"), &[s]);
        w += tensor(out, &format!("{prefix}.attn.query.weight"), &[s, s]);
        w += tensor(out, &format!("{prefix}.attn.query.bias"), &[s]);
        w += tensor(out, &format!("{prefix}.attn.key.weight"), &[s, s]);
        w += tensor(out, &format!("{prefix}.attn.value.weight"), &[s, s]);
        w += tensor(out, &format!("{prefix}.attn.value.bias"), &[s]);
        w += tensor(out, &format!("{prefix}.attn.out.weight"), &[s, s]);
        w += tensor(out, &format!("{prefix}.attn.out.bias"), &[s]);
        w
    };
    weights += block(&mut out, "encoder.blocks.0");
    weights += tensor(&mut out, "decoder.positional_embedding", &[s, N_TEXT_CTX]);
    weights += tensor(&mut out, "decoder.token_embedding.weight", &[s, N_VOCAB]);
    weights += tensor(&mut out, "decoder.ln.weight", &[s]);
    weights += tensor(&mut out, "decoder.ln.bias", &[s]);
    weights += block(&mut out, "decoder.blocks.0");
    for (name, ne) in [
        ("cross_attn_ln.weight", vec![s]),
        ("cross_attn_ln.bias", vec![s]),
        ("cross_attn.query.weight", vec![s, s]),
        ("cross_attn.query.bias", vec![s]),
        ("cross_attn.key.weight", vec![s, s]),
        ("cross_attn.value.weight", vec![s, s]),
        ("cross_attn.value.bias", vec![s]),
        ("cross_attn.out.weight", vec![s, s]),
        ("cross_attn.out.bias", vec![s]),
    ] {
        weights += tensor(&mut out, &format!("decoder.blocks.0.{name}"), &ne);
    }
    out.flush().unwrap();
    weights
}

/// Synthetisches Modell in einem temporären Verzeichnis.
pub struct SyntheticModel {
    _dir: tempfile::TempDir,
    pub path: PathBuf,
    pub weight_bytes: u64,
}

pub fn synthetic_model(dims: Dims) -> SyntheticModel {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ggml-synthetic.bin");
    let weight_bytes = write_model(&path, dims);
    SyntheticModel {
        _dir: dir,
        path,
        weight_bytes,
    }
}

/// Testsignal: 2 s Töne, 16 kHz mono.
pub fn tone(secs: usize) -> Vec<f32> {
    beton_voice::bench_signal(secs)
}

/// Offene Sockets dieses Prozesses (Unix): Dateideskriptoren unter `/dev/fd`, deren Ziel ein
/// Socket ist. `stat` auf `/dev/fd/N` liefert unter Linux und macOS die Angaben des
/// Deskriptors selbst.
#[cfg(unix)]
pub fn open_sockets() -> std::collections::BTreeSet<String> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
    let mut out = std::collections::BTreeSet::new();
    let Ok(dir) = std::fs::read_dir("/dev/fd") else {
        return out;
    };
    for entry in dir.flatten() {
        let Ok(meta) = std::fs::metadata(entry.path()) else {
            continue;
        };
        if meta.file_type().is_socket() {
            out.insert(format!(
                "{}:{}",
                entry.file_name().to_string_lossy(),
                meta.ino()
            ));
        }
    }
    out
}

/// Gemeinsamer Puffer für mitgeschnittene Logs.
#[derive(Clone, Default)]
pub struct LogBuffer(pub std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl LogBuffer {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// Wortfehlerrate (Levenshtein auf Wörtern, ohne Groß-/Kleinschreibung und Satzzeichen):
/// (Ersetzungen + Löschungen + Einfügungen) ÷ Wörter der Referenz.
pub fn word_error_rate(reference: &str, hypothesis: &str) -> f64 {
    let norm = |s: &str| -> Vec<String> {
        s.split_whitespace()
            .map(|w| {
                w.chars()
                    .filter(|c| c.is_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>()
            })
            .filter(|w| !w.is_empty())
            .collect()
    };
    let r = norm(reference);
    let h = norm(hypothesis);
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut cur = vec![i + 1; h.len() + 1];
        for (j, hw) in h.iter().enumerate() {
            let sub = prev[j] + usize::from(rw != hw);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[h.len()] as f64 / r.len() as f64
}
