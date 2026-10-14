//! Komposition der zweistufigen Sandbox im Runner (SBX-002, PRX-001, PRX-002, PRX-005,
//! PRX-008, #191): wann sie greift, fail closed ohne CA, `tool_isolation: inherited` nur mit
//! `require_tool_isolation: false` (SBX-002 AC4), je Stufe eine Proxy-Instanz mit eigenen
//! Regeln, Passthrough nur in Stufe 1, und `bt-exec` aus Stufe 1 landet in Stufe 2.
//!
//! Alle Homes sind Fixtures in Temp-Verzeichnissen; es gibt keinen Netzverkehr.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use beton_core::event::{EventPayload, ToolCallCompleted, ToolCallRequested, ToolStatus};
use beton_harness::ToolIsolation;
use beton_runner::stages::{ShellIntegrity, StageInput, StageOptions, compose, sandbox_vars};
use beton_sandbox::{CommandSpec, SandboxSpec, Stage, Stdio};

fn exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_beton-runner"))
}

struct Fx {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    ws: PathBuf,
}

fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let home = root.join("home");
    let ws = root.join("ws");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(home.join(".claude/.credentials.json"), "FIXTURE-CLAUDE").unwrap();
    std::fs::create_dir_all(&ws).unwrap();
    Fx {
        _tmp: tmp,
        home,
        ws,
    }
}

fn input<'a>(fx: &'a Fx, harness: &'a str, iso: ToolIsolation) -> StageInput<'a> {
    StageInput {
        session_id: "ses_stagetest".into(),
        harness,
        tool_isolation: iso,
        workdir: &fx.ws,
        home: fx.home.clone(),
        host_env: [
            ("PATH", "/usr/bin:/bin:/usr/sbin:/sbin"),
            ("SHELL", "/bin/sh"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect(),
        program: None,
        events: None,
    }
}

fn opts(yaml: &str, ca: bool) -> StageOptions {
    StageOptions {
        spec: SandboxSpec::from_yaml(yaml).unwrap(),
        proxy_ca: ca.then(|| {
            beton_proxy::BetonCa::generate("stagetest", time::OffsetDateTime::now_utc()).unwrap()
        }),
        proxy: beton_proxy::ProxyOptions::default(),
        exe: exe(),
    }
}

#[tokio::test]
async fn sbx_002_without_explicit_backend_sessions_stay_on_backend_none_until_wp49() {
    let fx = fx();
    for yaml in ["{}\n", "backend: none\n", "preset: dev\n"] {
        let s = compose(
            input(&fx, "claude", ToolIsolation::ShellOverride),
            opts(yaml, true),
        )
        .await
        .unwrap();
        assert!(s.is_none(), "{yaml}");
    }
}

#[tokio::test]
async fn sbx_002_ac4_inherited_tool_isolation_starts_only_with_require_tool_isolation_false() {
    let fx = fx();
    let err = compose(
        input(&fx, "codex", ToolIsolation::Inherited),
        opts("backend: auto\n", true),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), "sandbox_unavailable", "{err}");
    assert!(err.to_string().contains("require_tool_isolation"), "{err}");
    let s = compose(
        input(&fx, "codex", ToolIsolation::Inherited),
        opts("backend: auto\nrequire_tool_isolation: false\n", true),
    )
    .await
    .unwrap()
    .unwrap();
    // Die Session läuft, ist aber als degraded markiert.
    assert!(s.harness.started_event().degraded);
    let s = compose(
        input(&fx, "claude", ToolIsolation::ShellOverride),
        opts("backend: auto\n", true),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!s.harness.started_event().degraded);
}

#[tokio::test]
async fn prx_002_network_without_ca_fails_closed() {
    let fx = fx();
    let err = compose(
        input(&fx, "claude", ToolIsolation::ShellOverride),
        opts("backend: auto\n", false),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), "sandbox_unavailable", "{err}");
    // Stufe 1 braucht für Vendor-Hosts immer den Proxy, auch wenn Stufe 2 kein Netz hat.
    let err = compose(
        input(&fx, "claude", ToolIsolation::ShellOverride),
        opts("backend: auto\nallow_network: false\n", false),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), "sandbox_unavailable", "{err}");
    // Ein Harness ohne Vendor-Hosts braucht ohne Netz keine CA.
    let s = compose(
        input(&fx, "fake", ToolIsolation::Beton),
        opts("backend: auto\nallow_network: false\n", false),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!s.tools.policy().env.contains_key("HTTPS_PROXY"));
    assert!(!s.harness.policy().env.contains_key("HTTPS_PROXY"));
}

/// `allow_network: false` (z. B. Preset `readonly`) gilt für die Tools; die Vendor-CLI in
/// Stufe 1 erreicht weiter nur ihre Passthrough-Hosts (Security-Review #228).
#[tokio::test]
async fn sbx_012_readonly_keeps_the_vendor_hosts_for_stage_one() {
    let fx = fx();
    let s = compose(
        input(&fx, "claude", ToolIsolation::ShellOverride),
        opts("backend: auto\npreset: readonly\n", true),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(s.harness.policy().env.contains_key("HTTPS_PROXY"));
    assert!(
        s.passthrough(Stage::Harness)
            .contains(&"api.anthropic.com:443".to_owned())
    );
    assert!(!s.tools.policy().env.contains_key("HTTPS_PROXY"));
    assert!(s.passthrough(Stage::Tools).is_empty());
}

/// Stufe 1 startet Programme über ihren `PATH`: kein relativer Eintrag und nichts, was
/// Stufe 2 schreiben kann (sonst legt ein Tool dort ein Programm ab, das Stufe 1 ausführt).
#[tokio::test]
async fn sbx_002_stage_one_path_has_no_relative_or_stage_two_writable_entries() {
    let fx = fx();
    let mut inp = input(&fx, "claude", ToolIsolation::ShellOverride);
    let path = format!(
        "bin:./x::{}/node_modules/.bin:/usr/bin:/bin",
        fx.ws.display()
    );
    inp.host_env.insert("PATH".into(), path);
    let s = compose(inp, opts("backend: auto\n", true))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.harness.policy().env["PATH"], "/usr/bin:/bin");
    let tools = &s.tools.policy().env["PATH"];
    assert!(
        tools.split(':').all(|d| d.starts_with('/')),
        "keine relativen Einträge: {tools}"
    );
}

/// Der Austausch ist nicht das `TMPDIR` der Stufe 2: Stufe 1 liest nur, was Claude dort
/// erwartet.
#[tokio::test]
async fn sbx_002_exchange_is_separate_from_the_stage_two_tmpdir() {
    let fx = fx();
    let s = compose(
        input(&fx, "claude", ToolIsolation::ShellOverride),
        opts("backend: auto\n", true),
    )
    .await
    .unwrap()
    .unwrap();
    let tmp = PathBuf::from(&s.tools.policy().env["TMPDIR"]);
    assert_ne!(s.env.exchange, tmp);
    assert!(!s.harness.policy().write_roots.contains(&tmp));
    assert!(!s.harness.policy().read_roots.contains(&tmp));
    assert!(s.harness.policy().write_roots.contains(&s.env.exchange));
    assert!(s.tools.policy().write_roots.contains(&s.env.exchange));
}

/// `start.sandbox.enabled` meint die Tools: Läuft ein Harness degraded (Tools in Stufe 1),
/// ist es `false` (Policies, die eine Sandbox verlangen, greifen dann).
#[tokio::test]
async fn sbx_002_ac4_degraded_session_reports_no_tool_sandbox_to_policies() {
    let fx = fx();
    let s = compose(
        input(&fx, "codex", ToolIsolation::Inherited),
        opts("backend: auto\nrequire_tool_isolation: false\n", true),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(sandbox_vars(Some(&s)).enabled, Some(false));
    let s = compose(
        input(&fx, "claude", ToolIsolation::ShellOverride),
        opts("backend: auto\n", true),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(sandbox_vars(Some(&s)).enabled, Some(true));
    assert_eq!(sandbox_vars(None).enabled, Some(false));
}

fn requested(id: &str, tool: &str) -> EventPayload {
    EventPayload::ToolCallRequested(ToolCallRequested {
        call_id: id.into(),
        tool: tool.into(),
        ..ToolCallRequested::default()
    })
}

fn completed(id: &str, status: ToolStatus) -> EventPayload {
    EventPayload::ToolCallCompleted(ToolCallCompleted {
        call_id: id.into(),
        status,
        ..ToolCallCompleted::default()
    })
}

/// Fail closed, falls Claude `CLAUDE_CODE_SHELL` (künftig) ignoriert: Ein abgeschlossener
/// Bash-Aufruf ohne Start über den Exec-Broker beendet die Session (Security-Review #228).
/// Gezählt wird jeder Abschluss außer `denied` (auch gescheiterte Kommandos sind gelaufen).
#[test]
fn sbx_002_shell_call_without_broker_start_is_an_integrity_violation() {
    let mut i = ShellIntegrity::new(&["Bash"]);
    i.observe(&requested("a", "Bash"), 0).unwrap();
    assert!(i.observe(&completed("a", ToolStatus::Ok), 0).is_err());

    for status in [ToolStatus::Error, ToolStatus::Cancelled] {
        let mut i = ShellIntegrity::new(&["Bash"]);
        i.observe(&requested("x", "Bash"), 0).unwrap();
        assert!(
            i.observe(&completed("x", status.clone()), 0).is_err(),
            "{status:?} ohne Broker-Start"
        );
    }

    let mut i = ShellIntegrity::new(&["Bash"]);
    for (ev, starts) in [
        (requested("a", "Bash"), 0),
        (completed("a", ToolStatus::Ok), 1),
        // Abgelehnt: nie gestartet, zählt nicht.
        (requested("b", "Bash"), 1),
        (completed("b", ToolStatus::Denied), 1),
        (requested("c", "Bash"), 1),
        (completed("c", ToolStatus::Error), 2),
        // Andere Tools laufen in Stufe 1 und zählen nicht.
        (requested("d", "Read"), 2),
        (completed("d", ToolStatus::Ok), 2),
    ] {
        i.observe(&ev, starts).unwrap();
    }
    i.observe(&requested("e", "Bash"), 2).unwrap();
    assert!(i.observe(&completed("e", ToolStatus::Error), 2).is_err());
}

#[tokio::test]
async fn prx_005_prx_008_one_proxy_instance_per_stage_passthrough_only_in_stage_one() {
    let fx = fx();
    let s = compose(
        input(&fx, "claude", ToolIsolation::ShellOverride),
        opts(
            "backend: auto\negress_rules: [\"GET registry.npmjs.org/**\"]\n",
            true,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let h = s.harness.policy();
    let t = s.tools.policy();
    let (hp, tp) = (&h.env["HTTPS_PROXY"], &t.env["HTTPS_PROXY"]);
    assert_ne!(hp, tp, "eigene Instanz (Port, Token) je Stufe");
    assert!(hp.starts_with("http://bt:") && tp.starts_with("http://bt:"));
    let bundle = PathBuf::from(&t.env["SSL_CERT_FILE"]);
    let pem = std::fs::read_to_string(&bundle).unwrap();
    assert!(pem.contains("BEGIN CERTIFICATE") && !pem.contains("PRIVATE KEY"));
    assert!(
        s.passthrough(Stage::Harness)
            .contains(&"api.anthropic.com:443".to_owned())
    );
    assert!(s.passthrough(Stage::Tools).is_empty());
    // Stufe 2 bekommt weder Broker-Zugang noch Credentials der Stufe 1.
    assert!(!t.env.contains_key("BETON_EXEC_TOKEN"));
    assert!(h.env.contains_key("BETON_EXEC_TOKEN"));
    assert!(s.env.shim_dir.join("bt-exec-zsh").exists());
    assert!(s.env.shim_dir.join("bt-exec-bash").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sbx_002_bt_exec_from_stage_one_runs_in_stage_two() {
    let fx = fx();
    let started = Arc::new(AtomicUsize::new(0));
    let count = started.clone();
    let mut inp = input(&fx, "claude", ToolIsolation::ShellOverride);
    inp.events = Some(Arc::new(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
    }));
    let s = compose(inp, opts("backend: auto\nallow_network: false\n", true))
        .await
        .unwrap()
        .unwrap();
    let shim = s.env.shim_dir.join("bt-exec");
    let script = format!(
        "echo \"1:$BETON_STAGE\"; cat \"$HOME/.claude/.credentials.json\"; echo; {} /bin/sh -c 'echo \"2:$BETON_STAGE\"; cat \"$HOME/.claude/.credentials.json\"'",
        shim.display()
    );
    let out = s
        .harness
        .spawn(
            CommandSpec::new("/bin/sh")
                .args(["-c", &script])
                .cwd(&fx.ws)
                .stdio(Stdio::Null, Stdio::Piped, Stdio::Piped),
        )
        .unwrap()
        .output(Duration::from_secs(60))
        .await
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1:harness"), "{stdout}");
    assert!(stdout.contains("2:tools"), "{stdout}");
    assert_eq!(stdout.matches("FIXTURE-CLAUDE").count(), 1, "{stdout}");
    // Je ein sandbox.started für Stufe 1 und für den Start über den Broker.
    assert_eq!(started.load(Ordering::SeqCst), 2);
}
