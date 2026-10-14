//! Querschnitt der öffentlichen API (plattformunabhängig): Ausgabe von Kindprozessen, Fehler,
//! Explain ohne Werte, Schema und Serialisierung.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use beton_sandbox::{
    BackendChoice, Cap, CommandSpec, ProviderId, ResolvedPolicy, Sandbox, SandboxError,
    SandboxSpec, Stage, Stdio,
};

#[test]
fn sbx_003_ac5_explain_lists_env_names_never_values() {
    let mut p = ResolvedPolicy::unsandboxed(Stage::Tools);
    p.env.insert("GH_TOKEN".into(), "FIXTURE-TOKEN-WERT".into());
    p.env.insert("HOME".into(), "/home/x".into());
    p.egress_rules = vec!["GET,HEAD registry.npmjs.org/**".into()];
    let v = beton_sandbox::explain::explain(&p);
    assert_eq!(v["env"], serde_json::json!(["GH_TOKEN", "HOME"]));
    assert_eq!(v["stage"], "tools");
    assert_eq!(v["backend"], "none");
    assert_eq!(v["egress_rules"][0], "GET,HEAD registry.npmjs.org/**");
    assert_eq!(
        v["degraded"],
        serde_json::json!(["fs_read", "fs_write", "masks", "net", "env", "proc"])
    );
    assert!(!v.to_string().contains("FIXTURE-TOKEN-WERT"));
}

#[test]
fn sbx_001_errors_have_codes_and_map_to_io_errors() {
    let unavailable = SandboxError::Unavailable {
        provider: ProviderId::Landlock,
        missing: [Cap::Net, Cap::FsRead].into_iter().collect(),
        hint: "Kernel ≥ 5.19".into(),
    };
    assert_eq!(unavailable.code(), "sandbox_unavailable");
    let text = unavailable.to_string();
    assert!(
        text.contains("landlock") && text.contains("fehlt: fs_read, net"),
        "{text}"
    );
    assert_eq!(
        std::io::Error::from(unavailable).kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let invalid = SandboxError::InvalidSpec {
        path: "write_paths[0]".into(),
        message: "x".into(),
    };
    assert_eq!(
        std::io::Error::from(invalid).kind(),
        std::io::ErrorKind::InvalidInput
    );
    let escape = SandboxError::PathEscape {
        path: "read_paths[0]".into(),
        message: "x".into(),
    };
    assert_eq!(escape.code(), "path_escape");
    assert_eq!(
        std::io::Error::from(escape).kind(),
        std::io::ErrorKind::PermissionDenied
    );
    for cap in Cap::ALL {
        assert_eq!(cap.to_string(), cap.as_str());
    }
    assert_eq!(beton_sandbox::caps::all().len(), Cap::ALL.len());
    for p in ProviderId::ALL {
        assert_eq!(ProviderId::parse(p.as_str()), Some(p));
        assert_eq!(p.to_string(), p.as_str());
    }
    assert_eq!(ProviderId::parse("firejail"), None);
}

#[test]
fn sbx_003_spec_round_trips_and_has_a_schema() {
    let yaml = "backend: seatbelt\nworkspace: ro\nread_paths: [\"~/x\"]\nmasks: [secrets/]\nallow_network: false\nenv: {passthrough: [\"LC_*\"], set: {CI: \"1\"}}\negress_allow_private: [\"127.0.0.1:5432\"]\n";
    let spec = SandboxSpec::from_yaml(yaml).unwrap();
    let back = SandboxSpec::from_value(serde_json::to_value(&spec).unwrap()).unwrap();
    assert_eq!(back, spec);
    assert_eq!(serde_json::to_value(BackendChoice::Auto).unwrap(), "auto");
    let schema = serde_json::to_value(schemars::schema_for!(SandboxSpec)).unwrap();
    let text = schema.to_string();
    for needle in [
        "read_paths",
        "write_paths",
        "allow_hidden",
        "egress_rules",
        "seatbelt",
    ] {
        assert!(text.contains(needle), "{needle}");
    }
    assert!(SandboxSpec::from_yaml("read_paths: [").is_err());
    assert!(
        SandboxSpec::from_yaml("allow_hidden: [venv]\n").is_err(),
        "nur Dotfiles"
    );
    assert!(SandboxSpec::from_yaml("egress_rules: [\" \"]\n").is_err());
    assert!(SandboxSpec::from_yaml("env: {set: {\"A-B\": x}}\n").is_err());
    assert!(SandboxSpec::from_yaml("masks: [\"..\"]\n").is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn sbx_001_child_output_collects_streams_and_times_out() {
    let sandbox = Sandbox::unsandboxed(Stage::Tools, None);
    let cmd = CommandSpec::new("/bin/sh")
        .args(["-c", "echo out; echo err >&2; exit 3"])
        .stdio(Stdio::Null, Stdio::Piped, Stdio::Piped);
    let out = sandbox
        .spawn(cmd)
        .unwrap()
        .output(Duration::from_secs(10))
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(out.stdout, b"out\n");
    assert_eq!(out.stderr, b"err\n");
    let cmd = CommandSpec::new("/bin/sh")
        .args(["-c", "printf %s \"$BETON_X\""])
        .env("BETON_X", "gesetzt")
        .stdio(Stdio::Null, Stdio::Piped, Stdio::Null);
    let out = sandbox
        .spawn(cmd)
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();
    assert_eq!(out.stdout, b"gesetzt");
    let slow = CommandSpec::new("/bin/sh").args(["-c", "sleep 30"]).stdio(
        Stdio::Null,
        Stdio::Piped,
        Stdio::Null,
    );
    let err = sandbox
        .spawn(slow)
        .unwrap()
        .output(Duration::from_millis(200))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
}

#[cfg(unix)]
#[tokio::test]
async fn sbx_001_child_signals_and_kills_the_whole_tree() {
    let sandbox = Sandbox::unsandboxed(Stage::Harness, None);
    let cmd = CommandSpec::new("/bin/sh")
        .args(["-c", "sleep 30 & wait"])
        .stdio(Stdio::Null, Stdio::Null, Stdio::Null);
    let mut child = sandbox.spawn(cmd).unwrap();
    assert!(child.id().is_some());
    assert!(child.try_wait().unwrap().is_none());
    child.signal_tree(15).unwrap();
    let status = child.wait().await.unwrap();
    assert!(!status.success());
    let mut child = sandbox
        .spawn(CommandSpec::new("/bin/sh").args(["-c", "sleep 30"]).stdio(
            Stdio::Null,
            Stdio::Null,
            Stdio::Null,
        ))
        .unwrap();
    child.start_kill().unwrap();
    assert!(!child.wait().await.unwrap().success());
    assert!(format!("{child:?}").contains("SandboxedChild"));
    assert!(format!("{sandbox:?}").contains("Harness"));
    assert!(format!("{:?}", beton_sandbox::Registry::platform()).contains("Seatbelt"));
}
