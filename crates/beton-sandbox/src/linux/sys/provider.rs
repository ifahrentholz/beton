//! Die Provider `landlock` und `bwrap` (SBX-008, SBX-009): Probe mit Selbsttest, Start über den
//! Launcher und Verstoßmeldungen aus dessen Status-Kanal.

use std::os::fd::RawFd;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use beton_core::event::{SandboxViolation, ViolationKind};

use super::msg::{self, Ctl};
use super::util::{self, close};
use super::{cgroup, landlock, probe, seccomp};
use crate::caps::{Cap, CapSet};
use crate::child::{self, SandboxedChild, TreeGuard, Violations};
use crate::command::{CommandSpec, Stdio};
use crate::error::SandboxError;
use crate::linux::plan::{LaunchMode, LaunchPlan};
use crate::linux::{LAUNCHER_ARG, LinuxConfig, PLAN_VERSION};
use crate::provider::{ProbeReport, ProviderId, SandboxProvider};
use crate::registry::ViolationSink;
use crate::resolve::{Mask, NetMode, ResolvedPolicy, Stage};
use crate::spec::Limits;

/// Höchstdauer bis zur Rückmeldung des Launchers.
const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// Probe-Ergebnisse, die sich beide Linux-Provider teilen.
#[derive(Debug)]
struct Shared {
    cfg: LinuxConfig,
    landlock: OnceLock<ProbeReport>,
    bwrap: OnceLock<ProbeReport>,
    cgroup: OnceLock<Option<PathBuf>>,
}

/// `landlock` bzw. `bwrap`.
#[derive(Debug)]
pub struct LinuxProvider {
    id: ProviderId,
    shared: Arc<Shared>,
}

impl LinuxProvider {
    pub fn new(id: ProviderId, cfg: LinuxConfig) -> Self {
        Self {
            id,
            shared: Arc::new(Shared {
                cfg,
                landlock: OnceLock::new(),
                bwrap: OnceLock::new(),
                cgroup: OnceLock::new(),
            }),
        }
    }

    /// Beide Provider mit gemeinsamer Probe (für die Registry).
    pub fn pair(cfg: LinuxConfig) -> (Self, Self) {
        let a = Self::new(ProviderId::Landlock, cfg);
        let b = Self {
            id: ProviderId::Bwrap,
            shared: a.shared.clone(),
        };
        (a, b)
    }

    fn cfg(&self) -> &LinuxConfig {
        &self.shared.cfg
    }

    fn cgroup(&self) -> Option<PathBuf> {
        self.shared
            .cgroup
            .get_or_init(|| {
                if self.cfg().cgroup {
                    cgroup::discover()
                } else {
                    None
                }
            })
            .clone()
    }

    fn abi(&self) -> Result<u32, String> {
        let kernel = probe::landlock_abi();
        if let Some(why) = probe::landlock_missing(kernel) {
            return Err(why);
        }
        Ok(landlock::effective_abi(
            kernel.unwrap_or(0),
            self.cfg().abi_cap,
        ))
    }

    fn available(&self, id: ProviderId, abi: u32, extra: &str) -> ProbeReport {
        let mut caps: CapSet = [
            Cap::FsRead,
            Cap::FsWrite,
            Cap::Masks,
            Cap::Net,
            Cap::Env,
            Cap::Proc,
            Cap::Syscall,
        ]
        .into_iter()
        .collect();
        let mut degraded = std::collections::BTreeMap::new();
        if self.cgroup().is_some() {
            caps.insert(Cap::Resources);
        } else {
            degraded.insert(
                Cap::Resources,
                "keine cgroup-v2-Delegation (z. B. systemd-User-Delegation): limits.memory_mb \
                 ist nicht durchsetzbar; pids, CPU-Zeit und Dateigröße gelten per rlimit"
                    .to_owned(),
            );
        }
        ProbeReport {
            id,
            available: true,
            caps,
            degraded_reasons: degraded,
            unavailable_reason: None,
            version: format!(
                "Launcher v{PLAN_VERSION}, Landlock-ABI {abi}, Kernel {}, seccomp{extra}",
                util::kernel_release()
            ),
            kernel: None,
        }
    }

    fn checks(&self) -> Result<u32, String> {
        let abi = self.abi()?;
        if !seccomp::available() {
            return Err("seccomp-BPF ist im Kernel nicht verfügbar".into());
        }
        if !self.cfg().launcher.is_file() {
            return Err(format!("Launcher {} fehlt", self.cfg().launcher.display()));
        }
        Ok(abi)
    }

    fn probe_bwrap(&self) -> ProbeReport {
        self.shared
            .bwrap
            .get_or_init(|| {
                let id = ProviderId::Bwrap;
                let abi = match self.checks() {
                    Ok(a) => a,
                    Err(e) => return ProbeReport::unavailable(id, e),
                };
                let bwrap = match probe::find_bwrap(self.cfg().bwrap.as_deref()) {
                    Ok(b) => b,
                    Err(e) => return ProbeReport::unavailable(id, e),
                };
                let version = match probe::bwrap_version(&bwrap)
                    .and_then(|v| crate::linux::bwrap::check_version(&v))
                {
                    Ok(v) => v,
                    Err(e) => return ProbeReport::unavailable(id, e),
                };
                if let Err(e) = self.self_test(LaunchMode::Bwrap {
                    bwrap: bwrap.clone(),
                }) {
                    return ProbeReport::unavailable(
                        id,
                        format!("Selbsttest mit {} gescheitert: {e}", bwrap.display()),
                    );
                }
                let setuid = if probe::is_setuid(&bwrap) {
                    " (setuid)"
                } else {
                    ""
                };
                self.available(
                    id,
                    abi,
                    &format!(
                        ", bubblewrap {}.{}.{}{setuid}",
                        version.0, version.1, version.2
                    ),
                )
            })
            .clone()
    }

    fn probe_landlock(&self) -> ProbeReport {
        self.shared
            .landlock
            .get_or_init(|| {
                let id = ProviderId::Landlock;
                let abi = match self.checks() {
                    Ok(a) => a,
                    Err(e) => return ProbeReport::unavailable(id, e),
                };
                if let Err(e) = self.self_test(LaunchMode::Landlock) {
                    let why = match probe::userns_restriction(&self.cfg().launcher) {
                        Some(r) => format!("{r}; Selbsttest: {e}"),
                        None => format!("Selbsttest gescheitert: {e}"),
                    };
                    let b = self.probe_bwrap();
                    let fallback = if b.available {
                        "Fallback: bwrap".to_owned()
                    } else {
                        format!(
                            "kein Fallback (bwrap: {})",
                            b.unavailable_reason.unwrap_or_default()
                        )
                    };
                    return ProbeReport::unavailable(id, format!("{why}; {fallback}"));
                }
                self.available(id, abi, "")
            })
            .clone()
    }

    /// Selbsttest: Ein vollständiger Start, dessen Ziel prüft, dass verbotene Dateien
    /// unlesbar, der Workspace beschreibbar und neue Namespaces gesperrt sind.
    fn self_test(&self, mode: LaunchMode) -> Result<(), String> {
        let tmp = tempfile::Builder::new()
            .prefix("beton-sbx-probe-")
            .tempdir()
            .map_err(|e| format!("Temp-Verzeichnis: {e}"))?;
        let root = tmp
            .path()
            .canonicalize()
            .map_err(|e| format!("Temp-Verzeichnis: {e}"))?;
        let ws = root.join("ws");
        let read = root.join("read");
        let masked = read.join("maskiert");
        let outside = root.join("draussen");
        for d in [&ws, &masked, &outside] {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        for f in [masked.join("geheim"), outside.join("geheim")] {
            std::fs::write(&f, b"x").map_err(|e| format!("{}: {e}", f.display()))?;
        }
        let launcher = self.cfg().launcher.clone();
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let plan = LaunchPlan {
            version: PLAN_VERSION,
            mode,
            stage: Stage::Tools,
            program: launcher.clone(),
            args: vec![
                LAUNCHER_ARG.into(),
                "--probe-target".into(),
                "--read".into(),
                s(&outside.join("geheim")),
                "--read".into(),
                s(&masked.join("geheim")),
                "--write".into(),
                s(&ws.join("probe")),
            ],
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            cwd: ws.clone(),
            workspace: ws.clone(),
            scratch: ws.clone(),
            read_roots: vec![read.clone(), launcher.clone()],
            write_roots: vec![ws.clone()],
            masks: vec![Mask::Path {
                path: masked,
                source: "probe".into(),
            }],
            hidden: Vec::new(),
            git: None,
            net: NetMode::None,
            unix_sockets: Vec::new(),
            dev_ports: Vec::new(),
            read_only: Vec::new(),
            no_symlinks: Vec::new(),
            private: Vec::new(),
            limits: Limits::default(),
            abi_cap: self.cfg().abi_cap,
            cgroup: None,
            mask_file: None,
            inner: false,
        };
        let (cmd, pending) = command(&launcher, &Stdio::Null, &Stdio::Null, &Stdio::Null)
            .map_err(|e| format!("{}: {e}", launcher.display()))?;
        let mut cmd = cmd;
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("{}: {e}", launcher.display()))?;
        // Der Status-Socket bleibt offen, bis das Ziel endet: Sein Schließen hieße „Runner tot“.
        let ctl = match pending.finish(&plan) {
            Ok(fd) => fd,
            Err((e, _)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let start = Instant::now();
        let result = loop {
            match child.try_wait() {
                Ok(Some(st)) if st.success() => break Ok(()),
                Ok(Some(st)) => {
                    break Err(format!("die Sandbox erzwingt die Regeln nicht ({st})"));
                }
                Ok(None) if start.elapsed() < READY_TIMEOUT => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err("keine Antwort".into());
                }
            }
        };
        close(ctl);
        result
    }
}

/// Deskriptoren eines Starts, bis der Plan übergeben und die Rückmeldung da ist.
struct Pending {
    plan_r: RawFd,
    plan_w: RawFd,
    ctl: RawFd,
    ctl_child: RawFd,
}

impl Drop for Pending {
    fn drop(&mut self) {
        for fd in [self.plan_r, self.plan_w, self.ctl, self.ctl_child] {
            if fd >= 0 {
                close(fd);
            }
        }
    }
}

/// `launcher __sandbox-exec` ohne Environment, mit Plan-Pipe auf FD 3 und Status-Socket auf
/// FD 4. Nichts davon steht in argv.
fn command(
    launcher: &Path,
    stdin: &Stdio,
    stdout: &Stdio,
    stderr: &Stdio,
) -> std::io::Result<(std::process::Command, Pending)> {
    let (stdin, stdout, stderr) = (stdin.to_std()?, stdout.to_std()?, stderr.to_std()?);
    let (plan_r, plan_w) = util::pipe()?;
    let (ctl, ctl_child) = match util::seqpacket_pair() {
        Ok(p) => p,
        Err(e) => {
            close(plan_r);
            close(plan_w);
            return Err(e);
        }
    };
    #[expect(
        clippy::disallowed_methods,
        reason = "SBX-008: der Launcher entsteht nur hier, innerhalb von beton-sandbox"
    )]
    let mut c = std::process::Command::new(launcher);
    c.arg(LAUNCHER_ARG)
        .env_clear()
        .current_dir("/")
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr);
    // Coverage-Messung der Tests (cargo llvm-cov): nur der Launcher, nie das Ziel.
    if let Some(v) = std::env::var_os("LLVM_PROFILE_FILE") {
        c.env("LLVM_PROFILE_FILE", v);
    }
    let (a, b) = (plan_r, ctl_child);
    // SAFETY: Nach fork nur async-signal-sichere Aufrufe (fcntl, dup2, close).
    unsafe {
        c.pre_exec(move || {
            let pa = libc::fcntl(a, libc::F_DUPFD, 10);
            let pb = libc::fcntl(b, libc::F_DUPFD, 10);
            if pa < 0 || pb < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::dup2(pa, msg::PLAN_FD) < 0 || libc::dup2(pb, msg::CTL_FD) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(pa);
            libc::close(pb);
            Ok(())
        });
    }
    Ok((
        c,
        Pending {
            plan_r,
            plan_w,
            ctl,
            ctl_child,
        },
    ))
}

impl Pending {
    /// Übergibt den Plan und wartet auf `Ready`. Gibt den Status-Socket zurück.
    fn finish(mut self, plan: &LaunchPlan) -> Result<RawFd, (String, Option<i32>)> {
        close(self.plan_r);
        self.plan_r = -1;
        close(self.ctl_child);
        self.ctl_child = -1;
        let json = serde_json::to_vec(plan).map_err(|e| (e.to_string(), None))?;
        let w = util::write_all(self.plan_w, &json);
        close(self.plan_w);
        self.plan_w = -1;
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(("keine Rückmeldung des Launchers".into(), None));
            }
            let rev = util::poll(&[self.ctl], left.as_millis().min(1000) as i32)
                .map_err(|e| (e.to_string(), None))?;
            if rev[0] == 0 {
                continue;
            }
            return match msg::recv(self.ctl) {
                Ok(Some(Ctl::Ready)) => {
                    let fd = self.ctl;
                    self.ctl = -1;
                    Ok(fd)
                }
                Ok(Some(Ctl::Failed {
                    step,
                    message,
                    errno,
                })) => Err((format!("{step}: {message}"), errno)),
                Ok(Some(Ctl::Limit { .. })) => continue,
                Ok(None) => Err((
                    match w {
                        Err(e) => format!("Launcher endete ohne Rückmeldung (Plan: {e})"),
                        Ok(()) => "Launcher endete ohne Rückmeldung".into(),
                    },
                    None,
                )),
                Err(e) => Err((e.to_string(), None)),
            };
        }
    }
}

/// Liest Verstoßmeldungen bis zum Ende des Launchers.
struct LinuxGuard;

impl TreeGuard for LinuxGuard {}

fn watch(ctl: RawFd, stage: Stage, violations: Violations, sink: Option<ViolationSink>) {
    std::thread::spawn(move || {
        let mut seen: Vec<String> = Vec::new();
        while let Ok(Some(m)) = msg::recv(ctl) {
            let Ctl::Limit { target } = m else { continue };
            if seen.contains(&target) {
                continue;
            }
            seen.push(target.clone());
            let v = SandboxViolation {
                stage: stage.event(),
                kind: ViolationKind::Limit,
                target: Some(target),
                suppressed_count: 0,
            };
            if let Ok(mut list) = violations.lock() {
                list.push(v.clone());
            }
            if let Some(s) = &sink {
                s(v);
            }
        }
        close(ctl);
    });
}

impl SandboxProvider for LinuxProvider {
    fn id(&self) -> ProviderId {
        self.id
    }

    fn probe(&self) -> ProbeReport {
        let mut report = match self.id {
            ProviderId::Bwrap => self.probe_bwrap(),
            _ => self.probe_landlock(),
        };
        // SBX-010 AC1: ABI und Kernel auch für nicht verfügbare Backends.
        report.kernel = Some(crate::provider::KernelInfo {
            landlock_abi: probe::landlock_abi(),
            kernel: util::kernel_release(),
        });
        report
    }

    fn spawn(
        &self,
        policy: &ResolvedPolicy,
        cmd: CommandSpec,
    ) -> Result<SandboxedChild, SandboxError> {
        self.spawn_observed(policy, cmd, None)
    }

    fn spawn_observed(
        &self,
        policy: &ResolvedPolicy,
        cmd: CommandSpec,
        sink: Option<ViolationSink>,
    ) -> Result<SandboxedChild, SandboxError> {
        let probe = self.probe();
        crate::resolve::check_caps(&probe, policy)?;
        let mode = match self.id {
            ProviderId::Bwrap => LaunchMode::Bwrap {
                bwrap: probe::find_bwrap(self.cfg().bwrap.as_deref())
                    .map_err(SandboxError::spawn)?,
            },
            _ => LaunchMode::Landlock,
        };
        let mut plan = LaunchPlan::new(policy, &cmd, mode);
        plan.abi_cap = self.cfg().abi_cap;
        if policy.limits.memory_mb.is_some() || policy.limits.pids.is_some() {
            plan.cgroup = self.cgroup();
        }
        if let Some(g) = &policy.git {
            crate::linux::plan::ensure_git_hooks(g)
                .map_err(|e| SandboxError::spawn(format!("Git-Hooks anlegen: {e}")))?;
        }
        crate::resolve::ensure_read_only_dirs(policy)?;
        let launcher = &self.cfg().launcher;
        let (std_cmd, pending) =
            command(launcher, &cmd.stdin, &cmd.stdout, &cmd.stderr).map_err(|e| {
                SandboxError::SpawnFailed {
                    message: format!("{}: {e}", launcher.display()),
                    kind: e.kind(),
                }
            })?;
        let tokio_cmd = tokio::process::Command::from(std_cmd);
        let mut child = child::spawn(tokio_cmd).map_err(|e| SandboxError::SpawnFailed {
            message: format!("{}: {e}", launcher.display()),
            kind: e.kind(),
        })?;
        let ctl = match pending.finish(&plan) {
            Ok(fd) => fd,
            Err((e, errno)) => {
                let _ = child.start_kill();
                return Err(SandboxError::SpawnFailed {
                    message: format!("{}: {e}", self.id),
                    kind: errno.map_or(std::io::ErrorKind::Other, |n| {
                        std::io::Error::from_raw_os_error(n).kind()
                    }),
                });
            }
        };
        let violations = Violations::default();
        watch(ctl, policy.stage, violations.clone(), sink);
        Ok(child.with_guard(Box::new(LinuxGuard), violations))
    }
}
