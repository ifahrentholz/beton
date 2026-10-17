//! Exec-Broker und `bt-exec` (SBX-002) gegen das Backend dieser Plattform (macOS: Seatbelt,
//! Linux: Landlock bzw. bwrap): Ablehnung unzulässiger Anfragen (AC3), Exit-Code, Signale und
//! stdio (AC5), Ausführung in Stufe 2 ohne Keychain bzw. Runtime-Verzeichnis (AC2) und ohne
//! die Credentials der Stufe 1 (automatisierter Teil von AC1; mit der echten CLI prüft die
//! Checkliste im PR).
//!
//! `bt-exec` ist hier das Test-Binary `beton-sandbox-exec` unter dem Namen `bt-exec` (wie
//! `beton` im Produkt). Alle Homes und Secrets sind Fixtures in Temp-Verzeichnissen.
//!
//! Diese Tests werden nie übersprungen oder abgeschwächt, damit die CI grün wird (AGENTS.md).

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]
#![allow(unsafe_code, reason = "Signale an das eigene Kind (kill)")]

use std::os::fd::AsRawFd as _;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use beton_sandbox::broker::proto::{Op, Reply, Request, VERSION};
use beton_sandbox::broker::{Broker, client};
use beton_sandbox::{
    BackendChoice, CommandSpec, Registry, ResolveCtx, ResolvedPolicy, Sandbox, SandboxProvider,
    SandboxSpec, Stage, Stdio,
};

const FIXTURE_CLAUDE: &str = "FIXTURE-CLAUDE-CREDENTIALS";

struct Fx {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    ws: PathBuf,
    harness_tmp: PathBuf,
    tools_tmp: PathBuf,
    exec_dir: PathBuf,
}

fn write(p: &Path, content: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let home = root.join("home");
    let ws = root.join("ws");
    write(&home.join(".claude/.credentials.json"), FIXTURE_CLAUDE);
    write(&home.join(".ssh/id_ed25519"), "FIXTURE-SSH");
    write(&ws.join("src/main.rs"), "fn main() {}\n");
    let harness_tmp = root.join("sbx/harness");
    let tools_tmp = root.join("sbx/tools");
    let exec_dir = root.join("sbx/exec");
    for d in [&harness_tmp, &tools_tmp, &exec_dir] {
        std::fs::create_dir_all(d).unwrap();
    }
    Fx {
        _tmp: tmp,
        root,
        home,
        ws,
        harness_tmp,
        tools_tmp,
        exec_dir,
    }
}

/// Das Backend dieser Plattform.
fn provider() -> Arc<dyn SandboxProvider> {
    #[cfg(target_os = "linux")]
    let registry = Registry::with_launcher(&launcher());
    #[cfg(not(target_os = "linux"))]
    let registry = Registry::platform();
    let p = registry.select(BackendChoice::Auto).unwrap();
    let probe = p.probe();
    assert!(probe.available, "Sandbox muss verfügbar sein: {probe:?}");
    p
}

fn launcher() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_beton-sandbox-exec"))
}

/// `<exec>/bin/bt-exec` → Test-Binary.
fn shim_dir(fx: &Fx) -> PathBuf {
    let bin = fx.exec_dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for name in ["bt-exec", "bt-exec-sh"] {
        let link = bin.join(name);
        if !link.exists() {
            std::os::unix::fs::symlink(launcher(), &link).unwrap();
        }
    }
    bin
}

fn ctx(fx: &Fx, stage: Stage) -> ResolveCtx {
    let scratch = match stage {
        Stage::Harness => &fx.harness_tmp,
        Stage::Tools => &fx.tools_tmp,
    };
    let mut ctx = ResolveCtx::new(stage, &fx.ws, &fx.home, scratch);
    ctx.adapters = beton_sandbox::masks::builtin_harness_grants();
    ctx.session_id = Some("ses_brokertest".into());
    ctx.host_env = [
        ("PATH", "/usr/bin:/bin:/usr/sbin:/sbin"),
        ("LANG", "C.UTF-8"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    ctx
}

/// Stufe-2-Sandbox des Brokers; zählt die Starts (`sandbox.started`).
fn tools(fx: &Fx, ctx: ResolveCtx) -> (Sandbox, Arc<AtomicUsize>) {
    let p = provider();
    let spec = SandboxSpec::from_yaml(&format!(
        "read_paths: [\"{}\"]\n",
        launcher().parent().unwrap().display()
    ))
    .unwrap();
    let policy = p.resolve(&spec, &ctx).unwrap();
    let started = Arc::new(AtomicUsize::new(0));
    let s = started.clone();
    let _ = fx;
    (
        Sandbox::new(
            p,
            policy,
            Some(Arc::new(move |_| {
                s.fetch_add(1, Ordering::SeqCst);
            })),
        ),
        started,
    )
}

fn request(fx: &Fx, token: &str, argv: &[&str]) -> Request {
    Request {
        v: VERSION,
        op: Op::Exec,
        token: token.to_owned(),
        argv: argv.iter().map(|s| (*s).to_owned()).collect(),
        cwd: fx.ws.clone(),
        env_overrides: Default::default(),
        tty: false,
        cols: None,
        rows: None,
    }
}

fn devnull() -> [std::fs::File; 3] {
    let open = || {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .unwrap()
    };
    [open(), open(), open()]
}

fn ask(broker: &Broker, req: &Request) -> Reply {
    let fds = devnull();
    client::request(
        broker.socket(),
        req,
        [fds[0].as_raw_fd(), fds[1].as_raw_fd(), fds[2].as_raw_fd()],
    )
    .unwrap()
}

fn error_code(reply: &Reply) -> &str {
    match reply {
        Reply::Error { code, .. } => code,
        other => panic!("Fehler erwartet: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sbx_002_ac3_broker_rejects_foreign_cwd_denied_env_and_bad_requests() {
    let fx = fx();
    let (sandbox, started) = tools(&fx, ctx(&fx, Stage::Tools));
    let broker = Broker::start(&fx.exec_dir, sandbox).await.unwrap();
    let token = broker.token().expose().to_owned();
    // Gültige Anfrage als Kontrollgruppe.
    let ok = ask(&broker, &request(&fx, &token, &["/bin/sh", "-c", "exit 0"]));
    assert!(matches!(ok, Reply::Started { .. }), "{ok:?}");
    assert_eq!(started.load(Ordering::SeqCst), 1);

    let mut r = request(&fx, &token, &["/bin/sh", "-c", "exit 0"]);
    r.cwd = PathBuf::from("/etc");
    assert_eq!(error_code(&ask(&broker, &r)), "cwd_denied");
    r.cwd = fx.home.join(".ssh");
    assert_eq!(error_code(&ask(&broker, &r)), "cwd_denied");
    r.cwd = fx.ws.join("..");
    assert_eq!(error_code(&ask(&broker, &r)), "cwd_denied");
    r.cwd = fx.ws.join("gibt-es-nicht");
    assert_eq!(error_code(&ask(&broker, &r)), "cwd_denied");

    for name in [
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "SSH_AUTH_SOCK",
        "BETON_TOKEN",
        "BETON_EXEC_TOKEN",
        "BETON_STAGE",
        "HTTPS_PROXY",
        "SSL_CERT_FILE",
        "HOME",
        "TMPDIR",
        "ANTHROPIC_API_KEY",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "OPENAI_API_KEY",
        "bad name",
    ] {
        let mut r = request(&fx, &token, &["/bin/sh", "-c", "exit 0"]);
        r.env_overrides.insert(name.into(), "/tmp/x.so".into());
        assert_eq!(error_code(&ask(&broker, &r)), "env_denied", "{name}");
    }

    let mut wrong = request(&fx, "falsch", &["/bin/sh", "-c", "exit 0"]);
    assert_eq!(error_code(&ask(&broker, &wrong)), "unauthorized");
    wrong.token = String::new();
    assert_eq!(error_code(&ask(&broker, &wrong)), "unauthorized");
    assert_eq!(
        error_code(&ask(&broker, &request(&fx, &token, &[]))),
        "invalid_request"
    );
    assert_eq!(
        error_code(&ask(
            &broker,
            &request(&fx, &token, &["sh", "-c", "exit 0"])
        )),
        "invalid_request"
    );
    let mut tty = request(&fx, &token, &["/bin/sh"]);
    tty.tty = true;
    assert_eq!(error_code(&ask(&broker, &tty)), "unsupported");
    let mut old = request(&fx, &token, &["/bin/sh"]);
    old.v = VERSION + 1;
    assert_eq!(error_code(&ask(&broker, &old)), "invalid_request");
    // Nichts davon wurde gestartet.
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(broker.started(), 1, "Zähler für die Integritätsprüfung");
}

/// `<exec>/git/git` → Test-Binary: der Git-Shim der Stufe 1 (SBX-003, #311).
fn git_shim(fx: &Fx) -> PathBuf {
    let dir = fx.exec_dir.join("git");
    std::fs::create_dir_all(&dir).unwrap();
    let link = dir.join("git");
    if !link.exists() {
        std::os::unix::fs::symlink(launcher(), &link).unwrap();
    }
    link
}

/// Prüfung der Git-Steuerdateien, die per Schalter einen Verstoß meldet.
fn git_check(changed: &Arc<AtomicBool>) -> beton_sandbox::broker::GitCheck {
    let changed = changed.clone();
    Arc::new(move || {
        if changed.load(Ordering::SeqCst) {
            Err("Git-Steuerdatei geändert: .git/commondir".into())
        } else {
            Ok(())
        }
    })
}

/// Vor jedem `git` der Stufe 1 fragt der Shim den Broker (Stufe 0), ob die Git-Steuerdateien
/// dem Schnappschuss entsprechen (SBX-003, #311). Der Broker startet dabei nichts; ohne
/// gültiges Token oder ohne Prüfung lehnt er ab (fail closed).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sbx_003_git_preflight_asks_stage_zero_and_fails_closed() {
    let fx = fx();
    let (sandbox, started) = tools(&fx, ctx(&fx, Stage::Tools));
    let changed = Arc::new(AtomicBool::new(false));
    let broker = Broker::start_with_git_check(&fx.exec_dir, sandbox, git_check(&changed))
        .await
        .unwrap();
    let token = broker.token().expose().to_owned();
    let mut pre = request(&fx, &token, &[]);
    pre.op = Op::GitPreflight;
    // Kontrollgruppe: unverändert → git darf starten.
    assert_eq!(ask(&broker, &pre), Reply::Allowed);
    changed.store(true, Ordering::SeqCst);
    match ask(&broker, &pre) {
        Reply::Error { code, message } => {
            assert_eq!(code, "git_control_changed");
            assert!(message.contains(".git/commondir"), "{message}");
        }
        other => panic!("Ablehnung erwartet: {other:?}"),
    }
    changed.store(false, Ordering::SeqCst);
    let mut wrong = pre.clone();
    wrong.token = "falsch".into();
    assert_eq!(error_code(&ask(&broker, &wrong)), "unauthorized");
    let mut old = pre.clone();
    old.v = VERSION + 1;
    assert_eq!(error_code(&ask(&broker, &old)), "invalid_request");
    // Die Vorabprüfung startet nichts und zählt nicht als Start in Stufe 2.
    assert_eq!(started.load(Ordering::SeqCst), 0);
    assert_eq!(broker.started(), 0);
    drop(broker);

    // Ohne Prüfung des Runners: kein git (fail closed).
    let (sandbox, _) = tools(&fx, ctx(&fx, Stage::Tools));
    let broker = Broker::start(&fx.exec_dir, sandbox).await.unwrap();
    let mut pre = request(&fx, broker.token().expose(), &[]);
    pre.op = Op::GitPreflight;
    assert_eq!(error_code(&ask(&broker, &pre)), "unsupported");
}

/// Der Shim startet das echte git (absoluter Pfad aus Stufe 0) mit unveränderten Argumenten
/// und unveränderter Umgebung, aber nur nach erfolgreicher Prüfung; sonst endet er mit 126,
/// ohne git zu starten.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sbx_003_git_shim_starts_the_real_git_unchanged_only_after_the_check() {
    use std::os::unix::fs::PermissionsExt as _;
    let fx = fx();
    let (sandbox, _) = tools(&fx, ctx(&fx, Stage::Tools));
    let changed = Arc::new(AtomicBool::new(false));
    let broker = Broker::start_with_git_check(&fx.exec_dir, sandbox, git_check(&changed))
        .await
        .unwrap();
    // Attrappe des echten git: hält Aufrufpfad, Argumente und eine Variable fest.
    let marker = fx.root.join("git-aufgerufen");
    let real = fx.root.join("echt/git");
    write(
        &real,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$0\" \"$@\" \"FOO=$FOO\" > '{}'\nexit 3\n",
            marker.display()
        ),
    );
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Eine zweite Attrappe im `PATH`: Der Shim sucht nie selbst im `PATH`.
    let decoy = fx.root.join("lockvogel/git");
    write(
        &decoy,
        &format!("#!/bin/sh\necho lockvogel > '{}'\n", marker.display()),
    );
    std::fs::set_permissions(&decoy, std::fs::Permissions::from_mode(0o755)).unwrap();
    let shim = git_shim(&fx);
    let run = |real: Option<&str>| {
        let mut c = std::process::Command::new(&shim);
        c.args(["status", "--porcelain", "a b"])
            .current_dir(&fx.ws)
            .env_clear()
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", decoy.parent().unwrap().display()),
            )
            .env("BETON_EXEC_SOCKET", broker.socket())
            .env("BETON_EXEC_TOKEN", broker.token().expose())
            .env("FOO", "bar baz");
        if let Some(r) = real {
            c.env(beton_sandbox::broker::git_shim::ENV_GIT, r);
        }
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            c.env("LLVM_PROFILE_FILE", profile);
        }
        c.output().unwrap()
    };
    let real_str = real.to_str().unwrap();

    // Kontrollgruppe: unverändert → das echte git, Argumente und Umgebung unverändert,
    // Exit-Code durchgereicht.
    let out = run(Some(real_str));
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let got = std::fs::read_to_string(&marker).unwrap();
    assert_eq!(
        got.lines().collect::<Vec<_>>(),
        [real_str, "status", "--porcelain", "a b", "FOO=bar baz"]
    );
    std::fs::remove_file(&marker).unwrap();

    // Verändert → kein git, Exit ≠ 0 mit Meldung auf stderr.
    changed.store(true, Ordering::SeqCst);
    let out = run(Some(real_str));
    assert_eq!(out.status.code(), Some(126), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains(".git/commondir"), "{stderr}");
    assert!(!marker.exists(), "git darf nicht starten");
    changed.store(false, Ordering::SeqCst);

    // Fail closed: ohne bzw. mit relativem Pfad des echten git, ohne Broker, mit falschem
    // Token; nie über den `PATH`.
    for real in [None, Some("git"), Some("echt/git")] {
        let out = run(real);
        assert_eq!(out.status.code(), Some(126), "{real:?}: {out:?}");
        assert!(!marker.exists(), "{real:?}");
    }
    let out = {
        let mut c = std::process::Command::new(&shim);
        c.env_clear()
            .env("BETON_EXEC_SOCKET", fx.root.join("gibt-es-nicht.sock"))
            .env("BETON_EXEC_TOKEN", broker.token().expose())
            .env(beton_sandbox::broker::git_shim::ENV_GIT, real_str);
        c.output().unwrap()
    };
    assert_eq!(out.status.code(), Some(126), "{out:?}");
    let out = {
        let mut c = std::process::Command::new(&shim);
        c.env_clear()
            .env("BETON_EXEC_SOCKET", broker.socket())
            .env("BETON_EXEC_TOKEN", "falsch")
            .env(beton_sandbox::broker::git_shim::ENV_GIT, real_str);
        c.output().unwrap()
    };
    assert_eq!(out.status.code(), Some(126), "{out:?}");
    assert!(!marker.exists());
}

/// Shims der Stufe 1 (Verzeichnis des Brokers) gehören nicht in den `PATH` der Stufe 2: Dort
/// sind sie nicht lesbar, und `execvp` bräche an ihnen ab, statt weiterzusuchen.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sbx_002_broker_keeps_stage_one_shims_out_of_the_stage_two_path() {
    let fx = fx();
    let (sandbox, _) = tools(&fx, ctx(&fx, Stage::Tools));
    let broker = Broker::start(&fx.exec_dir, sandbox).await.unwrap();
    let shim_dir = git_shim(&fx).parent().unwrap().to_path_buf();
    let echo = |path: String| {
        let out = bt_exec(&fx, &broker, &["/bin/sh", "-c", "echo \"$PATH\""])
            .env("PATH", path)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    assert_eq!(
        echo(format!("{}:/usr/bin:/bin", shim_dir.display())),
        "/usr/bin:/bin"
    );
    // Nur Shims: Stufe 2 behält ihren eigenen `PATH`.
    let only = echo(shim_dir.display().to_string());
    assert!(
        !only.is_empty() && !only.contains(fx.exec_dir.to_str().unwrap()),
        "{only}"
    );
}

/// `bt-exec` mit dem Broker-Zugang im Environment.
fn bt_exec(fx: &Fx, broker: &Broker, args: &[&str]) -> std::process::Command {
    let mut c = std::process::Command::new(shim_dir(fx).join("bt-exec"));
    c.args(args)
        .current_dir(&fx.ws)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("BETON_EXEC_SOCKET", broker.socket())
        .env("BETON_EXEC_TOKEN", broker.token().expose());
    // Abdeckung des Shims unter `cargo llvm-cov` (QA-011); sonst ohne Wirkung.
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        c.env("LLVM_PROFILE_FILE", profile);
    }
    c
}

fn processes_with(marker: &str) -> Vec<String> {
    let out = std::process::Command::new("/bin/ps")
        .args(["-A", "-o", "pid=,command="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(marker) && !l.contains("/bin/ps"))
        .map(str::to_owned)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sbx_002_ac5_exit_codes_signals_and_stdio_pass_through_bt_exec() {
    let fx = fx();
    let (sandbox, _) = tools(&fx, ctx(&fx, Stage::Tools));
    let broker = Broker::start(&fx.exec_dir, sandbox).await.unwrap();

    let out = bt_exec(&fx, &broker, &["/bin/sh", "-c", "exit 7"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(7), "{out:?}");

    let out = bt_exec(
        &fx,
        &broker,
        &["sh", "-c", "echo aus; echo fehler >&2; cat"],
    )
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .spawn()
    .and_then(|mut c| {
        use std::io::Write as _;
        c.stdin.take().unwrap().write_all(b"ein\n")?;
        c.wait_with_output()
    })
    .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "aus\nein\n");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "fehler\n");

    // Ein Signal des Kommandos kommt als Signal zurück.
    let out = bt_exec(&fx, &broker, &["/bin/sh", "-c", "kill -TERM $$"])
        .output()
        .unwrap();
    assert_eq!(out.status.signal(), Some(libc::SIGTERM), "{out:?}");

    // SIGINT an bt-exec erreicht den Prozess in Stufe 2; danach bleibt nichts übrig.
    let marker = "61.4711";
    let mut child = bt_exec(&fx, &broker, &["/bin/sleep", marker])
        .stdin(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while processes_with(marker).len() < 2 {
        assert!(Instant::now() < deadline, "sleep startet nicht");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let pid = child.id() as i32;
    // SAFETY: kill an das eigene Kind.
    unsafe {
        libc::kill(pid, libc::SIGINT);
    }
    let started = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "bt-exec endet nicht nach SIGINT"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(status.signal(), Some(libc::SIGINT), "{status:?}");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !processes_with(marker).is_empty() {
        assert!(
            Instant::now() < deadline,
            "Prozess in Stufe 2 überlebt: {:?}",
            processes_with(marker)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Bricht bt-exec ab (SIGKILL), beendet der Broker den Baum.
    let marker = "62.4711";
    let mut child = bt_exec(&fx, &broker, &["/bin/sleep", marker])
        .stdin(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while processes_with(marker).len() < 2 {
        assert!(Instant::now() < deadline, "sleep startet nicht");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    child.kill().unwrap();
    let _ = child.wait();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !processes_with(marker).is_empty() {
        assert!(
            Instant::now() < deadline,
            "Prozess in Stufe 2 überlebt den Abbruch: {:?}",
            processes_with(marker)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Ohne bzw. mit falschem Token startet nichts (Exit 126 wie `env`).
    let out = bt_exec(&fx, &broker, &["/bin/sh", "-c", "exit 0"])
        .env("BETON_EXEC_TOKEN", "falsch")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(126), "{out:?}");
    let out = bt_exec(&fx, &broker, &["/bin/sh", "-c", "exit 0"])
        .env_remove("BETON_EXEC_SOCKET")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(126), "{out:?}");
}

/// Stufe-1-Sandbox mit Broker-Zugang (wie der Claude-Adapter) und das Ergebnis von `script`.
async fn in_stage_one(
    fx: &Fx,
    broker: &Broker,
    harness: &str,
    script: &str,
) -> (bool, String, String) {
    let p = provider();
    let mut c = ctx(fx, Stage::Harness);
    c.harness = beton_sandbox::masks::harness_grants(harness);
    c.exec = Some(broker.access(shim_dir(fx)));
    c.shared = vec![fx.tools_tmp.clone()];
    c.programs = vec![launcher()];
    let policy: ResolvedPolicy = p.resolve(&SandboxSpec::default(), &c).unwrap();
    let cmd = CommandSpec::new("/bin/sh")
        .args(["-c", script])
        .cwd(&fx.ws)
        .stdio(Stdio::Null, Stdio::Piped, Stdio::Piped);
    let out = p
        .spawn(&policy, cmd)
        .unwrap()
        .output(Duration::from_secs(60))
        .await
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sbx_002_ac2_command_via_bt_exec_runs_in_stage_two() {
    let fx = fx();
    // Ein Dienst im Runtime-Verzeichnis des Benutzers (wie der D-Bus-Session-Bus).
    let runtime = fx.root.join("run-user");
    std::fs::create_dir_all(&runtime).unwrap();
    let bus = runtime.join("bus");
    let l = std::os::unix::net::UnixListener::bind(&bus).unwrap();
    std::thread::spawn(move || {
        for c in l.incoming() {
            drop(c);
        }
    });
    let mut tctx = ctx(&fx, Stage::Tools);
    tctx.host_env
        .insert("XDG_RUNTIME_DIR".into(), runtime.display().to_string());
    let (sandbox, _) = tools(&fx, tctx);
    let broker = Broker::start(&fx.exec_dir, sandbox).await.unwrap();
    let shim = shim_dir(&fx).join("bt-exec");
    let (ok, stdout, stderr) = in_stage_one(
        &fx,
        &broker,
        "claude",
        &format!(
            "echo \"stufe=$BETON_STAGE\"; {} /bin/sh -c 'echo \"stufe=$BETON_STAGE\"; test -S \"{}\" && echo bus-sichtbar; echo \"token=${{BETON_EXEC_TOKEN:-}}\"'",
            shim.display(),
            bus.display()
        ),
    )
    .await;
    assert!(ok, "{stdout}{stderr}");
    assert_eq!(
        stdout
            .lines()
            .filter(|l| l.starts_with("stufe="))
            .collect::<Vec<_>>(),
        vec!["stufe=harness", "stufe=tools"],
        "{stdout}{stderr}"
    );
    assert!(!stdout.contains("bus-sichtbar"), "{stdout}");
    // Stufe 2 bekommt das Broker-Token nicht.
    assert!(
        stdout.contains("token=\n") || stdout.ends_with("token="),
        "{stdout}"
    );
    #[cfg(target_os = "macos")]
    {
        // Kein Mach-Lookup des Keychains in Stufe 2 (Stufe 1 hat ihn mit `keychain: true`).
        let (ok, stdout, stderr) = in_stage_one(
            &fx,
            &broker,
            "claude",
            &format!("{} /usr/bin/security list-keychains", shim.display()),
        )
        .await;
        assert!(!ok, "Keychain aus Stufe 2: {stdout}{stderr}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sbx_002_ac1_tool_via_bt_exec_cannot_read_the_harness_credentials() {
    let fx = fx();
    let (sandbox, _) = tools(&fx, ctx(&fx, Stage::Tools));
    let broker = Broker::start(&fx.exec_dir, sandbox).await.unwrap();
    let shim = shim_dir(&fx).join("bt-exec");
    let creds = "\"$HOME/.claude/.credentials.json\"";
    let (ok, stdout, stderr) = in_stage_one(
        &fx,
        &broker,
        "claude",
        &format!(
            "cat {creds} && echo; {} /bin/sh -c 'echo \"stufe=$BETON_STAGE\"; cat {creds}' ; echo \"rc=$?\"",
            shim.display()
        ),
    )
    .await;
    assert!(ok, "{stdout}{stderr}");
    // Der Harness selbst liest seine Credentials (Stufe 1) …
    assert!(stdout.starts_with(FIXTURE_CLAUDE), "{stdout}{stderr}");
    // … das Bash-Tool über bt-exec nicht (Stufe 2).
    assert_eq!(stdout.matches(FIXTURE_CLAUDE).count(), 1, "{stdout}");
    assert!(
        stdout.contains("stufe=tools"),
        "bt-exec lief nicht: {stdout}{stderr}"
    );
    assert!(!stdout.contains("rc=0"), "{stdout}{stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sbx_005_bt_exec_forwards_only_allowed_variables_to_stage_two() {
    // Escape-Test zu #259: Was die Vendor-CLI in Stufe 1 setzt, gelangt nur über die
    // Allowlist nach Stufe 2.
    let fx = fx();
    let mut tctx = ctx(&fx, Stage::Tools);
    tctx.host_env.insert("RUST_LOG".into(), "info".into());
    let p = provider();
    let spec = SandboxSpec::from_yaml(&format!(
        "read_paths: [\"{}\"]\nenv:\n  passthrough: [RUST_LOG]\n",
        launcher().parent().unwrap().display()
    ))
    .unwrap();
    let sandbox = Sandbox::new(p.clone(), p.resolve(&spec, &tctx).unwrap(), None);
    let broker = Broker::start(&fx.exec_dir, sandbox).await.unwrap();
    let shim = shim_dir(&fx).join("bt-exec");
    let (ok, stdout, stderr) = in_stage_one(
        &fx,
        &broker,
        "claude",
        &format!(
            "export FOO_TOKEN=FIXTURE-FOO GIT_ASKPASS=/tmp/x RUST_LOG=trace PWD=\"$PWD\"; \
             echo \"eins=$FOO_TOKEN\"; {} /usr/bin/env | sed 's/^/zwei:/'",
            shim.display()
        ),
    )
    .await;
    assert!(ok, "{stdout}{stderr}");
    // Kontrollgruppe: in Stufe 1 ist die Variable gesetzt …
    assert!(stdout.contains("eins=FIXTURE-FOO"), "{stdout}");
    // … in Stufe 2 nicht, ebenso wenig GIT_ASKPASS; erlaubte Werkzeug-Variablen schon.
    let two: Vec<&str> = stdout.lines().filter(|l| l.starts_with("zwei:")).collect();
    assert!(two.contains(&"zwei:BETON_STAGE=tools"), "{stdout}");
    assert!(!two.iter().any(|l| l.contains("FOO_TOKEN")), "{stdout}");
    assert!(!two.iter().any(|l| l.contains("GIT_ASKPASS")), "{stdout}");
    assert!(
        two.contains(&"zwei:RUST_LOG=trace"),
        "konfigurierte Variable: {stdout}"
    );
    // Der Broker lehnt eine solche Variable selbst ab (fail closed), auch ohne bt-exec.
    let token = broker.token().expose().to_owned();
    let mut r = request(&fx, &token, &["/bin/sh", "-c", "exit 0"]);
    r.env_overrides
        .insert("FOO_TOKEN".into(), "FIXTURE-FOO".into());
    assert_eq!(error_code(&ask(&broker, &r)), "env_denied");
    let mut r = request(&fx, &token, &["/bin/sh", "-c", "exit 0"]);
    r.env_overrides.insert("RUST_LOG".into(), "debug".into());
    assert!(matches!(ask(&broker, &r), Reply::Started { .. }));
    // `PATH` nur mit absoluten Einträgen (SBX-002): ein relativer hinge vom Arbeitsverzeichnis ab.
    for path in ["bin:/usr/bin", "/usr/bin::/bin", "./node_modules/.bin:/bin"] {
        let mut r = request(&fx, &token, &["/bin/sh", "-c", "exit 0"]);
        r.env_overrides.insert("PATH".into(), path.into());
        assert_eq!(error_code(&ask(&broker, &r)), "env_denied", "{path}");
    }
}

/// `script` direkt in einer Sandbox der Stufe `stage` (ohne Broker), mit `X` und `WS` im
/// Environment.
async fn in_stage(ctx: ResolveCtx, script: &str, vars: &[(&str, &Path)]) -> (bool, String) {
    let p = provider();
    let policy: ResolvedPolicy = p.resolve(&SandboxSpec::default(), &ctx).unwrap();
    let mut cmd = CommandSpec::new("/bin/sh")
        .args(["-c", script])
        .cwd(&ctx.workspace)
        .stdio(Stdio::Null, Stdio::Piped, Stdio::Piped);
    for (k, v) in vars {
        cmd = cmd.env(*k, v.display().to_string());
    }
    let out = p
        .spawn(&policy, cmd)
        .unwrap()
        .output(Duration::from_secs(60))
        .await
        .unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// Dasselbe ohne Sandbox (Kontrollgruppe).
fn on_host(cwd: &Path, script: &str, vars: &[(&str, &Path)]) -> String {
    let mut c = std::process::Command::new("/bin/sh");
    c.args(["-c", script]).current_dir(cwd);
    for (k, v) in vars {
        c.env(k, v);
    }
    let out = c.output().unwrap();
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Ein Tool in Stufe 2 darf die Projekt-Konfiguration nicht ändern, aus der der nächste Start
/// Stufe 1 aufbaut (`.beton/config.yaml`, Projekt-Agents, `.claude/settings.json` mit Hooks).
/// Sonst liefe beim nächsten Start Code des Tools in Stufe 1 (Security-Review #228).
#[tokio::test]
async fn sbx_002_stage_two_cannot_change_the_project_configuration() {
    const ATTACK: &str = "echo evil >> .beton/config.yaml && echo =W1=; \
        ln .beton/config.yaml hart && echo evil >> hart && echo =W7=; \
        mkdir -p .CLAUDE; echo '{\"hooks\":{}}' > .CLAUDE/settings.json && echo =W8=; \
        mkdir -p .claude; echo '{\"hooks\":{}}' > .claude/settings.json && echo =W2=; \
        ln -s /etc/hosts .claude/link && echo =W6=; \
        mv .beton .beton-alt && echo =W3=; \
        rm -rf .beton && echo =W4=; \
        mkdir -p .beton/agents; echo x > .beton/agents/evil.yaml && echo =W5=; \
        echo ok > src/neu.rs && echo =SRC_OK=";
    // Kontrollgruppe: Ohne Sandbox gelingt jeder Schritt.
    let host = fx();
    write(&host.ws.join(".beton/config.yaml"), "harnesses: {}\n");
    let out = on_host(&host.ws, ATTACK, &[]);
    for w in ["W1", "W2", "W3", "W5", "W6", "W7", "W8", "SRC_OK"] {
        assert!(out.contains(&format!("={w}=")), "Kontrollgruppe {w}: {out}");
    }

    let fx = fx();
    write(&fx.ws.join(".beton/config.yaml"), "harnesses: {}\n");
    let (_, out) = in_stage(ctx(&fx, Stage::Tools), ATTACK, &[]).await;
    for w in ["W1", "W2", "W3", "W4", "W5", "W6", "W7"] {
        assert!(!out.contains(&format!("={w}=")), "Stufe 2 {w}: {out}");
    }
    // `.CLAUDE` ist auf einem Dateisystem ohne Groß-/Kleinschreibung (APFS-Standard) dasselbe
    // wie `.claude` und darf dort nicht entstehen; unter Linux ist es ein anderes, harmloses
    // Verzeichnis. Entscheidend ist, was Claude als `.claude/settings.json` läse (unten).
    if cfg!(target_os = "macos") {
        assert!(!out.contains("=W8="), "Stufe 2 W8: {out}");
    }
    assert!(
        out.contains("=SRC_OK="),
        "Workspace bleibt beschreibbar: {out}"
    );
    assert_eq!(
        std::fs::read_to_string(fx.ws.join(".beton/config.yaml")).unwrap(),
        "harnesses: {}\n"
    );
    assert!(!fx.ws.join(".claude/settings.json").exists());
    assert!(!fx.ws.join(".beton/agents/evil.yaml").exists());
}

/// Stufe 1 liest, was Stufe 2 in den Austausch schreibt (cwd-Datei, Hintergrund-Ausgaben von
/// Claude). Ein Symlink dort auf die Credentials der Stufe 1 ließe Claude sie lesen und ans
/// Modell geben: In Stufe 2 entstehen im Austausch keine Symlinks, auch nicht per Umbenennen.
#[tokio::test]
async fn sbx_002_stage_two_cannot_plant_symlinks_in_the_exchange() {
    const ATTACK: &str = "ln -s \"$HOME/.claude/.credentials.json\" \"$X/claude-cwd\" && echo =L1=; \
        ln -s /etc/hosts \"$X/l2\" && echo =L2=; \
        ln -s /etc/hosts \"$WS/link\" && echo =WS_LINK=; mv \"$WS/link\" \"$X/moved\" && echo =L3=; \
        mkdir -p \"$X/sub\"; ln -s /etc/hosts \"$X/sub/l4\" && echo =L4=; \
        printf ok > \"$X/regular\" && echo =REGULAR_OK=; \
        mkdir -p \"$TMPDIR/d/tasks\"; ln -s \"$HOME/.claude/.credentials.json\" \"$TMPDIR/d/tasks/1.output\"; \
        mv \"$X/vorhanden\" \"$X/alt\"; mv \"$TMPDIR/d\" \"$X/vorhanden\" && echo =L5=; \
        ln -s /etc/hosts \"$TMPDIR/s\"; ln -P \"$TMPDIR/s\" \"$X/hart\" && echo =L6=; \
        ln -s /etc/hosts \"$TMPDIR/eigener\" && echo =TMP_LINK_OK=";
    let base = tempfile::tempdir_in(beton_sandbox::session_base()).unwrap();
    let base = base.path().canonicalize().unwrap();
    // Kontrollgruppe ohne Sandbox.
    let host = fx();
    let hx = base.join("host-xchg");
    // Unterverzeichnis, wie es Claude in Stufe 1 anlegt.
    std::fs::create_dir_all(hx.join("vorhanden/tasks")).unwrap();
    let out = on_host(
        &host.ws,
        &ATTACK.replace("$TMPDIR", "$X"),
        &[("X", &hx), ("WS", &host.ws), ("HOME", &host.home)],
    );
    for w in ["L1", "L2", "L3", "L4", "L5", "L6", "REGULAR_OK"] {
        assert!(out.contains(&format!("={w}=")), "Kontrollgruppe {w}: {out}");
    }

    let fx = fx();
    let xchg = base.join("xchg");
    std::fs::create_dir_all(xchg.join("vorhanden/tasks")).unwrap();
    let mut c = ctx(&fx, Stage::Tools);
    c.exchange = Some(xchg.clone());
    let (_, out) = in_stage(c, ATTACK, &[("X", &xchg), ("WS", &fx.ws)]).await;
    for w in ["L1", "L2", "L3", "L4", "L5", "L6"] {
        assert!(!out.contains(&format!("={w}=")), "Stufe 2 {w}: {out}");
    }
    // Nirgends im Austausch ein Symlink, auch nicht in Unterverzeichnissen.
    fn no_links(d: &Path) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let t = e.file_type().unwrap();
            assert!(
                !t.is_symlink(),
                "Symlink im Austausch: {}",
                e.path().display()
            );
            if t.is_dir() {
                no_links(&e.path());
            }
        }
    }
    no_links(&xchg);
    for w in ["WS_LINK", "REGULAR_OK", "TMP_LINK_OK"] {
        assert!(out.contains(&format!("={w}=")), "Stufe 2 {w}: {out}");
    }
    assert_eq!(std::fs::read_to_string(xchg.join("regular")).unwrap(), "ok");
    for name in ["claude-cwd", "l2", "moved", "sub/l4"] {
        assert!(
            std::fs::symlink_metadata(xchg.join(name)).is_err(),
            "{name} existiert"
        );
    }
}

/// Das Session-Verzeichnis (Scratch der Stufe 1 mit Claudes Sockets, Broker-Socket) ist in
/// Stufe 2 unsichtbar; nur Scratch, Austausch und CA-Bundle der Stufe 2 bleiben erreichbar.
/// Unter Linux vor Landlock-ABI 9 wäre ein Unix-Socket sonst per `connect` erreichbar.
#[tokio::test]
async fn sbx_002_stage_two_sees_only_its_part_of_the_session_directory() {
    let base = tempfile::tempdir_in(beton_sandbox::session_base()).unwrap();
    let root = base.path().canonicalize().unwrap();
    for d in ["harness", "tools", "xchg", "exec", "ca"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(root.join("harness/geheim"), "FIXTURE-HARNESS-SCRATCH").unwrap();
    std::fs::write(root.join("ca/bundle.pem"), "FIXTURE-CA").unwrap();
    let _sock = std::os::unix::net::UnixListener::bind(root.join("exec/exec.sock")).unwrap();
    const PROBE: &str = "cat \"$R/harness/geheim\" && echo =H1=; \
        [ -e \"$R/exec/exec.sock\" ] && echo =H2=; \
        [ -e \"$R/harness\" ] && echo =H3=; [ -e \"$R/exec\" ] && echo =H4=; \
        printf ok > \"$R/xchg/x\" && echo =XCHG_OK=; \
        cat \"$R/ca/bundle.pem\" && echo =CA_OK=; \
        printf ok > \"$TMPDIR/t\" && echo =TMP_OK=";
    // Kontrollgruppe ohne Sandbox.
    let host = fx();
    let out = on_host(
        &host.ws,
        PROBE,
        &[("R", &root), ("TMPDIR", &root.join("tools"))],
    );
    for w in ["H1", "H2", "H3", "H4", "XCHG_OK", "CA_OK", "TMP_OK"] {
        assert!(out.contains(&format!("={w}=")), "Kontrollgruppe {w}: {out}");
    }

    let fx = fx();
    let mut c = ResolveCtx::new(Stage::Tools, &fx.ws, &fx.home, &root.join("tools"));
    c.adapters = beton_sandbox::masks::builtin_harness_grants();
    c.host_env = ctx(&fx, Stage::Tools).host_env;
    c.exchange = Some(root.join("xchg"));
    c.session_dir = Some(root.clone());
    let spec = SandboxSpec::from_yaml(&format!(
        "read_paths: [\"{}\"]\n",
        root.join("ca/bundle.pem").display()
    ))
    .unwrap();
    let p = provider();
    let policy = p.resolve(&spec, &c).unwrap();
    let cmd = CommandSpec::new("/bin/sh")
        .args(["-c", PROBE])
        .cwd(&fx.ws)
        .env("R", root.display().to_string())
        .stdio(Stdio::Null, Stdio::Piped, Stdio::Piped);
    let out = p
        .spawn(&policy, cmd)
        .unwrap()
        .output(Duration::from_secs(60))
        .await
        .unwrap();
    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for w in ["H1", "H2", "H3", "H4"] {
        assert!(!out.contains(&format!("={w}=")), "Stufe 2 {w}: {out}");
    }
    assert!(!out.contains("FIXTURE-HARNESS-SCRATCH"), "{out}");
    for w in ["XCHG_OK", "CA_OK", "TMP_OK"] {
        assert!(out.contains(&format!("={w}=")), "Stufe 2 {w}: {out}");
    }
}
