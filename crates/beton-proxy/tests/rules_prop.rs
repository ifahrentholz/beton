//! Property-Tests für Parser, Matcher und Pfad-Normalisierung (PRX-003).
//!
//! Regeln werden strukturiert erzeugt, als Text gerendert und kompiliert; die Entscheidung
//! des Proxys muss für zufällige Requests mit einer unabhängigen Referenzimplementierung
//! übereinstimmen (Globs als reguläre Ausdrücke, Hosts als Zeichenkettenvergleich).

#![allow(clippy::unwrap_used)]

use beton_proxy::{Host, Request, RuleSet, Scheme, normalize_path};
use proptest::prelude::*;

const METHODS: [&str; 7] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
const REQUEST_METHODS: [&str; 10] = [
    "GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "TRACE", "CONNECT", "PROPFIND",
];
const LABELS: [&str; 5] = ["a", "b", "api", "x-1", "example"];
const SEGMENTS: [&str; 6] = ["a", "b", "ab", "repos", "v1", "x.json"];

#[derive(Debug, Clone)]
enum RefHost {
    Exact(String),
    Wild(String),
}

#[derive(Debug, Clone)]
struct RefTarget {
    http: bool,
    host: RefHost,
    port: Option<u16>,
    glob: Option<String>,
}

#[derive(Debug, Clone)]
struct RefRule {
    deny: bool,
    /// `None` = `*`.
    methods: Option<Vec<&'static str>>,
    targets: Vec<RefTarget>,
}

impl RefRule {
    fn render(&self) -> String {
        let mut s = String::new();
        if self.deny {
            s.push('!');
        }
        match &self.methods {
            None => s.push('*'),
            Some(m) => s.push_str(&m.join(",")),
        }
        for t in &self.targets {
            s.push(' ');
            if t.http {
                s.push_str("http://");
            }
            match &t.host {
                RefHost::Exact(h) => s.push_str(h),
                RefHost::Wild(h) => {
                    s.push_str("*.");
                    s.push_str(h);
                }
            }
            if let Some(p) = t.port {
                s.push_str(&format!(":{p}"));
            }
            if let Some(g) = &t.glob {
                s.push_str(g);
            }
        }
        s
    }
}

/// Referenz: Glob → regulärer Ausdruck über den ganzen Pfad.
fn glob_regex(glob: &str) -> regex::Regex {
    let mut re = String::from("^");
    for seg in glob[1..].split('/') {
        if seg == "**" {
            re.push_str("(?:/[^/]*)*");
            continue;
        }
        re.push('/');
        for c in seg.chars() {
            match c {
                '*' => re.push_str("[^/]*"),
                '?' => re.push_str("[^/]"),
                c => re.push_str(&regex::escape(&c.to_string())),
            }
        }
    }
    re.push('$');
    regex::Regex::new(&re).unwrap()
}

struct RefReq<'a> {
    method: &'a str,
    http: bool,
    host: &'a str,
    port: u16,
    path: &'a str,
}

fn ref_target_matches(t: &RefTarget, r: &RefReq<'_>) -> bool {
    let default_port = if t.http { 80 } else { 443 };
    if t.http != r.http || t.port.unwrap_or(default_port) != r.port {
        return false;
    }
    let host_ok = match &t.host {
        RefHost::Exact(h) => h == r.host,
        RefHost::Wild(h) => r.host.ends_with(&format!(".{h}")),
    };
    host_ok && glob_regex(t.glob.as_deref().unwrap_or("/**")).is_match(r.path)
}

fn ref_rule_matches(rule: &RefRule, r: &RefReq<'_>) -> bool {
    let method_ok = match &rule.methods {
        None => METHODS.contains(&r.method),
        Some(m) => m.contains(&r.method),
    };
    method_ok && rule.targets.iter().any(|t| ref_target_matches(t, r))
}

fn ref_decide(rules: &[RefRule], r: &RefReq<'_>) -> bool {
    if r.method == "TRACE" || r.method == "CONNECT" {
        return false;
    }
    if rules.iter().any(|x| x.deny && ref_rule_matches(x, r)) {
        return false;
    }
    rules.iter().any(|x| !x.deny && ref_rule_matches(x, r))
}

fn domain() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(LABELS.to_vec()), 1..3)
        .prop_map(|labels| format!("{}.test", labels.join(".")))
}

fn glob_segment() -> impl Strategy<Value = String> {
    prop_oneof![
        prop::sample::select(SEGMENTS.to_vec()).prop_map(str::to_owned),
        Just("*".to_owned()),
        Just("**".to_owned()),
        Just("a*".to_owned()),
        Just("?b".to_owned()),
        Just("v?".to_owned()),
        Just("*.json".to_owned()),
    ]
}

fn glob() -> impl Strategy<Value = Option<String>> {
    prop::option::of(
        prop::collection::vec(glob_segment(), 0..4).prop_map(|s| format!("/{}", s.join("/"))),
    )
}

fn target() -> impl Strategy<Value = RefTarget> {
    (
        any::<bool>(),
        any::<bool>(),
        domain(),
        prop::option::of(prop::sample::select(vec![80u16, 443, 8443])),
        glob(),
    )
        .prop_map(|(http, wild, host, port, glob)| RefTarget {
            http,
            host: if wild {
                RefHost::Wild(host)
            } else {
                RefHost::Exact(host)
            },
            port,
            glob,
        })
}

fn rule() -> impl Strategy<Value = RefRule> {
    (
        prop::bool::weighted(0.25),
        prop::option::weighted(0.7, prop::sample::subsequence(METHODS.to_vec(), 1..4)),
        prop::collection::vec(target(), 1..3),
    )
        .prop_map(|(deny, methods, targets)| RefRule {
            deny,
            methods,
            targets,
        })
}

fn path() -> impl Strategy<Value = String> {
    (
        prop::collection::vec(prop::sample::select(SEGMENTS.to_vec()), 0..5),
        any::<bool>(),
    )
        .prop_map(|(s, trailing)| {
            let mut p = format!("/{}", s.join("/"));
            if trailing && !s.is_empty() {
                p.push('/');
            }
            p
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Matcher gegen Referenzimplementierung.
    #[test]
    fn prx_003_matcher_agrees_with_reference(
        rules in prop::collection::vec(rule(), 0..5),
        requests in prop::collection::vec(
            (prop::sample::select(REQUEST_METHODS.to_vec()), any::<bool>(), domain(),
             prop::sample::select(vec![80u16, 443, 8443]), path()),
            1..8),
    ) {
        let texts: Vec<String> = rules.iter().map(RefRule::render).collect();
        let set = RuleSet::compile(&texts).map_err(|e| TestCaseError::fail(e.to_string()))?;
        for (method, http, host, port, path) in &requests {
            let h = Host::parse(host).unwrap();
            let scheme = if *http { Scheme::Http } else { Scheme::Https };
            let got = set.decide(&Request { method, scheme, host: &h, port: *port, path });
            let want = ref_decide(&rules, &RefReq { method, http: *http, host, port: *port, path });
            prop_assert_eq!(got.is_allowed(), want, "Regeln {:?}, Request {} {} {}:{}{}",
                texts, method, if *http { "http" } else { "https" }, host, port, path);
        }
    }

    /// Die CONNECT-Vorprüfung ist nie strenger als der Matcher: Was ein Request über HTTPS
    /// erlaubt bekäme, darf auch per CONNECT aufgebaut werden.
    #[test]
    fn prx_003_connect_precheck_covers_every_allowed_request(
        rules in prop::collection::vec(rule(), 0..5),
        method in prop::sample::select(METHODS.to_vec()),
        host in domain(),
        port in prop::sample::select(vec![443u16, 8443]),
        path in path(),
    ) {
        let texts: Vec<String> = rules.iter().map(RefRule::render).collect();
        let set = RuleSet::compile(&texts).unwrap();
        let h = Host::parse(&host).unwrap();
        let req = Request { method, scheme: Scheme::Https, host: &h, port, path: &path };
        if set.decide(&req).is_allowed() {
            prop_assert!(set.may_connect(Scheme::Https, &h, port));
        }
    }

    /// Der Parser stürzt bei beliebigen Eingaben nicht ab.
    #[test]
    fn prx_003_parser_never_panics(text in "[!*A-Z, a-z0-9./:?\\[\\]%-]{0,40}") {
        let _ = RuleSet::compile(&[text]);
    }

    /// Normalisierte Pfade sind idempotent und frei von Punkt-Segmenten, doppelten
    /// Schrägstrichen und kodierten Trennern.
    #[test]
    fn prx_003_normalized_paths_are_canonical(
        segs in prop::collection::vec(
            prop::sample::select(vec!["a", "b", ".", "..", "", "%2e", "%2E%2e", "%61", "x%20y", "%7e"]),
            0..8),
    ) {
        let raw = format!("/{}", segs.join("/"));
        if let Ok(n) = normalize_path(&raw) {
            prop_assert!(n.starts_with('/'));
            prop_assert!(!n.contains("//"), "{} -> {}", raw, n);
            for seg in n.split('/') {
                prop_assert!(seg != "." && seg != "..", "{} -> {}", raw, n);
            }
            prop_assert!(!n.to_ascii_lowercase().contains("%2e"));
            prop_assert!(!n.to_ascii_lowercase().contains("%2f"));
            prop_assert_eq!(normalize_path(&n).unwrap(), n);
        }
    }
}
