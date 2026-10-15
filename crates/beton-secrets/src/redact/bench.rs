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

/// Zahl der `main`-Messungen, deren Median die Basis bildet.
const HISTORY_LEN: usize = 5;

/// Liest die Messreihe aus einer Basis-Datei (höchstens die letzten [`HISTORY_LEN`] Werte).
fn baseline_history(_text: &str) -> Result<Vec<f64>, String> {
    unimplemented!()
}

/// Median der Messreihe; `None` bei leerer Reihe.
fn median(_values: &[f64]) -> Option<f64> {
    unimplemented!()
}

/// Hängt eine neue Messung an und behält die letzten [`HISTORY_LEN`] Werte.
fn advance(_history: &[f64], _value: f64) -> Vec<f64> {
    unimplemented!()
}

/// Inhalt der Datei für `BETON_BENCH_OUT`.
fn baseline_json(_value: f64, _history: &[f64]) -> String {
    unimplemented!()
}

/// Vergleicht eine Messung mit dem Median der Basis.
fn check_regression(_value: f64, _history: &[f64]) -> Result<f64, String> {
    unimplemented!()
}

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

#[test]
fn sec_013_ac3_old_baseline_without_history_counts_as_one_value() {
    assert_eq!(
        baseline_history(r#"{"mb_per_s":300.1}"#).unwrap(),
        vec![300.1]
    );
    assert_eq!(
        median(&baseline_history(r#"{"mb_per_s":300.1}"#).unwrap()),
        Some(300.1)
    );
}

#[test]
fn sec_013_ac3_history_wins_over_current_value() {
    let text = r#"{"mb_per_s":210.0,"history":[202.4,300.1,210.0]}"#;
    assert_eq!(baseline_history(text).unwrap(), vec![202.4, 300.1, 210.0]);
}

#[test]
fn sec_013_ac3_reading_keeps_last_five_values() {
    let text = r#"{"mb_per_s":6.0,"history":[1,2,3,4,5,6]}"#;
    assert_eq!(
        baseline_history(text).unwrap(),
        vec![2.0, 3.0, 4.0, 5.0, 6.0]
    );
}

#[test]
fn sec_013_ac3_invalid_baseline_is_rejected() {
    for text in [
        "",
        "kein json",
        "{}",
        "[]",
        r#"{"mb_per_s":"200"}"#,
        r#"{"mb_per_s":0}"#,
        r#"{"mb_per_s":-5}"#,
        r#"{"history":[]}"#,
        r#"{"history":200}"#,
        r#"{"history":[200,"x"]}"#,
        r#"{"history":[200,0]}"#,
        r#"{"mb_per_s":200,"history":[]}"#,
    ] {
        assert!(baseline_history(text).is_err(), "{text:?} wurde akzeptiert");
    }
}

#[test]
fn sec_013_ac3_median_of_odd_and_even_series() {
    assert_eq!(median(&[]), None);
    assert_eq!(median(&[300.1]), Some(300.1));
    assert_eq!(median(&[200.0, 300.0]), Some(250.0));
    assert_eq!(median(&[300.1, 202.4, 210.0]), Some(210.0));
    assert_eq!(median(&[202.4, 300.1, 205.0, 199.0, 210.0]), Some(205.0));
    assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
}

#[test]
fn sec_013_ac3_single_outlier_does_not_set_the_baseline() {
    // Issue #296: Eine Messung mit 300 MB/s zwischen sonst ~200 MB/s.
    let history = [202.4, 300.1, 205.0, 199.0, 210.0];
    assert_eq!(check_regression(203.7, &history), Ok(205.0));
    // Toleranz unverändert 20 %: 0,8 × 205 = 164.
    assert!(check_regression(164.0, &history).is_ok());
    assert!(check_regression(163.9, &history).is_err());
}

#[test]
fn sec_013_ac3_regression_against_old_baseline_still_fails() {
    // Alte Basis-Datei: Median aus genau einem Wert, keine Abschwächung.
    let history = baseline_history(r#"{"mb_per_s":250.0}"#).unwrap();
    assert!(check_regression(200.0, &history).is_ok());
    assert!(check_regression(199.9, &history).is_err());
    assert!(check_regression(100.0, &[]).is_err());
}

#[test]
fn sec_013_ac3_recording_appends_and_keeps_last_five() {
    assert_eq!(advance(&[], 200.0), vec![200.0]);
    assert_eq!(advance(&[300.1], 202.4), vec![300.1, 202.4]);
    assert_eq!(
        advance(&[1.0, 2.0, 3.0, 4.0, 5.0], 6.0),
        vec![2.0, 3.0, 4.0, 5.0, 6.0]
    );
}

#[test]
fn sec_013_ac3_written_baseline_round_trips() {
    let history = advance(&baseline_history(r#"{"mb_per_s":300.1}"#).unwrap(), 202.4);
    let text = baseline_json(202.4, &history);
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["mb_per_s"].as_f64(), Some(202.4));
    assert_eq!(baseline_history(&text).unwrap(), vec![300.1, 202.4]);
}
