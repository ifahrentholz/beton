//! Zweistufiges Modell auf Ebene der Auflösung (SBX-002, PRX-008, SES-015 AC2): was Stufe 1 und
//! Stufe 2 bekommen (Grants der Adapter, Exec-Broker, Austausch, Proxy- und CA-Variablen). Die
//! Durchsetzung im Kernel prüfen `tests/seatbelt.rs`, `tests/linux.rs` und `tests/broker.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use beton_sandbox::{
    ExecAccess, Mask, NetMode, ProviderId, ProxyEndpoint, ResolveCtx, ResolvedPolicy, SandboxSpec,
    Secret, Stage, resolve,
};

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
    let home = root.join("home");
    let ws = root.join("ws");
    let scratch = root.join("scratch");
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
        let mut ctx = ResolveCtx::new(stage, &self.ws, &self.home, &self.scratch);
        ctx.adapters = beton_sandbox::masks::builtin_harness_grants();
        ctx
    }
}

const TOKEN: &str = "FIXTURE-PROXY-TOKEN-0815";

fn proxy(e: &Env) -> ProxyEndpoint {
    let bundle = e.root.join("ca/bundle.pem");
    std::fs::create_dir_all(bundle.parent().unwrap()).unwrap();
    std::fs::write(&bundle, "-----BEGIN CERTIFICATE-----\n").unwrap();
    ProxyEndpoint {
        port: 4711,
        url: Some(Secret::new(format!("http://bt:{TOKEN}@127.0.0.1:4711"))),
        ca_bundle: Some(bundle),
    }
}

fn res(backend: ProviderId, yaml: &str, ctx: &ResolveCtx) -> ResolvedPolicy {
    resolve(backend, &SandboxSpec::from_yaml(yaml).unwrap(), ctx).unwrap()
}

#[test]
fn sbx_002_builtin_grants_match_the_verified_clis() {
    let claude = beton_sandbox::masks::harness_grants("claude").unwrap();
    assert!(claude.state_paths.contains(&"~/.claude".to_owned()));
    assert!(claude.state_paths.contains(&"~/.claude.json".to_owned()));
    assert!(
        claude
            .credential_paths
            .contains(&"~/.claude/.credentials.json".to_owned())
    );
    // macOS: Login im Keychain (`Claude Code-credentials`), Linux: Datei.
    assert_eq!(claude.keychain, cfg!(target_os = "macos"));
    for host in [
        "api.anthropic.com",
        "platform.claude.com",
        "claude.ai",
        "mcp-proxy.anthropic.com",
    ] {
        assert!(claude.passthrough.contains(&host.to_owned()), "{host}");
    }
    assert!(claude.env_families.contains(&"CLAUDE".to_owned()));
    assert!(claude.env_families.contains(&"ANTHROPIC_".to_owned()));
    let codex = beton_sandbox::masks::harness_grants("codex").unwrap();
    assert!(
        codex
            .credential_paths
            .contains(&"~/.codex/auth.json".to_owned())
    );
    assert!(!codex.keychain);
    for host in [
        "chatgpt.com",
        "ab.chatgpt.com",
        "auth.openai.com",
        "api.openai.com",
    ] {
        assert!(codex.passthrough.contains(&host.to_owned()), "{host}");
    }
    assert!(codex.env_families.contains(&"CODEX_".to_owned()));
    assert!(codex.env_families.contains(&"OPENAI_".to_owned()));
}

#[test]
fn prx_008_proxy_and_ca_variables_for_the_toolchains() {
    let e = env();
    let mut ctx = e.ctx(Stage::Tools);
    ctx.proxy = Some(proxy(&e));
    ctx.host_env = BTreeMap::from([
        ("SSL_CERT_DIR".to_owned(), "/etc/ssl/certs".to_owned()),
        ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
    ]);
    let p = res(
        ProviderId::Seatbelt,
        "env: {passthrough: [\"SSL_*\"]}\n",
        &ctx,
    );
    assert_eq!(p.net, NetMode::ProxyOnly { port: 4711 });
    let url = format!("http://bt:{TOKEN}@127.0.0.1:4711");
    for k in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        assert_eq!(p.env.get(k), Some(&url), "{k}");
    }
    let bundle = e.root.join("ca/bundle.pem").display().to_string();
    for k in [
        "SSL_CERT_FILE",
        "REQUESTS_CA_BUNDLE",
        "CURL_CA_BUNDLE",
        "NODE_EXTRA_CA_CERTS",
        "GIT_SSL_CAINFO",
        "PIP_CERT",
        "CARGO_HTTP_CAINFO",
        "AWS_CA_BUNDLE",
        "DENO_CERT",
    ] {
        assert_eq!(p.env.get(k), Some(&bundle), "{k}");
    }
    assert_eq!(
        p.env.get("NODE_USE_ENV_PROXY").map(String::as_str),
        Some("1")
    );
    // Kein eigenes Zertifikatsverzeichnis neben dem Bundle, auch nicht per Passthrough.
    assert!(!p.env.contains_key("SSL_CERT_DIR"));
    // macOS teilt das Loopback mit dem Host: dort geht auch localhost über den Proxy.
    assert_eq!(p.env.get("NO_PROXY").map(String::as_str), Some(""));
    assert_eq!(p.env.get("no_proxy").map(String::as_str), Some(""));
    // Linux: Loopback ist sandbox-lokal (eigener Net-Namespace).
    let l = res(ProviderId::Landlock, "{}\n", &ctx);
    assert_eq!(
        l.env.get("NO_PROXY").map(String::as_str),
        Some("localhost,127.0.0.1,::1")
    );
    // Das Bundle ist lesbar.
    assert!(
        p.read_roots
            .iter()
            .any(|r| e.root.join("ca/bundle.pem").starts_with(r)),
        "{:?}",
        p.read_roots
    );
    // Das Token erscheint weder in `Debug` noch in `explain`.
    assert!(!format!("{ctx:?}").contains(TOKEN));
    assert!(
        !beton_sandbox::explain::explain(&p)
            .to_string()
            .contains(TOKEN)
    );
    // Ohne Netz keine Proxy-Variablen.
    let off = res(ProviderId::Seatbelt, "allow_network: false\n", &ctx);
    assert!(!off.env.contains_key("HTTPS_PROXY"));
}

#[test]
fn prx_008_dev_ports_put_loopback_into_no_proxy_on_macos() {
    let e = env();
    let mut ctx = e.ctx(Stage::Tools);
    ctx.proxy = Some(proxy(&e));
    let p = res(ProviderId::Seatbelt, "dev_ports: [3000]\n", &ctx);
    assert_eq!(p.dev_ports, vec![3000]);
    assert_eq!(
        p.env.get("NO_PROXY").map(String::as_str),
        Some("localhost,127.0.0.1,::1")
    );
}

#[test]
fn sbx_002_stage_one_gets_exec_access_xdg_and_session_marker() {
    let e = env();
    let exec_dir = e.root.join("exec");
    std::fs::create_dir_all(exec_dir.join("bin")).unwrap();
    let socket = exec_dir.join("exec.sock");
    std::fs::write(&socket, "").unwrap();
    let mut ctx = e.ctx(Stage::Harness);
    ctx.harness = beton_sandbox::masks::harness_grants("claude");
    ctx.session_id = Some("ses_test".into());
    ctx.exec = Some(ExecAccess {
        socket: socket.clone(),
        shim_dir: exec_dir.join("bin"),
        token: Secret::new("FIXTURE-EXEC-TOKEN".into()),
    });
    ctx.host_env = BTreeMap::from([
        ("CLAUDE_CONFIG_DIR".to_owned(), "/x".to_owned()),
        ("CODEX_HOME".to_owned(), "/y".to_owned()),
        ("XDG_RUNTIME_DIR".to_owned(), "/run/user/1".to_owned()),
    ]);
    let p = res(ProviderId::Seatbelt, "{}\n", &ctx);
    assert_eq!(p.unix_sockets, vec![socket.clone()]);
    assert_eq!(
        p.env.get("BETON_EXEC_SOCKET"),
        Some(&socket.display().to_string())
    );
    assert_eq!(
        p.env.get("BETON_EXEC_TOKEN").map(String::as_str),
        Some("FIXTURE-EXEC-TOKEN")
    );
    assert!(p.read_roots.contains(&exec_dir.join("bin")));
    assert_eq!(
        p.env.get("BETON_SESSION_ID").map(String::as_str),
        Some("ses_test")
    );
    // XDG_RUNTIME_DIR des Hosts nie, stattdessen der Scratch der Stufe.
    assert_eq!(
        p.env.get("XDG_RUNTIME_DIR"),
        Some(&e.scratch.display().to_string())
    );
    // Env-Familie des eigenen Harness, nicht die eines anderen.
    assert_eq!(
        p.env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
        Some("/x")
    );
    assert!(!p.env.contains_key("CODEX_HOME"));
    // Das Token erscheint nicht in `explain`.
    assert!(
        !beton_sandbox::explain::explain(&p)
            .to_string()
            .contains("FIXTURE-EXEC-TOKEN")
    );
    // Stufe 2 bekommt keinen Zugang zum Broker.
    let mut tools = e.ctx(Stage::Tools);
    tools.exec = ctx.exec.clone();
    let err = resolve(ProviderId::Seatbelt, &SandboxSpec::default(), &tools).unwrap_err();
    assert_eq!(err.code(), "invalid_spec", "{err}");
}

#[test]
fn sbx_002_shared_exchange_and_programs_are_granted() {
    let e = env();
    let shared = e.root.join("tools-tmp");
    let program = e.root.join("bin/beton");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::create_dir_all(program.parent().unwrap()).unwrap();
    std::fs::write(&program, "#!/bin/sh\n").unwrap();
    let mut ctx = e.ctx(Stage::Harness);
    ctx.shared = vec![shared.clone()];
    ctx.programs = vec![program.clone()];
    let p = res(ProviderId::Seatbelt, "{}\n", &ctx);
    assert!(p.write_roots.contains(&shared));
    assert!(p.read_roots.contains(&program));
    // Verbotene Wurzeln bleiben verboten, auch für beton-eigene Grants.
    let mut bad = e.ctx(Stage::Harness);
    bad.programs = vec![e.home.join(".ssh")];
    std::fs::create_dir_all(e.home.join(".ssh")).unwrap();
    assert!(resolve(ProviderId::Seatbelt, &SandboxSpec::default(), &bad).is_err());
}

#[test]
fn ses_015_ac2_worktree_under_beton_home_is_the_only_unmasked_part_of_it() {
    let e = env();
    let wt = e.home.join(".beton/worktrees/repo-1234abcd/beton-x");
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::create_dir_all(e.home.join(".beton/run")).unwrap();
    let mut ctx = ResolveCtx::new(Stage::Tools, &wt, &e.home, &e.scratch);
    ctx.adapters = beton_sandbox::masks::builtin_harness_grants();
    let p = res(ProviderId::Seatbelt, "{}\n", &ctx);
    assert!(p.write_roots.contains(&wt.canonicalize().unwrap()));
    // `~/.beton` bleibt maskiert.
    assert!(p.masks.iter().any(|m| matches!(
        m,
        Mask::Path { path, .. } if path.ends_with(".beton")
    )));
    // Ein anderes Verzeichnis unter `~/.beton` ist als Workspace verboten.
    let run = e.home.join(".beton/run");
    let ctx = ResolveCtx::new(Stage::Tools, &run, &e.home, &e.scratch);
    assert!(resolve(ProviderId::Seatbelt, &SandboxSpec::default(), &ctx).is_err());
}
