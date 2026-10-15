//! QA-009: Egress-Regel-Parser und -Matcher (PRX-003), Host- und Pfad-Normalisierung,
//! Passthrough-Liste (PRX-005).
//!
//! Gefuzzt: `RuleSet::compile`, `RuleSet::decide`, `RuleSet::may_connect`,
//! `RuleSet::rule_hint`, `Host::parse`, `normalize_path`, `Passthrough::parse`.
//!
//! Eingabe (Zeilen): `R <regel>`, `P <passthrough-eintrag>`,
//! `Q <METHODE> <http|https> <host> <port> <pfad>`; andere Zeilen gelten als Regel.
//!
//! Invarianten:
//! - `Host::parse` ist idempotent über `Display` (klein, ohne Punkt am Ende);
//! - `normalize_path` ist idempotent und liefert kanonische Pfade (kein `//`, `.`, `..`,
//!   keine Steuerzeichen, Leerzeichen oder Backslashes);
//! - `decide` ist deterministisch und unabhängig von der Reihenfolge der Regeln (Deny vor
//!   Allow), `Allow` nennt eine Allow-Regel, `DeniedByRule` eine Deny-Regel aus der Eingabe;
//! - was `decide` erlaubt, besteht auch die CONNECT-Vorprüfung `may_connect` (sonst wäre
//!   ein erlaubter HTTPS-Request unerreichbar bzw. die Vorprüfung zu weit);
//! - `TRACE`/`CONNECT` werden immer abgelehnt;
//! - ein `rule_hint` (für eine der Methoden aus `METHODS`), der sich kompilieren lässt,
//!   erlaubt den abgelehnten Request;
//! - `Passthrough::parse` ist idempotent über `entries()` und enthält nie IP-Literale.

#![no_main]

use beton_proxy::{
    Decision, DenyReason, Host, Passthrough, Request, RuleSet, Scheme, normalize_path,
};
use libfuzzer_sys::fuzz_target;

struct Query {
    method: String,
    scheme: Scheme,
    host: Host,
    port: u16,
    path: String,
}

fn check_host(raw: &str) -> Option<Host> {
    let host = Host::parse(raw).ok()?;
    let shown = host.to_string();
    assert_eq!(
        Host::parse(&shown).as_ref(),
        Ok(&host),
        "Host::parse nicht idempotent: {raw:?} → {shown:?}"
    );
    if let Host::Domain(d) = &host {
        assert!(!d.ends_with('.') && d == &d.to_ascii_lowercase(), "{d:?}");
    }
    Some(host)
}

fn check_path(raw: &str) -> Option<String> {
    let path = normalize_path(raw).ok()?;
    assert_eq!(
        normalize_path(&path).as_ref(),
        Ok(&path),
        "normalize_path nicht idempotent: {raw:?}"
    );
    assert!(path.starts_with('/'), "{path:?}");
    assert!(!path.contains("//"), "{path:?}");
    for seg in path.split('/') {
        assert!(seg != "." && seg != "..", "Punkt-Segment in {path:?}");
    }
    assert!(
        path.bytes().all(|b| b.is_ascii_graphic() && b != b'\\'),
        "unzulässiges Zeichen in {path:?}"
    );
    Some(path)
}

fn decide(set: &RuleSet, q: &Query) -> Decision {
    set.decide(&Request {
        method: &q.method,
        scheme: q.scheme,
        host: &q.host,
        port: q.port,
        path: &q.path,
    })
}

fuzz_target!(|data: &[u8]| {
    let mut rules = Vec::new();
    let mut passthrough = Vec::new();
    let mut queries = Vec::new();
    for line in beton_fuzz::lines(data) {
        if let Some(r) = line.strip_prefix("R ") {
            rules.push(r.to_owned());
        } else if let Some(p) = line.strip_prefix("P ") {
            passthrough.push(p.to_owned());
        } else if let Some(q) = line.strip_prefix("Q ") {
            let parts: Vec<&str> = q.splitn(5, ' ').collect();
            let [method, scheme, host, port, path] = parts.as_slice() else {
                continue;
            };
            let scheme = if *scheme == "http" {
                Scheme::Http
            } else {
                Scheme::Https
            };
            let (Some(host), Some(path)) = (check_host(host), check_path(path)) else {
                continue;
            };
            let port = port.parse().unwrap_or(scheme.default_port());
            queries.push(Query {
                method: (*method).to_owned(),
                scheme,
                host,
                port,
                path,
            });
        } else if !line.is_empty() {
            rules.push(line);
        }
    }
    if queries.len() > 16 || rules.len() > 64 {
        return;
    }

    if let Ok(p) = Passthrough::parse(&passthrough) {
        let entries = p.entries();
        assert_eq!(
            Passthrough::parse(&entries).map(|q| q.entries()),
            Ok(entries.clone()),
            "Passthrough nicht idempotent"
        );
        for q in &queries {
            if p.contains(&q.host, q.port) {
                assert!(
                    matches!(q.host, Host::Domain(_)),
                    "IP-Literal im Passthrough"
                );
            }
        }
    }

    let Ok(set) = RuleSet::compile(&rules) else {
        return;
    };
    let reversed: Vec<String> = rules.iter().rev().cloned().collect();
    let reversed = RuleSet::compile(&reversed).expect("dieselben Regeln in anderer Reihenfolge");
    for q in &queries {
        let d = decide(&set, q);
        assert_eq!(d, decide(&set, q), "decide nicht deterministisch");
        assert_eq!(
            d.is_allowed(),
            decide(&reversed, q).is_allowed(),
            "Entscheidung hängt von der Reihenfolge der Regeln ab"
        );
        if q.method == "TRACE" || q.method == "CONNECT" {
            assert!(
                matches!(
                    d,
                    Decision::Deny {
                        reason: DenyReason::MethodNotAllowed,
                        ..
                    }
                ),
                "{} erlaubt",
                q.method
            );
        }
        match &d {
            Decision::Allow { rule } => {
                assert!(rules.contains(rule) && !rule.starts_with('!'), "{rule:?}");
                assert!(
                    set.may_connect(q.scheme, &q.host, q.port),
                    "erlaubt, aber CONNECT-Vorprüfung lehnt ab: {} {:?} {}:{}{}",
                    q.method,
                    q.scheme,
                    q.host,
                    q.port,
                    q.path
                );
            }
            Decision::Deny {
                reason: DenyReason::DeniedByRule,
                rule,
            } => {
                let rule = rule.as_ref().expect("Deny-Regel benannt");
                assert!(rules.contains(rule) && rule.starts_with('!'), "{rule:?}");
            }
            Decision::Deny {
                reason: DenyReason::NoMatchingRule,
                ..
            } => {
                let req = Request {
                    method: &q.method,
                    scheme: q.scheme,
                    host: &q.host,
                    port: q.port,
                    path: &q.path,
                };
                // Nur für Methoden, die Regeln kennen: Für andere (auch `!POST`, `!` ist ein
                // gültiges Token-Zeichen) gibt es keine Allow-Regel.
                let hint = RuleSet::rule_hint(&req);
                if let Some(h) = beton_proxy::rules::METHODS
                    .contains(&q.method.as_str())
                    .then(|| RuleSet::compile(&[hint.as_str()]).ok())
                    .flatten()
                {
                    assert!(
                        h.decide(&req).is_allowed(),
                        "rule_hint `{hint}` erlaubt den Request nicht"
                    );
                }
            }
            Decision::Deny { .. } => {}
        }
    }
});
