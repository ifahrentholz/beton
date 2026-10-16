//! QA-009: HTTP/1.1-Requests durch den Egress-Proxy (PRX-001 … PRX-006, PRX-010).
//!
//! Der Proxy hat keinen eigenen HTTP-Parser; hyper parst. Gefuzzt wird deshalb der ganze
//! Weg durch eine echte Proxy-Instanz (siehe `beton_fuzz::e2e`): Das erste Byte wählt den
//! Modus, der Rest geht als Byte-Strom an den Proxy.
//!
//! - `0`/`1` (Seeds beginnen mit `0`): Klartext an den Proxy: absolute Form, CONNECT,
//!   Pipelining, kaputte Requests. `@@AUTH@@` wird zur gültigen `Proxy-Authorization`.
//! - `2`: HTTP/1.1 im TLS-MITM-Tunnel zu `api.test:443` (Tunnel-Prüfung von `Host`/
//!   `:authority`, Regeln, Credential-Ersetzung, Redaction, Upgrades).
//! - `3`: wie `2`, aber zu `api.test:8443` (Regel nur `GET,HEAD`).
//!
//! `{A}`/`{O}` werden zu den Platzhaltern der Bindings `api`/`other`.
//!
//! Invarianten: kein Hänger (5 s), kein echter Credential-Wert beim Client, ohne gültige
//! `Proxy-Authorization` nur Fehlerantworten; die Upstreams prüfen Regeln, Normalisierung,
//! Hop-by-Hop-Header und Credential-Grenzen (Panic bei Verletzung).

#![no_main]

use beton_fuzz::e2e::{self, AUTH_MARKER};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if e2e::mentions_secret(data) {
        return;
    }
    let Some((mode, rest)) = data.split_first() else {
        return;
    };
    let p = e2e::proxy();
    match mode % 4 {
        0 | 1 => {
            let authorized = beton_fuzz::contains(rest, AUTH_MARKER);
            let out = e2e::raw(p, &e2e::expand(p, rest));
            e2e::assert_no_secret(&out, "Klartext-Verbindung");
            if !authorized {
                // Ohne gültige Proxy-Authorization darf nichts die Regelprüfung oder einen
                // Upstream erreichen: nur 407 bzw. Parserfehler von hyper.
                for s in e2e::statuses(&out) {
                    assert!(
                        matches!(s, 400..=499 | 501 | 505) && s != 403 && s != 421,
                        "Antwort {s} ohne gültige Proxy-Authorization"
                    );
                }
            }
        }
        m => {
            let authority: &[u8] = if m == 2 {
                b"api.test:443"
            } else {
                b"api.test:8443"
            };
            let t = e2e::connect(
                p,
                authority,
                &e2e::expand(p, rest),
                Some(("api.test", true)),
            );
            assert_eq!(t.status, Some(200), "CONNECT zu einem erlaubten Ziel");
            e2e::assert_no_secret(&t.inner, "MITM-Tunnel");
        }
    }
});
