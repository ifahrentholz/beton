//! `bt-exec`: Client des Exec-Brokers in Stufe 1 (SBX-002). Synchron und ohne Laufzeit, damit
//! er als Shell-Ersatz schnell startet.

use std::ffi::OsString;
use std::path::Path;

use super::proto::{Reply, Request};

/// Schickt `req` mit den Deskriptoren `stdio` und liefert die erste Antwort (Tests,
/// Diagnose).
#[cfg(unix)]
pub fn request(
    socket: &Path,
    req: &Request,
    stdio: [std::os::fd::RawFd; 3],
) -> std::io::Result<Reply> {
    let _ = (socket, req, stdio);
    Err(std::io::Error::other("noch nicht umgesetzt"))
}

/// Führt `bt-exec` aus: `argv0` bestimmt die Shell (`bt-exec-zsh` …) bzw. `args[0]` das
/// Programm. Liefert den Exit-Code; endet das Kommando durch ein Signal, beendet sich
/// `bt-exec` mit demselben Signal.
pub fn run(argv0: &OsString, args: Vec<OsString>) -> i32 {
    let _ = (argv0, args);
    eprintln!("bt-exec: noch nicht umgesetzt");
    126
}
