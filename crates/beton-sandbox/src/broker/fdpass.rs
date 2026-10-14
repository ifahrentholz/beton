//! Übergabe der stdio-Deskriptoren per `SCM_RIGHTS` (macOS und Linux).

use std::io;
use std::os::fd::{FromRawFd as _, OwnedFd, RawFd};

/// Anzahl der übergebenen Deskriptoren (stdin, stdout, stderr).
pub(crate) const STDIO: usize = 3;

/// Sendet ein Byte mit den Deskriptoren `fds`.
#[allow(unsafe_code, reason = "sendmsg mit SCM_RIGHTS gibt es nur über libc")]
pub(crate) fn send_fds(sock: RawFd, fds: &[RawFd; STDIO]) -> io::Result<()> {
    let mut data = *b"S";
    let mut iov = libc::iovec {
        iov_base: data.as_mut_ptr().cast(),
        iov_len: 1,
    };
    let payload = std::mem::size_of_val(fds);
    // SAFETY: msghdr mit gültigem iovec und Kontrollpuffer der berechneten Größe; die
    // Deskriptoren werden nur kopiert.
    unsafe {
        let space = libc::CMSG_SPACE(payload as _) as usize;
        let mut cbuf = vec![0u8; space];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        if cmsg.is_null() {
            return Err(io::Error::other("kein Platz für SCM_RIGHTS"));
        }
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(payload as _) as _;
        std::ptr::copy_nonoverlapping(fds.as_ptr().cast::<u8>(), libc::CMSG_DATA(cmsg), payload);
        loop {
            let n = libc::sendmsg(sock, &msg, 0);
            if n >= 0 {
                return Ok(());
            }
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
    }
}

/// Empfängt das Byte mit genau drei Deskriptoren; alles andere ist ein Fehler (fail closed).
/// `WouldBlock` reicht der Aufrufer an die Laufzeit zurück.
#[allow(unsafe_code, reason = "recvmsg mit SCM_RIGHTS gibt es nur über libc")]
pub(crate) fn recv_fds(sock: RawFd) -> io::Result<Vec<OwnedFd>> {
    let mut data = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: data.as_mut_ptr().cast(),
        iov_len: 1,
    };
    // Platz für mehr als erwartet, damit überzählige Deskriptoren erkannt und geschlossen
    // werden statt still zu verschwinden.
    let payload = std::mem::size_of::<RawFd>() * (STDIO + 8);
    #[cfg(target_os = "linux")]
    let flags = libc::MSG_CMSG_CLOEXEC;
    #[cfg(not(target_os = "linux"))]
    let flags = 0;
    // SAFETY: msghdr mit gültigem iovec und Kontrollpuffer; empfangene Deskriptoren werden
    // sofort in OwnedFd übernommen.
    unsafe {
        let space = libc::CMSG_SPACE(payload as _) as usize;
        let mut cbuf = vec![0u8; space];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        let n = libc::recvmsg(sock, &mut msg, flags);
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut fds = Vec::new();
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let len = (*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                let count = len / std::mem::size_of::<RawFd>();
                let data = libc::CMSG_DATA(cmsg).cast::<RawFd>();
                for i in 0..count {
                    let fd = std::ptr::read_unaligned(data.add(i));
                    #[cfg(not(target_os = "linux"))]
                    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                    fds.push(OwnedFd::from_raw_fd(fd));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Verbindung beendet",
            ));
        }
        if msg.msg_flags & libc::MSG_CTRUNC != 0 || fds.len() != STDIO || data[0] != b'S' {
            // `fds` werden beim Verlassen geschlossen.
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "erwartet werden genau drei Deskriptoren (stdin, stdout, stderr)",
            ));
        }
        Ok(fds)
    }
}
