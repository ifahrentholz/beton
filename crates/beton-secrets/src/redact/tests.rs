//! Tests der Redaction-Pipeline (SEC-013, OBS-002).

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use proptest::prelude::*;
use serde_json::json;

use super::*;
use crate::service::SecretKind;
use crate::value::SecretValue;

/// Ein bekannter Wert ohne erkennbares Token-Format: nur der exakte Abgleich findet ihn.
const KNOWN: &str = "Zx9-known-SECRET-value-4711";
const GITHUB: &str = "ghp_0123456789abcdefghijABCDEFGHIJ012345";

fn ready(values: &[&str]) -> Redactor {
    let values: Vec<SecretValue> = values.iter().map(|v| SecretValue::from(*v)).collect();
    let r = Redactor::pending();
    r.install(KnownSecrets::build(&values).unwrap(), KnownSource::Store);
    r
}

fn red(r: &Redactor, text: &str) -> String {
    r.redact_str(text).unwrap().into_owned()
}

#[test]
fn sec_013_known_value_exact_match() {
    let r = ready(&[KNOWN]);
    assert_eq!(
        red(&r, &format!("Ausgabe: {KNOWN} Ende")),
        "Ausgabe: [REDACTED:secret] Ende"
    );
    // Mehrfach und direkt aneinander.
    assert_eq!(
        red(&r, &format!("{KNOWN}{KNOWN}x")),
        "[REDACTED:secret][REDACTED:secret]x"
    );
    // Ohne Treffer bleibt der Text unverändert und wird nicht kopiert.
    assert!(matches!(
        r.redact_str("nichts Geheimes").unwrap(),
        Cow::Borrowed(_)
    ));
}

#[test]
fn sec_013_base64_variants_in_all_alignments() {
    let r = ready(&[KNOWN]);
    for prefix in ["", "a", "ab", "abc", "abcd"] {
        let plain = format!("{prefix}{KNOWN}-und-noch-mehr-daten");
        for (name, engine) in [
            ("std", &STANDARD),
            ("std-no-pad", &STANDARD_NO_PAD),
            ("url", &URL_SAFE),
            ("url-no-pad", &URL_SAFE_NO_PAD),
        ] {
            let blob = engine.encode(plain.as_bytes());
            let out = red(&r, &format!("data:{blob};"));
            assert!(
                out.contains("[REDACTED:secret]"),
                "{name}, Präfix {prefix:?}: {out}"
            );
            // Kein längeres Stück der Kodierung des Werts bleibt stehen.
            let core = engine.encode(KNOWN.as_bytes());
            assert!(
                !out.contains(&core[4..16]),
                "{name}, Präfix {prefix:?}: {out}"
            );
        }
    }
}

#[test]
fn sec_013_url_and_json_encoded_variants() {
    let value = "p@ss/w0rd+&=geheim!\"x\\y";
    let r = ready(&[value]);
    let url = "p%40ss%2Fw0rd%2B%26%3Dgeheim%21%22x%5Cy";
    assert_eq!(
        red(&r, &format!("https://h.example/?pw={url}&a=1")),
        "https://h.example/?pw=[REDACTED:secret]&a=1"
    );
    let json_text = serde_json::to_string(&json!({ "pw": value })).unwrap();
    let out = red(&r, &json_text);
    assert_eq!(out, r#"{"pw":"[REDACTED:secret]"}"#);
}

#[test]
fn sec_013_short_values_are_not_matched() {
    // Werte unter 8 Byte ergäben zu viele Fehltreffer; sie zählen nicht als bekannt.
    let r = ready(&["kurz", KNOWN]);
    assert_eq!(red(&r, "kurz und gut"), "kurz und gut");
    assert_eq!(
        r.status(),
        RedactionStatus::Ready {
            values: 1,
            source: KnownSource::Store
        }
    );
}

#[test]
fn sec_013_parts_of_basic_oauth_and_token_secrets() {
    let basic = SecretValue::from(r#"{"username":"ingo","password":"pa55-geheim-wort"}"#);
    let oauth = SecretValue::from(
        r#"{"access_token":"at-0123456789abcdef","refresh_token":"rt-fedcba9876543210","expires_at":"2030-01-01T00:00:00Z"}"#,
    );
    let token = SecretValue::from("tok-value-1234567\n");
    let mut values = known::parts(SecretKind::Basic, &basic);
    values.extend(known::parts(SecretKind::Oauth, &oauth));
    values.extend(known::parts(SecretKind::Token, &token));
    let r = Redactor::pending();
    r.install(KnownSecrets::build(&values).unwrap(), KnownSource::Store);

    let header = STANDARD.encode("ingo:pa55-geheim-wort");
    let text = format!(
        "pw=pa55-geheim-wort creds={header} at=at-0123456789abcdef rt=rt-fedcba9876543210 t=tok-value-1234567"
    );
    let out = red(&r, &text);
    for leaked in [
        "pa55-geheim-wort",
        header.as_str(),
        "at-0123456789abcdef",
        "rt-fedcba9876543210",
        "tok-value-1234567",
    ] {
        assert!(!out.contains(leaked), "{leaked} steht noch in {out}");
    }
    // Der Benutzername allein ist kein Secret.
    assert_eq!(red(&r, "user ingo"), "user ingo");
}

fn beton_token(prefix: &str) -> String {
    let random = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789abcd";
    assert_eq!(random.len(), 40);
    format!("{prefix}{random}{}", beton_token_checksum(random))
}

#[test]
fn sec_013_detectors_replace_known_token_formats() {
    let pat = beton_token("bt_pat_");
    let cases: Vec<(String, &str)> = vec![
        (GITHUB.into(), "github"),
        (format!("github_pat_{}", "A1b2C3d4E5".repeat(8)), "github"),
        ("glpat-abcdefghij0123456789".into(), "gitlab"),
        ("gloas-abcdefghij0123456789xyz".into(), "gitlab"),
        (
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCD".into(),
            "anthropic",
        ),
        (
            "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789ABCD".into(),
            "openai",
        ),
        ("sk-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKL".into(), "openai"),
        ("AKIAIOSFODNN7EXAMPLE".into(), "aws"),
        ("xoxb-123456789012-abcdefghijkl".into(), "slack"),
        (
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U".into(),
            "jwt",
        ),
        (pat.clone(), "beton"),
        (format!("bt_run_{}", "0123456789abcdef".repeat(4)), "beton"),
    ];
    let r = Redactor::detectors_only();
    for (token, kind) in cases {
        let out = red(&r, &format!("x = {token} ;"));
        assert_eq!(out, format!("x = [REDACTED:{kind}] ;"), "{token}");
    }
}

#[test]
fn sec_013_detectors_need_context_or_checksum() {
    let r = Redactor::detectors_only();
    // AWS-Secret-Key nur mit Kontext; der Name bleibt sichtbar.
    let secret = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
    assert_eq!(
        red(&r, &format!("aws_secret_access_key = {secret}")),
        "aws_secret_access_key = [REDACTED:aws]"
    );
    assert_eq!(red(&r, &format!("hash {secret}")), format!("hash {secret}"));
    // Authorization-Header: Bearer und Basic.
    assert_eq!(
        red(&r, "Authorization: Bearer abcdef0123456789xyz"),
        "Authorization: Bearer [REDACTED:authorization]"
    );
    assert_eq!(
        red(&r, r#"{"authorization":"Basic dXNlcjpwYXNzd29yZA=="}"#),
        r#"{"authorization":"Basic [REDACTED:authorization]"}"#
    );
    // Prosa ist kein Header.
    assert_eq!(
        red(&r, "basic understanding of bearer authentication"),
        "basic understanding of bearer authentication"
    );
    // beton-Token mit falscher Prüfsumme, kurzes `sk-` und Wörter mit `sk-` bleiben.
    let mut wrong = beton_token("bt_pat_");
    wrong.replace_range(wrong.len() - 1.., "Z");
    let wrong = if wrong == beton_token("bt_pat_") {
        wrong.replace('Z', "Y")
    } else {
        wrong
    };
    for text in [
        wrong.as_str(),
        "sk-short",
        "risk-assessment-framework-documentation",
    ] {
        assert_eq!(red(&r, text), text);
    }
}

#[test]
fn sec_013_credential_placeholders_stay_visible() {
    let r = ready(&[KNOWN]);
    for text in [
        "Authorization: Bearer bt_cred_0123456789abcdef0123",
        "GITHUB_TOKEN=bt_cred_0123456789abcdef0123",
        r#"{"authorization":"token bt_cred_abcdefabcdef0123"}"#,
    ] {
        assert_eq!(red(&r, text), text);
    }
}

#[test]
fn sec_013_pem_block_from_begin_to_end() {
    let r = Redactor::detectors_only();
    for label in [
        "RSA PRIVATE KEY",
        "OPENSSH PRIVATE KEY",
        "EC PRIVATE KEY",
        "PRIVATE KEY",
    ] {
        let pem = format!(
            "-----BEGIN {label}-----\nMIIEowIBAAKCAQEAx8Yq\nZm9vYmFyYmF6cXV4\n-----END {label}-----"
        );
        assert_eq!(
            red(&r, &format!("Schlüssel:\n{pem}\nEnde")),
            "Schlüssel:\n[REDACTED:private_key]\nEnde",
            "{label}"
        );
    }
    // Abgeschnittene Ausgabe ohne END: ab BEGIN alles weg.
    assert_eq!(
        red(&r, "a -----BEGIN PRIVATE KEY-----\nMIIEowIBAAKCAQEAx8Yq"),
        "a [REDACTED:private_key]"
    );
    // Öffentliche Schlüssel bleiben.
    let public = "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkq\n-----END PUBLIC KEY-----";
    assert_eq!(red(&r, public), public);
}

#[test]
fn sec_013_beton_token_checksum_is_crc32_base62() {
    assert_eq!(detect::crc32(b"123456789"), 0xCBF4_3926);
    let sum = beton_token_checksum("AbCdEfGhIjKlMnOpQrStUvWxYz0123456789abcd");
    assert_eq!(sum.len(), 6);
    assert!(sum.chars().all(|c| c.is_ascii_alphanumeric()));
    assert_ne!(
        sum,
        beton_token_checksum("bbCdEfGhIjKlMnOpQrStUvWxYz0123456789abcd")
    );
}

#[test]
fn sec_013_redaction_is_idempotent_and_keeps_utf8() {
    let r = ready(&[KNOWN]);
    let text = format!("ä€😀{KNOWN}😀€ä {GITHUB}");
    let once = red(&r, &text);
    assert_eq!(once, "ä€😀[REDACTED:secret]😀€ä [REDACTED:github]");
    assert_eq!(red(&r, &once), once);
}

#[test]
fn sec_013_json_strings_are_redacted_in_place() {
    let r = ready(&[KNOWN]);
    let mut v = json!({
        "call_id": "c1",
        "n": 42,
        "ok": true,
        "args": { "command": format!("echo {KNOWN}"), "list": [GITHUB, "harmlos"] },
    });
    assert!(r.redact_json(&mut v).unwrap());
    assert_eq!(
        v,
        json!({
            "call_id": "c1",
            "n": 42,
            "ok": true,
            "args": { "command": "echo [REDACTED:secret]", "list": ["[REDACTED:github]", "harmlos"] },
        })
    );
    assert!(
        !r.redact_json(&mut v).unwrap(),
        "zweiter Lauf ändert nichts"
    );
}

#[test]
fn sec_013_fail_closed_until_known_values_are_loaded() {
    let r = Redactor::pending();
    assert_eq!(r.status(), RedactionStatus::Pending);
    assert_eq!(r.redact_str("x").unwrap_err(), Unavailable);
    assert_eq!(r.redact_json(&mut json!("x")).unwrap_err(), Unavailable);
    // Logs laufen während des Starts schon durch die Detektoren.
    assert_eq!(
        r.redact_log_str(&format!("token {GITHUB}")).unwrap(),
        "token [REDACTED:github]"
    );
    // Gescheiterter Aufbau: auch Logs werden zurückgehalten.
    r.fail();
    assert_eq!(r.status(), RedactionStatus::Failed);
    assert_eq!(r.redact_log_str("x").unwrap_err(), Unavailable);
    assert_eq!(r.redact_log_json(&mut json!("x")).unwrap_err(), Unavailable);
    // Store gesperrt: die zuletzt bekannten Werte bleiben aktiv.
    let r = ready(&[KNOWN]);
    r.mark_store_unavailable();
    assert_eq!(
        r.status(),
        RedactionStatus::Ready {
            values: 1,
            source: KnownSource::StoreUnavailable
        }
    );
    assert_eq!(red(&r, KNOWN), "[REDACTED:secret]");
}

#[test]
fn sec_013_debug_never_shows_values() {
    let values = vec![SecretValue::from(KNOWN)];
    let known = KnownSecrets::build(&values).unwrap();
    assert!(!format!("{known:?}").contains("known"), "{known:?}");
    let r = ready(&[KNOWN]);
    let debug = format!("{r:?}");
    assert!(!debug.contains(KNOWN) && !debug.contains("4711"), "{debug}");
}

/// Ein zufälliges Token je Detektor-Art bzw. ein bekannter Wert.
fn token() -> impl Strategy<Value = String> {
    let re = |p: &str| proptest::string::string_regex(p).unwrap();
    prop_oneof![
        re("ghp_[A-Za-z0-9]{36}"),
        re("glpat-[A-Za-z0-9]{20,30}"),
        re("sk-ant-api03-[A-Za-z0-9]{30,60}"),
        re("sk-proj-[A-Za-z0-9]{30,60}"),
        re("AKIA[0-9A-Z]{16}"),
        re("xoxp-[0-9]{10}-[A-Za-z0-9]{12}"),
        re("eyJ[A-Za-z0-9]{12,20}\\.eyJ[A-Za-z0-9]{12,20}\\.[A-Za-z0-9]{20}"),
        re("-----BEGIN RSA PRIVATE KEY-----\n[A-Za-z0-9]{40}\n-----END RSA PRIVATE KEY-----"),
        re("Zx9-known-SECRET-value-4711"),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn obs_002_ac2_embedded_patterns_never_survive(
        prefix in "[ -~\\n]{0,40}",
        sep_a in "[ \\n\\t\"'=:,;(]",
        tok in token(),
        sep_b in "[ \\n\\t\"'=:,;)]",
        suffix in "[ -~\\n]{0,40}",
    ) {
        let r = ready(&[KNOWN]);
        let text = format!("{prefix}{sep_a}{tok}{sep_b}{suffix}");
        let out = red(&r, &text);
        prop_assert!(!out.contains(&tok), "{tok:?} überlebt in {out:?}");
        prop_assert!(out.contains(REDACTED_PREFIX));
        // Auch als JSON-String und in einem Log-Feld.
        let mut v = json!({ "message": text });
        r.redact_json(&mut v).unwrap();
        prop_assert!(!v.to_string().contains(&tok));
    }
}
