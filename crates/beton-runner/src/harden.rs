//! Prozess-Härtung von Daemon und Runner gegen Speicherabbilder (#295).
//!
//! Secret-Werte liegen in Stufe 0 auch in Puffern, die sich nicht nullen lassen
//! (`serde_json::Value`, Tunnel-Frames, hyper-Puffer); `zeroize` ist dort nur best effort.
//! Damit sie nicht in einem Core-Dump auf der Platte landen, setzen Daemon und Runner
//! `RLIMIT_CORE` auf 0; der Daemon ist unter Linux zusätzlich nicht dumpable
//! (`PR_SET_DUMPABLE`), sodass auch andere Prozesse desselben Users seinen Speicher nicht über
//! `/proc/<pid>/mem` oder ptrace lesen.

/// Keine Core-Dumps (weiches und hartes Limit 0); vererbt sich auf Kindprozesse.
pub fn no_core_dumps() -> std::io::Result<()> {
    #[cfg(unix)]
    rustix::process::setrlimit(
        rustix::process::Resource::Core,
        rustix::process::Rlimit {
            current: Some(0),
            maximum: Some(0),
        },
    )?;
    Ok(())
}

/// Prozess nicht dumpable (nur Linux; sonst ohne Wirkung). Gilt bis zum nächsten `execve`.
pub fn not_dumpable() -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #295: Nach der Härtung erzeugt der Prozess keine Core-Dumps und ist unter Linux nicht
    /// dumpable.
    #[test]
    fn prx_006_processes_write_no_core_dumps() {
        no_core_dumps().unwrap();
        #[cfg(unix)]
        {
            let limit = rustix::process::getrlimit(rustix::process::Resource::Core);
            assert_eq!(limit.current, Some(0));
            assert_eq!(limit.maximum, Some(0));
        }
        not_dumpable().unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(
            rustix::process::dumpable_behavior().unwrap(),
            rustix::process::DumpableBehavior::NotDumpable
        );
    }
}
