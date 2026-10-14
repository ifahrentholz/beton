//! bubblewrap-Fallback (SBX-009): Version prüfen und die Argumente für `bwrap` bauen.
//!
//! bwrap stellt Namespaces und Mounts her (ohne eigene User-Namespaces, wenn es setuid läuft
//! oder AppArmor es erlaubt); Landlock, seccomp, Limits und der Relay entstehen danach im
//! eigenen Launcher innerhalb von bwrap. Die Argumente enthalten nur Pfade und Schalter: Das
//! Environment des Ziels geht mit dem Plan über eine Pipe an den Launcher, nie per
//! `--setenv` (AC3), und bwrap selbst startet mit leerem Environment.

use std::path::Path;

use super::plan::{LaunchPlan, MountOp, mount_ops, sort_ops};
use crate::error::SandboxError;

/// Liest die Version aus `bwrap --version` (z. B. `bubblewrap 0.9.0`).
pub fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let word = text.split_whitespace().nth(1)?;
    let mut parts = word.trim().split('.').map(|p| {
        p.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse::<u32>()
            .ok()
    });
    let major = parts.next()??;
    let minor = parts.next().flatten().unwrap_or(0);
    let patch = parts.next().flatten().unwrap_or(0);
    Some((major, minor, patch))
}

/// Ist die Version mindestens [`super::BWRAP_MIN`]? Sonst ein Grund mit Versionshinweis.
pub fn check_version(text: &str) -> Result<(u32, u32, u32), String> {
    let (a, b, c) = super::BWRAP_MIN;
    let Some(v) = parse_version(text) else {
        return Err(format!(
            "bwrap meldet keine lesbare Version ({:?}); verlangt ist bubblewrap ≥ {a}.{b}",
            text.trim()
        ));
    };
    if v < super::BWRAP_MIN {
        return Err(format!(
            "bubblewrap {}.{}.{} ist zu alt; verlangt ist ≥ {a}.{b}.{c}",
            v.0, v.1, v.2
        ));
    }
    Ok(v)
}

fn path_arg(p: &Path) -> Result<String, SandboxError> {
    p.to_str().map(str::to_owned).ok_or_else(|| {
        SandboxError::invalid("paths", format!("{} ist kein gültiges UTF-8", p.display()))
    })
}

/// Die Argumente für `bwrap` (ohne Programmnamen). Nach `--` folgen nur der Launcher und
/// [`super::LAUNCHER_ARG`]; das Ziel steht im Plan.
pub fn args(plan: &LaunchPlan, launcher: &Path) -> Result<Vec<String>, SandboxError> {
    let mut a: Vec<String> = [
        "--die-with-parent",
        "--unshare-user-try",
        "--unshare-ipc",
        "--unshare-pid",
        "--unshare-net",
        "--unshare-uts",
        "--unshare-cgroup-try",
        "--hostname",
        "beton-sandbox",
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let (mut ops, _) = mount_ops(plan)?;
    // Unter `--ro-bind / /` braucht jede Schreib-Wurzel einen eigenen beschreibbaren Mount.
    for w in &plan.write_roots {
        let present = ops
            .iter()
            .any(|o| matches!(o, MountOp::Rebind { path, .. } if path == w));
        if !present {
            ops.push(MountOp::Rebind {
                path: w.clone(),
                write: true,
            });
        }
    }
    sort_ops(&mut ops);
    let mask_file = plan.mask_file.as_deref();
    for op in &ops {
        match op {
            // `--dev` bringt ein eigenes `/dev/shm` mit.
            MountOp::Tmpfs { path, .. } if path == Path::new("/dev/shm") => {}
            MountOp::Tmpfs { path, mode } => {
                a.extend([
                    "--perms".into(),
                    format!("{mode:04o}"),
                    "--tmpfs".into(),
                    path_arg(path)?,
                ]);
            }
            MountOp::MaskDir { path } => {
                let p = path_arg(path)?;
                a.extend([
                    "--perms".into(),
                    "0000".into(),
                    "--tmpfs".into(),
                    p.clone(),
                    "--remount-ro".into(),
                    p,
                ]);
            }
            MountOp::MaskFile { path } => {
                let src = mask_file.ok_or_else(|| {
                    SandboxError::spawn("bwrap: keine Maskendatei für Datei-Masken")
                })?;
                a.extend(["--ro-bind".into(), path_arg(src)?, path_arg(path)?]);
            }
            MountOp::Rebind { path, write } => {
                let p = path_arg(path)?;
                let flag = if *write { "--bind" } else { "--ro-bind" };
                a.extend([flag.into(), p.clone(), p]);
            }
            MountOp::ReadOnly { path } => {
                let p = path_arg(path)?;
                a.extend(["--ro-bind".into(), p.clone(), p]);
            }
            MountOp::Pin { path } => {
                let p = path_arg(path)?;
                a.extend(["--bind".into(), p.clone(), p]);
            }
        }
    }
    a.push("--".into());
    a.push(path_arg(launcher)?);
    a.push(super::LAUNCHER_ARG.into());
    Ok(a)
}
