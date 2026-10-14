//! Private-IP-Schutz (PRX-007): Welche Ziele sind gesperrt, und welche gibt
//! `egress_allow_private` ausdrücklich frei?
//!
//! Gesperrt ist alles Nicht-Globale: `0.0.0.0/8`, `10/8`, `100.64/10`, `127/8`, `169.254/16`
//! (inkl. Cloud-Metadaten), `172.16/12`, `192.0.0/24`, `192.168/16`, `198.18/15`,
//! Dokumentationsnetze, Multicast, reservierte Netze und Broadcast, `168.63.129.16` (Azure);
//! bei IPv6 `::`, `::1`, `fc00::/7`, `fe80::/10`, `fec0::/10`, Multicast, Dokumentation,
//! Discard, lokales NAT64, Teredo sowie alle IPv6-Formen, die eine gesperrte IPv4-Adresse
//! einbetten (IPv4-gemappt, -kompatibel, SIIT, NAT64 `64:ff9b::/96`, 6to4).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// Ungültiger Eintrag in `egress_allow_private` (Konfigurationsfehler).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("ungültiger Eintrag in egress_allow_private `{entry}`: {message}")]
pub struct AllowListError {
    pub entry: String,
    pub message: String,
}

/// Gesperrte IPv4-Netze (Adresse, Präfixlänge).
const BLOCKED_V4: [(Ipv4Addr, u8); 15] = [
    (Ipv4Addr::new(0, 0, 0, 0), 8),
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(100, 64, 0, 0), 10),
    (Ipv4Addr::new(127, 0, 0, 0), 8),
    (Ipv4Addr::new(169, 254, 0, 0), 16),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 0, 0, 0), 24),
    (Ipv4Addr::new(192, 0, 2, 0), 24),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    (Ipv4Addr::new(198, 18, 0, 0), 15),
    (Ipv4Addr::new(198, 51, 100, 0), 24),
    (Ipv4Addr::new(203, 0, 113, 0), 24),
    // Multicast
    (Ipv4Addr::new(224, 0, 0, 0), 4),
    // Reserviert inkl. Broadcast 255.255.255.255
    (Ipv4Addr::new(240, 0, 0, 0), 4),
    // Azure-Wireserver (DHCP, Metadaten)
    (Ipv4Addr::new(168, 63, 129, 16), 32),
];

/// Gesperrte IPv6-Netze ohne eingebettete IPv4-Adresse.
const BLOCKED_V6: [(Ipv6Addr, u8); 10] = [
    (Ipv6Addr::UNSPECIFIED, 128),
    (Ipv6Addr::LOCALHOST, 128),
    // Unique Local (fc00::/7), u. a. AWS-Metadaten fd00:ec2::254
    (Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0), 7),
    (Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0), 10),
    (Ipv6Addr::new(0xfec0, 0, 0, 0, 0, 0, 0, 0), 10),
    (Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0), 8),
    (Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0), 32),
    (Ipv6Addr::new(0x100, 0, 0, 0, 0, 0, 0, 0), 64),
    // Lokales NAT64 (RFC 8215)
    (Ipv6Addr::new(0x64, 0xff9b, 1, 0, 0, 0, 0, 0), 48),
    // Teredo: die eingebettete Client-Adresse ist verschleiert, daher ganz gesperrt.
    (Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 32),
];

fn in_v4(ip: Ipv4Addr, net: Ipv4Addr, prefix: u8) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    };
    u32::from(ip) & mask == u32::from(net) & mask
}

fn in_v6(ip: Ipv6Addr, net: Ipv6Addr, prefix: u8) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix))
    };
    u128::from(ip) & mask == u128::from(net) & mask
}

/// Die IPv4-Adresse, die eine IPv6-Adresse einbettet: IPv4-gemappt (`::ffff:a.b.c.d`),
/// IPv4-kompatibel (`::a.b.c.d`), SIIT (`::ffff:0:a.b.c.d`), NAT64 (`64:ff9b::/96`) und
/// 6to4 (`2002:AABB:CCDD::/48`).
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let bits = u128::from(ip);
    let low = Ipv4Addr::from((bits & 0xffff_ffff) as u32);
    let prefix96 = bits >> 32;
    match prefix96 {
        0xffff | 0xffff_0000 | 0 => Some(low),
        p if p == u128::from(Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, 0, 0)) >> 32 => Some(low),
        _ if bits >> 112 == 0x2002 => Some(Ipv4Addr::from(((bits >> 80) & 0xffff_ffff) as u32)),
        _ => None,
    }
}

fn blocked_v4(ip: Ipv4Addr) -> bool {
    BLOCKED_V4.iter().any(|(net, p)| in_v4(ip, *net, *p))
}

/// Ist die Adresse ein gesperrtes (nicht-globales) Ziel?
pub fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => blocked_v4(v4),
        IpAddr::V6(v6) => {
            if BLOCKED_V6.iter().any(|(net, p)| in_v6(v6, *net, *p)) {
                return true;
            }
            embedded_v4(v6).is_some_and(blocked_v4)
        }
    }
}

/// IPv4-gemappte IPv6-Adressen als IPv4, sonst unverändert.
pub fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AllowEntry {
    net: IpAddr,
    prefix: u8,
    port: u16,
}

impl AllowEntry {
    fn parse(entry: &str) -> Result<Self, AllowListError> {
        let fail = |message: &str| AllowListError {
            entry: entry.to_owned(),
            message: message.to_owned(),
        };
        let (addr, port) = if let Some(rest) = entry.strip_prefix('[') {
            let (inside, after) = rest.split_once(']').ok_or_else(|| fail("`]` fehlt"))?;
            let port = after
                .strip_prefix(':')
                .ok_or_else(|| fail("Port fehlt (ip:port oder cidr:port)"))?;
            (inside, port)
        } else {
            let (addr, port) = entry
                .rsplit_once(':')
                .ok_or_else(|| fail("Port fehlt (ip:port oder cidr:port)"))?;
            if addr.contains(':') {
                return Err(fail("IPv6-Adressen in eckigen Klammern angeben"));
            }
            (addr, port)
        };
        let port = match port.parse::<u16>() {
            Ok(p) if p > 0 && port.bytes().all(|b| b.is_ascii_digit()) => p,
            _ => return Err(fail("ungültiger Port")),
        };
        let (ip, prefix) = match addr.split_once('/') {
            Some((ip, prefix)) => (ip, Some(prefix)),
            None => (addr, None),
        };
        let ip: IpAddr = ip
            .parse()
            .map_err(|_| fail("ungültige IP-Adresse (Hostnamen sind nicht erlaubt)"))?;
        let ip = canonical(ip);
        let max = if ip.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => max,
            Some(p) => match p.parse::<u8>() {
                Ok(n) if n <= max && p.bytes().all(|b| b.is_ascii_digit()) => n,
                _ => return Err(fail("ungültige Präfixlänge")),
            },
        };
        Ok(Self {
            net: ip,
            prefix,
            port,
        })
    }

    fn permits(&self, ip: IpAddr, port: u16) -> bool {
        if port != self.port {
            return false;
        }
        match (self.net, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => in_v4(ip, net, self.prefix),
            (IpAddr::V6(net), IpAddr::V6(ip)) => in_v6(ip, net, self.prefix),
            _ => false,
        }
    }
}

/// Ausdrücklich freigegebene private Ziele: `ip:port` oder `cidr:port`, IPv6 in eckigen
/// Klammern (`[fd00::5]:5432`, `[fd00::/8]:5432`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivateAllowList {
    entries: Vec<AllowEntry>,
}

impl PrivateAllowList {
    pub fn parse<S: AsRef<str>>(entries: &[S]) -> Result<Self, AllowListError> {
        Ok(Self {
            entries: entries
                .iter()
                .map(|e| AllowEntry::parse(e.as_ref()))
                .collect::<Result<_, _>>()?,
        })
    }

    /// Gibt ein Eintrag dieses Ziel frei? IPv4-gemappte Adressen zählen als IPv4.
    pub fn permits(&self, addr: SocketAddr) -> bool {
        let ip = canonical(addr.ip());
        self.entries.iter().any(|e| e.permits(ip, addr.port()))
    }
}

/// Darf der Proxy sich mit diesem Ziel verbinden? Globale Ziele ja, gesperrte nur mit
/// passendem Eintrag in der Freigabeliste.
pub fn destination_allowed(addr: SocketAddr, allow: &PrivateAllowList) -> bool {
    !is_blocked(addr.ip()) || allow.permits(addr)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn blocked(s: &str) -> bool {
        is_blocked(s.parse().unwrap())
    }

    #[test]
    fn prx_007_spec_ranges_are_blocked() {
        for ip in [
            "0.0.0.0",
            "0.255.255.255",
            "10.0.0.1",
            "10.255.255.255",
            "100.64.0.1",
            "100.127.255.255",
            "127.0.0.1",
            "127.255.255.254",
            "169.254.169.254",
            "169.254.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.8",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
            "168.63.129.16",
            "192.0.2.1",
            "198.51.100.7",
            "203.0.113.9",
            "::",
            "::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "100::1",
            "64:ff9b:1::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
        ] {
            assert!(blocked(ip), "{ip} müsste gesperrt sein");
        }
    }

    #[test]
    fn prx_007_global_addresses_are_allowed() {
        for ip in [
            "93.184.216.34",
            "1.1.1.1",
            "8.8.8.8",
            "100.63.255.255",
            "100.128.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "169.255.0.1",
            "198.17.255.255",
            "198.20.0.0",
            "168.63.129.15",
            "2606:4700::1111",
            "2a00:1450:4001::200e",
            "::ffff:93.184.216.34",
            "64:ff9b::5db8:d822",
        ] {
            assert!(!blocked(ip), "{ip} müsste erlaubt sein");
        }
    }

    #[test]
    fn prx_007_ac3_embedded_private_ipv4_is_blocked() {
        for ip in [
            "::ffff:10.0.0.1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::10.0.0.1",
            "::127.0.0.1",
            "::ffff:0:10.0.0.1",
            "64:ff9b::a00:1",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "2002:a00:1::1",
            "2002:7f00:1::1",
        ] {
            assert!(blocked(ip), "{ip} müsste gesperrt sein");
        }
    }

    #[test]
    fn prx_007_mapped_addresses_are_canonicalized() {
        assert_eq!(
            canonical("::ffff:10.0.0.5".parse().unwrap()),
            "10.0.0.5".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            canonical("2001:db8::1".parse().unwrap()),
            "2001:db8::1".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn prx_007_ac4_allow_list_is_exact_on_ip_and_port() {
        let allow = PrivateAllowList::parse(&["10.0.0.5:5432"]).unwrap();
        let ok = |s: &str| destination_allowed(s.parse().unwrap(), &allow);
        assert!(ok("10.0.0.5:5432"));
        assert!(ok("[::ffff:10.0.0.5]:5432"));
        assert!(!ok("10.0.0.5:22"));
        assert!(!ok("10.0.0.6:5432"));
        assert!(!ok("127.0.0.1:5432"));
        assert!(ok("93.184.216.34:443"));
    }

    #[test]
    fn prx_007_allow_list_supports_cidr_and_ipv6() {
        let allow =
            PrivateAllowList::parse(&["192.168.0.0/16:8080", "[fd00::/8]:5432", "[::1]:3000"])
                .unwrap();
        let ok = |s: &str| destination_allowed(s.parse().unwrap(), &allow);
        assert!(ok("192.168.7.9:8080"));
        assert!(!ok("192.168.7.9:8081"));
        assert!(!ok("10.0.0.1:8080"));
        assert!(ok("[fd12::1]:5432"));
        assert!(!ok("[fe80::1]:5432"));
        assert!(ok("[::1]:3000"));
        assert!(!ok("127.0.0.1:3000"));
    }

    #[test]
    fn prx_007_invalid_allow_entries_are_config_errors() {
        for bad in [
            "",
            "10.0.0.5",
            "10.0.0.5:",
            "10.0.0.5:0",
            "10.0.0.5:99999",
            "10.0.0.5:*",
            "10.0.0.0/33:80",
            "fd00::1:5432",
            "[fd00::/129]:5432",
            "host.internal:5432",
            "10.0.0.0/8",
        ] {
            assert!(PrivateAllowList::parse(&[bad]).is_err(), "{bad:?}");
        }
    }

    proptest! {
        /// Jede IPv6-Form einer IPv4-Adresse wird wie die IPv4-Adresse selbst eingestuft.
        #[test]
        fn prx_007_ipv6_forms_classify_like_ipv4(raw in any::<u32>()) {
            let v4 = Ipv4Addr::from(raw);
            let expected = is_blocked(IpAddr::V4(v4));
            let o = v4.octets();
            let forms = [
                v4.to_ipv6_mapped(),
                Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0,
                    u16::from_be_bytes([o[0], o[1]]), u16::from_be_bytes([o[2], o[3]])),
                Ipv6Addr::new(0, 0, 0, 0, 0xffff, 0,
                    u16::from_be_bytes([o[0], o[1]]), u16::from_be_bytes([o[2], o[3]])),
                Ipv6Addr::new(0x2002, u16::from_be_bytes([o[0], o[1]]),
                    u16::from_be_bytes([o[2], o[3]]), 0, 0, 0, 0, 1),
            ];
            for v6 in forms {
                prop_assert_eq!(is_blocked(IpAddr::V6(v6)), expected, "{} / {}", v4, v6);
            }
        }
    }
}
