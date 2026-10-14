//! Ein gestarteter (ggf. isolierter) Prozessbaum.
//!
//! Der Prozess führt eine eigene Prozessgruppe (Unix) bzw. ein Job-Object (Windows), damit
//! Signale und das Beenden immer den ganzen Baum treffen; beim `Drop` wird er beendet. Ein
//! Backend kann zusätzlich einen [`TreeGuard`] anhängen, der Prozesse außerhalb der Gruppe
//! erfasst und beendet (macOS: Nachfahren-Erfassung, SBX-013 AC2) bzw. Ressourcen des Starts
//! hält (Linux: Status-Kanal des Launchers).

use std::io;
use std::process::ExitStatus;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use beton_core::event::SandboxViolation;

use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::io::AsyncReadExt as _;
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};

use crate::command::{CommandSpec, Stdio};

/// Zusatzaufgaben eines Backends am Prozessbaum.
pub(crate) trait TreeGuard: Send {
    /// Erfasst den Baum, bevor die Wurzel ein Signal bekommt (macOS: Nachfahren-Erfassung).
    fn capture(&mut self) {}
    /// Beendet Prozesse, die die Prozessgruppe verlassen haben (best effort).
    fn kill_tree(&mut self) {}
}

/// Gemeinsame Liste erkannter Verstöße eines Baums.
pub(crate) type Violations = Arc<Mutex<Vec<SandboxViolation>>>;

/// Ein laufender Prozessbaum.
pub struct SandboxedChild {
    inner: Box<dyn ChildWrapper>,
    violations: Violations,
    guard: Option<Box<dyn TreeGuard>>,
}

impl std::fmt::Debug for SandboxedChild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxedChild")
            .field("pid", &self.inner.id())
            .finish()
    }
}

/// Ausgabe eines beendeten Prozesses (siehe [`SandboxedChild::output`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Drop for SandboxedChild {
    fn drop(&mut self) {
        if let Some(g) = &mut self.guard {
            g.kill_tree();
        }
    }
}

impl SandboxedChild {
    /// Hängt Zusatzaufgaben und die Verstoß-Liste eines Backends an.
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "macos")),
        allow(
            dead_code,
            reason = "nur Linux- und macOS-Backends erfassen Prozessbäume"
        )
    )]
    pub(crate) fn with_guard(mut self, guard: Box<dyn TreeGuard>, violations: Violations) -> Self {
        self.guard = Some(guard);
        self.violations = violations;
        self
    }

    /// Bisher erkannte Verstöße (z. B. ein erreichtes Limit, SBX-013 AC1).
    pub fn violations(&self) -> Vec<SandboxViolation> {
        self.violations
            .lock()
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    pub fn id(&self) -> Option<u32> {
        self.inner.id()
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.inner.stdin().take()
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.inner.stdout().take()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.inner.stderr().take()
    }

    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.inner.wait().await
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }

    /// Beendet den ganzen Baum (SIGKILL bzw. Job-Object).
    ///
    /// Der Guard erfasst die Nachfahren, solange der Wurzelprozess noch lebt: Stirbt die Wurzel
    /// vorher, hängt ein per `setsid` abgelöster Prozess am Init-Prozess und ist weder über die
    /// Eltern-PID noch über die Prozessgruppe zu finden (SBX-013 AC2).
    pub fn start_kill(&mut self) -> io::Result<()> {
        if let Some(g) = &mut self.guard {
            g.capture();
        }
        let r = self.inner.start_kill();
        if let Some(g) = &mut self.guard {
            g.kill_tree();
        }
        r
    }

    /// Schickt ein Signal an die Prozessgruppe (Unix). Wie bei [`Self::start_kill`] erfasst der
    /// Guard den Baum vorher: Endet die Wurzel am Signal, sind per `setsid` abgelöste Nachfahren
    /// sonst nicht mehr zu finden, auch nicht beim späteren Beenden (SBX-013 AC2).
    #[cfg(unix)]
    pub fn signal_tree(&mut self, signal: i32) -> io::Result<()> {
        if let Some(g) = &mut self.guard {
            g.capture();
        }
        self.inner.signal(signal)
    }

    /// Wartet auf das Ende und sammelt stdout und stderr (sofern als Pipe angelegt). Nach
    /// `timeout` wird der Baum beendet und `TimedOut` gemeldet.
    pub async fn output(mut self, timeout: Duration) -> io::Result<Output> {
        match tokio::time::timeout(timeout, collect(&mut self)).await {
            Ok(r) => r,
            Err(_) => {
                let _ = self.start_kill();
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("keine Antwort innerhalb von {timeout:?}"),
                ))
            }
        }
    }

    /// Wie [`SandboxedChild::output`], ohne Frist.
    pub async fn wait_with_output(mut self) -> io::Result<Output> {
        collect(&mut self).await
    }
}

async fn collect(child: &mut SandboxedChild) -> io::Result<Output> {
    let stdout = child.take_stdout();
    let stderr = child.take_stderr();
    drop(child.take_stdin());
    async fn read_all<R: tokio::io::AsyncRead + Unpin>(r: Option<R>) -> io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        if let Some(mut r) = r {
            r.read_to_end(&mut buf).await?;
        }
        Ok(buf)
    }
    let (out, err) = tokio::join!(read_all(stdout), read_all(stderr));
    let status = child.inner.wait().await?;
    Ok(Output {
        status,
        stdout: out?,
        stderr: err?,
    })
}

fn stdio(mode: Stdio) -> std::process::Stdio {
    match mode {
        Stdio::Piped => std::process::Stdio::piped(),
        Stdio::Null => std::process::Stdio::null(),
        Stdio::Inherit => std::process::Stdio::inherit(),
    }
}

/// Baut das `tokio`-Kommando für `program` mit den Streams aus `cmd`. Environment und
/// Arbeitsverzeichnis setzt der jeweilige Provider.
#[expect(
    clippy::disallowed_methods,
    reason = "SBX-001: der einzige Ort, an dem Harness- und Tool-Prozesse entstehen"
)]
pub(crate) fn command(program: &std::path::Path, cmd: &CommandSpec) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(program);
    c.stdin(stdio(cmd.stdin))
        .stdout(stdio(cmd.stdout))
        .stderr(stdio(cmd.stderr));
    c
}

/// Startet `cmd` als Prozessgruppen-Leader (Unix) bzw. im Job-Object (Windows); der Baum
/// endet mit dem `Drop` des Kindes.
pub(crate) fn spawn(cmd: tokio::process::Command) -> io::Result<SandboxedChild> {
    let mut wrap = CommandWrap::from(cmd);
    #[cfg(unix)]
    wrap.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    wrap.wrap(process_wrap::tokio::JobObject);
    wrap.wrap(KillOnDrop);
    Ok(SandboxedChild {
        inner: wrap.spawn()?,
        violations: Violations::default(),
        guard: None,
    })
}
