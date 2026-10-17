//! Prozess-Abstammung (#247): Ist ein Prozess ein Nachfahre eines anderen? Grundlage der
//! Peer-Prüfung am Hook-Socket. Linux liest `/proc/<pid>/stat`, macOS `proc_pidinfo`; ohne
//! Auskunft gilt ein Prozess nie als Nachfahre (fail closed).
#![cfg_attr(
    target_os = "macos",
    allow(unsafe_code, reason = "Eltern-PID gibt es unter macOS nur über libc")
)]

/// Höchstens so viele Ebenen nach oben (Schutz gegen Zyklen durch PID-Wiederverwendung).
const MAX_DEPTH: usize = 512;

/// Ist `pid` gleich `root` oder ein Nachfahre davon?
pub fn is_descendant(pid: u32, root: u32) -> bool {
    let mut current = pid;
    for _ in 0..MAX_DEPTH {
        if current == root {
            return true;
        }
        match parent_of(current) {
            Some(parent) if parent != current && parent != 0 => current = parent,
            _ => return false,
        }
    }
    false
}

/// Eltern-PID von `pid`; `None`, wenn der Prozess nicht (mehr) existiert oder nicht lesbar ist.
#[cfg(target_os = "linux")]
pub fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `pid (comm) state ppid …`; `comm` kann Leerzeichen und Klammern enthalten.
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    let _state = fields.next()?;
    fields.next()?.parse().ok()
}

/// Eltern-PID von `pid`; `None`, wenn der Prozess nicht (mehr) existiert oder nicht lesbar ist.
#[cfg(target_os = "macos")]
pub fn parent_of(pid: u32) -> Option<u32> {
    let pid = libc::pid_t::try_from(pid).ok()?;
    // SAFETY: proc_pidinfo schreibt höchstens `size` Bytes in die eigene, genullte Struktur.
    unsafe {
        let mut info: libc::proc_bsdinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let r = libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        );
        (r == size).then_some(info.pbi_ppid)
    }
}

/// Andere Plattformen: keine Auskunft.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn parent_of(_pid: u32) -> Option<u32> {
    None
}
