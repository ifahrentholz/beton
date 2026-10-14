//! Kernel-Fähigkeiten für die Probe (SBX-008, SBX-010, OBS-005): Landlock-ABI, seccomp,
//! unprivilegierte User-Namespaces (sysctls, AppArmor) und bubblewrap.

use std::path::{Path, PathBuf};

/// Landlock-ABI des Kernels (`None`: Landlock fehlt oder ist nicht aktiv).
pub fn landlock_abi() -> Option<u32> {
    const LANDLOCK_CREATE_RULESET_VERSION: libc::c_uint = 1;
    // SAFETY: landlock_create_ruleset(NULL, 0, VERSION) liest und schreibt keinen Speicher.
    let r = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<libc::c_void>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    (r > 0).then_some(r as u32)
}

/// Grund für eine fehlende oder zu alte Landlock-ABI (`None`: reicht).
pub fn landlock_missing(abi: Option<u32>) -> Option<String> {
    match abi {
        None => Some(
            "Landlock ist nicht aktiv (Kernel < 5.13 oder Landlock fehlt in `lsm=`); verlangt ist \
             Kernel ≥ 5.19 mit Landlock-ABI ≥ 2"
                .into(),
        ),
        Some(1) => Some(
            "Landlock-ABI 1 (Kernel 5.13–5.18) reicht nicht; verlangt ist Kernel ≥ 5.19 mit \
             Landlock-ABI ≥ 2"
                .into(),
        ),
        Some(_) => None,
    }
}

/// Wert eines sysctls (`None`, wenn es ihn nicht gibt).
pub fn sysctl(name: &str) -> Option<String> {
    std::fs::read_to_string(format!("/proc/sys/{}", name.replace('.', "/")))
        .ok()
        .map(|s| s.trim().to_owned())
}

/// Warum unprivilegierte User-Namespaces nicht nutzbar sein könnten (laut sysctls).
pub fn userns_restriction(launcher: &Path) -> Option<String> {
    if sysctl("kernel.unprivileged_userns_clone").as_deref() == Some("0") {
        return Some(
            "unprivilegierte User-Namespaces sind abgeschaltet \
             (kernel.unprivileged_userns_clone=0)"
                .into(),
        );
    }
    if sysctl("user.max_user_namespaces").as_deref() == Some("0") {
        return Some("User-Namespaces sind abgeschaltet (user.max_user_namespaces=0)".into());
    }
    if sysctl("kernel.apparmor_restrict_unprivileged_userns").as_deref() == Some("1") {
        return Some(format!(
            "AppArmor schränkt unprivilegierte User-Namespaces ein \
             (kernel.apparmor_restrict_unprivileged_userns=1), und für {} gilt kein \
             AppArmor-Profil mit `userns`-Recht (Paketprofil: deploy/apparmor/beton)",
            launcher.display()
        ));
    }
    None
}

/// Der zu nutzende bwrap (ausdrücklich angegeben oder an einem festen Ort).
pub fn find_bwrap(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        return if p.is_file() {
            Ok(p.to_path_buf())
        } else {
            Err(format!("bwrap fehlt ({})", p.display()))
        };
    }
    crate::linux::BWRAP_PATHS
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .ok_or_else(|| {
            format!(
                "bwrap ist nicht installiert (gesucht: {})",
                crate::linux::BWRAP_PATHS.join(", ")
            )
        })
}

/// `bwrap --version`.
#[expect(
    clippy::disallowed_methods,
    reason = "SBX-009: Versionsabfrage von bwrap innerhalb von beton-sandbox (Stufe 0)"
)]
pub fn bwrap_version(bwrap: &Path) -> Result<String, String> {
    let out = std::process::Command::new(bwrap)
        .arg("--version")
        .env_clear()
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", bwrap.display()))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Ist `bwrap` setuid-root?
pub fn is_setuid(p: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata(p).is_ok_and(|m| m.uid() == 0 && m.mode() & 0o4000 != 0)
}
