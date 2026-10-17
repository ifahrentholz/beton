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

/// SBX-008: `beton __sandbox-exec` ist der Linux-Launcher; ohne Plan vom Runner startet er
/// nichts (fail closed) und endet mit 125.
#[cfg(unix)]
#[test]
fn sbx_008_sandbox_exec_without_plan_fails_closed() {
    let e = env("");
    let o = run(beton(&e.beton_home).arg("__sandbox-exec"));
    assert_eq!(o.status.code(), Some(125), "{}", stderr(&o));
    assert!(stderr(&o).contains("sandbox"), "{}", stderr(&o));
}

/// SBX-008: Unter Linux meldet die Probe das Landlock-Backend des `beton`-Binarys mit ABI und
/// Kernel. In der CI muss es verfügbar sein.
#[cfg(target_os = "linux")]
#[test]
fn sbx_008_probe_reports_landlock_abi_and_kernel() {
    let e = env("");
    let v = json(&sandbox(&e, &["probe", "--json"]));
    let landlock = v["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "landlock")
        .unwrap()
        .clone();
    if std::env::var_os("CI").is_some() {
        assert_eq!(landlock["available"], true, "{landlock}");
    }
    if landlock["available"] == true {
        let version = landlock["version"].as_str().unwrap();
        assert!(
            version.contains("ABI") && version.contains("Kernel"),
            "{version}"
        );
        assert!(
            landlock["caps"].to_string().contains("syscall"),
            "{landlock}"
        );
    }
}

#[test]
fn sbx_012_ac3_explain_shows_which_values_come_from_the_preset() {
    let e = env("sandbox:\n  preset: dev\n  workspace: ro\n  masks: [secrets/]\n");
    let v = json(&sandbox(&e, &["explain", "--json", "--stage", "tools"]));
    assert_eq!(v["origins"]["egress_rules"], "preset:dev", "{v}");
    assert_eq!(v["origins"]["write_paths"], "preset:dev", "{v}");
    assert_eq!(v["origins"]["workspace"], "user", "{v}");
    assert_eq!(v["origins"]["masks[secrets/]"], "user", "{v}");
    assert!(
        v["egress_rules"][0]
            .as_str()
            .unwrap()
            .contains("registry.npmjs.org"),
        "{v}"
    );
    let o = sandbox(&e, &["explain"]);
    let text = stdout(&o);
    assert!(text.contains("preset:dev"), "{text}");
}

#[test]
fn prx_002_extra_ca_files_must_exist_and_contain_a_certificate() {
    let e = env("");
    let missing = e.home.join("fehlt.pem");
    std::fs::write(
        e.beton_home.join("config.yaml"),
        format!("proxy:\n  extra_ca_files: [\"{}\"]\n", missing.display()),
    )
    .unwrap();
    let o = sandbox(&e, &["explain", "--json"]);
    assert!(!o.status.success(), "{}", stdout(&o));
    assert!(
        stderr(&o).contains("proxy.extra_ca_files"),
        "{}",
        stderr(&o)
    );
    let empty = e.home.join("leer.pem");
    std::fs::write(&empty, "").unwrap();
    std::fs::write(
        e.beton_home.join("config.yaml"),
        format!("proxy:\n  extra_ca_files: [\"{}\"]\n", empty.display()),
    )
    .unwrap();
    let o = sandbox(&e, &["explain", "--json"]);
    assert!(!o.status.success(), "{}", stdout(&o));
    // Im Projekt ist der Schlüssel verboten (nur User-Konfiguration).
    std::fs::write(e.beton_home.join("config.yaml"), "").unwrap();
    std::fs::create_dir_all(e.ws.join(".beton")).unwrap();
    std::fs::write(
        e.ws.join(".beton/config.yaml"),
        "proxy:\n  extra_ca_files: [\"/etc/x.pem\"]\n",
    )
    .unwrap();
    let o = sandbox(&e, &["explain", "--json"]);
    assert!(!o.status.success(), "{}", stdout(&o));
}

/// SBX-010 AC1: `beton sandbox probe --json` liefert pro Backend `available`, `caps`,
/// `degraded_reasons` und unter Linux `landlock_abi` und `kernel`.
#[test]
fn sbx_010_ac1_probe_json_has_per_backend_fields_and_linux_kernel_info() {
    let e = env("");
    let v = json(&sandbox(&e, &["probe", "--json"]));
    for p in v["providers"].as_array().unwrap() {
        assert!(p["available"].is_boolean(), "{p}");
        assert!(p["caps"].is_array(), "{p}");
        assert!(p["degraded_reasons"].is_object(), "{p}");
        let linux = cfg!(target_os = "linux") && (p["id"] == "landlock" || p["id"] == "bwrap");
        if linux {
            assert!(
                p["landlock_abi"].is_u64() || p.get("landlock_abi").is_some_and(|a| a.is_null()),
                "{p}"
            );
            assert!(p["kernel"].as_str().is_some_and(|k| !k.is_empty()), "{p}");
        } else {
            assert!(p.get("kernel").is_none(), "{p}");
        }
    }
}

/// SBX-010 AC2: `beton sandbox exec --stage tools -- cat ~/.ssh/config` scheitert mit Exit ≠ 0
/// und klarer Meldung; der Inhalt erscheint nirgends. Ein erlaubtes Kommando im Workspace
/// gelingt (Kontrollgruppe: dieselbe Sandbox lässt Erlaubtes zu).
#[cfg(unix)]
#[test]
fn sbx_010_ac2_exec_in_the_tools_stage_cannot_read_ssh_config() {
    const MARKER: &str = "FIXTURE-SSH-CONFIG";
    let e = env("");
    let config = e.home.join(".ssh/config");
    std::fs::write(&config, format!("Host fixture\n  IdentityFile {MARKER}\n")).unwrap();
    // Kontrollgruppe ohne Sandbox: Die Datei ist lesbar.
    assert!(std::fs::read_to_string(&config).unwrap().contains(MARKER));
    let o = sandbox(
        &e,
        &[
            "exec",
            "--stage",
            "tools",
            "--",
            "cat",
            config.to_str().unwrap(),
        ],
    );
    assert!(!o.status.success(), "{}", stdout(&o));
    assert_ne!(o.status.code(), Some(0));
    assert!(!stdout(&o).contains(MARKER), "Inhalt in stdout");
    let err = stderr(&o);
    assert!(!err.contains(MARKER), "Inhalt in stderr");
    assert!(
        err.contains("Sandbox") && err.contains("tools") && err.contains("explain"),
        "keine klare Meldung: {err}"
    );
    std::fs::write(e.ws.join("hallo.txt"), "hallo").unwrap();
    let o = sandbox(&e, &["exec", "--stage", "tools", "--", "cat", "hallo.txt"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o), "hallo");
}

/// SBX-010 AC2 (fail closed): `beton sandbox exec` startet nichts ohne volle Isolation, weder
/// mit `backend: none` (Datei oder `BETON_CFG_SANDBOX__BACKEND`) noch mit einer *degraded*
/// Policy (z. B. `limits.memory_mb` ohne durchsetzbares Speicherlimit).
#[cfg(unix)]
#[test]
fn sbx_010_ac2_exec_refuses_backend_none_and_degraded_policies() {
    let refused = |o: &std::process::Output| {
        assert!(
            !o.status.success(),
            "lief ohne volle Isolation: {}",
            stdout(o)
        );
        assert_eq!(o.status.code(), Some(1), "{}", stderr(o));
        assert!(!stdout(o).contains("hallo"), "Kommando lief: {}", stdout(o));
        let err = stderr(o);
        assert!(
            err.contains("explain") && err.contains("nichts gestartet"),
            "{err}"
        );
    };
    let e = env("sandbox:\n  backend: none\n");
    std::fs::write(e.ws.join("hallo.txt"), "hallo").unwrap();
    refused(&sandbox(&e, &["exec", "--", "cat", "hallo.txt"]));

    let e = env("");
    std::fs::write(e.ws.join("hallo.txt"), "hallo").unwrap();
    let o = run(beton(&e.beton_home)
        .env("HOME", &e.home)
        .env("BETON_CFG_SANDBOX__BACKEND", "none")
        .current_dir(&e.ws)
        .args(["sandbox", "exec", "--", "cat", "hallo.txt"]));
    refused(&o);

    // Speicherlimit: nur wenn das Backend `resources` nicht erzwingt, ist die Policy degraded.
    let e = env("sandbox:\n  limits: { memory_mb: 256 }\n");
    std::fs::write(e.ws.join("hallo.txt"), "hallo").unwrap();
    let probe = json(&sandbox(&e, &["probe", "--json"]));
    let auto = probe["auto"].as_str().unwrap().to_owned();
    let enforced = probe["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == auto.as_str())
        .unwrap()["caps"]
        .to_string()
        .contains("resources");
    let o = sandbox(&e, &["exec", "--", "cat", "hallo.txt"]);
    if enforced {
        assert!(o.status.success(), "{}", stderr(&o));
    } else {
        refused(&o);
    }
}

/// Ergebnis von `beton sandbox test --json`; auch bei Exit ≠ 0 steht der Bericht in stdout.
fn sandbox_test(o: &std::process::Output) -> Value {
    serde_json::from_str(&stdout(o))
        .unwrap_or_else(|e| panic!("{e}: {} / {}", stdout(o), stderr(o)))
}

/// SBX-010 AC3: `beton sandbox test` führt die Selbsttests der Probe aus (die vorhandenen Fälle
/// der Backends dieser Plattform, ADR-0042) und endet mit Exit 0 nur, wenn alle Pflichtfälle
/// bestanden sind. Pflicht ist das Backend, das eine Session hier nähme (`auto`).
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn sbx_010_ac3_sandbox_test_passes_with_the_backend_of_this_platform() {
    let e = env("");
    let o = sandbox(&e, &["test", "--json"]);
    let v = sandbox_test(&o);
    assert!(o.status.success(), "{v}\n{}", stderr(&o));
    assert_eq!(v["passed"], true, "{v}");
    let native: &[&str] = if cfg!(target_os = "macos") {
        &["seatbelt"]
    } else {
        &["landlock", "bwrap"]
    };
    let cases = v["cases"].as_array().unwrap();
    let ids: Vec<&str> = cases
        .iter()
        .map(|c| c["backend"].as_str().unwrap())
        .collect();
    assert_eq!(ids, native, "{v}");
    let required: Vec<&Value> = cases.iter().filter(|c| c["required"] == true).collect();
    assert_eq!(required.len(), 1, "{v}");
    assert_eq!(required[0]["backend"], v["backend"], "{v}");
    assert_eq!(required[0]["passed"], true, "{v}");
    assert!(
        required[0]["id"].as_str().unwrap().starts_with("selftest:"),
        "{v}"
    );
    // Text: jeder Fall mit Ergebnis.
    let o = sandbox(&e, &["test"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let text = stdout(&o);
    assert!(text.contains("bestanden"), "{text}");
}

/// SBX-010 AC3 (fail closed): Ist kein Backend nutzbar, endet `beton sandbox test` mit Exit 1;
/// der Bericht nennt den Grund.
#[test]
fn sbx_010_ac3_sandbox_test_fails_without_a_usable_backend() {
    let e = env("");
    let o = run(beton(&e.beton_home)
        .env("HOME", &e.home)
        .env("BETON_TEST_SANDBOX_UNAVAILABLE", "all")
        .current_dir(&e.ws)
        .args(["sandbox", "test", "--json"]));
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    let v = sandbox_test(&o);
    assert_eq!(v["passed"], false, "{v}");
    let required: Vec<&Value> = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["required"] == true)
        .collect();
    assert_eq!(required.len(), 1, "{v}");
    assert_eq!(required[0]["passed"], false, "{v}");
    assert!(
        required[0]["reason"]
            .as_str()
            .is_some_and(|r| !r.is_empty()),
        "{v}"
    );
    assert!(stderr(&o).contains("Pflichtfall"), "{}", stderr(&o));
}

/// SBX-010 AC3: Pflicht ist das konfigurierte Backend; `none` isoliert nichts und ein
/// nicht verfügbares Backend (Docker bis M5) besteht nicht.
#[test]
fn sbx_010_ac3_sandbox_test_requires_the_configured_backend() {
    for backend in ["none", "docker"] {
        let e = env(&format!("sandbox:\n  backend: {backend}\n"));
        let o = sandbox(&e, &["test", "--json"]);
        assert_eq!(o.status.code(), Some(1), "{backend}: {}", stderr(&o));
        let v = sandbox_test(&o);
        assert_eq!(v["passed"], false, "{v}");
        assert_eq!(v["backend"], backend, "{v}");
        let required: Vec<&Value> = v["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["required"] == true)
            .collect();
        assert_eq!(required.len(), 1, "{v}");
        assert_eq!(required[0]["backend"], backend, "{v}");
        assert_eq!(required[0]["passed"], false, "{v}");
    }
}
