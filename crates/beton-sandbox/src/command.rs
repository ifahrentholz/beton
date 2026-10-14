//! Was gestartet werden soll (unabhängig vom Backend).

use std::path::PathBuf;

/// Umgang mit einem Standard-Stream des Kindprozesses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Stdio {
    /// Pipe zum Elternprozess.
    #[default]
    Piped,
    /// `/dev/null`.
    Null,
    /// Vom Elternprozess geerbt (z. B. Terminal).
    Inherit,
}

/// Environment des Kindprozesses.
///
/// Ohne Sandbox (`backend: none`) gilt es wie angegeben (Erben, Entfernen, Setzen). Ein
/// isolierendes Backend erbt nie: Es startet mit dem expliziten Environment der
/// [`ResolvedPolicy`](crate::ResolvedPolicy) und ergänzt `set` (ohne gesperrte Variablen,
/// SBX-005).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvSpec {
    /// Nichts erben.
    pub clear: bool,
    pub set: Vec<(String, String)>,
    pub remove: Vec<String>,
}

/// Ein zu startender Prozess.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: EnvSpec,
    pub cwd: Option<PathBuf>,
    pub stdin: Stdio,
    pub stdout: Stdio,
    pub stderr: Stdio,
}

impl CommandSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            ..Self::default()
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.set.push((key.into(), value.into()));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn stdio(mut self, stdin: Stdio, stdout: Stdio, stderr: Stdio) -> Self {
        self.stdin = stdin;
        self.stdout = stdout;
        self.stderr = stderr;
        self
    }
}
