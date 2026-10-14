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

use crate::normalize::{Host, Scheme};

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

/// Kompilierte Regeln einer Proxy-Instanz.
#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    _rules: Vec<String>,
}

impl RuleSet {
    /// Kompiliert Regeln; die erste ungültige Regel ist ein Fehler.
    pub fn compile<S: AsRef<str>>(rules: &[S]) -> Result<Self, RuleError> {
        Ok(Self {
            _rules: rules.iter().map(|r| r.as_ref().to_owned()).collect(),
        })
    }

    /// Entscheidet über einen Request.
    pub fn decide(&self, _req: &Request<'_>) -> Decision {
        Decision::Deny {
            reason: DenyReason::NoMatchingRule,
            rule: None,
        }
    }

    /// Könnte irgendeine Allow-Regel Requests an `host:port` erlauben? Vorprüfung für CONNECT
    /// (PRX-003 AC5), bevor ein TLS-Handshake stattfindet.
    pub fn may_connect(&self, _scheme: Scheme, _host: &Host, _port: u16) -> bool {
        false
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
