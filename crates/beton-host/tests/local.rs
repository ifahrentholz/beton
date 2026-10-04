//! Provider `local` (RUN-001 AC4 Contract-Suite, RUN-002, RUN-003 AC4).

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use beton_host::local::for_command;
use beton_host::{RunnerBoot, RunnerProvider, RunnerSpec, RunnerStatus, TerminateMode};

fn ws() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn waiting_runner(state: &tempfile::TempDir) -> std::sync::Arc<beton_host::LocalProvider> {
    for_command(
        &["sh", "-c", "read t; sleep 300"],
        state.path().to_path_buf(),
    )
}

// Die Contract-Suite braucht einen Provider und einen Workspace je Test.
beton_host::runner_provider_contract!(
    local_contract,
    {
        let state = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        beton_host::local::for_command(
            &["sh", "-c", "read t; sleep 300"],
            state.path().to_path_buf(),
        )
    },
    tempfile::tempdir().unwrap()
);

fn spec(dir: &std::path::Path) -> RunnerSpec {
    RunnerSpec {
        runner_id: beton_core::id::RunnerId::new(),
        session_id: beton_core::id::SessionId::new(),
        harness: "fake".into(),
        workspace: dir.to_path_buf(),
        env_allowlist: Vec::new(),
        secret_env: Vec::new(),
    }
}

fn boot() -> RunnerBoot {
    RunnerBoot {
        tunnel_socket: "/nicht/vorhanden.sock".into(),
        token: "bt_run_geheim".into(),
        session_id: beton_core::id::SessionId::new(),
        epoch: 1,
        harness: "fake".into(),
        scenario: None,
        model: None,
        dev: true,
        resume: None,
        harnesses: Default::default(),
        agent_ref: None,
        agent_snapshot: None,
        snapshots: None,
        fork: None,
    }
}

async fn wait_exit(p: &dyn RunnerProvider, h: &beton_host::RunnerHandle) -> RunnerStatus {
    for _ in 0..100 {
        let s = p.status(h).await.unwrap();
        if matches!(s, RunnerStatus::Exited { .. }) {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("Runner endet nicht");
}

#[tokio::test]
async fn run_002_ac1_runner_cwd_is_the_workspace_and_token_comes_via_stdin() {
    let (state, work) = (ws(), ws());
    let out = work.path().join("aus.txt");
    let script = format!(
        "read t; echo \"$t\" > '{0}'; pwd >> '{0}'; env >> '{0}'",
        out.display()
    );
    let p = for_command(&["sh", "-c", &script], state.path().to_path_buf());
    let prov = p.provision(&spec(work.path())).await.unwrap();
    let h = p.start(&prov, &boot()).await.unwrap();
    wait_exit(&*p, &h).await;
    let text = std::fs::read_to_string(&out).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("bt_run_geheim"), "Token über stdin");
    assert_eq!(
        std::fs::canonicalize(lines.next().unwrap()).unwrap(),
        std::fs::canonicalize(work.path()).unwrap()
    );
    assert!(
        !text.contains("BETON_RUNNER_TOKEN"),
        "Token nie in der Umgebung"
    );
    assert!(text.contains("BETON_RUNNER_PARENT_PID="));
}

#[tokio::test]
async fn run_002_ac4_env_outside_allowlist_is_not_visible() {
    let (state, work) = (ws(), ws());
    let out = work.path().join("env.txt");
    let mut inherit = BTreeMap::new();
    inherit.insert("PATH".to_owned(), std::env::var("PATH").unwrap());
    inherit.insert("AWS_SECRET_ACCESS_KEY".to_owned(), "geheim".to_owned());
    inherit.insert("ANTHROPIC_API_KEY".to_owned(), "geheim".to_owned());
    let p = beton_host::LocalProvider::new(
        vec![
            "sh".into(),
            "-c".into(),
            format!("read t; env > '{}'", out.display()),
        ],
        state.path().to_path_buf(),
    )
    .with_inherited_env(inherit);
    let prov = p.provision(&spec(work.path())).await.unwrap();
    let h = p.start(&prov, &boot()).await.unwrap();
    wait_exit(&p, &h).await;
    let env = std::fs::read_to_string(&out).unwrap();
    assert!(env.contains("PATH="));
    assert!(!env.contains("AWS_SECRET_ACCESS_KEY"), "{env}");
    assert!(!env.contains("ANTHROPIC_API_KEY"));
}

/// HAR-011 AC5: Der API-Key einer Session kommt als zweite stdin-Zeile, nie als
/// Env-Variable des Runners (die alle Kindprozesse erben würden).
#[tokio::test]
async fn har_011_ac5_api_key_goes_via_stdin_not_env() {
    let (state, work) = (ws(), ws());
    let out = work.path().join("aus.txt");
    let mut inherit = BTreeMap::new();
    inherit.insert("PATH".to_owned(), std::env::var("PATH").unwrap());
    inherit.insert(
        "OPENROUTER_API_KEY".to_owned(),
        "bt-fake-key-MARKER-host".to_owned(),
    );
    let p = beton_host::LocalProvider::new(
        vec![
            "sh".into(),
            "-c".into(),
            format!(
                "read t; read k; echo \"$k\" > '{0}'; env >> '{0}'",
                out.display()
            ),
        ],
        state.path().to_path_buf(),
    )
    .with_inherited_env(inherit);
    let mut s = spec(work.path());
    s.harness = "direct:openrouter".into();
    s.secret_env = vec!["OPENROUTER_API_KEY".into(), "NICHT_GESETZT".into()];
    let prov = p.provision(&s).await.unwrap();
    let h = p.start(&prov, &boot()).await.unwrap();
    wait_exit(&p, &h).await;
    let text = std::fs::read_to_string(&out).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some(r#"{"OPENROUTER_API_KEY":"bt-fake-key-MARKER-host"}"#),
        "Key nur über stdin; fehlende Variablen fehlen"
    );
    let env: Vec<&str> = lines.collect();
    assert!(
        !env.iter()
            .any(|l| l.contains("MARKER") || l.starts_with("OPENROUTER_API_KEY")),
        "{env:?}"
    );
}

#[tokio::test]
async fn run_003_ac4_graceful_terminate_escalates_after_grace() {
    let (state, work) = (ws(), ws());
    let p = for_command(
        &["sh", "-c", "trap '' TERM; read t; sleep 300"],
        state.path().to_path_buf(),
    );
    let prov = p.provision(&spec(work.path())).await.unwrap();
    let h = p.start(&prov, &boot()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = Instant::now();
    p.terminate(
        &h,
        TerminateMode::Graceful {
            grace: Duration::from_millis(300),
        },
    )
    .await
    .unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(250),
        "wartet die Grace-Periode: {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(3));
    assert!(matches!(
        p.status(&h).await.unwrap(),
        RunnerStatus::Exited { .. }
    ));
}

#[tokio::test]
async fn run_001_capabilities_of_local_provider() {
    let state = ws();
    let p = waiting_runner(&state);
    let caps = p.capabilities();
    assert!(!caps.snapshot && !caps.interactive_exec);
    assert_eq!(caps.platforms.len(), 1);
    let json = serde_json::to_value(&caps).unwrap();
    assert_eq!(json["isolation"], "process");
}
