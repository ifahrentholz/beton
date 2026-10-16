//! Gemeinsame Bausteine der Fuzz-Targets (QA-009).
//!
//! Die Targets selbst liegen unter `fuzz_targets/`, ihre Seed-Korpora unter `corpus/<target>/`.
//! Gebaut und gestartet wird mit `cargo +nightly fuzz run <target>`; welche Targets es gibt
//! und welche Pfade sie abdecken, steht in `xtask/src/fuzz.rs` (`cargo xtask fuzz-targets`).

pub mod e2e;

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

/// Führt ein Future aus, das nie wirklich wartet (Fake-Quellen und -Audits liefern sofort).
/// Bleibt es hängen, ist das ein Fehler des Harness bzw. des geprüften Codes.
pub fn ready<F: Future>(fut: F) -> F::Output {
    let mut fut = pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..1_000 {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
    }
    panic!("Future wartet, obwohl alle Quellen sofort antworten");
}

/// Zerlegt die Eingabe in Zeilen (verlustbehaftet nach UTF-8, ohne `\r`).
pub fn lines(data: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(data)
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned())
        .collect()
}

/// Kommt `needle` in `haystack` vor?
pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// Ersetzt alle Vorkommen von `from` durch `to`.
pub fn replace(data: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}
