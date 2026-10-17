//! SBX-003 und SBX-004 auf Ebene der Auflösung (plattformunabhängig): Laden, Ebenen,
//! Pfadauflösung und Masken. Die Durchsetzung im Kernel prüft `tests/seatbelt.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use beton_sandbox::{
    GitLayout, HarnessGrants, Mask, NetMode, ProviderId, ProxyEndpoint, ResolveCtx, SandboxError,
    SandboxSpec, Stage, resolve,
};

/// Gefälschtes Home mit Fixture-Secrets, Workspace und Scratch in einem Temp-Verzeichnis.
struct Env {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    ws: PathBuf,
    scratch: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let home = root.join("home");
    let ws = root.join("ws");
    let scratch = root.join("scratch");
    for d in [&home, &ws, &scratch] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/id_ed25519"), "FIXTURE-SSH-KEY").unwrap();
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    Env {
        _tmp: tmp,
        home,
        ws,
        scratch,
    }
}

impl Env {
    fn ctx(&self, stage: Stage) -> ResolveCtx {
        ResolveCtx::new(stage, &self.ws, &self.home, &self.scratch)
    }

    fn resolve(
        &self,
        yaml: &str,
        stage: Stage,
    ) -> Result<beton_sandbox::ResolvedPolicy, SandboxError> {
        let spec = SandboxSpec::from_yaml(yaml)?;
        resolve(ProviderId::Seatbelt, &spec, &self.ctx(stage))
    }
}

fn mask_paths(p: &beton_sandbox::ResolvedPolicy) -> Vec<PathBuf> {
    p.masks
        .iter()
        .filter_map(|m| match m {
            Mask::Path { path, .. } => Some(path.clone()),
            Mask::Pattern { .. } => None,
        })
        .collect()
}

#[test]
fn sbx_003_ac1_write_paths_home_is_rejected_when_loading() {
    let err = SandboxSpec::from_yaml("write_paths: [\"~\"]\n").unwrap_err();
    match &err {
        SandboxError::InvalidSpec { path, message } => {
            assert_eq!(path, "write_paths[0]");
            assert!(message.contains('~'), "{message}");
        }
        other => panic!("{other}"),
    }
    for (yaml, field) in [
        ("write_paths: [/]\n", "write_paths[0]"),
        ("read_paths: [/]\n", "read_paths[0]"),
        ("write_paths: [\"~/\"]\n", "write_paths[0]"),
        ("write_paths: [/opt, \"~/Library\"]\n", "write_paths[1]"),
        (
            "write_paths: [\"~/Library/LaunchAgents\"]\n",
            "write_paths[0]",
        ),
        ("read_paths: [\"~/.beton\"]\n", "read_paths[0]"),
        ("read_paths: [\"~/.ssh/config\"]\n", "read_paths[0]"),
        ("read_paths: [/proc/self]\n", "read_paths[0]"),
        ("read_paths: [/sys]\n", "read_paths[0]"),
        ("write_paths: [/var/run/docker.sock]\n", "write_paths[0]"),
        ("read_paths: [\"~/x/../.ssh\"]\n", "read_paths[0]"),
    ] {
        match SandboxSpec::from_yaml(yaml).unwrap_err() {
            SandboxError::InvalidSpec { path, .. } => assert_eq!(path, field, "{yaml}"),
            other => panic!("{yaml}: {other}"),
        }
    }
    // `read_paths: ["~"]` ist erlaubt (unterliegt den Masken).
    SandboxSpec::from_yaml("read_paths: [\"~\"]\n").unwrap();
}

#[test]
fn sbx_003_ac1_ancestor_of_home_is_rejected_when_resolving() {
    let e = env();
    let parent = e.home.parent().unwrap().display().to_string();
    for yaml in [
        format!("write_paths: [\"{parent}\"]\n"),
        format!("read_paths: [\"{parent}\"]\n"),
        format!("write_paths: [\"{}\"]\n", e.home.display()),
    ] {
        let err = e.resolve(&yaml, Stage::Tools).unwrap_err();
        assert_eq!(err.code(), "invalid_spec", "{yaml}: {err}");
    }
}

#[cfg(unix)]
#[test]
fn sbx_003_ac2_symlink_into_ssh_is_path_escape() {
    let e = env();
    std::os::unix::fs::symlink(e.home.join(".ssh"), e.home.join("link")).unwrap();
    let err = e
        .resolve("read_paths: [\"~/link\"]\n", Stage::Tools)
        .unwrap_err();
    match &err {
        SandboxError::PathEscape { path, message } => {
            assert_eq!(path, "read_paths[0]");
            assert!(message.contains(".ssh"), "{message}");
        }
        other => panic!("{other}"),
    }
    // Auch über eine Kette von Symlinks und als Schreib-Grant.
    std::os::unix::fs::symlink(e.home.join("link"), e.home.join("link2")).unwrap();
    let err = e
        .resolve("write_paths: [\"~/link2\"]\n", Stage::Harness)
        .unwrap_err();
    assert_eq!(err.code(), "path_escape", "{err}");
    // Ein Symlink auf `/` oder auf das Home als Schreib-Wurzel ebenso.
    std::os::unix::fs::symlink("/", e.home.join("root")).unwrap();
    assert_eq!(
        e.resolve("read_paths: [\"~/root\"]\n", Stage::Tools)
            .unwrap_err()
            .code(),
        "path_escape"
    );
    std::os::unix::fs::symlink(&e.home, e.ws.parent().unwrap().join("h")).unwrap();
    let raw = e.ws.parent().unwrap().join("h").display().to_string();
    assert_eq!(
        e.resolve(&format!("write_paths: [\"{raw}\"]\n"), Stage::Tools)
            .unwrap_err()
            .code(),
        "path_escape"
    );
}

#[cfg(unix)]
#[test]
fn sbx_003_ac2_hardlinked_file_grant_is_rejected() {
    let e = env();
    std::fs::hard_link(e.home.join(".ssh/id_ed25519"), e.home.join("Documents/key")).unwrap();
    let err = e
        .resolve("read_paths: [\"~/Documents/key\"]\n", Stage::Tools)
        .unwrap_err();
    assert_eq!(err.code(), "path_escape", "{err}");
}

#[test]
fn sbx_003_ac3_dollar_home_is_not_expanded_and_rejected_as_relative() {
    for raw in ["$HOME/x", "${HOME}/x", "x/y", "./x", "~root/x", ""] {
        let yaml = format!("read_paths: [\"{raw}\"]\n");
        match SandboxSpec::from_yaml(&yaml).unwrap_err() {
            SandboxError::InvalidSpec { path, message } => {
                assert_eq!(path, "read_paths[0]", "{raw}");
                if raw.starts_with('$') {
                    assert!(message.contains("relativ"), "{message}");
                }
            }
            other => panic!("{raw}: {other}"),
        }
    }
    // Auch in Masken mit Pfad und als Schreib-Grant.
    assert!(SandboxSpec::from_yaml("write_paths: [\"$TMPDIR\"]\n").is_err());
}

#[test]
fn sbx_003_missing_grants_are_not_granted_but_noted() {
    let e = env();
    let p = e
        .resolve(
            "read_paths: [\"~/.cargo/registry\", \"~/Documents\"]\n",
            Stage::Tools,
        )
        .unwrap();
    assert!(p.read_roots.contains(&e.home.join("Documents")));
    assert!(!p.read_roots.iter().any(|r| r.ends_with("registry")));
    assert!(
        p.notes.iter().any(|n| n.contains(".cargo/registry")),
        "{:?}",
        p.notes
    );
}

#[test]
fn sbx_003_workspace_and_scratch_are_write_roots_and_env_is_explicit() {
    let e = env();
    let mut ctx = e.ctx(Stage::Tools);
    ctx.host_env = [
        ("PATH", "/usr/bin:/bin"),
        ("AWS_SECRET_ACCESS_KEY", "x"),
        ("SSH_AUTH_SOCK", "/tmp/agent"),
        ("LC_ALL", "de_DE.UTF-8"),
        ("RUST_LOG", "debug"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let spec =
        SandboxSpec::from_yaml("env: {passthrough: [RUST_LOG], set: {CI: \"1\"}}\n").unwrap();
    let p = resolve(ProviderId::Seatbelt, &spec, &ctx).unwrap();
    assert!(p.write_roots.contains(&e.ws));
    assert!(p.write_roots.contains(&e.scratch));
    assert_eq!(p.env["HOME"], e.home.display().to_string());
    assert_eq!(p.env["TMPDIR"], e.scratch.display().to_string());
    assert_eq!(p.env["BETON_STAGE"], "tools");
    assert_eq!(p.env["RUST_LOG"], "debug");
    assert_eq!(p.env["CI"], "1");
    assert_eq!(p.env["LC_ALL"], "de_DE.UTF-8");
    assert!(!p.env.contains_key("AWS_SECRET_ACCESS_KEY"));
    assert!(!p.env.contains_key("SSH_AUTH_SOCK"));
    // `workspace: ro` macht den Workspace nur lesbar.
    let ro = e.resolve("workspace: ro\n", Stage::Tools).unwrap();
    assert!(!ro.write_roots.contains(&e.ws));
    assert!(ro.read_roots.contains(&e.ws));
}

#[test]
fn sbx_003_denied_env_names_are_rejected_when_loading() {
    for yaml in [
        "env: {passthrough: [SSH_AUTH_SOCK]}\n",
        "env: {passthrough: [\"DYLD_*\"]}\n",
        "env: {passthrough: [\"*\"]}\n",
        "env: {set: {LD_PRELOAD: /x.so}}\n",
        "env: {set: {BETON_TOKEN: x}}\n",
    ] {
        let err = SandboxSpec::from_yaml(yaml).unwrap_err();
        assert_eq!(err.code(), "invalid_spec", "{yaml}");
    }
}

#[test]
fn sbx_003_layers_merge_grants_from_most_specific_and_union_masks() {
    let user = SandboxSpec::from_yaml(
        "read_paths: [/opt/homebrew]\nmasks: [secrets/]\nenv: {set: {A: \"1\", B: \"1\"}}\nallow_network: false\n",
    )
    .unwrap();
    let project = SandboxSpec::from_yaml("masks: [\"*.pem\"]\nenv: {set: {B: \"2\"}}\n").unwrap();
    let agent =
        SandboxSpec::from_yaml("read_paths: [\"~/.cargo/registry\"]\nbackend: none\n").unwrap();
    let m = SandboxSpec::merged(&[&user, &project, &agent]);
    assert_eq!(m.read_paths, Some(vec!["~/.cargo/registry".to_owned()]));
    assert_eq!(
        m.masks,
        Some(vec!["secrets/".to_owned(), "*.pem".to_owned()])
    );
    let env = m.env.unwrap();
    assert_eq!(env.set["A"], "1");
    assert_eq!(env.set["B"], "2");
    assert_eq!(m.allow_network, Some(false));
    assert_eq!(m.backend.unwrap().as_str(), "none");
}

#[test]
fn sbx_003_net_only_via_proxy_and_none_without_proxy() {
    let e = env();
    let mut ctx = e.ctx(Stage::Tools);
    let spec = SandboxSpec::default();
    assert_eq!(
        resolve(ProviderId::Seatbelt, &spec, &ctx).unwrap().net,
        NetMode::None
    );
    ctx.proxy = Some(ProxyEndpoint::port(4711));
    assert_eq!(
        resolve(ProviderId::Seatbelt, &spec, &ctx).unwrap().net,
        NetMode::ProxyOnly { port: 4711 }
    );
    let off = SandboxSpec::from_yaml("allow_network: false\n").unwrap();
    assert_eq!(
        resolve(ProviderId::Seatbelt, &off, &ctx).unwrap().net,
        NetMode::None
    );
}

fn git(dir: &Path, args: &[&str]) {
    let gitbin = std::env::var("BETON_TEST_GIT").unwrap_or_else(|_| "git".into());
    #[allow(clippy::disallowed_methods)]
    let out = std::process::Command::new(gitbin)
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

#[cfg(unix)]
#[test]
fn sbx_003_git_worktree_layout_is_validated_in_both_directions() {
    let e = env();
    let repo = e.ws.parent().unwrap().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(
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
            "i",
        ],
    );
    let wt = e.ws.join("wt");
    git(&repo, &["worktree", "add", "-q", wt.to_str().unwrap()]);
    let layout = GitLayout::discover(&wt.canonicalize().unwrap())
        .unwrap()
        .unwrap();
    assert!(layout.dot_git_is_file);
    assert_eq!(layout.common_dir, repo.canonicalize().unwrap().join(".git"));
    assert!(layout.git_dir.starts_with(&layout.common_dir));
    let roots = layout.write_roots();
    assert!(roots.contains(&layout.common_dir.join("objects")));
    assert!(
        !roots.contains(&layout.common_dir),
        "das ganze .git ist keine Schreib-Wurzel"
    );

    // Ein gefälschtes `.git` (zeigt auf ein fremdes Verzeichnis) gibt nichts frei.
    let fake = e.ws.join("fake");
    std::fs::create_dir_all(&fake).unwrap();
    std::fs::write(fake.join(".git"), format!("gitdir: {}\n", e.home.display())).unwrap();
    assert_eq!(
        GitLayout::discover(&fake).unwrap_err().code(),
        "invalid_spec"
    );
    let other = e.ws.join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join(".git"),
        format!("gitdir: {}\n", layout.git_dir.display()),
    )
    .unwrap();
    assert_eq!(
        GitLayout::discover(&other).unwrap_err().code(),
        "invalid_spec",
        "Rückverweis fehlt"
    );
    // Ein normales Repo: `.git` liegt im Workspace, keine zusätzlichen Wurzeln.
    let plain = GitLayout::discover(&repo.canonicalize().unwrap())
        .unwrap()
        .unwrap();
    assert!(!plain.dot_git_is_file);
    assert!(plain.write_roots().is_empty());
}

#[test]
fn sbx_004_default_masks_cover_credential_paths_in_both_stages() {
    let e = env();
    for stage in [Stage::Harness, Stage::Tools] {
        let p = e.resolve("read_paths: [\"~\"]\n", stage).unwrap();
        let masks = mask_paths(&p);
        for rel in [
            ".ssh",
            ".aws",
            ".config/gh",
            ".netrc",
            ".gnupg",
            ".beton",
            ".docker/config.json",
            "Library/Keychains",
        ] {
            assert!(masks.contains(&e.home.join(rel)), "{stage:?}: {rel} fehlt");
        }
        assert!(
            p.masks
                .iter()
                .any(|m| matches!(m, Mask::Pattern { glob, .. } if glob == "credentials*"))
        );
        // Dotfiles direkt im Grant `~` sind maskiert.
        assert!(p.hidden.iter().any(|h| h.root == e.home));
        assert!(p.read_roots.contains(&e.home));
    }
}

#[test]
fn sbx_004_ac2_workspace_masks_default_to_env_files_and_can_be_cleared() {
    let e = env();
    let p = e.resolve("{}\n", Stage::Tools).unwrap();
    let globs: Vec<&str> = p
        .masks
        .iter()
        .filter_map(|m| match m {
            Mask::Pattern {
                root,
                glob,
                any_depth: true,
                ..
            } if *root == e.ws => Some(glob.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(globs, [".env", ".env.*"]);
    // Der Workspace selbst ist keine Grant-Wurzel: seine Dotfiles bleiben sichtbar.
    assert!(!p.hidden.iter().any(|h| h.root == e.ws));
    let open = e
        .resolve("masks: []\nallow_hidden: [.env]\n", Stage::Tools)
        .unwrap();
    assert!(
        !open
            .masks
            .iter()
            .any(|m| matches!(m, Mask::Pattern { root, .. } if *root == e.ws))
    );
}

#[test]
fn sbx_004_ac3_stage_two_masks_all_adapter_paths_stage_one_grants_its_own() {
    let e = env();
    std::fs::create_dir_all(e.home.join(".codex")).unwrap();
    std::fs::write(e.home.join(".codex/auth.json"), "FIXTURE-CODEX").unwrap();
    let codex = beton_sandbox::masks::harness_grants("codex").unwrap();
    let mut ctx = e.ctx(Stage::Tools);
    ctx.adapters = beton_sandbox::masks::builtin_harness_grants();
    let spec = SandboxSpec::default();
    let tools = resolve(ProviderId::Seatbelt, &spec, &ctx).unwrap();
    let masks = mask_paths(&tools);
    assert!(masks.contains(&e.home.join(".codex/auth.json")));
    assert!(masks.contains(&e.home.join(".codex")));
    assert!(masks.contains(&e.home.join(".claude")));
    assert!(!tools.keychain);

    let mut ctx = e.ctx(Stage::Harness);
    ctx.harness = Some(codex);
    ctx.adapters = beton_sandbox::masks::builtin_harness_grants();
    let harness = resolve(ProviderId::Seatbelt, &spec, &ctx).unwrap();
    assert!(!mask_paths(&harness).contains(&e.home.join(".codex/auth.json")));
    assert!(
        harness
            .read_roots
            .contains(&e.home.join(".codex/auth.json"))
    );
    assert!(harness.write_roots.contains(&e.home.join(".codex")));
}

#[test]
fn sbx_004_keychain_only_in_stage_one_with_declared_need() {
    let e = env();
    let claude = HarnessGrants {
        harness: "claude".into(),
        keychain: true,
        ..HarnessGrants::default()
    };
    let mut ctx = e.ctx(Stage::Harness);
    ctx.harness = Some(claude.clone());
    let p = resolve(ProviderId::Seatbelt, &SandboxSpec::default(), &ctx).unwrap();
    assert!(p.keychain);
    assert!(!mask_paths(&p).contains(&e.home.join("Library/Keychains")));
    let mut ctx = e.ctx(Stage::Tools);
    ctx.harness = Some(claude);
    let p = resolve(ProviderId::Seatbelt, &SandboxSpec::default(), &ctx).unwrap();
    assert!(!p.keychain, "Stufe 2 bekommt nie den Keychain");
    assert!(mask_paths(&p).contains(&e.home.join("Library/Keychains")));
}

#[test]
fn sbx_004_grant_inside_a_mask_is_rejected() {
    let e = env();
    std::fs::create_dir_all(e.home.join(".aws/sso")).unwrap();
    let err = e
        .resolve("read_paths: [\"~/.aws/sso\"]\n", Stage::Tools)
        .unwrap_err();
    assert_eq!(err.code(), "path_escape", "{err}");
    let err = e.resolve("masks: [\"a/b\"]\n", Stage::Tools).unwrap_err();
    assert_eq!(err.code(), "invalid_spec", "Muster mit `/` sind umgehbar");
}

/// #203 (SBX-004): Seatbelt prüft Pfade, Landlock und die Masken-Mounts hängen am Zugriffspfad:
/// Ein außerhalb der Sandbox angelegter Hardlink auf eine maskierte Datei wäre über den Link in
/// einer Freigabe lesbar (Restrisiko). Die Auflösung meldet maskierte Dateien mit weiteren
/// Hardlinks deshalb (Hinweis in `explain`, Warnung beim Start der Session), in beiden Stufen
/// und auch, wenn der Link nicht in einer Freigabe liegt (ihn zu suchen wäre nicht praktikabel).
/// Kontrollgruppe: ohne zweiten Link keine Meldung.
#[cfg(unix)]
#[test]
fn sbx_004_masked_files_with_further_hardlinks_are_reported() {
    let e = env();
    let key = e.home.join(".ssh/id_ed25519");
    for stage in [Stage::Harness, Stage::Tools] {
        let p = e.resolve("read_paths: [\"~\"]\n", stage).unwrap();
        assert!(p.hardlinked_masks.is_empty(), "{:?}", p.hardlinked_masks);
    }
    std::fs::hard_link(&key, e.home.join("Documents/key")).unwrap();
    std::fs::write(e.home.join(".netrc"), "FIXTURE-NETRC").unwrap();
    std::fs::hard_link(e.home.join(".netrc"), e.ws.join("netrc")).unwrap();
    for stage in [Stage::Harness, Stage::Tools] {
        let p = e.resolve("read_paths: [\"~\"]\n", stage).unwrap();
        assert_eq!(
            p.hardlinked_masks,
            vec![e.home.join(".netrc"), key.clone()],
            "{stage:?}"
        );
        assert!(
            p.notes
                .iter()
                .any(|n| n.contains(".ssh/id_ed25519") && n.contains("Hardlink")),
            "{:?}",
            p.notes
        );
    }
}

/// #211: Beide Stufen verdecken das Elternverzeichnis des Session-Verzeichnisses (alle
/// Sessions; unter Linux per tmpfs), nicht nur das eigene. Liegt es in einer Freigabe, ist das
/// ein Fehler (fail closed).
#[test]
fn sbx_008_session_directories_of_all_sessions_are_private_in_both_stages() {
    let e = env();
    let base = e.home.parent().unwrap().join("sessions");
    let session = base.join("beton-sbx-eins");
    std::fs::create_dir_all(&session).unwrap();
    for stage in [Stage::Harness, Stage::Tools] {
        let mut ctx = e.ctx(stage);
        ctx.session_dir = Some(session.clone());
        let p = resolve(ProviderId::Seatbelt, &SandboxSpec::default(), &ctx).unwrap();
        assert_eq!(p.private, vec![base.clone()], "{stage:?}");
        let spec =
            SandboxSpec::from_yaml(&format!("read_paths: [\"{}\"]\n", base.display())).unwrap();
        assert!(
            matches!(
                resolve(ProviderId::Seatbelt, &spec, &ctx),
                Err(SandboxError::PathEscape { .. })
            ),
            "{stage:?}"
        );
    }
}
