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

/// Ist die Adresse ein gesperrtes (nicht-globales) Ziel?
pub fn is_blocked(_ip: IpAddr) -> bool {
    true
}

/// IPv4-gemappte IPv6-Adressen als IPv4, sonst unverändert.
pub fn canonical(ip: IpAddr) -> IpAddr {
    ip
}

/// Ausdrücklich freigegebene private Ziele: `ip:port` oder `cidr:port`, IPv6 in eckigen
/// Klammern (`[fd00::5]:5432`, `[fd00::/8]:5432`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivateAllowList {
    _entries: Vec<String>,
}

impl PrivateAllowList {
    pub fn parse<S: AsRef<str>>(entries: &[S]) -> Result<Self, AllowListError> {
        Ok(Self {
            _entries: entries.iter().map(|e| e.as_ref().to_owned()).collect(),
        })
    }

    /// Gibt der Eintrag dieses Ziel frei?
    pub fn permits(&self, _addr: SocketAddr) -> bool {
        false
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
