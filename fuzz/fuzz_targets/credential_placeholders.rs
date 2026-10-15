//! QA-009: Credential-Platzhalter-Ersetzung (PRX-006).
//!
//! Gefuzzt: `credentials::find_placeholders`, `Placeholder::parse` und die Ersetzung im
//! Request (`Credentials::plan` + `Plan::apply` über `beton_proxy::fuzzing`) mit festen
//! Bindings verschiedener Typen.
//!
//! Eingabe (Zeilen): `S http|https`, `T <host>`, `P <pfad>`, `Q <query>`,
//! `H <name>: <wert>`; andere Zeilen werden zum Wert eines Headers `x-fuzz`. `{A}`, `{B}`,
//! `{G}`, `{T}`, `{X}` werden zu den Platzhaltern der Bindings `api`, `basic`, `github`,
//! `git` bzw. einem unbekannten.
//!
//! Invarianten („Platzhalter nie an fremde Hosts“):
//! - `find_placeholders` findet genau die Stellen, an denen `Placeholder::parse` greift;
//! - über `http://` wird nie ein Wert eingesetzt;
//! - der Wert eines Bindings erscheint im Ergebnis nur, wenn der Host zum Binding gehört
//!   (auch im dekodierten `Authorization: Basic`);
//! - nach der Ersetzung steht in Header-Werten, Pfad und Query kein Platzhalter mehr;
//! - mit eingesetztem Wert trägt der Request `Accept-Encoding: identity` (PRX-010);
//! - Ablehnungen sind 403 (`credential_unknown`, `credential_host_mismatch`) oder 502.

#![no_main]

use std::sync::Arc;

use base64::Engine as _;
use beton_proxy::credentials::{BoxFuture, PLACEHOLDER_LEN, find_placeholders};
use beton_proxy::fuzzing::{CredentialOutcome, substitute_credentials};
use beton_proxy::{
    Binding, CredentialAudit, CredentialKind, CredentialSource, Credentials, Host, Placeholder,
    Scheme, SecretBytes,
};
use http::{HeaderMap, HeaderName, HeaderValue};
use libfuzzer_sys::fuzz_target;

/// Name, Typ, Hosts, Benutzer, Platzhalter-Zeichen, Wert (nur Großbuchstaben und Ziffern).
type Def = (
    &'static str,
    CredentialKind,
    &'static [&'static str],
    Option<&'static str>,
    char,
    &'static str,
);

const DEFS: [Def; 4] = [
    (
        "api",
        CredentialKind::Bearer,
        &["api.test", "*.api.test"],
        None,
        'a',
        "VALUEAPI0BEARER0SECRET",
    ),
    (
        "basic",
        CredentialKind::Basic,
        &["basic.test"],
        Some("user"),
        'b',
        "VALUEBASIC0PASSWORD",
    ),
    // Standard-Hosts von GitHub; implizit.
    (
        "github",
        CredentialKind::Github,
        &[],
        None,
        'g',
        "VALUEGITHUB0TOKEN",
    ),
    (
        "git",
        CredentialKind::GitHttps,
        &["git.test"],
        Some("git"),
        't',
        "VALUEGIT0HTTPS0PASSWORD",
    ),
];

fn placeholder(c: char) -> String {
    format!("bt_cred_{}", c.to_string().repeat(32))
}

struct Fixed;

impl CredentialSource for Fixed {
    fn value<'a>(&'a self, binding: &'a str) -> BoxFuture<'a, Result<SecretBytes, String>> {
        let v = DEFS.iter().find(|d| d.0 == binding).map(|d| d.5);
        Box::pin(async move {
            v.map(|v| SecretBytes::new(v.as_bytes().to_vec()))
                .ok_or_else(|| "unbekannt".to_owned())
        })
    }
}

impl CredentialAudit for Fixed {
    fn used<'a>(&'a self, _binding: &'a str, _host: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn misuse<'a>(
        &'a self,
        _binding: Option<&'a str>,
        _host: &'a str,
        _reason: &'a str,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

fn credentials() -> Credentials {
    Credentials::new(
        DEFS.iter()
            .map(|(name, kind, hosts, user, c, _)| Binding {
                name: (*name).into(),
                kind: *kind,
                hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
                username: user.map(str::to_owned),
                placeholder: Placeholder::parse(&placeholder(*c)).expect("Platzhalter"),
                implicit: matches!(kind, CredentialKind::Github | CredentialKind::GitHttps),
            })
            .collect(),
        Arc::new(Fixed),
        Arc::new(Fixed),
    )
    .expect("gültige Bindings")
}

fn binds(def: &Def, host: &Host) -> bool {
    let Host::Domain(d) = host else {
        return false;
    };
    let hosts: Vec<&str> = if def.2.is_empty() {
        def.1.default_hosts().to_vec()
    } else {
        def.2.to_vec()
    };
    hosts.iter().any(|h| match h.strip_prefix("*.") {
        Some(suffix) => d.len() > suffix.len() + 1 && d.ends_with(&format!(".{suffix}")),
        None => d == h,
    })
}

fn expand(s: &str) -> String {
    s.replace("{A}", &placeholder('a'))
        .replace("{B}", &placeholder('b'))
        .replace("{G}", &placeholder('g'))
        .replace("{T}", &placeholder('t'))
        .replace("{X}", &placeholder('x'))
}

/// Wo der Proxy Platzhalter ersetzt und ein Upstream Werte sehen würde: Header-Werte (auch
/// dekodiertes `Authorization: Basic`), Pfad, Query. Header-Namen ersetzt der Proxy nicht.
fn visible(headers: &HeaderMap, path: &str, query: Option<&str>) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, value) in headers {
        out.extend_from_slice(value.as_bytes());
        out.push(b'\n');
        // Wie der Proxy: nur `Authorization` wird als Basic dekodiert.
        let v = value.as_bytes();
        if name == http::header::AUTHORIZATION
            && v.len() > 6
            && v[..6].eq_ignore_ascii_case(b"basic ")
            && let Ok(decoded) =
                base64::engine::general_purpose::STANDARD.decode(v[6..].trim_ascii())
        {
            out.extend_from_slice(&decoded);
            out.push(b'\n');
        }
    }
    out.extend_from_slice(path.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(query.unwrap_or_default().as_bytes());
    out
}

fn check_find(text: &[u8]) {
    let found: Vec<usize> = find_placeholders(text)
        .into_iter()
        .map(|(i, _)| i)
        .collect();
    let naive: Vec<usize> = (0..text.len().saturating_sub(PLACEHOLDER_LEN - 1))
        .filter(|i| {
            std::str::from_utf8(&text[*i..*i + PLACEHOLDER_LEN])
                .ok()
                .and_then(Placeholder::parse)
                .is_some()
        })
        .collect();
    assert_eq!(
        found, naive,
        "find_placeholders ≠ Placeholder::parse an jeder Stelle"
    );
}

fuzz_target!(|data: &[u8]| {
    let mut scheme = Scheme::Https;
    let mut host = "api.test".to_owned();
    let mut path = "/".to_owned();
    let mut query = None;
    let mut headers = HeaderMap::new();
    for line in beton_fuzz::lines(data) {
        let line = expand(&line);
        check_find(line.as_bytes());
        if let Some(s) = line.strip_prefix("S ") {
            scheme = if s == "http" {
                Scheme::Http
            } else {
                Scheme::Https
            };
        } else if let Some(h) = line.strip_prefix("T ") {
            h.clone_into(&mut host);
        } else if let Some(p) = line.strip_prefix("P ") {
            p.clone_into(&mut path);
        } else if let Some(q) = line.strip_prefix("Q ") {
            query = Some(q.to_owned());
        } else if let Some(h) = line.strip_prefix("H ") {
            let Some((name, value)) = h.split_once(':') else {
                continue;
            };
            if let (Ok(n), Ok(v)) = (
                HeaderName::from_bytes(name.trim().as_bytes()),
                HeaderValue::from_bytes(value.trim_start().as_bytes()),
            ) {
                headers.append(n, v);
            }
        } else if let Ok(v) = HeaderValue::from_bytes(line.as_bytes()) {
            headers.append("x-fuzz", v);
        }
    }
    let Ok(host) = Host::parse(&host) else {
        return;
    };
    // Werte, die schon in der Eingabe stehen, kann der Client selbst geschickt haben.
    let input = visible(&headers, &path, query.as_deref());
    if DEFS
        .iter()
        .any(|d| beton_fuzz::contains(&input, d.5.as_bytes()))
    {
        return;
    }
    let creds = credentials();
    let outcome = beton_fuzz::ready(substitute_credentials(
        &creds,
        scheme,
        &host,
        &path,
        query.as_deref(),
        headers,
    ));
    match outcome {
        CredentialOutcome::Untouched => {}
        CredentialOutcome::Refused { status, error } => {
            assert!(
                matches!(
                    (status, error),
                    (403, "credential_unknown" | "credential_host_mismatch")
                        | (502, "credential_source_error")
                ),
                "unerwartete Ablehnung {status} {error}"
            );
        }
        CredentialOutcome::Applied(out) => {
            assert_eq!(scheme, Scheme::Https, "Wert über Klartext eingesetzt");
            let seen = visible(&out.headers, &out.path, out.query.as_deref());
            for def in &DEFS {
                if !binds(def, &host) {
                    assert!(
                        !beton_fuzz::contains(&seen, def.5.as_bytes()),
                        "Wert von `{}` an fremden Host {host}",
                        def.0
                    );
                }
            }
            if let Some((pos, p)) = find_placeholders(&seen).first() {
                let line = seen[..*pos]
                    .split(|b| *b == b'\n')
                    .next_back()
                    .unwrap_or_default();
                panic!(
                    "Platzhalter {p:?} nach der Ersetzung übrig, nach {:?}",
                    String::from_utf8_lossy(line)
                );
            }
            assert_eq!(
                out.headers
                    .get(http::header::ACCEPT_ENCODING)
                    .map(HeaderValue::as_bytes),
                Some(&b"identity"[..]),
                "Accept-Encoding nicht auf identity gesetzt"
            );
        }
    }
});
