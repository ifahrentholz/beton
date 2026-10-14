//! Bekannte Secret-Werte als Aho-Corasick-Matcher (SEC-013, #186).
//!
//! Aus jedem Wert entstehen Suchmuster: der Wert selbst, seine Base64-Kodierung (Standard und
//! URL-sicher, in allen drei Ausrichtungen innerhalb eines längeren Base64-Texts), seine
//! URL-Kodierung und seine JSON-Escape-Form. Die Zwischenpuffer sind [`SecretValue`]s und
//! werden beim Drop genullt. Der fertige Automat hält die Muster in seinen Übergangstabellen;
//! er wird nie serialisiert, hat kein Klartext-`Debug` und keine Methode, die Muster herausgibt.
//! Ihn beim Drop zu nullen erlaubt das Crate `aho-corasick` nicht (Restrisiko, im Security-
//! Review genannt).

use aho_corasick::{AhoCorasick, AhoCorasickKind, MatchKind};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};

use crate::service::SecretKind;
use crate::value::SecretValue;

use super::Span;

/// Kürzere Werte zählen nicht als bekannt: Sie ergäben zu viele Fehltreffer.
pub(crate) const MIN_LEN: usize = 8;

/// Art der Ersetzung für bekannte Werte.
pub(crate) const KIND: &str = "secret";

/// Matcher über die bekannten Werte eines Knotens.
pub(crate) struct KnownSecrets {
    matcher: Option<AhoCorasick>,
    values: usize,
    patterns: usize,
}

impl std::fmt::Debug for KnownSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnownSecrets")
            .field("values", &self.values)
            .field("patterns", &self.patterns)
            .finish_non_exhaustive()
    }
}

impl KnownSecrets {
    pub(crate) fn empty() -> Self {
        Self {
            matcher: None,
            values: 0,
            patterns: 0,
        }
    }

    /// Baut den Matcher. Werte unter [`MIN_LEN`] Byte und doppelte Werte zählen nicht.
    pub(crate) fn build(values: &[SecretValue]) -> Result<Self, String> {
        let mut distinct: Vec<&[u8]> = values
            .iter()
            .map(SecretValue::expose)
            .filter(|v| v.len() >= MIN_LEN)
            .collect();
        distinct.sort_unstable();
        distinct.dedup();
        let mut patterns: Vec<SecretValue> = Vec::new();
        for v in &distinct {
            for p in variants(v) {
                if p.len() >= MIN_LEN && !patterns.iter().any(|q| q.expose() == p.expose()) {
                    patterns.push(p);
                }
            }
        }
        if patterns.is_empty() {
            return Ok(Self::empty());
        }
        let matcher = AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .kind(Some(AhoCorasickKind::ContiguousNFA))
            .build(patterns.iter().map(SecretValue::expose))
            .map_err(|e| format!("Matcher für bekannte Werte nicht baubar: {e}"))?;
        Ok(Self {
            matcher: Some(matcher),
            values: distinct.len(),
            patterns: patterns.len(),
        })
    }

    pub(crate) fn values(&self) -> usize {
        self.values
    }

    pub(crate) fn find(&self, text: &str, out: &mut Vec<Span>) {
        let Some(matcher) = &self.matcher else {
            return;
        };
        for m in matcher.find_iter(text) {
            out.push(Span {
                start: m.start(),
                end: m.end(),
                kind: KIND,
            });
        }
    }
}

/// Die Teile eines gespeicherten Werts, die als bekannt gelten.
///
/// - `token`: der Wert, zusätzlich ohne umgebende Leerzeichen.
/// - `basic`: das Passwort und `username:password` (dessen Base64 ist der Basic-Header).
/// - `oauth`: `access_token` und `refresh_token`.
pub(crate) fn parts(kind: SecretKind, value: &SecretValue) -> Vec<SecretValue> {
    let mut out = Vec::new();
    match kind {
        SecretKind::Token => {
            out.push(SecretValue::from_bytes(value.expose().to_vec()));
            let trimmed = value.expose().trim_ascii();
            if trimmed.len() != value.len() {
                out.push(SecretValue::from_bytes(trimmed.to_vec()));
            }
        }
        SecretKind::Basic | SecretKind::Oauth => {
            let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(value.expose()) else {
                // Unlesbar: den ganzen Wert als bekannt behandeln.
                out.push(SecretValue::from_bytes(value.expose().to_vec()));
                return out;
            };
            let field = |name: &str| json.get(name).and_then(serde_json::Value::as_str);
            if kind == SecretKind::Basic {
                if let Some(password) = field("password") {
                    out.push(SecretValue::from(password));
                    let user = field("username").unwrap_or_default();
                    let mut joined = String::with_capacity(user.len() + 1 + password.len());
                    joined.push_str(user);
                    joined.push(':');
                    joined.push_str(password);
                    out.push(SecretValue::from(joined));
                }
            } else {
                for name in ["access_token", "refresh_token"] {
                    if let Some(token) = field(name) {
                        out.push(SecretValue::from(token));
                    }
                }
            }
            // Die geparsten Strings liegen auch im JSON-Objekt: vor dem Drop nullen.
            zeroize_json(&mut json);
        }
    }
    out
}

/// Suchmuster zu einem Wert: roh, Base64 (je drei Ausrichtungen, Standard und URL-sicher),
/// URL- und JSON-kodiert.
fn variants(value: &[u8]) -> Vec<SecretValue> {
    let mut out = vec![SecretValue::from_bytes(value.to_vec())];
    for engine in [&STANDARD_NO_PAD, &URL_SAFE_NO_PAD] {
        for offset in 0..3 {
            out.push(base64_core(engine, value, offset));
        }
    }
    let url = percent_encode(value);
    if url.expose() != value {
        out.push(url);
    }
    if let Ok(quoted) = std::str::from_utf8(value).map(serde_json::to_string) {
        let quoted = SecretValue::from(quoted.unwrap_or_default());
        let escaped = quoted.expose().get(1..quoted.len().saturating_sub(1));
        if let Some(escaped) = escaped.filter(|e| *e != value) {
            out.push(SecretValue::from_bytes(escaped.to_vec()));
        }
    }
    out
}

/// Der Teil der Base64-Kodierung, den allein die Bytes von `value` bestimmen, wenn ihm im
/// kodierten Strom `offset` (0–2) Bytes seiner Dreiergruppe vorausgehen.
fn base64_core(
    engine: &base64::engine::GeneralPurpose,
    value: &[u8],
    offset: usize,
) -> SecretValue {
    // Kapazität vorab: kein Umkopieren, das ungenullte Reste hinterließe.
    let mut joined = Vec::with_capacity(offset + value.len());
    joined.resize(offset, 0);
    joined.extend_from_slice(value);
    let joined = SecretValue::from_bytes(joined);
    let encoded = SecretValue::from(engine.encode(joined.expose()));
    // Führende Zeichen hängen von den Bytes davor ab: bei 1 Byte zwei, bei 2 Bytes drei.
    let skip = [0, 2, 3][offset];
    // Am Ende hängt das letzte Zeichen einer unvollständigen Gruppe von Folgebytes ab.
    let cut = usize::from(!joined.len().is_multiple_of(3));
    let end = encoded.len().saturating_sub(cut).max(skip);
    SecretValue::from_bytes(encoded.expose()[skip..end].to_vec())
}

/// Prozent-Kodierung aller Bytes außer `A-Z a-z 0-9 - . _ ~` (RFC 3986, Großbuchstaben).
fn percent_encode(value: &[u8]) -> SecretValue {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Vec::with_capacity(value.len() * 3);
    for &b in value {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b);
        } else {
            out.extend_from_slice(&[b'%', HEX[usize::from(b >> 4)], HEX[usize::from(b & 15)]]);
        }
    }
    SecretValue::from_bytes(out)
}

/// Nullt alle Strings eines geparsten JSON-Werts.
fn zeroize_json(value: &mut serde_json::Value) {
    use zeroize::Zeroize as _;
    match value {
        serde_json::Value::String(s) => s.zeroize(),
        serde_json::Value::Array(items) => items.iter_mut().for_each(zeroize_json),
        serde_json::Value::Object(map) => map.values_mut().for_each(zeroize_json),
        _ => {}
    }
}
