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

/// Art einer Anfrage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// Ein Kommando in einer frischen Stufe-2-Sandbox starten (`bt-exec`).
    #[default]
    Exec,
    /// Git-Shim der Stufe 1 (SBX-003, #311): Darf `git` jetzt starten? Der Broker vergleicht
    /// die Git-Steuerdateien mit dem Schnappschuss des Runners und startet nichts; Antwort
    /// [`Reply::Allowed`] oder `error{git_control_changed}`.
    GitPreflight,
}

impl Op {
    fn is_exec(&self) -> bool {
        *self == Op::Exec
    }
}

/// Anfrage von `bt-exec` bzw. dem Git-Shim.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub v: u32,
    #[serde(default, skip_serializing_if = "Op::is_exec")]
    pub op: Op,
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
            .field("op", &self.op)
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
    /// Antwort auf [`Op::GitPreflight`]: `git` darf starten.
    Allowed,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> Request {
        Request {
            v: VERSION,
            op: Op::Exec,
            token: "GEHEIM-TOKEN".into(),
            argv: vec!["/bin/sh".into(), "-c".into(), "true".into()],
            cwd: PathBuf::from("/ws"),
            env_overrides: [("LANG".to_owned(), "C".to_owned())].into(),
            tty: false,
            cols: None,
            rows: None,
        }
    }

    #[test]
    fn sbx_002_frames_round_trip_and_end_cleanly() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &req()).unwrap();
        write_frame(&mut buf, &Reply::Started { pid: 7 }).unwrap();
        let mut r = buf.as_slice();
        let got: Option<Request> = read_frame(&mut r).unwrap();
        assert_eq!(got, Some(req()));
        let got: Option<Reply> = read_frame(&mut r).unwrap();
        assert_eq!(got, Some(Reply::Started { pid: 7 }));
        let end: Option<Reply> = read_frame(&mut r).unwrap();
        assert_eq!(end, None);
    }

    #[test]
    fn sbx_002_oversized_truncated_and_invalid_frames_fail_closed() {
        let mut big = (MAX_FRAME as u32 + 1).to_be_bytes().to_vec();
        big.extend_from_slice(b"{}");
        assert!(read_frame::<_, Reply>(&mut big.as_slice()).is_err());
        let mut short = 10u32.to_be_bytes().to_vec();
        short.extend_from_slice(b"{}");
        assert!(read_frame::<_, Reply>(&mut short.as_slice()).is_err());
        let mut bad = 2u32.to_be_bytes().to_vec();
        bad.extend_from_slice(b"[]");
        let err = read_frame::<_, Reply>(&mut bad.as_slice()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        let huge = Reply::Error {
            code: "x".into(),
            message: "y".repeat(MAX_FRAME),
        };
        assert!(write_frame(&mut Vec::new(), &huge).is_err());
    }

    #[test]
    fn sbx_002_request_debug_hides_the_token_and_env_values() {
        let text = format!("{:?}", req());
        assert!(!text.contains("GEHEIM-TOKEN"), "{text}");
        assert!(text.contains("LANG") && !text.contains("\"C\""), "{text}");
        let msg = ClientMsg::Signal { signal: 2 };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"signal":{"signal":2}}"#);
    }
}
