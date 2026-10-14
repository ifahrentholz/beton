//! SBX-013: Ressourcenlimits und Lebenszyklus des Prozessbaums (Linux und macOS).
//!
//! Die Fork-Bombe läuft nur mit gesetztem Limit; der Host bleibt bedienbar. Alle Prozesse
//! tragen eine eindeutige Markierung, damit die Tests prüfen können, dass nichts übrig bleibt.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use beton_core::event::{SandboxStage, SandboxViolation, ViolationKind};
use beton_sandbox::{Limits, SandboxSpec};

#[test]
fn sbx_013_limits_are_validated_and_merged_per_field() {
    let spec = SandboxSpec::from_yaml(
        "limits: { memory_mb: 512, pids: 256, cpu_seconds: 60, file_size_mb: 100 }\n",
    )
    .unwrap();
    assert_eq!(
        spec.limits,
        Some(Limits {
            memory_mb: Some(512),
            pids: Some(256),
            cpu_seconds: Some(60),
            file_size_mb: Some(100),
        })
    );
    for (yaml, field) in [
        ("limits: { pids: 0 }\n", "limits.pids"),
        ("limits: { pids: 4 }\n", "limits.pids"),
        ("limits: { memory_mb: 0 }\n", "limits.memory_mb"),
        ("limits: { cpu_seconds: 0 }\n", "limits.cpu_seconds"),
        ("limits: { file_size_mb: 0 }\n", "limits.file_size_mb"),
    ] {
        let err = SandboxSpec::from_yaml(yaml).unwrap_err();
        assert_eq!(err.code(), "invalid_spec", "{yaml}: {err}");
        assert!(err.to_string().contains(field), "{yaml}: {err}");
    }
    let err = SandboxSpec::from_yaml("limits: { threads: 3 }\n").unwrap_err();
    assert_eq!(err.code(), "invalid_spec");
    // Über die Ebenen gilt je Feld die spezifischste Ebene.
    let user = SandboxSpec::from_yaml("limits: { memory_mb: 512, pids: 256 }\n").unwrap();
    let agent = SandboxSpec::from_yaml("limits: { pids: 64 }\n").unwrap();
    let merged = SandboxSpec::merged(&[&user, &agent]);
    assert_eq!(merged.limits.unwrap().memory_mb, Some(512));
    assert_eq!(
        SandboxSpec::merged(&[&user, &agent]).limits.unwrap().pids,
        Some(64)
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod live {
    use super::*;
    use beton_sandbox::{
        BackendChoice, CommandSpec, NoneProvider, Registry, ResolvedPolicy, Sandbox,
        SandboxProvider, Stage, Stdio,
    };
    use support::*;

    /// Das Standard-Backend dieser Plattform (macOS: Seatbelt, Linux: Landlock bzw. bwrap).
    fn provider() -> Arc<dyn SandboxProvider> {
        #[cfg(target_os = "linux")]
        let registry = Registry::with_launcher(std::path::Path::new(env!(
            "CARGO_BIN_EXE_beton-sandbox-exec"
        )));
        #[cfg(not(target_os = "linux"))]
        let registry = Registry::platform();
        let p = registry.select(BackendChoice::Auto).unwrap();
        let probe = p.probe();
        assert!(probe.available, "{probe:?}");
        p
    }

    #[tokio::test]
    async fn sbx_013_ac1_fork_bomb_is_limited_host_stays_usable_and_reported() {
        let fx = fx();
        let p = provider();
        let yaml = format!("read_paths: [{}]\nlimits: {{ pids: 256 }}\n", exe_grant());
        let policy = fx.policy(p.as_ref(), &yaml, Stage::Tools);
        assert_eq!(policy.limits.pids, Some(256));
        let seen: Arc<Mutex<Vec<SandboxViolation>>> = Arc::default();
        let sink = seen.clone();
        let sandbox = Sandbox::new(p.clone(), policy.clone(), None)
            .with_violations(Arc::new(move |v| sink.lock().unwrap().push(v)));
        let child = sandbox
            .spawn(probe_cmd(&policy, "fork", &["1000"]).stdio(
                Stdio::Null,
                Stdio::Piped,
                Stdio::Piped,
            ))
            .unwrap();
        // Während die Bombe läuft, startet der Host weiter Prozesse.
        let host = NoneProvider
            .spawn(
                &ResolvedPolicy::unsandboxed(Stage::Tools),
                CommandSpec::new("/bin/sh")
                    .args(["-c", "echo host-ok"])
                    .stdio(Stdio::Null, Stdio::Piped, Stdio::Null),
            )
            .unwrap()
            .output(Duration::from_secs(30))
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&host.stdout).contains("host-ok"));
        let out = child.output(Duration::from_secs(120)).await.unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let line = stdout
            .lines()
            .find_map(|l| l.strip_prefix("PROBE "))
            .unwrap_or_else(|| panic!("Sonde lief nicht: {stdout}"));
        let started: usize = line
            .split_whitespace()
            .find_map(|w| w.strip_prefix("started:"))
            .unwrap()
            .parse()
            .unwrap();
        let failed: usize = line
            .split_whitespace()
            .find_map(|w| w.strip_prefix("failed:"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(started < 256, "{}: Limit greift nicht: {line}", p.id());
        assert!(failed > 0, "{}: {line}", p.id());
        // Die Sandbox meldet die Überschreitung.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let v = seen.lock().unwrap().clone();
            if let Some(v) = v.iter().find(|v| v.kind == ViolationKind::Limit) {
                assert_eq!(v.stage, SandboxStage::Tools);
                assert_eq!(v.target.as_deref(), Some("pids"), "{v:?}");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{}: keine Meldung sandbox.violation {{kind: limit}}: {v:?}",
                p.id()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn sbx_013_ac2_stop_leaves_no_process_of_the_sandbox() {
        let fx = fx();
        let p = provider();
        let policy = fx.policy(p.as_ref(), "{}\n", Stage::Tools);
        let marker = format!("1000.{}", std::process::id());
        // Hintergrundprozess, neue Sitzung (setsid) und verwaister Enkel in der Prozessgruppe.
        let script = format!(
            "sleep {marker} & perl -e 'use POSIX; POSIX::setsid(); exec \"sleep\", \"{marker}\"' & (sleep {marker} &) ; sleep {marker}"
        );
        let mut child = p
            .spawn(
                &policy,
                CommandSpec::new("/bin/sh")
                    .args(["-c", &script])
                    .cwd(&policy.workspace)
                    .stdio(Stdio::Null, Stdio::Null, Stdio::Null),
            )
            .unwrap();
        let start = std::time::Instant::now();
        while processes_with(&format!("sleep {marker}")).len() < 4 {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "{}: Prozesse starten nicht: {:?}",
                p.id(),
                processes_with(&marker)
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        child.start_kill().unwrap();
        let _ = child.wait().await;
        let left = gone_within(&marker, Duration::from_secs(2)).await;
        assert!(left.is_ok(), "{}: übrig: {left:?}", p.id());
        // Dasselbe beim Fallenlassen des Kindes (Session-Ende ohne ausdrückliches Stoppen).
        let child = p
            .spawn(
                &policy,
                CommandSpec::new("/bin/sh")
                    .args(["-c", &script])
                    .cwd(&policy.workspace)
                    .stdio(Stdio::Null, Stdio::Null, Stdio::Null),
            )
            .unwrap();
        let start = std::time::Instant::now();
        while processes_with(&format!("sleep {marker}")).len() < 4 {
            assert!(start.elapsed() < Duration::from_secs(10));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        drop(child);
        let left = gone_within(&marker, Duration::from_secs(2)).await;
        assert!(left.is_ok(), "{}: übrig nach Drop: {left:?}", p.id());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn sbx_013_ac3_memory_limit_needs_cgroup_delegation_or_is_reported_degraded() {
        use beton_sandbox::Cap;
        let fx = fx();
        let p = provider();
        let probe = p.probe();
        let delegated = probe.caps.contains(&Cap::Resources);
        if std::env::var("BETON_TEST_EXPECT_CGROUP").as_deref() == Ok("1") {
            assert!(
                delegated,
                "die CI richtet eine cgroup-Delegation ein: {probe:?}"
            );
        }
        let yaml = format!(
            "read_paths: [{}]\nlimits: {{ memory_mb: 64 }}\n",
            exe_grant()
        );
        let policy = fx.policy(p.as_ref(), &yaml, Stage::Tools);
        if !delegated {
            let reason = probe.degraded_reasons.get(&Cap::Resources).unwrap();
            assert!(reason.contains("cgroup"), "{reason}");
            // Das nicht durchsetzbare Speicherlimit ist sichtbar degradiert.
            assert!(policy.degraded.contains(&Cap::Resources), "{policy:?}");
            assert!(
                policy.notes.iter().any(|n| n.contains("memory_mb")),
                "{:?}",
                policy.notes
            );
            return;
        }
        assert!(policy.degraded.is_empty(), "{policy:?}");
        let seen: Arc<Mutex<Vec<SandboxViolation>>> = Arc::default();
        let sink = seen.clone();
        let sandbox = Sandbox::new(p.clone(), policy.clone(), None)
            .with_violations(Arc::new(move |v| sink.lock().unwrap().push(v)));
        let out = sandbox
            .spawn(probe_cmd(&policy, "alloc", &["512"]).stdio(
                Stdio::Null,
                Stdio::Piped,
                Stdio::Piped,
            ))
            .unwrap()
            .output(Duration::from_secs(60))
            .await
            .unwrap();
        // Das Tool stirbt am OOM, der Runner (dieser Test) lebt weiter.
        assert!(!out.status.success(), "{out:?}");
        assert!(!String::from_utf8_lossy(&out.stdout).contains("allocated:512"));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !seen
            .lock()
            .unwrap()
            .iter()
            .any(|v| v.kind == ViolationKind::Limit && v.target.as_deref() == Some("memory"))
        {
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                seen.lock().unwrap()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
