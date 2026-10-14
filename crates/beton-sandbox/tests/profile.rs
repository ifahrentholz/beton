//! SBX-007 AC3 (plattformunabhängig): Der Profil-Generator übernimmt nie Pfade in den
//! SBPL-Text, und die regulären Ausdrücke der Masken treffen genau den maskierten Pfad.
//! Der Lauf gegen das echte `sandbox-exec` steht in `tests/seatbelt.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use beton_sandbox::resolve::{GitProtection, HiddenDotfiles};
use beton_sandbox::seatbelt::profile::{
    MAX_PROFILE_BYTES, PROFILE_VERSION, TEMPLATE, glob_regex, regex_escape, render,
};
use beton_sandbox::{Mask, NetMode, ProviderId, ResolvedPolicy, Stage};
use proptest::prelude::*;

fn policy_for(ws: &str) -> ResolvedPolicy {
    let ws = PathBuf::from(ws);
    let mut p = ResolvedPolicy::unsandboxed(Stage::Tools);
    p.backend = ProviderId::Seatbelt;
    p.required = beton_sandbox::caps::required_default();
    p.degraded.clear();
    p.workspace = ws.clone();
    p.scratch = ws.join("scratch");
    p.write_roots = vec![ws.clone(), ws.join("scratch")];
    p.read_roots = vec![ws.join("ro")];
    p.hidden = vec![HiddenDotfiles {
        root: ws.join("ro"),
        allow: vec![".venv".into()],
        writable: false,
    }];
    p.masks = vec![
        Mask::Path {
            path: ws.join("geheim"),
            source: "masks".into(),
        },
        Mask::Pattern {
            root: ws.clone(),
            glob: ".env.*".into(),
            any_depth: true,
            source: "masks".into(),
        },
    ];
    p.git = Some(GitProtection {
        dot_git: ws.join(".git"),
        dot_git_is_file: false,
        git_dir: ws.join(".git"),
        common_dir: ws.join(".git"),
    });
    p.net = NetMode::ProxyOnly { port: 4711 };
    p
}

/// Der feste Teil des Profils: alles außer den generierten Regelzeilen.
fn template_lines() -> Vec<&'static str> {
    TEMPLATE.lines().filter(|l| !l.starts_with(";;@")).collect()
}

#[test]
fn sbx_007_template_is_versioned_deny_default() {
    assert_eq!(PROFILE_VERSION, 1);
    assert!(TEMPLATE.starts_with("(version 1)"));
    assert!(TEMPLATE.contains("\n(deny default)\n"));
    // Kein DNS-Dienst, kein Keychain im festen Teil.
    for forbidden in ["dnssd", "mDNSResponder", "SecurityServer", "securityd", "network-outbound", "(allow default)"] {
        assert!(
            !template_lines().iter().any(|l| !l.trim_start().starts_with(";;") && l.contains(forbidden)),
            "{forbidden} im Template"
        );
    }
}

#[test]
fn sbx_007_generated_lines_only_reference_parameters() {
    let p = render(&policy_for("/Users/x/ws")).unwrap();
    let fixed = template_lines();
    let key = regex::Regex::new(r#"\(param "([A-Z0-9_]+)"\)"#).unwrap();
    for line in p.text.lines() {
        if fixed.contains(&line) {
            continue;
        }
        // Generierte Zeilen enthalten keinen Pfad, nur Schlüssel.
        assert!(!line.contains('/'), "Pfad im Profil: {line}");
        assert!(key.is_match(line), "Zeile ohne Parameter: {line}");
    }
    for (k, _) in &p.params {
        assert!(k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'), "{k}");
        assert!(p.text.contains(&format!("(param \"{k}\")")), "{k} unbenutzt");
    }
    assert!(p.params.iter().any(|(_, v)| v == "4711"));
}

#[test]
fn sbx_007_masks_come_after_grants_and_stage_two_has_no_keychain() {
    let mut policy = policy_for("/w");
    policy.keychain = true; // Stufe 2 ignoriert das immer.
    let p = render(&policy).unwrap();
    let pos = |needle: &str| p.text.find(needle).unwrap_or_else(|| panic!("{needle} fehlt"));
    let write = p.params.iter().find(|(_, v)| v == "/w").unwrap().0.clone();
    let mask = p.params.iter().find(|(_, v)| v == "/w/geheim").unwrap().0.clone();
    assert!(pos(&format!("(param \"{write}\")")) < pos(&format!("(param \"{mask}\")")));
    assert!(!p.text.contains("SecurityServer"));
    policy.stage = Stage::Harness;
    let p = render(&policy).unwrap();
    assert!(p.text.contains("com.apple.SecurityServer"));
    policy.keychain = false;
    assert!(!render(&policy).unwrap().text.contains("SecurityServer"));
}

#[test]
fn sbx_007_no_network_rule_without_proxy() {
    let mut policy = policy_for("/w");
    policy.net = NetMode::None;
    let p = render(&policy).unwrap();
    assert!(!p.text.contains("network-outbound"));
}

#[test]
fn sbx_007_oversized_profile_is_rejected() {
    let mut policy = policy_for("/w");
    policy.read_roots = (0..5000).map(|i| PathBuf::from(format!("/w/{i:0>60}"))).collect();
    let err = render(&policy).unwrap_err();
    assert_eq!(err.code(), "invalid_spec");
    assert!(err.to_string().contains(&MAX_PROFILE_BYTES.to_string()), "{err}");
}

#[test]
fn sbx_007_non_utf8_paths_are_rejected() {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let mut policy = policy_for("/w");
        policy
            .read_roots
            .push(PathBuf::from(std::ffi::OsStr::from_bytes(b"/w/\xff")));
        assert_eq!(render(&policy).unwrap_err().code(), "invalid_spec");
    }
}

fn path_segment() -> impl Strategy<Value = String> {
    // Zeichen, die SBPL oder reguläre Ausdrücke stören würden, plus Unicode.
    prop::collection::vec(
        prop_oneof![
            Just("\"".to_owned()),
            Just(")".to_owned()),
            Just("(".to_owned()),
            Just("\n".to_owned()),
            Just("\\".to_owned()),
            Just(";".to_owned()),
            Just("#".to_owned()),
            Just(".".to_owned()),
            Just("*".to_owned()),
            Just("[".to_owned()),
            Just("$".to_owned()),
            Just("^".to_owned()),
            Just("|".to_owned()),
            Just("ä".to_owned()),
            Just(" ".to_owned()),
            "[a-z]{1,3}",
        ],
        1..8,
    )
    .prop_map(|parts| parts.concat())
    .prop_filter("kein . oder ..", |s| s != "." && s != "..")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn sbx_007_ac3_workspace_names_never_reach_the_profile_text(seg in path_segment()) {
        let ws = format!("/tmp/{seg}/ws");
        let p = render(&policy_for(&ws)).unwrap();
        let fixed = template_lines();
        for line in p.text.lines() {
            if !fixed.contains(&line) {
                prop_assert!(!line.contains("/tmp"), "Pfad im Profil: {line}");
            }
        }
        // Gleiche Struktur unabhängig vom Namen: nur die Werte unterscheiden sich.
        let reference = render(&policy_for("/tmp/a/ws")).unwrap();
        prop_assert_eq!(&p.text, &reference.text);
        prop_assert_eq!(p.params.len(), reference.params.len());
        prop_assert!(p.params.iter().any(|(_, v)| v == &ws));
    }

    #[test]
    fn sbx_007_ac3_escaped_paths_match_only_themselves(seg in path_segment(), other in path_segment()) {
        let path = format!("/tmp/{seg}");
        let re = regex::Regex::new(&format!("(?s)^{}(/.*)?$", regex_escape(&path))).unwrap();
        prop_assert!(re.is_match(&path));
        let child = format!("{path}/x");
        prop_assert!(re.is_match(&child));
        let sibling = format!("/tmp/{other}");
        prop_assert_eq!(re.is_match(&sibling), sibling == path || sibling.starts_with(&format!("{path}/")));
        let longer = format!("{path}x");
        prop_assert!(!re.is_match(&longer));
    }

    #[test]
    fn sbx_007_ac3_glob_masks_match_one_segment(name in "[a-z.]{1,6}", other in "[a-z./]{1,8}") {
        let glob = format!("{name}*");
        let re = regex::Regex::new(&format!("^{}$", glob_regex(&glob))).unwrap();
        let hit = format!("{name}x");
        prop_assert!(re.is_match(&hit));
        prop_assert_eq!(re.is_match(&other), other.starts_with(&name) && !other.contains('/'));
    }
}
