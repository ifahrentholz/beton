//! SBX-008 (Landlock + seccomp + Namespaces), SBX-009 AC1 (dieselbe Suite unter bubblewrap) und
//! die Kernel-Durchsetzung von SBX-003 AC4 und SBX-004 unter Linux.
//!
//! Jeder Fall läuft gegen jedes Backend aus `BETON_TEST_LINUX_BACKENDS` (Default `landlock`,
//! dazu `bwrap`, falls installiert); ein dort genanntes Backend **muss** verfügbar sein, sonst
//! scheitert der Test. Die CI setzt die Liste ausdrücklich, damit nichts still übersprungen
//! wird. Jeder Angriff hat eine Kontrollgruppe ohne Sandbox (QA-005 AC1). Alle Homes und
//! Secrets sind Fixtures in Temp-Verzeichnissen; Netz-Tests laufen nur gegen lokale Upstreams
//! über den Egress-Proxy aus `beton-proxy` (ADR-0033).
//!
//! Diese Tests werden nie übersprungen oder abgeschwächt, damit die CI grün wird (AGENTS.md).

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use beton_sandbox::linux::{LinuxConfig, bwrap_provider, landlock_provider};
use beton_sandbox::{
    BackendChoice, CommandSpec, GitLayout, ProviderId, ProxyEndpoint, Registry, ResolveCtx,
    ResolvedPolicy, SandboxProvider, SandboxSpec, Stage,
};
use support::*;

fn launcher() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_beton-sandbox-exec"))
}

fn config() -> LinuxConfig {
    LinuxConfig::new(launcher())
}

/// Die zu prüfenden Backends; jedes muss verfügbar sein.
fn backends() -> Vec<Arc<dyn SandboxProvider>> {
    let names = std::env::var("BETON_TEST_LINUX_BACKENDS").ok();
    let explicit = names.is_some();
    let names: Vec<String> = names
        .unwrap_or_else(|| "landlock,bwrap".into())
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    let mut out = Vec::new();
    for name in names {
        let p = match name.as_str() {
            "landlock" => landlock_provider(config()),
            "bwrap" => bwrap_provider(config()),
            other => panic!("unbekanntes Backend {other}"),
        };
        let probe = p.probe();
        if !probe.available && name == "bwrap" && !explicit {
            // Lokal ohne bwrap: nur Landlock. Die CI nennt bwrap ausdrücklich.
            eprintln!(
                "bwrap lokal nicht verfügbar: {:?}",
                probe.unavailable_reason
            );
            continue;
        }
        assert!(probe.available, "{name} muss verfügbar sein: {probe:?}");
        out.push(p);
    }
    assert!(!out.is_empty(), "kein Linux-Backend zu prüfen");
    out
}

#[tokio::test]
async fn sbx_004_ac1_home_grant_masks_ssh_key_in_both_stages() {
    let fx = fx();
    let attack = "cat \"$HOME/.ssh/id_ed25519\"";
    let c = control(&fx, attack).await;
    assert!(
        c.ok && c.stdout.contains(FIXTURE_SSH),
        "Kontrollgruppe: {c:?}"
    );
    for p in backends() {
        for stage in [Stage::Harness, Stage::Tools] {
            let policy = fx.policy(p.as_ref(), "read_paths: [\"~\"]\n", stage);
            let out = sh(p.as_ref(), &policy, attack).await;
            assert!(!out.ok, "{} {stage:?}: {out:?}", p.id());
            assert!(
                !out.stdout.contains(FIXTURE_SSH),
                "{}: Secret gelesen",
                p.id()
            );
            let out = sh(
                p.as_ref(),
                &policy,
                "ls \"$HOME/Documents\" && cat \"$HOME/Documents/notiz.txt\"",
            )
            .await;
            assert!(
                out.ok && out.stdout.contains("notiz.txt"),
                "{} {stage:?}: {out:?}",
                p.id()
            );
            for script in [
                "ls \"$HOME/.ssh\"",
                "cp \"$HOME/.ssh/id_ed25519\" ./k && cat ./k",
                "cat \"$HOME/.aws/credentials\"",
                "cat \"$HOME/.config/gh/hosts.yml\"",
                "cat \"$HOME/.netrc\"",
                "cat \"$HOME/.zshrc\"",
            ] {
                let out = sh(p.as_ref(), &policy, script).await;
                assert!(
                    !out.ok && !out.stdout.contains("FIXTURE"),
                    "{} {stage:?}: {script}: {out:?}",
                    p.id()
                );
            }
        }
    }
}

#[tokio::test]
async fn sbx_004_allow_hidden_unmasks_only_the_named_dotfile() {
    let fx = fx();
    write(&fx.home.join(".cargo/config.toml"), "[build]\n");
    for p in backends() {
        let policy = fx.policy(
            p.as_ref(),
            "read_paths: [\"~\"]\nallow_hidden: [.cargo]\n",
            Stage::Tools,
        );
        let out = sh(p.as_ref(), &policy, "cat \"$HOME/.cargo/config.toml\"").await;
        assert!(out.ok, "{}: {out:?}", p.id());
        let out = sh(p.as_ref(), &policy, "cat \"$HOME/.zshrc\"").await;
        assert!(!out.ok, "{}: {out:?}", p.id());
        let policy = fx.policy(
            p.as_ref(),
            "read_paths: [\"~\"]\nallow_hidden: [.ssh]\n",
            Stage::Tools,
        );
        let out = sh(p.as_ref(), &policy, "cat \"$HOME/.ssh/id_ed25519\"").await;
        assert!(!out.ok && !out.stdout.contains(FIXTURE_SSH), "{out:?}");
    }
}

#[tokio::test]
async fn sbx_004_ac2_dotenv_in_workspace_is_masked_by_default() {
    let fx = fx();
    write(&fx.ws.join("app/.env.local"), FIXTURE_ENV);
    let c = control(&fx, "cat .env && cat app/.env.local").await;
    assert!(
        c.ok && c.stdout.contains(FIXTURE_ENV),
        "Kontrollgruppe: {c:?}"
    );
    for p in backends() {
        let policy = fx.policy(p.as_ref(), "{}\n", Stage::Tools);
        let out = sh(p.as_ref(), &policy, "cat .env").await;
        assert!(
            !out.ok && !out.stdout.contains(FIXTURE_ENV),
            "{}: {out:?}",
            p.id()
        );
        let out = sh(
            p.as_ref(),
            &policy,
            "cat .github/workflows/ci.yml && cat src/main.rs",
        )
        .await;
        assert!(out.ok, "{}: {out:?}", p.id());
        let out = sh(p.as_ref(), &policy, "cat app/.env.local").await;
        assert!(!out.ok, "{}: {out:?}", p.id());
        let open = fx.policy(
            p.as_ref(),
            "masks: []\nallow_hidden: [.env]\n",
            Stage::Tools,
        );
        let out = sh(p.as_ref(), &open, "cat .env").await;
        assert!(
            out.ok && out.stdout.contains(FIXTURE_ENV),
            "{}: {out:?}",
            p.id()
        );
    }
}

#[tokio::test]
async fn sbx_004_ac3_codex_auth_readable_only_in_codex_stage_one() {
    let fx = fx();
    let attack = "cat \"$HOME/.codex/auth.json\"";
    for p in backends() {
        let tools = fx.policy(
            p.as_ref(),
            "read_paths: [\"~\"]\nallow_hidden: [.codex]\n",
            Stage::Tools,
        );
        let out = sh(p.as_ref(), &tools, attack).await;
        assert!(
            !out.ok && !out.stdout.contains(FIXTURE_CODEX),
            "{}: {out:?}",
            p.id()
        );
        let mut ctx = fx.ctx(Stage::Harness);
        ctx.harness = beton_sandbox::masks::harness_grants("codex");
        let harness = fx.policy_with(p.as_ref(), "{}\n", ctx);
        let out = sh(p.as_ref(), &harness, attack).await;
        assert!(
            out.ok && out.stdout.contains(FIXTURE_CODEX),
            "{}: {out:?}",
            p.id()
        );
        let out = sh(
            p.as_ref(),
            &harness,
            "cat \"$HOME/.claude/.credentials.json\"",
        )
        .await;
        assert!(!out.ok, "{}: {out:?}", p.id());
    }
}

#[tokio::test]
async fn sbx_004_ac4_masks_survive_rename_hardlink_and_symlink_attempts() {
    for p in backends() {
        let fx = fx();
        let policy = fx.policy(
            p.as_ref(),
            "read_paths: [\"~\"]\nwrite_paths: [\"~/.config\"]\nallow_hidden: [.config]\n",
            Stage::Tools,
        );
        for attack in [
            "mv \"$HOME/.config/gh\" \"$HOME/.config/gh2\"; cat \"$HOME/.config/gh2/hosts.yml\"",
            "mv \"$HOME/.config\" \"$HOME/.config2\"; cat \"$HOME/.config2/gh/hosts.yml\"",
            "ln \"$HOME/.config/gh/hosts.yml\" \"$HOME/.config/h\"; cat \"$HOME/.config/h\"",
            "ln \"$HOME/.ssh/id_ed25519\" ./k; cat ./k",
            "ln -s \"$HOME/.ssh\" ./s; cat ./s/id_ed25519",
            "ln -s \"$HOME/.config/gh\" ./g; cat ./g/hosts.yml",
            "cat /proc/self/root\"$HOME/.ssh/id_ed25519\"",
        ] {
            let out = sh(p.as_ref(), &policy, attack).await;
            assert!(
                !out.stdout.contains("FIXTURE"),
                "{}: {attack}: {out:?}",
                p.id()
            );
        }
        assert!(
            fx.home.join(".config/gh/hosts.yml").is_file(),
            "Maske wurde verschoben"
        );
        assert!(!fx.home.join(".config2").exists());
        // Eine nicht vorhandene Default-Maske in einer Schreib-Wurzel lässt sich nicht anlegen
        // (sonst schriebe die Sandbox z. B. eine gh-Konfiguration mit Aliasen für den Host).
        std::fs::create_dir_all(fx.home.join(".docker")).unwrap();
        let docker = fx.policy(
            p.as_ref(),
            "read_paths: [\"~\"]\nwrite_paths: [\"~/.docker\"]\nallow_hidden: [.docker]\n",
            Stage::Tools,
        );
        let out = sh(
            p.as_ref(),
            &docker,
            "echo '{}' > \"$HOME/.docker/config.json\"",
        )
        .await;
        assert!(!out.ok, "{}: {out:?}", p.id());
        assert!(!fx.home.join(".docker/config.json").exists());

        write(&fx.ws.join("a/b/geheim.txt"), "FIXTURE-OWN");
        let own = format!("masks: [\"{}\"]\n", fx.ws.join("a/b/geheim.txt").display());
        let policy = fx.policy(p.as_ref(), &own, Stage::Tools);
        for attack in [
            "cat a/b/geheim.txt",
            "mv a z; cat z/b/geheim.txt",
            "mv a/b a/c; cat a/c/geheim.txt",
            "ln a/b/geheim.txt x; cat x",
            "mv a/b/geheim.txt a/b/offen.txt; cat a/b/offen.txt",
        ] {
            let out = sh(p.as_ref(), &policy, attack).await;
            assert!(
                !out.stdout.contains("FIXTURE"),
                "{}: {attack}: {out:?}",
                p.id()
            );
        }
        // Auch über zwei Starts hinweg: Die maskierte Datei bleibt, wo sie war (sonst fände der
        // nächste Start sie unter neuem Pfad unmaskiert).
        assert!(
            fx.ws.join("a/b/geheim.txt").is_file(),
            "{}: Maske verschoben",
            p.id()
        );
        let out = sh(p.as_ref(), &policy, "cat z/b/geheim.txt a/c/geheim.txt x").await;
        assert!(!out.stdout.contains("FIXTURE"), "{}: {out:?}", p.id());
        let out = sh(
            p.as_ref(),
            &policy,
            "echo ok > neu.txt && cat neu.txt && rm neu.txt",
        )
        .await;
        assert!(out.ok && out.stdout.contains("ok"), "{}: {out:?}", p.id());
    }
}

#[tokio::test]
async fn sbx_008_ac1_system_secrets_unreadable_writes_only_in_workspace() {
    let fx = fx();
    // Datei außerhalb aller Freigaben, die ohne Sandbox lesbar ist (Kontrollgruppe).
    write(&fx.root.join("draussen/geheim.txt"), "FIXTURE-DRAUSSEN");
    let outside = fx.root.join("draussen/geheim.txt");
    let c = control(&fx, &format!("cat '{}'", outside.display())).await;
    assert!(c.ok && c.stdout.contains("FIXTURE-DRAUSSEN"), "{c:?}");
    for p in backends() {
        for stage in [Stage::Harness, Stage::Tools] {
            let policy = fx.policy(p.as_ref(), "{}\n", stage);
            for attack in [
                "cat /etc/shadow".to_owned(),
                "echo x > /etc/beton-test".to_owned(),
                "cat \"$HOME/.ssh/id_ed25519\"".to_owned(),
                format!("cat '{}'", outside.display()),
                "echo x > \"$HOME/Documents/neu.txt\"".to_owned(),
                format!("echo x > '{}'", fx.root.join("draussen/neu.txt").display()),
            ] {
                let out = sh(p.as_ref(), &policy, &attack).await;
                assert!(!out.ok, "{} {stage:?}: {attack}: {out:?}", p.id());
                assert!(!out.stdout.contains("FIXTURE"), "{attack}: {out:?}");
            }
            assert!(!Path::new("/etc/beton-test").exists());
            assert!(!fx.root.join("draussen/neu.txt").exists());
            // Workspace und Scratch sind beschreibbar, `TMPDIR` zeigt auf den Scratch; `/tmp`
            // ist privat und erreicht das Host-`/tmp` nicht.
            let marker = format!("beton-sbx-{}", std::process::id());
            let out = sh(
                p.as_ref(),
                &policy,
                &format!(
                    "echo x > ws.txt && echo y > \"$TMPDIR/t\" && cat \"$TMPDIR/t\" && echo z > /tmp/{marker}"
                ),
            )
            .await;
            assert!(out.ok && out.stdout.contains('y'), "{}: {out:?}", p.id());
            assert!(fx.ws.join("ws.txt").is_file() && fx.scratch.join("t").is_file());
            assert!(
                !Path::new("/tmp").join(&marker).exists(),
                "/tmp ist nicht privat"
            );
            std::fs::remove_file(fx.ws.join("ws.txt")).unwrap();
        }
    }
}

#[tokio::test]
async fn sbx_012_ac2_readonly_preset_blocks_every_write_in_the_workspace() {
    let fx = fx();
    let c = control(&fx, "test -w src/main.rs && test -w .").await;
    assert!(c.ok, "Kontrollgruppe: {c:?}");
    for p in backends() {
        let policy = fx.policy(p.as_ref(), "preset: readonly\n", Stage::Tools);
        for attack in [
            "echo x > neu.txt",
            "echo x >> src/main.rs",
            "mkdir neu",
            "rm src/main.rs",
            "mv src/main.rs src/umbenannt.rs",
            "touch src/main.rs",
            "ln -s /etc/hosts link",
        ] {
            let out = sh(p.as_ref(), &policy, attack).await;
            assert!(!out.ok, "{} {attack}: {out:?}", p.id());
        }
        assert_eq!(
            std::fs::read_to_string(fx.ws.join("src/main.rs")).unwrap(),
            "fn main() {}\n"
        );
        assert!(!fx.ws.join("neu.txt").exists() && !fx.ws.join("neu").exists());
        let out = sh(
            p.as_ref(),
            &policy,
            "cat src/main.rs && echo y > \"$TMPDIR/t\"",
        )
        .await;
        assert!(
            out.ok && out.stdout.contains("fn main"),
            "{}: {out:?}",
            p.id()
        );
    }
}

#[tokio::test]
async fn sbx_008_ac1_shell_rc_and_autostart_are_not_writable_in_both_stages() {
    let fx = fx();
    let attack = "echo pwned >> \"$HOME/.bashrc\"; echo pwned > \"$HOME/.config/systemd/user/x.service\"; echo pwned > \"$HOME/.profile\"";
    let c = control(&fx, "echo ok >> \"$HOME/.bashrc\"").await;
    assert!(c.ok, "{c:?}");
    write(&fx.home.join(".bashrc"), "# fixture\n");
    for p in backends() {
        let mut stage_one = fx.ctx(Stage::Harness);
        stage_one.harness = beton_sandbox::masks::harness_grants("claude");
        for policy in [
            fx.policy(p.as_ref(), "read_paths: [\"~\"]\n", Stage::Tools),
            fx.policy_with(p.as_ref(), "read_paths: [\"~\"]\n", stage_one),
        ] {
            let out = sh(p.as_ref(), &policy, attack).await;
            assert!(!out.ok, "{} {:?}: {out:?}", p.id(), policy.stage);
            assert_eq!(
                std::fs::read_to_string(fx.home.join(".bashrc")).unwrap(),
                "# fixture\n"
            );
            assert!(!fx.home.join(".config/systemd/user/x.service").exists());
            assert!(!fx.home.join(".profile").exists());
        }
    }
}

#[tokio::test]
async fn sbx_008_environment_is_not_inherited_and_launcher_argv_has_no_secrets() {
    let fx = fx();
    for p in backends() {
        let mut ctx = fx.ctx(Stage::Tools);
        ctx.host_env
            .insert("AWS_SECRET_ACCESS_KEY".into(), "FIXTURE-AWS-ENV".into());
        ctx.host_env.insert("PATH".into(), "/usr/bin:/bin".into());
        let spec = SandboxSpec::from_yaml("env:\n  set: { GEHEIM_X: FIXTURE-SET }\n").unwrap();
        let policy = p.resolve(&spec, &ctx).unwrap();
        let cmd = CommandSpec::new("/bin/sh")
            .args([
                "-c",
                "env; for f in /proc/[0-9]*/cmdline; do tr '\\0' ' ' < $f; echo; done",
            ])
            .env("LD_PRELOAD", "/tmp/x.so")
            .env("SSH_AUTH_SOCK", "/tmp/agent")
            .env("BETON_TOOL_TOKEN", "FIXTURE-CMD-ENV")
            .cwd(&policy.workspace);
        let out = run_with(p.as_ref(), &policy, cmd).await;
        assert!(out.ok, "{}: {out:?}", p.id());
        assert!(!out.stdout.contains("FIXTURE-AWS-ENV"), "{}", out.stdout);
        assert!(
            !out.stdout.contains("LD_PRELOAD") && !out.stdout.contains("SSH_AUTH_SOCK"),
            "{}",
            out.stdout
        );
        assert!(out.stdout.contains("BETON_STAGE=tools"));
        assert!(out.stdout.contains("GEHEIM_X=FIXTURE-SET"));
        assert!(out.stdout.contains("BETON_TOOL_TOKEN=FIXTURE-CMD-ENV"));
        // Keine Werte in den Kommandozeilen der Sandbox-Prozesse (SBX-005 AC4, SBX-009 AC3).
        let cmdlines: Vec<&str> = out.stdout.lines().filter(|l| !l.contains('=')).collect();
        for l in &cmdlines {
            assert!(!l.contains("FIXTURE"), "{}: argv mit Wert: {l}", p.id());
        }
    }
    // Auch der Launcher außerhalb der Sandbox (Host-Sicht) trägt keine Werte in argv.
    let p = backends().remove(0);
    let mut ctx = fx.ctx(Stage::Tools);
    ctx.host_env
        .insert("GH_TOKEN".into(), "FIXTURE-HOST-ARGV".into());
    let spec = SandboxSpec::from_yaml("env:\n  passthrough: [GH_TOKEN]\n").unwrap();
    let policy = p.resolve(&spec, &ctx).unwrap();
    let marker = format!("1000.{}", std::process::id());
    let child = p
        .spawn(
            &policy,
            CommandSpec::new("/bin/sleep")
                .arg(&marker)
                .cwd(&policy.workspace)
                .stdio(
                    beton_sandbox::Stdio::Null,
                    beton_sandbox::Stdio::Null,
                    beton_sandbox::Stdio::Null,
                ),
        )
        .unwrap();
    let launcher_pid = child.id().unwrap();
    let argv = std::fs::read(format!("/proc/{launcher_pid}/cmdline")).unwrap();
    let argv = String::from_utf8_lossy(&argv);
    assert!(argv.contains("__sandbox-exec"), "{argv}");
    assert!(!argv.contains("FIXTURE"), "{argv}");
    // Der Launcher ist nicht dumpable; sein Environment ist dann nicht einmal lesbar.
    match std::fs::read(format!("/proc/{launcher_pid}/environ")) {
        Ok(environ) => assert!(!String::from_utf8_lossy(&environ).contains("FIXTURE")),
        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}"),
    }
    drop(child);
}

#[tokio::test]
async fn sbx_008_foreign_processes_are_invisible() {
    let fx = fx();
    let me = std::process::id();
    let c = control(
        &fx,
        &format!("cat /proc/{me}/environ | tr '\\0' '\\n' | head -1"),
    )
    .await;
    assert!(c.ok, "Kontrollgruppe: {c:?}");
    for p in backends() {
        let policy = fx.policy(p.as_ref(), "{}\n", Stage::Tools);
        // Eigene PID-Sicht: Runner und andere Prozesse des Benutzers fehlen.
        let out = sh(
            p.as_ref(),
            &policy,
            &format!("cat /proc/{me}/environ; cat /proc/{me}/cmdline"),
        )
        .await;
        assert!(!out.ok && out.stdout.is_empty(), "{}: {out:?}", p.id());
        let out = sh(p.as_ref(), &policy, "ls /proc | grep -c '^[0-9]'").await;
        let n: usize = out.stdout.trim().parse().unwrap_or(usize::MAX);
        assert!(n < 16, "{}: {n} Prozesse sichtbar: {out:?}", p.id());
    }
}

#[tokio::test]
async fn sbx_008_ac3_namespaces_mount_ptrace_and_kill_fail() {
    let fx = fx();
    let me = std::process::id().to_string();
    // Kontrollgruppe ohne Sandbox.
    // `unshare -r` gelingt ohne Sandbox nur, wo unprivilegierte User-Namespaces erlaubt sind
    // (nicht im CI-Lauf für SBX-009 AC1 mit `unprivileged_userns_clone=0`).
    let userns = sysctl("kernel.unprivileged_userns_clone").as_deref() != Some("0");
    let script = if userns {
        format!("kill -0 {me} && unshare -r true && echo ok")
    } else {
        format!("kill -0 {me} && echo ok")
    };
    let c = control(&fx, &script).await;
    assert!(c.ok, "Kontrollgruppe: {c:?}");
    for p in backends() {
        let policy = fx.policy(
            p.as_ref(),
            &format!("read_paths: [{}]\n", exe_grant()),
            Stage::Tools,
        );
        for attack in [
            "unshare -r true".to_owned(),
            "unshare -n true".to_owned(),
            "mkdir -p m && mount -t tmpfs none m".to_owned(),
            format!("kill -9 {me}"),
            format!("kill -0 {me}"),
        ] {
            let out = sh(p.as_ref(), &policy, &attack).await;
            assert!(!out.ok, "{}: {attack}: {out:?}", p.id());
        }
        let ws = policy.workspace.join("m2");
        std::fs::create_dir_all(&ws).unwrap();
        let cases: Vec<(&str, Vec<String>, &str)> = vec![
            ("ptrace", vec![me.clone()], "err:"),
            ("kill", vec![me.clone()], "err:"),
            ("unshare_user", vec![], "err:1"),
            ("unshare_net", vec![], "err:1"),
            ("clone_newuser", vec![], "err:1"),
            ("clone3", vec![], "err:38"),
            ("x32_unshare", vec![], "err:"),
            ("mount_tmpfs", vec![ws.display().to_string()], "err:1"),
            ("socket", vec![libc::AF_NETLINK.to_string()], "err:1"),
            ("socket", vec![libc::AF_PACKET.to_string()], "err:1"),
            ("io_uring", vec![], "err:1"),
            ("bpf", vec![], "err:1"),
            ("keyctl", vec![], "err:1"),
            ("tiocsti", vec![], "err:1"),
        ];
        for (probe, args, expected) in cases {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = run_with(p.as_ref(), &policy, probe_cmd(&policy, probe, &args)).await;
            let r = probe_result(&out);
            assert!(
                r.starts_with(expected),
                "{}: {probe}: {r} (erwartet {expected})",
                p.id()
            );
        }
        // Die Prozesse der Sandbox-Verwaltung (PID 1, unter bwrap auch der innere Launcher)
        // sind nicht lesbar, auch wo Yama ptrace nicht einschränkt; der eigene Kindprozess schon.
        let out = run_with(
            p.as_ref(),
            &policy,
            probe_cmd(&policy, "procmem_child", &[]),
        )
        .await;
        assert_eq!(
            probe_result(&out),
            "ok",
            "{}: Kontrollgruppe: {out:?}",
            p.id()
        );
        let out = run_with(
            p.as_ref(),
            &policy,
            probe_cmd(&policy, "procmem_ancestors", &[]),
        )
        .await;
        let r = probe_result(&out);
        assert!(r.starts_with("err:"), "{}: Vorfahren lesbar: {r}", p.id());
        // Keine Capabilities, auch wenn die Sandbox im eigenen User-Namespace als UID 0 läuft.
        let out = run_with(p.as_ref(), &policy, probe_cmd(&policy, "caps", &[])).await;
        let caps = probe_result(&out);
        for field in ["CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"] {
            assert!(
                caps.contains(&format!("{field}:\t0000000000000000")),
                "{}: {caps}",
                p.id()
            );
        }
    }
}

#[tokio::test]
async fn sbx_008_ac4_abi2_truncate_is_blocked_by_seccomp() {
    let fx = fx();
    let ro = fx.root.join("ro");
    write(&ro.join("datei.txt"), "FIXTURE-INHALT");
    let copy = fx.root.join("kopie.txt");
    write(&copy, "FIXTURE-INHALT");
    // Kontrollgruppe: ohne Sandbox kürzt `truncate(2)` eine beschreibbare Datei.
    let none = ResolvedPolicy::unsandboxed(Stage::Tools);
    let mut cmd = probe_cmd(&none, "truncate", &[copy.to_str().unwrap()]);
    cmd.cwd = Some(fx.ws.clone());
    let c = run_with(&beton_sandbox::NoneProvider, &none, cmd).await;
    assert_eq!(probe_result(&c), "ok", "Kontrollgruppe: {c:?}");
    // Landlock-ABI 2 (Kernel 5.19 – 6.1, z. B. Debian 12) emuliert: Die Truncate-Kontrolle von
    // Landlock fehlt, seccomp sperrt `truncate(2)` und `open(O_RDONLY|O_TRUNC)`. Jedes Backend
    // läuft einmal mit ABI 2 und einmal mit der ABI des Kernels.
    let mut providers = Vec::new();
    for p in backends() {
        let mut cfg = config();
        cfg.abi_cap = Some(2);
        let abi2 = match p.id() {
            ProviderId::Bwrap => bwrap_provider(cfg),
            _ => landlock_provider(cfg),
        };
        let probe = abi2.probe();
        assert!(probe.available, "{probe:?}");
        assert!(probe.version.contains("ABI 2"), "{}", probe.version);
        providers.push(abi2);
        providers.push(p);
    }
    for p in providers {
        let yaml = format!("read_paths: [{}, \"{}\"]\n", exe_grant(), ro.display());
        let policy = fx.policy(p.as_ref(), &yaml, Stage::Tools);
        let file = ro.join("datei.txt");
        let file = file.to_str().unwrap();
        let out = sh(p.as_ref(), &policy, &format!("truncate -s0 '{file}'")).await;
        assert!(!out.ok, "{}: {out:?}", p.id());
        for probe in ["truncate", "open_rdonly_trunc"] {
            let out = run_with(p.as_ref(), &policy, probe_cmd(&policy, probe, &[file])).await;
            assert!(
                probe_result(&out).starts_with("err:"),
                "{}: {probe}: {out:?}",
                p.id()
            );
        }
        assert_eq!(
            std::fs::read_to_string(ro.join("datei.txt")).unwrap(),
            "FIXTURE-INHALT"
        );
        // seccomp wirkt unabhängig von Mounts und Landlock: Mit ABI 2 ist `truncate(2)` auch im
        // Workspace gesperrt, mit ABI ≥ 3 regelt Landlock das (im Workspace erlaubt).
        let abi2 = p.probe().version.contains("ABI 2");
        write(&fx.ws.join("t.txt"), "abc");
        let ws_file = fx.ws.join("t.txt");
        let ws_file = ws_file.to_str().unwrap();
        for probe in ["truncate", "open_rdonly_trunc"] {
            let out = run_with(p.as_ref(), &policy, probe_cmd(&policy, probe, &[ws_file])).await;
            let r = probe_result(&out);
            if abi2 {
                assert_eq!(r, "err:1", "{}: {probe} im Workspace: {out:?}", p.id());
            } else {
                assert_eq!(r, "ok", "{}: {probe} im Workspace: {out:?}", p.id());
            }
        }
        // Im Workspace bleibt Kürzen per Shell-Umleitung möglich.
        let out = sh(
            p.as_ref(),
            &policy,
            "echo abc > w.txt && : > w.txt && wc -c < w.txt",
        )
        .await;
        assert!(out.ok && out.stdout.trim() == "0", "{}: {out:?}", p.id());
    }
}

/// Lokaler TLS-Upstream, der auf jeden Request mit `upstream` antwortet.
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
async fn sbx_008_ac2_only_loopback_and_the_relay_to_the_proxy() {
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
    std::fs::write(fx.scratch.join("beton-ca.pem"), proxy.ca_pem()).unwrap();
    std::fs::copy(&upstream_ca, fx.scratch.join("upstream-ca.pem")).unwrap();
    // Host-Dienste auf Loopback (TCP und UDP), die die Sandbox nicht erreichen darf.
    let host_tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tcp_addr = host_tcp.local_addr().unwrap();
    let accept = tokio::spawn(async move { while host_tcp.accept().await.is_ok() {} });
    let host_udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    host_udp
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let udp_addr = host_udp.local_addr().unwrap();

    let url = format!("https://127.0.0.1:{port}/ok");
    let c = control(
        &fx,
        &format!(
            "curl -sS --max-time 10 --cacert {} {url}",
            upstream_ca.display()
        ),
    )
    .await;
    assert!(
        c.ok && c.stdout.contains("upstream"),
        "Kontrollgruppe: {c:?}"
    );

    for p in backends() {
        let mut ctx = fx.ctx(Stage::Tools);
        ctx.proxy = Some(ProxyEndpoint::port(instance.addr().port()));
        let yaml = format!(
            "read_paths: [{}]\negress_rules: [\"GET 127.0.0.1/**\"]\n",
            exe_grant()
        );
        let policy = fx.policy_with(p.as_ref(), &yaml, ctx);
        let proxy_url = instance.proxy_url().to_string();
        let run = |script: String| {
            let policy = policy.clone();
            let p = p.clone();
            let proxy_url = proxy_url.clone();
            async move {
                let cmd = CommandSpec::new("/bin/sh")
                    .args(["-c", &script])
                    .cwd(&policy.workspace)
                    .env("HTTPS_PROXY", proxy_url);
                run_with(p.as_ref(), &policy, cmd).await
            }
        };
        // Der Net-NS enthält nur `lo`.
        let out = run("tail -n +3 /proc/net/dev | cut -d: -f1 | tr -d ' '".into()).await;
        assert!(out.ok && out.stdout.trim() == "lo", "{}: {out:?}", p.id());
        // Über den Relay zum Proxy und mit passender Regel zum Upstream.
        let out = run(format!(
            "curl -sS --max-time 10 --cacert \"$TMPDIR/beton-ca.pem\" {url}"
        ))
        .await;
        assert!(
            out.ok && out.stdout.contains("upstream"),
            "{}: {out:?}",
            p.id()
        );
        // Ohne passende Regel lehnt der Proxy ab.
        let out = run(format!(
            "curl -sS -f --max-time 10 --cacert \"$TMPDIR/beton-ca.pem\" -X POST {url}"
        ))
        .await;
        assert!(!out.ok, "{}: {out:?}", p.id());
        // Am Proxy vorbei: kein Weg zum Upstream, zu Host-Diensten oder ins Netz.
        let out = run(format!(
            "curl -sS --noproxy '*' --max-time 5 --cacert \"$TMPDIR/upstream-ca.pem\" {url}"
        ))
        .await;
        assert!(
            !out.ok && !out.stdout.contains("upstream"),
            "{}: {out:?}",
            p.id()
        );
        for target in [
            tcp_addr.to_string(),
            "192.0.2.1:443".into(),
            "1.1.1.1:53".into(),
        ] {
            let out = run_with(
                p.as_ref(),
                &policy,
                probe_cmd(&policy, "connect", &[&target]),
            )
            .await;
            assert!(
                probe_result(&out).starts_with("err:"),
                "{}: {target}: {out:?}",
                p.id()
            );
        }
        let out = run_with(
            p.as_ref(),
            &policy,
            probe_cmd(&policy, "udp", &[&udp_addr.to_string()]),
        )
        .await;
        let _ = probe_result(&out);
        let mut buf = [0u8; 64];
        assert!(
            host_udp.recv_from(&mut buf).is_err(),
            "{}: UDP-Paket aus der Sandbox kam am Host an",
            p.id()
        );
        // Ohne Proxy gibt es gar kein Netz, auch nicht zum Proxy-Port.
        let none = fx.policy(
            p.as_ref(),
            &format!("read_paths: [{}]\n", exe_grant()),
            Stage::Tools,
        );
        let target = instance.addr().to_string();
        let out = run_with(p.as_ref(), &none, probe_cmd(&none, "connect", &[&target])).await;
        assert!(
            probe_result(&out).starts_with("err:"),
            "{}: {out:?}",
            p.id()
        );
    }
    // Kontrollgruppe UDP: ohne Sandbox kommt das Paket an.
    let none = ResolvedPolicy::unsandboxed(Stage::Tools);
    let mut cmd = probe_cmd(&none, "udp", &[&udp_addr.to_string()]);
    cmd.cwd = Some(fx.ws.clone());
    let _ = run_with(&beton_sandbox::NoneProvider, &none, cmd).await;
    let mut buf = [0u8; 64];
    assert!(host_udp.recv_from(&mut buf).is_ok(), "Kontrollgruppe UDP");
    accept.abort();
}

/// Git-Werkzeug für die Tests.
fn git_host(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
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

fn init_commit(dir: &Path) {
    git_host(dir, &["init", "-q"]);
    git_host(
        dir,
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
}

#[tokio::test]
async fn sbx_003_ac4_git_hooks_are_read_only_but_commits_work() {
    for p in backends() {
        let fx = fx();
        init_commit(&fx.ws);
        let mut ctx = fx.ctx(Stage::Tools);
        ctx.git = GitLayout::discover(&fx.ws).unwrap();
        let policy = fx.policy_with(p.as_ref(), "{}\n", ctx);
        let out = sh(
            p.as_ref(),
            &policy,
            &format!("{GIT_ENV} echo a > a.txt && git add a.txt && git commit -q -m c1 && git log --oneline"),
        )
        .await;
        assert!(out.ok && out.stdout.contains("c1"), "{}: {out:?}", p.id());
        for attack in [
            "echo 'echo pwned' > .git/hooks/pre-commit",
            "mkdir -p .git/hooks/x",
            "rm -rf .git/hooks",
            "mv .git/hooks .git/h2",
            &format!("{GIT_ENV} git config core.hooksPath /tmp"),
            "echo '[core]' >> .git/config",
            "mv .git .git-alt",
        ] {
            let out = sh(p.as_ref(), &policy, attack).await;
            assert!(!out.ok, "{}: {attack}: {out:?}", p.id());
        }
        assert!(!fx.ws.join(".git/hooks/pre-commit").exists());
        assert!(fx.ws.join(".git/HEAD").is_file());
    }
}

#[tokio::test]
async fn sbx_003_ac4_commits_in_a_worktree_reach_the_object_db_but_not_hooks() {
    for p in backends() {
        let fx = fx();
        let repo = fx.root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_commit(&repo);
        let wt = fx.ws.join("wt");
        git_host(
            &repo,
            &["worktree", "add", "-q", "-b", "agent", wt.to_str().unwrap()],
        );
        let wt = wt.canonicalize().unwrap();
        let mut ctx = ResolveCtx::new(Stage::Tools, &wt, &fx.home, &fx.scratch);
        ctx.git = GitLayout::discover(&wt).unwrap();
        let policy = p.resolve(&SandboxSpec::default(), &ctx).unwrap();
        let out = sh(p.as_ref(), &policy, &format!("{GIT_ENV} echo a > a.txt && git add a.txt && git commit -q -m im-worktree && git log --oneline -1")).await;
        assert!(
            out.ok && out.stdout.contains("im-worktree"),
            "{}: {out:?}",
            p.id()
        );
        for attack in [
            format!("echo x > {}/.git/hooks/post-checkout", repo.display()),
            format!("echo x >> {}/.git/config", repo.display()),
            "echo 'gitdir: /tmp' > .git".to_owned(),
            format!("echo x > {}/main.txt", repo.display()),
        ] {
            let out = sh(p.as_ref(), &policy, &attack).await;
            assert!(!out.ok, "{}: {attack}: {out:?}", p.id());
        }
        assert!(!repo.join("main.txt").exists());
        assert!(
            std::fs::read_to_string(wt.join(".git"))
                .unwrap()
                .starts_with("gitdir: ")
        );
    }
}

/// Repo im Workspace mit Commit und einem zweiten Worktree (`<root>/wt2`); liefert dessen
/// Eintrag `.git/worktrees/wt2`.
fn repo_with_worktree(fx: &Fx) -> PathBuf {
    init_commit(&fx.ws);
    let wt2 = fx.root.join("wt2");
    git_host(
        &fx.ws,
        &["worktree", "add", "-q", "-b", "wt2", wt2.to_str().unwrap()],
    );
    fx.ws.join(".git/worktrees/wt2")
}

/// `commondir`/`gitdir` lenken Host-git auf ein anderes Git-Verzeichnis (#209).
const GIT_DIR_ATTACKS: [&str; 5] = [
    "echo /tmp/x > .git/commondir",
    "echo /tmp/x > .git/gitdir",
    "echo /tmp/x > .git/worktrees/wt2/commondir",
    "echo /tmp/x > .git/worktrees/wt2/gitdir",
    // Den Eintrag als Ganzes wegbewegen (dann ließe sich ein vorbereiteter einsetzen).
    "mv .git/worktrees/wt2 .git/wt2-alt",
];

#[tokio::test]
async fn sbx_003_ac4_commondir_and_gitdir_cannot_be_created_or_changed() {
    // Kontrollgruppe: ohne Sandbox gelingt jeder Angriff.
    let fc = fx();
    repo_with_worktree(&fc);
    for attack in GIT_DIR_ATTACKS {
        let c = control(&fc, attack).await;
        assert!(c.ok, "Kontrollgruppe {attack}: {c:?}");
    }
    assert!(fc.ws.join(".git/commondir").is_file());

    for p in backends() {
        let fx = fx();
        let admin = repo_with_worktree(&fx);
        let before = [
            std::fs::read(admin.join("commondir")).unwrap(),
            std::fs::read(admin.join("gitdir")).unwrap(),
        ];
        let mut ctx = fx.ctx(Stage::Tools);
        ctx.git = GitLayout::discover(&fx.ws).unwrap();
        let policy = fx.policy_with(p.as_ref(), "{}\n", ctx);
        for attack in GIT_DIR_ATTACKS {
            let out = sh(p.as_ref(), &policy, attack).await;
            assert!(!out.ok, "{}: {attack}: {out:?}", p.id());
            assert!(
                !fx.ws.join(".git/commondir").exists(),
                "{}: {attack}",
                p.id()
            );
            assert!(!fx.ws.join(".git/gitdir").exists(), "{}: {attack}", p.id());
        }
        assert_eq!(std::fs::read(admin.join("commondir")).unwrap(), before[0]);
        assert_eq!(std::fs::read(admin.join("gitdir")).unwrap(), before[1]);
        // git arbeitet weiter.
        let out = sh(
            p.as_ref(),
            &policy,
            &format!(
                "{GIT_ENV} echo a > a.txt && git add a.txt && git commit -q -m c1 && \
                 git checkout -q -b neu && git log --oneline -1"
            ),
        )
        .await;
        assert!(out.ok && out.stdout.contains("c1"), "{}: {out:?}", p.id());
    }
}

#[tokio::test]
async fn sbx_003_ac4_commondir_and_gitdir_of_the_own_worktree_are_read_only() {
    let attacks = |admin: &Path| {
        [
            format!("echo /tmp/x > {}/commondir", admin.display()),
            format!("echo /tmp/x > {}/gitdir", admin.display()),
        ]
    };
    // Kontrollgruppe: ohne Sandbox gelingt es.
    let fc = fx();
    let admin = repo_with_worktree(&fc);
    for attack in attacks(&admin) {
        let c = control(&fc, &attack).await;
        assert!(c.ok, "Kontrollgruppe {attack}: {c:?}");
    }

    for p in backends() {
        let fx = fx();
        let admin = repo_with_worktree(&fx).canonicalize().unwrap();
        let wt = fx.root.join("wt2");
        let before = [
            std::fs::read(admin.join("commondir")).unwrap(),
            std::fs::read(admin.join("gitdir")).unwrap(),
        ];
        let mut ctx = ResolveCtx::new(Stage::Tools, &wt, &fx.home, &fx.scratch);
        ctx.git = GitLayout::discover(&wt).unwrap();
        let policy = p.resolve(&SandboxSpec::default(), &ctx).unwrap();
        for attack in attacks(&admin) {
            let out = sh(p.as_ref(), &policy, &attack).await;
            assert!(!out.ok, "{}: {attack}: {out:?}", p.id());
        }
        assert_eq!(std::fs::read(admin.join("commondir")).unwrap(), before[0]);
        assert_eq!(std::fs::read(admin.join("gitdir")).unwrap(), before[1]);
        let out = sh(
            p.as_ref(),
            &policy,
            &format!(
                "{GIT_ENV} echo a > a.txt && git add a.txt && git commit -q -m im-worktree && \
                 git checkout -q -b neu && git log --oneline -1"
            ),
        )
        .await;
        assert!(
            out.ok && out.stdout.contains("im-worktree"),
            "{}: {out:?}",
            p.id()
        );
    }
}

#[tokio::test]
async fn ses_015_ac2_worktree_under_beton_home_writes_only_itself() {
    for p in backends() {
        let fx = fx();
        let repo = fx.root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_commit(&repo);
        write(&fx.home.join(".beton/run/token"), "FIXTURE-BETON-TOKEN");
        let wt = fx.home.join(".beton/worktrees/repo-1234abcd/beton-x");
        std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
        git_host(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "beton/x",
                wt.to_str().unwrap(),
            ],
        );
        let wt = wt.canonicalize().unwrap();
        let mut ctx = ResolveCtx::new(Stage::Tools, &wt, &fx.home, &fx.scratch);
        ctx.adapters = beton_sandbox::masks::builtin_harness_grants();
        ctx.git = GitLayout::discover(&wt).unwrap();
        let policy = p.resolve(&SandboxSpec::default(), &ctx).unwrap();
        let out = sh(
            p.as_ref(),
            &policy,
            &format!("{GIT_ENV} echo a > a.txt && git add a.txt && git commit -q -m im-worktree"),
        )
        .await;
        assert!(out.ok, "{}: {out:?}", p.id());
        for attack in [
            format!("echo x > {}/main.txt", repo.display()),
            "cat \"$HOME/.beton/run/token\"".to_owned(),
            "ls \"$HOME/.beton/run\"".to_owned(),
            "echo x > \"$HOME/.beton/worktrees/repo-1234abcd/neu\"".to_owned(),
        ] {
            let out = sh(p.as_ref(), &policy, &attack).await;
            assert!(!out.ok, "{}: {attack}: {out:?}", p.id());
            assert!(!out.stdout.contains("FIXTURE"), "{attack}: {out:?}");
        }
        assert!(!repo.join("main.txt").exists());
    }
}

#[tokio::test]
async fn sbx_002_stage_one_reaches_its_exec_socket_stage_two_does_not() {
    let abi = beton_sandbox::linux::kernel_landlock_abi().unwrap_or(0);
    for p in backends() {
        let fx = fx();
        let dir = fx.root.join("exec");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let socket = dir.join("exec.sock");
        let other = dir.join("other.sock");
        for path in [&socket, &other] {
            let l = std::os::unix::net::UnixListener::bind(path).unwrap();
            std::thread::spawn(move || {
                for c in l.incoming() {
                    drop(c);
                }
            });
        }
        let mut ctx = fx.ctx(Stage::Harness);
        ctx.harness = beton_sandbox::masks::harness_grants("claude");
        ctx.exec = Some(beton_sandbox::ExecAccess {
            socket: socket.clone(),
            shim_dir: dir.join("bin"),
            token: beton_sandbox::Secret::new("FIXTURE-EXEC-TOKEN".into()),
        });
        let harness = fx.policy_with(p.as_ref(), &format!("read_paths: [{}]\n", exe_grant()), ctx);
        let s = socket.display().to_string();
        let out = run_with(p.as_ref(), &harness, probe_cmd(&harness, "unix", &[&s])).await;
        assert_eq!(
            probe_result(&out),
            "ok",
            "{}: Broker aus Stufe 1: {out:?}",
            p.id()
        );
        if abi >= 9 {
            // Ab Landlock-ABI 9 nur der freigegebene Socket (vorher Restrisiko #211).
            let o = other.display().to_string();
            let out = run_with(p.as_ref(), &harness, probe_cmd(&harness, "unix", &[&o])).await;
            assert!(
                probe_result(&out).starts_with("err:"),
                "{}: {out:?}",
                p.id()
            );
        }
        let tools = fx.policy(
            p.as_ref(),
            &format!("read_paths: [{}]\n", exe_grant()),
            Stage::Tools,
        );
        let out = run_with(p.as_ref(), &tools, probe_cmd(&tools, "unix", &[&s])).await;
        assert!(
            probe_result(&out).starts_with("err:"),
            "{}: Broker aus Stufe 2: {out:?}",
            p.id()
        );
    }
}

#[tokio::test]
async fn prx_008_ac3_dev_server_in_the_same_sandbox_but_no_host_service() {
    let abi = beton_sandbox::linux::kernel_landlock_abi().unwrap_or(0);
    let host = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let host_addr = host.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for c in host.incoming() {
            drop(c);
        }
    });
    for p in backends() {
        let fx = fx();
        let yaml = format!("read_paths: [{}]\ndev_ports: [3000]\n", exe_grant());
        let policy = fx.policy(p.as_ref(), &yaml, Stage::Tools);
        let out = run_with(
            p.as_ref(),
            &policy,
            probe_cmd(&policy, "devserver", &["3000"]),
        )
        .await;
        assert_eq!(probe_result(&out), "ok:dev-ok", "{}: {out:?}", p.id());
        // Loopback ist sandbox-lokal: Ein Host-Dienst auf 127.0.0.1 bleibt unerreichbar.
        let out = run_with(
            p.as_ref(),
            &policy,
            probe_cmd(&policy, "connect", &[&host_addr]),
        )
        .await;
        assert!(
            probe_result(&out).starts_with("err:"),
            "{}: {out:?}",
            p.id()
        );
        // Ohne dev_ports sperrt Landlock (ab ABI 4) schon bind.
        let none = fx.policy(
            p.as_ref(),
            &format!("read_paths: [{}]\n", exe_grant()),
            Stage::Tools,
        );
        let out = run_with(p.as_ref(), &none, probe_cmd(&none, "devserver", &["3000"])).await;
        if abi >= 4 {
            assert!(
                probe_result(&out).starts_with("err:bind"),
                "{}: {out:?}",
                p.id()
            );
        }
    }
}

#[test]
fn sbx_001_ac3_registry_with_launcher_picks_landlock_or_bwrap() {
    let registry = Registry::with_launcher(&launcher());
    let landlock = registry.get(ProviderId::Landlock).unwrap().probe();
    let bwrap = registry.get(ProviderId::Bwrap).unwrap().probe();
    let picked = registry.select(BackendChoice::Auto).unwrap().id();
    let expected = match std::env::var("BETON_TEST_EXPECT_AUTO").as_deref() {
        Ok("bwrap") => {
            assert!(!landlock.available, "{landlock:?}");
            ProviderId::Bwrap
        }
        Ok(other) => panic!("unbekannte Erwartung {other}"),
        Err(_) => {
            assert!(landlock.available, "{landlock:?}");
            ProviderId::Landlock
        }
    };
    assert_eq!(picked, expected, "landlock: {landlock:?}, bwrap: {bwrap:?}");
    for r in [&landlock, &bwrap] {
        if r.available {
            assert!(r.caps.contains(&beton_sandbox::Cap::Syscall), "{r:?}");
            assert!(r.version.contains("ABI"), "{r:?}");
        }
    }
}

/// Liest einen sysctl-Wert (`None`, wenn es ihn nicht gibt).
fn sysctl(name: &str) -> Option<String> {
    std::fs::read_to_string(format!("/proc/sys/{}", name.replace('.', "/")))
        .ok()
        .map(|s| s.trim().to_owned())
}

#[tokio::test]
async fn sbx_008_ac5_apparmor_userns_restriction_reports_reason_and_fallback() {
    let restricted = sysctl("kernel.apparmor_restrict_unprivileged_userns").as_deref() == Some("1");
    if std::env::var("BETON_TEST_EXPECT_USERNS_RESTRICTED").as_deref() == Ok("1") {
        assert!(
            restricted,
            "die CI setzt die AppArmor-Restriktion für diesen Lauf"
        );
    }
    let fx = fx();
    // Eine Kopie des Launchers: Für sie gilt kein AppArmor-Profil (Profile hängen am Pfad).
    let copy = fx.root.join("beton-ohne-profil");
    std::fs::copy(launcher(), &copy).unwrap();
    let unprofiled = landlock_provider(LinuxConfig::new(copy.clone())).probe();
    if !restricted {
        // Ohne Restriktion funktioniert das Backend mit und ohne Profil.
        assert!(unprofiled.available, "{unprofiled:?}");
        return;
    }
    let profiled = PathBuf::from(
        std::env::var("BETON_TEST_APPARMOR_LAUNCHER")
            .expect("mit Restriktion braucht der Test den Launcher mit Profil"),
    );
    let p = landlock_provider(LinuxConfig::new(profiled));
    assert!(p.probe().available, "mit Profil: {:?}", p.probe());
    let policy = fx.policy(p.as_ref(), "read_paths: [\"~\"]\n", Stage::Tools);
    let out = sh(p.as_ref(), &policy, "cat \"$HOME/.ssh/id_ed25519\"").await;
    assert!(!out.ok && !out.stdout.contains(FIXTURE_SSH), "{out:?}");
    let out = sh(p.as_ref(), &policy, "echo x > ok.txt && cat ok.txt").await;
    assert!(out.ok, "{out:?}");
    // Ohne Profil: nicht verfügbar mit Grund und Fallback.
    assert!(!unprofiled.available, "{unprofiled:?}");
    let reason = unprofiled.unavailable_reason.clone().unwrap();
    assert!(
        reason.contains("AppArmor") && reason.contains("apparmor_restrict_unprivileged_userns"),
        "{reason}"
    );
    assert!(reason.contains("bwrap"), "Fallback fehlt: {reason}");
    let registry = Registry::with_launcher(&copy);
    let picked = registry.select(BackendChoice::Auto).unwrap();
    let bwrap = registry.get(ProviderId::Bwrap).unwrap().probe();
    if bwrap.available {
        assert_eq!(picked.id(), ProviderId::Bwrap);
    } else {
        assert!(reason.contains("kein Fallback"), "{reason}");
    }
}

/// Ein „Runner“ in einem eigenen Prozess: startet eine Sandbox und wartet. Tut nur etwas, wenn
/// der Test unten dieses Binary mit `BETON_SBX_FAKE_RUNNER` startet.
#[test]
fn fake_runner() {
    let Ok(marker) = std::env::var("BETON_SBX_FAKE_RUNNER") else {
        return;
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let fx = fx();
        let backend = std::env::var("BETON_SBX_FAKE_BACKEND").unwrap();
        let p = match backend.as_str() {
            "bwrap" => bwrap_provider(config()),
            _ => landlock_provider(config()),
        };
        let policy = fx.policy(p.as_ref(), "{}\n", Stage::Tools);
        let script = format!("sleep {marker} & setsid sleep {marker} & sleep {marker}");
        let _child = p
            .spawn(
                &policy,
                CommandSpec::new("/bin/sh")
                    .args(["-c", &script])
                    .cwd(&policy.workspace)
                    .stdio(
                        beton_sandbox::Stdio::Null,
                        beton_sandbox::Stdio::Null,
                        beton_sandbox::Stdio::Null,
                    ),
            )
            .unwrap();
        println!("READY");
        tokio::time::sleep(Duration::from_secs(600)).await;
    });
}

#[tokio::test]
async fn sbx_008_ac6_runner_sigkill_leaves_no_sandbox_process_within_one_second() {
    for p in backends() {
        let marker = format!("1000.{}{}", std::process::id(), p.id().as_str().len());
        let exe = std::env::current_exe().unwrap();
        let mut runner = std::process::Command::new(&exe)
            .args(["--exact", "fake_runner", "--nocapture", "--test-threads=1"])
            .env("BETON_SBX_FAKE_RUNNER", &marker)
            .env("BETON_SBX_FAKE_BACKEND", p.id().as_str())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = runner.stdout.take().unwrap();
        let ready = tokio::task::spawn_blocking(move || {
            use std::io::BufRead as _;
            std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
                .any(|l| l.contains("READY"))
        });
        assert!(
            tokio::time::timeout(Duration::from_secs(30), ready)
                .await
                .unwrap()
                .unwrap(),
            "Runner meldet sich nicht"
        );
        let start = std::time::Instant::now();
        while processes_with(&marker).len() < 3 {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "Sandbox startet nicht"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        runner.kill().unwrap();
        let _ = runner.wait();
        let left = gone_within(&marker, Duration::from_secs(1)).await;
        assert!(left.is_ok(), "{}: nach 1 s noch da: {left:?}", p.id());
    }
}
