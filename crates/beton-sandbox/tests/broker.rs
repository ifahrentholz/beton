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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use beton_sandbox::broker::proto::{Reply, Request, VERSION};
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
    const ATTACK: &str = "echo evil >> .beton/config.yaml && echo W1; \
        mkdir -p .claude; echo '{\"hooks\":{}}' > .claude/settings.json && echo W2; \
        mv .beton .beton-alt && echo W3; \
        rm -rf .beton && echo W4; \
        mkdir -p .beton/agents; echo x > .beton/agents/evil.yaml && echo W5; \
        ln -s /etc/hosts .claude/link && echo W6; \
        echo ok > src/neu.rs && echo SRC_OK";
    // Kontrollgruppe: Ohne Sandbox gelingt jeder Schritt.
    let host = fx();
    write(&host.ws.join(".beton/config.yaml"), "harnesses: {}\n");
    let out = on_host(&host.ws, ATTACK, &[]);
    for w in ["W1", "W2", "W3", "W5", "W6", "SRC_OK"] {
        assert!(out.contains(w), "Kontrollgruppe {w}: {out}");
    }

    let fx = fx();
    write(&fx.ws.join(".beton/config.yaml"), "harnesses: {}\n");
    let (_, out) = in_stage(ctx(&fx, Stage::Tools), ATTACK, &[]).await;
    for w in ["W1", "W2", "W3", "W4", "W5", "W6"] {
        assert!(!out.contains(w), "Stufe 2 {w}: {out}");
    }
    assert!(
        out.contains("SRC_OK"),
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
    const ATTACK: &str = "ln -s \"$HOME/.claude/.credentials.json\" \"$X/claude-cwd\" && echo L1; \
        ln -s /etc/hosts \"$X/l2\" && echo L2; \
        ln -s /etc/hosts \"$WS/link\" && echo WS_LINK; mv \"$WS/link\" \"$X/moved\" && echo L3; \
        mkdir -p \"$X/sub\"; ln -s /etc/hosts \"$X/sub/l4\" && echo L4; \
        printf ok > \"$X/regular\" && echo REGULAR_OK; \
        ln -s /etc/hosts \"$TMPDIR/eigener\" && echo TMP_LINK_OK";
    let base = tempfile::tempdir_in(beton_sandbox::session_base()).unwrap();
    let base = base.path().canonicalize().unwrap();
    // Kontrollgruppe ohne Sandbox.
    let host = fx();
    let hx = base.join("host-xchg");
    std::fs::create_dir_all(&hx).unwrap();
    let out = on_host(
        &host.ws,
        &ATTACK.replace("$TMPDIR", "$X"),
        &[("X", &hx), ("WS", &host.ws), ("HOME", &host.home)],
    );
    for w in ["L1", "L2", "L3", "L4", "REGULAR_OK"] {
        assert!(out.contains(w), "Kontrollgruppe {w}: {out}");
    }

    let fx = fx();
    let xchg = base.join("xchg");
    std::fs::create_dir_all(&xchg).unwrap();
    let mut c = ctx(&fx, Stage::Tools);
    c.exchange = Some(xchg.clone());
    let (_, out) = in_stage(c, ATTACK, &[("X", &xchg), ("WS", &fx.ws)]).await;
    for w in ["L1", "L2", "L3", "L4"] {
        assert!(!out.contains(w), "Stufe 2 {w}: {out}");
    }
    for w in ["WS_LINK", "REGULAR_OK", "TMP_LINK_OK"] {
        assert!(out.contains(w), "Stufe 2 {w}: {out}");
    }
    assert_eq!(std::fs::read_to_string(xchg.join("regular")).unwrap(), "ok");
    for name in ["claude-cwd", "l2", "moved", "sub/l4"] {
        assert!(
            std::fs::symlink_metadata(xchg.join(name)).is_err(),
            "{name} existiert"
        );
    }
}
