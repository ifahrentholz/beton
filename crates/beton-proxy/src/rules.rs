//! Egress-Regeln (PRX-003): `"[!]METHODS target [target …]"`, Default-Deny.
//!
//! ```text
//! rule      = [ "!" ] methods SP target *( SP target )
//! methods   = "*" / method *( "," method )        ; GET HEAD POST PUT PATCH DELETE OPTIONS
//! target    = [ "http://" ] host [ ":" port ] [ path-glob ]
//! host      = fqdn / "*." fqdn / ip-literal      ; "*.x.com" = Subdomains, NICHT x.com selbst
//! path-glob = "/" …                               ; "*" Segment, "**" beliebig tief, "?" ein Zeichen
//! ```
//!
//! Ohne passende Allow-Regel wird abgelehnt; passt eine Deny-Regel (`!`), wird abgelehnt,
//! auch wenn eine Allow-Regel passt. `TRACE` und `CONNECT` sind im Tunnel immer verboten.
//! Regeln werden beim Laden kompiliert; eine ungültige Regel ist ein Konfigurationsfehler.

use crate::normalize::{Host, Scheme, decode_escapes};

/// Die sieben Methoden, die Regeln kennen; `*` steht für alle.
pub const METHODS: [&str; 7] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];

/// Ungültige Regel (Konfigurationsfehler).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("ungültige Egress-Regel `{rule}`: {message}")]
pub struct RuleError {
    pub rule: String,
    pub message: String,
}

/// Ein geprüfter Request: Methode, Schema, Host, Port und **normalisierter** Pfad.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub method: &'a str,
    pub scheme: Scheme,
    pub host: &'a Host,
    pub port: u16,
    pub path: &'a str,
}

/// Warum abgelehnt wurde.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// Keine Allow-Regel passt (Default-Deny).
    NoMatchingRule,
    /// Eine Deny-Regel passt.
    DeniedByRule,
    /// `TRACE` oder `CONNECT` im Tunnel.
    MethodNotAllowed,
}

impl DenyReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoMatchingRule => "no_matching_rule",
            Self::DeniedByRule => "denied_by_rule",
            Self::MethodNotAllowed => "method_not_allowed",
        }
    }
}

/// Entscheidung über einen Request; `rule` ist der Text der auslösenden Regel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow {
        rule: String,
    },
    Deny {
        reason: DenyReason,
        rule: Option<String>,
    },
}

impl Decision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }
}

/// Host-Muster einer Regel.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HostPattern {
    /// Genau dieser Host (Domain oder IP).
    Exact(Host),
    /// `*.x.com`: beliebige Subdomain (auch mehrstufig), nicht `x.com` selbst. Gespeichert
    /// mit führendem Punkt (`.x.com`).
    Subdomains(String),
}

impl HostPattern {
    fn matches(&self, host: &Host) -> bool {
        match (self, host) {
            (Self::Exact(h), host) => h == host,
            (Self::Subdomains(suffix), Host::Domain(d)) => {
                d.len() > suffix.len() && d.ends_with(suffix.as_str())
            }
            (Self::Subdomains(_), Host::Ip(_)) => false,
        }
    }
}

/// Teil eines Glob-Segments.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Literal(String),
    /// `*`: beliebig viele Zeichen außer `/`.
    Star,
    /// `?`: genau ein Zeichen außer `/`.
    Question,
}

/// Ein Segment eines Pfad-Globs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// `**`: null oder mehr ganze Segmente.
    Any,
    Pattern(Vec<Token>),
}

#[derive(Debug, Clone)]
struct Target {
    scheme: Scheme,
    host: HostPattern,
    port: u16,
    path: Vec<Segment>,
}

#[derive(Debug, Clone)]
struct Rule {
    text: String,
    deny: bool,
    /// Bitmaske über [`METHODS`].
    methods: u8,
    targets: Vec<Target>,
}

/// Kompilierte Regeln einer Proxy-Instanz.
#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    rules: Vec<Rule>,
}

fn method_bit(method: &str) -> Option<u8> {
    METHODS.iter().position(|m| *m == method).map(|i| 1 << i)
}

fn err(rule: &str, message: impl Into<String>) -> RuleError {
    RuleError {
        rule: rule.to_owned(),
        message: message.into(),
    }
}

fn parse_methods(rule: &str, text: &str) -> Result<u8, RuleError> {
    if text == "*" {
        return Ok((1 << METHODS.len()) - 1);
    }
    let mut mask = 0;
    for m in text.split(',') {
        let bit = method_bit(m).ok_or_else(|| {
            err(
                rule,
                format!(
                    "unbekannte Methode `{m}` (erlaubt: *, {})",
                    METHODS.join(", ")
                ),
            )
        })?;
        mask |= bit;
    }
    Ok(mask)
}

fn parse_glob(rule: &str, glob: &str) -> Result<Vec<Segment>, RuleError> {
    let raw_segments: Vec<&str> = glob[1..].split('/').collect();
    let count = raw_segments.len();
    let mut out = Vec::with_capacity(count);
    for (i, seg) in raw_segments.into_iter().enumerate() {
        if seg.is_empty() && i + 1 != count {
            return Err(err(rule, "leeres Pfadsegment (`//`)"));
        }
        if seg == "**" {
            out.push(Segment::Any);
            continue;
        }
        if seg.contains("**") {
            return Err(err(rule, "`**` nur als ganzes Segment"));
        }
        // Escapes wie im Request-Pfad darstellen; Punkt-Segmente sind nicht erlaubt.
        let decoded = decode_escapes(seg).map_err(|e| err(rule, e.to_string()))?;
        if decoded == "." || decoded == ".." {
            return Err(err(rule, "Punkt-Segmente sind in Regeln nicht erlaubt"));
        }
        out.push(Segment::Pattern(tokenize(&decoded)));
    }
    Ok(out)
}

fn tokenize(seg: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut lit = String::new();
    for c in seg.chars() {
        match c {
            '*' | '?' => {
                if !lit.is_empty() {
                    tokens.push(Token::Literal(std::mem::take(&mut lit)));
                }
                tokens.push(if c == '*' {
                    Token::Star
                } else {
                    Token::Question
                });
            }
            c => lit.push(c),
        }
    }
    if !lit.is_empty() {
        tokens.push(Token::Literal(lit));
    }
    tokens
}

fn parse_target(rule: &str, text: &str) -> Result<Target, RuleError> {
    let (scheme, rest) = match text.split_once("://") {
        Some(("http", rest)) => (Scheme::Http, rest),
        Some((other, _)) => {
            return Err(err(
                rule,
                format!("Schema `{other}://` nicht erlaubt (Default https, sonst `http://`)"),
            ));
        }
        None => (Scheme::Https, text),
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/**"),
    };
    let (host_text, port_text) = if let Some(after) = authority.strip_prefix('[') {
        let close = after
            .find(']')
            .ok_or_else(|| err(rule, "`]` fehlt im IPv6-Literal"))?;
        let host = &authority[..close + 2];
        let rest = &after[close + 1..];
        match rest.strip_prefix(':') {
            Some(p) => (host, Some(p)),
            None if rest.is_empty() => (host, None),
            None => return Err(err(rule, "ungültiger Host")),
        }
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    let port = match port_text {
        None => scheme.default_port(),
        Some(p) => match p.parse::<u16>() {
            Ok(n) if n > 0 && p.bytes().all(|b| b.is_ascii_digit()) => n,
            _ => return Err(err(rule, format!("ungültiger Port `{p}`"))),
        },
    };
    let host = if let Some(suffix) = host_text.strip_prefix("*.") {
        match Host::parse(suffix) {
            Ok(Host::Domain(d)) => HostPattern::Subdomains(format!(".{d}")),
            _ => return Err(err(rule, format!("ungültiger Host `{host_text}`"))),
        }
    } else {
        HostPattern::Exact(
            Host::parse(host_text)
                .map_err(|_| err(rule, format!("ungültiger Host `{host_text}`")))?,
        )
    };
    Ok(Target {
        scheme,
        host,
        port,
        path: parse_glob(rule, path)?,
    })
}

fn parse_rule(text: &str) -> Result<Rule, RuleError> {
    let (deny, body) = match text.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let mut parts = body.split(' ');
    let methods = parts
        .next()
        .filter(|m| !m.is_empty())
        .ok_or_else(|| err(text, "Methoden fehlen"))?;
    let methods = parse_methods(text, methods)?;
    let targets = parts
        .map(|t| {
            if t.is_empty() {
                Err(err(text, "doppeltes Leerzeichen oder leeres Ziel"))
            } else {
                parse_target(text, t)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if targets.is_empty() {
        return Err(err(text, "Ziel fehlt (z. B. `GET api.github.com/**`)"));
    }
    Ok(Rule {
        text: text.to_owned(),
        deny,
        methods,
        targets,
    })
}

/// Zeichen eines Segment-Musters (für [`segment_matches`]).
enum Pat {
    Char(char),
    One,
    Star,
}

/// Passt ein Segment-Muster auf ein Pfadsegment? Greedy mit Rücksprung auf das letzte `*`:
/// höchstens `O(|Muster| · |Segment|)` Schritte statt exponentiellem Backtracking (PRX-003;
/// ein Agent bestimmt den Pfad).
fn segment_matches(tokens: &[Token], seg: &[char]) -> bool {
    let pat: Vec<Pat> = tokens
        .iter()
        .flat_map(|t| match t {
            Token::Literal(l) => l.chars().map(Pat::Char).collect::<Vec<_>>(),
            Token::Question => vec![Pat::One],
            Token::Star => vec![Pat::Star],
        })
        .collect();
    let (mut p, mut s) = (0, 0);
    // Position nach dem letzten `*` und die Segmentposition, ab der es zuletzt griff.
    let mut back: Option<(usize, usize)> = None;
    while s < seg.len() {
        match pat.get(p) {
            Some(Pat::Star) => {
                back = Some((p + 1, s));
                p += 1;
            }
            Some(Pat::One) => {
                p += 1;
                s += 1;
            }
            Some(Pat::Char(c)) if *c == seg[s] => {
                p += 1;
                s += 1;
            }
            _ => match back {
                // Das letzte `*` verschluckt ein Zeichen mehr.
                Some((bp, bs)) => {
                    back = Some((bp, bs + 1));
                    p = bp;
                    s = bs + 1;
                }
                None => return false,
            },
        }
    }
    pat[p..].iter().all(|x| matches!(x, Pat::Star))
}

/// Passt der Glob auf die Pfadsegmente? `**` verschluckt null oder mehr Segmente. Dynamische
/// Programmierung über die Glob-Segmente: höchstens `|Glob| · |Pfad|` Segmentvergleiche statt
/// exponentiellem Backtracking (PRX-003).
fn path_matches(glob: &[Segment], path: &[Vec<char>]) -> bool {
    // reach[j]: Die bisherigen Glob-Segmente passen auf die ersten `j` Pfadsegmente.
    let mut reach = vec![false; path.len() + 1];
    reach[0] = true;
    for g in glob {
        let mut next = vec![false; path.len() + 1];
        match g {
            Segment::Any => {
                let mut any = false;
                for (j, r) in reach.iter().enumerate() {
                    any |= *r;
                    next[j] = any;
                }
            }
            Segment::Pattern(tokens) => {
                for (j, seg) in path.iter().enumerate() {
                    if reach[j] && segment_matches(tokens, seg) {
                        next[j + 1] = true;
                    }
                }
            }
        }
        if !next.contains(&true) {
            return false;
        }
        reach = next;
    }
    reach[path.len()]
}

impl Target {
    fn matches(&self, req: &Request<'_>, path: &[Vec<char>]) -> bool {
        self.scheme == req.scheme
            && self.port == req.port
            && self.host.matches(req.host)
            && path_matches(&self.path, path)
    }
}

impl Rule {
    fn matches(&self, bit: u8, req: &Request<'_>, path: &[Vec<char>]) -> bool {
        self.methods & bit != 0 && self.targets.iter().any(|t| t.matches(req, path))
    }
}

impl RuleSet {
    /// Kompiliert Regeln; die erste ungültige Regel ist ein Fehler.
    pub fn compile<S: AsRef<str>>(rules: &[S]) -> Result<Self, RuleError> {
        Ok(Self {
            rules: rules
                .iter()
                .map(|r| parse_rule(r.as_ref()))
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Entscheidet über einen Request; `req.path` muss normalisiert sein
    /// ([`crate::normalize_path`]).
    pub fn decide(&self, req: &Request<'_>) -> Decision {
        if req.method == "TRACE" || req.method == "CONNECT" {
            return Decision::Deny {
                reason: DenyReason::MethodNotAllowed,
                rule: None,
            };
        }
        let Some(bit) = method_bit(req.method) else {
            return Decision::Deny {
                reason: DenyReason::NoMatchingRule,
                rule: None,
            };
        };
        let segments: Vec<Vec<char>> = req
            .path
            .strip_prefix('/')
            .unwrap_or(req.path)
            .split('/')
            .map(|s| s.chars().collect())
            .collect();
        if let Some(rule) = self
            .rules
            .iter()
            .find(|r| r.deny && r.matches(bit, req, &segments))
        {
            return Decision::Deny {
                reason: DenyReason::DeniedByRule,
                rule: Some(rule.text.clone()),
            };
        }
        match self
            .rules
            .iter()
            .find(|r| !r.deny && r.matches(bit, req, &segments))
        {
            Some(rule) => Decision::Allow {
                rule: rule.text.clone(),
            },
            None => Decision::Deny {
                reason: DenyReason::NoMatchingRule,
                rule: None,
            },
        }
    }

    /// Könnte irgendeine Allow-Regel Requests an `host:port` erlauben? Vorprüfung für CONNECT
    /// (PRX-003 AC5), bevor ein TLS-Handshake stattfindet.
    pub fn may_connect(&self, scheme: Scheme, host: &Host, port: u16) -> bool {
        self.rules.iter().filter(|r| !r.deny).any(|r| {
            r.targets
                .iter()
                .any(|t| t.scheme == scheme && t.port == port && t.host.matches(host))
        })
    }

    /// Regel, die diesen Request erlauben würde (`rule_hint` in Fehlerantworten).
    pub fn rule_hint(req: &Request<'_>) -> String {
        let scheme = match req.scheme {
            Scheme::Http => "http://",
            Scheme::Https => "",
        };
        let port = if req.port == req.scheme.default_port() {
            String::new()
        } else {
            format!(":{}", req.port)
        };
        format!("{} {scheme}{}{port}{}", req.method, req.host, req.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(r: &[&str]) -> RuleSet {
        RuleSet::compile(r).unwrap()
    }

    fn decide(set: &RuleSet, method: &str, url: &str) -> Decision {
        let uri: http::Uri = url.parse().unwrap();
        let scheme = match uri.scheme_str() {
            Some("http") => Scheme::Http,
            _ => Scheme::Https,
        };
        let host = Host::parse(uri.host().unwrap()).unwrap();
        let path = crate::normalize_path(uri.path()).unwrap();
        set.decide(&Request {
            method,
            scheme,
            host: &host,
            port: uri.port_u16().unwrap_or(scheme.default_port()),
            path: &path,
        })
    }

    /// PRX-003: `*` und `**` backtracken nicht exponentiell; ein Agent kann mit einem Pfad
    /// gegen eine Regel mit vielen Wildcards keinen Proxy-Worker blockieren (Befund des
    /// Fuzz-Targets `egress_rules`, QA-009). Jede Entscheidung braucht unter 10 ms.
    #[test]
    fn prx_003_wildcards_do_not_backtrack_exponentially() {
        use std::time::{Duration, Instant};
        let cases = [
            (
                "GET api.test/*a*a*a*a*a*a*a*b/**",
                format!("https://api.test/{}", "a".repeat(60)),
            ),
            (
                "GET api.test/**/a/**/a/**/a/**/a/**/a/**/b",
                format!("https://api.test/{}c", "a/".repeat(80)),
            ),
        ];
        let n = cases.len();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (rule, url) in cases {
                let set = rules(&[rule]);
                let mut best = Duration::MAX;
                for _ in 0..3 {
                    let t = Instant::now();
                    let d = decide(&set, "GET", &url);
                    best = best.min(t.elapsed());
                    assert!(!d.is_allowed(), "{rule}");
                }
                tx.send((rule, best)).unwrap();
            }
        });
        for _ in 0..n {
            let (rule, took) = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("Matching hängt (exponentielles Backtracking)");
            assert!(took < Duration::from_millis(10), "{rule}: {took:?}");
        }
    }

    /// Die frühere, rekursive Implementierung als Referenz für die Semantik.
    fn reference_segment(tokens: &[Token], seg: &[char]) -> bool {
        match tokens.split_first() {
            None => seg.is_empty(),
            Some((Token::Literal(l), rest)) => {
                let l: Vec<char> = l.chars().collect();
                seg.starts_with(&l) && reference_segment(rest, &seg[l.len()..])
            }
            Some((Token::Question, rest)) => !seg.is_empty() && reference_segment(rest, &seg[1..]),
            Some((Token::Star, rest)) => {
                (0..=seg.len()).any(|i| reference_segment(rest, &seg[i..]))
            }
        }
    }

    fn reference_path(glob: &[Segment], path: &[Vec<char>]) -> bool {
        match glob.split_first() {
            None => path.is_empty(),
            Some((Segment::Any, rest)) => {
                (0..=path.len()).any(|i| reference_path(rest, &path[i..]))
            }
            Some((Segment::Pattern(tokens), rest)) => match path.split_first() {
                Some((seg, path_rest)) => {
                    reference_segment(tokens, seg) && reference_path(rest, path_rest)
                }
                None => false,
            },
        }
    }

    mod matcher_props {
        use super::*;
        use proptest::collection::vec;
        use proptest::prelude::*;

        fn token() -> impl Strategy<Value = Token> {
            prop_oneof![
                "[ab]{1,2}".prop_map(Token::Literal),
                Just(Token::Star),
                Just(Token::Question),
            ]
        }

        fn segment() -> impl Strategy<Value = Segment> {
            prop_oneof![
                1 => Just(Segment::Any),
                3 => vec(token(), 0..5).prop_map(Segment::Pattern),
            ]
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(2000))]

            /// PRX-003: Das lineare Matching entscheidet wie die frühere Implementierung.
            #[test]
            fn prx_003_matcher_agrees_with_the_backtracking_reference(
                glob in vec(segment(), 0..6),
                path in vec("[ab]{0,4}", 0..6),
            ) {
                let path: Vec<Vec<char>> = path.iter().map(|s| s.chars().collect()).collect();
                prop_assert_eq!(path_matches(&glob, &path), reference_path(&glob, &path));
                for g in &glob {
                    if let Segment::Pattern(tokens) = g {
                        for seg in &path {
                            prop_assert_eq!(
                                segment_matches(tokens, seg),
                                reference_segment(tokens, seg)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn prx_003_ac2_repo_glob() {
        let set = rules(&["GET api.github.com/repos/a/**"]);
        assert!(decide(&set, "GET", "https://api.github.com/repos/a/x").is_allowed());
        assert!(decide(&set, "GET", "https://api.github.com/repos/a").is_allowed());
        assert!(!decide(&set, "GET", "https://api.github.com/repos/a/../b/x").is_allowed());
        assert!(!decide(&set, "GET", "https://api.github.com/repos/ab").is_allowed());
        assert!(!decide(&set, "POST", "https://api.github.com/repos/a/x").is_allowed());
    }

    #[test]
    fn prx_003_ac3_wildcard_subdomains_only() {
        let set = rules(&["* *.example.com"]);
        assert!(decide(&set, "GET", "https://a.example.com/").is_allowed());
        assert!(decide(&set, "GET", "https://b.a.example.com/x").is_allowed());
        assert!(!decide(&set, "GET", "https://example.com/").is_allowed());
        assert!(!decide(&set, "GET", "https://example.com.evil.test/").is_allowed());
        assert!(!decide(&set, "GET", "https://aexample.com/").is_allowed());
    }

    #[test]
    fn prx_003_ac4_http_needs_explicit_http_rule() {
        let set = rules(&["* example.test"]);
        assert!(decide(&set, "GET", "https://example.test/").is_allowed());
        assert!(!decide(&set, "GET", "http://example.test/").is_allowed());
        let set = rules(&["* http://example.test"]);
        assert!(decide(&set, "GET", "http://example.test/x").is_allowed());
        assert!(!decide(&set, "GET", "https://example.test/x").is_allowed());
    }

    #[test]
    fn prx_003_deny_rules_win_and_name_the_rule() {
        let set = rules(&["* api.github.com/**", "!DELETE api.github.com/**"]);
        assert_eq!(
            decide(&set, "GET", "https://api.github.com/x"),
            Decision::Allow {
                rule: "* api.github.com/**".into()
            }
        );
        assert_eq!(
            decide(&set, "DELETE", "https://api.github.com/x"),
            Decision::Deny {
                reason: DenyReason::DeniedByRule,
                rule: Some("!DELETE api.github.com/**".into())
            }
        );
        assert_eq!(
            decide(&set, "GET", "https://other.test/"),
            Decision::Deny {
                reason: DenyReason::NoMatchingRule,
                rule: None
            }
        );
    }

    #[test]
    fn prx_003_trace_and_connect_are_never_allowed() {
        let set = rules(&["* example.test"]);
        for m in ["TRACE", "CONNECT"] {
            assert_eq!(
                decide(&set, m, "https://example.test/"),
                Decision::Deny {
                    reason: DenyReason::MethodNotAllowed,
                    rule: None
                }
            );
        }
        assert!(!decide(&set, "PROPFIND", "https://example.test/").is_allowed());
    }

    #[test]
    fn prx_003_ac5_connect_precheck() {
        let set = rules(&[
            "GET api.github.com/repos/**",
            "* *.example.test:8443",
            "* http://plain.test",
            "!* evil.test",
        ]);
        let h = |s: &str| Host::parse(s).unwrap();
        assert!(set.may_connect(Scheme::Https, &h("api.github.com"), 443));
        assert!(!set.may_connect(Scheme::Https, &h("api.github.com"), 8443));
        assert!(set.may_connect(Scheme::Https, &h("a.example.test"), 8443));
        assert!(!set.may_connect(Scheme::Https, &h("example.test"), 8443));
        assert!(!set.may_connect(Scheme::Https, &h("plain.test"), 443));
        assert!(!set.may_connect(Scheme::Https, &h("evil.test"), 443));
        assert!(!set.may_connect(Scheme::Https, &h("unknown.test"), 443));
        assert!(!RuleSet::default().may_connect(Scheme::Https, &h("x.test"), 443));
    }

    #[test]
    fn prx_003_invalid_rules_are_config_errors() {
        for bad in [
            "",
            "GET",
            "get example.test",
            "FOO example.test",
            "GET,,HEAD example.test",
            "GET example.test/a/**b",
            "GET https://example.test",
            "GET ftp://example.test",
            "GET example.test:0",
            "GET example.test:99999",
            "GET example.test:abc",
            "GET *.example.test.evil/a/../b",
            "GET example.test/a//b",
            "GET example.test/a%2Fb",
            "GET *example.test",
            "GET *",
            "GET a.*.example.test",
            "!! GET example.test",
            "GET  example.test",
        ] {
            let err = RuleSet::compile(&[bad]);
            assert!(err.is_err(), "{bad:?} müsste ungültig sein");
        }
        let err = RuleSet::compile(&["GET ok.test", "BAD x"]).unwrap_err();
        assert_eq!(err.rule, "BAD x");
    }

    #[test]
    fn prx_003_rule_hint_names_a_matching_rule() {
        let host = Host::parse("api.github.com").unwrap();
        let req = Request {
            method: "GET",
            scheme: Scheme::Https,
            host: &host,
            port: 443,
            path: "/repos/a",
        };
        let hint = RuleSet::rule_hint(&req);
        assert_eq!(hint, "GET api.github.com/repos/a");
        assert!(rules(&[&hint]).decide(&req).is_allowed());
        let req = Request {
            scheme: Scheme::Http,
            port: 8080,
            ..req
        };
        let hint = RuleSet::rule_hint(&req);
        assert_eq!(hint, "GET http://api.github.com:8080/repos/a");
        assert!(rules(&[&hint]).decide(&req).is_allowed());
    }
}
