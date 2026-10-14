//! SBX-007 (und die Kernel-Durchsetzung von SBX-003 AC4 und SBX-004) auf macOS gegen das echte
//! `sandbox-exec`.
//!
//! Jeder Angriff hat eine Kontrollgruppe: Derselbe Befehl ohne Sandbox gelingt, damit ein
//! Scheitern in der Sandbox nicht an der Fixture liegt (vgl. QA-005 AC1). Alle Homes,
//! Dotfiles und Secrets sind Fixtures in Temp-Verzeichnissen; kein Test liest echte
//! Nutzerdaten. Netz-Tests laufen ausschließlich gegen lokale Test-Upstreams über den
//! Egress-Proxy aus `beton-proxy` (ADR-0033).
//!
//! Diese Tests werden nie übersprungen oder abgeschwächt, damit die CI grün wird (AGENTS.md).

#![cfg(target_os = "macos")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use beton_sandbox::{
    CommandSpec, GitLayout, NoneProvider, ProviderId, ProxyEndpoint, Registry, ResolveCtx,
    ResolvedPolicy, SandboxProvider, SandboxSpec, Stage, Stdio,
};

const FIXTURE_SSH: &str = "FIXTURE-SSH-PRIVATE-KEY";
const FIXTURE_ENV: &str = "FIXTURE-DOTENV";
const FIXTURE_CODEX: &str = "FIXTURE-CODEX-AUTH";

struct Fx {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    ws: PathBuf,
    scratch: PathBuf,
}

fn write(p: &Path, content: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Gefälschtes Home voller Fixture-Secrets, Workspace mit `.env`, Scratch.
fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let home = root.join("home");
    let ws = root.join("ws");
    let scratch = root.join("scratch");
    write(&home.join(".ssh/id_ed25519"), FIXTURE_SSH);
    write(&home.join(".aws/credentials"), "FIXTURE-AWS");
    write(&home.join(".config/gh/hosts.yml"), "FIXTURE-GH");
    write(&home.join(".codex/auth.json"), FIXTURE_CODEX);
    write(&home.join(".claude/.credentials.json"), "FIXTURE-CLAUDE");
    write(&home.join(".zshrc"), "# fixture\n");
    write(&home.join("Documents/notiz.txt"), "Notiz");
    std::fs::create_dir_all(home.join("Library/LaunchAgents")).unwrap();
    write(&ws.join(".env"), FIXTURE_ENV);
    write(&ws.join(".github/workflows/ci.yml"), "on: push\n");
    write(&ws.join("src/main.rs"), "fn main() {}\n");
    std::fs::create_dir_all(&scratch).unwrap();
    Fx {
        _tmp: tmp,
        root,
        home,
        ws,
        scratch,
    }
}

impl Fx {
    fn ctx(&self, stage: Stage) -> ResolveCtx {
        let mut ctx = ResolveCtx::new(stage, &self.ws, &self.home, &self.scratch);
        ctx.adapters = beton_sandbox::masks::builtin_harness_grants();
        ctx
    }

    fn policy(&self, yaml: &str, stage: Stage) -> ResolvedPolicy {
        self.policy_with(yaml, self.ctx(stage))
    }

    fn policy_with(&self, yaml: &str, ctx: ResolveCtx) -> ResolvedPolicy {
        let spec = SandboxSpec::from_yaml(yaml).unwrap();
        seatbelt().resolve(&spec, &ctx).unwrap()
    }
}

fn seatbelt() -> Arc<dyn SandboxProvider> {
    let p = Registry::platform().get(ProviderId::Seatbelt).unwrap();
    let probe = p.probe();
    assert!(
        probe.available,
        "Seatbelt muss auf macOS verfügbar sein: {probe:?}"
    );
    p
}

#[derive(Debug)]
#[allow(
    dead_code,
    reason = "stderr erscheint in den Fehlermeldungen der Asserts"
)]
struct Out {
    ok: bool,
    stdout: String,
    stderr: String,
}

async fn run_with(
    provider: &dyn SandboxProvider,
    policy: &ResolvedPolicy,
    cmd: CommandSpec,
) -> Out {
    let child = provider
        .spawn(policy, cmd.stdio(Stdio::Null, Stdio::Piped, Stdio::Piped))
        .unwrap();
    let out = child.output(Duration::from_secs(60)).await.unwrap();
    Out {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// `sh -c` in der Sandbox.
async fn sh(policy: &ResolvedPolicy, script: &str) -> Out {
    run_with(
        seatbelt().as_ref(),
        policy,
        CommandSpec::new("/bin/sh")
            .args(["-c", script])
            .cwd(&policy.workspace),
    )
    .await
}

/// Kontrollgruppe: derselbe Befehl ohne Sandbox (Fixture-Home als `HOME`).
async fn control(fx: &Fx, script: &str) -> Out {
    let policy = ResolvedPolicy::unsandboxed(Stage::Tools);
    let mut cmd = CommandSpec::new("/bin/sh").args(["-c", script]).cwd(&fx.ws);
    cmd.env.clear = true;
    cmd.env.set = vec![
        ("HOME".into(), fx.home.display().to_string()),
        ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
    ];
    run_with(&NoneProvider, &policy, cmd).await
}

#[tokio::test]
async fn sbx_004_ac1_home_grant_masks_ssh_key_in_both_stages() {
    let fx = fx();
    let attack = "cat \"$HOME/.ssh/id_ed25519\"";
    let control_out = control(&fx, attack).await;
    assert!(
        control_out.ok && control_out.stdout.contains(FIXTURE_SSH),
        "Kontrollgruppe: {control_out:?}"
    );
    for stage in [Stage::Harness, Stage::Tools] {
        let policy = fx.policy("read_paths: [\"~\"]\n", stage);
        let out = sh(&policy, attack).await;
        assert!(!out.ok, "{stage:?}: {out:?}");
        assert!(
            !out.stdout.contains(FIXTURE_SSH),
            "{stage:?}: Secret gelesen"
        );
        let out = sh(
            &policy,
            "ls \"$HOME/Documents\" && cat \"$HOME/Documents/notiz.txt\"",
        )
        .await;
        assert!(
            out.ok && out.stdout.contains("notiz.txt"),
            "{stage:?}: {out:?}"
        );
        // Auch Listing, Kopieren und Dotfiles direkt im Grant scheitern.
        for script in [
            "ls \"$HOME/.ssh\"",
            "cp \"$HOME/.ssh/id_ed25519\" ./k && cat ./k",
            "cat \"$HOME/.aws/credentials\"",
            "cat \"$HOME/.config/gh/hosts.yml\"",
            "cat \"$HOME/.zshrc\"",
        ] {
            let out = sh(&policy, script).await;
            assert!(
                !out.ok && !out.stdout.contains("FIXTURE"),
                "{stage:?}: {script}: {out:?}"
            );
        }
    }
}

#[tokio::test]
async fn sbx_004_allow_hidden_unmasks_only_the_named_dotfile() {
    let fx = fx();
    write(&fx.home.join(".cargo/config.toml"), "[build]\n");
    let policy = fx.policy(
        "read_paths: [\"~\"]\nallow_hidden: [.cargo]\n",
        Stage::Tools,
    );
    let out = sh(&policy, "cat \"$HOME/.cargo/config.toml\"").await;
    assert!(out.ok, "{out:?}");
    let out = sh(&policy, "cat \"$HOME/.zshrc\"").await;
    assert!(!out.ok, "{out:?}");
    // `allow_hidden` hebt keine Default-Maske auf.
    let policy = fx.policy("read_paths: [\"~\"]\nallow_hidden: [.ssh]\n", Stage::Tools);
    let out = sh(&policy, "cat \"$HOME/.ssh/id_ed25519\"").await;
    assert!(!out.ok && !out.stdout.contains(FIXTURE_SSH), "{out:?}");
}

#[tokio::test]
async fn sbx_004_ac2_dotenv_in_workspace_is_masked_by_default() {
    let fx = fx();
    let policy = fx.policy("{}\n", Stage::Tools);
    let out = sh(&policy, "cat .env").await;
    assert!(!out.ok && !out.stdout.contains(FIXTURE_ENV), "{out:?}");
    // Andere Dotfiles des Workspace bleiben sichtbar.
    let out = sh(&policy, "cat .github/workflows/ci.yml && cat src/main.rs").await;
    assert!(out.ok, "{out:?}");
    // Auch tiefer und als `.env.*`.
    write(&fx.ws.join("app/.env.local"), FIXTURE_ENV);
    let out = sh(&policy, "cat app/.env.local").await;
    assert!(!out.ok, "{out:?}");
    let open = fx.policy("masks: []\nallow_hidden: [.env]\n", Stage::Tools);
    let out = sh(&open, "cat .env").await;
    assert!(out.ok && out.stdout.contains(FIXTURE_ENV), "{out:?}");
}

#[tokio::test]
async fn sbx_004_ac3_codex_auth_readable_only_in_codex_stage_one() {
    let fx = fx();
    let attack = "cat \"$HOME/.codex/auth.json\"";
    let tools = fx.policy("{}\n", Stage::Tools);
    let out = sh(&tools, attack).await;
    assert!(!out.ok && !out.stdout.contains(FIXTURE_CODEX), "{out:?}");
    // Auch mit `read_paths: ["~"]` und `allow_hidden: [.codex]` bleibt es in Stufe 2 maskiert.
    let tools = fx.policy(
        "read_paths: [\"~\"]\nallow_hidden: [.codex]\n",
        Stage::Tools,
    );
    let out = sh(&tools, attack).await;
    assert!(!out.ok && !out.stdout.contains(FIXTURE_CODEX), "{out:?}");

    let mut ctx = fx.ctx(Stage::Harness);
    ctx.harness = beton_sandbox::masks::harness_grants("codex");
    let harness = fx.policy_with("{}\n", ctx);
    let out = sh(&harness, attack).await;
    assert!(out.ok && out.stdout.contains(FIXTURE_CODEX), "{out:?}");
    // Die Credentials von Claude bleiben auch in der Codex-Stufe 1 unsichtbar.
    let out = sh(&harness, "cat \"$HOME/.claude/.credentials.json\"").await;
    assert!(!out.ok, "{out:?}");
}

#[tokio::test]
async fn sbx_004_ac4_masks_survive_rename_hardlink_and_symlink_attempts() {
    let fx = fx();
    // Schreib-Wurzel, die eine Default-Maske enthält.
    let policy = fx.policy(
        "read_paths: [\"~\"]\nwrite_paths: [\"~/.config\"]\nallow_hidden: [.config]\n",
        Stage::Tools,
    );
    let attacks = [
        // Maskiertes Verzeichnis umbenennen bzw. das Elternverzeichnis.
        "mv \"$HOME/.config/gh\" \"$HOME/.config/gh2\"; cat \"$HOME/.config/gh2/hosts.yml\"",
        "mv \"$HOME/.config\" \"$HOME/.config2\"; cat \"$HOME/.config2/gh/hosts.yml\"",
        // Hardlink aus der Maske in eine Schreib-Wurzel.
        "ln \"$HOME/.config/gh/hosts.yml\" \"$HOME/.config/h\"; cat \"$HOME/.config/h\"",
        "ln \"$HOME/.ssh/id_ed25519\" ./k; cat ./k",
        // Symlink im Workspace auf die Maske.
        "ln -s \"$HOME/.ssh\" ./s; cat ./s/id_ed25519",
        "ln -s \"$HOME/.config/gh\" ./g; cat ./g/hosts.yml",
    ];
    for attack in attacks {
        let out = sh(&policy, attack).await;
        assert!(!out.stdout.contains("FIXTURE"), "{attack}: {out:?}");
    }
    assert!(
        fx.home.join(".config/gh/hosts.yml").is_file(),
        "Maske wurde verschoben"
    );
    assert!(!fx.home.join(".config2").exists());

    // Eigene Maske tief im Workspace: Umbenennen der Elternverzeichnisse hilft nicht.
    write(&fx.ws.join("a/b/geheim.txt"), "FIXTURE-OWN");
    let own = format!("masks: [\"{}\"]\n", fx.ws.join("a/b/geheim.txt").display());
    let policy = fx.policy(&own, Stage::Tools);
    for attack in [
        "cat a/b/geheim.txt",
        "mv a z; cat z/b/geheim.txt",
        "mv a/b a/c; cat a/c/geheim.txt",
        "ln a/b/geheim.txt x; cat x",
    ] {
        let out = sh(&policy, attack).await;
        assert!(!out.stdout.contains("FIXTURE"), "{attack}: {out:?}");
    }
    // Normale Arbeit in denselben Verzeichnissen geht weiter.
    let out = sh(
        &policy,
        "echo ok > a/b/neu.txt && cat a/b/neu.txt && rm a/b/neu.txt",
    )
    .await;
    assert!(out.ok && out.stdout.contains("ok"), "{out:?}");
    // Kontrollgruppe: ohne Sandbox gelingt der Umweg.
    let c = control(&fx, "mv a z && cat z/b/geheim.txt").await;
    assert!(c.ok && c.stdout.contains("FIXTURE-OWN"), "{c:?}");
}

#[tokio::test]
async fn sbx_007_ac4_launch_agents_and_shell_rc_are_not_writable_in_both_stages() {
    let fx = fx();
    let attack =
        "echo pwned > \"$HOME/Library/LaunchAgents/x.plist\"; echo pwned >> \"$HOME/.zshrc\"";
    let c = control(
        &fx,
        "echo ok > \"$HOME/Library/LaunchAgents/c.plist\" && echo ok >> \"$HOME/.zshrc\"",
    )
    .await;
    assert!(c.ok, "{c:?}");
    std::fs::remove_file(fx.home.join("Library/LaunchAgents/c.plist")).unwrap();
    write(&fx.home.join(".zshrc"), "# fixture\n");
    let mut stage_one = fx.ctx(Stage::Harness);
    stage_one.harness = beton_sandbox::masks::harness_grants("claude");
    for policy in [
        fx.policy("read_paths: [\"~\"]\n", Stage::Tools),
        fx.policy_with("read_paths: [\"~\"]\n", stage_one),
    ] {
        let out = sh(&policy, attack).await;
        assert!(!out.ok, "{:?}: {out:?}", policy.stage);
        assert!(!fx.home.join("Library/LaunchAgents/x.plist").exists());
        assert_eq!(
            std::fs::read_to_string(fx.home.join(".zshrc")).unwrap(),
            "# fixture\n"
        );
    }
}

#[tokio::test]
async fn sbx_007_writes_outside_the_workspace_fail() {
    let fx = fx();
    let policy = fx.policy("read_paths: [\"~\"]\n", Stage::Tools);
    for (attack, target) in [
        (
            "echo x > \"$HOME/Documents/neu.txt\"",
            fx.home.join("Documents/neu.txt"),
        ),
        ("echo x > ../draussen.txt", fx.root.join("draussen.txt")),
        (
            "echo x > /private/tmp/beton-sbx-escape.txt",
            PathBuf::from("/private/tmp/beton-sbx-escape.txt"),
        ),
    ] {
        let out = sh(&policy, attack).await;
        assert!(!out.ok, "{attack}: {out:?}");
        assert!(!target.exists(), "{attack}");
    }
    let out = sh(&policy, "echo x > /etc/beton-test").await;
    assert!(!out.ok);
    // Scratch und Workspace sind beschreibbar; `TMPDIR` zeigt auf den Scratch.
    let out = sh(
        &policy,
        "echo x > ws.txt && echo y > \"$TMPDIR/t\" && cat \"$TMPDIR/t\"",
    )
    .await;
    assert!(out.ok && out.stdout.contains('y'), "{out:?}");
    assert!(fx.scratch.join("t").is_file());
}

#[tokio::test]
async fn sbx_007_signals_only_reach_the_own_tree() {
    let fx = fx();
    let policy = fx.policy("{}\n", Stage::Tools);
    let me = std::process::id();
    let out = sh(&policy, &format!("kill -0 {me}")).await;
    assert!(
        !out.ok,
        "Signal an den Runner (Elternprozess) muss scheitern: {out:?}"
    );
    let out = sh(&policy, "sleep 30 & kill $! && wait $!; echo eigenes-kind").await;
    assert!(out.stdout.contains("eigenes-kind"), "{out:?}");
    let c = control(&fx, &format!("kill -0 {me}")).await;
    assert!(c.ok, "{c:?}");
}

/// Probe für den folgenden Test: liest argv und Environment eines fremden Prozesses über
/// `sysctl(KERN_PROCARGS2)` (das macOS-Gegenstück zu `/proc/<pid>/environ`). Tut nur etwas,
/// wenn der Test dieses Binary mit `BETON_SBX_PROBE_PID` in der Sandbox startet.
#[test]
#[allow(unsafe_code, reason = "sysctl(2) gibt es nur über libc")]
fn procargs_probe() {
    let Ok(pid) = std::env::var("BETON_SBX_PROBE_PID") else {
        return;
    };
    let marker = std::env::var("BETON_SBX_PROBE_MARKER").unwrap();
    let pid: libc::c_int = pid.parse().unwrap();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut buf = vec![0u8; 256 * 1024];
    let mut len = buf.len();
    // SAFETY: mib und buf sind gültig, len enthält die Puffergröße.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        println!("PROBE-DENIED {}", std::io::Error::last_os_error());
        return;
    }
    let found = buf[..len]
        .windows(marker.len())
        .any(|w| w == marker.as_bytes());
    println!("PROBE-READ {}", if found { "LEAK" } else { "ohne Marker" });
}

/// Zielprozess für die Probe: ein Nicht-Plattform-Binary (wie Runner oder Vendor-CLI); bei
/// Plattform-Binaries wie `/bin/sh` blendet macOS das Environment ohnehin aus.
#[test]
fn procargs_host() {
    if std::env::var_os("BETON_SBX_HOST").is_some() {
        std::thread::sleep(Duration::from_secs(60));
    }
}

#[tokio::test]
async fn sbx_007_foreign_processes_arguments_and_environment_are_invisible() {
    let fx = fx();
    let marker = format!("FIXTURE-PROCARGS-{}", std::process::id());
    // Ein Prozess des Benutzers außerhalb der Sandbox (z. B. Runner oder Harness) mit einem
    // Secret im Environment.
    let exe = std::env::current_exe().unwrap();
    let mut host = std::process::Command::new(&exe)
        .args(["--exact", "procargs_host", "--test-threads=1"])
        .env("BETON_SBX_HOST", "1")
        .env("BETON_FIXTURE_SECRET", &marker)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let probe = |policy: &ResolvedPolicy| {
        CommandSpec::new(&exe)
            .args([
                "--exact",
                "procargs_probe",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("BETON_SBX_PROBE_PID", host.id().to_string())
            .env("BETON_SBX_PROBE_MARKER", &marker)
            .cwd(&policy.workspace)
    };
    // Kontrollgruppe: ohne Sandbox ist das Environment lesbar.
    let none = ResolvedPolicy::unsandboxed(Stage::Tools);
    let mut cmd = probe(&fx.policy("{}\n", Stage::Tools));
    cmd.cwd = Some(fx.ws.clone());
    let c = run_with(&NoneProvider, &none, cmd).await;
    assert!(
        c.stdout.contains("PROBE-READ LEAK"),
        "Kontrollgruppe: {c:?}"
    );
    let grant = format!("read_paths: [\"{}\"]\n", exe.parent().unwrap().display());
    for stage in [Stage::Harness, Stage::Tools] {
        let policy = fx.policy(&grant, stage);
        let out = run_with(seatbelt().as_ref(), &policy, probe(&policy)).await;
        assert!(
            out.stdout.contains("PROBE-"),
            "{stage:?}: Probe lief nicht: {out:?}"
        );
        assert!(!out.stdout.contains("LEAK"), "{stage:?}: {out:?}");
    }
    let _ = host.kill();
    let _ = host.wait();
}

/// Git-Binary für die Tests (Xcode bzw. Homebrew) und die Lese-Wurzel seines Präfixes.
fn git_binary() -> (PathBuf, PathBuf) {
    let found = std::process::Command::new("/usr/bin/xcrun")
        .args(["-f", "git"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .or_else(|| {
            std::process::Command::new("/usr/bin/which")
                .arg("git")
                .output()
                .ok()
                .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        })
        .expect("git");
    let git = found.canonicalize().unwrap();
    let prefix = if git.starts_with("/opt/homebrew") {
        PathBuf::from("/opt/homebrew")
    } else if git.starts_with("/usr/local") {
        PathBuf::from("/usr/local")
    } else {
        // …/Contents/Developer/usr/bin/git → …/Contents/Developer
        git.ancestors().nth(3).unwrap().to_path_buf()
    };
    (git, prefix)
}

fn git_host(git: &Path, dir: &Path, args: &[&str]) {
    let out = std::process::Command::new(git)
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

const GIT_ENV: &str = "export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null \
                       GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t;";

#[tokio::test]
async fn sbx_003_ac4_git_hooks_are_read_only_but_commits_work() {
    let fx = fx();
    let (git, prefix) = git_binary();
    git_host(&git, &fx.ws, &["init", "-q"]);
    git_host(
        &git,
        &fx.ws,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    let mut ctx = fx.ctx(Stage::Tools);
    ctx.git = GitLayout::discover(&fx.ws).unwrap();
    let yaml = format!("read_paths: [\"{}\"]\n", prefix.display());
    let policy = fx.policy_with(&yaml, ctx);
    let g = git.display();
    let out = sh(
        &policy,
        &format!(
            "{GIT_ENV} echo a > a.txt && {g} add a.txt && {g} commit -q -m c1 && {g} log --oneline"
        ),
    )
    .await;
    assert!(out.ok && out.stdout.contains("c1"), "{out:?}");
    for attack in [
        "echo 'echo pwned' > .git/hooks/pre-commit",
        "mkdir -p .git/hooks/x",
        "rm -rf .git/hooks",
        "mv .git/hooks .git/h2",
        &format!("{GIT_ENV} {g} config core.hooksPath /tmp"),
        "echo '[core]' >> .git/config",
        "mv .git .git-alt",
    ] {
        let out = sh(&policy, attack).await;
        assert!(!out.ok, "{attack}: {out:?}");
    }
    assert!(!fx.ws.join(".git/hooks/pre-commit").exists());
    assert!(fx.ws.join(".git/HEAD").is_file());
}

#[tokio::test]
async fn sbx_003_ac4_commits_in_a_worktree_reach_the_object_db_but_not_hooks() {
    let fx = fx();
    let (git, prefix) = git_binary();
    let repo = fx.root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git_host(&git, &repo, &["init", "-q"]);
    git_host(
        &git,
        &repo,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    let wt = fx.ws.join("wt");
    git_host(
        &git,
        &repo,
        &["worktree", "add", "-q", "-b", "agent", wt.to_str().unwrap()],
    );
    let wt = wt.canonicalize().unwrap();
    let mut ctx = ResolveCtx::new(Stage::Tools, &wt, &fx.home, &fx.scratch);
    ctx.git = GitLayout::discover(&wt).unwrap();
    let spec =
        SandboxSpec::from_yaml(&format!("read_paths: [\"{}\"]\n", prefix.display())).unwrap();
    let policy = seatbelt().resolve(&spec, &ctx).unwrap();
    let g = git.display();
    let out = sh(&policy, &format!("{GIT_ENV} echo a > a.txt && {g} add a.txt && {g} commit -q -m im-worktree && {g} log --oneline -1")).await;
    assert!(out.ok && out.stdout.contains("im-worktree"), "{out:?}");
    for attack in [
        format!("echo x > {}/.git/hooks/post-checkout", repo.display()),
        format!("echo x >> {}/.git/config", repo.display()),
        "echo 'gitdir: /tmp' > .git".to_owned(),
        // Das Haupt-Checkout liegt außerhalb des Worktrees (SES-015).
        format!("echo x > {}/main.txt", repo.display()),
    ] {
        let out = sh(&policy, &attack).await;
        assert!(!out.ok, "{attack}: {out:?}");
    }
    assert!(!repo.join("main.txt").exists());
    assert!(
        std::fs::read_to_string(wt.join(".git"))
            .unwrap()
            .starts_with("gitdir: ")
    );
}

/// Lokaler TLS-Upstream, der auf jeden Request mit `ok` antwortet.
struct Upstream {
    addr: std::net::SocketAddr,
    ca_pem: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn tls_upstream() -> Upstream {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, PKCS_ECDSA_P256_SHA256,
    };
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut ca = CertificateParams::default();
    ca.distinguished_name
        .push(DnType::CommonName, "beton sandbox test upstream CA");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let leaf = CertificateParams::new(vec!["127.0.0.1".to_owned()])
        .unwrap()
        .signed_by(&key, &Issuer::from_params(&ca, &ca_key))
        .unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone(), ca_cert.der().clone()],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let _ = tls
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nupstream\n")
                    .await;
                let _ = tls.shutdown().await;
            });
        }
    });
    Upstream {
        addr,
        ca_pem: ca_cert.pem(),
        task,
    }
}

#[tokio::test]
async fn sbx_007_ac2_network_only_through_the_proxy() {
    let fx = fx();
    let up = tls_upstream().await;
    let upstream_ca = fx.root.join("upstream-ca.pem");
    std::fs::write(&upstream_ca, &up.ca_pem).unwrap();
    let ca = beton_proxy::BetonCa::generate("sbx-test", time::OffsetDateTime::now_utc()).unwrap();
    let proxy = beton_proxy::EgressProxy::new(
        ca,
        &beton_proxy::ProxyOptions {
            extra_ca_files: vec![upstream_ca.clone()],
        },
    )
    .unwrap();
    let port = up.addr.port();
    let instance = proxy
        .start(beton_proxy::InstanceSpec {
            label: "sbx-test/tools".into(),
            rules: beton_proxy::RuleSet::compile(&[format!("GET 127.0.0.1:{port}/**")]).unwrap(),
            allow_private: beton_proxy::PrivateAllowList::parse(&[format!("127.0.0.1:{port}")])
                .unwrap(),
            ..beton_proxy::InstanceSpec::default()
        })
        .await
        .unwrap();
    // CA-Bundles im Scratch (dort darf die Sandbox lesen).
    let beton_ca = fx.scratch.join("beton-ca.pem");
    std::fs::write(&beton_ca, proxy.ca_pem()).unwrap();
    std::fs::copy(&upstream_ca, fx.scratch.join("upstream-ca.pem")).unwrap();

    let mut ctx = fx.ctx(Stage::Tools);
    ctx.proxy = Some(ProxyEndpoint {
        port: instance.addr().port(),
    });
    let policy = fx.policy_with("egress_rules: [\"GET 127.0.0.1/**\"]\n", ctx);
    let url = format!("https://127.0.0.1:{port}/ok");
    let run = |script: String| {
        let policy = policy.clone();
        let proxy_url = instance.proxy_url().to_string();
        async move {
            let cmd = CommandSpec::new("/bin/sh")
                .args(["-c", &script])
                .cwd(&policy.workspace)
                .env("HTTPS_PROXY", proxy_url);
            run_with(seatbelt().as_ref(), &policy, cmd).await
        }
    };
    // Kontrollgruppe: Der Upstream ist ohne Sandbox direkt erreichbar.
    let c = control(
        &fx,
        &format!(
            "curl -sS --max-time 10 --cacert {} {url}",
            upstream_ca.display()
        ),
    )
    .await;
    assert!(c.ok && c.stdout.contains("upstream"), "{c:?}");
    // Ohne Proxy-Variable: Verbindungsfehler (Exit 7), auch mit der richtigen CA.
    let out = run(format!(
        "env -u HTTPS_PROXY curl -sS --max-time 10 --cacert \"$TMPDIR/upstream-ca.pem\" {url}"
    ))
    .await;
    assert!(!out.ok && !out.stdout.contains("upstream"), "{out:?}");
    let out = run(format!(
        "curl -sS --noproxy '*' --max-time 10 --cacert \"$TMPDIR/upstream-ca.pem\" {url}"
    ))
    .await;
    assert!(!out.ok, "{out:?}");
    // Mit Proxy und passender Regel gelingt es (TLS terminiert der Proxy mit der beton-CA).
    let out = run(format!(
        "curl -sS --max-time 10 --cacert \"$TMPDIR/beton-ca.pem\" {url}"
    ))
    .await;
    assert!(out.ok && out.stdout.contains("upstream"), "{out:?}");
    // Ohne passende Regel lehnt der Proxy ab.
    let out = run(format!(
        "curl -sS -f --max-time 10 --cacert \"$TMPDIR/beton-ca.pem\" -X POST {url}"
    ))
    .await;
    assert!(!out.ok, "{out:?}");
    // Direktverbindung an eine Nicht-Loopback-Adresse (TEST-NET-1, kein öffentlicher Host).
    let out = run("nc -z -w 3 192.0.2.1 443".into()).await;
    assert!(
        !out.ok,
        "Direktverbindung an eine Nicht-Loopback-Adresse: {out:?}"
    );
}

#[tokio::test]
async fn sbx_007_ac5_other_localhost_ports_are_unreachable() {
    let fx = fx();
    let host_service = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let service_port = host_service.local_addr().unwrap().port();
    let proxy_like = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_port = proxy_like.local_addr().unwrap().port();
    let accept = tokio::spawn(async move {
        loop {
            tokio::select! {
                r = host_service.accept() => { if r.is_err() { break } }
                r = proxy_like.accept() => { if r.is_err() { break } }
            }
        }
    });
    let mut ctx = fx.ctx(Stage::Tools);
    ctx.proxy = Some(ProxyEndpoint { port: proxy_port });
    let policy = fx.policy_with("{}\n", ctx);
    let c = control(&fx, &format!("nc -z -w 3 127.0.0.1 {service_port}")).await;
    assert!(c.ok, "Kontrollgruppe: {c:?}");
    let out = sh(&policy, &format!("nc -z -w 3 127.0.0.1 {service_port}")).await;
    assert!(!out.ok, "Host-Dienst erreichbar: {out:?}");
    let out = sh(&policy, &format!("nc -z -w 3 localhost {service_port}")).await;
    assert!(!out.ok, "{out:?}");
    let out = sh(&policy, &format!("nc -z -w 3 127.0.0.1 {proxy_port}")).await;
    assert!(out.ok, "Proxy-Port muss erreichbar sein: {out:?}");
    // Ohne Proxy gibt es gar kein Netz.
    let none = fx.policy("{}\n", Stage::Tools);
    let out = sh(&none, &format!("nc -z -w 3 127.0.0.1 {proxy_port}")).await;
    assert!(!out.ok, "{out:?}");
    accept.abort();
}

#[tokio::test]
async fn sbx_007_unix_sockets_listening_and_apple_events_are_blocked() {
    let fx = fx();
    // Ein Host-Dienst auf einem Unix-Socket (wie SSH-Agent oder Docker) im Workspace.
    let sock = fx.ws.join("dienst.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    std::thread::spawn(move || {
        // Jede Verbindung sofort schließen; `nc` endet dann.
        for conn in listener.incoming() {
            drop(conn);
        }
    });
    let connect = format!("nc -U '{}' </dev/null", sock.display());
    let c = control(&fx, &connect).await;
    assert!(c.ok, "Kontrollgruppe: {c:?}");
    let policy = fx.policy("read_paths: [\"~\"]\n", Stage::Tools);
    let out = sh(&policy, &connect).await;
    assert!(!out.ok, "Unix-Socket erreichbar: {out:?}");
    // Kein eigener Dienst in der Sandbox (bind/listen).
    let out = sh(
        &policy,
        "nc -l 127.0.0.1 0 & pid=$!; sleep 1; kill $pid 2>/dev/null; wait $pid",
    )
    .await;
    assert!(out.stderr.contains("Operation not permitted"), "{out:?}");
    // Keine Apple Events an andere Apps (z. B. „Terminal: do script“).
    let out = sh(
        &policy,
        "osascript -e 'tell application \"System Events\" to get name of every process'",
    )
    .await;
    assert!(!out.ok, "{out:?}");
}

/// Keychain-Test nur mit dem temporären Keychain der CI (`BETON_TEST_KEYCHAIN=1`); lokal
/// berührt kein Test den Keychain eines Entwicklers. In der CI fehlt er nie.
fn test_keychain() -> Option<PathBuf> {
    match std::env::var("BETON_TEST_KEYCHAIN").as_deref() {
        Ok("1") => Some(PathBuf::from(
            std::env::var("BETON_TEST_KEYCHAIN_PATH").expect("BETON_TEST_KEYCHAIN_PATH"),
        )),
        _ => {
            assert!(
                std::env::var_os("CI").is_none(),
                "In der CI muss der temporäre Keychain gesetzt sein (BETON_TEST_KEYCHAIN)"
            );
            eprintln!("SBX-007 AC1: lokal ohne temporären Keychain nur die Profilprüfung");
            None
        }
    }
}

#[tokio::test]
async fn sbx_007_ac1_keychain_only_in_stage_one_with_declared_need() {
    let fx = fx();
    // Profil: Stufe 2 bekommt die Keychain-Dienste nie, auch wenn der Harness sie deklariert.
    let mut tools_ctx = fx.ctx(Stage::Tools);
    tools_ctx.harness = beton_sandbox::masks::harness_grants("claude");
    let tools = fx.policy_with("{}\n", tools_ctx);
    let text = beton_sandbox::seatbelt::profile::render(&tools)
        .unwrap()
        .text;
    assert!(!text.contains("SecurityServer") && !text.contains("securityd"));
    let Some(kc) = test_keychain() else { return };
    let kc_s = kc.display().to_string();
    let add = std::process::Command::new("/usr/bin/security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            "Claude Code-credentials",
            "-a",
            "beton-test",
            "-w",
            "FIXTURE-KEYCHAIN",
            &kc_s,
        ])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let query =
        format!("/usr/bin/security find-generic-password -s 'Claude Code-credentials' -w '{kc_s}'");
    let c = control(&fx, &query).await;
    assert!(
        c.ok && c.stdout.contains("FIXTURE-KEYCHAIN"),
        "Kontrollgruppe: {c:?}"
    );
    let grant = format!("read_paths: [\"{}\"]\n", kc.parent().unwrap().display());
    let mut tools_ctx = fx.ctx(Stage::Tools);
    tools_ctx.harness = beton_sandbox::masks::harness_grants("claude");
    let tools = fx.policy_with(&grant, tools_ctx);
    let out = sh(&tools, &query).await;
    assert!(
        !out.ok && !out.stdout.contains("FIXTURE-KEYCHAIN"),
        "Stufe 2: {out:?}"
    );
    let mut harness_ctx = fx.ctx(Stage::Harness);
    harness_ctx.harness = beton_sandbox::masks::harness_grants("claude");
    let harness = fx.policy_with(&grant, harness_ctx);
    let out = sh(&harness, &query).await;
    assert!(
        out.ok && out.stdout.contains("FIXTURE-KEYCHAIN"),
        "Stufe 1: {out:?}"
    );
}

#[tokio::test]
async fn sbx_007_environment_is_not_inherited() {
    let fx = fx();
    let mut ctx = fx.ctx(Stage::Tools);
    ctx.host_env = [
        ("AWS_SECRET_ACCESS_KEY", "FIXTURE-AWS-ENV"),
        ("PATH", "/usr/bin:/bin"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let policy = fx.policy_with("{}\n", ctx);
    // Auch ein Aufrufer kann gesperrte Variablen nicht durchreichen.
    let cmd = CommandSpec::new("/usr/bin/env")
        .env("DYLD_INSERT_LIBRARIES", "/tmp/x.dylib")
        .env("SSH_AUTH_SOCK", "/tmp/agent")
        .cwd(&policy.workspace);
    let out = run_with(seatbelt().as_ref(), &policy, cmd).await;
    assert!(out.ok, "{out:?}");
    assert!(!out.stdout.contains("FIXTURE-AWS-ENV"));
    assert!(
        !out.stdout.contains("DYLD_") && !out.stdout.contains("SSH_AUTH_SOCK"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("BETON_STAGE=tools"));
}

mod weird_names {
    use proptest::prelude::*;

    use super::*;

    fn name() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just("\"".to_owned()),
                Just(")".to_owned()),
                Just("(".to_owned()),
                Just("\n".to_owned()),
                Just("\\".to_owned()),
                Just(".".to_owned()),
                Just("*".to_owned()),
                Just("[".to_owned()),
                Just("$".to_owned()),
                Just("ä".to_owned()),
                Just(" ".to_owned()),
                Just("#|".to_owned()),
                "[a-z]{1,3}",
            ],
            1..6,
        )
        .prop_map(|p| format!("ws{}", p.concat()))
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

        /// SBX-007 AC3: Workspace-Namen mit `"`, `)`, Zeilenumbruch u. a. ergeben ein gültiges
        /// Profil, das genau diesen Workspace freigibt und seine Masken durchsetzt.
        #[test]
        fn sbx_007_ac3_weird_workspace_names_yield_a_valid_tight_profile(n in name()) {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                let fx = fx();
                let ws = fx.root.join(&n);
                std::fs::create_dir_all(&ws).unwrap();
                write(&ws.join(".env"), FIXTURE_ENV);
                // Nachbar, den ein falsch maskierter Ausdruck mit erfassen würde.
                let sibling = fx.root.join(format!("{n}x"));
                write(&sibling.join("nachbar.txt"), "FIXTURE-NACHBAR");
                let ctx = ResolveCtx::new(Stage::Tools, &ws, &fx.home, &fx.scratch);
                let policy = seatbelt().resolve(&SandboxSpec::default(), &ctx).unwrap();
                let out = sh(&policy, "echo inhalt > datei && cat datei").await;
                prop_assert!(out.ok && out.stdout.contains("inhalt"), "{:?}: {:?}", n, out);
                let out = sh(&policy, "cat .env").await;
                prop_assert!(!out.ok && !out.stdout.contains(FIXTURE_ENV), "{:?}: {:?}", n, out);
                let script = format!("cat '{}'", sibling.join("nachbar.txt").display().to_string().replace('\'', "'\\''"));
                let out = sh(&policy, &script).await;
                prop_assert!(!out.ok && !out.stdout.contains("FIXTURE-NACHBAR"), "{:?}: {:?}", n, out);
                Ok(())
            })?;
        }
    }
}
