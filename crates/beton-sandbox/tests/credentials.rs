//! SBX-005 und PRX-006 auf Ebene von Konfiguration und Auflösung (plattformunabhängig):
//! `sandbox.credentials`, Platzhalter im Environment der Stufe 2, Environment ohne Erben und
//! Credential-Dateien außerhalb aller Grants. Werte sind Fixtures, Homes Temp-Verzeichnisse.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use beton_sandbox::{
    CredentialType, ProviderId, ResolveCtx, SandboxError, SandboxSpec, Stage,
    check_credential_file, resolve,
};

const PLACEHOLDER: &str = "bt_cred_abcdefghijklmnopqrstuvwxyz234567";

struct Env {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    ws: PathBuf,
    scratch: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let (home, ws, scratch) = (root.join("home"), root.join("ws"), root.join("scratch"));
    for d in [&home, &ws, &scratch] {
        std::fs::create_dir_all(d).unwrap();
    }
    Env {
        _tmp: tmp,
        root,
        home,
        ws,
        scratch,
    }
}

impl Env {
    fn ctx(&self, stage: Stage) -> ResolveCtx {
        let mut c = ResolveCtx::new(stage, &self.ws, &self.home, &self.scratch);
        c.host_env = [
            ("PATH", "/usr/bin:/bin"),
            ("AWS_SECRET_ACCESS_KEY", "FIXTURE-AWS-ENV"),
            ("GH_TOKEN", "ghp_FIXTURE-REAL-TOKEN"),
            ("SSH_AUTH_SOCK", "/tmp/agent.sock"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        c
    }
}

fn invalid(yaml: &str) -> (String, String) {
    match SandboxSpec::from_yaml(yaml) {
        Err(SandboxError::InvalidSpec { path, message }) => (path, message),
        other => panic!("InvalidSpec erwartet für {yaml}: {other:?}"),
    }
}

#[test]
fn prx_006_credentials_are_loaded_and_validated() {
    let spec = SandboxSpec::from_yaml(
        "credentials:\n\
         \x20 - { name: github, type: github, source: { secret: \"secret://user/github.com/me\" }, env: [GH_TOKEN] }\n\
         \x20 - { name: reg, type: basic, username: deploy, hosts: [registry.test], source: { env: REG_PASSWORD } }\n\
         \x20 - { name: vault, type: bearer, hosts: [\"*.vault.test\"], source: { command: [/usr/local/bin/vault, read, x], ttl: 1m } }\n\
         \x20 - { name: file, type: git_https, hosts: [git.test], source: { file: \"~/.config/beton-token\" } }\n",
    )
    .unwrap();
    let creds = spec.credentials.as_ref().unwrap();
    assert_eq!(creds.len(), 4);
    assert_eq!(creds[0].kind, CredentialType::Github);
    assert_eq!(creds[2].source.ttl_secs().unwrap(), Some(60));
    // Fehler mit Feldangabe; nie still ignoriert.
    for (yaml, field) in [
        (
            "credentials: [{ name: A!, type: github, source: { env: X } }]\n",
            "credentials[0].name",
        ),
        (
            "credentials: [{ name: a, type: bearer, source: { env: X } }]\n",
            "credentials[0].hosts",
        ),
        (
            "credentials: [{ name: a, type: basic, hosts: [h.test], source: { env: X } }]\n",
            "credentials[0].username",
        ),
        (
            "credentials: [{ name: a, type: github, source: {} }]\n",
            "credentials[0].source",
        ),
        (
            "credentials: [{ name: a, type: github, source: { env: X, file: /x } }]\n",
            "credentials[0].source",
        ),
        (
            "credentials: [{ name: a, type: github, source: { secret: \"user/x\" } }]\n",
            "credentials[0].source.secret",
        ),
        (
            "credentials: [{ name: a, type: github, source: { env: BETON_TOKEN } }]\n",
            "credentials[0].source.env",
        ),
        (
            "credentials: [{ name: a, type: github, source: { file: relativ } }]\n",
            "credentials[0].source.file",
        ),
        (
            "credentials: [{ name: a, type: github, source: { command: [] } }]\n",
            "credentials[0].source.command",
        ),
        (
            "credentials: [{ name: a, type: github, source: { env: X, ttl: 1m } }]\n",
            "credentials[0].source.ttl",
        ),
        (
            "credentials: [{ name: a, type: github, source: { command: [x], ttl: bald } }]\n",
            "credentials[0].source.ttl",
        ),
        (
            "credentials: [{ name: a, type: github, source: { env: X }, env: [HTTPS_PROXY] }]\n",
            "credentials[0].env[0]",
        ),
        (
            "credentials: [{ name: a, type: github, source: { env: X }, env: [LD_PRELOAD] }]\n",
            "credentials[0].env[0]",
        ),
        (
            "credentials: [{ name: a, type: github, source: { env: X }, env: [\"GH_*\"] }]\n",
            "credentials[0].env[0]",
        ),
        (
            "credentials: [{ name: a, type: github, source: { env: X }, hosts: [\"1.2.3.4\"] }]\n",
            "credentials[0].hosts[0]",
        ),
        (
            "credentials:\n  - { name: a, type: github, source: { env: X } }\n  - { name: a, type: gitlab, source: { env: Y } }\n",
            "credentials[1].name",
        ),
        (
            "credentials:\n  - { name: a, type: github, source: { env: X }, env: [T] }\n  - { name: b, type: gitlab, source: { env: Y }, env: [T] }\n",
            "credentials[1].env[0]",
        ),
        (
            "env: { set: { GH_TOKEN: x } }\ncredentials: [{ name: a, type: github, source: { env: X }, env: [GH_TOKEN] }]\n",
            "credentials[0].env[0]",
        ),
    ] {
        let (path, message) = invalid(yaml);
        assert_eq!(path, field, "{yaml}: {message}");
    }
}

#[test]
fn prx_006_source_variable_must_not_be_passed_through() {
    // Sonst stünde der echte Wert der Quelle unter ihrem eigenen Namen in Stufe 1/2.
    for pass in ["GH_PAT", "GH_*", "GH_P*"] {
        let (path, message) = invalid(&format!(
            "env:\n  passthrough: [\"{pass}\"]\ncredentials: [{{ name: gh, type: github, source: {{ env: GH_PAT }}, env: [GH_TOKEN] }}]\n"
        ));
        assert_eq!(path, "credentials[0].source.env", "{pass}: {message}");
    }
    // Andere Variablen bleiben erlaubt.
    SandboxSpec::from_yaml(
        "env:\n  passthrough: [RUST_LOG]\ncredentials: [{ name: gh, type: github, source: { env: GH_PAT }, env: [GH_TOKEN] }]\n",
    )
    .unwrap();
}

#[test]
fn sbx_005_ac2_passthrough_of_ssh_auth_sock_is_a_configuration_error() {
    for name in [
        "SSH_AUTH_SOCK",
        "LD_PRELOAD",
        "DYLD_INSERT_LIBRARIES",
        "BETON_TOKEN",
        "DYLD_*",
    ] {
        let (path, message) = invalid(&format!("env:\n  passthrough: [\"{name}\"]\n"));
        assert_eq!(path, "env.passthrough[0]", "{name}");
        assert!(message.contains("SBX-005"), "{message}");
    }
}

#[test]
fn sbx_005_ac1_daemon_secrets_reach_neither_stage() {
    let e = env();
    for stage in [Stage::Harness, Stage::Tools] {
        let mut c = e.ctx(stage);
        c.harness = beton_sandbox::masks::harness_grants("claude");
        let p = resolve(ProviderId::Seatbelt, &SandboxSpec::default(), &c).unwrap();
        for k in ["AWS_SECRET_ACCESS_KEY", "GH_TOKEN", "SSH_AUTH_SOCK"] {
            assert!(!p.env.contains_key(k), "{stage:?}: {k}");
        }
        assert!(!p.env.values().any(|v| v.contains("FIXTURE")), "{stage:?}");
    }
}

#[test]
fn sbx_005_ac3_credential_variables_hold_only_the_placeholder() {
    let e = env();
    let mut c = e.ctx(Stage::Tools);
    c.credential_env = BTreeMap::from([("GH_TOKEN".to_owned(), PLACEHOLDER.to_owned())]);
    // Auch mit Passthrough des echten Werts gewinnt der Platzhalter.
    let spec = SandboxSpec::from_yaml("env:\n  passthrough: [\"GH_*\"]\n").unwrap();
    let p = resolve(ProviderId::Seatbelt, &spec, &c).unwrap();
    assert_eq!(p.env["GH_TOKEN"], PLACEHOLDER);
    assert!(!p.env.values().any(|v| v.contains("FIXTURE")));
    assert_eq!(p.credential_env, vec!["GH_TOKEN".to_owned()]);
    // `explain` nennt nur Namen.
    let text = format!("{:?}", beton_sandbox::explain::explain(&p));
    assert!(
        text.contains("GH_TOKEN") && !text.contains("bt_cred_"),
        "{text}"
    );
}

#[cfg(unix)]
#[test]
fn prx_006_ac4_file_source_inside_a_sandbox_grant_is_rejected() {
    let e = env();
    let inside = e.ws.join("token.txt");
    std::fs::write(&inside, "FIXTURE").unwrap();
    let docs = e.home.join("Documents");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("token"), "FIXTURE").unwrap();
    let outside = e.root.join("secret/token");
    std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
    std::fs::write(&outside, "FIXTURE").unwrap();
    let spec = SandboxSpec::from_yaml("read_paths: [\"~/Documents\"]\n").unwrap();
    let tools = resolve(ProviderId::Seatbelt, &spec, &e.ctx(Stage::Tools)).unwrap();
    let harness = resolve(ProviderId::Seatbelt, &spec, &e.ctx(Stage::Harness)).unwrap();
    let policies = [&tools, &harness];
    for bad in [inside.clone(), docs.join("token")] {
        match check_credential_file(&bad, &e.home, &policies) {
            Err(SandboxError::PathEscape { .. }) => {}
            other => panic!("{}: PathEscape erwartet, {other:?}", bad.display()),
        }
    }
    // Ein Symlink außerhalb, der in eine Freigabe zeigt, zählt wie sein Ziel.
    let link = e.root.join("link");
    std::os::unix::fs::symlink(&inside, &link).unwrap();
    assert!(check_credential_file(&link, &e.home, &policies).is_err());
    // Hardlinks und zu große Dateien sind ebenfalls Fehler.
    let hard = e.root.join("secret/hard");
    std::fs::hard_link(&outside, &hard).unwrap();
    assert!(check_credential_file(&outside, &e.home, &policies).is_err());
    std::fs::remove_file(&hard).unwrap();
    let big = e.root.join("secret/big");
    std::fs::write(&big, vec![b'x'; 64 * 1024 + 1]).unwrap();
    assert!(check_credential_file(&big, &e.home, &policies).is_err());
    assert!(check_credential_file(&e.root.join("fehlt"), &e.home, &policies).is_err());
    // Außerhalb aller Grants gelingt es.
    assert_eq!(
        check_credential_file(&outside, &e.home, &policies).unwrap(),
        outside.canonicalize().unwrap()
    );
}
