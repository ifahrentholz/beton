//! Ein gestarteter (ggf. isolierter) Prozessbaum.
//!
//! Der Prozess führt eine eigene Prozessgruppe (Unix) bzw. ein Job-Object (Windows), damit
//! Signale und das Beenden immer den ganzen Baum treffen; beim `Drop` wird er beendet.

use std::io;
use std::process::ExitStatus;
use std::time::Duration;

use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::io::AsyncReadExt as _;
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};

use crate::command::{CommandSpec, Stdio};

/// Ein laufender Prozessbaum.
pub struct SandboxedChild {
    inner: Box<dyn ChildWrapper>,
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

impl SandboxedChild {
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
    pub fn start_kill(&mut self) -> io::Result<()> {
        self.inner.start_kill()
    }

    /// Schickt ein Signal an die Prozessgruppe (Unix).
    #[cfg(unix)]
    pub fn signal_tree(&self, signal: i32) -> io::Result<()> {
        self.inner.signal(signal)
    }

    /// Wartet auf das Ende und sammelt stdout und stderr (sofern als Pipe angelegt). Nach
    /// `timeout` wird der Baum beendet und `TimedOut` gemeldet.
    pub async fn output(mut self, timeout: Duration) -> io::Result<Output> {
        let stdout = self.take_stdout();
        let stderr = self.take_stderr();
        drop(self.take_stdin());
        let read_all = |r: Option<Box<dyn tokio::io::AsyncRead + Send + Unpin>>| async move {
            let mut buf = Vec::new();
            if let Some(mut r) = r {
                r.read_to_end(&mut buf).await?;
            }
            Ok::<_, io::Error>(buf)
        };
        let work = async {
            let (out, err) = tokio::join!(
                read_all(stdout.map(|s| Box::new(s) as Box<_>)),
                read_all(stderr.map(|s| Box::new(s) as Box<_>))
            );
            let status = self.inner.wait().await?;
            Ok::<_, io::Error>(Output {
                status,
                stdout: out?,
                stderr: err?,
            })
        };
        match tokio::time::timeout(timeout, work).await {
            Ok(r) => r,
            Err(_) => {
                let _ = self.inner.start_kill();
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("keine Antwort innerhalb von {timeout:?}"),
                ))
            }
        }
    }
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
    })
}
