//! Prozess-Supervision für prozessbasierte Transporte (HAR-001).
//!
//! Harness-Prozesse laufen in einer eigenen Prozessgruppe (Unix) bzw. einem Job-Object
//! (Windows), damit beim Beenden der gesamte Baum verschwindet. Ablauf bei
//! `Shutdown::Graceful`: Der Adapter beendet das Protokoll (stdin schließen bzw.
//! `shutdown`-RPC), danach wartet [`ProcessHandle::terminate`] die Grace-Period, schickt
//! SIGTERM an die Gruppe und nach weiteren 5 s SIGKILL.
//!
//! [`ProcessLauncher`] ist die Naht für Tests: Golden-Transcripts ersetzen den echten
//! Prozess durch einen simulierten (HAR-025).

use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::task::JoinHandle;

use crate::adapter::ExitInfo;

/// Höchstens so viel stderr wird für `harness.exited` aufbewahrt.
pub const STDERR_TAIL_BYTES: usize = 8 * 1024;

/// Was gestartet werden soll.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Aus der geerbten Umgebung entfernen (z. B. API-Keys bei Subscription, HAR-015).
    pub env_remove: Vec<String>,
    /// Nichts erben; nur `env` gilt (deny-by-default, RUN-002 AC4).
    pub clear_env: bool,
    pub cwd: Option<PathBuf>,
}

/// stdin und stdout eines Prozesses als Byte-Ströme.
pub struct ProcessIo {
    pub stdin: Box<dyn AsyncWrite + Send + Unpin>,
    pub stdout: Box<dyn AsyncRead + Send + Unpin>,
}

impl std::fmt::Debug for ProcessIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProcessIo")
    }
}

/// Fristen beim Beenden (Default 10 s + 5 s, also höchstens 15 s, HAR-001 AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownTimeouts {
    /// Wartezeit nach dem protokolleigenen Ende, bevor SIGTERM kommt.
    pub graceful: Duration,
    /// Wartezeit nach SIGTERM, bevor SIGKILL kommt.
    pub term: Duration,
}

impl Default for ShutdownTimeouts {
    fn default() -> Self {
        Self {
            graceful: Duration::from_secs(10),
            term: Duration::from_secs(5),
        }
    }
}

/// Ein laufender (echter oder simulierter) Prozess.
#[async_trait]
pub trait ProcessHandle: Send {
    /// stdin/stdout; genau einmal entnehmbar.
    fn take_io(&mut self) -> Option<ProcessIo>;
    fn id(&self) -> Option<u32>;
    /// Wartet auf das Ende des Prozesses (z. B. um einen Crash zu erkennen).
    async fn wait(&mut self) -> io::Result<ExitInfo>;
    /// Schon beendet? Blockiert nicht.
    async fn try_wait(&mut self) -> io::Result<Option<ExitInfo>> {
        Ok(None)
    }
    /// Beendet nach dem protokolleigenen Ende: warten, SIGTERM, SIGKILL – immer für den
    /// gesamten Prozessbaum.
    async fn terminate(&mut self, timeouts: ShutdownTimeouts) -> io::Result<ExitInfo>;
    /// Sofort SIGKILL für den gesamten Prozessbaum.
    async fn kill(&mut self) -> io::Result<ExitInfo>;
}

/// Startet Harness-Prozesse.
#[async_trait]
pub trait ProcessLauncher: Send + Sync {
    async fn launch(&self, spec: LaunchSpec) -> io::Result<Box<dyn ProcessHandle>>;
}

/// Startet echte Prozesse mit Prozessgruppe bzw. Job-Object.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealLauncher;

#[async_trait]
impl ProcessLauncher for RealLauncher {
    async fn launch(&self, spec: LaunchSpec) -> io::Result<Box<dyn ProcessHandle>> {
        let mut cmd = tokio::process::Command::new(&spec.program);
        if spec.clear_env {
            cmd.env_clear();
        }
        for key in &spec.env_remove {
            cmd.env_remove(key);
        }
        cmd.args(&spec.args)
            .envs(spec.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &spec.cwd {
            cmd.current_dir(cwd);
        }
        let mut wrap = CommandWrap::from(cmd);
        #[cfg(unix)]
        wrap.wrap(process_wrap::tokio::ProcessGroup::leader());
        #[cfg(windows)]
        wrap.wrap(process_wrap::tokio::JobObject);
        wrap.wrap(KillOnDrop);
        let mut child = wrap.spawn()?;
        let stdin = child.stdin().take();
        let stdout = child.stdout().take();
        let stderr = child.stderr().take();
        let tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_BYTES)));
        let stderr_task = stderr.map(|mut stderr| {
            let tail = tail.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = stderr.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut t) = tail.lock() {
                        t.extend(&buf[..n]);
                        let excess = t.len().saturating_sub(STDERR_TAIL_BYTES);
                        t.drain(..excess);
                    }
                }
            })
        });
        let io = match (stdin, stdout) {
            (Some(stdin), Some(stdout)) => Some(ProcessIo {
                stdin: Box::new(stdin),
                stdout: Box::new(stdout),
            }),
            _ => None,
        };
        Ok(Box::new(RealProcess {
            child,
            io,
            tail,
            stderr_task,
            exit: None,
        }))
    }
}

struct RealProcess {
    child: Box<dyn ChildWrapper>,
    io: Option<ProcessIo>,
    tail: Arc<Mutex<VecDeque<u8>>>,
    stderr_task: Option<JoinHandle<()>>,
    exit: Option<ExitInfo>,
}

impl RealProcess {
    async fn finish(&mut self, status: ExitStatus) -> ExitInfo {
        // Restliche Kinder der Gruppe dürfen den Leader nicht überleben.
        self.signal_group(Signal::Kill);
        if let Some(task) = self.stderr_task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(1), task).await;
        }
        let stderr_tail = self
            .tail
            .lock()
            .map(|t| String::from_utf8_lossy(&t.iter().copied().collect::<Vec<_>>()).into_owned())
            .unwrap_or_default();
        let info = ExitInfo {
            code: status.code(),
            signal: signal_name(&status),
            stderr_tail,
        };
        self.exit = Some(info.clone());
        info
    }

    fn signal_group(&self, signal: Signal) {
        #[cfg(unix)]
        {
            let _ = self.child.signal(match signal {
                Signal::Term => 15,
                Signal::Kill => 9,
            });
        }
        #[cfg(not(unix))]
        {
            // Windows: Das Job-Object beendet beim Kill den ganzen Baum; SIGTERM gibt es nicht.
            let _ = signal;
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Signal {
    Term,
    Kill,
}

#[async_trait]
impl ProcessHandle for RealProcess {
    fn take_io(&mut self) -> Option<ProcessIo> {
        self.io.take()
    }

    fn id(&self) -> Option<u32> {
        self.child.id()
    }

    async fn wait(&mut self) -> io::Result<ExitInfo> {
        if let Some(exit) = &self.exit {
            return Ok(exit.clone());
        }
        let status = self.child.wait().await?;
        Ok(self.finish(status).await)
    }

    async fn try_wait(&mut self) -> io::Result<Option<ExitInfo>> {
        if let Some(exit) = &self.exit {
            return Ok(Some(exit.clone()));
        }
        match self.child.try_wait()? {
            Some(status) => Ok(Some(self.finish(status).await)),
            None => Ok(None),
        }
    }

    async fn terminate(&mut self, timeouts: ShutdownTimeouts) -> io::Result<ExitInfo> {
        if let Some(exit) = &self.exit {
            return Ok(exit.clone());
        }
        // stdin schließen gehört zum protokolleigenen Ende.
        self.io = None;
        if let Ok(status) = tokio::time::timeout(timeouts.graceful, self.child.wait()).await {
            return Ok(self.finish(status?).await);
        }
        #[cfg(unix)]
        {
            self.signal_group(Signal::Term);
            if let Ok(status) = tokio::time::timeout(timeouts.term, self.child.wait()).await {
                return Ok(self.finish(status?).await);
            }
        }
        self.kill().await
    }

    async fn kill(&mut self) -> io::Result<ExitInfo> {
        if let Some(exit) = &self.exit {
            return Ok(exit.clone());
        }
        self.child.start_kill()?;
        let status = self.child.wait().await?;
        Ok(self.finish(status).await)
    }
}

#[cfg(unix)]
fn signal_name(status: &ExitStatus) -> Option<String> {
    use std::os::unix::process::ExitStatusExt;
    status.signal().map(|s| {
        match s {
            1 => "SIGHUP",
            2 => "SIGINT",
            6 => "SIGABRT",
            9 => "SIGKILL",
            11 => "SIGSEGV",
            13 => "SIGPIPE",
            15 => "SIGTERM",
            _ => return format!("SIG{s}"),
        }
        .to_owned()
    })
}

#[cfg(not(unix))]
fn signal_name(_status: &ExitStatus) -> Option<String> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    use std::time::Instant;

    use tokio::io::AsyncBufReadExt;

    use super::*;

    fn sh(script: &str) -> LaunchSpec {
        LaunchSpec {
            program: "sh".into(),
            args: vec!["-c".into(), script.into()],
            ..LaunchSpec::default()
        }
    }

    /// Lebt noch irgendein Prozess der Gruppe?
    fn group_alive(pgid: u32) -> bool {
        std::process::Command::new("kill")
            .args(["-0", "--", &format!("-{pgid}")])
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    async fn wait_until_gone(pgid: u32) -> bool {
        for _ in 0..50 {
            if !group_alive(pgid) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    #[test]
    fn har_001_ac2_default_timeouts_stay_within_15_seconds() {
        let t = ShutdownTimeouts::default();
        assert!(t.graceful + t.term <= Duration::from_secs(15));
    }

    #[tokio::test]
    async fn har_001_ac2_graceful_shutdown_kills_whole_process_group() {
        // Ein Harness mit Kindprozessen, die SIGTERM ignorieren: erst SIGKILL räumt auf.
        let mut p = RealLauncher
            .launch(sh(
                "trap '' TERM; sleep 300 & sleep 300 & echo bereit; wait",
            ))
            .await
            .unwrap();
        let pgid = p.id().unwrap();
        let mut io = p.take_io().unwrap();
        let mut line = String::new();
        tokio::io::BufReader::new(&mut io.stdout)
            .read_line(&mut line)
            .await
            .unwrap();
        assert_eq!(line.trim(), "bereit");
        assert!(group_alive(pgid));
        drop(io);

        let started = Instant::now();
        let exit = p
            .terminate(ShutdownTimeouts {
                graceful: Duration::from_millis(200),
                term: Duration::from_millis(200),
            })
            .await
            .unwrap();
        assert_eq!(exit.signal.as_deref(), Some("SIGKILL"), "{exit:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(wait_until_gone(pgid).await, "Prozessgruppe lebt noch");
    }

    #[tokio::test]
    async fn har_001_ac2_term_is_enough_for_cooperative_harness() {
        let mut p = RealLauncher
            .launch(sh("sleep 300 & echo bereit; wait"))
            .await
            .unwrap();
        let pgid = p.id().unwrap();
        let mut io = p.take_io().unwrap();
        let mut line = String::new();
        tokio::io::BufReader::new(&mut io.stdout)
            .read_line(&mut line)
            .await
            .unwrap();
        let exit = p
            .terminate(ShutdownTimeouts {
                graceful: Duration::from_millis(100),
                term: Duration::from_secs(5),
            })
            .await
            .unwrap();
        assert_eq!(exit.signal.as_deref(), Some("SIGTERM"), "{exit:?}");
        assert!(wait_until_gone(pgid).await);
    }

    #[tokio::test]
    async fn har_001_ac1_crash_reports_code_and_stderr_tail() {
        let big = "x".repeat(10_000);
        let mut p = RealLauncher
            .launch(sh(&format!(
                "echo '{big}' >&2; echo 'Fehler: kaputt' >&2; exit 3"
            )))
            .await
            .unwrap();
        let exit = p.wait().await.unwrap();
        assert_eq!(exit.code, Some(3));
        assert_eq!(exit.signal, None);
        assert!(exit.stderr_tail.len() <= STDERR_TAIL_BYTES);
        assert!(
            exit.stderr_tail.ends_with("Fehler: kaputt\n"),
            "{:?}",
            &exit.stderr_tail[exit.stderr_tail.len() - 40..]
        );
    }
}

/// Höchstens so viel stdout liest [`run_once`] (Einmal-Aufrufe liefern kurze Antworten).
pub const ONE_SHOT_MAX_STDOUT: usize = 4 * 1024 * 1024;

/// Ergebnis von [`run_once`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnceOutput {
    pub stdout: Vec<u8>,
    pub exit: ExitInfo,
}

/// Startet einen Prozess für einen Einmal-Aufruf (SES-010): schreibt `stdin` und schließt es,
/// liest stdout bis zum Ende und wartet auf das Prozessende. Nach `timeout` wird der gesamte
/// Prozessbaum beendet.
pub async fn run_once(
    launcher: &dyn ProcessLauncher,
    spec: LaunchSpec,
    stdin: &[u8],
    timeout: Duration,
) -> io::Result<OnceOutput> {
    use tokio::io::AsyncWriteExt;
    let mut process = launcher.launch(spec).await?;
    let io = process
        .take_io()
        .ok_or_else(|| io::Error::other("stdin/stdout fehlen"))?;
    let ProcessIo {
        stdin: mut input,
        stdout: mut output,
    } = io;
    let input_bytes = stdin.to_vec();
    let work = async move {
        let writer = async move {
            // Ein Prozess, der stdin nicht liest, darf das Lesen von stdout nicht blockieren.
            let _ = input.write_all(&input_bytes).await;
            let _ = input.shutdown().await;
            drop(input);
        };
        let reader = async move {
            let mut out = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                match output.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if out.len() + n > ONE_SHOT_MAX_STDOUT {
                            return Err(io::Error::other("Ausgabe zu groß"));
                        }
                        out.extend_from_slice(&buf[..n]);
                    }
                }
            }
            Ok(out)
        };
        let ((), out) = tokio::join!(writer, reader);
        out
    };
    match tokio::time::timeout(timeout, work).await {
        Ok(Ok(stdout)) => {
            let exit = match tokio::time::timeout(Duration::from_secs(5), process.wait()).await {
                Ok(r) => r?,
                Err(_) => process.kill().await?,
            };
            Ok(OnceOutput { stdout, exit })
        }
        Ok(Err(e)) => {
            let _ = process.kill().await;
            Err(e)
        }
        Err(_) => {
            let _ = process.kill().await;
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("keine Antwort innerhalb von {timeout:?}"),
            ))
        }
    }
}
