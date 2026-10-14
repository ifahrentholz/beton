//! Normalisierung von Hosts und Pfaden (PRX-003, PRX-007).
//!
//! Regeln und Requests werden mit denselben Funktionen normalisiert, damit es keine
//! Parser-Differenzen zwischen Prüfung und Weiterleitung gibt.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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
    /// Normalisiert einen Host aus einer Regel, einer URL oder einem CONNECT: IPv6 in eckigen
    /// Klammern, IPv4 auch in Dezimal-, Hex- und Oktalform (wie WHATWG/`inet_aton`, damit
    /// `http://2130706433/` als `127.0.0.1` geprüft wird), Domains klein, ohne abschließenden
    /// Punkt und als Punycode.
    pub fn parse(raw: &str) -> Result<Self, InvalidHost> {
        let invalid = || InvalidHost(raw.chars().take(256).collect());
        if let Some(inner) = raw.strip_prefix('[') {
            let inner = inner.strip_suffix(']').ok_or_else(invalid)?;
            return inner
                .parse::<Ipv6Addr>()
                .map(|ip| Self::Ip(IpAddr::V6(ip)))
                .map_err(|_| invalid());
        }
        if raw.contains(':') {
            return raw
                .parse::<Ipv6Addr>()
                .map(|ip| Self::Ip(IpAddr::V6(ip)))
                .map_err(|_| invalid());
        }
        let trimmed = raw.strip_suffix('.').unwrap_or(raw);
        if trimmed.is_empty() || trimmed.len() > 253 {
            return Err(invalid());
        }
        let ascii = if trimmed.is_ascii() {
            trimmed.to_ascii_lowercase()
        } else {
            idna::domain_to_ascii(trimmed).map_err(|_| invalid())?
        };
        let labels: Vec<&str> = ascii.split('.').collect();
        let last = labels.last().copied().unwrap_or_default();
        if ends_in_number(last) {
            return parse_ipv4(&labels)
                .map(|ip| Self::Ip(IpAddr::V4(ip)))
                .ok_or_else(invalid);
        }
        let label_ok = |l: &str| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        };
        if ascii.len() > 253 || !labels.iter().all(|l| label_ok(l)) {
            return Err(invalid());
        }
        Ok(Self::Domain(ascii))
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

/// WHATWG: Endet ein Host auf eine Zahl, ist er eine IPv4-Adresse (oder ungültig).
fn ends_in_number(label: &str) -> bool {
    if !label.is_empty() && label.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    label
        .strip_prefix("0x")
        .is_some_and(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Eine Zahl im IPv4-Parser von WHATWG: `0x…` hex, führende `0` oktal, sonst dezimal.
fn ipv4_number(part: &str) -> Option<u64> {
    if part.is_empty() {
        return None;
    }
    let (digits, radix) = if let Some(hex) = part.strip_prefix("0x") {
        (hex, 16)
    } else if part.len() > 1 && part.starts_with('0') {
        (&part[1..], 8)
    } else {
        (part, 10)
    };
    if digits.is_empty() {
        return Some(0);
    }
    if digits.len() > 12 {
        return None;
    }
    u64::from_str_radix(digits, radix).ok()
}

fn parse_ipv4(labels: &[&str]) -> Option<Ipv4Addr> {
    if labels.is_empty() || labels.len() > 4 {
        return None;
    }
    let numbers: Vec<u64> = labels
        .iter()
        .map(|l| ipv4_number(l))
        .collect::<Option<_>>()?;
    let (last, init) = numbers.split_last()?;
    if init.iter().any(|n| *n > 255) {
        return None;
    }
    let rest_bits = 8 * (5 - numbers.len() as u32);
    if *last >= 1u64 << rest_bits {
        return None;
    }
    let mut value = *last;
    for (i, n) in init.iter().enumerate() {
        value += n << (8 * (3 - i as u32));
    }
    u32::try_from(value).ok().map(Ipv4Addr::from)
}

fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

fn hex_value(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

/// Prozent-Dekodierung unreservierter Zeichen; übrige Escapes in Großschreibung. `%2F`,
/// `%5C`, Backslash, Steuerzeichen und kaputte Escapes sind ungültig.
pub(crate) fn decode_escapes(raw: &str) -> Result<String, InvalidPath> {
    let invalid = |why: &str| InvalidPath(why.to_owned());
    let bytes = raw.as_bytes();
    let mut decoded = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\\' => return Err(invalid("Backslash im Pfad")),
            b'%' => {
                let (Some(hi), Some(lo)) = (
                    bytes.get(i + 1).copied().and_then(hex_value),
                    bytes.get(i + 2).copied().and_then(hex_value),
                ) else {
                    return Err(invalid("ungültiges Prozent-Escape"));
                };
                let v = hi * 16 + lo;
                match v {
                    b'/' | b'\\' => return Err(invalid("kodierter Pfadtrenner (%2F/%5C)")),
                    v if is_unreserved(v) => decoded.push(v as char),
                    v => decoded.push_str(&format!("%{v:02X}")),
                }
                i += 3;
            }
            b if b.is_ascii() && !b.is_ascii_control() && b != b' ' => {
                decoded.push(b as char);
                i += 1;
            }
            _ => return Err(invalid("unzulässiges Zeichen im Pfad")),
        }
    }
    Ok(decoded)
}

/// Normalisiert den Pfad eines Requests (ohne Query): Prozent-Dekodierung unreservierter
/// Zeichen (übrige Escapes in Großschreibung), `.`/`..` aufgelöst, `//` zusammengefasst.
/// `%2F`, `%5C`, Backslash, kaputte Escapes und Escape über die Wurzel sind ungültig (400).
pub fn normalize_path(raw: &str) -> Result<String, InvalidPath> {
    if !raw.starts_with('/') {
        return Err(InvalidPath("Pfad muss mit / beginnen".into()));
    }
    let decoded = decode_escapes(raw)?;
    let segments: Vec<&str> = decoded[1..].split('/').collect();
    let mut stack: Vec<&str> = Vec::with_capacity(segments.len());
    let mut trailing = false;
    let count = segments.len();
    for (idx, seg) in segments.into_iter().enumerate() {
        let last = idx + 1 == count;
        match seg {
            "" | "." => trailing = last,
            ".." => {
                if stack.pop().is_none() {
                    return Err(InvalidPath("Pfad verlässt die Wurzel".into()));
                }
                trailing = last;
            }
            s => {
                stack.push(s);
                trailing = false;
            }
        }
    }
    let mut out = String::with_capacity(decoded.len());
    out.push('/');
    out.push_str(&stack.join("/"));
    if trailing && !stack.is_empty() {
        out.push('/');
    }
    Ok(out)
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
