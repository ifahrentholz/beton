//! Deklarative Regeltests (PRX-003 AC1): `tests/egress/*.yaml` enthält Gruppen aus Regeln und
//! Fällen (Request → erwartete Entscheidung). Jede Datei läuft in der CI; ein neuer Fall ist
//! eine Zeile YAML.
//!
//! ```yaml
//! - name: Repo-Glob
//!   rules: ["GET api.github.com/repos/a/**"]
//!   cases:
//!     - { request: "GET https://api.github.com/repos/a/x", expect: allow }
//!     - { request: "GET https://api.github.com/repos/a%2F..%2Fb", expect: invalid }
//!     - { connect: "api.github.com:443", expect: allow }
//! - name: Ungültig
//!   invalid_rules: ["FOO example.test"]
//! ```
//!
//! `expect`: `allow`, `deny` oder `invalid` (Pfad bzw. Host ergibt 400). Optional `reason`
//! (`no_matching_rule`, `denied_by_rule`, `method_not_allowed`) und `rule` (Text der
//! auslösenden Regel).

#![allow(clippy::unwrap_used)]

use std::path::Path;

use beton_proxy::{Decision, Host, Request, RuleSet, Scheme, normalize_path};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    name: String,
    #[serde(default)]
    rules: Vec<String>,
    #[serde(default)]
    cases: Vec<Case>,
    #[serde(default)]
    invalid_rules: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    request: Option<String>,
    connect: Option<String>,
    expect: Expect,
    reason: Option<String>,
    rule: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Expect {
    Allow,
    Deny,
    Invalid,
}

/// Wertet einen Request-Fall aus: `Err(())` heißt 400 (ungültiger Host oder Pfad).
fn evaluate(set: &RuleSet, request: &str) -> Result<Decision, ()> {
    let (method, url) = request.split_once(' ').unwrap();
    let uri: http::Uri = url.parse().unwrap();
    let scheme = match uri.scheme_str() {
        Some("http") => Scheme::Http,
        Some("https") => Scheme::Https,
        other => panic!("Schema {other:?} in {request}"),
    };
    let host = Host::parse(uri.host().unwrap()).map_err(|_| ())?;
    let path = normalize_path(uri.path()).map_err(|_| ())?;
    Ok(set.decide(&Request {
        method,
        scheme,
        host: &host,
        port: uri.port_u16().unwrap_or(scheme.default_port()),
        path: &path,
    }))
}

fn connect(set: &RuleSet, authority: &str) -> Result<bool, ()> {
    let (host, port) = authority.rsplit_once(':').unwrap();
    let host = Host::parse(host).map_err(|_| ())?;
    Ok(set.may_connect(Scheme::Https, &host, port.parse().unwrap()))
}

fn run_case(file: &str, group: &Group, set: &RuleSet, case: &Case) -> Result<(), String> {
    let what = case
        .request
        .as_deref()
        .or(case.connect.as_deref())
        .unwrap_or("?");
    let ctx = format!("{file} / {} / {what}", group.name);
    match (&case.request, &case.connect) {
        (Some(request), None) => {
            let got = evaluate(set, request);
            let actual = match &got {
                Err(()) => Expect::Invalid,
                Ok(d) if d.is_allowed() => Expect::Allow,
                Ok(_) => Expect::Deny,
            };
            if actual != case.expect {
                return Err(format!("{ctx}: erwartet {:?}, bekam {got:?}", case.expect));
            }
            if let Some(reason) = &case.reason {
                match &got {
                    Ok(Decision::Deny { reason: r, .. }) if r.as_str() == reason => {}
                    _ => return Err(format!("{ctx}: Grund {reason} erwartet, bekam {got:?}")),
                }
            }
            if let Some(rule) = &case.rule {
                let actual_rule = match &got {
                    Ok(Decision::Allow { rule }) => Some(rule),
                    Ok(Decision::Deny { rule, .. }) => rule.as_ref(),
                    Err(()) => None,
                };
                if actual_rule != Some(rule) {
                    return Err(format!("{ctx}: Regel {rule:?} erwartet, bekam {got:?}"));
                }
            }
            Ok(())
        }
        (None, Some(authority)) => {
            let allowed = connect(set, authority);
            let actual = match allowed {
                Err(()) => Expect::Invalid,
                Ok(true) => Expect::Allow,
                Ok(false) => Expect::Deny,
            };
            if actual != case.expect {
                return Err(format!(
                    "{ctx}: erwartet {:?}, bekam {actual:?}",
                    case.expect
                ));
            }
            Ok(())
        }
        _ => Err(format!("{ctx}: genau eines von request/connect angeben")),
    }
}

#[test]
fn prx_003_ac1_declarative_rule_tests() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/egress");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .collect();
    files.sort();
    assert!(files.len() >= 8, "Regeltests fehlen in {}", dir.display());
    let mut failures = Vec::new();
    let mut cases = 0;
    for path in &files {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let groups: Vec<Group> =
            serde_norway::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        for group in &groups {
            for bad in &group.invalid_rules {
                cases += 1;
                if RuleSet::compile(&[bad]).is_ok() {
                    failures.push(format!(
                        "{file} / {}: {bad:?} müsste ungültig sein",
                        group.name
                    ));
                }
            }
            if group.cases.is_empty() {
                continue;
            }
            let set = match RuleSet::compile(&group.rules) {
                Ok(set) => set,
                Err(e) => {
                    failures.push(format!("{file} / {}: {e}", group.name));
                    continue;
                }
            };
            for case in &group.cases {
                cases += 1;
                if let Err(e) = run_case(&file, group, &set, case) {
                    failures.push(e);
                }
            }
        }
    }
    assert!(cases >= 80, "nur {cases} Fälle");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
