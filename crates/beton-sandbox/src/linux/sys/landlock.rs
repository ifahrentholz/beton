//! Landlock-Regeln anwenden (SBX-008). Die Regeln entstehen plattformunabhängig in
//! [`crate::linux::plan::landlock_rules`]; hier werden sie im Mount-Namespace der Sandbox
//! geöffnet und für den aufrufenden Prozess erzwungen.
//!
//! Gehandhabt werden alle Dateisystem-Rechte der erkannten ABI außer `IOCTL_DEV` (Terminals
//! und Geräte; `TIOCSTI` sperrt seccomp). Ab ABI 4 zusätzlich TCP: `connect` nur zum
//! Relay-Port, kein `bind`. Ab ABI 6 sind Signale und abstrakte Unix-Sockets auf die eigene
//! Domain begrenzt. Ab ABI 9 sind Verbindungen zu Unix-Sockets mit Pfad nur im privaten `/tmp`
//! und im Scratch erlaubt. `CompatLevel::HardRequirement`: Fehlt etwas, scheitert der Start.

use std::path::{Path, PathBuf};

use landlock::{
    ABI, Access, AccessFs, AccessNet, BitFlags, CompatLevel, Compatible, NetPort, PathBeneath,
    PathFd, Ruleset, RulesetAttr, RulesetCreated, RulesetCreatedAttr, RulesetStatus, Scope,
};

use crate::linux::plan::Access as Acc;

/// Die genutzte ABI: die des Kernels, höchstens `cap`.
pub fn effective_abi(kernel: u32, cap: Option<u8>) -> u32 {
    cap.map_or(kernel, |c| kernel.min(u32::from(c)))
}

fn file_rights() -> BitFlags<AccessFs> {
    AccessFs::Execute
        | AccessFs::WriteFile
        | AccessFs::ReadFile
        | AccessFs::Truncate
        | AccessFs::IoctlDev
}

/// Baut den Ruleset (ohne ihn zu erzwingen). `proxy_port`: einzige erlaubte TCP-Verbindung
/// (ABI ≥ 4).
pub fn ruleset(
    abi: u32,
    rules: &[(PathBuf, Acc)],
    proxy_port: Option<u16>,
    unix_dirs: &[&Path],
) -> Result<RulesetCreated, String> {
    let lab = ABI::from(abi as i32);
    let mut handled = AccessFs::from_all(lab);
    handled.remove(AccessFs::IoctlDev);
    let read = AccessFs::from_read(lab);
    let err = |e: landlock::RulesetError| e.to_string();
    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(handled)
        .map_err(err)?;
    if abi >= 4 {
        ruleset = ruleset
            .handle_access(AccessNet::BindTcp | AccessNet::ConnectTcp)
            .map_err(err)?;
    }
    if abi >= 6 {
        ruleset = ruleset
            .scope(Scope::AbstractUnixSocket | Scope::Signal)
            .map_err(err)?;
    }
    let resolve_unix = handled.contains(AccessFs::ResolveUnix);
    let mut created = ruleset.create().map_err(err)?;
    for (path, acc) in rules {
        let Ok(meta) = std::fs::metadata(path) else {
            continue;
        };
        let mut rights = match acc {
            Acc::ListDir => BitFlags::from(AccessFs::ReadDir),
            Acc::ReadExec => read,
            Acc::Device => {
                AccessFs::ReadFile | AccessFs::WriteFile | AccessFs::ReadDir | AccessFs::Truncate
            }
            Acc::Write => {
                let mut w = handled;
                w.remove(AccessFs::ResolveUnix);
                w
            }
        } & handled;
        if !meta.is_dir() {
            rights &= file_rights();
        }
        if rights.is_empty() {
            continue;
        }
        let fd = PathFd::new(path).map_err(|e| format!("{}: {e}", path.display()))?;
        created = created
            .add_rule(PathBeneath::new(fd, rights))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    if resolve_unix {
        for d in unix_dirs {
            if let Ok(fd) = PathFd::new(d) {
                created = created
                    .add_rule(PathBeneath::new(fd, AccessFs::ResolveUnix))
                    .map_err(err)?;
            }
        }
    }
    if abi >= 4
        && let Some(port) = proxy_port
    {
        created = created
            .add_rule(NetPort::new(port, AccessNet::ConnectTcp))
            .map_err(err)?;
    }
    Ok(created)
}

/// Erzwingt einen Ruleset für den aufrufenden Thread (und alles, was er startet). Nur
/// vollständig erzwungen gilt als Erfolg.
pub fn enforce(created: RulesetCreated) -> Result<(), String> {
    let status = created.restrict_self().map_err(|e| e.to_string())?;
    if status.ruleset != RulesetStatus::FullyEnforced {
        return Err(format!(
            "Landlock nicht vollständig erzwungen: {:?}",
            status.ruleset
        ));
    }
    Ok(())
}

/// Wendet die Regeln an.
pub fn apply(
    abi: u32,
    rules: &[(PathBuf, Acc)],
    proxy_port: Option<u16>,
    unix_dirs: &[&Path],
) -> Result<(), String> {
    enforce(ruleset(abi, rules, proxy_port, unix_dirs)?)
}

/// Sperrt jeden Dateizugriff (für Hilfsprozesse wie Relay und Connector) und erlaubt höchstens
/// eine TCP-Verbindung zu `connect_port`.
pub fn deny_all(abi: u32, connect_port: Option<u16>) -> Result<(), String> {
    enforce(ruleset(abi, &[], connect_port, &[])?)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn kernel_abi() -> u32 {
        super::super::probe::landlock_abi().expect("Landlock (Kernel >= 5.19) für die Tests")
    }

    #[test]
    fn sbx_008_rulesets_build_for_the_kernel_and_for_abi_2() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f");
        std::fs::write(&file, "x").unwrap();
        let rules = vec![
            (tmp.path().to_path_buf(), Acc::Write),
            (file, Acc::ReadExec),
            ("/dev/null".into(), Acc::Device),
            ("/dev".into(), Acc::ListDir),
            (tmp.path().join("fehlt"), Acc::ReadExec),
        ];
        for abi in [2, effective_abi(kernel_abi(), None)] {
            ruleset(abi, &rules, Some(8080), &[tmp.path()]).unwrap();
        }
        assert_eq!(effective_abi(6, Some(2)), 2);
        assert_eq!(effective_abi(3, Some(5)), 3);
    }

    #[test]
    fn sbx_008_landlock_restricts_the_calling_thread() {
        let abi = kernel_abi();
        let tmp = tempfile::tempdir().unwrap();
        let inside = tmp.path().join("innen");
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::write(inside.join("a"), "x").unwrap();
        std::fs::write(outside.path().join("b"), "x").unwrap();
        let (inside2, outside2) = (inside.clone(), outside.path().to_path_buf());
        // Landlock gilt nur für den Thread, der es anwendet (ohne TSYNC).
        let r = std::thread::spawn(move || {
            apply(abi, &[(inside2.clone(), Acc::ReadExec)], None, &[]).unwrap();
            (
                std::fs::read(inside2.join("a")).is_ok(),
                std::fs::read(outside2.join("b")).is_err(),
                std::fs::write(inside2.join("neu"), "x").is_err(),
            )
        })
        .join()
        .unwrap();
        assert_eq!(r, (true, true, true));
        // Der Test-Thread selbst ist frei.
        assert!(std::fs::read(outside.path().join("b")).is_ok());
        std::thread::spawn(move || deny_all(abi, None).unwrap())
            .join()
            .unwrap();
    }
}
