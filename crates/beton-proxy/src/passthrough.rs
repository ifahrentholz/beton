//! Passthrough für Vendor-Hosts (PRX-005): CONNECT ohne TLS-Terminierung.
//!
//! Damit beton Subscription-Tokens nie sieht (HAR-015, ADR-0005), tunnelt eine Instanz die
//! Vendor-Hosts ihres Harness unverändert zum Upstream. Geprüft werden nur Host und Port
//! (Liste), die Private-IP-Sperre nach eigener Auflösung (PRX-007) und dass die SNI im
//! `ClientHello` dem CONNECT-Host entspricht; sonst wird die Verbindung getrennt, bevor ein
//! Byte den Upstream erreicht. Der Proxy parst kein HTTP im Tunnel, protokolliert keine Header
//! und injiziert nie Credentials.

use crate::normalize::Host;

/// Höchstgröße des gepufferten `ClientHello` (Handshake-Nachricht über mehrere Records).
pub const MAX_CLIENT_HELLO: usize = 64 * 1024;

/// Ungültiger Passthrough-Eintrag (Konfigurationsfehler).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("passthrough[{index}] `{entry}`: {message}")]
pub struct PassthroughError {
    pub index: usize,
    pub entry: String,
    pub message: String,
}

/// Passthrough-Ziele einer Proxy-Instanz: `host` (Port 443) oder `host:port`. Nur exakte
/// Hostnamen: keine IP-Literale (ohne Namen gibt es keine SNI zu prüfen) und keine
/// Wildcards.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Passthrough {
    targets: Vec<(String, u16)>,
}

impl Passthrough {
    /// Liest die Einträge; Domains werden wie Regeln normalisiert (klein, Punycode, ohne
    /// abschließenden Punkt).
    pub fn parse<S: AsRef<str>>(entries: &[S]) -> Result<Self, PassthroughError> {
        let mut targets = Vec::new();
        for (index, e) in entries.iter().enumerate() {
            let raw = e.as_ref();
            let err = |message: &str| PassthroughError {
                index,
                entry: raw.chars().take(256).collect(),
                message: message.to_owned(),
            };
            if raw.contains(['/', '*', '[', ']', '@', ' ']) {
                return Err(err(
                    "erwartet wird `host` oder `host:port` (ohne Pfad, Wildcard oder IP)",
                ));
            }
            let (host, port) = match raw.rsplit_once(':') {
                Some((h, p)) => {
                    let port: u16 = p.parse().map_err(|_| err("ungültiger Port"))?;
                    if port == 0 {
                        return Err(err("ungültiger Port"));
                    }
                    (h, port)
                }
                None => (raw, 443),
            };
            match Host::parse(host) {
                Ok(Host::Domain(d)) => {
                    if !targets.contains(&(d.clone(), port)) {
                        targets.push((d, port));
                    }
                }
                Ok(Host::Ip(_)) => {
                    return Err(err(
                        "IP-Literale sind kein Passthrough-Ziel (ohne Namen keine SNI-Prüfung)",
                    ));
                }
                Err(_) => return Err(err("ungültiger Host")),
            }
        }
        Ok(Self { targets })
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// Ist `host:port` ein Passthrough-Ziel? Subdomains sind nicht mitgemeint.
    pub fn contains(&self, host: &Host, port: u16) -> bool {
        match host {
            Host::Domain(d) => self.targets.iter().any(|(h, p)| h == d && *p == port),
            Host::Ip(_) => false,
        }
    }

    /// Die Ziele als `host:port` (für Logs und `explain`).
    pub fn entries(&self) -> Vec<String> {
        self.targets
            .iter()
            .map(|(h, p)| format!("{h}:{p}"))
            .collect()
    }
}

/// Ergebnis der SNI-Prüfung eines (teilweise) empfangenen `ClientHello`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Hello {
    /// Mehr Bytes nötig.
    Incomplete,
    /// Vollständig; die SNI (normalisiert) bzw. `None` ohne Erweiterung `server_name`.
    Sni(Option<String>),
    /// Kein TLS-`ClientHello` oder fehlerhaft.
    Invalid,
}

fn u16_at(b: &[u8], i: usize) -> Option<usize> {
    Some(usize::from(*b.get(i)?) << 8 | usize::from(*b.get(i + 1)?))
}

/// Prüft die ersten Bytes eines Tunnels: TLS-Records vom Typ Handshake, darin ein vollständiges
/// `ClientHello`, aus dem die SNI (`server_name`, Typ `host_name`) gelesen wird.
pub(crate) fn client_hello_sni(buf: &[u8]) -> Hello {
    let mut hs: Vec<u8> = Vec::new();
    let mut i = 0;
    loop {
        if hs.len() >= 4 {
            // Handshake-Typ ClientHello (1) mit 24-Bit-Länge.
            if hs[0] != 1 {
                return Hello::Invalid;
            }
            let msg_len = usize::from(hs[1]) << 16 | usize::from(hs[2]) << 8 | usize::from(hs[3]);
            if msg_len + 4 > MAX_CLIENT_HELLO {
                return Hello::Invalid;
            }
            if hs.len() >= msg_len + 4 {
                return match parse_sni(&hs[4..4 + msg_len]) {
                    Some(sni) => Hello::Sni(sni),
                    None => Hello::Invalid,
                };
            }
        }
        if buf.len() < i + 5 {
            return if buf.len() > MAX_CLIENT_HELLO {
                Hello::Invalid
            } else {
                Hello::Incomplete
            };
        }
        // ContentType handshake (22), Version 3.x.
        if buf[i] != 22 || buf[i + 1] != 3 {
            return Hello::Invalid;
        }
        let Some(len) = u16_at(buf, i + 3) else {
            return Hello::Invalid;
        };
        if len == 0 || len > 16_384 + 256 {
            return Hello::Invalid;
        }
        if buf.len() < i + 5 + len {
            return Hello::Incomplete;
        }
        hs.extend_from_slice(&buf[i + 5..i + 5 + len]);
        i += 5 + len;
    }
}

/// SNI aus dem Körper eines `ClientHello`; `None` bei Formatfehlern, `Some(None)` ohne SNI.
fn parse_sni(m: &[u8]) -> Option<Option<String>> {
    // legacy_version (2) + random (32)
    let mut i = 34;
    let sid = usize::from(*m.get(i)?);
    i += 1 + sid;
    let suites = u16_at(m, i)?;
    i += 2 + suites;
    let comp = usize::from(*m.get(i)?);
    i += 1 + comp;
    if i == m.len() {
        return Some(None);
    }
    let ext_len = u16_at(m, i)?;
    i += 2;
    let end = i.checked_add(ext_len)?;
    if end > m.len() {
        return None;
    }
    let mut sni = None;
    while i < end {
        let ty = u16_at(m, i)?;
        let len = u16_at(m, i + 2)?;
        let body = m.get(i + 4..i + 4 + len)?;
        i += 4 + len;
        if ty != 0 {
            continue;
        }
        if sni.is_some() {
            // Doppelte Erweiterung: mehrdeutig.
            return None;
        }
        let list = u16_at(body, 0)?;
        let mut j = 2;
        let list_end = 2 + list;
        if list_end > body.len() {
            return None;
        }
        while j < list_end {
            let name_type = *body.get(j)?;
            let n = u16_at(body, j + 1)?;
            let name = body.get(j + 3..j + 3 + n)?;
            j += 3 + n;
            if name_type == 0 {
                if sni.is_some() {
                    return None;
                }
                let name = std::str::from_utf8(name).ok()?;
                let name = name.strip_suffix('.').unwrap_or(name).to_ascii_lowercase();
                sni = Some(name);
            }
        }
    }
    Some(sni)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimaler ClientHello mit optionaler SNI, verpackt in `records` Records.
    fn hello(sni: Option<&str>, split: Option<usize>) -> Vec<u8> {
        let mut body = vec![3, 3];
        body.extend_from_slice(&[7u8; 32]);
        body.push(0); // session id
        body.extend_from_slice(&[0, 2, 0x13, 0x01]); // eine Cipher-Suite
        body.extend_from_slice(&[1, 0]); // compression null
        let mut ext = Vec::new();
        if let Some(name) = sni {
            let n = name.as_bytes();
            let mut list = vec![0u8];
            list.extend_from_slice(&(n.len() as u16).to_be_bytes());
            list.extend_from_slice(n);
            let mut sn = (list.len() as u16).to_be_bytes().to_vec();
            sn.extend_from_slice(&list);
            ext.extend_from_slice(&[0, 0]);
            ext.extend_from_slice(&(sn.len() as u16).to_be_bytes());
            ext.extend_from_slice(&sn);
        }
        // eine fremde Erweiterung
        ext.extend_from_slice(&[0, 43, 0, 3, 2, 3, 4]);
        body.extend_from_slice(&(ext.len() as u16).to_be_bytes());
        body.extend_from_slice(&ext);
        let mut hs = vec![1];
        hs.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        hs.extend_from_slice(&body);
        let chunks: Vec<&[u8]> = match split {
            Some(at) => vec![&hs[..at], &hs[at..]],
            None => vec![&hs[..]],
        };
        let mut out = Vec::new();
        for c in chunks {
            out.extend_from_slice(&[22, 3, 1]);
            out.extend_from_slice(&(c.len() as u16).to_be_bytes());
            out.extend_from_slice(c);
        }
        out
    }

    #[test]
    fn prx_005_sni_is_read_from_one_or_more_records() {
        let h = hello(Some("API.Anthropic.com."), None);
        assert_eq!(
            client_hello_sni(&h),
            Hello::Sni(Some("api.anthropic.com".into()))
        );
        let h = hello(Some("api.anthropic.com"), Some(10));
        assert_eq!(
            client_hello_sni(&h),
            Hello::Sni(Some("api.anthropic.com".into()))
        );
        // Jede abgeschnittene Form ist unvollständig, nie ein Treffer.
        for cut in 0..h.len() {
            assert_eq!(client_hello_sni(&h[..cut]), Hello::Incomplete, "cut {cut}");
        }
        assert_eq!(client_hello_sni(&hello(None, None)), Hello::Sni(None));
    }

    #[test]
    fn prx_005_non_tls_bytes_are_invalid() {
        assert_eq!(client_hello_sni(b"GET / HTTP/1.1\r\n\r\n"), Hello::Invalid);
        assert_eq!(client_hello_sni(&[22, 3, 1, 0, 0]), Hello::Invalid);
        // ServerHello statt ClientHello.
        let mut h = hello(Some("a.test"), None);
        h[5] = 2;
        assert_eq!(client_hello_sni(&h), Hello::Invalid);
    }

    #[test]
    fn prx_005_garbage_never_panics() {
        let mut seed = 1u32;
        for len in 0..2048usize {
            let mut b = vec![22u8, 3, 1];
            for _ in 0..len {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                b.push((seed >> 16) as u8);
            }
            let _ = client_hello_sni(&b);
        }
    }
}
