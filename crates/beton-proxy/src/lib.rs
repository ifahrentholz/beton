//! Egress-Proxy von beton (`beton-proxy`, PRX-001 … PRX-007, ADR-0007).
//!
//! Läuft im Runner (Stufe 0). Jede Session-Stufe bekommt eine eigene Proxy-Instanz mit eigenen
//! Regeln, eigenem Auth-Token und eigenem Leaf-Schlüssel. Der Proxy
//! - lehnt Requests ohne gültige `Proxy-Authorization` mit 407 ab (PRX-001),
//! - terminiert TLS mit Leaf-Zertifikaten der beton-CA, deren Schlüssel nur im Secret-Store
//!   liegt, und prüft Upstream-TLS strikt (PRX-002),
//! - erlaubt nur, was eine Regel `"METHODS host/path-glob"` erlaubt; Deny-Regeln haben
//!   Vorrang (PRX-003),
//! - prüft bei HTTP/2 jeden Stream einzeln und reicht WebSockets nach geprüftem Upgrade
//!   durch (PRX-004),
//! - tunnelt die Vendor-Hosts eines Harness ohne TLS-Terminierung, prüft dabei nur Host, Port,
//!   Private-IP-Sperre und SNI (Passthrough, PRX-005),
//! - löst Hostnamen selbst auf, prüft alle Adressen und verbindet sich nur mit einer geprüften
//!   IP; private Ziele sind gesperrt, außer sie sind ausdrücklich freigegeben (PRX-007).
//!
//! Fail closed: Jeder Fehler (Regel ungültig, DNS, TLS, Secret-Store gesperrt) führt zur
//! Ablehnung, nie zu stillschweigendem Erlauben.

pub mod ca;
pub mod ip;
mod net;
pub mod normalize;
pub mod passthrough;
pub mod proxy;
pub mod rules;

#[cfg(test)]
mod tests;

pub use ca::{BetonCa, CaError, CaInfo, CaStore, LeafIssuer};
pub use ip::{AllowListError, PrivateAllowList};
pub use normalize::{Host, InvalidHost, InvalidPath, Scheme, normalize_path};
pub use passthrough::{Passthrough, PassthroughError};
pub use proxy::{EgressProxy, InstanceSpec, ProxyError, ProxyInstance, ProxyOptions, ProxyToken};
pub use rules::{Decision, DenyReason, Request, RuleError, RuleSet};
