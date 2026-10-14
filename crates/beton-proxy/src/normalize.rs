//! Normalisierung von Hosts und Pfaden (PRX-003, PRX-007).
//!
//! Regeln und Requests werden mit denselben Funktionen normalisiert, damit es keine
//! Parser-Differenzen zwischen Prüfung und Weiterleitung gibt.

use std::fmt;
use std::net::IpAddr;

/// Schema eines Ziels: `https` (Default, Port 443) oder `http` (nur mit expliziter Regel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    pub fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }
}

/// Ein normalisierter Host: Domain (klein, Punycode, ohne abschließenden Punkt) oder IP.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Host {
    Domain(String),
    Ip(IpAddr),
}

/// Ungültiger Host.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("ungültiger Host: {0}")]
pub struct InvalidHost(pub String);

/// Ungültiger Pfad (400): `%2F`, `%5C`, Backslash, Escape über die Wurzel, kaputte Escapes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("ungültiger Pfad: {0}")]
pub struct InvalidPath(pub String);

impl Host {
    /// Normalisiert einen Host aus einer Regel, einer URL oder einem CONNECT.
    pub fn parse(_raw: &str) -> Result<Self, InvalidHost> {
        Err(InvalidHost("noch nicht umgesetzt".into()))
    }

    /// Form für URIs: IPv6 in eckigen Klammern.
    pub fn to_uri_host(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain(d) => f.write_str(d),
            Self::Ip(IpAddr::V4(ip)) => write!(f, "{ip}"),
            Self::Ip(IpAddr::V6(ip)) => write!(f, "[{ip}]"),
        }
    }
}

/// Normalisiert den Pfad eines Requests (ohne Query).
pub fn normalize_path(_raw: &str) -> Result<String, InvalidPath> {
    Err(InvalidPath("noch nicht umgesetzt".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(s: &str) -> String {
        Host::parse(s).unwrap().to_string()
    }

    #[test]
    fn prx_003_hosts_are_lowercased_without_trailing_dot() {
        assert_eq!(host("API.GitHub.com."), "api.github.com");
        assert_eq!(host("registry.npmjs.org"), "registry.npmjs.org");
        assert_eq!(host("under_score.internal"), "under_score.internal");
    }

    #[test]
    fn prx_003_idn_hosts_become_punycode() {
        assert_eq!(host("bücher.example"), "xn--bcher-kva.example");
        assert_eq!(host("xn--bcher-kva.example"), "xn--bcher-kva.example");
    }

    #[test]
    fn prx_003_invalid_hosts_are_rejected() {
        for bad in [
            "",
            ".",
            "a..b",
            "-a.example",
            "a b.example",
            "a/b",
            "a:b",
            "[::1",
            "[zz::1]",
            "*.example.com",
            "1.2.3.4.5",
            "999.1.1.1",
            "0x1g.example.123",
            &format!("{}.example", "a".repeat(64)),
        ] {
            assert!(Host::parse(bad).is_err(), "{bad:?} müsste ungültig sein");
        }
    }

    #[test]
    fn prx_007_ac3_numeric_ipv4_forms_are_ip_literals() {
        // Wie WHATWG/inet_aton: Dezimal, Hex, Oktal und verkürzte Formen.
        assert_eq!(host("2130706433"), "127.0.0.1");
        assert_eq!(host("0x7f000001"), "127.0.0.1");
        assert_eq!(host("0177.0.0.1"), "127.0.0.1");
        assert_eq!(host("127.1"), "127.0.0.1");
        assert_eq!(host("10.0.258"), "10.0.1.2");
        assert_eq!(host("169.254.169.254."), "169.254.169.254");
        assert!(matches!(Host::parse("2130706433"), Ok(Host::Ip(_))));
    }

    #[test]
    fn prx_007_ac3_ipv6_literals_are_parsed() {
        assert_eq!(host("[::ffff:10.0.0.1]"), "[::ffff:10.0.0.1]");
        assert_eq!(host("[0:0:0:0:0:0:0:1]"), "[::1]");
        assert_eq!(host("::1"), "[::1]");
        assert_eq!(Host::parse("[::1]").unwrap().to_uri_host(), "[::1]");
    }

    #[test]
    fn prx_003_ac2_paths_are_normalized() {
        let n = |p: &str| normalize_path(p).unwrap();
        assert_eq!(n("/"), "/");
        assert_eq!(n("/repos/a/x"), "/repos/a/x");
        assert_eq!(n("/repos/a/../b/x"), "/repos/b/x");
        assert_eq!(n("/repos/./a//x"), "/repos/a/x");
        assert_eq!(n("/a/b/.."), "/a/");
        assert_eq!(n("/a/b/."), "/a/b/");
        assert_eq!(n("/a/"), "/a/");
        assert_eq!(n("/%61%62%2d%7E"), "/ab-~");
        assert_eq!(n("/a/%2e%2E/b"), "/b");
        assert_eq!(n("/a%20b/%c3%a4"), "/a%20b/%C3%A4");
    }

    #[test]
    fn prx_003_ac2_encoded_separators_and_escapes_are_invalid() {
        for bad in [
            "/repos/a%2F..%2Fb",
            "/repos/a%2f",
            "/a%5Cb",
            "/a%5cb",
            "/a\\b",
            "/../x",
            "/..",
            "/a/../../x",
            "/%2e%2e/x",
            "/a%",
            "/a%4",
            "/a%zz",
            "a/b",
            "",
            "*",
        ] {
            assert!(normalize_path(bad).is_err(), "{bad:?} müsste 400 ergeben");
        }
    }
}
