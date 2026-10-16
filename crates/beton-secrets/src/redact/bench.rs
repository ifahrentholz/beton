//! SEC-013 AC3: Durchsatz der Redaction bei 1 000 bekannten Werten, ≥ 50 MB/s pro Kern.
//!
//! Läuft im Release-Build in der CI (Job „test (linux)“):
//! `cargo test -p beton-secrets --release --lib sec_013_ac3 -- --ignored --nocapture`
//!
//! Regression (#296): `BETON_BENCH_BASELINE` ist die Basis-Datei von `main`
//! (`{"mb_per_s": …, "history": [ … ]}`, bis zu fünf Messungen; eine alte Datei ohne `history`
//! gilt als Reihe mit einem Wert). Ein Rückgang um mehr als 20 % gegenüber dem Median der Reihe
//! schlägt fehl. `BETON_BENCH_OUT` nimmt den gemessenen Wert und die um ihn fortgeschriebene
//! Reihe auf. Mit `BETON_BENCH_RECORD=1` (Pushes auf `main`) wird nur fortgeschrieben, nicht
//! verglichen; der Floor von 50 MB/s gilt immer.

use std::time::Instant;

use super::{KnownSecrets, KnownSource, Redactor};
use crate::value::SecretValue;

// Unit-Tests der Basis (Lesen, Median, Fortschreiben) in `bench/tests.rs`.
mod tests;

/// Mindestdurchsatz laut AC3.
const MIN_MB_PER_S: f64 = 50.0;
/// Erlaubter Rückgang gegenüber `main`.
const MAX_REGRESSION: f64 = 0.20;

/// Zahl der `main`-Messungen, deren Median die Basis bildet.
const HISTORY_LEN: usize = 5;

/// Liest die Messreihe aus einer Basis-Datei (höchstens die letzten [`HISTORY_LEN`] Werte).
///
/// Eine Datei im alten Format (`{"mb_per_s": …}` ohne `history`) gilt als Reihe mit einem Wert.
/// Alles andere als eine nicht leere Reihe positiver, endlicher Zahlen ist ein Fehler, damit
/// eine kaputte Basis den Vergleich nicht stillschweigend abschaltet.
fn baseline_history(text: &str) -> Result<Vec<f64>, String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("Basis ist kein JSON: {e}"))?;
    let values: Vec<&serde_json::Value> = match (json.get("history"), json.get("mb_per_s")) {
        (Some(history), _) => history
            .as_array()
            .ok_or("`history` ist kein Array")?
            .iter()
            .collect(),
        (None, Some(value)) => vec![value],
        (None, None) => return Err("Basis ohne `history` und `mb_per_s`".into()),
    };
    if values.is_empty() {
        return Err("`history` ist leer".into());
    }
    let mut history = values
        .into_iter()
        .map(|v| match v.as_f64() {
            Some(x) if x.is_finite() && x > 0.0 => Ok(x),
            _ => Err(format!("ungültiger Messwert in der Basis: {v}")),
        })
        .collect::<Result<Vec<f64>, String>>()?;
    let skip = history.len().saturating_sub(HISTORY_LEN);
    history.drain(..skip);
    Ok(history)
}

/// Median der Messreihe; `None` bei leerer Reihe.
fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    Some(if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    })
}

/// Hängt eine neue Messung an und behält die letzten [`HISTORY_LEN`] Werte.
fn advance(history: &[f64], value: f64) -> Vec<f64> {
    let mut next: Vec<f64> = history.to_vec();
    next.push(value);
    let skip = next.len().saturating_sub(HISTORY_LEN);
    next.drain(..skip);
    next
}

/// Inhalt der Datei für `BETON_BENCH_OUT`: aktueller Wert und fortgeschriebene Reihe.
fn baseline_json(value: f64, history: &[f64]) -> String {
    serde_json::json!({ "mb_per_s": value, "history": history }).to_string()
}

/// Vergleicht eine Messung mit dem Median der Basis; liefert den Median.
fn check_regression(value: f64, history: &[f64]) -> Result<f64, String> {
    let base = median(history).ok_or("Basis ohne Messwerte")?;
    if value >= base * (1.0 - MAX_REGRESSION) {
        Ok(base)
    } else {
        Err(format!(
            "Regression: {value:.1} MB/s gegenüber {base:.1} MB/s (Median der letzten {} \
             Messungen auf main) (> 20 %)",
            history.len()
        ))
    }
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

    // Bisherige Basis: Messreihe von `main`. Ohne Datei (erster Lauf, Cache leer) gibt es
    // nur den Floor; eine vorhandene, aber kaputte Datei schlägt fehl.
    let history = match std::env::var_os("BETON_BENCH_BASELINE") {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(text) => baseline_history(&text).unwrap(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => panic!("Basis {} nicht lesbar: {e}", path.display()),
        },
        None => Vec::new(),
    };
    let record = std::env::var_os("BETON_BENCH_RECORD").is_some_and(|v| v == "1");
    if record {
        // main: Basis fortschreiben, nicht gegen sich selbst vergleichen.
        println!(
            "Basis wird fortgeschrieben ({} Werte bisher)",
            history.len()
        );
    } else if !history.is_empty() {
        let series: Vec<String> = history.iter().map(|v| format!("{v:.1}")).collect();
        println!("Messreihe von main: [{}] MB/s", series.join(", "));
        let base = check_regression(best, &history).unwrap_or_else(|e| panic!("{e}"));
        println!("Basis von main (Median): {base:.1} MB/s");
    }
    if let Some(path) = std::env::var_os("BETON_BENCH_OUT") {
        let path = std::path::PathBuf::from(path);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(&path, baseline_json(best, &advance(&history, best))).unwrap();
    }
}
