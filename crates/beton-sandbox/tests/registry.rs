//! SBX-001: Provider-Trait, Registry, Auswahl und `sandbox.started`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use beton_core::event::{SandboxStage, SandboxStarted};
use beton_sandbox::{
    BackendChoice, Cap, CommandSpec, NoneProvider, Os, ProbeReport, ProviderId, Registry,
    ResolvedPolicy, Sandbox, SandboxError, SandboxProvider, SandboxSpec, Stage, Stdio,
    auto_backend,
};

#[test]
fn sbx_001_ac1_every_registered_provider_reports_caps_and_reasons() {
    let reports = Registry::platform().probe_all();
    let ids: Vec<ProviderId> = reports.iter().map(|r| r.id).collect();
    for id in ProviderId::ALL {
        assert!(ids.contains(&id), "{id} fehlt in der Probe: {ids:?}");
    }
    for r in &reports {
        // Jede nicht erzwingbare Capability hat einen Grund.
        for cap in Cap::ALL {
            if !r.caps.contains(&cap) {
                let reason = r.degraded_reasons.get(&cap);
                assert!(
                    reason.is_some_and(|s| !s.is_empty()),
                    "{}: kein Grund für fehlendes {cap}",
                    r.id
                );
            }
        }
        if !r.available {
            assert!(r.caps.is_empty(), "{}: nicht verfügbar, aber caps", r.id);
            assert!(r.unavailable_reason.is_some(), "{}: ohne Grund", r.id);
        }
    }
    let json = serde_json::to_value(&reports).unwrap();
    let none = json
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "none")
        .unwrap();
    assert_eq!(none["available"], true);
    assert_eq!(none["caps"], serde_json::json!([]));
    assert!(none["degraded_reasons"]["fs_read"].is_string());
}

#[test]
fn sbx_001_ac1_unavailable_backends_fail_closed_on_this_platform() {
    let registry = Registry::platform();
    // Linux-, Windows- und Container-Backends folgen mit eigenen Arbeitspaketen; bis dahin
    // melden sie `unavailable` und starten nichts (kein Fallback).
    for id in [ProviderId::Landlock, ProviderId::Bwrap, ProviderId::Windows, ProviderId::Docker, ProviderId::Podman] {
        let p = registry.get(id).unwrap();
        let report = p.probe();
        assert!(!report.available, "{id} meldet sich verfügbar");
        let mut policy = ResolvedPolicy::unsandboxed(Stage::Tools);
        policy.backend = id;
        policy.required = beton_sandbox::caps::required_default();
        let err = p.spawn(&policy, CommandSpec::new("/bin/sh")).unwrap_err();
        assert_eq!(err.code(), "sandbox_unavailable", "{err}");
    }
    let seatbelt = registry.get(ProviderId::Seatbelt).unwrap().probe();
    if cfg!(target_os = "macos") {
        assert!(seatbelt.available, "{seatbelt:?}");
        assert!(seatbelt.caps.contains(&Cap::FsRead) && seatbelt.caps.contains(&Cap::Net));
    } else {
        assert!(!seatbelt.available);
    }
}

#[test]
fn sbx_001_ac3_auto_picks_seatbelt_on_macos_and_landlock_on_linux() {
    assert_eq!(auto_backend(Os::MacOs), Some(ProviderId::Seatbelt));
    assert_eq!(auto_backend(Os::Linux), Some(ProviderId::Landlock));
    assert_eq!(auto_backend(Os::Windows), Some(ProviderId::Windows));
    assert_eq!(auto_backend(Os::Other), None);
    let registry = Registry::platform();
    let picked = registry.select(BackendChoice::Auto).unwrap().id();
    let expected = if cfg!(target_os = "macos") {
        ProviderId::Seatbelt
    } else if cfg!(target_os = "linux") {
        ProviderId::Landlock
    } else {
        ProviderId::Windows
    };
    assert_eq!(picked, expected);
    // `none` wird nie automatisch gewählt, nur ausdrücklich.
    assert_eq!(
        registry.select(BackendChoice::Provider(ProviderId::None)).unwrap().id(),
        ProviderId::None
    );
}

#[test]
fn sbx_001_ac3_unknown_backend_is_invalid_spec_when_loading() {
    let err = SandboxSpec::from_yaml("backend: firejail\n").unwrap_err();
    assert_eq!(err.code(), "invalid_spec", "{err}");
    assert!(err.to_string().contains("firejail"), "{err}");
    for known in ["auto", "seatbelt", "landlock", "bwrap", "windows", "docker", "podman", "none"] {
        let spec = SandboxSpec::from_yaml(&format!("backend: {known}\n")).unwrap();
        assert_eq!(spec.backend.unwrap().as_str(), known);
    }
    // Unbekannte Felder sind ebenfalls ein Ladefehler, nie still ignoriert.
    let err = SandboxSpec::from_yaml("read_pahts: [/opt]\n").unwrap_err();
    assert_eq!(err.code(), "invalid_spec", "{err}");
}

#[test]
fn sbx_001_ac3_registry_without_backend_is_unavailable_not_none() {
    let empty = Registry::empty(Os::Linux);
    let err = empty.select(BackendChoice::Auto).err().unwrap();
    assert!(matches!(err, SandboxError::Unavailable { provider: ProviderId::Landlock, .. }), "{err}");
    let err = Registry::empty(Os::Other).select(BackendChoice::Auto).err().unwrap();
    assert_eq!(err.code(), "sandbox_unavailable");
}

fn collector() -> (beton_sandbox::EventSink, Arc<Mutex<Vec<SandboxStarted>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    (Arc::new(move |ev| sink.lock().unwrap().push(ev)), seen)
}

fn true_cmd() -> CommandSpec {
    let program = if cfg!(windows) { "cmd" } else { "true" };
    let mut c = CommandSpec::new(program).stdio(Stdio::Null, Stdio::Null, Stdio::Null);
    if cfg!(windows) {
        c = c.args(["/C", "exit 0"]);
    }
    c
}

#[tokio::test]
async fn sbx_001_ac4_each_successful_spawn_emits_exactly_one_started_event() {
    let (sink, seen) = collector();
    let sandbox = Sandbox::unsandboxed(Stage::Harness, Some(sink));
    for _ in 0..3 {
        let mut child = sandbox.spawn(true_cmd()).unwrap();
        assert!(child.wait().await.unwrap().success());
    }
    let events = seen.lock().unwrap().clone();
    assert_eq!(events.len(), 3, "{events:?}");
    for ev in events {
        assert_eq!(ev.stage, SandboxStage::Harness);
        assert_eq!(ev.backend, "none");
        assert_eq!(ev.caps, serde_json::json!([]));
        // `none` erzwingt nichts: sichtbar als degraded.
        assert!(ev.degraded);
    }
}

#[tokio::test]
async fn sbx_001_ac4_failed_spawn_emits_no_event() {
    let (sink, seen) = collector();
    let sandbox = Sandbox::unsandboxed(Stage::Tools, Some(sink));
    let err = sandbox
        .spawn(CommandSpec::new("/nonexistent/beton-sbx-missing"))
        .unwrap_err();
    assert_eq!(err.code(), "spawn_failed");
    assert_eq!(std::io::Error::from(err).kind(), std::io::ErrorKind::NotFound);
    // Ein nicht verfügbares Backend startet nichts und meldet nichts.
    let unavailable = Registry::platform().get(ProviderId::Docker).unwrap();
    let mut policy = ResolvedPolicy::unsandboxed(Stage::Tools);
    policy.backend = ProviderId::Docker;
    let sandbox = Sandbox::new(unavailable, policy, Some(collector().0));
    assert!(sandbox.spawn(true_cmd()).is_err());
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn sbx_001_policy_for_other_backend_is_rejected() {
    // Eine Policy gilt nur für ihr Backend; `none` startet keine Seatbelt-Policy und umgekehrt.
    let mut policy = ResolvedPolicy::unsandboxed(Stage::Tools);
    policy.backend = ProviderId::Seatbelt;
    let err = NoneProvider.spawn(&policy, true_cmd()).unwrap_err();
    assert_eq!(err.code(), "spawn_failed");
}

#[test]
fn sbx_001_missing_required_cap_is_unavailable_with_reasons() {
    let probe = ProbeReport {
        id: ProviderId::Seatbelt,
        available: true,
        caps: [Cap::FsRead, Cap::FsWrite].into_iter().collect(),
        degraded_reasons: [(Cap::Net, "kein Netzfilter".to_owned())].into_iter().collect(),
        unavailable_reason: None,
        version: String::new(),
    };
    let mut policy = ResolvedPolicy::unsandboxed(Stage::Tools);
    policy.backend = ProviderId::Seatbelt;
    policy.required = [Cap::FsRead, Cap::Net].into_iter().collect();
    match beton_sandbox::check_caps(&probe, &policy).unwrap_err() {
        SandboxError::Unavailable { provider, missing, hint } => {
            assert_eq!(provider, ProviderId::Seatbelt);
            assert_eq!(missing, [Cap::Net].into_iter().collect());
            assert!(hint.contains("kein Netzfilter"), "{hint}");
        }
        other => panic!("{other}"),
    }
}
