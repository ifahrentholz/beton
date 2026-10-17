//! Fuzz-Targets (QA-009) und die Pfade, deren Änderung sie betrifft.
//!
//! `cargo xtask fuzz-targets` gibt alle Targets aus, `--affected --range <base>..<head>` nur
//! die, deren Quellen der Diff ändert (die CI lässt diese 60 s laufen, QA-009 AC2). Die
//! Harnesses liegen unter `fuzz/fuzz_targets/<target>.rs`, die Seed-Korpora unter
//! `fuzz/corpus/<target>/`.

use crate::security_tests::Change;

/// Ein Fuzz-Target und die Pfadpräfixe seines geprüften Codes.
#[derive(Debug, Clone, Copy)]
pub struct Target {
    pub name: &'static str,
    pub paths: &'static [&'static str],
}

const PROXY: &[&str] = &["crates/beton-proxy/"];
const POLICY: &[&str] = &["crates/beton-policy/"];

/// Alle Targets; jedes hat einen Harness, einen `[[bin]]`-Eintrag und ein Seed-Korpus.
pub const TARGETS: [Target; 10] = [
    // HTTP/1.1-Requests und CONNECT durch eine echte Proxy-Instanz (hyper parst).
    Target {
        name: "http_request",
        paths: PROXY,
    },
    Target {
        name: "http_connect",
        paths: PROXY,
    },
    Target {
        name: "tls_client_hello",
        paths: PROXY,
    },
    Target {
        name: "egress_rules",
        paths: PROXY,
    },
    Target {
        name: "credential_placeholders",
        paths: PROXY,
    },
    // WebSocket-Upgrade im Proxy (einen eigenen Frame-Decoder gibt es nicht).
    Target {
        name: "ws_upgrade",
        paths: PROXY,
    },
    // Nachrichten in den Text-Frames des Daemon-WebSockets.
    Target {
        name: "ws_client_msg",
        paths: &[
            "crates/beton-proto/",
            "crates/beton-core/src/id.rs",
            "crates/beton-server/src/ws.rs",
        ],
    },
    Target {
        name: "cel_eval",
        paths: POLICY,
    },
    Target {
        name: "policy_yaml",
        paths: POLICY,
    },
    Target {
        name: "sbpl_escape",
        paths: &[
            "crates/beton-sandbox/src/seatbelt/",
            "crates/beton-sandbox/src/resolve.rs",
        ],
    },
];

/// Änderungen hieran betreffen alle Targets (Fuzz-Crate, Abhängigkeiten, diese Liste).
pub const SHARED: [&str; 4] = [
    "fuzz/Cargo.toml",
    "fuzz/Cargo.lock",
    "fuzz/src/",
    "xtask/src/fuzz.rs",
];

/// Ändert `path` den Code oder Harness bzw. Korpus von `t`?
fn touches(t: &Target, path: &str) -> bool {
    SHARED.iter().any(|p| path.starts_with(p))
        || t.paths.iter().any(|p| path.starts_with(p))
        || path == format!("fuzz/fuzz_targets/{}.rs", t.name)
        || path.starts_with(&format!("fuzz/corpus/{}/", t.name))
}

/// Die Targets, die von `changes` betroffen sind (in der Reihenfolge von [`TARGETS`]).
pub fn affected(changes: &[Change]) -> Vec<&'static str> {
    TARGETS
        .iter()
        .filter(|t| changes.iter().any(|c| touches(t, &c.path)))
        .map(|t| t.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::repo_root;

    fn change(path: &str) -> Change {
        Change {
            status: "M".into(),
            path: path.into(),
        }
    }

    #[test]
    fn qa_009_ac1_every_target_has_a_harness_and_a_seed_corpus() {
        let fuzz = repo_root().join("fuzz");
        let manifest: toml::Table = std::fs::read_to_string(fuzz.join("Cargo.toml"))
            .unwrap()
            .parse()
            .unwrap();
        let bins: Vec<(String, String)> = manifest["bin"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                (
                    b["name"].as_str().unwrap().to_owned(),
                    b["path"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        let names: BTreeSet<&str> = TARGETS.iter().map(|t| t.name).collect();
        assert_eq!(names.len(), TARGETS.len(), "doppelter Target-Name");
        for t in &TARGETS {
            let harness = format!("fuzz_targets/{}.rs", t.name);
            assert!(fuzz.join(&harness).is_file(), "{harness} fehlt");
            assert!(
                bins.iter().any(|(n, p)| n == t.name && *p == harness),
                "[[bin]] `{}` mit path `{harness}` fehlt in fuzz/Cargo.toml",
                t.name
            );
            let corpus = fuzz.join("corpus").join(t.name);
            let seeds = std::fs::read_dir(&corpus)
                .unwrap_or_else(|e| panic!("{}: {e}", corpus.display()))
                .filter_map(Result::ok)
                .filter(|e| e.path().is_file())
                .count();
            assert!(seeds > 0, "Seed-Korpus von {} ist leer", t.name);
            assert!(!t.paths.is_empty(), "{} ohne Quellpfade", t.name);
        }
        // Umgekehrt: kein Harness und kein [[bin]] ohne Eintrag in TARGETS.
        for (n, _) in &bins {
            assert!(names.contains(n.as_str()), "[[bin]] `{n}` fehlt in TARGETS");
        }
        for entry in std::fs::read_dir(fuzz.join("fuzz_targets")).unwrap() {
            let path = entry.unwrap().path();
            let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
            assert!(
                names.contains(stem.as_str()),
                "Harness {} fehlt in TARGETS",
                path.display()
            );
        }
    }

    #[test]
    fn qa_009_ac2_proxy_change_selects_all_proxy_targets() {
        let proxy = [
            "http_request",
            "http_connect",
            "tls_client_hello",
            "egress_rules",
            "credential_placeholders",
            "ws_upgrade",
        ];
        let selected = affected(&[change("crates/beton-proxy/src/proxy.rs")]);
        assert_eq!(selected, proxy, "Änderung in beton-proxy");
        assert!(affected(&[change("docs/spec/12-distribution-quality.md")]).is_empty());
        assert!(affected(&[change("crates/beton-cli/src/main.rs")]).is_empty());
    }

    #[test]
    fn qa_009_ac2_harness_corpus_and_shared_changes() {
        assert_eq!(
            affected(&[change("fuzz/corpus/cel_eval/shell.txt")]),
            ["cel_eval"]
        );
        assert_eq!(
            affected(&[change("fuzz/fuzz_targets/sbpl_escape.rs")]),
            ["sbpl_escape"]
        );
        assert_eq!(
            affected(&[change("crates/beton-policy/src/cel/eval.rs")]),
            ["cel_eval", "policy_yaml"]
        );
        assert_eq!(affected(&[change("fuzz/src/e2e.rs")]).len(), TARGETS.len());
        assert_eq!(affected(&[change("fuzz/Cargo.lock")]).len(), TARGETS.len());
    }
}
