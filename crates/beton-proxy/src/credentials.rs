//! Credential-Injection mit Platzhaltern (PRX-006) und Antwort-Redaction (PRX-010).
//!
//! Die Sandbox sieht nur Platzhalter `bt_cred_<32 Zeichen base32>` (160 bit, je Session und
//! Binding neu). Der Proxy kennt die Zuordnung Platzhalter → Binding (Name, Typ, gebundene
//! Hosts) und holt den echten Wert erst, wenn ein geprüfter Request an einen gebundenen Host
//! ihn braucht, über eine [`CredentialSource`] aus Stufe 0.
//!
//! Grenzen der Ersetzung (fail closed):
//! - ersetzt wird nur in Request-Headern (auch im dekodierten `Authorization: Basic …`) und in
//!   der URL (Pfad und Query), nie im Body;
//! - nur über TLS (MITM) an einen Host des Bindings; `TRACE` und `CONNECT` im Tunnel lehnt der
//!   Proxy ohnehin ab, auf Passthrough-Verbindungen sieht er keinen Klartext;
//! - ein Platzhalter für einen fremden Host oder über Klartext-HTTP ergibt 403
//!   `credential_host_mismatch` und einen Audit-Eintrag `credential.misuse`, ein unbekannter
//!   (syntaktisch gültiger) Platzhalter 403 `credential_unknown`;
//! - scheitert die Quelle oder das Audit, gibt es 502 `credential_source_error` und keinen
//!   Wert (nie einen veralteten).
//!
//! Ohne Platzhalter und ohne `Authorization`-Header injiziert der Proxy den Wert, wenn genau
//! ein Binding der Instanz den Host bindet und das Format feststeht (z. B. Git über HTTPS bei
//! `github`). Jede Nutzung geht mit Binding-Name und Host ins Audit ([`CredentialAudit`]), nie
//! mit dem Wert.
//!
//! Spiegelt ein Upstream einen injizierten Wert zurück, ersetzt [`ResponseRedactor`] ihn in
//! Antwort-Headern und textartigen Bodies durch den Platzhalter (PRX-010).

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use aho_corasick::{AhoCorasick, MatchKind};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::{Bytes, BytesMut};
use http::StatusCode;
use http::header::{
    ACCEPT_ENCODING, AUTHORIZATION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, HeaderMap,
    HeaderName, HeaderValue,
};
use hyper::body::{Body, Frame, SizeHint};
use zeroize::Zeroizing;

use crate::normalize::{Host, Scheme};

/// Future eines Aufrufs in Stufe 0 (Quelle, Audit).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Präfix aller Platzhalter.
pub const PREFIX: &str = "bt_cred_";
/// Länge eines Platzhalters: Präfix plus 32 Zeichen base32 (160 bit).
pub const PLACEHOLDER_LEN: usize = PREFIX.len() + 32;
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Höchstens so viel einer Antwort wird redigiert (PRX-010); was darüber hinausgeht, fließt
/// unverändert durch (Restrisiko).
pub const REDACT_LIMIT: usize = 1024 * 1024;

fn is_base32(b: u8) -> bool {
    b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b)
}

/// Fehler beim Aufbau der Bindings (Konfiguration).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("credentials: {0}")]
pub struct CredentialError(pub String);

/// Ein Platzhalter `bt_cred_<32×base32>`; kein Secret (er steht im Environment der Sandbox).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Placeholder(String);

impl Placeholder {
    /// Neuer Platzhalter mit 160 bit Zufall.
    pub fn generate() -> Result<Self, CredentialError> {
        let mut raw = Zeroizing::new([0u8; 20]);
        getrandom::fill(raw.as_mut()).map_err(|e| CredentialError(format!("Zufall: {e}")))?;
        let mut s = String::with_capacity(PLACEHOLDER_LEN);
        s.push_str(PREFIX);
        let (mut acc, mut bits) = (0u64, 0u32);
        for b in raw.iter() {
            acc = (acc << 8) | u64::from(*b);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                s.push(char::from(ALPHABET[((acc >> bits) & 31) as usize]));
            }
        }
        Ok(Self(s))
    }

    /// Prüft die Syntax.
    pub fn parse(s: &str) -> Option<Self> {
        (s.len() == PLACEHOLDER_LEN
            && s.starts_with(PREFIX)
            && s.as_bytes()[PREFIX.len()..].iter().all(|b| is_base32(*b)))
        .then(|| Self(s.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Placeholder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Placeholder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Placeholder({})", self.0)
    }
}

/// Alle syntaktisch gültigen Platzhalter in `text` mit ihrer Position.
pub fn find_placeholders(text: &[u8]) -> Vec<(usize, Placeholder)> {
    let prefix = PREFIX.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + PLACEHOLDER_LEN <= text.len() {
        if text[i..].starts_with(prefix)
            && text[i + prefix.len()..i + PLACEHOLDER_LEN]
                .iter()
                .all(|b| is_base32(*b))
        {
            let p = String::from_utf8_lossy(&text[i..i + PLACEHOLDER_LEN]).into_owned();
            out.push((i, Placeholder(p)));
            i += PLACEHOLDER_LEN;
        } else {
            i += 1;
        }
    }
    out
}

/// Injektionsformat eines Bindings (PRX-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    /// `Authorization: Bearer <v>`; `hosts` Pflicht.
    Bearer,
    /// `Authorization: Basic base64(username:<v>)`; `hosts` und `username` Pflicht.
    Basic,
    /// Git über HTTPS: Platzhalter als Passwort im Basic-Header; `hosts` Pflicht.
    GitHttps,
    /// GitHub: Git mit Basic `x-access-token:<v>`, API mit `Bearer <v>` (auch `token <v>`).
    Github,
    /// GitLab: Git mit Basic `oauth2:<v>`, API mit `Bearer <v>` bzw. `PRIVATE-TOKEN: <v>`.
    Gitlab,
}

impl CredentialKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::Basic => "basic",
            Self::GitHttps => "git_https",
            Self::Github => "github",
            Self::Gitlab => "gitlab",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "bearer" => Self::Bearer,
            "basic" => Self::Basic,
            "git_https" => Self::GitHttps,
            "github" => Self::Github,
            "gitlab" => Self::Gitlab,
            _ => return None,
        })
    }

    /// Hosts ohne Angabe `hosts`; leer heißt: `hosts` ist Pflicht.
    pub fn default_hosts(self) -> &'static [&'static str] {
        match self {
            Self::Github => &[
                "github.com",
                "api.github.com",
                "uploads.github.com",
                "objects.githubusercontent.com",
            ],
            Self::Gitlab => &["gitlab.com"],
            Self::Bearer | Self::Basic | Self::GitHttps => &[],
        }
    }

    /// Benutzername im Basic-Header, wenn der Platzhalter das Passwort ist.
    fn basic_user<'a>(self, binding: Option<&'a str>, client: &'a str) -> &'a str {
        match self {
            Self::Github => "x-access-token",
            Self::Gitlab => "oauth2",
            Self::Basic | Self::GitHttps => binding.unwrap_or(client),
            Self::Bearer => client,
        }
    }
}

/// Ein gebundener Host: exakt oder `*.domain` (beliebige Subdomain, nicht `domain` selbst).
#[derive(Debug, Clone, PartialEq, Eq)]
enum HostPattern {
    Exact(String),
    Subdomains(String),
}

impl HostPattern {
    fn parse(raw: &str) -> Result<Self, CredentialError> {
        let bad = || CredentialError(format!("`{raw}` ist kein Hostname (ohne Port, ohne IP)"));
        let (wild, rest) = match raw.strip_prefix("*.") {
            Some(r) => (true, r),
            None => (false, raw),
        };
        match Host::parse(rest).map_err(|_| bad())? {
            Host::Domain(d) if !d.contains('*') => Ok(if wild {
                Self::Subdomains(format!(".{d}"))
            } else {
                Self::Exact(d)
            }),
            _ => Err(bad()),
        }
    }

    fn matches(&self, host: &Host) -> bool {
        let Host::Domain(d) = host else {
            return false;
        };
        match self {
            Self::Exact(e) => d == e,
            Self::Subdomains(s) => d.len() > s.len() && d.ends_with(s.as_str()),
        }
    }
}

/// Ein Credential-Binding einer Proxy-Instanz.
#[derive(Debug, Clone)]
pub struct Binding {
    /// Name, z. B. `github`; steht im Audit, nie der Wert.
    pub name: String,
    pub kind: CredentialKind,
    /// Gebundene Hosts; leer: die Default-Hosts des Typs.
    pub hosts: Vec<String>,
    /// Benutzername für `basic` (Pflicht) und `git_https` (optional).
    pub username: Option<String>,
    pub placeholder: Placeholder,
    /// Ohne Platzhalter und ohne `Authorization`-Header injizieren (PRX-006).
    pub implicit: bool,
}

/// Ein Secret-Wert im Speicher des Proxys: `Debug` zeigt ihn nie, beim Drop wird er genullt.
pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    pub fn new(value: Vec<u8>) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Quelle der echten Werte (Stufe 0: Secret-Store, Env des Daemons, Datei, Kommando).
pub trait CredentialSource: Send + Sync {
    /// Wert des Bindings `binding`. Fehlertexte nennen nie den Wert. Ein abgelaufener Wert
    /// (`ttl`) wird neu geholt; scheitert das, kommt ein Fehler, nie der alte Wert.
    fn value<'a>(&'a self, binding: &'a str) -> BoxFuture<'a, Result<SecretBytes, String>>;
}

/// Audit der Credential-Nutzung (PRX-006 AC6, PRX-009 AC4, SEC-012); nie mit dem Wert.
pub trait CredentialAudit: Send + Sync {
    /// Eine Nutzung an `host`. Scheitert das Audit, wird der Wert nicht eingesetzt.
    fn used<'a>(&'a self, binding: &'a str, host: &'a str) -> BoxFuture<'a, Result<(), String>>;
    /// Fehlgebrauch (`credential.misuse`): Platzhalter an fremden Host, über Klartext oder
    /// unbekannt (`binding` dann `None`).
    fn misuse<'a>(
        &'a self,
        binding: Option<&'a str>,
        host: &'a str,
        reason: &'a str,
    ) -> BoxFuture<'a, ()>;
}

struct Compiled {
    binding: Binding,
    hosts: Vec<HostPattern>,
}

struct Inner {
    bindings: Vec<Compiled>,
    source: Arc<dyn CredentialSource>,
    audit: Arc<dyn CredentialAudit>,
}

/// Die Bindings einer Proxy-Instanz; leer ohne Bindings (dann wird nie etwas ersetzt, ein
/// Platzhalter ist aber trotzdem unbekannt und wird abgelehnt).
#[derive(Clone, Default)]
pub struct Credentials(Option<Arc<Inner>>);

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self
            .0
            .iter()
            .flat_map(|i| i.bindings.iter().map(|c| c.binding.name.as_str()))
            .collect();
        f.debug_tuple("Credentials").field(&names).finish()
    }
}

impl Credentials {
    /// Prüft und übernimmt die Bindings (fail closed: jeder Fehler ist ein
    /// Konfigurationsfehler).
    pub fn new(
        bindings: Vec<Binding>,
        source: Arc<dyn CredentialSource>,
        audit: Arc<dyn CredentialAudit>,
    ) -> Result<Self, CredentialError> {
        let mut compiled: Vec<Compiled> = Vec::with_capacity(bindings.len());
        for b in bindings {
            if b.name.is_empty() {
                return Err(CredentialError("Binding ohne Namen".into()));
            }
            if compiled.iter().any(|c| c.binding.name == b.name) {
                return Err(CredentialError(format!("Binding `{}` doppelt", b.name)));
            }
            if compiled
                .iter()
                .any(|c| c.binding.placeholder == b.placeholder)
            {
                return Err(CredentialError(format!(
                    "Platzhalter von `{}` doppelt",
                    b.name
                )));
            }
            let raw: Vec<String> = if b.hosts.is_empty() {
                b.kind
                    .default_hosts()
                    .iter()
                    .map(|h| (*h).to_owned())
                    .collect()
            } else {
                b.hosts.clone()
            };
            if raw.is_empty() {
                return Err(CredentialError(format!(
                    "Binding `{}` ({}) braucht `hosts`",
                    b.name,
                    b.kind.as_str()
                )));
            }
            if b.kind == CredentialKind::Basic && b.username.is_none() {
                return Err(CredentialError(format!(
                    "Binding `{}` (basic) braucht `username`",
                    b.name
                )));
            }
            if let Some(u) = &b.username
                && (u.is_empty() || u.contains(':') || u.chars().any(char::is_control))
            {
                return Err(CredentialError(format!(
                    "Binding `{}`: ungültiger `username`",
                    b.name
                )));
            }
            let hosts = raw
                .iter()
                .map(|h| HostPattern::parse(h))
                .collect::<Result<Vec<_>, _>>()?;
            compiled.push(Compiled { binding: b, hosts });
        }
        if compiled.is_empty() {
            return Ok(Self::default());
        }
        Ok(Self(Some(Arc::new(Inner {
            bindings: compiled,
            source,
            audit,
        }))))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// Namen der Bindings (für Logs).
    pub fn names(&self) -> Vec<String> {
        self.0
            .iter()
            .flat_map(|i| i.bindings.iter().map(|c| c.binding.name.clone()))
            .collect()
    }

    /// Erkennt Platzhalter eines Requests und prüft sie (vor der Regelprüfung, damit auch ein
    /// abgelehnter Fehlgebrauch auditiert wird). `Ok(None)`: nichts zu ersetzen.
    pub(crate) fn plan(
        &self,
        scheme: Scheme,
        host: &Host,
        path: &str,
        query: Option<&str>,
        headers: &HeaderMap,
    ) -> Result<Option<Plan>, Refusal> {
        let mut found: Vec<Placeholder> = Vec::new();
        for (name, value) in headers {
            found.extend(
                find_placeholders(value.as_bytes())
                    .into_iter()
                    .map(|(_, p)| p),
            );
            if name == AUTHORIZATION
                && let Some(decoded) = decode_basic(value.as_bytes())
            {
                found.extend(find_placeholders(&decoded).into_iter().map(|(_, p)| p));
            }
        }
        found.extend(
            find_placeholders(path.as_bytes())
                .into_iter()
                .map(|(_, p)| p),
        );
        if let Some(q) = query {
            found.extend(find_placeholders(q.as_bytes()).into_iter().map(|(_, p)| p));
        }
        found.sort();
        found.dedup();
        let Some(inner) = &self.0 else {
            return match found.first() {
                None => Ok(None),
                Some(_) => Err(Refusal::unknown(None)),
            };
        };
        if found.is_empty() {
            if scheme != Scheme::Https || headers.contains_key(AUTHORIZATION) {
                return Ok(None);
            }
            let candidates: Vec<usize> = inner
                .bindings
                .iter()
                .enumerate()
                .filter(|(_, c)| {
                    c.binding.implicit
                        && c.hosts.iter().any(|h| h.matches(host))
                        && implicit_format(&c.binding, host, path).is_some()
                })
                .map(|(i, _)| i)
                .collect();
            return Ok(match candidates.as_slice() {
                [one] => Some(Plan {
                    inner: inner.clone(),
                    used: vec![*one],
                    implicit: true,
                }),
                _ => None,
            });
        }
        let mut used = Vec::new();
        for p in &found {
            let Some(i) = inner
                .bindings
                .iter()
                .position(|c| &c.binding.placeholder == p)
            else {
                return Err(Refusal::unknown(Some(inner.clone())));
            };
            let c = &inner.bindings[i];
            if !c.hosts.iter().any(|h| h.matches(host)) {
                return Err(Refusal::mismatch(
                    inner.clone(),
                    &c.binding.name,
                    "host_mismatch",
                    format!(
                        "Der Platzhalter von `{}` gilt nicht für {host}",
                        c.binding.name
                    ),
                ));
            }
            if scheme != Scheme::Https {
                return Err(Refusal::mismatch(
                    inner.clone(),
                    &c.binding.name,
                    "insecure_transport",
                    format!(
                        "Credentials setzt der Proxy nur über HTTPS ein, nicht über http://{host}"
                    ),
                ));
            }
            used.push(i);
        }
        Ok(Some(Plan {
            inner: inner.clone(),
            used,
            implicit: false,
        }))
    }
}

/// Dekodiert `Basic <base64>` (sonst `None`).
fn decode_basic(value: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    if value.len() < 6 || !value[..6].eq_ignore_ascii_case(b"basic ") {
        return None;
    }
    STANDARD
        .decode(value[6..].trim_ascii())
        .ok()
        .map(Zeroizing::new)
}

/// Format der impliziten Injektion; `None`, wenn es für diesen Host nicht feststeht.
enum Implicit<'a> {
    Bearer,
    Basic(&'a str),
}

fn implicit_format<'a>(b: &'a Binding, host: &Host, path: &str) -> Option<Implicit<'a>> {
    let domain = match host {
        Host::Domain(d) => d.as_str(),
        Host::Ip(_) => "",
    };
    let api = path == "/api" || path.starts_with("/api/");
    match b.kind {
        CredentialKind::Bearer => Some(Implicit::Bearer),
        CredentialKind::Basic => b.username.as_deref().map(Implicit::Basic),
        CredentialKind::GitHttps => b.username.as_deref().map(Implicit::Basic),
        CredentialKind::Github => match domain {
            "api.github.com" | "uploads.github.com" => Some(Implicit::Bearer),
            // Signierte Objekt-URLs (LFS, Releases) vertragen keinen zusätzlichen Header.
            "objects.githubusercontent.com" => None,
            _ if api => Some(Implicit::Bearer),
            _ => Some(Implicit::Basic("x-access-token")),
        },
        CredentialKind::Gitlab if api => Some(Implicit::Bearer),
        CredentialKind::Gitlab => Some(Implicit::Basic("oauth2")),
    }
}

/// Warum ein Request mit Platzhalter abgelehnt wird.
pub(crate) struct Refusal {
    pub status: StatusCode,
    pub error: &'static str,
    pub reason: String,
    /// Audit `credential.misuse`: (Binding, Grund).
    misuse: Option<(Arc<Inner>, Option<String>, &'static str)>,
}

impl Refusal {
    fn unknown(inner: Option<Arc<Inner>>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            error: "credential_unknown",
            reason: "Unbekannter Platzhalter (bt_cred_…): gehört zu keinem Binding dieser Session"
                .into(),
            misuse: inner.map(|i| (i, None, "unknown_placeholder")),
        }
    }

    fn mismatch(inner: Arc<Inner>, binding: &str, why: &'static str, reason: String) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            error: "credential_host_mismatch",
            reason,
            misuse: Some((inner, Some(binding.to_owned()), why)),
        }
    }

    fn source(reason: String) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            error: "credential_source_error",
            reason,
            misuse: None,
        }
    }

    /// Schreibt den Audit-Eintrag `credential.misuse` (falls einer fällig ist).
    pub(crate) async fn audit(&self, host: &Host) {
        if let Some((inner, binding, why)) = &self.misuse {
            inner
                .audit
                .misuse(binding.as_deref(), &host.to_string(), why)
                .await;
        }
    }
}

/// Was in einem Request zu ersetzen bzw. zu injizieren ist.
pub(crate) struct Plan {
    inner: Arc<Inner>,
    used: Vec<usize>,
    implicit: bool,
}

impl Plan {
    /// Namen der beteiligten Bindings (für Logs).
    pub(crate) fn names(&self) -> Vec<&str> {
        self.used
            .iter()
            .map(|i| self.inner.bindings[*i].binding.name.as_str())
            .collect()
    }

    /// Holt die Werte, auditiert jede Nutzung und setzt sie ein. Liefert den Redactor für die
    /// Antwort (PRX-010).
    pub(crate) async fn apply(
        &self,
        host: &Host,
        path: &str,
        query: Option<&str>,
        headers: &mut HeaderMap,
    ) -> Result<(String, Option<String>, ResponseRedactor), Refusal> {
        let host_s = host.to_string();
        let mut values: Vec<(usize, SecretBytes)> = Vec::with_capacity(self.used.len());
        for i in &self.used {
            let b = &self.inner.bindings[*i].binding;
            let v = self
                .inner
                .source
                .value(&b.name)
                .await
                .map_err(|e| Refusal::source(format!("Quelle von `{}`: {e}", b.name)))?;
            if v.expose().is_empty() {
                return Err(Refusal::source(format!("Quelle von `{}` ist leer", b.name)));
            }
            values.push((*i, v));
        }
        // Erst das Audit, dann der Wert im Request (ohne Eintrag keine Nutzung).
        for (i, _) in &values {
            let b = &self.inner.bindings[*i].binding;
            self.inner
                .audit
                .used(&b.name, &host_s)
                .await
                .map_err(|e| Refusal::source(format!("Audit von `{}`: {e}", b.name)))?;
        }
        let lookup = |p: &Placeholder| -> Option<(&Binding, &SecretBytes)> {
            values
                .iter()
                .map(|(i, v)| (&self.inner.bindings[*i].binding, v))
                .find(|(b, _)| &b.placeholder == p)
        };
        if self.implicit {
            let (b, v) = values
                .first()
                .map(|(i, v)| (&self.inner.bindings[*i].binding, v))
                .ok_or_else(|| Refusal::source("kein Wert".into()))?;
            let header = match implicit_format(b, host, path) {
                Some(Implicit::Bearer) => bearer(v.expose()),
                Some(Implicit::Basic(user)) => basic(user, v.expose()),
                None => return Err(Refusal::source("Format unbestimmt".into())),
            };
            headers.insert(AUTHORIZATION, header?);
        } else {
            substitute_headers(headers, &lookup)?;
        }
        let path = substitute_url(path, &lookup);
        let query = query.map(|q| substitute_url(q, &lookup));
        // Komprimierte Antworten müsste der Proxy für die Redaction puffern (PRX-010).
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        let redactor = ResponseRedactor::new(
            values
                .iter()
                .map(|(i, v)| (v.expose(), &self.inner.bindings[*i].binding.placeholder)),
        );
        Ok((path, query, redactor))
    }
}

fn header_value(bytes: &[u8]) -> Result<HeaderValue, Refusal> {
    let mut v = HeaderValue::from_bytes(bytes).map_err(|_| {
        Refusal::source("Der Wert ist als HTTP-Header ungültig (Steuerzeichen)".into())
    })?;
    v.set_sensitive(true);
    Ok(v)
}

fn bearer(value: &[u8]) -> Result<HeaderValue, Refusal> {
    let mut out = Zeroizing::new(b"Bearer ".to_vec());
    out.extend_from_slice(value);
    header_value(&out)
}

fn basic(user: &str, value: &[u8]) -> Result<HeaderValue, Refusal> {
    let mut plain = Zeroizing::new(Vec::with_capacity(user.len() + 1 + value.len()));
    plain.extend_from_slice(user.as_bytes());
    plain.push(b':');
    plain.extend_from_slice(value);
    let encoded = Zeroizing::new(format!("Basic {}", STANDARD.encode(plain.as_slice())));
    header_value(encoded.as_bytes())
}

/// Ersetzt alle Platzhalter in `text` durch `f(wert)`.
fn replace_all<'a>(
    text: &[u8],
    lookup: &impl Fn(&Placeholder) -> Option<(&'a Binding, &'a SecretBytes)>,
    encode: impl Fn(&[u8], &mut Vec<u8>),
) -> Zeroizing<Vec<u8>> {
    let mut out = Zeroizing::new(Vec::with_capacity(text.len()));
    let mut last = 0;
    for (pos, p) in find_placeholders(text) {
        out.extend_from_slice(&text[last..pos]);
        match lookup(&p) {
            Some((_, v)) => encode(v.expose(), &mut out),
            None => out.extend_from_slice(p.as_str().as_bytes()),
        }
        last = pos + PLACEHOLDER_LEN;
    }
    out.extend_from_slice(&text[last..]);
    out
}

fn substitute_headers<'a>(
    headers: &mut HeaderMap,
    lookup: &impl Fn(&Placeholder) -> Option<(&'a Binding, &'a SecretBytes)>,
) -> Result<(), Refusal> {
    let raw = |v: &[u8], out: &mut Vec<u8>| out.extend_from_slice(v);
    let old = std::mem::take(headers);
    let mut current: Option<HeaderName> = None;
    for (name, value) in old {
        if let Some(n) = name {
            current = Some(n);
        }
        let Some(name) = current.clone() else {
            continue;
        };
        let bytes = value.as_bytes();
        let new = if name == AUTHORIZATION {
            authorization(bytes, lookup)?
        } else if find_placeholders(bytes).is_empty() {
            value
        } else {
            header_value(&replace_all(bytes, lookup, raw))?
        };
        headers.append(name, new);
    }
    Ok(())
}

/// `Authorization` im Format des Bindings: Basic dekodiert (Platzhalter als Passwort → Name
/// nach Typ), `Bearer`/`token` roh; ein Binding vom Typ `basic` macht aus `<schema> <ph>`
/// einen Basic-Header.
fn authorization<'a>(
    bytes: &[u8],
    lookup: &impl Fn(&Placeholder) -> Option<(&'a Binding, &'a SecretBytes)>,
) -> Result<HeaderValue, Refusal> {
    let raw = |v: &[u8], out: &mut Vec<u8>| out.extend_from_slice(v);
    if let Some(decoded) = decode_basic(bytes) {
        if find_placeholders(&decoded).is_empty() {
            return header_value(bytes);
        }
        let (user, pass) = match decoded.iter().position(|b| *b == b':') {
            Some(i) => (&decoded[..i], &decoded[i + 1..]),
            None => (&decoded[..], &[][..]),
        };
        if let Some(p) = std::str::from_utf8(pass).ok().and_then(Placeholder::parse)
            && let Some((b, v)) = lookup(&p)
        {
            let client = String::from_utf8_lossy(user);
            let name = b.kind.basic_user(b.username.as_deref(), &client);
            return basic(name, v.expose());
        }
        let replaced = replace_all(&decoded, lookup, raw);
        let encoded = Zeroizing::new(format!("Basic {}", STANDARD.encode(replaced.as_slice())));
        return header_value(encoded.as_bytes());
    }
    let found = find_placeholders(bytes);
    if let [(pos, p)] = found.as_slice()
        && let Some((b, v)) = lookup(p)
        && b.kind == CredentialKind::Basic
        && pos + PLACEHOLDER_LEN == bytes.len()
        && let Some(user) = b.username.as_deref()
    {
        return basic(user, v.expose());
    }
    if found.is_empty() {
        return header_value(bytes);
    }
    header_value(&replace_all(bytes, lookup, raw))
}

/// Ersetzt Platzhalter in Pfad bzw. Query; der Wert wird prozent-kodiert (alles außer
/// unreservierten Zeichen).
fn substitute_url<'a>(
    text: &str,
    lookup: &impl Fn(&Placeholder) -> Option<(&'a Binding, &'a SecretBytes)>,
) -> String {
    if find_placeholders(text.as_bytes()).is_empty() {
        return text.to_owned();
    }
    let out = replace_all(text.as_bytes(), lookup, |v, out| {
        for b in v {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                out.push(*b);
            } else {
                out.extend_from_slice(format!("%{b:02X}").as_bytes());
            }
        }
    });
    String::from_utf8_lossy(&out).into_owned()
}

/// Ersetzt zurückgespiegelte echte Werte durch ihre Platzhalter (PRX-010).
pub(crate) struct ResponseRedactor {
    matcher: Option<AhoCorasick>,
    replacements: Vec<Vec<u8>>,
    longest: usize,
}

impl ResponseRedactor {
    fn new<'a>(pairs: impl Iterator<Item = (&'a [u8], &'a Placeholder)>) -> Self {
        let mut patterns: Vec<Zeroizing<Vec<u8>>> = Vec::new();
        let mut replacements = Vec::new();
        for (value, placeholder) in pairs {
            let mut forms = vec![Zeroizing::new(value.to_vec())];
            // JSON-Escape-Form (z. B. ein Wert mit `/` oder `"` in einer JSON-Antwort).
            if let Ok(s) = std::str::from_utf8(value)
                && let Ok(json) = serde_json::to_string(s)
            {
                let inner = json.as_bytes()[1..json.len() - 1].to_vec();
                if inner != value {
                    forms.push(Zeroizing::new(inner));
                }
            }
            for f in forms {
                patterns.push(f);
                replacements.push(placeholder.as_str().as_bytes().to_vec());
            }
        }
        let longest = patterns.iter().map(|p| p.len()).max().unwrap_or(0);
        let matcher = AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .build(patterns.iter().map(|p| p.as_slice()))
            .ok();
        Self {
            matcher: matcher.filter(|_| longest > 0),
            replacements,
            longest,
        }
    }

    fn redact(&self, text: &[u8]) -> Option<Vec<u8>> {
        let m = self.matcher.as_ref()?;
        if !m.is_match(text) {
            return None;
        }
        let mut out = Vec::with_capacity(text.len());
        m.replace_all_with_bytes(text, &mut out, |mat, _, dst| {
            dst.extend_from_slice(&self.replacements[mat.pattern().as_usize()]);
            true
        });
        Some(out)
    }

    /// Antwort-Header.
    pub(crate) fn headers(&self, headers: &mut HeaderMap) {
        let names: Vec<HeaderName> = headers.keys().cloned().collect();
        for name in names {
            let values: Vec<HeaderValue> = headers.get_all(&name).iter().cloned().collect();
            if !values.iter().any(|v| self.redact(v.as_bytes()).is_some()) {
                continue;
            }
            headers.remove(&name);
            for v in values {
                let new = match self.redact(v.as_bytes()) {
                    Some(r) => HeaderValue::from_bytes(&r).ok(),
                    None => Some(v),
                };
                if let Some(n) = new {
                    headers.append(name.clone(), n);
                }
            }
        }
    }

    /// Wie der Body der Antwort behandelt wird: textartige Bodies bis [`REDACT_LIMIT`]
    /// unkomprimiert im Strom, gzip/br gepuffert und dekodiert; alles andere unverändert.
    pub(crate) fn body<B>(self, headers: &mut HeaderMap, body: B) -> RedactBody<B> {
        let textual = headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|ct| {
                let ct = ct
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                ct.starts_with("text/")
                    || ct == "application/json"
                    || ct == "application/xml"
                    || ct.ends_with("+json")
                    || ct.ends_with("+xml")
            })
            .unwrap_or(false);
        let length = headers
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let encoding = headers
            .get(CONTENT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_ascii_lowercase());
        let small = length.is_none_or(|l| l <= REDACT_LIMIT as u64);
        let mode = match (textual && small, encoding.as_deref()) {
            (true, None | Some("identity")) => {
                headers.remove(CONTENT_LENGTH);
                Mode::Stream {
                    carry: BytesMut::new(),
                    seen: 0,
                }
            }
            (true, Some(enc @ ("gzip" | "br"))) => Mode::Buffer {
                buf: BytesMut::new(),
                gzip: enc == "gzip",
                decided: None,
            },
            _ => Mode::Pass,
        };
        if matches!(mode, Mode::Buffer { .. }) {
            // Länge und Kodierung stehen erst nach dem Dekodieren fest.
            headers.remove(CONTENT_LENGTH);
        }
        RedactBody {
            inner: body,
            redactor: Arc::new(self),
            mode,
            pending: None,
            done: false,
        }
    }
}

enum Mode {
    /// Unverändert durchreichen.
    Pass,
    /// Unkomprimiert im Strom; `carry` hält das Ende, das ein Wert über eine Frame-Grenze
    /// haben könnte.
    Stream { carry: BytesMut, seen: usize },
    /// gzip/br: bis [`REDACT_LIMIT`] puffern, dann dekodiert redigieren; größer → unverändert.
    Buffer {
        buf: BytesMut,
        gzip: bool,
        decided: Option<bool>,
    },
}

/// Body einer Antwort mit Redaction (PRX-010).
pub(crate) struct RedactBody<B> {
    inner: B,
    redactor: Arc<ResponseRedactor>,
    mode: Mode,
    pending: Option<Frame<Bytes>>,
    done: bool,
}

fn decode(buf: &[u8], gzip: bool) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let mut out = Vec::new();
    let limit = REDACT_LIMIT as u64 + 1;
    let ok = if gzip {
        flate2::read::GzDecoder::new(buf)
            .take(limit)
            .read_to_end(&mut out)
            .is_ok()
    } else {
        brotli_decompressor::Decompressor::new(buf, 4096)
            .take(limit)
            .read_to_end(&mut out)
            .is_ok()
    };
    (ok && out.len() <= REDACT_LIMIT).then_some(out)
}

impl<B> Body for RedactBody<B>
where
    B: Body<Data = Bytes> + Unpin,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        loop {
            if let Some(f) = this.pending.take() {
                return Poll::Ready(Some(Ok(f)));
            }
            if this.done {
                return Poll::Ready(None);
            }
            let next = match Pin::new(&mut this.inner).poll_frame(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(n) => n,
            };
            let redactor = this.redactor.clone();
            match (&mut this.mode, next) {
                (_, Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                (Mode::Pass, other) => return Poll::Ready(other),
                (Mode::Stream { carry, seen }, Some(Ok(frame))) => {
                    let frame = match frame.into_data() {
                        Ok(data) => data,
                        Err(other) => {
                            // Trailer: erst den Rest ausgeben.
                            let rest = carry.split().freeze();
                            this.pending = Some(other);
                            if !rest.is_empty() {
                                let out = redactor.redact(&rest).map_or(rest, Bytes::from);
                                return Poll::Ready(Some(Ok(Frame::data(out))));
                            }
                            continue;
                        }
                    };
                    if *seen >= REDACT_LIMIT {
                        let mut out = carry.split();
                        out.extend_from_slice(&frame);
                        return Poll::Ready(Some(Ok(Frame::data(out.freeze()))));
                    }
                    *seen += frame.len();
                    carry.extend_from_slice(&frame);
                    let keep = redactor.longest.saturating_sub(1);
                    if carry.len() <= keep {
                        continue;
                    }
                    // Bis `cut` ausgeben, aber nie mitten in einem Treffer schneiden.
                    let mut cut = carry.len() - keep;
                    if let Some(m) = redactor.matcher.as_ref() {
                        for mat in m.find_iter(carry.as_ref()) {
                            if mat.start() < cut && mat.end() > cut {
                                cut = mat.end();
                            }
                        }
                    }
                    let head = carry.split_to(cut).freeze();
                    let out = redactor.redact(&head).map_or(head, Bytes::from);
                    return Poll::Ready(Some(Ok(Frame::data(out))));
                }
                (Mode::Stream { carry, .. }, None) => {
                    this.done = true;
                    let rest = carry.split().freeze();
                    if rest.is_empty() {
                        return Poll::Ready(None);
                    }
                    let out = redactor.redact(&rest).map_or(rest, Bytes::from);
                    return Poll::Ready(Some(Ok(Frame::data(out))));
                }
                (Mode::Buffer { buf, decided, .. }, Some(Ok(frame))) => {
                    if *decided == Some(false) {
                        return Poll::Ready(Some(Ok(frame)));
                    }
                    let data = match frame.into_data() {
                        Ok(d) => d,
                        Err(other) => {
                            this.pending = Some(other);
                            *decided = Some(false);
                            let all = buf.split().freeze();
                            if all.is_empty() {
                                continue;
                            }
                            return Poll::Ready(Some(Ok(Frame::data(all))));
                        }
                    };
                    buf.extend_from_slice(&data);
                    if buf.len() > REDACT_LIMIT {
                        // Zu groß: unverändert (kodiert) weiter.
                        *decided = Some(false);
                        return Poll::Ready(Some(Ok(Frame::data(buf.split().freeze()))));
                    }
                }
                (Mode::Buffer { buf, gzip, decided }, None) => {
                    this.done = true;
                    if *decided == Some(false) || buf.is_empty() {
                        return Poll::Ready(None);
                    }
                    let all = buf.split().freeze();
                    let out = match decode(&all, *gzip) {
                        Some(plain) => Bytes::from(redactor.redact(&plain).unwrap_or(plain)),
                        // Nicht dekodierbar: unverändert (der Client scheitert selbst daran).
                        None => all,
                    };
                    return Poll::Ready(Some(Ok(Frame::data(out))));
                }
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.done && self.pending.is_none()
    }

    fn size_hint(&self) -> SizeHint {
        match self.mode {
            Mode::Pass => self.inner.size_hint(),
            _ => SizeHint::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prx_006_placeholders_have_160_bit_in_base32() {
        let a = Placeholder::generate().unwrap();
        let b = Placeholder::generate().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.as_str().len(), PLACEHOLDER_LEN);
        assert!(a.as_str().starts_with("bt_cred_"));
        assert_eq!(Placeholder::parse(a.as_str()), Some(a.clone()));
        assert!(Placeholder::parse("bt_cred_tooshort").is_none());
        assert!(Placeholder::parse(&a.as_str().to_uppercase()).is_none());
        let text = format!("x {a}y{b}");
        let found = find_placeholders(text.as_bytes());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], (2, a));
    }

    #[test]
    fn prx_006_host_patterns() {
        let p = HostPattern::parse("*.example.com").unwrap();
        assert!(p.matches(&Host::parse("a.example.com").unwrap()));
        assert!(p.matches(&Host::parse("a.b.example.com").unwrap()));
        assert!(!p.matches(&Host::parse("example.com").unwrap()));
        assert!(!p.matches(&Host::parse("example.com.evil.test").unwrap()));
        assert!(!p.matches(&Host::parse("aexample.com").unwrap()));
        let e = HostPattern::parse("API.github.com").unwrap();
        assert!(e.matches(&Host::parse("api.github.com").unwrap()));
        assert!(HostPattern::parse("127.0.0.1").is_err());
        assert!(HostPattern::parse("a.test:443").is_err());
    }
}
