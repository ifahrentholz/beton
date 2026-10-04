//! Worktrees, Änderungen und Snapshots gegen echte, temporäre Git-Repositories
//! (SES-015, SES-016, SES-018). Kein Netzwerk: Remotes sind lokale Pfade.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use beton_git::changes::{self, ChangeStatus};
use beton_git::worktree::{
    self, BranchAction, CreateOptions, FetchOutcome, RemoveError, RemoveOptions, Uncommitted,
    WorktreeError, WorktreeRef,
};
use beton_git::{Git, ShadowRepo};

fn git(dir: &Path, args: &[&str]) -> String {
    Git::new(dir)
        .with_config("user.name=Test")
        .with_config("user.email=test@example.invalid")
        .with_config("commit.gpgsign=false")
        .text(args)
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Repository mit `main` und einem Commit.
fn repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("projekt");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet", "-b", "main"]);
    write(&repo, "README.md", "eins\nzwei\ndrei\n");
    write(&repo, "src/lib.rs", "fn a() {}\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "--quiet", "-m", "start"]);
    let repo = repo.canonicalize().unwrap();
    (tmp, repo)
}

fn opts<'a>(cwd: &'a Path, root: &'a Path, tag: &'a str) -> CreateOptions<'a> {
    CreateOptions {
        cwd,
        root,
        base: None,
        branch: None,
        title: Some("Login reparieren"),
        tag,
        fetch: true,
        fetch_timeout: Duration::from_secs(15),
    }
}

fn wt_ref(c: &worktree::Created) -> WorktreeRef {
    WorktreeRef {
        repo: c.repo.clone(),
        path: c.path.clone(),
        branch: c.branch.clone(),
        base: c.base.clone(),
        base_sha: c.base_sha.clone(),
    }
}

#[test]
fn ses_015_ac1_two_worktrees_use_different_directories_and_branches() {
    let (tmp, repo) = repo();
    let root = tmp.path().join("worktrees");
    let a = worktree::create(&opts(&repo, &root, "aaaa")).unwrap();
    let b = worktree::create(&opts(&repo, &root, "bbbb")).unwrap();
    assert_ne!(a.path, b.path);
    assert_ne!(a.branch, b.branch);
    assert_eq!(a.branch, "beton/login-reparieren-aaaa");
    assert_eq!(a.base, "main", "ohne origin/HEAD: aktueller Branch");
    assert_eq!(a.base_sha, git(&repo, &["rev-parse", "main"]));
    assert_eq!(a.fetch, FetchOutcome::Skipped);
    assert!(a.path.starts_with(root.canonicalize().unwrap()));
    assert!(!a.path.starts_with(&repo), "außerhalb des Repositorys");
    let list = git(&repo, &["worktree", "list", "--porcelain"]);
    for c in [&a, &b] {
        assert!(list.contains(&c.path.display().to_string()), "{list}");
        assert!(list.contains(&format!("refs/heads/{}", c.branch)));
        assert_eq!(git(&c.path, &["branch", "--show-current"]), c.branch);
    }
    // Kein Upstream auf die Base (ein `push` ginge sonst nach main).
    assert!(
        Git::new(&a.path)
            .text(&["rev-parse", "--abbrev-ref", "@{upstream}"])
            .is_err()
    );
}

#[test]
fn ses_015_ac3_unresolvable_base_names_available_branches() {
    let (tmp, repo) = repo();
    git(&repo, &["branch", "feature/x"]);
    let root = tmp.path().join("worktrees");
    let mut o = opts(&repo, &root, "cccc");
    o.base = Some("gibt-es-nicht");
    let err = worktree::create(&o).unwrap_err();
    match &err {
        WorktreeError::BaseNotFound { base, available } => {
            assert_eq!(base, "gibt-es-nicht");
            assert!(available.contains(&"main".to_owned()));
            assert!(available.contains(&"feature/x".to_owned()));
        }
        other => panic!("{other}"),
    }
    assert!(err.to_string().contains("main"));
    assert!(!root.exists() || std::fs::read_dir(&root).unwrap().next().is_none());
    assert_eq!(git(&repo, &["worktree", "list"]).lines().count(), 1);
}

#[test]
fn ses_015_ac5_unreachable_remote_falls_back_to_local_base() {
    let (tmp, upstream) = repo();
    let clone = tmp.path().join("klon");
    git(
        tmp.path(),
        &["clone", "--quiet", &upstream.display().to_string(), "klon"],
    );
    // Remote „offline“: das Upstream-Verzeichnis ist weg.
    std::fs::rename(&upstream, tmp.path().join("weg")).unwrap();
    let root = tmp.path().join("worktrees");
    let created = worktree::create(&opts(&clone, &root, "dddd")).unwrap();
    assert_eq!(created.base, "origin/main", "origin/HEAD");
    match &created.fetch {
        FetchOutcome::Failed { remote, branch, .. } => {
            assert_eq!((remote.as_str(), branch.as_str()), ("origin", "main"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(created.base_sha, git(&clone, &["rev-parse", "origin/main"]));
    assert!(created.path.join("README.md").is_file());
}

#[test]
fn ses_015_fetch_updates_a_reachable_remote_base() {
    let (tmp, upstream) = repo();
    git(
        tmp.path(),
        &["clone", "--quiet", &upstream.display().to_string(), "klon"],
    );
    let clone = tmp.path().join("klon");
    write(&upstream, "neu.txt", "neu\n");
    git(&upstream, &["add", "-A"]);
    git(&upstream, &["commit", "--quiet", "-m", "neu"]);
    let root = tmp.path().join("worktrees");
    let created = worktree::create(&opts(&clone, &root, "eeee")).unwrap();
    assert!(matches!(created.fetch, FetchOutcome::Fetched { .. }));
    assert_eq!(created.base_sha, git(&upstream, &["rev-parse", "HEAD"]));
    assert!(created.path.join("neu.txt").is_file());
}

#[test]
fn ses_016_ac1_dirty_worktree_is_not_removed_without_decision() {
    let (tmp, repo) = repo();
    let root = tmp.path().join("worktrees");
    let c = worktree::create(&opts(&repo, &root, "ffff")).unwrap();
    write(&c.path, "README.md", "geändert\n");
    write(&c.path, "neu.txt", "x\n");
    let err = worktree::remove(&wt_ref(&c), RemoveOptions::default()).unwrap_err();
    match err {
        RemoveError::Dirty { files } => {
            assert!(files.contains(&"README.md".to_owned()), "{files:?}");
            assert!(files.contains(&"neu.txt".to_owned()));
        }
        other => panic!("{other}"),
    }
    assert!(c.path.join("neu.txt").is_file(), "nichts entfernt");
    assert_eq!(
        git(&repo, &["branch", "--list", &c.branch]).trim_start_matches(['*', '+', ' ']),
        c.branch
    );

    // WIP-Commit: Branch hat dann eigene, ungepushte Commits → zweite Rückfrage.
    let err = worktree::remove(
        &wt_ref(&c),
        RemoveOptions {
            uncommitted: Some(Uncommitted::Commit),
            branch: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, RemoveError::Unpushed { .. }), "{err}");
    assert!(c.path.join("neu.txt").is_file(), "nichts entfernt");

    let removed = worktree::remove(
        &wt_ref(&c),
        RemoveOptions {
            uncommitted: Some(Uncommitted::Commit),
            branch: Some(BranchAction::Keep),
        },
    )
    .unwrap();
    let wip = removed.wip_commit.unwrap();
    assert!(!removed.branch_deleted);
    assert!(!c.path.exists());
    assert_eq!(
        git(&repo, &["rev-parse", &c.branch]),
        wip,
        "Arbeit bleibt im Branch"
    );
}

#[test]
fn ses_016_discard_removes_changes_and_unchanged_branch() {
    let (tmp, repo) = repo();
    let root = tmp.path().join("worktrees");
    let c = worktree::create(&opts(&repo, &root, "gggg")).unwrap();
    write(&c.path, "neu.txt", "x\n");
    let removed = worktree::remove(
        &wt_ref(&c),
        RemoveOptions {
            uncommitted: Some(Uncommitted::Discard),
            branch: None,
        },
    )
    .unwrap();
    assert!(removed.branch_deleted, "keine eigenen Commits");
    assert!(!c.path.exists());
}

#[test]
fn ses_016_ac2_merged_branch_and_directory_are_removed() {
    let (tmp, repo) = repo();
    let root = tmp.path().join("worktrees");
    let c = worktree::create(&opts(&repo, &root, "hhhh")).unwrap();
    write(&c.path, "feature.txt", "fertig\n");
    git(&c.path, &["add", "-A"]);
    git(&c.path, &["commit", "--quiet", "-m", "feature"]);
    // Unpushed und nicht gemergt: Rückfrage.
    assert!(matches!(
        worktree::remove(&wt_ref(&c), RemoveOptions::default()),
        Err(RemoveError::Unpushed { commits: 1, .. })
    ));
    git(&repo, &["merge", "--quiet", "--ff-only", &c.branch]);
    let removed = worktree::remove(&wt_ref(&c), RemoveOptions::default()).unwrap();
    assert!(removed.branch_deleted);
    assert!(!c.path.exists());
    assert!(git(&repo, &["branch", "--list", &c.branch]).is_empty());
    assert_eq!(git(&repo, &["worktree", "list"]).lines().count(), 1);
}

#[test]
fn ses_016_ac3_orphaned_worktree_directories_are_found_with_size() {
    let (tmp, repo) = repo();
    let root = tmp.path().join("worktrees");
    let kept = worktree::create(&opts(&repo, &root, "iiii")).unwrap();
    let orphan = worktree::create(&opts(&repo, &root, "jjjj")).unwrap();
    let found = worktree::orphans(&root, std::slice::from_ref(&kept.path)).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, orphan.path);
    assert!(found[0].size >= "eins\nzwei\ndrei\n".len() as u64);
}

#[test]
fn ses_018_ac2_branch_scope_matches_git_diff_merge_base() {
    let (tmp, repo) = repo();
    let root = tmp.path().join("worktrees");
    let c = worktree::create(&opts(&repo, &root, "kkkk")).unwrap();
    write(&c.path, "README.md", "eins\nZWEI\ndrei\nvier\n");
    write(&c.path, "neu/datei.txt", "a\nb\n");
    std::fs::remove_file(c.path.join("src/lib.rs")).unwrap();
    git(&c.path, &["add", "-A"]);
    git(&c.path, &["commit", "--quiet", "-m", "änderungen"]);
    // Die Base läuft weiter; das darf die Branch-Sicht nicht beeinflussen.
    write(&repo, "main-only.txt", "x\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "--quiet", "-m", "main"]);

    let (mb, head, files) = changes::branch(&c.path, "main").unwrap();
    assert_eq!(mb, c.base_sha);
    assert_eq!(head, git(&c.path, &["rev-parse", "HEAD"]));
    // Vergleich mit `git diff --numstat <merge-base>...HEAD`.
    let expected: Vec<(String, String, String)> =
        git(&c.path, &["diff", "--numstat", "main...HEAD"])
            .lines()
            .map(|l| {
                let mut p = l.split('\t');
                (
                    p.next().unwrap().to_owned(),
                    p.next().unwrap().to_owned(),
                    p.next().unwrap().to_owned(),
                )
            })
            .collect();
    let actual: Vec<(String, String, String)> = files
        .iter()
        .map(|f| {
            (
                f.additions.unwrap().to_string(),
                f.deletions.unwrap().to_string(),
                f.path.clone(),
            )
        })
        .collect();
    let mut expected_sorted = expected.clone();
    expected_sorted.sort_by(|a, b| a.2.cmp(&b.2));
    assert_eq!(actual, expected_sorted);
    let status: Vec<_> = files.iter().map(|f| (f.path.as_str(), f.status)).collect();
    assert!(status.contains(&("neu/datei.txt", ChangeStatus::Added)));
    assert!(status.contains(&("src/lib.rs", ChangeStatus::Deleted)));
    assert!(status.contains(&("README.md", ChangeStatus::Modified)));

    let (_, _, diff) = changes::diff_branch(&c.path, "main", "README.md").unwrap();
    let diff = diff.unwrap();
    assert!(diff.patch.contains("+ZWEI"));
    assert_eq!(diff.hunks.len(), 1);
}

#[test]
fn ses_018_uncommitted_scope_includes_untracked_files() {
    let (_tmp, repo) = repo();
    write(&repo, "README.md", "eins\n");
    write(&repo, "neu.txt", "a\nb\nc\n");
    write(&repo, ".gitignore", "*.log\n");
    write(&repo, "x.log", "ignoriert\n");
    let (base, files) = changes::uncommitted(&repo).unwrap();
    assert_eq!(base, git(&repo, &["rev-parse", "HEAD"]));
    let by_path: Vec<_> = files
        .iter()
        .map(|f| (f.path.as_str(), f.status, f.additions, f.deletions))
        .collect();
    assert_eq!(
        by_path,
        vec![
            (".gitignore", ChangeStatus::Added, Some(1), Some(0)),
            ("README.md", ChangeStatus::Modified, Some(0), Some(2)),
            ("neu.txt", ChangeStatus::Added, Some(3), Some(0)),
        ]
    );
    let d = changes::diff_uncommitted(&repo, "neu.txt")
        .unwrap()
        .unwrap();
    assert_eq!(d.status, ChangeStatus::Added);
    assert_eq!(d.hunks[0].lines.len(), 3);
    assert_eq!(d.hunks[0].lines[2].new_line, Some(3));
    assert!(
        changes::diff_uncommitted(&repo, "src/lib.rs")
            .unwrap()
            .is_none()
    );
}

#[test]
fn ses_018_ac3_snapshots_work_without_git_repository() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ohne-git");
    std::fs::create_dir_all(&ws).unwrap();
    write(&ws, "a.txt", "eins\nzwei\n");
    assert!(!changes::is_repo(&ws));
    let shadow = ShadowRepo::open(&tmp.path().join("shadow.git"), &ws).unwrap();
    let before = shadow.snapshot().unwrap();
    write(&ws, "a.txt", "eins\nZWEI\ndrei\n");
    write(&ws, "b.txt", "neu\n");
    let after = shadow.snapshot().unwrap();
    let files = shadow.changes(&before, &after).unwrap();
    let got: Vec<_> = files
        .iter()
        .map(|f| (f.path.as_str(), f.status, f.additions, f.deletions))
        .collect();
    assert_eq!(
        got,
        vec![
            ("a.txt", ChangeStatus::Modified, Some(2), Some(1)),
            ("b.txt", ChangeStatus::Added, Some(1), Some(0)),
        ]
    );
    let d = shadow.diff(&before, &after, "a.txt").unwrap().unwrap();
    assert!(d.patch.contains("-zwei"));
    // Kein `.git` im Workspace entstanden.
    assert!(!ws.join(".git").exists());
    // Detached-Snapshot sieht denselben Stand, ohne den Index zu verändern.
    assert_eq!(shadow.snapshot_detached().unwrap(), after);
    shadow
        .set_ref("refs/beton/turns/t1/before", &before)
        .unwrap();
    assert_eq!(
        shadow.get_ref("refs/beton/turns/t1/before").as_deref(),
        Some(before.as_str())
    );
    assert!(shadow.get_ref("refs/beton/turns/t2/before").is_none());
}

#[test]
fn snapshots_respect_gitignore_and_leave_the_user_repo_untouched() {
    let (tmp, repo) = repo();
    write(&repo, ".gitignore", "target/\n");
    write(&repo, "target/big.bin", "x");
    let refs_before = git(&repo, &["for-each-ref"]);
    let count_before = git(&repo, &["count-objects"]);
    let shadow = ShadowRepo::open(&tmp.path().join("s.git"), &repo).unwrap();
    let a = shadow.snapshot().unwrap();
    write(&repo, "target/big.bin", "y");
    write(&repo, "src/lib.rs", "fn b() {}\n");
    let b = shadow.snapshot().unwrap();
    let files = shadow.changes(&a, &b).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "src/lib.rs");
    assert_eq!(git(&repo, &["for-each-ref"]), refs_before);
    assert_eq!(git(&repo, &["count-objects"]), count_before);
}

#[test]
fn shadow_repository_inside_the_workspace_is_not_snapshotted() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    write(&ws, "a.txt", "a\n");
    // Datenverzeichnis im Workspace (z. B. Workspace = $HOME mit ~/.beton).
    let shadow = ShadowRepo::open(&ws.join(".beton/snapshots/s1.git"), &ws).unwrap();
    let first = shadow.snapshot().unwrap();
    write(&ws, ".beton/beton.db", "daten");
    let second = shadow.snapshot().unwrap();
    assert_eq!(first, second, "nur das Datenverzeichnis hat sich geändert");
    assert!(!shadow.exceeds_files(1).unwrap());
    write(&ws, "b.txt", "b\n");
    assert!(shadow.exceeds_files(1).unwrap());
    assert!(!shadow.exceeds_files(10).unwrap());
}
