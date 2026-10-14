//! Gemeinsame Fixtures der Sandbox-Tests unter Linux und macOS (SBX-004, SBX-008, SBX-009,
//! SBX-013).
//!
//! Alle Homes, Dotfiles und Secrets sind Fixtures in Temp-Verzeichnissen; kein Test liest echte
//! Nutzerdaten. Jeder Angriff hat eine Kontrollgruppe ohne Sandbox (QA-005 AC1).
//!
//! Die Sonden (`probe_*`) sind Tests dieses Binaries, die nur etwas tun, wenn sie mit
//! `BETON_SBX_PROBE` gestartet werden: Der Test startet dasselbe Binary in der Sandbox
//! (`--exact support::probe`) und liest das Ergebnis aus stdout. So prüfen die Tests Syscalls
//! direkt, ohne auf Werkzeuge wie `strace` oder `nc` angewiesen zu sein.

#![allow(dead_code, reason = "nicht jedes Test-Binary nutzt jede Hilfsfunktion")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use beton_sandbox::{
    CommandSpec, NoneProvider, ResolveCtx, ResolvedPolicy, SandboxProvider, SandboxSpec, Stage,
    Stdio,
};

pub const FIXTURE_SSH: &str = "FIXTURE-SSH-PRIVATE-KEY";
pub const FIXTURE_ENV: &str = "FIXTURE-DOTENV";
pub const FIXTURE_CODEX: &str = "FIXTURE-CODEX-AUTH";

pub struct Fx {
    pub _tmp: tempfile::TempDir,
    pub root: PathBuf,
    pub home: PathBuf,
    pub ws: PathBuf,
    pub scratch: PathBuf,
}

pub fn write(p: &Path, content: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Gefälschtes Home voller Fixture-Secrets, Workspace mit `.env`, Scratch.
pub fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let home = root.join("home");
    let ws = root.join("ws");
    let scratch = root.join("scratch");
    write(&home.join(".ssh/id_ed25519"), FIXTURE_SSH);
    write(&home.join(".aws/credentials"), "FIXTURE-AWS");
    write(&home.join(".config/gh/hosts.yml"), "FIXTURE-GH");
    write(&home.join(".codex/auth.json"), FIXTURE_CODEX);
    write(&home.join(".claude/.credentials.json"), "FIXTURE-CLAUDE");
    write(
        &home.join(".netrc"),
        "machine x login y password FIXTURE-NETRC",
    );
    write(&home.join(".zshrc"), "# fixture\n");
    write(&home.join(".bashrc"), "# fixture\n");
    write(&home.join("Documents/notiz.txt"), "Notiz");
    std::fs::create_dir_all(home.join("Library/LaunchAgents")).unwrap();
    std::fs::create_dir_all(home.join(".config/systemd/user")).unwrap();
    write(&ws.join(".env"), FIXTURE_ENV);
    write(&ws.join(".github/workflows/ci.yml"), "on: push\n");
    write(&ws.join("src/main.rs"), "fn main() {}\n");
    std::fs::create_dir_all(&scratch).unwrap();
    Fx {
        _tmp: tmp,
        root,
        home,
        ws,
        scratch,
    }
}

impl Fx {
    pub fn ctx(&self, stage: Stage) -> ResolveCtx {
        let mut ctx = ResolveCtx::new(stage, &self.ws, &self.home, &self.scratch);
        ctx.adapters = beton_sandbox::masks::builtin_harness_grants();
        ctx.host_env = [("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        ctx
    }

    pub fn policy(&self, p: &dyn SandboxProvider, yaml: &str, stage: Stage) -> ResolvedPolicy {
        self.policy_with(p, yaml, self.ctx(stage))
    }

    pub fn policy_with(
        &self,
        p: &dyn SandboxProvider,
        yaml: &str,
        ctx: ResolveCtx,
    ) -> ResolvedPolicy {
        let spec = SandboxSpec::from_yaml(yaml).unwrap();
        p.resolve(&spec, &ctx).unwrap()
    }
}

#[derive(Debug)]
#[allow(
    dead_code,
    reason = "stderr erscheint in den Fehlermeldungen der Asserts"
)]
pub struct Out {
    pub ok: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub async fn run_with(
    provider: &dyn SandboxProvider,
    policy: &ResolvedPolicy,
    cmd: CommandSpec,
) -> Out {
    let child = provider
        .spawn(policy, cmd.stdio(Stdio::Null, Stdio::Piped, Stdio::Piped))
        .unwrap_or_else(|e| panic!("{}: Start scheitert: {e}", provider.id()));
    let out = child.output(Duration::from_secs(60)).await.unwrap();
    Out {
        ok: out.status.success(),
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// `sh -c` in der Sandbox.
pub async fn sh(provider: &dyn SandboxProvider, policy: &ResolvedPolicy, script: &str) -> Out {
    run_with(
        provider,
        policy,
        CommandSpec::new("/bin/sh")
            .args(["-c", script])
            .cwd(&policy.workspace),
    )
    .await
}

/// Kontrollgruppe: derselbe Befehl ohne Sandbox (Fixture-Home als `HOME`).
pub async fn control(fx: &Fx, script: &str) -> Out {
    let policy = ResolvedPolicy::unsandboxed(Stage::Tools);
    let mut cmd = CommandSpec::new("/bin/sh").args(["-c", script]).cwd(&fx.ws);
    cmd.env.clear = true;
    cmd.env.set = vec![
        ("HOME".into(), fx.home.display().to_string()),
        (
            "PATH".into(),
            "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".into(),
        ),
    ];
    run_with(&NoneProvider, &policy, cmd).await
}

/// Dieses Test-Binary als Sonde in der Sandbox: führt `probe` mit `args` aus.
pub fn probe_cmd(policy: &ResolvedPolicy, probe: &str, args: &[&str]) -> CommandSpec {
    let exe = std::env::current_exe().unwrap();
    CommandSpec::new(exe)
        .args([
            "--exact",
            "support::probe",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("BETON_SBX_PROBE", probe)
        .env("BETON_SBX_PROBE_ARGS", args.join("\u{1f}"))
        .cwd(&policy.workspace)
}

/// Lese-Grant für das Verzeichnis dieses Test-Binaries (für die Sonden).
pub fn exe_grant() -> String {
    let exe = std::env::current_exe().unwrap();
    format!("\"{}\"", exe.parent().unwrap().display())
}

/// Ergebniszeile einer Sonde (`PROBE <wert>`).
pub fn probe_result(out: &Out) -> String {
    out.stdout
        .lines()
        .find_map(|l| l.split_once("PROBE ").map(|(_, r)| r))
        .unwrap_or_else(|| panic!("Sonde lief nicht: {out:?}"))
        .to_owned()
}

/// Sonden für Syscalls, die die Sandbox verhindern muss. Tun nur etwas, wenn
/// `BETON_SBX_PROBE` gesetzt ist; jede gibt genau eine Zeile `PROBE <ergebnis>` aus.
#[test]
#[allow(
    unsafe_code,
    reason = "Syscalls ohne Wrapper (ptrace, clone, io_uring …)"
)]
fn probe() {
    let Ok(what) = std::env::var("BETON_SBX_PROBE") else {
        return;
    };
    let args: Vec<String> = std::env::var("BETON_SBX_PROBE_ARGS")
        .unwrap_or_default()
        .split('\u{1f}')
        .map(str::to_owned)
        .collect();
    let result = probes::run(&what, &args);
    println!("PROBE {result}");
}

mod probes {
    /// Ergebnis eines Syscalls als `ok` bzw. `err:<errno-name>`.
    #[cfg(unix)]
    fn rc(r: i64) -> String {
        if r >= 0 {
            "ok".into()
        } else {
            let e = std::io::Error::last_os_error();
            format!("err:{}", e.raw_os_error().unwrap_or(0))
        }
    }

    #[cfg(unix)]
    pub fn run(what: &str, args: &[String]) -> String {
        match what {
            "read" => match std::fs::read(&args[0]) {
                Ok(b) => format!("ok:{}", String::from_utf8_lossy(&b)),
                Err(e) => format!("err:{}", e.raw_os_error().unwrap_or(0)),
            },
            "connect" => {
                let addr: std::net::SocketAddr = args[0].parse().unwrap();
                match std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(3))
                {
                    Ok(_) => "ok".into(),
                    Err(e) => format!("err:{}", e.raw_os_error().unwrap_or(0)),
                }
            }
            "udp" => {
                let s = std::net::UdpSocket::bind("0.0.0.0:0");
                match s.and_then(|s| s.send_to(b"FIXTURE-UDP", args[0].as_str())) {
                    Ok(_) => "ok".into(),
                    Err(e) => format!("err:{}", e.raw_os_error().unwrap_or(0)),
                }
            }
            "fork" => {
                // Versucht `n` Kindprozesse zu starten, die bis zum Ende schlafen; meldet,
                // wie viele gelangen.
                let n: usize = args[0].parse().unwrap();
                let mut ok = 0usize;
                let mut failed = 0usize;
                let mut children = Vec::new();
                for _ in 0..n {
                    match std::process::Command::new("/bin/sleep")
                        .arg("30")
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                    {
                        Ok(c) => {
                            ok += 1;
                            children.push(c);
                        }
                        Err(_) => failed += 1,
                    }
                }
                // Wie eine echte Fork-Bombe: Die Prozesse bleiben eine Weile bestehen.
                std::thread::sleep(std::time::Duration::from_millis(500));
                for mut c in children {
                    let _ = c.kill();
                    let _ = c.wait();
                }
                format!("started:{ok} failed:{failed}")
            }
            "alloc" => {
                // Belegt `mb` MiB und schreibt jede Seite, damit sie wirklich Speicher kostet.
                let mb: usize = args[0].parse().unwrap();
                let mut v: Vec<Vec<u8>> = Vec::new();
                for _ in 0..mb {
                    v.push(vec![1u8; 1024 * 1024]);
                }
                format!("allocated:{}", v.len())
            }
            #[cfg(target_os = "linux")]
            other => linux::run(other, args),
            #[cfg(not(target_os = "linux"))]
            other => format!("unbekannt:{other}"),
        }
    }

    #[cfg(not(unix))]
    pub fn run(what: &str, _args: &[String]) -> String {
        format!("unbekannt:{what}")
    }

    #[cfg(target_os = "linux")]
    #[allow(unsafe_code, reason = "Syscalls ohne Wrapper")]
    mod linux {
        use super::rc;

        pub fn run(what: &str, args: &[String]) -> String {
            let num = |i: usize| -> i64 { args[i].parse().unwrap() };
            // SAFETY: Alle Aufrufe übergeben gültige oder Null-Zeiger; die Sonde prüft nur den
            // Rückgabewert.
            unsafe {
                match what {
                    "ptrace" => rc(libc::ptrace(
                        libc::PTRACE_ATTACH,
                        num(0) as libc::pid_t,
                        std::ptr::null_mut::<libc::c_void>(),
                        std::ptr::null_mut::<libc::c_void>(),
                    )),
                    "kill" => rc(libc::kill(num(0) as libc::pid_t, libc::SIGKILL) as i64),
                    "unshare_user" => rc(libc::unshare(libc::CLONE_NEWUSER) as i64),
                    "unshare_net" => rc(libc::unshare(libc::CLONE_NEWNET) as i64),
                    "clone_newuser" => {
                        let r = libc::syscall(
                            libc::SYS_clone,
                            (libc::CLONE_NEWUSER | libc::SIGCHLD) as libc::c_ulong,
                            0usize,
                            0usize,
                            0usize,
                            0usize,
                        );
                        if r == 0 {
                            libc::_exit(0);
                        }
                        rc(r)
                    }
                    "clone3" => {
                        let r = libc::syscall(libc::SYS_clone3, std::ptr::null::<u8>(), 0usize);
                        rc(r)
                    }
                    "mount_tmpfs" => {
                        let t = std::ffi::CString::new(args[0].as_str()).unwrap();
                        rc(libc::mount(
                            c"none".as_ptr(),
                            t.as_ptr(),
                            c"tmpfs".as_ptr(),
                            0,
                            std::ptr::null(),
                        ) as i64)
                    }
                    "socket" => {
                        let fd = libc::socket(num(0) as libc::c_int, libc::SOCK_RAW, 0);
                        if fd >= 0 {
                            libc::close(fd);
                        }
                        rc(fd as i64)
                    }
                    "io_uring" => {
                        let mut params = [0u8; 120];
                        rc(libc::syscall(
                            libc::SYS_io_uring_setup,
                            8u32,
                            params.as_mut_ptr(),
                        ))
                    }
                    "bpf" => rc(libc::syscall(libc::SYS_bpf, 0, 0usize, 0usize)),
                    "keyctl" => rc(libc::syscall(libc::SYS_keyctl, 0, 0usize)),
                    "tiocsti" => {
                        let c: u8 = b'x';
                        rc(libc::ioctl(0, libc::TIOCSTI, &c as *const u8) as i64)
                    }
                    "truncate" => {
                        let p = std::ffi::CString::new(args[0].as_str()).unwrap();
                        rc(libc::truncate(p.as_ptr(), 0) as i64)
                    }
                    "open_rdonly_trunc" => {
                        let p = std::ffi::CString::new(args[0].as_str()).unwrap();
                        let fd = libc::open(p.as_ptr(), libc::O_RDONLY | libc::O_TRUNC);
                        if fd >= 0 {
                            libc::close(fd);
                        }
                        rc(fd as i64)
                    }
                    #[cfg(target_arch = "x86_64")]
                    "x32_unshare" => rc(libc::syscall(
                        0x4000_0000 | libc::SYS_unshare,
                        libc::CLONE_NEWUSER as libc::c_long,
                    )),
                    #[cfg(not(target_arch = "x86_64"))]
                    "x32_unshare" => "err:38".into(),
                    "procmem_ancestors" => {
                        // /proc/<pid>/mem aller Vorfahren bis PID 1 (PID 1 und unter bwrap der
                        // innere Launcher verwalten die Sandbox).
                        let mut pid = libc::getppid();
                        let mut reached = Vec::new();
                        while pid > 0 {
                            let p = std::ffi::CString::new(format!("/proc/{pid}/mem")).unwrap();
                            let fd = libc::open(p.as_ptr(), libc::O_RDONLY);
                            if fd >= 0 {
                                libc::close(fd);
                                reached.push(pid);
                            }
                            if pid == 1 {
                                break;
                            }
                            let status = std::fs::read_to_string(format!("/proc/{pid}/status"))
                                .unwrap_or_default();
                            pid = status
                                .lines()
                                .find_map(|l| l.strip_prefix("PPid:"))
                                .and_then(|v| v.trim().parse().ok())
                                .unwrap_or(0);
                        }
                        if reached.is_empty() {
                            "err:alle".into()
                        } else {
                            format!("ok:{reached:?}")
                        }
                    }
                    "procmem_child" => {
                        // Kontrollgruppe: der Speicher eines eigenen Kindes ist erreichbar.
                        let mut c = std::process::Command::new("/bin/sleep")
                            .arg("5")
                            .spawn()
                            .unwrap();
                        let p = std::ffi::CString::new(format!("/proc/{}/mem", c.id())).unwrap();
                        let fd = libc::open(p.as_ptr(), libc::O_RDONLY);
                        if fd >= 0 {
                            libc::close(fd);
                        }
                        let _ = c.kill();
                        let _ = c.wait();
                        rc(fd as i64)
                    }
                    "uid" => format!("{}", libc::getuid()),
                    "caps" => {
                        let status = std::fs::read_to_string("/proc/self/status").unwrap();
                        status
                            .lines()
                            .filter(|l| l.starts_with("Cap"))
                            .collect::<Vec<_>>()
                            .join(",")
                    }
                    other => format!("unbekannt:{other}"),
                }
            }
        }
    }
}

/// Prozesse, deren Kommandozeile `marker` enthält (Linux: `/proc`, macOS: `ps`).
pub fn processes_with(marker: &str) -> Vec<u32> {
    #[cfg(target_os = "linux")]
    {
        let mut found = Vec::new();
        for e in std::fs::read_dir("/proc").unwrap().flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            // Argumente in `cmdline` sind durch NUL getrennt.
            if let Ok(cmd) = std::fs::read(e.path().join("cmdline"))
                && String::from_utf8_lossy(&cmd)
                    .replace('\0', " ")
                    .contains(marker)
            {
                found.push(pid);
            }
        }
        found
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out = std::process::Command::new("/bin/ps")
            .args(["-axo", "pid=,command="])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.contains(marker) && !l.contains("/bin/ps"))
            .filter_map(|l| l.split_whitespace().next()?.parse().ok())
            .collect()
    }
}

/// Wartet bis `deadline`, dass kein Prozess mit `marker` mehr existiert.
pub async fn gone_within(marker: &str, deadline: Duration) -> Result<(), Vec<u32>> {
    let start = std::time::Instant::now();
    loop {
        let left = processes_with(marker);
        if left.is_empty() {
            return Ok(());
        }
        if start.elapsed() > deadline {
            return Err(left);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
