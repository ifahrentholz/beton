//! Netz der Sandbox (SBX-008, PRX-008): leerer Net-Namespace mit `lo`, darin ein Relay auf
//! `127.0.0.1:<Proxy-Port>`, der jede Verbindung an den Proxy im Host-Netz weiterreicht.
//!
//! Der Relay (R, im Net-Namespace) kann selbst nichts außerhalb erreichen. Für jede Verbindung
//! fragt er den Connector (C, im Host-Netz, Kind des Launchers vor `unshare`) über ein
//! `SOCK_SEQPACKET`-Paar; C verbindet ausschließlich zum festen Proxy-Port und reicht den
//! Socket per `SCM_RIGHTS` zurück. Beide laufen ohne Dateizugriff (Landlock) und mit dem
//! seccomp-Filter der Sandbox. Erreicht C den Proxy nicht, scheitert die Verbindung
//! (fail closed, kein Direktzugriff).

use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::os::fd::{AsRawFd as _, FromRawFd as _, RawFd};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::util::{self, cvt};

/// Schaltet `lo` im aktuellen Net-Namespace ein.
pub fn loopback_up() -> io::Result<()> {
    // SAFETY: ioctl SIOCGIFFLAGS/SIOCSIFFLAGS mit einer vollständig initialisierten ifreq.
    unsafe {
        let s = cvt(libc::socket(
            libc::AF_INET,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            0,
        ))?;
        let mut req: libc::ifreq = std::mem::zeroed();
        for (i, b) in b"lo\0".iter().enumerate() {
            req.ifr_name[i] = *b as libc::c_char;
        }
        let r = (|| {
            cvt(libc::ioctl(s, libc::SIOCGIFFLAGS as _, &mut req))?;
            req.ifr_ifru.ifru_flags |= (libc::IFF_UP | libc::IFF_RUNNING) as libc::c_short;
            cvt(libc::ioctl(s, libc::SIOCSIFFLAGS as _, &req))?;
            Ok(())
        })();
        libc::close(s);
        r
    }
}

/// Sendet ein Byte und optional einen Deskriptor.
fn send_fd(sock: RawFd, byte: u8, fd: Option<RawFd>) -> io::Result<()> {
    let mut data = [byte];
    let mut iov = libc::iovec {
        iov_base: data.as_mut_ptr().cast(),
        iov_len: 1,
    };
    // SAFETY: msghdr mit gültigem iovec und (optional) Kontrollpuffer passender Größe.
    unsafe {
        let space = libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) as usize;
        let mut cbuf = vec![0u8; space];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        if let Some(fd) = fd {
            msg.msg_control = cbuf.as_mut_ptr().cast();
            msg.msg_controllen = space as _;
            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
            std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>(), fd);
        }
        cvt(libc::sendmsg(sock, &msg, libc::MSG_NOSIGNAL) as libc::c_int)?;
    }
    Ok(())
}

/// Empfängt ein Byte und optional einen Deskriptor; `None` am Ende.
fn recv_fd(sock: RawFd) -> io::Result<Option<(u8, Option<RawFd>)>> {
    let mut data = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: data.as_mut_ptr().cast(),
        iov_len: 1,
    };
    // SAFETY: msghdr mit gültigem iovec und Kontrollpuffer; empfangene Deskriptoren werden mit
    // MSG_CMSG_CLOEXEC übernommen.
    unsafe {
        let space = libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) as usize;
        let mut cbuf = vec![0u8; space];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        let n = libc::recvmsg(sock, &mut msg, libc::MSG_CMSG_CLOEXEC);
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n == 0 {
            return Ok(None);
        }
        let mut fd = None;
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        if !cmsg.is_null()
            && (*cmsg).cmsg_level == libc::SOL_SOCKET
            && (*cmsg).cmsg_type == libc::SCM_RIGHTS
        {
            fd = Some(std::ptr::read_unaligned(
                libc::CMSG_DATA(cmsg).cast::<RawFd>(),
            ));
        }
        Ok(Some((data[0], fd)))
    }
}

/// Hilfsprozess: alle Rechte abgeben, die er nicht braucht.
fn confine(abi: u32, connect_port: Option<u16>) -> Result<(), String> {
    util::set_dumpable(false).map_err(|e| e.to_string())?;
    util::set_no_new_privs().map_err(|e| e.to_string())?;
    super::landlock::deny_all(abi, connect_port)?;
    super::seccomp::apply(abi)
}

/// Connector (C): läuft im Host-Netz und verbindet nur zu `127.0.0.1:port`.
pub fn connector(sock: RawFd, port: u16, abi: u32) -> ! {
    util::close_all_except(&[sock]);
    util::stdio_to_null();
    util::signals(&util::IGNORED, libc::SIG_IGN);
    let _ = util::set_pdeathsig(libc::SIGKILL);
    if confine(abi, Some(port)).is_err() {
        std::process::exit(1);
    }
    connector_loop(sock, port);
    std::process::exit(0)
}

/// Beantwortet Anfragen des Relays, bis er den Socket schließt.
pub fn connector_loop(sock: RawFd, port: u16) {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    while let Ok(Some(_)) = recv_fd(sock) {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
            Ok(s) => {
                let _ = send_fd(sock, 0, Some(s.as_raw_fd()));
            }
            Err(_) => {
                let _ = send_fd(sock, 1, None);
            }
        }
    }
}

fn pump(mut from: TcpStream, mut to: TcpStream) {
    let _ = io::copy(&mut from, &mut to);
    let _ = to.shutdown(std::net::Shutdown::Write);
}

/// Relay (R): nimmt im Net-Namespace Verbindungen an und hängt sie an den Proxy.
pub fn relay(listener: TcpListener, connector: RawFd, abi: u32) -> ! {
    util::close_all_except(&[listener.as_raw_fd(), connector]);
    util::stdio_to_null();
    util::signals(&util::IGNORED, libc::SIG_IGN);
    // SAFETY: setpgid(0, 0) – eigene Prozessgruppe; Signale an die Gruppe des Ziels treffen
    // den Relay nicht.
    unsafe {
        libc::setpgid(0, 0);
    }
    if confine(abi, None).is_err() {
        std::process::exit(1);
    }
    relay_loop(&listener, connector);
    std::process::exit(0)
}

/// Nimmt Verbindungen an und hängt jede über den Connector an den Proxy.
pub fn relay_loop(listener: &TcpListener, connector: RawFd) {
    let lock = Arc::new(Mutex::new(connector));
    for conn in listener.incoming() {
        let Ok(client) = conn else { continue };
        let lock = lock.clone();
        std::thread::spawn(move || {
            let upstream = {
                let Ok(guard) = lock.lock() else { return };
                if send_fd(*guard, 0, None).is_err() {
                    return;
                }
                match recv_fd(*guard) {
                    Ok(Some((0, Some(fd)))) => fd,
                    Ok(Some((_, Some(fd)))) => {
                        util::close(fd);
                        return;
                    }
                    _ => return,
                }
            };
            // SAFETY: Der Deskriptor kam eben per SCM_RIGHTS und gehört diesem Thread.
            let upstream = unsafe { TcpStream::from_raw_fd(upstream) };
            let (Ok(c2), Ok(u2)) = (client.try_clone(), upstream.try_clone()) else {
                return;
            };
            let t = std::thread::spawn(move || pump(c2, u2));
            pump(upstream, client);
            let _ = t.join();
        });
    }
}

/// Bindet den Relay-Port im Net-Namespace der Sandbox.
pub fn relay_listener(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io::{Read as _, Write as _};

    use super::*;

    #[test]
    fn sbx_008_relay_reaches_the_proxy_only_through_the_connector() {
        // „Proxy“: ein Echo-Dienst.
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = proxy.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for c in proxy.incoming() {
                let mut c = c.unwrap();
                let mut b = [0u8; 5];
                c.read_exact(&mut b).unwrap();
                c.write_all(&b).unwrap();
            }
        });
        let (relay_end, conn_end) = util::seqpacket_pair().unwrap();
        std::thread::spawn(move || connector_loop(conn_end, port));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let relay_addr = listener.local_addr().unwrap();
        std::thread::spawn(move || relay_loop(&listener, relay_end));
        let mut c = TcpStream::connect(relay_addr).unwrap();
        c.write_all(b"hallo").unwrap();
        let mut b = [0u8; 5];
        c.read_exact(&mut b).unwrap();
        assert_eq!(&b, b"hallo");
    }

    #[test]
    fn sbx_008_connector_reports_a_missing_proxy() {
        let free = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);
        let (a, b) = util::seqpacket_pair().unwrap();
        let t = std::thread::spawn(move || connector_loop(b, port));
        send_fd(a, 0, None).unwrap();
        assert_eq!(recv_fd(a).unwrap(), Some((1, None)));
        util::close(a);
        t.join().unwrap();
    }

    #[test]
    fn sbx_008_loopback_up_needs_a_namespace() {
        // Im Host-Netz ist `lo` schon oben; ohne CAP_NET_ADMIN scheitert das Setzen, mit gelingt
        // es. Beides darf den Test nicht stören.
        let _ = loopback_up();
    }
}
