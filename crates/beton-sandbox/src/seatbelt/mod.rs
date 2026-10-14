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
    use crate::child::{self, SandboxedChild};
    use crate::command::CommandSpec;
    use crate::error::SandboxError;
    use crate::provider::{ProbeReport, ProviderId, SandboxProvider};
    use crate::resolve::ResolvedPolicy;
    use crate::spec::env_denied;

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
            return ProbeReport::unavailable(
                ProviderId::Seatbelt,
                format!("{SANDBOX_EXEC} fehlt"),
            );
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
                    "Ressourcenlimits folgen mit SBX-013".to_owned(),
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
            let _ = (policy, cmd, profile::render, env_denied, child::spawn);
            todo!("SBX-007")
        }
    }
}
