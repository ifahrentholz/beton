//! Aufruf der `git`-CLI mit kontrollierter Umgebung.
//!
//! beton spricht Git ausschließlich über die installierte CLI: dieselbe Semantik wie im
//! Terminal des Nutzers (`.gitignore`, Hooks, Konfiguration), keine zweite Implementierung.
//! Variablen wie `GIT_DIR` aus der Umgebung des Daemons würden jeden Aufruf umlenken; sie werden
//! entfernt. Rückfragen sind abgeschaltet (`GIT_TERMINAL_PROMPT=0`, SSH im Batch-Modus).

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Variablen, die Git auf ein anderes Repository oder einen anderen Index umlenken.
const REDIRECTING_VARS: [&str; 9] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_QUARANTINE_PATH",
];

/// Fehler beim Aufruf von Git.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git ist nicht installiert oder nicht im PATH")]
    NotInstalled,
    #[error("git {args}: E/A-Fehler: {source}")]
    Io {
        args: String,
        source: std::io::Error,
    },
    #[error("git {args} endete mit {code:?}: {stderr}")]
    Failed {
        args: String,
        code: Option<i32>,
        stderr: String,
    },
    #[error("git {args}: keine Antwort nach {secs} s")]
    Timeout { args: String, secs: u64 },
}

/// Ergebnis eines Aufrufs unabhängig vom Exit-Code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Ein Git-Aufruf in einem Verzeichnis, optional mit eigenem Git-Verzeichnis, Worktree und
/// Index (Schatten-Repository der Turn-Snapshots).
#[derive(Debug, Clone)]
pub struct Git {
    dir: PathBuf,
    git_dir: Option<PathBuf>,
    work_tree: Option<PathBuf>,
    index: Option<PathBuf>,
    timeout: Option<Duration>,
    config: Vec<String>,
}

impl Git {
    /// Git im Verzeichnis `dir` (wie `git -C dir`).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            git_dir: None,
            work_tree: None,
            index: None,
            timeout: None,
            config: Vec::new(),
        }
    }

    /// Eigenes Git-Verzeichnis für den Worktree `work_tree`.
    pub fn with_git_dir(
        mut self,
        git_dir: impl Into<PathBuf>,
        work_tree: impl Into<PathBuf>,
    ) -> Self {
        let work_tree = work_tree.into();
        self.dir.clone_from(&work_tree);
        self.git_dir = Some(git_dir.into());
        self.work_tree = Some(work_tree);
        self
    }

    /// Eigene Index-Datei statt `<git-dir>/index`.
    pub fn with_index(mut self, index: impl Into<PathBuf>) -> Self {
        self.index = Some(index.into());
        self
    }

    /// Bricht den Aufruf nach `timeout` ab.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Zusätzliche `-c key=value`-Optionen.
    pub fn with_config(mut self, key_value: impl Into<String>) -> Self {
        self.config.push(key_value.into());
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.dir);
        for var in REDIRECTING_VARS {
            cmd.env_remove(var);
        }
        cmd.env("GIT_TERMINAL_PROMPT", "0")
            // Lesende Aufrufe sollen den Index nicht nebenbei auffrischen (keine Sperren).
            .env("GIT_OPTIONAL_LOCKS", "0")
            // Stabile, unübersetzte Ausgaben.
            .env("LC_ALL", "C")
            .env("LANGUAGE", "C");
        if std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() {
            cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
        }
        if let Some(index) = &self.index {
            cmd.env("GIT_INDEX_FILE", index);
        }
        if let Some(git_dir) = &self.git_dir {
            cmd.arg("--git-dir").arg(git_dir);
        }
        if let Some(work_tree) = &self.work_tree {
            cmd.arg("--work-tree").arg(work_tree);
        }
        for c in &self.config {
            cmd.arg("-c").arg(c);
        }
        cmd.args(args);
        cmd
    }

    /// Führt `git <args>` aus und liefert das Ergebnis unabhängig vom Exit-Code.
    pub fn run(&self, args: &[&str]) -> Result<Output, GitError> {
        self.run_with_input(args, None)
    }

    /// Wie [`Git::run`], mit Daten auf stdin.
    pub fn run_with_input(&self, args: &[&str], input: Option<&[u8]>) -> Result<Output, GitError> {
        let joined = args.join(" ");
        let io = |source: std::io::Error| {
            if source.kind() == std::io::ErrorKind::NotFound {
                GitError::NotInstalled
            } else {
                GitError::Io {
                    args: joined.clone(),
                    source,
                }
            }
        };
        let mut child = self
            .command(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(io)?;
        if let (Some(data), Some(mut stdin)) = (input, child.stdin.take()) {
            stdin.write_all(data).map_err(io)?;
        }
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let out_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut s) = stdout {
                let _ = s.read_to_end(&mut buf);
            }
            buf
        });
        let err_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut s) = stderr {
                let _ = s.read_to_end(&mut buf);
            }
            buf
        });
        let status = match self.timeout {
            None => child.wait().map_err(io)?,
            Some(limit) => {
                let deadline = Instant::now() + limit;
                loop {
                    if let Some(status) = child.try_wait().map_err(io)? {
                        break status;
                    }
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        // Leser nicht abwarten: Kindprozesse (ssh) können die Pipes halten.
                        return Err(GitError::Timeout {
                            args: joined,
                            secs: limit.as_secs(),
                        });
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        let stdout = out_reader.join().unwrap_or_default();
        let stderr = err_reader.join().unwrap_or_default();
        Ok(Output {
            code: status.code(),
            stdout,
            stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
        })
    }

    /// Zählt NUL-getrennte Datensätze der Ausgabe von `git <args>` und beendet den Prozess,
    /// sobald `limit` erreicht ist (für große Verzeichnisbäume).
    pub fn count_records(&self, args: &[&str], limit: usize) -> Result<usize, GitError> {
        let joined = args.join(" ");
        let mut child = self
            .command(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|source| {
                if source.kind() == std::io::ErrorKind::NotFound {
                    GitError::NotInstalled
                } else {
                    GitError::Io {
                        args: joined.clone(),
                        source,
                    }
                }
            })?;
        let mut count = 0;
        if let Some(mut out) = child.stdout.take() {
            let mut buf = [0u8; 64 * 1024];
            loop {
                match out.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        count += buf[..n].iter().filter(|b| **b == 0).count();
                        if count >= limit {
                            let _ = child.kill();
                            break;
                        }
                    }
                }
            }
        }
        let status = child.wait().map_err(|source| GitError::Io {
            args: joined.clone(),
            source,
        })?;
        if count < limit && !status.success() {
            return Err(GitError::Failed {
                args: joined,
                code: status.code(),
                stderr: String::new(),
            });
        }
        Ok(count)
    }

    /// Führt `git <args>` aus und verlangt Exit-Code 0; liefert stdout.
    pub fn ok(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let out = self.run(args)?;
        if out.success() {
            Ok(out.stdout)
        } else {
            Err(GitError::Failed {
                args: args.join(" "),
                code: out.code,
                stderr: out.stderr,
            })
        }
    }

    /// Wie [`Git::ok`], stdout als getrimmter Text.
    pub fn text(&self, args: &[&str]) -> Result<String, GitError> {
        Ok(String::from_utf8_lossy(&self.ok(args)?).trim().to_owned())
    }
}

/// Teilt NUL-getrennte Ausgabe (`-z`) in Felder; ein abschließendes NUL ergibt kein leeres Feld.
pub fn split_nul(bytes: &[u8]) -> Vec<String> {
    let mut parts: Vec<String> = bytes
        .split(|b| *b == 0)
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    if parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    parts
}

/// Ist `git` installiert?
pub fn available() -> bool {
    Git::new(".").run(&["--version"]).is_ok_and(|o| o.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_nul_drops_trailing_terminator() {
        assert_eq!(split_nul(b"a\0b\0"), vec!["a", "b"]);
        assert_eq!(split_nul(b""), Vec::<String>::new());
        assert_eq!(split_nul(b"a"), vec!["a"]);
    }

    #[test]
    fn failing_command_reports_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let err = Git::new(dir.path())
            .ok(&["rev-parse", "--verify", "HEAD"])
            .unwrap_err();
        assert!(matches!(err, GitError::Failed { .. }), "{err}");
    }
}
