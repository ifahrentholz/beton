//! Was gestartet werden soll (unabhängig vom Backend).

use std::path::PathBuf;

/// Umgang mit einem Standard-Stream des Kindprozesses.
#[derive(Debug, Clone, Default)]
pub enum Stdio {
    /// Pipe zum Elternprozess.
    #[default]
    Piped,
    /// `/dev/null`.
    Null,
    /// Vom Elternprozess geerbt (z. B. Terminal).
    Inherit,
    /// Ein übergebener Deskriptor, z. B. vom Exec-Broker per `SCM_RIGHTS` empfangen (SBX-002);
    /// das Kind bekommt eine Kopie.
    #[cfg(unix)]
    Fd(std::sync::Arc<std::os::fd::OwnedFd>),
}

impl PartialEq for Stdio {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Stdio::Piped, Stdio::Piped)
            | (Stdio::Null, Stdio::Null)
            | (Stdio::Inherit, Stdio::Inherit) => true,
            #[cfg(unix)]
            (Stdio::Fd(a), Stdio::Fd(b)) => {
                use std::os::fd::AsRawFd as _;
                a.as_raw_fd() == b.as_raw_fd()
            }
            _ => false,
        }
    }
}

impl Eq for Stdio {}

impl Stdio {
    /// Die Form für `std::process::Command`; ein Deskriptor wird dupliziert.
    pub(crate) fn to_std(&self) -> std::io::Result<std::process::Stdio> {
        Ok(match self {
            Stdio::Piped => std::process::Stdio::piped(),
            Stdio::Null => std::process::Stdio::null(),
            Stdio::Inherit => std::process::Stdio::inherit(),
            #[cfg(unix)]
            Stdio::Fd(fd) => std::process::Stdio::from(fd.try_clone()?),
        })
    }
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn stdio_compares_by_kind_and_descriptor() {
        assert_eq!(Stdio::Piped, Stdio::Piped);
        assert_ne!(Stdio::Piped, Stdio::Null);
        assert_ne!(Stdio::Inherit, Stdio::Null);
        for s in [Stdio::Piped, Stdio::Null, Stdio::Inherit] {
            s.to_std().unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn sbx_002_passed_descriptor_is_duplicated_for_the_child() {
        let file = std::fs::File::open("/dev/null").unwrap();
        let fd = std::sync::Arc::new(std::os::fd::OwnedFd::from(file));
        let a = Stdio::Fd(fd.clone());
        assert_eq!(a, Stdio::Fd(fd.clone()));
        let other = std::sync::Arc::new(std::os::fd::OwnedFd::from(
            std::fs::File::open("/dev/null").unwrap(),
        ));
        assert_ne!(a, Stdio::Fd(other));
        assert_ne!(a, Stdio::Null);
        a.to_std().unwrap();
        let spec = CommandSpec::new("/bin/true")
            .arg("x")
            .env("A", "1")
            .cwd("/")
            .stdio(a.clone(), Stdio::Null, Stdio::Inherit);
        assert_eq!(spec.stdin, a);
        assert_eq!(spec.env.set, [("A".to_owned(), "1".to_owned())]);
    }
}
