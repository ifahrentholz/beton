//! Protokoll zwischen `bt-exec` und Broker: Rahmen aus 4 Byte Länge (Big Endian) und JSON;
//! zuerst ein Byte mit den drei stdio-Deskriptoren (`SCM_RIGHTS`), dann die Anfrage.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Version des Protokolls.
pub const VERSION: u32 = 1;
/// Höchstgröße eines Rahmens.
pub const MAX_FRAME: usize = 1024 * 1024;

/// Anfrage von `bt-exec`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub v: u32,
    pub token: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    #[serde(default)]
    pub env_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub tty: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("argv0", &self.argv.first())
            .field("cwd", &self.cwd)
            .field("env", &self.env_overrides.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// Antwort des Brokers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Started {
        pid: u32,
    },
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    Error {
        code: String,
        message: String,
    },
}

/// Nachricht von `bt-exec` nach dem Start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientMsg {
    Signal { signal: i32 },
}

/// Schreibt einen Rahmen.
pub fn write_frame<W: Write, T: Serialize>(w: &mut W, msg: &T) -> std::io::Result<()> {
    let body = serde_json::to_vec(msg).map_err(std::io::Error::other)?;
    if body.len() > MAX_FRAME {
        return Err(std::io::Error::other("Rahmen zu groß"));
    }
    w.write_all(&(body.len() as u32).to_be_bytes())?;
    w.write_all(&body)?;
    w.flush()
}

/// Liest einen Rahmen; `None` am Ende der Verbindung.
pub fn read_frame<R: Read, T: for<'de> Deserialize<'de>>(r: &mut R) -> std::io::Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::other("Rahmen zu groß"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
