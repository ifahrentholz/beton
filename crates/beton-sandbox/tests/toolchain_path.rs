//! Toolchains außerhalb der Systempfade in Stufe 2 (SBX-012, #241): Aus dem `PATH` des Daemons
//! leitet die Auflösung lesende Grants für Installationsverzeichnisse ab (Homebrew, nvm,
//! rustup, `/opt/hostedtoolcache`), kanonisch und ohne Home-Dotfiles; Schreibrechte und
//! Lockerungen der Masken gibt es nicht.
//!
//! Alle Homes und Toolchains sind Fixtures in Temp-Verzeichnissen.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

mod support;

use std::path::{Path, PathBuf};

use beton_sandbox::{Mask, ProviderId, ResolveCtx, SandboxSpec, Stage, resolve};

struct Tc {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    ws: PathBuf,
    scratch: PathBuf,
    opt: PathBuf,
}

fn exe(p: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn tc() -> Tc {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let home = root.join("home");
    let ws = root.join("ws");
    let scratch = root.join("scratch");
    let opt = root.join("opt");
    for d in [&home, &ws, &scratch] {
        std::fs::create_dir_all(d).unwrap();
    }
    // Homebrew-artig: Präfix mit bin, Cellar und einer globalen npmrc mit Token.
    exe(
        &opt.join("brew/bin/brewtool"),
        "#!/bin/sh\ncat \"$(dirname \"$0\")/../Cellar/tool/data\"\n",
    );
    support::write(&opt.join("brew/Cellar/tool/data"), "BREW-DATA\n");
    support::write(
        &opt.join("brew/etc/npmrc"),
        "//registry.npmjs.org/:_authToken=FIXTURE-NPM-TOKEN\n",
    );
    // nvm: Präfix unter einem Dotfile-Verzeichnis, aber tiefer als das Dotfile selbst.
    exe(
        &home.join(".nvm/versions/node/v24.0.0/bin/nodeish"),
        "#!/bin/sh\ncat \"$(dirname \"$0\")/../lib/node_modules/marker\"\n",
    );
    support::write(
        &home.join(".nvm/versions/node/v24.0.0/lib/node_modules/marker"),
        "NVM-LIB\n",
    );
    support::write(&home.join(".nvm/alias/default"), "FIXTURE-NVM-ALIAS\n");
    // rustup: Proxies in ~/.cargo/bin, Toolchains in ~/.rustup.
    exe(&home.join(".cargo/bin/rustup"), "#!/bin/sh\necho rustup\n");
    exe(&home.join(".cargo/bin/cargo"), "#!/bin/sh\necho cargo\n");
    support::write(
        &home.join(".cargo/credentials.toml"),
        "token = \"FIXTURE-CARGO-TOKEN\"\n",
    );
    support::write(
        &home.join(".rustup/settings.toml"),
        "default_toolchain = \"stable\"\n",
    );
    support::write(
        &home.join(".rustup/toolchains/stable/lib/rustlib/marker"),
        "RUSTUP-TC\n",
    );
    // Weitere PATH-Einträge, die keine Freigabe bekommen dürfen.
    exe(&home.join(".bin/dotty"), "#!/bin/sh\necho dot\n");
    exe(&home.join(".ssh/bin/sshtool"), "#!/bin/sh\necho ssh\n");
    exe(&home.join("bin/mine"), "#!/bin/sh\necho mine\n");
    support::write(&home.join(".ssh/id_ed25519"), support::FIXTURE_SSH);
    Tc {
        _tmp: tmp,
        home,
        ws,
        scratch,
        opt,
    }
}

impl Tc {
    fn path(&self) -> String {
        [
            self.opt.join("brew/bin"),
            self.home.join(".nvm/versions/node/v24.0.0/bin"),
            self.home.join(".cargo/bin"),
            self.home.join(".bin"),
            self.home.join(".ssh/bin"),
            self.home.join("bin"),
            self.home.clone(),
            PathBuf::from("/"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
            self.ws.join("node_modules/.bin"),
        ]
        .iter()
        .map(|p| p.display().to_string())
        .chain(["relativ/bin".to_owned()])
        .collect::<Vec<_>>()
        .join(":")
    }

    fn ctx(&self, stage: Stage) -> ResolveCtx {
        let mut ctx = ResolveCtx::new(stage, &self.ws, &self.home, &self.scratch);
        ctx.adapters = beton_sandbox::masks::builtin_harness_grants();
        ctx.host_env = [("PATH".to_owned(), self.path())].into_iter().collect();
        ctx
    }
}

fn masked(policy: &beton_sandbox::ResolvedPolicy, p: &Path) -> bool {
    policy
        .masks
        .iter()
        .any(|m| matches!(m, Mask::Path { path, .. } if p.starts_with(path)))
}

/// SBX-012, #241: Präfixe (Elternverzeichnis von `bin`) werden lesend frei, außer das Präfix
/// wäre `~` selbst oder ein Dotfile direkt in `~`; dann nur das `bin`-Verzeichnis. Nie
/// schreibend, nie ein maskierter Pfad, nie `~`, `/` oder relative Einträge.
#[test]
fn sbx_012_toolchain_dirs_from_path_are_read_only_grants_without_home_dotfiles() {
    let t = tc();
    let p = resolve(
        ProviderId::Seatbelt,
        &SandboxSpec::default(),
        &t.ctx(Stage::Tools),
    )
    .unwrap();
    let reads = &p.read_roots;
    for want in [
        t.opt.join("brew"),
        t.home.join(".nvm/versions/node/v24.0.0"),
        t.home.join(".cargo/bin"),
        t.home.join("bin"),
        t.home.join(".rustup/toolchains"),
        t.home.join(".rustup/settings.toml"),
    ] {
        assert!(
            reads.contains(&want),
            "{} fehlt in {reads:#?}",
            want.display()
        );
    }
    for never in [
        t.home.clone(),
        t.home.join(".cargo"),
        t.home.join(".nvm"),
        t.home.join(".bin"),
        t.home.join(".ssh/bin"),
        t.home.join(".ssh"),
        t.home.join(".rustup"),
        PathBuf::from("/"),
    ] {
        assert!(!reads.contains(&never), "{} freigegeben", never.display());
    }
    for w in &p.write_roots {
        assert!(
            !w.starts_with(&t.opt) && !w.starts_with(t.home.join(".nvm")),
            "schreibbar: {}",
            w.display()
        );
    }
    // Zugangsdaten in Präfixen bleiben maskiert (globale npmrc, Cargo-Credentials).
    assert!(masked(&p, &t.opt.join("brew/etc/npmrc")), "{:#?}", p.masks);
    assert!(p.masks.iter().any(|m| matches!(m, Mask::Pattern { root, glob, .. } if root == &t.home.join(".cargo") && glob == "credentials*")));
    // rustup findet seine Toolchains auch mit anderem HOME.
    assert_eq!(
        p.env.get("RUSTUP_HOME").map(String::as_str),
        Some(t.home.join(".rustup").to_str().unwrap())
    );
    // Hinweis in `explain`.
    assert!(
        p.notes
            .iter()
            .any(|n| n.contains("PATH") && n.contains("brew")),
        "{:#?}",
        p.notes
    );
    // Stufe 1 bekommt diese Freigaben nicht (dort gelten die Programm-Grants des Harness).
    let h = resolve(
        ProviderId::Seatbelt,
        &SandboxSpec::default(),
        &t.ctx(Stage::Harness),
    )
    .unwrap();
    assert!(!h.read_roots.contains(&t.opt.join("brew")));
}

/// #241: Ein Symlink im `PATH` wird kanonisch freigegeben (das Ziel), nie der Link allein auf
/// ein verbotenes Ziel.
#[test]
fn sbx_012_symlinked_path_entries_resolve_to_their_target() {
    let t = tc();
    let link = t.home.join("toolbin");
    std::os::unix::fs::symlink(t.home.join(".ssh/bin"), &link).unwrap();
    let mut ctx = t.ctx(Stage::Tools);
    ctx.host_env
        .insert("PATH".into(), link.display().to_string());
    let p = resolve(ProviderId::Seatbelt, &SandboxSpec::default(), &ctx).unwrap();
    assert!(
        !p.read_roots
            .iter()
            .any(|r| r.starts_with(t.home.join(".ssh"))),
        "{:#?}",
        p.read_roots
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod live {
    use super::*;
    use beton_sandbox::{BackendChoice, Registry, SandboxProvider};
    use std::sync::Arc;

    fn provider() -> Arc<dyn SandboxProvider> {
        #[cfg(target_os = "linux")]
        let registry = Registry::with_launcher(std::path::Path::new(env!(
            "CARGO_BIN_EXE_beton-sandbox-exec"
        )));
        #[cfg(not(target_os = "linux"))]
        let registry = Registry::platform();
        let p = registry.select(BackendChoice::Auto).unwrap();
        assert!(p.probe().available);
        p
    }

    /// SBX-012, #241: Werkzeuge aus Homebrew- und nvm-Präfixen laufen in Stufe 2 und lesen ihre
    /// Bibliotheken; Schreiben in die Präfixe scheitert, Zugangsdaten bleiben unlesbar.
    #[tokio::test(flavor = "multi_thread")]
    async fn sbx_012_tools_from_path_prefixes_run_in_stage_two_read_only() {
        let t = tc();
        let p = provider();
        let policy = p
            .resolve(
                &SandboxSpec::from_yaml("workspace: rw\n").unwrap(),
                &t.ctx(Stage::Tools),
            )
            .unwrap();
        let brew = t.opt.join("brew/bin/brewtool");
        let node = t.home.join(".nvm/versions/node/v24.0.0/bin/nodeish");
        let script = format!(
            "{brew}; {node}; echo x > {w1} || echo schreiben-verboten; cat {npmrc} || echo npmrc-maskiert; \
             cat {alias} || echo alias-unsichtbar; cat {key} || echo key-unsichtbar; cat {cred} || echo cargo-maskiert",
            brew = brew.display(),
            node = node.display(),
            w1 = t.opt.join("brew/neu").display(),
            npmrc = t.opt.join("brew/etc/npmrc").display(),
            alias = t.home.join(".nvm/alias/default").display(),
            key = t.home.join(".ssh/id_ed25519").display(),
            cred = t.home.join(".cargo/credentials.toml").display(),
        );
        let out = support::sh(&*p, &policy, &script).await;
        let text = format!("{}{}", out.stdout, out.stderr);
        for want in [
            "BREW-DATA",
            "NVM-LIB",
            "schreiben-verboten",
            "npmrc-maskiert",
            "alias-unsichtbar",
            "key-unsichtbar",
            "cargo-maskiert",
        ] {
            assert!(text.contains(want), "{want} fehlt: {out:?}");
        }
        for secret in [
            "FIXTURE-NPM-TOKEN",
            "FIXTURE-NVM-ALIAS",
            support::FIXTURE_SSH,
            "FIXTURE-CARGO-TOKEN",
        ] {
            assert!(!text.contains(secret), "{secret} gelesen: {out:?}");
        }
        assert!(!t.opt.join("brew/neu").exists());
        // Kontrollgruppe ohne Sandbox liest das Token.
        let ctl = support::control_in(
            &t.ws,
            &t.home,
            &format!("cat {}", t.opt.join("brew/etc/npmrc").display()),
        )
        .await;
        assert!(ctl.stdout.contains("FIXTURE-NPM-TOKEN"), "{ctl:?}");
    }

    /// #241: `/usr/bin/git` (xcrun-Stub) läuft in Stufe 2, auch wenn Xcode.app das
    /// Developer-Verzeichnis ist (`Info.plist`, `SharedFrameworks` lesbar).
    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread")]
    async fn sbx_012_usr_bin_git_runs_in_stage_two_with_xcode_app() {
        let t = tc();
        let p = provider();
        let policy = p
            .resolve(&SandboxSpec::default(), &t.ctx(Stage::Tools))
            .unwrap();
        let out = support::sh(&*p, &policy, "/usr/bin/git --version").await;
        assert!(out.ok && out.stdout.starts_with("git version"), "{out:?}");
    }
}
