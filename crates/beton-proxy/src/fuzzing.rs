//! Zugänge für die Fuzz-Targets (QA-009, `fuzz/`): nur mit Feature `fuzzing` (und in den
//! Tests dieses Crates), nie in Produktionsbuilds.
//!
//! Das Modul reicht bestehende interne Funktionen unverändert durch, damit die Harnesses
//! genau den Code treffen, der im Proxy läuft. Es enthält keine eigene Logik außer der
//! Übersetzung interner Typen in öffentliche.

use http::{HeaderMap, Request};

use crate::credentials::Credentials;
use crate::normalize::{Host, Scheme};

/// Ergebnis von [`client_hello_sni`] (Spiegel des internen `Hello`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientHello {
    /// Mehr Bytes nötig.
    Incomplete,
    /// Vollständiges `ClientHello`; die SNI (normalisiert) bzw. `None` ohne `server_name`.
    Sni(Option<String>),
    /// Kein bzw. fehlerhaftes `ClientHello`.
    Invalid,
}

/// SNI-Prüfung der ersten Bytes eines Passthrough-Tunnels (PRX-005).
pub fn client_hello_sni(buf: &[u8]) -> ClientHello {
    match crate::passthrough::client_hello_sni(buf) {
        crate::passthrough::Hello::Incomplete => ClientHello::Incomplete,
        crate::passthrough::Hello::Sni(sni) => ClientHello::Sni(sni),
        crate::passthrough::Hello::Invalid => ClientHello::Invalid,
    }
}

/// Ist der Request ein HTTP/1.1-Upgrade (WebSocket, PRX-004)?
pub fn is_upgrade<B>(req: &Request<B>) -> bool {
    crate::proxy::is_upgrade(req)
}

/// Entfernt Hop-by-Hop-Header wie vor dem Weiterleiten bzw. Zurückgeben.
pub fn strip_hop_by_hop(headers: &mut HeaderMap, upgrade: bool) {
    crate::proxy::strip_hop_by_hop(headers, upgrade);
}

/// Host und Port eines Requests im Tunnel (`:authority`/absolute URI und `Host`-Header);
/// `None`, wenn eine Angabe fehlt oder ungültig ist.
pub fn request_authorities<B>(req: &Request<B>) -> Option<Vec<(Host, Option<u16>)>> {
    crate::proxy::request_authorities(req).ok()
}

/// Request nach der Credential-Ersetzung (PRX-006).
#[derive(Debug, Clone)]
pub struct Substituted {
    pub path: String,
    pub query: Option<String>,
    pub headers: HeaderMap,
}

/// Ergebnis von [`substitute_credentials`].
#[derive(Debug, Clone)]
pub enum CredentialOutcome {
    /// Kein Platzhalter und keine implizite Injektion: Request unverändert.
    Untouched,
    /// Abgelehnt (Status und Fehlercode der Antwort an den Client).
    Refused { status: u16, error: &'static str },
    /// Werte eingesetzt.
    Applied(Substituted),
}

/// Platzhalter erkennen und prüfen (`Credentials::plan`), bei Ablehnung auditieren, sonst
/// Werte holen und einsetzen (`Plan::apply`), in derselben Reihenfolge wie der Proxy.
pub async fn substitute_credentials(
    credentials: &Credentials,
    scheme: Scheme,
    host: &Host,
    path: &str,
    query: Option<&str>,
    mut headers: HeaderMap,
) -> CredentialOutcome {
    let plan = match credentials.plan(scheme, host, path, query, &headers) {
        Ok(None) => return CredentialOutcome::Untouched,
        Ok(Some(plan)) => plan,
        Err(refusal) => {
            refusal.audit(host).await;
            return CredentialOutcome::Refused {
                status: refusal.status.as_u16(),
                error: refusal.error,
            };
        }
    };
    match plan.apply(host, path, query, &mut headers).await {
        Ok((path, query, _redactor)) => CredentialOutcome::Applied(Substituted {
            path,
            query,
            headers,
        }),
        Err(refusal) => CredentialOutcome::Refused {
            status: refusal.status.as_u16(),
            error: refusal.error,
        },
    }
}
