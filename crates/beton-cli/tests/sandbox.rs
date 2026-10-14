//! `beton sandbox probe|explain` und die Sandbox-Konfiguration beim Laden (SBX-001 AC1, AC3,
//! SBX-003 AC1, AC5). Home, Workspace und Konfiguration liegen in Temp-Verzeichnissen.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::PathBuf;

use common::{beton, run, stderr, stdout};
use serde_json::Value;

struct Env {
    _tmp: tempfile::TempDir,
    beton_home: PathBuf,
    home: PathBuf,
    ws: PathBuf,
}

fn env(config: &str) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let beton_home = root.join("beton");
    let home = root.join("home");
    let ws = root.join("ws");
    for d in [&beton_home, &home, &ws] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(beton_home.join("config.yaml"), config).unwrap();
    Env {
        _tmp: tmp,
        beton_home,
        home,
        ws,
    }
}

fn sandbox(e: &Env, args: &[&str]) -> std::process::Output {
    run(beton(&e.beton_home)
        .env("HOME", &e.home)
        .current_dir(&e.ws)
        .arg("sandbox")
        .args(args))
}

fn json(o: &std::process::Output) -> Value {
    assert!(o.status.success(), "{}", stderr(o));
    serde_json::from_str(&stdout(o)).unwrap_or_else(|e| panic!("{e}: {}", stdout(o)))
}

#[test]
fn sbx_001_ac1_probe_json_lists_every_provider_with_caps_and_reasons() {
    let e = env("");
    let v = json(&sandbox(&e, &["probe", "--json"]));
    let providers = v["providers"].as_array().unwrap();
    let ids: Vec<&str> = providers
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    for id in [
        "seatbelt", "landlock", "bwrap", "windows", "docker", "podman", "none",
    ] {
        assert!(ids.contains(&id), "{id} fehlt: {ids:?}");
    }
    let all = [
        "fs_read",
        "fs_write",
        "masks",
        "net",
        "env",
        "proc",
        "syscall",
        "resources",
    ];
    for p in providers {
        let caps: Vec<&str> = p["caps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap())
            .collect();
        assert!(p["available"].is_boolean(), "{p}");
        for cap in all {
            if !caps.contains(&cap) {
                assert!(
                    p["degraded_reasons"][cap]
                        .as_str()
                        .is_some_and(|r| !r.is_empty()),
                    "{}: kein Grund für {cap}: {p}",
                    p["id"]
                );
            }
        }
    }
    let auto = if cfg!(target_os = "macos") {
        "seatbelt"
    } else if cfg!(target_os = "linux") {
        "landlock"
    } else {
        "windows"
    };
    assert_eq!(v["auto"], auto);
    // Ohne `--json`: eine Zeile je Backend.
    let o = sandbox(&e, &["probe"]);
    assert!(o.status.success(), "{}", stderr(&o));
    for id in ids {
        assert!(stdout(&o).contains(id), "{}", stdout(&o));
    }
}

const CONFIG: &str = "sandbox:\n  read_paths: [\"~/Documents\", \"~/.cache/fehlt\"]\n  egress_rules: [\"GET,HEAD registry.npmjs.org/**\"]\n  env:\n    set: { GEHEIM_X: \"FIXTURE-WERT\" }\n";

#[test]
fn sbx_003_ac5_explain_shows_roots_masks_env_names_and_rules_without_values() {
    let e = env(CONFIG);
    let v = json(&sandbox(&e, &["explain", "--json", "--stage", "tools"]));
    assert_eq!(v["stage"], "tools");
    let roots = |k: &str| -> Vec<String> {
        v[k].as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap().to_owned())
            .collect()
    };
    assert!(
        roots("read_roots").contains(&e.home.join("Documents").display().to_string()),
        "{v}"
    );
    assert!(
        roots("write_roots").contains(&e.ws.display().to_string()),
        "{v}"
    );
    let masks = v["masks"].to_string();
    assert!(
        masks.contains(&e.home.join(".ssh").display().to_string()),
        "{masks}"
    );
    assert!(masks.contains(".env"), "{masks}");
    let env_names: Vec<&str> = v["env"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    assert!(
        env_names.contains(&"GEHEIM_X") && env_names.contains(&"HOME"),
        "{env_names:?}"
    );
    assert_eq!(v["egress_rules"][0], "GET,HEAD registry.npmjs.org/**");
    assert!(v["notes"].to_string().contains(".cache/fehlt"), "{v}");
    assert!(
        !v.to_string().contains("FIXTURE-WERT"),
        "Env-Wert in der Ausgabe"
    );

    let o = sandbox(&e, &["explain"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let text = stdout(&o);
    for needle in [
        "Backend",
        "Stufe",
        "Lesen",
        "Schreiben",
        "Masken",
        "Environment",
        "GEHEIM_X",
        "registry.npmjs.org",
    ] {
        assert!(text.contains(needle), "{needle} fehlt:\n{text}");
    }
    assert!(!text.contains("FIXTURE-WERT"), "{text}");
}

#[test]
fn sbx_003_ac5_explain_stage_harness_grants_own_credentials() {
    let e = env("");
    std::fs::create_dir_all(e.home.join(".codex")).unwrap();
    std::fs::write(e.home.join(".codex/auth.json"), "FIXTURE").unwrap();
    let v = json(&sandbox(
        &e,
        &[
            "explain",
            "--json",
            "--stage",
            "harness",
            "--harness",
            "codex",
        ],
    ));
    assert_eq!(v["stage"], "harness");
    assert!(
        v["read_roots"].to_string().contains(".codex/auth.json"),
        "{v}"
    );
    let tools = json(&sandbox(&e, &["explain", "--json", "--stage", "tools"]));
    assert!(tools["masks"].to_string().contains(".codex"), "{tools}");
}

fn assert_config_error(config: &str, key: &str, needle: &str) {
    let e = env(config);
    let o = sandbox(&e, &["explain"]);
    assert!(!o.status.success(), "{}", stdout(&o));
    let err = stderr(&o);
    assert!(
        err.contains(key) && err.contains(needle),
        "{key}/{needle}: {err}"
    );
}

#[test]
fn sbx_003_ac1_write_paths_home_is_rejected_when_loading_the_config() {
    assert_config_error(
        "sandbox:\n  write_paths: [\"~\"]\n",
        "sandbox.write_paths[0]",
        "~",
    );
    assert_config_error(
        "sandbox:\n  read_paths: [\"$HOME/x\"]\n",
        "sandbox.read_paths[0]",
        "relativ",
    );
    assert_config_error(
        "sandbox:\n  egress_rules: [\"FETCH x.example\"]\n",
        "sandbox.egress_rules[0]",
        "FETCH",
    );
    // Auch Befehle ohne Sandbox-Bezug laden die Konfiguration und scheitern (fail closed).
    let e = env("sandbox:\n  write_paths: [\"~\"]\n");
    let o = run(beton(&e.beton_home)
        .env("HOME", &e.home)
        .args(["config", "list"]));
    assert!(!o.status.success());
    assert!(
        stderr(&o).contains("sandbox.write_paths[0]"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn sbx_001_ac3_unknown_backend_is_rejected_when_loading_the_config() {
    assert_config_error(
        "sandbox:\n  backend: firejail\n",
        "sandbox.backend",
        "firejail",
    );
    assert_config_error("sandbox:\n  limits: { pids: 3 }\n", "sandbox", "limits");
}

#[cfg(unix)]
#[test]
fn sbx_003_ac2_symlink_grant_into_ssh_is_path_escape_in_explain() {
    let e = env("sandbox:\n  read_paths: [\"~/link\"]\n");
    std::os::unix::fs::symlink(e.home.join(".ssh"), e.home.join("link")).unwrap();
    let o = sandbox(&e, &["explain"]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("path_escape"), "{}", stderr(&o));
}

#[test]
fn sbx_003_project_config_may_not_widen_the_sandbox() {
    let e = env("");
    let project = e.ws.join(".beton");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("config.yaml"),
        "sandbox:\n  read_paths: [\"~\"]\n",
    )
    .unwrap();
    // Ein Repo darf sich keine Grants geben (Projektebene nur `harnesses`, siehe Issue).
    let o = sandbox(&e, &["explain"]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("sandbox"), "{}", stderr(&o));
}
