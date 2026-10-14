//! macOS-Backend: Seatbelt über `/usr/bin/sandbox-exec` (SBX-007), deny-default.
//!
//! Das Profil entsteht pro Start aus dem versionierten Template ([`profile`]) und geht als
//! Argument (`-p`) an `sandbox-exec`, die Pfade als `-D KEY=VALUE`. Bewusst keine Profildatei
//! im Scratch: Den Scratch kann eine laufende Sandbox beschreiben und so das Profil des
//! nächsten Starts austauschen (TOCTOU).
//!
//! Die Probe prüft die Funktion selbst (ein verbotener Lesezugriff muss scheitern, ein
//! erlaubter Start gelingen); erst dann gilt das Backend als verfügbar. Verschachtelte
//! Sandboxes sind mit Seatbelt nicht möglich und führen zu `unavailable`.

pub mod profile;

use std::sync::Arc;

use crate::provider::SandboxProvider;

/// Pfad von `sandbox-exec` (fest, nie über `PATH`).
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Der Seatbelt-Provider dieser Plattform.
pub fn provider() -> Arc<dyn SandboxProvider> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::SeatbeltProvider::default())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(crate::registry::UnavailableProvider {
            id: crate::provider::ProviderId::Seatbelt,
            reason: "nur unter macOS".into(),
        })
    }
}

#[cfg(target_os = "macos")]
pub mod macos {
    use std::path::Path;
    use std::sync::OnceLock;

    use super::SANDBOX_EXEC;
    use super::profile::{self, PROFILE_VERSION};
    use crate::caps::{Cap, CapSet};
    use crate::child::Violations;
    use crate::child::{self, SandboxedChild};
    use crate::command::CommandSpec;
    use crate::error::SandboxError;
    use crate::proctree::Tracker;
    use crate::provider::{ProbeReport, ProviderId, SandboxProvider};
    use crate::registry::ViolationSink;
    use crate::resolve::ResolvedPolicy;
    use crate::spec::{Limits, env_denied};

    /// rlimits des Baums (SBX-013). `RLIMIT_NPROC` zählt unter macOS alle Prozesse des
    /// Benutzers; das Limit gilt deshalb zusätzlich zu den schon laufenden. Weil andere Prozesse
    /// des Benutzers währenddessen enden können, bleibt ein Abstand (höchstens 8) zum Limit.
    #[allow(unsafe_code, reason = "setrlimit nach fork gibt es nur über libc")]
    fn limits(c: &mut tokio::process::Command, l: &Limits) {
        let mut set: Vec<(libc::c_int, u64)> = vec![(libc::RLIMIT_CORE, 0)];
        if let Some(p) = l.pids {
            // SAFETY: getuid ohne Seiteneffekte.
            let uid = unsafe { libc::getuid() };
            set.push((
                libc::RLIMIT_NPROC,
                crate::proctree::count_of_uid(uid) + u64::from(crate::proctree::budget(p)),
            ));
        }
        if let Some(s) = l.cpu_seconds {
            set.push((libc::RLIMIT_CPU, s));
        }
        if let Some(mb) = l.file_size_mb {
            set.push((libc::RLIMIT_FSIZE, mb.saturating_mul(1024 * 1024)));
        }
        // SAFETY: Nach fork nur setrlimit (async-signal-sicher) mit vorab berechneten Werten.
        unsafe {
            c.pre_exec(move || {
                for (res, v) in &set {
                    let r = libc::rlimit {
                        rlim_cur: *v,
                        rlim_max: *v,
                    };
                    if libc::setrlimit(*res, &r) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
    }

    /// Seatbelt (`sandbox-exec`).
    #[derive(Debug, Default)]
    pub struct SeatbeltProvider {
        probe: OnceLock<ProbeReport>,
    }

    fn macos_version() -> String {
        let plist = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
            .unwrap_or_default();
        plist
            .split("<key>ProductVersion</key>")
            .nth(1)
            .and_then(|rest| rest.split("<string>").nth(1))
            .and_then(|rest| rest.split("</string>").next())
            .unwrap_or("unbekannt")
            .trim()
            .to_owned()
    }

    /// Selbsttest: Ein Start unter deny-default gelingt, ein verbotener Lesezugriff scheitert.
    #[expect(
        clippy::disallowed_methods,
        reason = "SBX-007: Funktionsprobe von sandbox-exec innerhalb von beton-sandbox"
    )]
    fn self_test() -> Result<(), String> {
        let dir = tempfile::tempdir().map_err(|e| format!("Temp-Verzeichnis: {e}"))?;
        let secret = dir.path().join("probe");
        std::fs::write(&secret, b"x").map_err(|e| format!("Probe-Datei: {e}"))?;
        let base = "(version 1)(deny default)(allow process-exec)(allow process-fork)\
                    (allow file-read* (literal \"/\") (subpath \"/usr\") (subpath \"/bin\") \
                    (subpath \"/System\") (subpath \"/private/var/select\"))";
        let run = |program: &str, arg: Option<&Path>| {
            let mut c = std::process::Command::new(SANDBOX_EXEC);
            c.arg("-p").arg(base).arg("--").arg(program);
            if let Some(a) = arg {
                c.arg(a);
            }
            c.env_clear()
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
        };
        match run("/usr/bin/true", None) {
            Ok(s) if s.success() => {}
            Ok(s) => {
                return Err(format!(
                    "sandbox-exec startet nicht ({s}); läuft beton selbst in einer Sandbox?"
                ));
            }
            Err(e) => return Err(format!("{SANDBOX_EXEC}: {e}")),
        }
        match run("/bin/cat", Some(&secret)) {
            Ok(s) if !s.success() => Ok(()),
            Ok(_) => Err("sandbox-exec erzwingt deny-default nicht".into()),
            Err(e) => Err(format!("{SANDBOX_EXEC}: {e}")),
        }
    }

    fn probe_now() -> ProbeReport {
        if !Path::new(SANDBOX_EXEC).is_file() {
            return ProbeReport::unavailable(ProviderId::Seatbelt, format!("{SANDBOX_EXEC} fehlt"));
        }
        if let Err(reason) = self_test() {
            return ProbeReport::unavailable(ProviderId::Seatbelt, reason);
        }
        let caps: CapSet = [
            Cap::FsRead,
            Cap::FsWrite,
            Cap::Masks,
            Cap::Net,
            Cap::Env,
            Cap::Proc,
        ]
        .into_iter()
        .collect();
        ProbeReport {
            id: ProviderId::Seatbelt,
            available: true,
            caps,
            degraded_reasons: [
                (
                    Cap::Syscall,
                    "Seatbelt filtert keine Syscalls (seccomp gibt es nur unter Linux)".to_owned(),
                ),
                (
                    Cap::Resources,
                    "macOS kennt kein Speicherlimit für Prozessbäume (memory_mb); pids, CPU-Zeit \
                     und Dateigröße gelten per rlimit"
                        .to_owned(),
                ),
            ]
            .into_iter()
            .collect(),
            unavailable_reason: None,
            version: format!("profile v{PROFILE_VERSION}, macOS {}", macos_version()),
        }
    }

    impl SandboxProvider for SeatbeltProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Seatbelt
        }

        fn probe(&self) -> ProbeReport {
            self.probe.get_or_init(probe_now).clone()
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
            crate::resolve::check_caps(&self.probe(), policy)?;
            let profile = profile::render(policy)?;
            let mut c = child::command(Path::new(SANDBOX_EXEC), &cmd);
            c.arg("-p").arg(&profile.text);
            for (k, v) in &profile.params {
                c.arg("-D").arg(format!("{k}={v}"));
            }
            c.arg("--").arg(&cmd.program).args(&cmd.args);
            // Nie erben: explizites Environment der Policy plus erlaubte Variablen des Aufrufs.
            c.env_clear().envs(&policy.env);
            for (k, v) in &cmd.env.set {
                if !env_denied(k) {
                    c.env(k, v);
                }
            }
            for k in &cmd.env.remove {
                c.env_remove(k);
            }
            c.current_dir(cmd.cwd.as_deref().unwrap_or(&policy.workspace));
            limits(&mut c, &policy.limits);
            let child = child::spawn(c).map_err(|e| SandboxError::SpawnFailed {
                message: format!("{SANDBOX_EXEC}: {e}"),
                kind: e.kind(),
            })?;
            let violations = Violations::default();
            let Some(pid) = child.id() else {
                return Ok(child);
            };
            let tracker = Tracker::start(
                pid,
                policy.stage.event(),
                policy.limits.pids.map(crate::proctree::budget),
                violations.clone(),
                sink,
            );
            Ok(child.with_guard(Box::new(tracker), violations))
        }
    }
}
