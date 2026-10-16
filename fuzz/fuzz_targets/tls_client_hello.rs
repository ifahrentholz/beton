//! QA-009: TLS-`ClientHello`/SNI-Extraktion des Passthrough (PRX-005,
//! `beton_proxy::passthrough::client_hello_sni` über `beton_proxy::fuzzing`).
//!
//! Invarianten:
//! - kein Panic, gefundene SNI ist klein geschrieben;
//! - eine einmal gefundene SNI ändert sich durch weitere Bytes nicht (der Proxy entscheidet
//!   beim ersten vollständigen `ClientHello`, egal wie die Bytes in Stücken ankommen);
//! - Differenz zu rustls: Liest rustls aus denselben Bytes ein vollständiges `ClientHello` mit
//!   SNI, und findet auch der Proxy eine, sind beide gleich. Sonst könnte ein Client dem Proxy
//!   einen anderen Namen zeigen als einem TLS-Server.

#![no_main]

use beton_proxy::fuzzing::{ClientHello, client_hello_sni};
use libfuzzer_sys::fuzz_target;

fn normalize(s: &str) -> String {
    s.strip_suffix('.').unwrap_or(s).to_ascii_lowercase()
}

/// SNI, wie rustls sie aus `data` liest; `None`, wenn rustls kein vollständiges
/// `ClientHello` mit Namen sieht.
fn rustls_sni(data: &[u8]) -> Option<String> {
    let mut acceptor = rustls::server::Acceptor::default();
    let mut rd = data;
    while !rd.is_empty() {
        match acceptor.read_tls(&mut rd) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
    match acceptor.accept() {
        Ok(Some(accepted)) => accepted.client_hello().server_name().map(normalize),
        _ => None,
    }
}

fuzz_target!(|data: &[u8]| {
    let full = client_hello_sni(data);
    if let ClientHello::Sni(Some(sni)) = &full {
        // Ein abschließender Punkt wird entfernt (wie bei `Host::parse`); `a..` bleibt `a.`
        // und passt so nie auf einen gültigen CONNECT-Host.
        assert_eq!(
            sni,
            &sni.to_ascii_lowercase(),
            "SNI nicht klein geschrieben"
        );
    }
    // Präfixe: höchstens 64 Schnitte, gleichmäßig verteilt.
    let step = (data.len() / 64).max(1);
    for cut in (0..data.len()).step_by(step) {
        if let decided @ ClientHello::Sni(_) = client_hello_sni(&data[..cut]) {
            assert_eq!(
                decided, full,
                "SNI nach {cut} Bytes weicht vom Ergebnis für alle Bytes ab"
            );
        }
    }
    if let (ClientHello::Sni(Some(ours)), Some(theirs)) = (&full, rustls_sni(data)) {
        assert_eq!(ours, &theirs, "Proxy und rustls lesen verschiedene SNI");
    }
});
