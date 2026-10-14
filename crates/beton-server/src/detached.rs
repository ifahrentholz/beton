//! Runner, die einen Neustart des Daemons überdauern (POL-010 AC4).
//!
//! Wartet ein Runner auf eine Freigabe, beendet der Daemon ihn beim Herunterfahren nicht,
//! sondern koppelt ihn ab: Session, SHA-256 seines Tokens und die Frist der offenen Freigaben
//! landen in `run/runners-detached.json` (0600). Der neue Daemon liest die Datei genau einmal,
//! löscht sie und nimmt die Tokens bis zur Frist an; der Runner verbindet sich neu und die
//! Freigabe bleibt entscheidbar. Das Token selbst steht nie auf der Platte.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use beton_core::id::SessionId;
use serde::{Deserialize, Serialize};

/// Dateiname neben dem Tunnel-Socket.
pub const FILE: &str = "runners-detached.json";

/// Ein abgekoppelter Runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachedRunner {
    pub session_id: SessionId,
    /// SHA-256 des Runner-Tokens, hex.
    pub token_sha256: String,
    /// Bis wann der Runner zurückkommen darf (Unix-Sekunden, Ablauf der letzten Freigabe).
    pub until: i64,
}

/// Pfad der Datei zum Tunnel-Socket.
pub fn path_for(tunnel_socket: &Path) -> Option<PathBuf> {
    tunnel_socket.parent().map(|d| d.join(FILE))
}

/// Schreibt die abgekoppelten Runner (0600, atomar); ohne Einträge wird die Datei entfernt.
pub fn save(path: &Path, runners: &[DetachedRunner]) -> std::io::Result<()> {
    if runners.is_empty() {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    let tmp = path.with_extension("json.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(&serde_json::to_vec(runners).map_err(std::io::Error::other)?)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

/// Liest die Datei einmal und löscht sie; abgelaufene Einträge und eine unlesbare Datei
/// ergeben nichts (fail closed).
pub fn take(path: &Path) -> Vec<DetachedRunner> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let _ = std::fs::remove_file(path);
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    match serde_json::from_slice::<Vec<DetachedRunner>>(&bytes) {
        Ok(list) => list.into_iter().filter(|d| d.until > now).collect(),
        Err(e) => {
            tracing::warn!("Abgekoppelte Runner nicht lesbar: {e}");
            Vec::new()
        }
    }
}
