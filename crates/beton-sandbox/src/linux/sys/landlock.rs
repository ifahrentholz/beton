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
    PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus, Scope,
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

/// Wendet die Regeln an. `proxy_port`: einzige erlaubte TCP-Verbindung (ABI ≥ 4).
pub fn apply(
    abi: u32,
    rules: &[(PathBuf, Acc)],
    proxy_port: Option<u16>,
    unix_dirs: &[&Path],
) -> Result<(), String> {
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
    let status = created.restrict_self().map_err(err)?;
    if status.ruleset != RulesetStatus::FullyEnforced {
        return Err(format!(
            "Landlock nicht vollständig erzwungen: {:?}",
            status.ruleset
        ));
    }
    Ok(())
}

/// Sperrt jeden Dateizugriff (für Hilfsprozesse wie Relay und Connector) und erlaubt höchstens
/// eine TCP-Verbindung zu `connect_port`.
pub fn deny_all(abi: u32, connect_port: Option<u16>) -> Result<(), String> {
    let lab = ABI::from(abi as i32);
    let mut handled = AccessFs::from_all(lab);
    handled.remove(AccessFs::IoctlDev);
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
    let mut created = ruleset.create().map_err(err)?;
    if abi >= 4
        && let Some(port) = connect_port
    {
        created = created
            .add_rule(NetPort::new(port, AccessNet::ConnectTcp))
            .map_err(err)?;
    }
    let status = created.restrict_self().map_err(err)?;
    if status.ruleset != RulesetStatus::FullyEnforced {
        return Err(format!("Landlock: {:?}", status.ruleset));
    }
    Ok(())
}
