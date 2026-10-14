//! SBX-009: bubblewrap-Fallback. Version (AC2) und Argumente (AC3) prüfen reine Funktionen
//! auf allen Plattformen; unter Linux zusätzlich ein gefälschtes `bwrap` in alter Version. Die
//! Escape-Suite unter bwrap (AC1) läuft in `tests/linux.rs` für jedes Backend.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use beton_sandbox::linux::bwrap;
use beton_sandbox::linux::plan::{LaunchMode, LaunchPlan};
use beton_sandbox::{CommandSpec, ProviderId, ResolveCtx, SandboxSpec, Stage};

#[test]
fn sbx_009_ac2_versions_below_0_8_are_rejected_with_hint() {
    assert_eq!(bwrap::parse_version("bubblewrap 0.9.0\n"), Some((0, 9, 0)));
    assert_eq!(bwrap::parse_version("bubblewrap 0.11.0"), Some((0, 11, 0)));
    assert_eq!(bwrap::parse_version("bwrap 1.0"), Some((1, 0, 0)));
    assert_eq!(bwrap::parse_version("kein bwrap"), None);
    assert!(bwrap::check_version("bubblewrap 0.8.0").is_ok());
    assert!(bwrap::check_version("bubblewrap 0.10.0").is_ok());
    let err = bwrap::check_version("bubblewrap 0.7.0").unwrap_err();
    assert!(err.contains("0.7.0") && err.contains("0.8"), "{err}");
    let err = bwrap::check_version("Unsinn").unwrap_err();
    assert!(err.contains("Version"), "{err}");
}

fn plan(fx_root: &std::path::Path) -> LaunchPlan {
    let home = fx_root.join("home");
    let ws = fx_root.join("ws");
    let scratch = fx_root.join("scratch");
    for d in [&home, &ws, &scratch] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/id_ed25519"), "x").unwrap();
    std::fs::write(ws.join(".env"), "x").unwrap();
    let mut ctx = ResolveCtx::new(Stage::Tools, &ws, &home, &scratch);
    ctx.host_env = [
        ("PATH", "/usr/bin:/bin"),
        ("GH_TOKEN", "FIXTURE-HOST-TOKEN"),
        ("LANG", "C.UTF-8"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let spec = SandboxSpec::from_yaml(
        "read_paths: [\"~\"]\nenv:\n  passthrough: [GH_TOKEN]\n  set: { PLATZHALTER: bt_cred_FIXTUREFIXTUREFIXTUREFIXTURE12 }\n",
    )
    .unwrap();
    let policy = beton_sandbox::resolve(ProviderId::Bwrap, &spec, &ctx).unwrap();
    let cmd = CommandSpec::new("/bin/sh")
        .args(["-c", "true"])
        .env("BETON_TOOL_TOKEN", "FIXTURE-CMD-TOKEN");
    let mut plan = LaunchPlan::new(
        &policy,
        &cmd,
        LaunchMode::Bwrap {
            bwrap: "/usr/bin/bwrap".into(),
        },
    );
    plan.mask_file = Some(fx_root.join("maske"));
    plan
}

#[test]
fn sbx_009_ac3_bwrap_arguments_contain_no_environment_values() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let plan = plan(&root);
    // Das Environment steckt im Plan (er geht per Pipe an den Launcher), nie in argv.
    assert!(plan.env.iter().any(|(k, _)| k == "GH_TOKEN"));
    assert!(plan.env.iter().any(|(k, _)| k == "BETON_TOOL_TOKEN"));
    let args = bwrap::args(&plan, std::path::Path::new("/usr/bin/beton")).unwrap();
    let joined = args.join(" ");
    for forbidden in ["FIXTURE", "bt_cred_", "--setenv", "--unsetenv", "GH_TOKEN"] {
        assert!(!joined.contains(forbidden), "{forbidden} in {joined}");
    }
    // Namespaces, Aufräumen mit dem Elternprozess, Masken und der Launcher als Ziel.
    for needed in [
        "--unshare-pid",
        "--unshare-net",
        "--unshare-ipc",
        "--die-with-parent",
        "--proc",
        "--dev",
        "__sandbox-exec",
    ] {
        assert!(args.iter().any(|a| a == needed), "{needed} fehlt: {joined}");
    }
    let ssh = root.join("home/.ssh").display().to_string();
    let env = root.join("ws/.env").display().to_string();
    assert!(
        args.iter().any(|a| *a == ssh),
        "Maske ~/.ssh fehlt: {joined}"
    );
    assert!(args.iter().any(|a| *a == env), "Maske .env fehlt: {joined}");
    // Nach `--` folgen nur der Launcher und sein Modus, nicht das Ziel-Kommando.
    let tail = &args[args.iter().position(|a| a == "--").unwrap() + 1..];
    assert_eq!(tail, ["/usr/bin/beton", "__sandbox-exec"]);
}

#[cfg(target_os = "linux")]
#[test]
fn sbx_009_ac2_old_bwrap_is_reported_unavailable_with_version_hint() {
    use std::os::unix::fs::PermissionsExt as _;
    let tmp = tempfile::tempdir().unwrap();
    let fake = tmp.path().join("bwrap");
    std::fs::write(&fake, "#!/bin/sh\necho 'bubblewrap 0.7.0'\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut cfg = beton_sandbox::linux::LinuxConfig::new(env!("CARGO_BIN_EXE_beton-sandbox-exec"));
    cfg.bwrap = Some(fake);
    let probe = beton_sandbox::linux::bwrap_provider(cfg).probe();
    assert!(!probe.available, "{probe:?}");
    let reason = probe.unavailable_reason.unwrap();
    assert!(
        reason.contains("0.7.0") && reason.contains("0.8"),
        "{reason}"
    );
    // Fehlt bwrap ganz, nennt die Probe das ebenfalls.
    let mut cfg = beton_sandbox::linux::LinuxConfig::new(env!("CARGO_BIN_EXE_beton-sandbox-exec"));
    cfg.bwrap = Some(tmp.path().join("fehlt"));
    let probe = beton_sandbox::linux::bwrap_provider(cfg).probe();
    assert!(!probe.available);
    assert!(probe.unavailable_reason.unwrap().contains("bwrap"));
}
