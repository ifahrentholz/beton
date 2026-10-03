//! Host: Runner-Lebenszyklus über `RunnerProvider` (RUN-001, RUN-002).
//!
//! Lokal ist der Daemon selbst der einzige Host (`hst_local`) mit dem Provider `local`.
//! Docker, Kubernetes und Plugins (M4/M5) implementieren denselben Trait und laufen gegen
//! dieselbe Contract-Suite ([`runner_provider_contract!`]).

pub mod local;

use std::path::PathBuf;

use async_trait::async_trait;
use beton_core::id::{RunnerId, SessionId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub use crate::local::LocalProvider;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Isolation {
    Process,
    Container,
    Pod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMode {
    HostPath,
    Bind,
    Volume,
    Clone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Cpu,
    Memory,
    Pids,
    Disk,
    Timeout,
}

/// Was ein Provider kann (RUN-001). Der Server kennt Provider nur hierüber.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct RunnerCapabilities {
    pub isolation: Isolation,
    pub workspace_modes: Vec<WorkspaceMode>,
    pub persistent_workspace: bool,
    /// PTY-Exec, z. B. für Device-Flow-Logins.
    pub interactive_exec: bool,
    pub snapshot: bool,
    pub resource_limits: Vec<ResourceKind>,
    /// `os/arch` der erzeugten Runner, z. B. `linux/x86_64`.
    pub platforms: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
}

/// Was ein Runner braucht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerSpec {
    pub runner_id: RunnerId,
    pub session_id: SessionId,
    pub harness: String,
    /// Arbeitsverzeichnis (lokal: Projektverzeichnis oder Worktree).
    pub workspace: PathBuf,
    /// Zusätzlich durchgereichte Env-Variablen (deny-by-default).
    pub env_allowlist: Vec<String>,
}

/// Ergebnis von `provision`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provisioned {
    pub runner_id: RunnerId,
    pub workdir: PathBuf,
}

/// Startdaten für den Runner (RUN-002, AUTH-011).
#[derive(Debug, Clone)]
pub struct RunnerBoot {
    pub tunnel_socket: PathBuf,
    /// Geht über stdin an den Runner, nie über Env oder argv.
    pub token: String,
    pub session_id: SessionId,
    pub epoch: u64,
    pub harness: String,
    pub scenario: Option<PathBuf>,
    pub model: Option<String>,
    pub dev: bool,
    /// Native Session-Referenz zum Fortsetzen (SES-003).
    pub resume: Option<String>,
}

/// Laufender Runner.
#[derive(Debug, Clone)]
pub struct RunnerHandle {
    pub runner_id: RunnerId,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerStatus {
    Running,
    Exited {
        code: Option<i32>,
        stderr_tail: String,
    },
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminateMode {
    /// SIGTERM, `grace` warten, dann Force (RUN-003 AC4).
    Graceful {
        grace: std::time::Duration,
    },
    Force,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRequest {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedResource {
    pub runner_id: RunnerId,
    pub kind: String,
    pub location: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRef(pub String);

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("unsupported: vom Provider nicht unterstützt")]
    Unsupported,
    #[error("not_found: {0}")]
    NotFound(String),
    #[error("invalid_spec: {0}")]
    InvalidSpec(String),
    #[error("E/A-Fehler: {0}")]
    Io(#[from] std::io::Error),
}

/// Backend für Runner-Umgebungen (RUN-001).
#[async_trait]
pub trait RunnerProvider: Send + Sync {
    /// `local`, `docker`, `kubernetes` oder Plugin-Name.
    fn id(&self) -> &str;
    fn capabilities(&self) -> RunnerCapabilities;
    /// Ressourcen anlegen; idempotent über `spec.runner_id`.
    async fn provision(&self, spec: &RunnerSpec) -> Result<Provisioned, RunnerError>;
    async fn start(
        &self,
        env: &Provisioned,
        boot: &RunnerBoot,
    ) -> Result<RunnerHandle, RunnerError>;
    async fn exec(&self, h: &RunnerHandle, req: ExecRequest) -> Result<ExecOutput, RunnerError>;
    async fn status(&self, h: &RunnerHandle) -> Result<RunnerStatus, RunnerError>;
    async fn terminate(&self, h: &RunnerHandle, mode: TerminateMode) -> Result<(), RunnerError>;
    async fn snapshot(&self, _h: &RunnerHandle) -> Result<SnapshotRef, RunnerError> {
        Err(RunnerError::Unsupported)
    }
    async fn restore(
        &self,
        _s: &SnapshotRef,
        _spec: &RunnerSpec,
    ) -> Result<Provisioned, RunnerError> {
        Err(RunnerError::Unsupported)
    }
    /// Alles, was dieser Provider verwaltet (für den Reaper).
    async fn list_managed(&self) -> Result<Vec<ManagedResource>, RunnerError>;
}

/// Gemeinsame Contract-Suite für alle Provider (RUN-001 AC4).
///
/// `$factory` erzeugt einen Provider, dessen Runner-Kommando auf stdin ein Token liest und
/// dann wartet (z. B. `sh -c 'read t; sleep 300'`); `$workspace` ein Arbeitsverzeichnis.
#[macro_export]
macro_rules! runner_provider_contract {
    ($name:ident, $factory:expr, $workspace:expr) => {
        mod $name {
            #![allow(clippy::unwrap_used)]
            use std::time::Duration;

            use $crate::{RunnerError, RunnerProvider, RunnerSpec, RunnerStatus, TerminateMode};

            fn spec(ws: &std::path::Path) -> RunnerSpec {
                RunnerSpec {
                    runner_id: beton_core::id::RunnerId::new(),
                    session_id: beton_core::id::SessionId::new(),
                    harness: "fake".into(),
                    workspace: ws.to_path_buf(),
                    env_allowlist: Vec::new(),
                }
            }

            fn boot() -> $crate::RunnerBoot {
                $crate::RunnerBoot {
                    tunnel_socket: "/nicht/vorhanden.sock".into(),
                    token: "bt_run_test".into(),
                    session_id: beton_core::id::SessionId::new(),
                    epoch: 1,
                    harness: "fake".into(),
                    scenario: None,
                    model: None,
                    dev: true,
                    resume: None,
                }
            }

            #[tokio::test]
            async fn run_001_ac2_provision_is_idempotent() {
                let (provider, ws) = ($factory, $workspace);
                let s = spec(ws.path());
                let a = provider.provision(&s).await.unwrap();
                let b = provider.provision(&s).await.unwrap();
                assert_eq!(a, b);
                let managed = provider.list_managed().await.unwrap();
                assert_eq!(
                    managed
                        .iter()
                        .filter(|m| m.runner_id == s.runner_id)
                        .count(),
                    1
                );
            }

            #[tokio::test]
            async fn run_001_ac1_snapshot_without_capability_is_unsupported() {
                let (provider, ws) = ($factory, $workspace);
                let caps = provider.capabilities();
                let p = provider.provision(&spec(ws.path())).await.unwrap();
                let h = provider.start(&p, &boot()).await.unwrap();
                if !caps.snapshot {
                    assert!(matches!(
                        provider.snapshot(&h).await,
                        Err(RunnerError::Unsupported)
                    ));
                }
                provider.terminate(&h, TerminateMode::Force).await.unwrap();
            }

            #[tokio::test]
            async fn run_contract_start_status_terminate() {
                let (provider, ws) = ($factory, $workspace);
                let p = provider.provision(&spec(ws.path())).await.unwrap();
                let h = provider.start(&p, &boot()).await.unwrap();
                assert_eq!(provider.status(&h).await.unwrap(), RunnerStatus::Running);
                provider
                    .terminate(
                        &h,
                        TerminateMode::Graceful {
                            grace: Duration::from_secs(2),
                        },
                    )
                    .await
                    .unwrap();
                assert!(matches!(
                    provider.status(&h).await.unwrap(),
                    RunnerStatus::Exited { .. }
                ));
            }
        }
    };
}
