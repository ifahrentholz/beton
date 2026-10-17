//! Prozesse am PTY unter Sandbox (SBX-017, SES-019 AC3): Das PTY hält beton in Stufe 0
//! (`beton-pty`), der Kindprozess startet über `SandboxProvider::spawn` mit dem Slave als
//! Controlling Terminal. Er hat dieselben Grenzen wie jedes Tool in Stufe 2; Fenstergröße und
//! Ctrl+C erreichen ihn, andere Terminals des Benutzers nicht.
//!
//! Alle Homes und Secrets sind Fixtures in Temp-Verzeichnissen.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]
#![cfg(any(target_os = "linux", target_os = "macos"))]

mod support;

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beton_pty::Pty;
use beton_sandbox::{
    BackendChoice, CommandSpec, Registry, SandboxProvider, SandboxedChild, Stage, Tty,
};
use support::*;

/// Das Standard-Backend dieser Plattform (macOS: Seatbelt, Linux: Landlock bzw. bwrap).
fn provider() -> Arc<dyn SandboxProvider> {
    #[cfg(target_os = "linux")]
    let registry = Registry::with_launcher(std::path::Path::new(env!(
        "CARGO_BIN_EXE_beton-sandbox-exec"
    )));
    #[cfg(not(target_os = "linux"))]
    let registry = Registry::platform();
    let p = registry.select(BackendChoice::Auto).unwrap();
    let probe = p.probe();
    assert!(probe.available, "{probe:?}");
    p
}

/// Ein Terminal mit laufendem Kind: Ausgabe sammelt ein Lese-Thread.
struct Term {
    pty: Pty,
    out: Arc<Mutex<Vec<u8>>>,
    input: Box<dyn Write + Send>,
    child: SandboxedChild,
}

impl Term {
    fn start(
        p: &dyn SandboxProvider,
        policy: &beton_sandbox::ResolvedPolicy,
        script: &str,
    ) -> Self {
        let mut pty = Pty::open(80, 24).unwrap();
        let slave = pty.take_slave().unwrap();
        let tty = Tty::new(Arc::new(slave), pty.slave_path().to_path_buf());
        let cmd = CommandSpec::new("/bin/sh")
            .args(["-c", script])
            .cwd(&policy.workspace)
            .tty(tty);
        let child = p.spawn(policy, cmd).unwrap();
        let out = Arc::new(Mutex::new(Vec::new()));
        let mut reader = pty.reader().unwrap();
        let sink = out.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                sink.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        let input = pty.writer().unwrap();
        Self {
            pty,
            out,
            input,
            child,
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.out.lock().unwrap()).into_owned()
    }

    async fn wait_for(&self, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let t = self.text();
            if t.contains(needle) {
                return t;
            }
            assert!(Instant::now() < deadline, "`{needle}` fehlt in: {t:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// SES-019 AC3, SBX-017 AC2: Ein Terminal hat dieselben FS-Grenzen wie Stufe 2: Der
/// Fixture-Schlüssel ist nicht lesbar, der Workspace schon; Kontrollgruppe ohne Sandbox liest
/// ihn.
#[tokio::test(flavor = "multi_thread")]
async fn ses_019_ac3_sbx_017_ac2_cat_ssh_key_in_a_terminal_fails() {
    let fx = fx();
    let p = provider();
    let policy = fx.policy(&*p, "workspace: rw\n", Stage::Tools);
    let script = format!(
        "cat {}/.ssh/id_ed25519; echo \"rc=$?\"; echo im-workspace > neu.txt && echo ws-ok; echo ende",
        fx.home.display()
    );
    let t = Term::start(&*p, &policy, &script);
    let text = t.wait_for("ende").await;
    assert!(!text.contains(FIXTURE_SSH), "{text}");
    assert!(text.contains("rc=1"), "{text}");
    assert!(text.contains("ws-ok"), "{text}");
    assert_eq!(
        std::fs::read_to_string(fx.ws.join("neu.txt")).unwrap(),
        "im-workspace\n"
    );
    let ctl = control(&fx, &format!("cat {}/.ssh/id_ed25519", fx.home.display())).await;
    assert!(ctl.stdout.contains(FIXTURE_SSH), "Kontrollgruppe: {ctl:?}");
}

/// SBX-017: Der Slave ist das Controlling Terminal des Kindes (`/dev/tty` öffnet, `stty`
/// liest die Größe), sonst gingen Ctrl+C und Fenstergrößen ins Leere.
#[tokio::test(flavor = "multi_thread")]
async fn sbx_017_the_slave_is_the_controlling_terminal_of_the_child() {
    let fx = fx();
    let p = provider();
    let policy = fx.policy(&*p, "workspace: rw\n", Stage::Tools);
    let t = Term::start(
        &*p,
        &policy,
        "[ -t 0 ] && echo stdin-tty; (exec </dev/tty) && echo ctty-ok; echo \"size=$(stty size)\"; echo ende",
    );
    let text = t.wait_for("ende").await;
    assert!(text.contains("stdin-tty"), "{text}");
    assert!(text.contains("ctty-ok"), "{text}");
    assert!(text.contains("size=24 80"), "{text}");
}

/// SBX-017 AC3: Fenstergrößenänderungen (`SIGWINCH`) und Ctrl+C (`SIGINT` über die
/// Line-Discipline) erreichen den Prozess in der Sandbox; der Exit-Code kommt zurück.
#[tokio::test(flavor = "multi_thread")]
async fn sbx_017_ac3_resize_and_ctrl_c_reach_the_sandboxed_process() {
    let fx = fx();
    let p = provider();
    let policy = fx.policy(&*p, "workspace: rw\n", Stage::Tools);
    let mut t = Term::start(
        &*p,
        &policy,
        "trap 'echo \"WINCH $(stty size)\"' WINCH; trap 'echo INTERRUPTED; exit 7' INT; \
         echo bereit; while :; do sleep 0.1; done",
    );
    t.wait_for("bereit").await;
    t.pty.resize(50, 20).unwrap();
    t.wait_for("WINCH 20 50").await;
    t.input.write_all(b"\x03").unwrap();
    t.input.flush().unwrap();
    t.wait_for("INTERRUPTED").await;
    let status = tokio::time::timeout(Duration::from_secs(20), t.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.code(), Some(7), "{status:?}");
}

/// Escape (SBX-017): Ein Terminal in der Sandbox erreicht kein anderes Terminal des Benutzers.
/// Ein zweites PTY auf dem Host steht für dessen Shell; Schreiben auf seinen Slave scheitert,
/// am Master kommt nichts an. Kontrollgruppe ohne Sandbox: Das Schreiben gelingt.
#[tokio::test(flavor = "multi_thread")]
async fn sbx_017_a_sandboxed_terminal_cannot_write_to_other_terminals() {
    let fx = fx();
    let p = provider();
    let policy = fx.policy(&*p, "workspace: rw\n", Stage::Tools);
    let victim = Pty::open(80, 24).unwrap();
    let path = victim.slave_path().display().to_string();
    let seen = Arc::new(Mutex::new(Vec::new()));
    {
        let mut reader = victim.reader().unwrap();
        let sink = seen.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                sink.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
    }
    let t = Term::start(
        &*p,
        &policy,
        &format!("echo PWNED-SANDBOX > {path}; echo \"rc=$?\"; echo ende"),
    );
    let text = t.wait_for("ende").await;
    assert!(!text.contains("rc=0"), "{text}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let got = String::from_utf8_lossy(&seen.lock().unwrap()).into_owned();
    assert!(!got.contains("PWNED-SANDBOX"), "{got}");
    // Kontrollgruppe.
    let ctl = control(&fx, &format!("echo PWNED-CONTROL > {path}")).await;
    assert!(ctl.ok, "{ctl:?}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !String::from_utf8_lossy(&seen.lock().unwrap()).contains("PWNED-CONTROL") {
        assert!(
            Instant::now() < deadline,
            "Kontrollgruppe erreicht das Terminal nicht"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
