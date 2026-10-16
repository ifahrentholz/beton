//! Ein Pseudo-Terminal in Stufe 0.

use std::io::{Read, Write};
use std::sync::Mutex;

use portable_pty::{MasterPty, PtySize, native_pty_system};

/// Fehler beim Öffnen oder Bedienen eines PTY.
#[derive(Debug)]
pub struct PtyError(pub String);

impl std::fmt::Display for PtyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PTY: {}", self.0)
    }
}

impl std::error::Error for PtyError {}

fn err(e: impl std::fmt::Display) -> PtyError {
    PtyError(e.to_string())
}

/// Kleinste und größte erlaubte Fenstergröße (Zeilen bzw. Spalten).
pub const MIN_SIZE: u16 = 2;
pub const MAX_SIZE: u16 = 1000;

/// Prüft eine Fenstergröße (fail closed bei Unsinn wie 0 × 0 oder 65 535 Spalten).
pub fn valid_size(cols: u16, rows: u16) -> bool {
    (MIN_SIZE..=MAX_SIZE).contains(&cols) && (MIN_SIZE..=MAX_SIZE).contains(&rows)
}

/// Ein offenes Pseudo-Terminal: Master in diesem Prozess, Slave für genau einen Kindprozess.
pub struct Pty {
    master: Mutex<Box<dyn MasterPty>>,
    #[cfg(unix)]
    slave: Option<std::os::fd::OwnedFd>,
    #[cfg(unix)]
    slave_path: std::path::PathBuf,
}

impl std::fmt::Debug for Pty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("Pty");
        #[cfg(unix)]
        d.field("slave", &self.slave_path);
        d.finish_non_exhaustive()
    }
}

impl Pty {
    /// Öffnet ein PTY mit `cols` × `rows`.
    pub fn open(cols: u16, rows: u16) -> Result<Self, PtyError> {
        if !valid_size(cols, rows) {
            return Err(PtyError(format!("ungültige Größe {cols} × {rows}")));
        }
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(err)?;
        #[cfg(unix)]
        {
            // Der Slave-Deskriptor von `portable-pty` ist nicht zugänglich; beton öffnet den
            // Slave selbst (ohne ihn zum Controlling Terminal dieses Prozesses zu machen) und
            // schließt den von `portable-pty` mit `pair.slave`.
            let slave_path = pair
                .master
                .tty_name()
                .ok_or_else(|| PtyError("Pfad des PTY-Slaves unbekannt".into()))?;
            let slave = open_slave(&slave_path)?;
            drop(pair.slave);
            Ok(Self {
                master: Mutex::new(pair.master),
                slave: Some(slave),
                slave_path,
            })
        }
        #[cfg(not(unix))]
        {
            // ConPTY: Prozesse am Pseudo-Terminal startet erst das Windows-Backend der Sandbox
            // (SBX-016, M5); bis dahin gibt es dort keine Terminals (fail closed).
            drop(pair);
            Err(PtyError(
                "Terminals unter Windows folgen mit dem Sandbox-Backend (SBX-016)".into(),
            ))
        }
    }

    /// Nimmt den Slave-Deskriptor für den Kindprozess (genau einmal). Nach dem Start muss der
    /// Aufrufer seine Kopie schließen, damit das Lesen am Master mit dem Ende des Kindes endet.
    #[cfg(unix)]
    pub fn take_slave(&mut self) -> Option<std::os::fd::OwnedFd> {
        self.slave.take()
    }

    /// Pfad des Slaves (z. B. `/dev/ttys004`); die Seatbelt-Regel erlaubt `ioctl` nur darauf.
    #[cfg(unix)]
    pub fn slave_path(&self) -> &std::path::Path {
        &self.slave_path
    }

    /// Leser für die Ausgabe des Kindes (blockierend).
    pub fn reader(&self) -> Result<Box<dyn Read + Send>, PtyError> {
        self.lock().try_clone_reader().map_err(err)
    }

    /// Schreiber für Eingaben an das Kind (blockierend, genau einmal).
    pub fn writer(&self) -> Result<Box<dyn Write + Send>, PtyError> {
        self.lock().take_writer().map_err(err)
    }

    /// Neue Fenstergröße; der Kernel schickt `SIGWINCH` an die Vordergrund-Prozessgruppe.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        if !valid_size(cols, rows) {
            return Err(PtyError(format!("ungültige Größe {cols} × {rows}")));
        }
        self.lock()
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(err)
    }

    /// Aktuelle Größe laut Kernel `(cols, rows)`.
    pub fn size(&self) -> Result<(u16, u16), PtyError> {
        let s = self.lock().get_size().map_err(err)?;
        Ok((s.cols, s.rows))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Box<dyn MasterPty>> {
        self.master
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(unix)]
fn open_slave(path: &std::path::Path) -> Result<std::os::fd::OwnedFd, PtyError> {
    use std::os::unix::fs::OpenOptionsExt as _;
    // O_NOCTTY: Der Slave wird nicht zum Controlling Terminal des Runners.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| PtyError(format!("{}: {e}", path.display())))?;
    Ok(std::os::fd::OwnedFd::from(file))
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn ses_019_pty_opens_with_size_and_a_slave_for_the_child() {
        let mut pty = Pty::open(100, 30).unwrap();
        assert_eq!(pty.size().unwrap(), (100, 30));
        assert!(pty.slave_path().starts_with("/dev"));
        assert!(pty.take_slave().is_some());
        assert!(pty.take_slave().is_none(), "nur einmal");
        pty.resize(80, 24).unwrap();
        assert_eq!(pty.size().unwrap(), (80, 24));
    }

    #[test]
    fn ses_019_invalid_sizes_are_rejected() {
        assert!(Pty::open(0, 0).is_err());
        assert!(Pty::open(5000, 10).is_err());
        let pty = Pty::open(80, 24).unwrap();
        assert!(pty.resize(1, 24).is_err());
        assert_eq!(pty.size().unwrap(), (80, 24));
    }

    #[test]
    fn ses_019_bytes_written_to_the_slave_arrive_at_the_master() {
        let mut pty = Pty::open(80, 24).unwrap();
        let slave = pty.take_slave().unwrap();
        let mut reader = pty.reader().unwrap();
        let mut file = std::fs::File::from(slave);
        file.write_all(b"hallo").unwrap();
        let mut buf = [0u8; 16];
        let n = reader.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hallo");
    }
}
