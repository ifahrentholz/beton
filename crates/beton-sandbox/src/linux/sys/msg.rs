//! Status-Kanal zwischen Launcher und Runner: ein `SOCK_SEQPACKET`-Socket (Nachrichtengrenzen
//! bleiben erhalten, mehrere Schreiber – L und P1 – mischen sich nicht). Der Runner liest; der
//! Launcher schreibt. Das Ende des Sockets auf Runner-Seite bedeutet „Runner tot“ (L beendet
//! dann die Sandbox).

use std::io;
use std::os::fd::RawFd;

use serde::{Deserialize, Serialize};

use super::util::cvt;

/// Plan vom Runner (Pipe).
pub const PLAN_FD: RawFd = 3;
/// Status-Socket zum Runner.
pub const CTL_FD: RawFd = 4;
/// Connector zum Proxy (nur im Launcher innerhalb von bwrap).
pub const CONNECTOR_FD: RawFd = 5;
/// Endstatus des Ziels (nur im Launcher innerhalb von bwrap).
pub const STATUS_FD: RawFd = 6;

/// Höchstgröße des Plans.
pub const MAX_PLAN: usize = 4 * 1024 * 1024;

/// Eine Nachricht des Launchers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Ctl {
    /// Das Ziel läuft (`execve` gelang).
    Ready,
    /// Der Start scheiterte vor dem Ziel; nichts läuft.
    Failed {
        step: String,
        message: String,
        #[serde(default)]
        errno: Option<i32>,
    },
    /// Ein Limit wurde erreicht (`pids`, `memory`).
    Limit { target: String },
    /// In der Sandbox entstand eine Git-Steuerdatei (`commondir`/`gitdir`, #209); der
    /// Launcher hat die Sandbox beendet und die Datei entfernt.
    Fs { target: String },
}

/// Sendet eine Nachricht (best effort, Nachrichten sind klein).
pub fn send(fd: RawFd, msg: &Ctl) -> io::Result<()> {
    let data = serde_json::to_vec(msg).map_err(io::Error::other)?;
    // SAFETY: send aus einem gültigen Puffer; MSG_NOSIGNAL verhindert SIGPIPE.
    cvt(unsafe { libc::send(fd, data.as_ptr().cast(), data.len(), libc::MSG_NOSIGNAL) } as i32)?;
    Ok(())
}

/// Empfängt eine Nachricht; `None` am Ende (Gegenseite geschlossen).
pub fn recv(fd: RawFd) -> io::Result<Option<Ctl>> {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        // SAFETY: recv in einen eigenen Puffer.
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            return Ok(None);
        }
        let msg = serde_json::from_slice(&buf[..n as usize]).map_err(io::Error::other)?;
        return Ok(Some(msg));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn sbx_008_messages_keep_their_boundaries() {
        let (a, b) = super::super::util::seqpacket_pair().unwrap();
        send(a, &Ctl::Ready).unwrap();
        send(
            a,
            &Ctl::Failed {
                step: "mount".into(),
                message: "x".into(),
                errno: Some(2),
            },
        )
        .unwrap();
        send(
            a,
            &Ctl::Limit {
                target: "pids".into(),
            },
        )
        .unwrap();
        super::super::util::close(a);
        assert_eq!(recv(b).unwrap(), Some(Ctl::Ready));
        assert!(matches!(
            recv(b).unwrap(),
            Some(Ctl::Failed { errno: Some(2), .. })
        ));
        assert_eq!(
            recv(b).unwrap(),
            Some(Ctl::Limit {
                target: "pids".into()
            })
        );
        assert_eq!(recv(b).unwrap(), None);
        super::super::util::close(b);
    }
}
