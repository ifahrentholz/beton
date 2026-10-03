//! Subscription-Regel (HAR-015): beton reicht keine Anthropic-Keys an die CLI weiter und
//! fasst keine Vendor-Credentials an.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

const CHILD_ENV: &str = "BETON_TEST_ENV_OUT";

/// Kindprozess: startet den Adapter mit einem Skript als `claude`, das seine Umgebung
/// ausgibt. Läuft nur, wenn der Elterntest ihn mit `BETON_TEST_ENV_OUT` startet.
#[cfg(unix)]
#[tokio::test]
#[ignore = "wird von har_015_ac1_api_keys_do_not_reach_the_cli gestartet"]
async fn env_child() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    use beton_harness::process::RealLauncher;
    use beton_harness::{AdapterContext, AllowAll, HarnessAdapter, HostEnv, SessionSpec};
    use beton_harness_claude::ClaudeAdapter;

    let Ok(out) = std::env::var(CHILD_ENV) else {
        return;
    };
    let dir = PathBuf::from(&out).parent().unwrap().to_path_buf();
    let script = dir.join("claude");
    std::fs::write(&script, format!("#!/bin/sh\nenv > '{out}'\n")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut env = HostEnv::default();
    env.vars
        .insert("BETON_CLAUDE_PATH".into(), script.display().to_string());
    let ctx = AdapterContext {
        gate: Arc::new(AllowAll),
        launcher: Arc::new(RealLauncher),
        env,
    };
    let mut session = ClaudeAdapter::default()
        .start(
            SessionSpec {
                workdir: dir.clone(),
                ..SessionSpec::default()
            },
            ctx,
        )
        .await
        .unwrap();
    let mut rx = session.events().unwrap();
    let exited = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while let Some(e) = rx.recv().await {
            if matches!(e.payload, beton_core::event::EventPayload::HarnessExited(_)) {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(
        exited,
        Ok(true),
        "Skript endet sofort; harness.exited erwartet"
    );
}

#[cfg(unix)]
#[test]
fn har_015_ac1_api_keys_do_not_reach_the_cli() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("env.txt");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["env_child", "--exact", "--ignored", "--nocapture"])
        .env(CHILD_ENV, &out)
        .env("ANTHROPIC_API_KEY", "test-key-darf-nicht-ankommen")
        .env("ANTHROPIC_AUTH_TOKEN", "test-token-darf-nicht-ankommen")
        .env("BETON_TEST_MARKER", "geerbt")
        .status()
        .unwrap();
    assert!(status.success());
    let env = std::fs::read_to_string(&out).unwrap();
    assert!(
        env.contains("BETON_TEST_MARKER=geerbt"),
        "Umgebung wird sonst geerbt"
    );
    assert!(!env.contains("ANTHROPIC_API_KEY"), "{env}");
    assert!(!env.contains("ANTHROPIC_AUTH_TOKEN"));
}

/// HAR-015 AC2: Kein Code in `beton-harness*` öffnet bekannte Vendor-Credential-Speicher.
#[test]
fn har_015_ac2_no_code_touches_vendor_credentials() {
    let forbidden = [
        concat!(".credentials", ".json"),
        concat!(".codex/", "auth.json"),
        concat!("Claude Code", "-credentials"),
        concat!("find-generic", "-password"),
        concat!("keyring", "::"),
    ];
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut checked = 0;
    for entry in std::fs::read_dir(crates).unwrap() {
        let krate = entry.unwrap().path();
        let name = krate.file_name().unwrap().to_string_lossy().into_owned();
        if !name.starts_with("beton-harness") {
            continue;
        }
        for file in walk(&krate.join("src")) {
            let text = std::fs::read_to_string(&file).unwrap();
            checked += 1;
            for pattern in forbidden {
                assert!(
                    !text.contains(pattern),
                    "{} enthält {pattern}",
                    file.display()
                );
            }
        }
    }
    assert!(checked >= 5, "nur {checked} Dateien geprüft");
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = entry.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
    out
}
