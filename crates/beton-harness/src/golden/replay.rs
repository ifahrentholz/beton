//! Simulierter Harness-Prozess für Golden-Tests: spielt `raw.jsonl` als stdout ab und
//! zeichnet auf, was der Adapter nach stdin schreibt.

use std::io;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Notify, watch};

use crate::adapter::ExitInfo;
use crate::process::{LaunchSpec, ProcessHandle, ProcessIo, ProcessLauncher, ShutdownTimeouts};

/// Eine Zeile aus `raw.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawLine {
    /// Millisekunden seit Prozessstart (nur Information, Replay wartet nicht).
    pub ms: u64,
    /// Die Zeile erst ausgeben, nachdem der Adapter so viele stdin-Zeilen geschrieben hat.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub after_stdin: usize,
    /// Exakte stdout-Zeile des Vendor-CLIs (ohne Zeilenende).
    pub out: String,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Startet für jeden `launch` einen Replay derselben Aufnahme.
#[derive(Debug, Clone)]
pub struct ReplayLauncher {
    lines: Arc<Vec<RawLine>>,
    exit_code: i32,
    /// Alles, was Adapter nach stdin geschrieben haben, zeilenweise.
    stdin: Arc<Mutex<Vec<String>>>,
    launches: Arc<Mutex<Vec<LaunchSpec>>>,
}

impl ReplayLauncher {
    pub fn new(lines: Vec<RawLine>, exit_code: i32) -> Self {
        Self {
            lines: Arc::new(lines),
            exit_code,
            stdin: Arc::default(),
            launches: Arc::default(),
        }
    }

    pub fn stdin_lines(&self) -> Vec<String> {
        self.stdin.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn launches(&self) -> Vec<LaunchSpec> {
        self.launches.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl ProcessLauncher for ReplayLauncher {
    async fn launch(&self, spec: LaunchSpec) -> io::Result<Box<dyn ProcessHandle>> {
        if let Ok(mut l) = self.launches.lock() {
            l.push(spec);
        }
        let (adapter_stdin, our_stdin) = tokio::io::duplex(64 * 1024);
        let (mut our_stdout, adapter_stdout) = tokio::io::duplex(64 * 1024);
        let (count_tx, mut count_rx) = watch::channel(0usize);
        let (closed_tx, mut closed_rx) = watch::channel(false);
        let done = Arc::new(Notify::new());
        let finished = Arc::new(Mutex::new(false));

        // stdin des "Prozesses" mitschneiden und zählen.
        let stdin_log = self.stdin.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(our_stdin).lines();
            let mut n = 0;
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut log) = stdin_log.lock() {
                    log.push(line);
                }
                n += 1;
                let _ = count_tx.send(n);
            }
            // Wie die echte CLI: Ende von stdin beendet den Prozess.
            let _ = closed_tx.send(true);
        });

        // stdout abspielen.
        let lines = self.lines.clone();
        let done_tx = done.clone();
        let finished_flag = finished.clone();
        tokio::spawn(async move {
            for line in lines.iter() {
                if count_rx.wait_for(|n| *n >= line.after_stdin).await.is_err() {
                    break;
                }
                let mut bytes = line.out.clone().into_bytes();
                bytes.push(b'\n');
                if our_stdout.write_all(&bytes).await.is_err() {
                    break;
                }
            }
            // stdout erst schließen, wenn der Adapter stdin geschlossen hat.
            let _ = closed_rx.wait_for(|c| *c).await;
            drop(our_stdout);
            if let Ok(mut f) = finished_flag.lock() {
                *f = true;
            }
            done_tx.notify_waiters();
        });

        Ok(Box::new(ReplayProcess {
            io: Some(ProcessIo {
                stdin: Box::new(adapter_stdin),
                stdout: Box::new(adapter_stdout),
            }),
            done,
            finished,
            exit: ExitInfo {
                code: Some(self.exit_code),
                signal: None,
                stderr_tail: String::new(),
            },
        }))
    }
}

struct ReplayProcess {
    io: Option<ProcessIo>,
    done: Arc<Notify>,
    finished: Arc<Mutex<bool>>,
    exit: ExitInfo,
}

impl ReplayProcess {
    fn is_finished(&self) -> bool {
        self.finished.lock().map(|f| *f).unwrap_or(true)
    }
}

#[async_trait]
impl ProcessHandle for ReplayProcess {
    fn take_io(&mut self) -> Option<ProcessIo> {
        self.io.take()
    }

    fn id(&self) -> Option<u32> {
        None
    }

    async fn wait(&mut self) -> io::Result<ExitInfo> {
        let notified = self.done.notified();
        if !self.is_finished() {
            notified.await;
        }
        Ok(self.exit.clone())
    }

    async fn terminate(&mut self, _timeouts: ShutdownTimeouts) -> io::Result<ExitInfo> {
        self.io = None;
        Ok(self.exit.clone())
    }

    async fn kill(&mut self) -> io::Result<ExitInfo> {
        self.io = None;
        Ok(self.exit.clone())
    }
}
