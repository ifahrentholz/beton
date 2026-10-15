//! `bt-exec`: Client des Exec-Brokers in Stufe 1 (SBX-002). Synchron und ohne Laufzeit, damit
//! er als Shell-Ersatz schnell startet.
//!
//! - `bt-exec <programm> [args…]` (bzw. `beton __exec …`): `programm` ohne `/` wird im `PATH`
//!   gesucht.
//! - `bt-exec-zsh`, `bt-exec-bash`, `bt-exec-sh`: die jeweilige Shell (aus `SHELL`, sonst
//!   `/bin/<shell>`) mit allen Argumenten; so ruft Claude Code sein Bash-Tool auf
//!   (`CLAUDE_CODE_SHELL`, verifiziert gegen 2.1.296).
//!
//! `bt-exec` gibt seine drei stdio-Deskriptoren, das Arbeitsverzeichnis und die erlaubten
//! Variablen seines Environments weiter ([`super::override_rejected`]); Proxy-, CA- und
//! Broker-Variablen der Stufe 1 bleiben zurück. Signale (SIGINT, SIGTERM, SIGHUP, SIGQUIT)
//! reicht er an den Prozess in Stufe 2 weiter; endet dieser durch ein Signal, beendet sich
//! `bt-exec` mit demselben Signal, sonst mit dessen Exit-Code. Kann er nichts starten, endet
//! er mit 126 (wie `env`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::proto::{Reply, Request};

/// Exit-Code, wenn nichts gestartet werden konnte.
pub const CANNOT_EXECUTE: i32 = 126;

/// Das Zielprogramm aus Aufrufname und Argumenten.
fn target(argv0: &OsString, mut args: Vec<OsString>) -> Result<Vec<String>, String> {
    let base = Path::new(argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let utf8 = |a: OsString| {
        a.into_string()
            .map_err(|_| "Argument ist kein gültiges UTF-8".to_owned())
    };
    if let Some(shell) = base.strip_prefix("bt-exec-") {
        if !matches!(shell, "zsh" | "bash" | "sh") {
            return Err(format!("unbekannte Shell `{shell}`"));
        }
        let from_env = std::env::var("SHELL")
            .ok()
            .filter(|s| s.starts_with('/'))
            .filter(|s| Path::new(s).file_name().and_then(|n| n.to_str()) == Some(shell));
        let program = from_env
            .or_else(|| {
                [format!("/bin/{shell}"), format!("/usr/bin/{shell}")]
                    .into_iter()
                    .find(|p| Path::new(p).is_file())
            })
            .ok_or_else(|| format!("{shell} nicht gefunden"))?;
        let mut argv = vec![program];
        for a in args {
            argv.push(utf8(a)?);
        }
        return Ok(argv);
    }
    if args.is_empty() {
        return Err("Aufruf: bt-exec <programm> [argumente…]".into());
    }
    let program = utf8(args.remove(0))?;
    let program = if program.contains('/') {
        let p = PathBuf::from(&program);
        let p = if p.is_absolute() {
            p
        } else {
            std::env::current_dir().map_err(|e| e.to_string())?.join(p)
        };
        p.to_string_lossy().into_owned()
    } else {
        std::env::var("PATH")
            .unwrap_or_else(|_| "/usr/bin:/bin".into())
            .split(':')
            .filter(|d| !d.is_empty())
            .map(|d| Path::new(d).join(&program))
            .find(|p| p.is_file())
            .map(|p| p.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{program}: nicht gefunden"))?
    };
    let mut argv = vec![program];
    for a in args {
        argv.push(utf8(a)?);
    }
    Ok(argv)
}

#[cfg(unix)]
mod imp {
    use std::io::Read as _;
    use std::os::fd::{AsRawFd as _, RawFd};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::sync::atomic::{AtomicI32, Ordering};

    use super::super::proto::{ClientMsg, Reply, Request, read_frame, write_frame};
    use super::super::{ENV_SOCKET, ENV_TOKEN, fdpass, override_rejected};
    use super::CANNOT_EXECUTE;

    /// Schreibende Seite der Selbst-Pipe für Signale.
    static SIGNAL_PIPE: AtomicI32 = AtomicI32::new(-1);

    const FORWARD: [i32; 4] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];

    #[allow(
        unsafe_code,
        reason = "Signal-Handler: nur write(2), async-signal-sicher"
    )]
    extern "C" fn on_signal(sig: libc::c_int) {
        let fd = SIGNAL_PIPE.load(Ordering::Relaxed);
        if fd >= 0 {
            let b = sig as u8;
            // SAFETY: write auf eine Pipe ist async-signal-sicher.
            unsafe {
                libc::write(fd, (&b as *const u8).cast(), 1);
            }
        }
    }

    #[allow(unsafe_code, reason = "sigaction gibt es nur über libc")]
    fn handlers(install: bool) {
        for sig in FORWARD {
            // SAFETY: vollständig initialisierte sigaction; der Handler ist async-signal-sicher.
            unsafe {
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_sigaction = if install {
                    on_signal as extern "C" fn(libc::c_int) as usize
                } else {
                    libc::SIG_DFL
                };
                libc::sigemptyset(&mut sa.sa_mask);
                sa.sa_flags = libc::SA_RESTART;
                libc::sigaction(sig, &sa, std::ptr::null_mut());
            }
        }
    }

    pub fn request(
        socket: &Path,
        req: &Request,
        stdio: [RawFd; 3],
    ) -> std::io::Result<(UnixStream, Reply)> {
        let mut conn = UnixStream::connect(socket)?;
        fdpass::send_fds(conn.as_raw_fd(), &stdio)?;
        write_frame(&mut conn, req)?;
        let reply: Option<Reply> = read_frame(&mut conn)?;
        let reply = reply.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Exec-Broker hat ohne Antwort getrennt",
            )
        })?;
        Ok((conn, reply))
    }

    #[allow(unsafe_code, reason = "pipe, poll und raise gibt es nur über libc")]
    pub fn run(argv: Vec<String>) -> i32 {
        let fail = |msg: &str| {
            eprintln!("bt-exec: {msg}");
            CANNOT_EXECUTE
        };
        let (Ok(socket), Ok(token)) = (std::env::var(ENV_SOCKET), std::env::var(ENV_TOKEN)) else {
            return fail("kein Exec-Broker (BETON_EXEC_SOCKET, BETON_EXEC_TOKEN fehlen)");
        };
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => return fail(&format!("Arbeitsverzeichnis: {e}")),
        };
        // Allowlist (#259): Basis-, Werkzeug- und die konfigurierten Namen der Stufe 2.
        let configured: Vec<String> = std::env::var(super::super::ENV_FORWARD)
            .unwrap_or_default()
            .split(',')
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .collect();
        let env_overrides = std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .filter(|(k, _)| override_rejected(k, &configured).is_none())
            .collect();
        let req = Request {
            v: super::super::proto::VERSION,
            token,
            argv,
            cwd,
            env_overrides,
            tty: false,
            cols: None,
            rows: None,
        };
        // Selbst-Pipe vor der Anfrage, damit kein Signal zwischen Start und Handler verloren geht.
        let mut pipe = [0 as libc::c_int; 2];
        // SAFETY: pipe mit gültigem Puffer.
        if unsafe { libc::pipe(pipe.as_mut_ptr()) } != 0 {
            return fail(&format!("pipe: {}", std::io::Error::last_os_error()));
        }
        for fd in pipe {
            // SAFETY: fcntl auf eben erzeugte Deskriptoren.
            unsafe {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
        }
        SIGNAL_PIPE.store(pipe[1], Ordering::Relaxed);
        handlers(true);
        let (mut conn, reply) = match request(Path::new(&socket), &req, [0, 1, 2]) {
            Ok(r) => r,
            Err(e) => return fail(&format!("Exec-Broker nicht erreichbar: {e}")),
        };
        drop(req);
        match reply {
            Reply::Started { .. } => {}
            Reply::Error { code, message } => return fail(&format!("{code}: {message}")),
            Reply::Exited { .. } => return fail("unerwartete Antwort"),
        }
        loop {
            let mut fds = [
                libc::pollfd {
                    fd: conn.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: pipe[0],
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            // SAFETY: poll mit gültigem Array.
            let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
            if n < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return fail("poll");
            }
            if fds[1].revents & libc::POLLIN != 0 {
                let mut buf = [0u8; 16];
                // SAFETY: read in einen gültigen Puffer.
                let got = unsafe { libc::read(pipe[0], buf.as_mut_ptr().cast(), buf.len()) };
                for sig in buf.iter().take(got.max(0) as usize) {
                    let _ = write_frame(
                        &mut conn,
                        &ClientMsg::Signal {
                            signal: i32::from(*sig),
                        },
                    );
                }
            }
            if fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                let reply: std::io::Result<Option<Reply>> = read_frame(&mut conn);
                return match reply {
                    Ok(Some(Reply::Exited {
                        signal: Some(sig), ..
                    })) => {
                        handlers(false);
                        // SAFETY: Standard-Handler wiederhergestellt; raise beendet den Prozess.
                        unsafe {
                            libc::raise(sig);
                        }
                        128 + sig
                    }
                    Ok(Some(Reply::Exited { code: Some(c), .. })) => c,
                    Ok(Some(Reply::Error { code, message })) => fail(&format!("{code}: {message}")),
                    _ => {
                        // Den Rest der Verbindung verwerfen.
                        let _ = conn.read(&mut [0u8; 1]);
                        fail("Verbindung zum Exec-Broker abgebrochen")
                    }
                };
            }
        }
    }
}

/// Schickt `req` mit den Deskriptoren `stdio` und liefert die erste Antwort (Tests,
/// Diagnose).
#[cfg(unix)]
pub fn request(
    socket: &Path,
    req: &Request,
    stdio: [std::os::fd::RawFd; 3],
) -> std::io::Result<Reply> {
    imp::request(socket, req, stdio).map(|(_, reply)| reply)
}

/// Führt `bt-exec` aus: `argv0` bestimmt die Shell (`bt-exec-zsh` …) bzw. `args[0]` das
/// Programm. Liefert den Exit-Code; endet das Kommando durch ein Signal, beendet sich
/// `bt-exec` mit demselben Signal.
pub fn run(argv0: &OsString, args: Vec<OsString>) -> i32 {
    let argv = match target(argv0, args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("bt-exec: {e}");
            return CANNOT_EXECUTE;
        }
    };
    #[cfg(unix)]
    {
        imp::run(argv)
    }
    #[cfg(not(unix))]
    {
        let _ = argv;
        eprintln!("bt-exec: Exec-Broker gibt es unter Windows erst mit SBX-016 (M5)");
        CANNOT_EXECUTE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sbx_002_shell_shims_pick_the_shell_and_keep_all_arguments() {
        let argv = target(
            &OsString::from("/x/bt-exec-sh"),
            vec!["-c".into(), "echo hi".into()],
        )
        .unwrap();
        assert!(argv[0].ends_with("/sh"), "{argv:?}");
        assert_eq!(&argv[1..], ["-c", "echo hi"]);
        assert!(target(&OsString::from("bt-exec-fish"), vec![]).is_err());
        assert!(target(&OsString::from("bt-exec"), vec![]).is_err());
        let argv = target(&OsString::from("bt-exec"), vec!["sh".into()]).unwrap();
        assert!(Path::new(&argv[0]).is_absolute(), "{argv:?}");
        let argv = target(
            &OsString::from("bt-exec"),
            vec!["./skript".into(), "a".into()],
        )
        .unwrap();
        assert!(
            Path::new(&argv[0]).is_absolute() && argv[0].ends_with("/skript"),
            "{argv:?}"
        );
        assert_eq!(&argv[1..], ["a"]);
        let argv = target(&OsString::from("bt-exec"), vec!["/bin/sh".into()]).unwrap();
        assert_eq!(argv, ["/bin/sh"]);
        assert!(target(&OsString::from("bt-exec"), vec!["gibt-es-nicht-xyz".into()]).is_err());
        assert_eq!(run(&OsString::from("bt-exec-fish"), vec![]), CANNOT_EXECUTE);
    }

    #[test]
    fn sbx_002_bt_exec_forwards_only_absolute_path_entries() {
        assert_eq!(
            super::super::absolute_path("bin:/usr/bin::./x:/bin"),
            "/usr/bin:/bin"
        );
        assert!(super::super::path_ok("/usr/bin:/bin"));
        for bad in ["bin:/usr/bin", "/usr/bin::/bin", "", "./x", "/usr/bin:"] {
            assert!(!super::super::path_ok(bad), "{bad:?}");
        }
    }

    #[test]
    fn sbx_005_overrides_are_an_allowlist() {
        // #259: Nur Basis-, Werkzeug- und ausdrücklich konfigurierte Variablen gehen von
        // Stufe 1 nach Stufe 2; ein Token unter neuem Namen nicht.
        use super::super::override_rejected as r;
        let configured = vec!["RUST_LOG".to_owned(), "NODE_OPTIONS".to_owned()];
        for bad in [
            "FOO_TOKEN",
            "GITHUB_TOKEN",
            "NPM_TOKEN",
            "GIT_ASKPASS",
            "GIT_CONFIG_PARAMETERS",
            "SSH_ASKPASS",
            "AWS_SECRET_ACCESS_KEY",
        ] {
            assert!(r(bad, &configured).is_some(), "{bad}");
        }
        for ok in [
            "PATH",
            "LANG",
            "LC_ALL",
            "TZ",
            "PWD",
            "OLDPWD",
            "TERM",
            "COLUMNS",
            "GIT_EDITOR",
            "GIT_AUTHOR_NAME",
            "RUST_LOG",
            "NODE_OPTIONS",
        ] {
            assert!(r(ok, &configured).is_none(), "{ok}");
        }
        // `LC_*` nur mit POSIX- bzw. glibc-Kategorien, nicht als Präfix für beliebige Namen.
        for bad in ["LC_FOO_TOKEN", "LC_", "LC_SECRET"] {
            assert!(r(bad, &configured).is_some(), "{bad}");
        }
        for ok in ["LC_CTYPE", "LC_MESSAGES", "LC_TIME", "LC_PAPER"] {
            assert!(r(ok, &configured).is_none(), "{ok}");
        }
        // Konfigurierte Namen heben Sperren nie auf.
        let bad = vec![
            "LD_PRELOAD".to_owned(),
            "HOME".to_owned(),
            "CLAUDE_X".to_owned(),
        ];
        for name in &bad {
            assert!(r(name, &bad).is_some(), "{name}");
        }
    }

    #[test]
    fn sbx_002_overrides_drop_stage_one_and_denied_variables() {
        let r = |name: &str| super::super::override_rejected(name, &[]);
        for bad in [
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "HTTPS_PROXY",
            "https_proxy",
            "SSL_CERT_FILE",
            "BETON_EXEC_TOKEN",
            "HOME",
            "TMPDIR",
            "XDG_RUNTIME_DIR",
            "CLAUDECODE",
            "CLAUDE_CODE_SHELL",
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "CODEX_HOME",
            "a b",
            "1A",
            "",
        ] {
            assert!(r(bad).is_some(), "{bad}");
        }
        for ok in ["PATH", "LANG", "PWD", "GIT_EDITOR", "CI"] {
            assert!(r(ok).is_none(), "{ok}");
        }
    }
}
