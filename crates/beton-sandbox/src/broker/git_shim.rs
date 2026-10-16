//! `git`-Shim der Stufe 1 (SBX-003, #311): Im Stufenmodell ruft die Vendor-CLI `git` selbst in
//! Stufe 1 auf (z. B. Claude Code für den Repo-Status). Unter Linux existiert ein in Stufe 2
//! angelegtes `commondir`/`gitdir` kurz, bis der Launcher es entfernt (#209); ein `git` der
//! Stufe 1 in diesem Fenster nähme ein fremdes Git-Verzeichnis samt Konfiguration und Hooks.
//!
//! Der Runner (Stufe 0) legt deshalb `<session>/exec/git/git` an (Symlink auf beton, nur für
//! Stufe 0 beschreibbar) und stellt das Verzeichnis vorn in den `PATH` der Stufe 1. Vor jedem
//! Start fragt der Shim den Exec-Broker ([`Op::GitPreflight`]), der die Git-Steuerdateien mit
//! dem Schnappschuss des Runners vergleicht. Nur bei Erfolg ersetzt er sich durch das echte git
//! – den absoluten Pfad, den Stufe 0 beim Start ermittelt hat ([`ENV_GIT`]), nie über eine
//! Suche im `PATH` –, mit unveränderten Argumenten und unveränderter Umgebung. Sonst endet er
//! mit 126 und einer Meldung auf stderr, ohne git zu starten; der Runner beendet die Session.
//! Fehlt etwas (Pfad, Broker, Token), startet git ebenfalls nicht (fail closed).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::client::CANNOT_EXECUTE;
#[cfg(unix)]
use super::proto::{Op, Reply, Request, VERSION};

/// Absoluter Pfad des echten `git` im Environment der Stufe 1 (von Stufe 0 beim Start
/// ermittelt; als `BETON_*` weder per `env.set` noch über den Exec-Broker änderbar).
pub const ENV_GIT: &str = "BETON_GIT";
/// Aufrufname des Shims.
pub const NAME: &str = "git";
/// Verzeichnis des Shims unterhalb des Broker-Verzeichnisses (`<session>/exec`).
pub const DIR: &str = "git";

/// Das erste ausführbare `git` in `path` außerhalb von `exclude` (Verzeichnis des Shims);
/// Stufe 0 ruft das beim Start mit dem `PATH` der Stufe 1 auf. Nur absolute Einträge.
pub fn find_real_git(path: &str, exclude: &Path) -> Option<PathBuf> {
    path.split(':')
        .filter(|d| d.starts_with('/'))
        .map(Path::new)
        .filter(|d| !d.starts_with(exclude))
        .map(|d| d.join(NAME))
        .find(|p| executable(p))
}

fn executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Echtes git aus [`ENV_GIT`]: absolut und nicht der Shim selbst.
fn real_git() -> Result<PathBuf, String> {
    let real = std::env::var_os(ENV_GIT)
        .map(PathBuf::from)
        .ok_or_else(|| format!("kein echtes git ({ENV_GIT} fehlt)"))?;
    if !real.is_absolute() {
        return Err(format!("{ENV_GIT} ist kein absoluter Pfad"));
    }
    let me = std::env::current_exe().and_then(|p| p.canonicalize()).ok();
    if me.is_some() && real.canonicalize().ok() == me {
        return Err(format!("{ENV_GIT} zeigt auf den Shim selbst"));
    }
    Ok(real)
}

/// Fragt den Exec-Broker, ob git jetzt starten darf.
#[cfg(unix)]
fn preflight() -> Result<(), String> {
    use std::os::fd::AsRawFd as _;
    let (Some(socket), Ok(token)) = (
        std::env::var_os(super::ENV_SOCKET),
        std::env::var(super::ENV_TOKEN),
    ) else {
        return Err("kein Exec-Broker (BETON_EXEC_SOCKET, BETON_EXEC_TOKEN fehlen)".into());
    };
    // Der Broker erwartet drei Deskriptoren; die Vorabprüfung braucht keine.
    let null = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
        .map_err(|e| format!("/dev/null: {e}"))?;
    let fd = null.as_raw_fd();
    let req = Request {
        v: VERSION,
        op: Op::GitPreflight,
        token,
        argv: Vec::new(),
        cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
        env_overrides: Default::default(),
        tty: false,
        cols: None,
        rows: None,
    };
    match super::client::request(Path::new(&socket), &req, [fd, fd, fd]) {
        Ok(Reply::Allowed) => Ok(()),
        Ok(Reply::Error { code, message }) => Err(format!("{code}: {message}")),
        Ok(_) => Err("unerwartete Antwort des Exec-Brokers".into()),
        Err(e) => Err(format!("Exec-Broker nicht erreichbar: {e}")),
    }
}

/// Ersetzt den Prozess durch das echte git; kehrt nur bei einem Fehler zurück.
#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "der Shim ersetzt sich in Stufe 1 per exec durch das echte git (kein neuer Prozess)"
)]
fn exec(real: &Path, argv0: &OsString, args: Vec<OsString>) -> String {
    use std::os::unix::process::CommandExt as _;
    // Erbt Environment, Arbeitsverzeichnis und Deskriptoren; setzt SIGPIPE wie beim direkten
    // Start auf den Standard zurück.
    let err = std::process::Command::new(real)
        .arg0(argv0)
        .args(args)
        .exec();
    format!("{} nicht startbar: {err}", real.display())
}

/// Führt den Shim aus: `argv0` und `args` gehen unverändert an das echte git. Liefert nur bei
/// einer Ablehnung oder einem Fehler (126).
pub fn run(argv0: &OsString, args: Vec<OsString>) -> i32 {
    let fail = |why: String| {
        eprintln!("beton: git nicht gestartet (Sandbox, SBX-003): {why}");
        CANNOT_EXECUTE
    };
    let real = match real_git() {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    #[cfg(unix)]
    {
        if let Err(e) = preflight() {
            return fail(e);
        }
        fail(exec(&real, argv0, args))
    }
    #[cfg(not(unix))]
    {
        let _ = (real, argv0, args);
        fail("Exec-Broker gibt es unter Windows erst mit SBX-016 (M5)".into())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn sbx_003_real_git_is_the_first_executable_outside_the_shim_dir() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let mk = |dir: &str, mode: u32| {
            let d = root.join(dir);
            std::fs::create_dir_all(&d).unwrap();
            let g = d.join("git");
            std::fs::write(&g, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&g, std::fs::Permissions::from_mode(mode)).unwrap();
            g
        };
        let shim = mk("shim", 0o755);
        let plain = mk("plain", 0o644);
        let real = mk("real", 0o755);
        let path = format!(
            "relativ:{}:{}:{}:{}",
            shim.parent().unwrap().display(),
            root.join("leer").display(),
            plain.parent().unwrap().display(),
            real.parent().unwrap().display()
        );
        assert_eq!(
            find_real_git(&path, shim.parent().unwrap()),
            Some(real.clone())
        );
        assert_eq!(find_real_git("", shim.parent().unwrap()), None);
        assert_eq!(
            find_real_git(
                &shim.parent().unwrap().display().to_string(),
                shim.parent().unwrap()
            ),
            None
        );
    }
}
