//! Aufnahme echter Vendor-CLIs für Golden-Transcripts (HAR-025, `beton dev record-golden`).
//!
//! [`RecordingLauncher`] sitzt zwischen Adapter und echtem Prozess: Er reicht stdin und
//! stdout durch und notiert jede stdout-Zeile mit Zeitmarke und der Zahl der bis dahin
//! gesendeten stdin-Zeilen (`after_stdin`), damit das Replay dieselbe Reihenfolge erzwingt.

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::RawLine;
use crate::adapter::ExitInfo;
use crate::process::{LaunchSpec, ProcessHandle, ProcessIo, ProcessLauncher, ShutdownTimeouts};

#[derive(Clone)]
pub struct RecordingLauncher {
    inner: Arc<dyn ProcessLauncher>,
    raw: Arc<Mutex<Vec<RawLine>>>,
    stdin: Arc<Mutex<Vec<String>>>,
}

impl std::fmt::Debug for RecordingLauncher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecordingLauncher")
    }
}

impl RecordingLauncher {
    pub fn new(inner: Arc<dyn ProcessLauncher>) -> Self {
        Self {
            inner,
            raw: Arc::default(),
            stdin: Arc::default(),
        }
    }

    pub fn raw_lines(&self) -> Vec<RawLine> {
        self.raw.lock().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn stdin_lines(&self) -> Vec<String> {
        self.stdin.lock().map(|r| r.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl ProcessLauncher for RecordingLauncher {
    async fn launch(&self, spec: LaunchSpec) -> io::Result<Box<dyn ProcessHandle>> {
        let mut inner = self.inner.launch(spec).await?;
        let real = inner
            .take_io()
            .ok_or_else(|| io::Error::other("Prozess ohne stdin/stdout"))?;
        let (adapter_stdin, tap_stdin) = tokio::io::duplex(64 * 1024);
        let (mut tap_stdout, adapter_stdout) = tokio::io::duplex(64 * 1024);
        let started = Instant::now();

        let stdin_log = self.stdin.clone();
        let mut real_stdin = real.stdin;
        tokio::spawn(async move {
            let mut lines = BufReader::new(tap_stdin).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut log) = stdin_log.lock() {
                    log.push(line.clone());
                }
                if real_stdin
                    .write_all(format!("{line}\n").as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
                let _ = real_stdin.flush().await;
            }
            // Adapter schließt stdin → echtes stdin schließen.
            drop(real_stdin);
        });

        let (raw_log, stdin_log) = (self.raw.clone(), self.stdin.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(real.stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let after_stdin = stdin_log.lock().map(|l| l.len()).unwrap_or(0);
                if let Ok(mut log) = raw_log.lock() {
                    log.push(RawLine {
                        ms: started.elapsed().as_millis() as u64,
                        after_stdin,
                        out: line.clone(),
                    });
                }
                if tap_stdout
                    .write_all(format!("{line}\n").as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        Ok(Box::new(Recorded {
            inner,
            io: Some(ProcessIo {
                stdin: Box::new(adapter_stdin),
                stdout: Box::new(adapter_stdout),
            }),
        }))
    }
}

struct Recorded {
    inner: Box<dyn ProcessHandle>,
    io: Option<ProcessIo>,
}

#[async_trait]
impl ProcessHandle for Recorded {
    fn take_io(&mut self) -> Option<ProcessIo> {
        self.io.take()
    }
    fn id(&self) -> Option<u32> {
        self.inner.id()
    }
    async fn wait(&mut self) -> io::Result<ExitInfo> {
        self.inner.wait().await
    }
    async fn terminate(&mut self, timeouts: ShutdownTimeouts) -> io::Result<ExitInfo> {
        self.inner.terminate(timeouts).await
    }
    async fn kill(&mut self) -> io::Result<ExitInfo> {
        self.inner.kill().await
    }
}
