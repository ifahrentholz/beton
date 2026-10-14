//! Dünne Wrapper um libc für den Launcher. Jeder Aufruf liefert `io::Result`; Fehler tragen den
//! Schritt, in dem sie auftraten (für die Meldung an den Runner).

use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

/// Fehler eines Startschritts.
#[derive(Debug)]
pub struct StepError {
    pub step: &'static str,
    pub message: String,
    pub errno: Option<i32>,
}

impl StepError {
    pub fn new(step: &'static str, message: impl Into<String>) -> Self {
        Self {
            step,
            message: message.into(),
            errno: None,
        }
    }

    pub fn io(step: &'static str, what: impl std::fmt::Display, e: io::Error) -> Self {
        Self {
            step,
            message: format!("{what}: {e}"),
            errno: e.raw_os_error(),
        }
    }
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.step, self.message)
    }
}

pub type StepResult<T> = Result<T, StepError>;

/// Rückgabewert eines libc-Aufrufs als `io::Result`.
pub fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

pub fn cvt_long(r: libc::c_long) -> io::Result<libc::c_long> {
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

pub fn cstr(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Pfad mit NUL-Byte"))
}

pub fn close(fd: RawFd) {
    // SAFETY: Schließt einen Deskriptor, den der Aufrufer besitzt.
    unsafe {
        libc::close(fd);
    }
}

/// Eine Pipe mit `O_CLOEXEC` (lesen, schreiben).
pub fn pipe() -> io::Result<(RawFd, RawFd)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` hat Platz für zwei Deskriptoren.
    cvt(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) })?;
    Ok((fds[0], fds[1]))
}

/// Ein `SOCK_SEQPACKET`-Paar mit `SOCK_CLOEXEC`.
pub fn seqpacket_pair() -> io::Result<(RawFd, RawFd)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` hat Platz für zwei Deskriptoren.
    cvt(unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    })?;
    Ok((fds[0], fds[1]))
}

/// Legt `fd` auf die feste Nummer `target` (ohne `O_CLOEXEC`). Nur async-signal-sichere
/// Aufrufe: auch nach `fork` vor `execve` nutzbar.
pub fn move_fd(fd: RawFd, target: RawFd) -> io::Result<()> {
    if fd == target {
        // SAFETY: Löscht nur das Flag FD_CLOEXEC eines gültigen Deskriptors.
        cvt(unsafe { libc::fcntl(fd, libc::F_SETFD, 0) })?;
    } else {
        // SAFETY: dup2 auf eine feste Nummer; ein dort offener Deskriptor wird ersetzt.
        cvt(unsafe { libc::dup2(fd, target) })?;
    }
    Ok(())
}

/// Dupliziert `fd` auf eine Nummer ≥ `min` mit `O_CLOEXEC`.
pub fn dup_above(fd: RawFd, min: RawFd) -> io::Result<RawFd> {
    // SAFETY: F_DUPFD_CLOEXEC auf einem gültigen Deskriptor.
    cvt(unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, min) })
}

pub fn set_cloexec(fd: RawFd) -> io::Result<()> {
    // SAFETY: setzt FD_CLOEXEC auf einem gültigen Deskriptor.
    cvt(unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) })?;
    Ok(())
}

/// Markiert alle Deskriptoren ab `from` mit `O_CLOEXEC` (`close_range`, Kernel ≥ 5.11), sonst
/// über `/proc/self/fd`.
pub fn cloexec_from(from: RawFd) -> io::Result<()> {
    const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
    // SAFETY: close_range mit Flag CLOEXEC schließt nichts, es setzt nur Flags.
    let r = unsafe {
        libc::syscall(
            libc::SYS_close_range,
            from as libc::c_uint,
            libc::c_uint::MAX,
            CLOSE_RANGE_CLOEXEC,
        )
    };
    if r == 0 {
        return Ok(());
    }
    for e in std::fs::read_dir("/proc/self/fd")?.flatten() {
        if let Some(fd) = e.file_name().to_str().and_then(|s| s.parse::<RawFd>().ok())
            && fd >= from
        {
            let _ = set_cloexec(fd);
        }
    }
    Ok(())
}

/// Schließt alle Deskriptoren außer 0–2 und `keep`.
pub fn close_all_except(keep: &[RawFd]) {
    let fds: Vec<RawFd> = std::fs::read_dir("/proc/self/fd")
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.file_name().to_str().and_then(|s| s.parse().ok()))
                .collect()
        })
        .unwrap_or_default();
    for fd in fds {
        if fd > 2 && !keep.contains(&fd) {
            close(fd);
        }
    }
}

/// Lenkt stdin, stdout und stderr auf `/dev/null` (Hilfsprozesse halten keine Pipes des
/// Runners offen).
pub fn stdio_to_null() {
    // SAFETY: open/dup2/close auf eigenen Deskriptoren.
    unsafe {
        let fd = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC);
        if fd >= 0 {
            for t in 0..3 {
                libc::dup2(fd, t);
            }
            libc::close(fd);
        }
    }
}

/// `fork(2)`. Der Aufrufer muss einthreadig sein (Launcher) oder im Kind nur
/// async-signal-sichere Funktionen nutzen.
pub fn fork() -> io::Result<libc::pid_t> {
    // SAFETY: Der Launcher ist einthreadig; das Kind läuft mit einer Kopie des Speichers weiter.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(pid)
    }
}

pub fn set_pdeathsig(sig: libc::c_int) -> io::Result<()> {
    // SAFETY: prctl mit einfachen Ganzzahl-Argumenten.
    cvt(unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, sig as libc::c_ulong, 0, 0, 0) })?;
    Ok(())
}

/// Nicht „dumpable“: `/proc/<pid>/mem` und ptrace verlangen dann `CAP_SYS_PTRACE`. Die
/// Hilfsprozesse der Sandbox (Launcher, Relay, Connector) sind so auch dort nicht von außen
/// beschreibbar, wo Yama ptrace nicht einschränkt (z. B. Debian, `ptrace_scope=0`). `execve`
/// setzt das für das Ziel zurück.
pub fn set_dumpable(yes: bool) -> io::Result<()> {
    // SAFETY: prctl mit einfachen Ganzzahl-Argumenten.
    cvt(unsafe { libc::prctl(libc::PR_SET_DUMPABLE, libc::c_ulong::from(yes), 0, 0, 0) })?;
    Ok(())
}

pub fn set_no_new_privs() -> io::Result<()> {
    // SAFETY: prctl mit einfachen Ganzzahl-Argumenten.
    cvt(unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) })?;
    Ok(())
}

/// Setzt die Disposition von `sigs` (`SIG_IGN` bzw. `SIG_DFL`).
pub fn signals(sigs: &[libc::c_int], disposition: libc::sighandler_t) {
    for &s in sigs {
        // SAFETY: signal(2) mit SIG_IGN/SIG_DFL; kein eigener Handler.
        unsafe {
            libc::signal(s, disposition);
        }
    }
}

/// Signale, die Hilfsprozesse ignorieren: Ctrl+C an die Prozessgruppe gilt dem Ziel.
pub const IGNORED: [libc::c_int; 8] = [
    libc::SIGINT,
    libc::SIGTERM,
    libc::SIGHUP,
    libc::SIGQUIT,
    libc::SIGTSTP,
    libc::SIGTTIN,
    libc::SIGTTOU,
    libc::SIGPIPE,
];

/// Setzt alle Signale auf die Voreinstellung zurück und entsperrt sie (vor `execve`; ignorierte
/// Signale würden sonst vererbt).
pub fn reset_signals() {
    for s in 1..=64 {
        if s == libc::SIGKILL || s == libc::SIGSTOP {
            continue;
        }
        // SAFETY: Setzt nur die Disposition zurück.
        unsafe {
            libc::signal(s, libc::SIG_DFL);
        }
    }
    // SAFETY: Leere Signalmaske setzen.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
    }
}

/// Blockiert `SIGCHLD` und liefert ein `signalfd` dafür.
pub fn sigchld_fd() -> io::Result<RawFd> {
    // SAFETY: Signalmaske und signalfd mit gültigem sigset.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGCHLD);
        cvt(libc::sigprocmask(
            libc::SIG_BLOCK,
            &set,
            std::ptr::null_mut(),
        ))?;
        cvt(libc::signalfd(
            -1,
            &set,
            libc::SFD_CLOEXEC | libc::SFD_NONBLOCK,
        ))
    }
}

/// Leert ein `signalfd` (nicht blockierend).
pub fn drain_signalfd(fd: RawFd) {
    let mut buf = [0u8; 128];
    loop {
        // SAFETY: Liest in einen eigenen Puffer.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            break;
        }
    }
}

/// Wartet auf Lesbarkeit/Schließen von `fds` höchstens `timeout_ms`; liefert `revents` je FD.
pub fn poll(fds: &[RawFd], timeout_ms: libc::c_int) -> io::Result<Vec<libc::c_short>> {
    let mut p: Vec<libc::pollfd> = fds
        .iter()
        .map(|&fd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    loop {
        // SAFETY: `p` ist ein gültiges Array von pollfd.
        let r = unsafe { libc::poll(p.as_mut_ptr(), p.len() as libc::nfds_t, timeout_ms) };
        if r < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        return Ok(p.iter().map(|x| x.revents).collect());
    }
}

/// Exit-Status eines Kindes (`code`, `signal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Exited(i32),
    Signaled(i32),
}

impl Status {
    pub fn encode(self) -> [u8; 8] {
        let (kind, v) = match self {
            Status::Exited(c) => (0i32, c),
            Status::Signaled(s) => (1i32, s),
        };
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&kind.to_le_bytes());
        b[4..].copy_from_slice(&v.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != 8 {
            return None;
        }
        let kind = i32::from_le_bytes(b[..4].try_into().ok()?);
        let v = i32::from_le_bytes(b[4..].try_into().ok()?);
        Some(if kind == 0 {
            Status::Exited(v)
        } else {
            Status::Signaled(v)
        })
    }
}

/// `waitpid`; `None`, wenn (bei `WNOHANG`) noch nichts beendet ist.
pub fn waitpid(pid: libc::pid_t, nohang: bool) -> io::Result<Option<(libc::pid_t, Status)>> {
    let mut status = 0;
    let flags = if nohang { libc::WNOHANG } else { 0 };
    loop {
        // SAFETY: `status` ist ein gültiger Zeiger.
        let r = unsafe { libc::waitpid(pid, &mut status, flags) };
        if r < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if r == 0 {
            return Ok(None);
        }
        let s = if libc::WIFSIGNALED(status) {
            Status::Signaled(libc::WTERMSIG(status))
        } else if libc::WIFEXITED(status) {
            Status::Exited(libc::WEXITSTATUS(status))
        } else {
            continue;
        };
        return Ok(Some((r, s)));
    }
}

pub fn kill(pid: libc::pid_t, sig: libc::c_int) {
    // SAFETY: kill(2) auf einen eigenen Kindprozess.
    unsafe {
        libc::kill(pid, sig);
    }
}

/// Schreibt alles (für Pipes und Dateien).
pub fn write_all(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        // SAFETY: Schreibt aus einem gültigen Puffer.
        let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        buf = &buf[n as usize..];
    }
    Ok(())
}

/// Liest bis EOF (höchstens `max` Bytes).
pub fn read_to_end(fd: RawFd, max: usize) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        // SAFETY: Liest in einen eigenen Puffer.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..n as usize]);
        if out.len() > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("mehr als {max} Bytes"),
            ));
        }
    }
}

/// Kernel-Version (`uname -r`).
pub fn kernel_release() -> String {
    // SAFETY: uname füllt die übergebene Struktur.
    unsafe {
        let mut u: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut u) != 0 {
            return "unbekannt".into();
        }
        CStr::from_ptr(u.release.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn sbx_008_status_roundtrip_and_pipes() {
        for st in [Status::Exited(0), Status::Exited(125), Status::Signaled(9)] {
            assert_eq!(Status::decode(&st.encode()), Some(st));
        }
        assert_eq!(Status::decode(&[1, 2]), None);
        let (r, w) = pipe().unwrap();
        write_all(w, b"hallo").unwrap();
        close(w);
        assert_eq!(poll(&[r], 1000).unwrap()[0] & libc::POLLIN, libc::POLLIN);
        assert_eq!(read_to_end(r, 100).unwrap(), b"hallo");
        close(r);
        let (r, w) = pipe().unwrap();
        write_all(w, &[0u8; 64]).unwrap();
        close(w);
        assert!(read_to_end(r, 10).is_err());
        let high = dup_above(r, 100).unwrap();
        assert!(high >= 100);
        set_cloexec(high).unwrap();
        close(high);
        close(r);
        assert!(!kernel_release().is_empty());
        let e = StepError::io("mount", "/x", io::Error::from_raw_os_error(libc::ENOENT));
        assert_eq!(e.errno, Some(libc::ENOENT));
        assert!(e.to_string().starts_with("mount: /x"));
    }

    #[test]
    fn sbx_008_waitpid_reports_exit_and_signal() {
        let child = |code: i32| {
            let pid = fork().unwrap();
            if pid == 0 {
                // SAFETY: Im Kind nur _exit.
                unsafe { libc::_exit(code) };
            }
            pid
        };
        let pid = child(7);
        assert_eq!(waitpid(pid, false).unwrap(), Some((pid, Status::Exited(7))));
        let pid = fork().unwrap();
        if pid == 0 {
            // SAFETY: Im Kind nur pause/_exit.
            unsafe {
                libc::pause();
                libc::_exit(0);
            }
        }
        kill(pid, libc::SIGKILL);
        assert_eq!(
            waitpid(pid, false).unwrap(),
            Some((pid, Status::Signaled(libc::SIGKILL)))
        );
    }
}
