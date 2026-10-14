//! SEC-013 AC3: Durchsatz der Redaction bei 1 000 bekannten Werten, ≥ 50 MB/s pro Kern.
//!
//! Läuft im Release-Build in der CI (Job „test (linux)“):
//! `cargo test -p beton-secrets --release --lib sec_013_ac3 -- --ignored --nocapture`
//!
//! Regression: Ist `BETON_BENCH_BASELINE` eine Datei mit dem Wert von `main`
//! (`{"mb_per_s": …}`), schlägt ein Rückgang um mehr als 20 % fehl. `BETON_BENCH_OUT` nimmt
//! den gemessenen Wert für die nächste Basis auf.

use std::time::Instant;

use super::{KnownSecrets, KnownSource, Redactor};
use crate::value::SecretValue;

/// Mindestdurchsatz laut AC3.
const MIN_MB_PER_S: f64 = 50.0;
/// Erlaubter Rückgang gegenüber `main`.
const MAX_REGRESSION: f64 = 0.20;

/// Deterministischer Pseudozufall (xorshift64*), damit Läufe vergleichbar sind.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn token(&mut self, len: usize) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        (0..len)
            .map(|_| char::from(CHARS[(self.next() % CHARS.len() as u64) as usize]))
            .collect()
    }
}

/// Typischer Inhalt von Events und Logs: Quelltext, JSON, Tool-Ausgaben; dazwischen selten
/// ein bekannter Wert oder ein Token.
fn corpus(rng: &mut Rng, known: &[String], bytes: usize) -> Vec<String> {
    const LINES: &[&str] = &[
        "fn main() { let config = Config::load(\"settings.toml\")?; println!(\"{config:?}\"); }\n",
        "{\"type\":\"tool.call.completed\",\"payload\":{\"call_id\":\"call_01J\",\"status\":\"ok\",\"duration_ms\":42}}\n",
        "2026-10-14T10:00:00Z INFO beton_server: Runner gestartet session_id=ses_01J runner_id=run_01J\n",
        "    Compiling serde v1.0.228\n    Compiling tokio v1.48.0 (features: rt, macros)\n",
        "drwxr-xr-x  12 user  staff   384 Oct 14 10:00 crates\n-rw-r--r--   1 user  staff  2291 Oct 14 10:00 Cargo.toml\n",
        "Die Änderung betrifft drei Dateien; Tests laufen grün, der Benchmark liegt im Rahmen.\n",
        "diff --git a/src/lib.rs b/src/lib.rs\n@@ -1,4 +1,6 @@\n+use std::sync::Arc;\n",
    ];
    let mut chunks = Vec::new();
    let mut total = 0;
    while total < bytes {
        let mut chunk = String::with_capacity(64 * 1024);
        while chunk.len() < 64 * 1024 {
            let r = rng.next() % 1000;
            if r == 0 {
                chunk.push_str(&known[(rng.next() % known.len() as u64) as usize]);
                chunk.push('\n');
            } else if r == 1 {
                chunk.push_str("token=ghp_");
                chunk.push_str(&rng.token(36).replace(['-', '_'], "x"));
                chunk.push('\n');
            } else {
                chunk.push_str(LINES[(r % LINES.len() as u64) as usize]);
            }
        }
        total += chunk.len();
        chunks.push(chunk);
    }
    chunks
}

#[test]
#[ignore = "Benchmark; läuft im Release-Build (CI)"]
fn sec_013_ac3_throughput_with_1000_known_values() {
    let mut rng = Rng(0x00C0_FFEE_D00D_F00D);
    let known: Vec<String> = (0..1000).map(|_| rng.token(40)).collect();
    let values: Vec<SecretValue> = known
        .iter()
        .map(|k| SecretValue::from(k.as_str()))
        .collect();
    let redactor = Redactor::pending();
    redactor.install(KnownSecrets::build(&values).unwrap(), KnownSource::Store);
    let chunks = corpus(&mut rng, &known, 32 * 1024 * 1024);
    let bytes: usize = chunks.iter().map(String::len).sum();

    // Aufwärmen und Prüfen: kein bekannter Wert überlebt.
    for chunk in chunks.iter().take(8) {
        let out = redactor.redact_str(chunk).unwrap();
        assert!(known.iter().all(|k| !out.contains(k.as_str())));
    }
    let mut best = 0.0_f64;
    for _ in 0..5 {
        let start = Instant::now();
        let mut redacted = 0usize;
        for chunk in &chunks {
            redacted += redactor.redact_str(chunk).unwrap().len();
        }
        let secs = start.elapsed().as_secs_f64();
        assert!(redacted > 0);
        best = best.max(bytes as f64 / secs / 1_000_000.0);
    }
    println!("SEC-013 AC3: {best:.1} MB/s ({bytes} Bytes, 1 000 bekannte Werte, ein Kern)");
    assert!(
        best >= MIN_MB_PER_S,
        "{best:.1} MB/s liegt unter {MIN_MB_PER_S} MB/s"
    );

    if let Some(path) = std::env::var_os("BETON_BENCH_BASELINE")
        && let Ok(text) = std::fs::read_to_string(&path)
    {
        let baseline: serde_json::Value = serde_json::from_str(&text).unwrap();
        if let Some(base) = baseline["mb_per_s"].as_f64() {
            println!("Basis von main: {base:.1} MB/s");
            assert!(
                best >= base * (1.0 - MAX_REGRESSION),
                "Regression: {best:.1} MB/s gegenüber {base:.1} MB/s auf main (> 20 %)"
            );
        }
    }
    if let Some(path) = std::env::var_os("BETON_BENCH_OUT") {
        let path = std::path::PathBuf::from(path);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(&path, serde_json::json!({ "mb_per_s": best }).to_string()).unwrap();
    }
}
