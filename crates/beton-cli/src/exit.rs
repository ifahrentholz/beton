//! Stabile Exit-Codes (CLI-001).

use std::fmt;

/// Exit-Codes des Binaries `beton`. Kommandospezifische Codes stehen beim jeweiligen Feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Exit {
    Ok = 0,
    General = 1,
    Usage = 2,
    PolicyDeny = 3,
    ApprovalDenied = 4,
    BudgetExhausted = 5,
    Harness = 6,
    Unreachable = 7,
    Interrupted = 130,
}

impl From<Exit> for std::process::ExitCode {
    fn from(e: Exit) -> Self {
        std::process::ExitCode::from(e as u8)
    }
}

/// Fehler eines Kommandos mit zugehörigem Exit-Code.
#[derive(Debug)]
pub struct CliError {
    pub exit: Exit,
    pub error: anyhow::Error,
}

impl CliError {
    pub fn new(exit: Exit, error: impl Into<anyhow::Error>) -> Self {
        Self {
            exit,
            error: error.into(),
        }
    }

    pub fn usage(msg: impl fmt::Display) -> Self {
        Self::new(Exit::Usage, anyhow::anyhow!("{msg}"))
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#}", self.error)
    }
}

impl From<anyhow::Error> for CliError {
    fn from(error: anyhow::Error) -> Self {
        Self {
            exit: Exit::General,
            error,
        }
    }
}

impl From<beton_sdk::Error> for CliError {
    fn from(e: beton_sdk::Error) -> Self {
        let exit = match &e {
            beton_sdk::Error::Unreachable { .. } => Exit::Unreachable,
            _ => Exit::General,
        };
        Self::new(exit, e)
    }
}

pub type CliResult<T = ()> = Result<T, CliError>;
