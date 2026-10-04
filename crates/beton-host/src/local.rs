//! Provider `local`: ein Runner-Prozess pro Session auf der eigenen Maschine (RUN-002).
//!
//! - Workspace ist ein Host-Pfad; der Runner läuft mit diesem Arbeitsverzeichnis.
//! - Umgebung deny-by-default: nur eine kleine Allowlist plus die Startparameter.
//! - Das Binding-Token geht über stdin, nicht über Env oder argv (AUTH-011).
//! - Der Runner bekommt `BETON_RUNNER_PARENT_PID` und beendet sich, wenn der Daemon stirbt.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use beton_core::id::RunnerId;
use beton_harness::process::{
    LaunchSpec, ProcessHandle, ProcessLauncher, RealLauncher, ShutdownTimeouts,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use crate::{
    ExecOutput, ExecRequest, Isolation, ManagedResource, Provisioned, RunnerBoot,
    RunnerCapabilities, RunnerError, RunnerHandle, RunnerProvider, RunnerSpec, RunnerStatus,
    TerminateMode, WorkspaceMode,
};

/// Variablen, die ein Runner erben darf (deny-by-default, analog SBX-005).
pub const ENV_ALLOWLIST: [&str; 16] = [
    "PATH",
    // Datenverzeichnis und Log-Filter für die Logs des Runners (OBS-001).
    "BETON_HOME",
    "BETON_LOG",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
    "TMPDIR",
    "SHELL",
    "TZ",
    "SYSTEMROOT",
    // Konfigurationsverzeichnisse der Vendor-CLIs (nur Pfade): Der Harness soll dieselben
    // Sessions sehen wie im Terminal des Nutzers (Resume, History-Rebuild HAR-019).
    "CLAUDE_CONFIG_DIR",
    "CODEX_HOME",
];

/// Der lokale Provider.
pub struct LocalProvider {
    /// Runner-Kommando (Programm und Argumente), z. B. `beton runner`.
    command: Vec<String>,
    state_dir: PathBuf,
    /// Umgebung, aus der die Allowlist schöpft (Default: die des Daemons).
    inherit: BTreeMap<String, String>,
    provisioned: Mutex<HashMap<RunnerId, (Provisioned, Vec<String>)>>,
    running: Mutex<HashMap<RunnerId, Box<dyn ProcessHandle>>>,
}

impl std::fmt::Debug for LocalProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalProvider")
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

impl LocalProvider {
    pub fn new(command: Vec<String>, state_dir: PathBuf) -> Self {
        Self {
            command,
            state_dir,
            inherit: std::env::vars().collect(),
            provisioned: Mutex::default(),
            running: Mutex::default(),
        }
    }

    /// Andere Ausgangsumgebung (Tests).
    pub fn with_inherited_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.inherit = env;
        self
    }

    fn env_for(&self, allow: &[String]) -> Vec<(String, String)> {
        self.inherit
            .iter()
            .filter(|(k, _)| {
                ENV_ALLOWLIST.contains(&k.as_str())
                    || allow.contains(k)
                    // Binary-Präzedenz der Harnesses (HAR-003), z. B. BETON_CLAUDE_PATH.
                    || (k.starts_with("BETON_") && k.ends_with("_PATH"))
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

/// Capabilities des lokalen Providers (auch für `GET /v1/hosts/{id}`).
pub fn capabilities() -> RunnerCapabilities {
    RunnerCapabilities {
        isolation: Isolation::Process,
        workspace_modes: vec![WorkspaceMode::HostPath],
        persistent_workspace: true,
        interactive_exec: false,
        snapshot: false,
        resource_limits: Vec::new(),
        platforms: vec![platform()],
        max_concurrent: None,
    }
}

fn platform() -> String {
    format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
}

#[async_trait]
impl RunnerProvider for LocalProvider {
    fn id(&self) -> &str {
        "local"
    }

    fn capabilities(&self) -> RunnerCapabilities {
        capabilities()
    }

    async fn provision(&self, spec: &RunnerSpec) -> Result<Provisioned, RunnerError> {
        let mut map = self.provisioned.lock().await;
        if let Some((p, _)) = map.get(&spec.runner_id) {
            return Ok(p.clone());
        }
        if !spec.workspace.is_dir() {
            return Err(RunnerError::InvalidSpec(format!(
                "Workspace {} existiert nicht",
                spec.workspace.display()
            )));
        }
        std::fs::create_dir_all(self.state_dir.join(spec.runner_id.to_string()))?;
        let p = Provisioned {
            runner_id: spec.runner_id,
            workdir: spec.workspace.clone(),
        };
        map.insert(spec.runner_id, (p.clone(), spec.env_allowlist.clone()));
        Ok(p)
    }

    async fn start(
        &self,
        env: &Provisioned,
        boot: &RunnerBoot,
    ) -> Result<RunnerHandle, RunnerError> {
        let allow = self
            .provisioned
            .lock()
            .await
            .get(&env.runner_id)
            .map(|(_, allow)| allow.clone())
            .unwrap_or_default();
        let mut vars = self.env_for(&allow);
        let pairs = [
            (
                beton_runner::env::SOCKET,
                boot.tunnel_socket.display().to_string(),
            ),
            (beton_runner::env::SESSION, boot.session_id.to_string()),
            (beton_runner::env::RUNNER, env.runner_id.to_string()),
            (beton_runner::env::EPOCH, boot.epoch.to_string()),
            (beton_runner::env::HARNESS, boot.harness.clone()),
            (
                beton_runner::env::PARENT_PID,
                std::process::id().to_string(),
            ),
            (
                beton_runner::env::DEV,
                if boot.dev { "1" } else { "0" }.into(),
            ),
        ];
        vars.extend(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)));
        if let Some(s) = &boot.scenario {
            vars.push((beton_runner::env::SCENARIO.into(), s.display().to_string()));
        }
        if let Some(r) = &boot.resume {
            vars.push((beton_runner::env::RESUME.into(), r.clone()));
        }
        if let Some(m) = &boot.model {
            vars.push((beton_runner::env::MODEL.into(), m.clone()));
        }
        if let Some(a) = &boot.agent_ref {
            vars.push((beton_runner::env::AGENT_REF.into(), a.clone()));
        }
        if let Some(dir) = &boot.snapshots {
            vars.push((
                beton_runner::env::SNAPSHOTS.into(),
                dir.display().to_string(),
            ));
        }
        if let Some(fork) = &boot.fork {
            vars.push((
                beton_runner::env::FORK_PLAN.into(),
                fork.plan.display().to_string(),
            ));
            if fork.logged {
                vars.push((beton_runner::env::FORK_LOGGED.into(), "1".into()));
            }
        }
        if boot.harnesses != beton_harness::registry::HarnessLayers::default() {
            vars.push((
                beton_runner::env::HARNESSES.into(),
                serde_json::to_string(&boot.harnesses).unwrap_or_default(),
            ));
        }
        let (program, args) = self
            .command
            .split_first()
            .ok_or_else(|| RunnerError::InvalidSpec("Runner-Kommando fehlt".into()))?;
        let mut process = RealLauncher
            .launch(LaunchSpec {
                program: program.into(),
                args: args.to_vec(),
                env: vars,
                env_remove: Vec::new(),
                clear_env: true,
                cwd: Some(env.workdir.clone()),
            })
            .await?;
        let pid = process.id();
        if let Some(io) = process.take_io() {
            let mut stdin = io.stdin;
            stdin
                .write_all(format!("{}\n", boot.token).as_bytes())
                .await?;
            stdin.flush().await?;
            drop(stdin);
            // stdout leeren, damit der Runner nie an einer vollen Pipe hängt.
            let runner = env.runner_id;
            tokio::spawn(async move {
                let mut lines = BufReader::new(io.stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(%runner, "runner: {line}");
                }
            });
        }
        self.running.lock().await.insert(env.runner_id, process);
        Ok(RunnerHandle {
            runner_id: env.runner_id,
            pid,
        })
    }

    async fn exec(&self, h: &RunnerHandle, req: ExecRequest) -> Result<ExecOutput, RunnerError> {
        let workdir = self
            .provisioned
            .lock()
            .await
            .get(&h.runner_id)
            .map(|(p, _)| p.workdir.clone())
            .ok_or_else(|| RunnerError::NotFound(h.runner_id.to_string()))?;
        let out = tokio::process::Command::new(&req.program)
            .args(&req.args)
            .current_dir(workdir)
            .env_clear()
            .envs(self.env_for(&[]))
            .output()
            .await?;
        Ok(ExecOutput {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    async fn status(&self, h: &RunnerHandle) -> Result<RunnerStatus, RunnerError> {
        let mut running = self.running.lock().await;
        let Some(p) = running.get_mut(&h.runner_id) else {
            return Ok(RunnerStatus::Unknown);
        };
        Ok(match p.try_wait().await? {
            None => RunnerStatus::Running,
            Some(x) => RunnerStatus::Exited {
                code: x.code,
                stderr_tail: x.stderr_tail,
            },
        })
    }

    async fn terminate(&self, h: &RunnerHandle, mode: TerminateMode) -> Result<(), RunnerError> {
        let mut running = self.running.lock().await;
        let Some(p) = running.get_mut(&h.runner_id) else {
            return Err(RunnerError::NotFound(h.runner_id.to_string()));
        };
        match mode {
            TerminateMode::Force => {
                p.kill().await?;
            }
            TerminateMode::Graceful { grace } => {
                // SIGTERM sofort, nach `grace` SIGKILL (RUN-003 AC4).
                p.terminate(ShutdownTimeouts {
                    graceful: Duration::ZERO,
                    term: grace,
                })
                .await?;
            }
        }
        Ok(())
    }

    async fn list_managed(&self) -> Result<Vec<ManagedResource>, RunnerError> {
        let mut out = Vec::new();
        if !self.state_dir.exists() {
            return Ok(out);
        }
        for entry in std::fs::read_dir(&self.state_dir)? {
            let entry = entry?;
            if let Ok(runner_id) = entry.file_name().to_string_lossy().parse() {
                out.push(ManagedResource {
                    runner_id,
                    kind: "directory".into(),
                    location: entry.path().display().to_string(),
                });
            }
        }
        Ok(out)
    }
}

/// Eine Fabrik für Tests: Provider mit einem frei wählbaren Kommando.
pub fn for_command(command: &[&str], state_dir: PathBuf) -> Arc<LocalProvider> {
    Arc::new(LocalProvider::new(
        command.iter().map(|s| (*s).to_owned()).collect(),
        state_dir,
    ))
}
