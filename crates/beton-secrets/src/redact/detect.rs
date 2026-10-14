//! Muster-Detektoren für verbreitete Token-Formate (SEC-013, OBS-002).
//!
//! Jeder Detektor ist ein regulärer Ausdruck mit einem literalen Präfix (schnelle Vorfilterung
//! im `regex`-Crate). Kontextabhängige Detektoren (AWS-Secret-Key, `Authorization`-Header)
//! ersetzen nur die Gruppe `secret`, der Kontext bleibt lesbar. `bt_cred_`-Platzhalter sind
//! harmlos und bleiben stehen. Eine Entropie-Heuristik gibt es nicht (Spec: standardmäßig
//! aus; folgt mit #214).

use std::sync::LazyLock;

use regex::Regex;

use super::{Span, Unavailable};

/// Ein Detektor: Art, Ausdruck, optional die zu ersetzende Gruppe und eine Prüfung.
struct Detector {
    kind: &'static str,
    re: Regex,
    /// Nur diese benannte Gruppe ersetzen (Kontext bleibt stehen).
    group: Option<&'static str>,
    /// Zusätzliche Prüfung des Treffers (z. B. CRC der beton-Tokens).
    check: Option<Check>,
}

/// Präfix der Credential-Platzhalter des Proxys (PRX-006); bleibt immer stehen.
const PLACEHOLDER: &str = "bt_cred_";

/// Prüfung eines Treffers.
type Check = fn(&str) -> bool;

/// Art, Ausdruck, zu ersetzende Gruppe, Prüfung.
type Spec = (
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<Check>,
);

const SPECS: &[Spec] = &[
    // Reihenfolge zählt bei gleichem Start: Anthropic vor OpenAI (`sk-ant-` vs. `sk-`).
    (
        "github",
        r"(?:gh[pousr]_[A-Za-z0-9]{36,255}|github_pat_[A-Za-z0-9_]{22,255})",
        None,
        None,
    ),
    (
        "gitlab",
        r"(?:glpat|gloas)-[A-Za-z0-9_\-]{20,255}",
        None,
        None,
    ),
    ("anthropic", r"sk-ant-[A-Za-z0-9_\-]{20,255}", None, None),
    (
        "openai",
        r"\bsk-(?:proj-|svcacct-|admin-)?[A-Za-z0-9_\-]{20,255}",
        None,
        Some(not_anthropic),
    ),
    ("aws", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", None, None),
    (
        "aws",
        r#"(?i:aws_?secret_?access_?key|secret_?access_?key)["']?\s*[:=]\s*["']?(?P<secret>[A-Za-z0-9/+]{40})(?:[^A-Za-z0-9/+]|$)"#,
        Some("secret"),
        None,
    ),
    ("slack", r"xox[abpr]-[A-Za-z0-9\-]{10,255}", None, None),
    (
        "private_key",
        r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----(?:(?s:.*?)-----END [A-Z0-9 ]*PRIVATE KEY-----|(?s:.*))",
        None,
        None,
    ),
    (
        "beton",
        r"bt_(?:pat|sat|dev|run|loc)_[0-9A-Za-z]{46}\b",
        None,
        Some(beton_checksum_ok),
    ),
    // Heutiges Format der Runner-Tokens (`bt_run_` + 64 Hex), bis AUTH-011 vereinheitlicht.
    ("beton", r"bt_run_[0-9a-f]{64}\b", None, None),
    (
        "jwt",
        r"eyJ[A-Za-z0-9_\-]{10,}\.eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]*",
        None,
        None,
    ),
    (
        "authorization",
        r#"(?i:\b(?:proxy-)?authorization)["']?\s*[:=]\s*["']?(?i:bearer|basic|token)\s+(?P<secret>[^\s"',;]{8,})"#,
        Some("secret"),
        Some(not_placeholder),
    ),
];

static DETECTORS: LazyLock<Result<Vec<Detector>, String>> = LazyLock::new(|| {
    SPECS
        .iter()
        .map(|(kind, re, group, check)| {
            Ok(Detector {
                kind,
                re: Regex::new(re).map_err(|e| format!("Detektor {kind}: {e}"))?,
                group: *group,
                check: *check,
            })
        })
        .collect()
});

/// Sucht Token-Muster; `Err`, wenn die Detektoren nicht bereitstehen (fail closed).
pub(crate) fn find(text: &str, out: &mut Vec<Span>) -> Result<(), Unavailable> {
    let detectors = DETECTORS.as_ref().map_err(|_| Unavailable)?;
    for d in detectors {
        match d.group {
            None => {
                for m in d.re.find_iter(text) {
                    push(d, m, out);
                }
            }
            Some(group) => {
                for caps in d.re.captures_iter(text) {
                    if let Some(m) = caps.name(group) {
                        push(d, m, out);
                    }
                }
            }
        }
    }
    Ok(())
}

fn push(d: &Detector, m: regex::Match<'_>, out: &mut Vec<Span>) {
    if d.check.is_some_and(|check| !check(m.as_str())) {
        return;
    }
    out.push(Span {
        start: m.start(),
        end: m.end(),
        kind: d.kind,
    });
}

fn not_anthropic(token: &str) -> bool {
    !token.starts_with("sk-ant-")
}

fn not_placeholder(token: &str) -> bool {
    !token.starts_with(PLACEHOLDER)
}

/// `bt_<art>_<40 Zeichen><6 Zeichen Prüfsumme>`: nur mit gültiger Prüfsumme ein Treffer.
fn beton_checksum_ok(token: &str) -> bool {
    let Some(body) = token.get(7..) else {
        return false;
    };
    let (random, sum) = body.split_at(body.len().saturating_sub(6));
    random.len() == 40 && beton_token_checksum(random) == sum
}

/// CRC32 (IEEE 802.3, reflektiert, Polynom `0xEDB88320`) für die beton-Token-Prüfsumme.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Prüfsumme eines beton-Tokens (Token-Formate, 05-security-identity.md): CRC32 der
/// 40 Zufallszeichen in base62 (`0-9A-Za-z`), 6 Zeichen, vorne mit `0` aufgefüllt.
pub fn beton_token_checksum(random: &str) -> String {
    const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut n = u64::from(crc32(random.as_bytes()));
    let mut out = [b'0'; 6];
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[(n % 62) as usize];
        n /= 62;
    }
    String::from_utf8_lossy(&out).into_owned()
}
