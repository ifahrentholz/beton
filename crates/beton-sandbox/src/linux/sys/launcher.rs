//! Der Launcher `beton __sandbox-exec` (SBX-008, SBX-009, SBX-013).
//!
//! Prozesse (siehe Modul [`crate::linux`]): L (außen, wartet und räumt auf), C (Connector zum
//! Proxy), P1 (PID 1 der Sandbox, Reaper), R (Relay), T (Ziel). L ist einthreadig, damit
//! `unshare(CLONE_NEWUSER)` möglich ist; `beton` ruft den Launcher vor der Tokio-Laufzeit auf.
//!
//! Fail closed: Jeder Fehler vor `execve` des Ziels geht als [`Ctl::Failed`] an den Runner, und
//! das Ziel läuft nie.

use std::convert::Infallible;
use std::ffi::{CString, OsString};
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::msg::{self, CONNECTOR_FD, CTL_FD, Ctl, MAX_PLAN, PLAN_FD, STATUS_FD};
use super::util::{self, Status, StepError, StepResult, close, cstr, cvt};
use super::{cgroup, gitwatch, landlock, mounts, net, probe, seccomp};
use crate::linux::plan::{self, LaunchMode, LaunchPlan};
use crate::linux::{LAUNCH_FAILED, PLAN_VERSION};
use crate::resolve::NetMode;

/// Abstand zum `pids`-Limit, ab dem ohne cgroup ein Verstoß gemeldet wird (Launcher und
/// Init-Prozess zählen mit).
const PIDS_SLACK: u32 = 2;

/// Einstieg (nach `__sandbox-exec`).
pub fn main(args: &[OsString]) -> ExitCode {
    if args.first().is_some_and(|a| a == "--probe-target") {
        return probe_target(&args[1..]);
    }
    match run() {
        Ok(never) => match never {},
        Err(e) => {
            fail(&e);
            ExitCode::from(LAUNCH_FAILED)
        }
    }
}

/// Meldet einen Fehler an den Runner (oder, ohne Status-Kanal, auf stderr).
fn fail(e: &StepError) {
    let mut message = e.message.clone();
    if message.len() > 4096 {
        let mut end = 4096;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    let sent = msg::send(
        CTL_FD,
        &Ctl::Failed {
            step: e.step.into(),
            message,
            errno: e.errno,
        },
    );
    if sent.is_err() {
        eprintln!("beton sandbox: {e}");
    }
}

fn is_type(fd: RawFd, kind: libc::mode_t) -> bool {
    // SAFETY: fstat in eine eigene Struktur.
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        libc::fstat(fd, &mut st) == 0 && (st.st_mode & libc::S_IFMT) == kind
    }
}

fn is_seqpacket(fd: RawFd) -> bool {
    let mut ty: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: getsockopt in eine eigene Variable.
    let r = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut ty as *mut libc::c_int).cast(),
            &mut len,
        )
    };
    r == 0 && ty == libc::SOCK_SEQPACKET
}

fn run() -> StepResult<Infallible> {
    if !is_type(PLAN_FD, libc::S_IFIFO) || !is_seqpacket(CTL_FD) {
        return Err(StepError::new(
            "plan",
            "kein Startplan vom Runner (der Launcher startet nur über beton-sandbox)",
        ));
    }
    let data =
        util::read_to_end(PLAN_FD, MAX_PLAN).map_err(|e| StepError::io("plan", "lesen", e))?;
    close(PLAN_FD);
    let plan: LaunchPlan =
        serde_json::from_slice(&data).map_err(|e| StepError::new("plan", e.to_string()))?;
    if plan.version != PLAN_VERSION {
        return Err(StepError::new(
            "plan",
            format!("Version {} statt {PLAN_VERSION}", plan.version),
        ));
    }
    if plan.inner { inner(plan) } else { outer(plan) }
}

fn abi_of(plan: &LaunchPlan) -> StepResult<u32> {
    let kernel = probe::landlock_abi();
    if let Some(why) = probe::landlock_missing(kernel) {
        return Err(StepError::new("landlock", why));
    }
    let abi = landlock::effective_abi(kernel.unwrap_or(0), plan.abi_cap);
    if abi < 2 {
        return Err(StepError::new(
            "landlock",
            format!("ABI {abi} reicht nicht"),
        ));
    }
    Ok(abi)
}

fn write_file(p: &str, s: &str) -> StepResult<()> {
    std::fs::write(p, s).map_err(|e| StepError::io("user-namespace", p, e))
}

/// L: außerhalb der Sandbox; startet C und P1 (bzw. bwrap), wartet und räumt auf.
fn outer(plan: LaunchPlan) -> StepResult<Infallible> {
    util::signals(&util::IGNORED, libc::SIG_IGN);
    let abi = abi_of(&plan)?;
    if !seccomp::available() {
        return Err(StepError::new("seccomp", "seccomp-BPF ist nicht verfügbar"));
    }
    let mut cg = match (
        &plan.cgroup,
        plan.limits.memory_mb.is_some() || plan.limits.pids.is_some(),
    ) {
        (Some(base), true) => Some(
            cgroup::Cgroup::create(base, plan.limits.memory_mb, plan.limits.pids)
                .map_err(|e| StepError::io("cgroup", base.display(), e))?,
        ),
        _ => None,
    };
    // Connector vor allen Namespaces: Er bleibt im Host-Netz.
    let connector = match plan.net {
        NetMode::ProxyOnly { port } => {
            let (mine, theirs) =
                util::seqpacket_pair().map_err(|e| StepError::io("netz", "socketpair", e))?;
            match util::fork().map_err(|e| StepError::io("netz", "fork", e))? {
                0 => {
                    close(mine);
                    net::connector(theirs, port, abi)
                }
                _ => {
                    close(theirs);
                    Some(mine)
                }
            }
        }
        NetMode::None => None,
    };
    // Vor dem Start des Ziels: neue `commondir`/`gitdir` im Git-Dir erkennen (#209).
    let git_watch = gitwatch::GitWatch::new(&plan::git_watch_dirs(&plan))
        .map_err(|e| StepError::io("git", "inotify", e))?;
    let (status_r, status_w) = util::pipe().map_err(|e| StepError::io("start", "pipe", e))?;
    let (go_r, go_w) = util::pipe().map_err(|e| StepError::io("start", "pipe", e))?;
    let mut mask_dir = None;
    let child = match &plan.mode {
        LaunchMode::Landlock => {
            // SAFETY: getuid/getgid ohne Seiteneffekte.
            let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
            let flags = libc::CLONE_NEWUSER
                | libc::CLONE_NEWNS
                | libc::CLONE_NEWPID
                | libc::CLONE_NEWNET
                | libc::CLONE_NEWIPC
                | libc::CLONE_NEWUTS;
            // SAFETY: unshare in einem einthreadigen Prozess.
            cvt(unsafe { libc::unshare(flags) })
                .map_err(|e| StepError::io("user-namespace", "unshare", e))?;
            write_file("/proc/self/setgroups", "deny")?;
            write_file("/proc/self/uid_map", &format!("{uid} {uid} 1"))?;
            write_file("/proc/self/gid_map", &format!("{gid} {gid} 1"))?;
            // Erst nach den ID-Maps: Sie verlangen ein beschreibbares /proc/self.
            util::set_dumpable(false).map_err(|e| StepError::io("start", "dumpable", e))?;
            match util::fork().map_err(|e| StepError::io("start", "fork", e))? {
                0 => {
                    close(go_w);
                    close(status_r);
                    p1(&plan, abi, connector, status_w, Some(go_r))
                }
                pid => pid,
            }
        }
        LaunchMode::Bwrap { bwrap } => {
            util::set_dumpable(false).map_err(|e| StepError::io("start", "dumpable", e))?;
            let dir = tempfile::Builder::new()
                .prefix("beton-mask-")
                .tempdir()
                .map_err(|e| StepError::io("bwrap", "Maskendatei", e))?;
            let file = dir.path().join("leer");
            std::fs::File::create(&file).map_err(|e| StepError::io("bwrap", file.display(), e))?;
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000))
                    .map_err(|e| StepError::io("bwrap", file.display(), e))?;
            }
            let mut inner_plan = plan.clone();
            inner_plan.inner = true;
            inner_plan.mask_file = Some(file);
            let launcher =
                std::env::current_exe().map_err(|e| StepError::io("bwrap", "eigener Pfad", e))?;
            let args = crate::linux::bwrap::args(&inner_plan, &launcher)
                .map_err(|e| StepError::new("bwrap", e.to_string()))?;
            let mut argv: Vec<CString> =
                vec![cstr(bwrap).map_err(|e| StepError::io("bwrap", bwrap.display(), e))?];
            for a in &args {
                argv.push(
                    CString::new(a.as_str())
                        .map_err(|_| StepError::new("bwrap", "NUL in Argument"))?,
                );
            }
            let json = serde_json::to_vec(&inner_plan)
                .map_err(|e| StepError::new("bwrap", e.to_string()))?;
            let (pr, pw) = util::pipe().map_err(|e| StepError::io("bwrap", "pipe", e))?;
            match util::fork().map_err(|e| StepError::io("bwrap", "fork", e))? {
                0 => {
                    close(go_w);
                    close(status_r);
                    close(pw);
                    let e = exec_bwrap(&argv, pr, connector, status_w, go_r);
                    fail(&e);
                    std::process::exit(i32::from(LAUNCH_FAILED));
                }
                pid => {
                    close(pr);
                    let r = util::write_all(pw, &json);
                    close(pw);
                    if let Err(e) = r {
                        util::kill(pid, libc::SIGKILL);
                        return Err(StepError::io("bwrap", "Plan", e));
                    }
                    mask_dir = Some(dir);
                    pid
                }
            }
        }
    };
    if let Some(cg) = &cg
        && let Err(e) = cg.add(child)
    {
        util::kill(child, libc::SIGKILL);
        cg.remove();
        return Err(StepError::io("cgroup", cg.path.display(), e));
    }
    let _ = util::write_all(go_w, b"g");
    close(go_w);
    close(go_r);
    close(status_w);
    util::stdio_to_null();
    let sigfd = util::sigchld_fd().map_err(|e| StepError::io("start", "signalfd", e))?;
    let mut fds = vec![CTL_FD, sigfd];
    fds.extend(git_watch.as_ref().map(gitwatch::GitWatch::fd));
    let mut git_hits: Vec<PathBuf> = Vec::new();
    let child_status = loop {
        let rev = util::poll(&fds, 200).unwrap_or_default();
        if let Some(w) = &git_watch
            && git_hits.is_empty()
        {
            git_hits = w.check();
            if !git_hits.is_empty() {
                // Eine Git-Steuerdatei entstand: Sandbox sofort beenden (fail closed).
                util::kill(child, libc::SIGKILL);
            }
        }
        if rev
            .first()
            .is_some_and(|r| r & (libc::POLLHUP | libc::POLLERR) != 0)
        {
            // Der Runner ist weg (SBX-008 AC6): alles beenden.
            util::kill(child, libc::SIGKILL);
        }
        if rev.get(1).is_some_and(|r| *r != 0) {
            util::drain_signalfd(sigfd);
        }
        if let Some(cg) = &mut cg {
            for target in cg.new_events() {
                let _ = msg::send(
                    CTL_FD,
                    &Ctl::Limit {
                        target: target.into(),
                    },
                );
            }
        }
        match util::waitpid(child, true) {
            Ok(Some((_, s))) => break s,
            Ok(None) => {}
            Err(_) => break Status::Exited(i32::from(LAUNCH_FAILED)),
        }
    };
    if let Some(cg) = &mut cg {
        for target in cg.new_events() {
            let _ = msg::send(
                CTL_FD,
                &Ctl::Limit {
                    target: target.into(),
                },
            );
        }
        cg.remove();
    }
    if let Some(c) = connector {
        close(c);
    }
    drop(mask_dir);
    let reported = util::read_to_end(status_r, 64)
        .ok()
        .and_then(|b| Status::decode(&b));
    let mut status = reported.unwrap_or(child_status);
    if let Some(w) = &git_watch {
        // Nach dem Ende aller Prozesse der Sandbox: entfernen und melden.
        for p in w.sweep() {
            if !git_hits.contains(&p) {
                git_hits.push(p);
            }
        }
        for p in &git_hits {
            let _ = msg::send(
                CTL_FD,
                &Ctl::Fs {
                    target: p.display().to_string(),
                },
            );
        }
        if !git_hits.is_empty() {
            status = Status::Signaled(libc::SIGKILL);
        }
    }
    exit_like(status)
}

/// Kind von L im bwrap-Modus: wartet auf die cgroup, legt die Deskriptoren fest und startet bwrap.
fn exec_bwrap(
    argv: &[CString],
    plan_r: RawFd,
    connector: Option<RawFd>,
    status_w: RawFd,
    go_r: RawFd,
) -> StepError {
    let r = (|| -> StepResult<()> {
        util::set_pdeathsig(libc::SIGKILL).map_err(|e| StepError::io("bwrap", "pdeathsig", e))?;
        let mut go = [0u8; 1];
        // SAFETY: Liest ein Byte in einen eigenen Puffer.
        if unsafe { libc::read(go_r, go.as_mut_ptr().cast(), 1) } != 1 {
            return Err(StepError::new("bwrap", "Launcher beendet"));
        }
        // Erst alles weg von 3–6, dann auf die festen Nummern.
        let plan = util::dup_above(plan_r, 10).map_err(|e| StepError::io("bwrap", "dup", e))?;
        let status = util::dup_above(status_w, 10).map_err(|e| StepError::io("bwrap", "dup", e))?;
        let conn = connector
            .map(|c| util::dup_above(c, 10))
            .transpose()
            .map_err(|e| StepError::io("bwrap", "dup", e))?;
        util::move_fd(plan, PLAN_FD).map_err(|e| StepError::io("bwrap", "Plan-FD", e))?;
        util::move_fd(status, STATUS_FD).map_err(|e| StepError::io("bwrap", "Status-FD", e))?;
        match conn {
            Some(c) => util::move_fd(c, CONNECTOR_FD),
            None => {
                close(CONNECTOR_FD);
                Ok(())
            }
        }
        .map_err(|e| StepError::io("bwrap", "Connector-FD", e))?;
        util::reset_signals();
        let mut ptrs: Vec<*const libc::c_char> = argv.iter().map(|a| a.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let env: [*const libc::c_char; 1] = [std::ptr::null()];
        // SAFETY: argv und envp sind NULL-terminierte Arrays gültiger C-Strings.
        unsafe {
            libc::execve(ptrs[0], ptrs.as_ptr(), env.as_ptr());
        }
        Err(StepError::io(
            "bwrap",
            "exec",
            std::io::Error::last_os_error(),
        ))
    })();
    match r {
        Ok(()) => StepError::new("bwrap", "exec"),
        Err(e) => e,
    }
}

/// Endet wie das Ziel (gleicher Code bzw. dasselbe Signal).
fn exit_like(s: Status) -> ! {
    match s {
        Status::Exited(c) => std::process::exit(c),
        Status::Signaled(sig) => {
            util::reset_signals();
            // SAFETY: Signal an sich selbst; bei Rückkehr bleibt der Exit-Code 128+n.
            unsafe {
                libc::raise(sig);
            }
            std::process::exit(128 + sig)
        }
    }
}

/// Launcher innerhalb von bwrap (Namespaces und Mounts stehen schon).
fn inner(plan: LaunchPlan) -> StepResult<Infallible> {
    let abi = abi_of(&plan)?;
    let connector = matches!(plan.net, NetMode::ProxyOnly { .. }).then_some(CONNECTOR_FD);
    p1(&plan, abi, connector, STATUS_FD, None)
}

/// P1 bzw. der innere Launcher: richtet die Sandbox ein, startet T und wartet als Reaper.
fn p1(
    plan: &LaunchPlan,
    abi: u32,
    connector: Option<RawFd>,
    status_w: RawFd,
    go_r: Option<RawFd>,
) -> ! {
    match p1_run(plan, abi, connector, go_r) {
        Ok(st) => {
            let _ = util::write_all(status_w, &st.encode());
            std::process::exit(0)
        }
        Err(e) => {
            fail(&e);
            std::process::exit(i32::from(LAUNCH_FAILED))
        }
    }
}

fn count_processes() -> u32 {
    std::fs::read_dir("/proc")
        .map(|d| {
            d.flatten()
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .is_some_and(|s| s.bytes().all(|b| b.is_ascii_digit()))
                })
                .count() as u32
        })
        .unwrap_or(0)
}

fn p1_run(
    plan: &LaunchPlan,
    abi: u32,
    connector: Option<RawFd>,
    go_r: Option<RawFd>,
) -> StepResult<Status> {
    let own_ns = plan.mode == LaunchMode::Landlock;
    let targets = if own_ns {
        util::set_pdeathsig(libc::SIGKILL).map_err(|e| StepError::io("start", "pdeathsig", e))?;
        mounts::base()?;
        let (ops, targets) =
            plan::mount_ops(plan).map_err(|e| StepError::new("mount", e.to_string()))?;
        mounts::apply(&ops)?;
        let name = b"beton-sandbox";
        // SAFETY: sethostname mit gültigem Puffer im eigenen UTS-Namespace.
        cvt(unsafe { libc::sethostname(name.as_ptr().cast(), name.len()) })
            .map_err(|e| StepError::io("uts", "sethostname", e))?;
        net::loopback_up().map_err(|e| StepError::io("netz", "lo", e))?;
        targets
    } else {
        plan::mask_targets(plan).map_err(|e| StepError::new("masken", e.to_string()))?
    };
    // Nach den Mounts (die /proc/self/fd brauchen): Das Ziel kann P1 bzw. den inneren Launcher
    // weder über /proc/<pid>/mem noch per ptrace erreichen, auch ohne Yama.
    util::set_dumpable(false).map_err(|e| StepError::io("start", "dumpable", e))?;
    let proxy_port = match plan.net {
        NetMode::ProxyOnly { port } => Some(port),
        NetMode::None => None,
    };
    if let (Some(port), Some(conn)) = (proxy_port, connector) {
        let listener = net::relay_listener(port).map_err(|e| StepError::io("netz", "Relay", e))?;
        match util::fork().map_err(|e| StepError::io("netz", "fork", e))? {
            0 => net::relay(listener, conn, abi),
            _ => {
                drop(listener);
                close(conn);
            }
        }
    }
    if let Some(go) = go_r {
        let mut b = [0u8; 1];
        // SAFETY: Liest ein Byte in einen eigenen Puffer.
        if unsafe { libc::read(go, b.as_mut_ptr().cast(), 1) } != 1 {
            return Err(StepError::new("start", "Launcher beendet"));
        }
        close(go);
    }
    let (err_r, err_w) = util::pipe().map_err(|e| StepError::io("start", "pipe", e))?;
    let sigfd = util::sigchld_fd().map_err(|e| StepError::io("start", "signalfd", e))?;
    let t = match util::fork().map_err(|e| StepError::io("start", "fork", e))? {
        0 => {
            close(err_r);
            target(plan, abi, proxy_port, &targets.missing_in_write, err_w)
        }
        pid => pid,
    };
    close(err_w);
    let err = util::read_to_end(err_r, 64 * 1024).unwrap_or_default();
    close(err_r);
    if !err.is_empty() {
        let text = String::from_utf8_lossy(&err);
        let mut parts = text.splitn(3, '\n');
        let step = parts.next().unwrap_or("start");
        let errno: i32 = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        let message = parts.next().unwrap_or_default();
        util::kill(t, libc::SIGKILL);
        let _ = util::waitpid(t, false);
        let step: &'static str = match step {
            "rlimit" => "rlimit",
            "cwd" => "cwd",
            "caps" => "caps",
            "landlock" => "landlock",
            "seccomp" => "seccomp",
            "exec" => "exec",
            _ => "start",
        };
        let mut e = StepError::new(step, message.trim().to_owned());
        e.errno = (errno != 0).then_some(errno);
        return Err(e);
    }
    let _ = msg::send(CTL_FD, &Ctl::Ready);
    util::stdio_to_null();
    let watch_pids = plan.cgroup.is_none().then_some(plan.limits.pids).flatten();
    let mut reported = false;
    // Mit `pids`-Limit ohne cgroup kurz takten, damit auch kurze Spitzen auffallen.
    let tick = if watch_pids.is_some() { 20 } else { 250 };
    loop {
        let _ = util::poll(&[sigfd], tick);
        util::drain_signalfd(sigfd);
        loop {
            match util::waitpid(-1, true) {
                Ok(Some((pid, s))) if pid == t => return Ok(s),
                Ok(Some(_)) => continue,
                Ok(None) => break,
                Err(_) => return Ok(Status::Exited(i32::from(LAUNCH_FAILED))),
            }
        }
        if let (Some(limit), false) = (watch_pids, reported)
            && count_processes() + PIDS_SLACK >= limit
        {
            reported = true;
            let _ = msg::send(
                CTL_FD,
                &Ctl::Limit {
                    target: "pids".into(),
                },
            );
        }
    }
}

/// Läuft der Prozess im ersten User-Namespace (setuid-bwrap ohne eigenen User-Namespace)?
fn in_initial_userns() -> bool {
    std::fs::read_to_string("/proc/self/uid_map")
        .map(|m| m.split_whitespace().collect::<Vec<_>>() == ["0", "0", "4294967295"])
        .unwrap_or(false)
}

fn processes_of_uid(uid: libc::uid_t) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::read_dir("/proc")
        .map(|d| {
            d.flatten()
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .is_some_and(|s| s.bytes().all(|b| b.is_ascii_digit()))
                        && e.metadata().is_ok_and(|m| m.uid() == uid)
                })
                .count() as u64
        })
        .unwrap_or(0)
}

#[cfg(target_env = "gnu")]
type Resource = libc::__rlimit_resource_t;
#[cfg(not(target_env = "gnu"))]
type Resource = libc::c_int;

fn setrlimit(res: Resource, v: u64) -> std::io::Result<()> {
    let l = libc::rlimit {
        rlim_cur: v,
        rlim_max: v,
    };
    // SAFETY: setrlimit mit gültiger Struktur.
    cvt(unsafe { libc::setrlimit(res, &l) })?;
    Ok(())
}

fn limits(plan: &LaunchPlan) -> StepResult<()> {
    let e = |e| StepError::io("rlimit", "setrlimit", e);
    // Mit cgroup setzt `pids.max` die Grenze (und meldet sie genau); RLIMIT_NPROC griffe
    // sonst vorher, weil es auch den Launcher außerhalb der cgroup mitzählt.
    if let (Some(p), None) = (plan.limits.pids, &plan.cgroup) {
        let mut n = u64::from(p);
        if in_initial_userns() {
            // Ohne eigenen User-Namespace zählt RLIMIT_NPROC alle Prozesse des Benutzers.
            // SAFETY: getuid ohne Seiteneffekte.
            n += processes_of_uid(unsafe { libc::getuid() });
        }
        setrlimit(libc::RLIMIT_NPROC, n).map_err(e)?;
    }
    if let Some(s) = plan.limits.cpu_seconds {
        setrlimit(libc::RLIMIT_CPU, s).map_err(e)?;
    }
    if let Some(mb) = plan.limits.file_size_mb {
        setrlimit(libc::RLIMIT_FSIZE, mb.saturating_mul(1024 * 1024)).map_err(e)?;
    }
    setrlimit(libc::RLIMIT_CORE, 0).map_err(e)?;
    Ok(())
}

/// Gibt alle Capabilities ab; auch UID 0 im eigenen User-Namespace bekommt sie über `execve`
/// nicht zurück (`SECBIT_NOROOT`, leere Bounding-Menge).
fn drop_caps() -> StepResult<()> {
    const SECBIT_NOROOT: libc::c_ulong = 1 << 0;
    const SECBIT_NOROOT_LOCKED: libc::c_ulong = 1 << 1;
    const SECBIT_NO_SETUID_FIXUP: libc::c_ulong = 1 << 2;
    const SECBIT_NO_SETUID_FIXUP_LOCKED: libc::c_ulong = 1 << 3;
    const SECBIT_KEEP_CAPS_LOCKED: libc::c_ulong = 1 << 5;
    const SECBIT_NO_CAP_AMBIENT_RAISE: libc::c_ulong = 1 << 6;
    const SECBIT_NO_CAP_AMBIENT_RAISE_LOCKED: libc::c_ulong = 1 << 7;
    let e = |what: &str, e| StepError::io("caps", what, e);
    // SAFETY: prctl/capset mit gültigen Argumenten und eigener Struktur.
    unsafe {
        let mut bounding = Vec::new();
        for cap in 0..64 {
            if libc::prctl(libc::PR_CAPBSET_READ, cap, 0, 0, 0) == 1 {
                bounding.push(cap);
            }
        }
        let bits = SECBIT_NOROOT
            | SECBIT_NOROOT_LOCKED
            | SECBIT_NO_SETUID_FIXUP
            | SECBIT_NO_SETUID_FIXUP_LOCKED
            | SECBIT_KEEP_CAPS_LOCKED
            | SECBIT_NO_CAP_AMBIENT_RAISE
            | SECBIT_NO_CAP_AMBIENT_RAISE_LOCKED;
        let secure = libc::prctl(libc::PR_SET_SECUREBITS, bits, 0, 0, 0);
        for cap in &bounding {
            if libc::prctl(libc::PR_CAPBSET_DROP, *cap, 0, 0, 0) != 0 && libc::getuid() == 0 {
                // UID 0 mit verbleibender Bounding-Menge erhielte über execve Capabilities.
                return Err(e("Bounding-Menge", std::io::Error::last_os_error()));
            }
        }
        if secure != 0 && libc::getuid() == 0 && !bounding.is_empty() {
            return Err(e("securebits", std::io::Error::last_os_error()));
        }
        libc::prctl(
            libc::PR_CAP_AMBIENT,
            libc::PR_CAP_AMBIENT_CLEAR_ALL,
            0,
            0,
            0,
        );
        #[repr(C)]
        struct Header {
            version: u32,
            pid: libc::c_int,
        }
        #[repr(C)]
        struct Data {
            effective: u32,
            permitted: u32,
            inheritable: u32,
        }
        let mut h = Header {
            version: 0x2008_0522,
            pid: 0,
        };
        let d = [
            Data {
                effective: 0,
                permitted: 0,
                inheritable: 0,
            },
            Data {
                effective: 0,
                permitted: 0,
                inheritable: 0,
            },
        ];
        cvt_long(libc::syscall(libc::SYS_capset, &mut h, d.as_ptr()))
            .map_err(|err| e("capset", err))?;
    }
    Ok(())
}

fn cvt_long(r: libc::c_long) -> std::io::Result<libc::c_long> {
    util::cvt_long(r)
}

/// Sucht `program` wie `execvp`, aber im `PATH` des Ziels.
fn candidates(plan: &LaunchPlan) -> Vec<PathBuf> {
    if plan.program.as_os_str().as_bytes().contains(&b'/') {
        return vec![plan.program.clone()];
    }
    let path = plan
        .env
        .iter()
        .find(|(k, _)| k == "PATH")
        .map(|(_, v)| v.as_str())
        .unwrap_or("/usr/bin:/bin");
    path.split(':')
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join(&plan.program))
        .collect()
}

/// T: Rechte abgeben und das Ziel starten.
fn target(
    plan: &LaunchPlan,
    abi: u32,
    proxy_port: Option<u16>,
    missing: &[PathBuf],
    err_w: RawFd,
) -> ! {
    let e = target_run(plan, abi, proxy_port, missing);
    let text = format!("{}\n{}\n{}", e.step, e.errno.unwrap_or(0), e.message);
    let _ = util::write_all(err_w, text.as_bytes());
    std::process::exit(i32::from(LAUNCH_FAILED))
}

fn target_run(
    plan: &LaunchPlan,
    abi: u32,
    proxy_port: Option<u16>,
    missing: &[PathBuf],
) -> StepError {
    let r = (|| -> StepResult<()> {
        util::reset_signals();
        std::env::set_current_dir(&plan.cwd)
            .map_err(|e| StepError::io("cwd", plan.cwd.display(), e))?;
        limits(plan)?;
        drop_caps()?;
        util::set_no_new_privs().map_err(|e| StepError::io("landlock", "no_new_privs", e))?;
        let rules = plan::landlock_rules(plan, missing);
        // Unix-Sockets mit Pfad (ab ABI 9): privates `/tmp`, Scratch und die Verzeichnisse
        // ausdrücklich freigegebener Sockets (Stufe 1: Exec-Broker, SBX-002).
        let mut unix_dirs = vec![Path::new("/tmp"), plan.scratch.as_path()];
        unix_dirs.extend(plan.unix_sockets.iter().filter_map(|s| s.parent()));
        let tcp = landlock::Tcp {
            connect: proxy_port
                .into_iter()
                .chain(plan.dev_ports.iter().copied())
                .collect(),
            bind: plan.dev_ports.clone(),
        };
        landlock::apply(abi, &rules, &tcp, &unix_dirs)
            .map_err(|m| StepError::new("landlock", m))?;
        util::cloexec_from(3).map_err(|e| StepError::io("start", "close_range", e))?;
        seccomp::apply(abi).map_err(|m| StepError::new("seccomp", m))?;
        let args: Vec<CString> = std::iter::once(plan.program.as_os_str().as_bytes().to_vec())
            .chain(plan.args.iter().map(|a| a.as_bytes().to_vec()))
            .map(CString::new)
            .collect::<Result<_, _>>()
            .map_err(|_| StepError::new("exec", "NUL-Byte in einem Argument"))?;
        let env: Vec<CString> = plan
            .env
            .iter()
            .map(|(k, v)| CString::new(format!("{k}={v}")))
            .collect::<Result<_, _>>()
            .map_err(|_| StepError::new("exec", "NUL-Byte im Environment"))?;
        let mut argv: Vec<*const libc::c_char> = args.iter().map(|a| a.as_ptr()).collect();
        argv.push(std::ptr::null());
        let mut envp: Vec<*const libc::c_char> = env.iter().map(|a| a.as_ptr()).collect();
        envp.push(std::ptr::null());
        let mut last = std::io::Error::from_raw_os_error(libc::ENOENT);
        for c in candidates(plan) {
            let Ok(p) = cstr(&c) else { continue };
            // SAFETY: argv und envp sind NULL-terminierte Arrays gültiger C-Strings.
            unsafe {
                libc::execve(p.as_ptr(), argv.as_ptr(), envp.as_ptr());
            }
            last = std::io::Error::last_os_error();
            if !matches!(
                last.raw_os_error(),
                Some(libc::ENOENT | libc::EACCES | libc::ENOTDIR)
            ) {
                break;
            }
        }
        Err(StepError::io("exec", plan.program.display(), last))
    })();
    match r {
        Ok(()) => StepError::new("exec", "unerwartet zurückgekehrt"),
        Err(e) => e,
    }
}

/// Ziel des Selbsttests der Probe: `--read P …` darf nicht lesbar sein, `--write W` muss
/// beschreibbar sein, und neue User-Namespaces sind gesperrt.
fn probe_target(args: &[OsString]) -> ExitCode {
    let mut ok = true;
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let Some(p) = it.next() else { break };
        if flag == "--read" {
            ok &= std::fs::read(p).is_err();
        } else if flag == "--write" {
            ok &= std::fs::write(p, b"ok").is_ok();
        }
    }
    // SAFETY: unshare wird von seccomp abgelehnt (und wäre sonst nur für diesen Prozess).
    let r = unsafe { libc::unshare(libc::CLONE_NEWUSER) };
    ok &= r != 0;
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::command::CommandSpec;
    use crate::resolve::{ResolvedPolicy, Stage};

    fn plan(program: &str, path: Option<&str>) -> LaunchPlan {
        let mut policy = ResolvedPolicy::unsandboxed(Stage::Tools);
        policy.workspace = "/".into();
        if let Some(p) = path {
            policy.env.insert("PATH".into(), p.into());
        }
        LaunchPlan::new(&policy, &CommandSpec::new(program), LaunchMode::Landlock)
    }

    #[test]
    fn sbx_008_programs_are_searched_in_the_targets_path() {
        assert_eq!(
            candidates(&plan("/bin/sh", None)),
            vec![PathBuf::from("/bin/sh")]
        );
        assert_eq!(
            candidates(&plan("sh", Some("/a::/b"))),
            vec![PathBuf::from("/a/sh"), PathBuf::from("/b/sh")]
        );
        assert_eq!(candidates(&plan("sh", None)).len(), 2);
    }

    #[test]
    fn sbx_008_caps_drop_in_the_calling_thread() {
        // Capabilities, Bounding-Menge und securebits gehören dem Thread.
        let caps = std::thread::spawn(|| {
            drop_caps().unwrap();
            std::fs::read_to_string("/proc/thread-self/status").unwrap()
        })
        .join()
        .unwrap();
        for field in ["CapPrm", "CapEff", "CapAmb"] {
            assert!(
                caps.contains(&format!("{field}:\t0000000000000000")),
                "{caps}"
            );
        }
    }

    #[test]
    fn sbx_013_rlimits_without_limits_only_disable_core_dumps() {
        limits(&plan("/bin/true", None)).unwrap();
        assert!(count_processes() >= 1);
        let _ = in_initial_userns();
        // SAFETY: getuid ohne Seiteneffekte.
        assert!(processes_of_uid(unsafe { libc::getuid() }) >= 1);
    }

    #[test]
    fn sbx_008_launcher_without_plan_fails_closed() {
        // Ohne FD 3/4 vom Runner startet nichts.
        let e = run().unwrap_err();
        assert_eq!(e.step, "plan");
    }
}
