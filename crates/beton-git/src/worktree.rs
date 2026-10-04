//! Worktree pro Session: anlegen mit Base-Auflösung (SES-015), kontrolliert entfernen ohne
//! Arbeit zu verlieren (SES-016), verwaiste Verzeichnisse finden.
//!
//! Ablage: `<root>/<repo-slug>-<hash8>/<branch-slug>/` außerhalb des Repositorys, damit es
//! nicht verschmutzt wird. Branch-Name `beton/<titel-slug>-<id4>`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::cmd::{Git, GitError, split_nul};

/// Standard-Timeout für `git fetch` vor dem Anlegen.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Optionen für [`create`].
#[derive(Debug, Clone)]
pub struct CreateOptions<'a> {
    /// Verzeichnis im Repository (Arbeitsverzeichnis der Anfrage).
    pub cwd: &'a Path,
    /// Wurzel aller Worktrees, z. B. `~/.beton/worktrees`.
    pub root: &'a Path,
    /// Explizite Base; sonst `origin/HEAD`, sonst der aktuelle Branch.
    pub base: Option<&'a str>,
    /// Expliziter Branch-Name; sonst `beton/<titel-slug>-<tag>`.
    pub branch: Option<&'a str>,
    /// Titel der Session für den Branch-Namen.
    pub title: Option<&'a str>,
    /// Kurzkennung der Session (z. B. die letzten vier Zeichen der ID).
    pub tag: &'a str,
    /// `git fetch <remote> <branch>` vor dem Anlegen (nur bei Remote-Tracking-Bases).
    pub fetch: bool,
    pub fetch_timeout: Duration,
}

/// Ergebnis des Fetch vor dem Anlegen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    /// Base ist kein Remote-Tracking-Branch oder Fetch abgeschaltet.
    Skipped,
    Fetched {
        remote: String,
        branch: String,
    },
    /// Remote nicht erreichbar: Der Worktree entstand aus dem lokalen Stand (ADR-0033).
    Failed {
        remote: String,
        branch: String,
        reason: String,
    },
}

/// Ein angelegter Worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    /// Wurzel des Haupt-Checkouts.
    pub repo: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    pub base: String,
    pub base_sha: String,
    pub fetch: FetchOutcome,
}

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("kein Git-Repository")]
    NotARepo,
    #[error("Base `{base}` ist nicht auflösbar; verfügbare Branches: {}", available.join(", "))]
    BaseNotFound {
        base: String,
        available: Vec<String>,
    },
    #[error("Branch-Name `{0}` ist ungültig")]
    InvalidBranch(String),
    #[error("{0} existiert bereits")]
    Exists(String),
    #[error("{0}")]
    Git(#[from] GitError),
    #[error("E/A-Fehler: {0}")]
    Io(#[from] std::io::Error),
}

/// Kleinbuchstaben, Ziffern und `-`; höchstens `max` Zeichen.
pub fn slug(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let mut out: String = out.trim_matches('-').chars().take(max).collect();
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Standard-Branch `beton/<titel-slug>-<tag>`.
pub fn default_branch(title: Option<&str>, tag: &str) -> String {
    let title = title.map(|t| slug(t, 40)).filter(|s| !s.is_empty());
    format!(
        "beton/{}-{}",
        title.as_deref().unwrap_or("session"),
        slug(tag, 8)
    )
}

/// Verzeichnis eines Repositorys unter der Worktree-Wurzel: `<repo-slug>-<hash8>`.
pub fn repo_dir_name(repo: &Path) -> String {
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let hash = hex::encode(Sha256::digest(repo.to_string_lossy().as_bytes()));
    let slug = slug(&name, 40);
    format!(
        "{}-{}",
        if slug.is_empty() { "repo" } else { &slug },
        &hash[..8]
    )
}

/// Wurzel des Haupt-Checkouts zu einem Verzeichnis in einem Repository oder Worktree.
pub fn main_checkout(dir: &Path) -> Result<PathBuf, WorktreeError> {
    let git = Git::new(dir);
    let common = git
        .text(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .map_err(|_| WorktreeError::NotARepo)?;
    let common = PathBuf::from(common);
    // `<repo>/.git` → `<repo>`; bei Bare-Repositories das Verzeichnis selbst.
    Ok(if common.file_name().is_some_and(|n| n == ".git") {
        common.parent().map(Path::to_path_buf).unwrap_or(common)
    } else {
        common
    })
}

/// Lokale und Remote-Tracking-Branches, z. B. `main`, `origin/main`.
pub fn branches(git: &Git) -> Result<Vec<String>, GitError> {
    let out = git.text(&[
        "for-each-ref",
        "--format=%(refname:short)",
        "refs/heads",
        "refs/remotes",
    ])?;
    Ok(out
        .lines()
        .map(str::to_owned)
        .filter(|b| !b.ends_with("/HEAD") && b != "HEAD")
        .collect())
}

fn remotes(git: &Git) -> Vec<String> {
    git.text(&["remote"])
        .map(|s| s.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Base laut SES-015: explizit → `origin/HEAD` → aktueller Branch (sonst `HEAD`).
pub fn resolve_base(git: &Git, explicit: Option<&str>) -> String {
    if let Some(b) = explicit.map(str::trim).filter(|b| !b.is_empty()) {
        return b.to_owned();
    }
    if let Ok(origin_head) = git.text(&[
        "symbolic-ref",
        "--quiet",
        "--short",
        "refs/remotes/origin/HEAD",
    ]) && !origin_head.is_empty()
    {
        return origin_head;
    }
    git.text(&["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "HEAD".to_owned())
}

/// `<remote>/<branch>` einer Remote-Tracking-Base, sonst `None`.
fn tracking(git: &Git, base: &str) -> Option<(String, String)> {
    remotes(git).into_iter().find_map(|r| {
        base.strip_prefix(&format!("{r}/"))
            .filter(|b| !b.is_empty())
            .map(|b| (r.clone(), b.to_owned()))
    })
}

/// Legt einen Worktree mit eigenem Branch an (SES-015).
pub fn create(opts: &CreateOptions<'_>) -> Result<Created, WorktreeError> {
    let repo = main_checkout(opts.cwd)?;
    let git = Git::new(&repo);
    let base = resolve_base(&Git::new(opts.cwd), opts.base);

    let fetch = match (opts.fetch, tracking(&git, &base)) {
        (true, Some((remote, branch))) => {
            match git.clone().with_timeout(opts.fetch_timeout).run(&[
                "fetch",
                "--quiet",
                "--no-tags",
                &remote,
                &branch,
            ]) {
                Ok(o) if o.success() => FetchOutcome::Fetched { remote, branch },
                Ok(o) => FetchOutcome::Failed {
                    remote,
                    branch,
                    reason: first_line(&o.stderr),
                },
                Err(e) => FetchOutcome::Failed {
                    remote,
                    branch,
                    reason: e.to_string(),
                },
            }
        }
        _ => FetchOutcome::Skipped,
    };

    let base_sha = git
        .text(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{base}^{{commit}}"),
        ])
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| WorktreeError::BaseNotFound {
            base: base.clone(),
            available: branches(&git).unwrap_or_default(),
        })?;

    let branch = match opts.branch.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => b.to_owned(),
        None => default_branch(opts.title, opts.tag),
    };
    if !git
        .run(&["check-ref-format", "--branch", &branch])
        .is_ok_and(|o| o.success())
    {
        return Err(WorktreeError::InvalidBranch(branch));
    }
    let exists = git
        .run(&[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .is_ok_and(|o| o.success());

    let path = opts
        .root
        .join(repo_dir_name(&repo))
        .join(slug(&branch.replace('/', "-"), 80));
    if path.exists() {
        return Err(WorktreeError::Exists(path.display().to_string()));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path_text = path.to_string_lossy().into_owned();
    if exists {
        // Vorhandener Branch (z. B. `--worktree=feature/x`): ohne neuen Branch auschecken.
        git.ok(&["worktree", "add", "--quiet", &path_text, &branch])?;
    } else {
        // Start bei der Commit-ID: kein Upstream auf die Base (ein `push` ginge sonst dorthin).
        git.ok(&[
            "worktree",
            "add",
            "--quiet",
            "--no-track",
            "-b",
            &branch,
            &path_text,
            &base_sha,
        ])?;
    }
    let path = path.canonicalize().unwrap_or(path);
    Ok(Created {
        repo,
        path,
        branch,
        base,
        base_sha,
        fetch,
    })
}

fn first_line(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("unbekannter Fehler")
        .trim()
        .to_owned()
}

/// Was mit uncommitteten Änderungen geschehen soll (Rückfrage, SES-016).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Uncommitted {
    /// WIP-Commit auf dem Branch.
    Commit,
    /// Änderungen verwerfen.
    Discard,
}

/// Was mit einem Branch mit eigenen, nicht gemergten Commits geschehen soll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchAction {
    Keep,
    Delete,
}

/// Bekannte Daten eines Session-Worktrees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRef {
    pub repo: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    pub base: String,
    pub base_sha: String,
}

/// Entscheidungen des Nutzers für [`remove`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RemoveOptions {
    pub uncommitted: Option<Uncommitted>,
    pub branch: Option<BranchAction>,
}

#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    /// Uncommittete Änderungen ohne Entscheidung: nichts wurde verändert.
    #[error("Worktree hat uncommittete Änderungen: {}", files.join(", "))]
    Dirty { files: Vec<String> },
    /// Ungepushte, nicht gemergte Commits ohne Entscheidung: nichts wurde verändert.
    #[error("Branch {branch} hat {commits} ungepushte Commits, die nicht in {base} gemergt sind")]
    Unpushed {
        branch: String,
        base: String,
        commits: u32,
    },
    #[error("{0}")]
    Git(#[from] GitError),
}

/// Ergebnis von [`remove`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Removed {
    pub wip_commit: Option<String>,
    pub branch_deleted: bool,
}

fn count(git: &Git, args: &[&str]) -> u32 {
    git.text(args)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Uncommittete Dateien eines Worktrees (`git status --porcelain`).
pub fn dirty_files(path: &Path) -> Result<Vec<String>, GitError> {
    let out = Git::new(path).ok(&["status", "--porcelain", "-z", "--untracked-files=all"])?;
    let mut files = Vec::new();
    let mut fields = split_nul(&out).into_iter();
    while let Some(entry) = fields.next() {
        if entry.len() < 4 {
            continue;
        }
        let renamed = matches!(entry.as_bytes()[0], b'R' | b'C');
        files.push(entry[3..].to_owned());
        if renamed {
            // Der alte Pfad einer Umbenennung folgt als eigenes Feld.
            fields.next();
        }
    }
    Ok(files)
}

/// Entfernt einen Worktree kontrolliert (SES-016): (1) uncommittete Änderungen → Rückfrage,
/// (2) ungepushte eigene Commits → Rückfrage, (3) `git worktree remove`, (4) Branch nur löschen,
/// wenn er in der Base enthalten ist (bzw. nach Bestätigung).
///
/// Alle Rückfragen werden geprüft, bevor irgendetwas verändert wird.
pub fn remove(wt: &WorktreeRef, opts: RemoveOptions) -> Result<Removed, RemoveError> {
    let repo = Git::new(&wt.repo);
    let present = wt.path.is_dir();
    let dirty = if present {
        dirty_files(&wt.path)?
    } else {
        Vec::new()
    };
    if !dirty.is_empty() && opts.uncommitted.is_none() {
        return Err(RemoveError::Dirty { files: dirty });
    }
    let branch_exists = repo
        .run(&[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{}", wt.branch),
        ])
        .is_ok_and(|o| o.success());
    // Die Base kann inzwischen fehlen (z. B. gelöschter Remote-Branch): dann die Base-ID.
    let base = if repo
        .run(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{}^{{commit}}", wt.base),
        ])
        .is_ok_and(|o| o.success())
    {
        wt.base.clone()
    } else {
        wt.base_sha.clone()
    };
    let (own, unpushed) = if branch_exists {
        (
            count(
                &repo,
                &["rev-list", "--count", &format!("{base}..{}", wt.branch)],
            ),
            // Eigene Commits, die auf keinem Remote liegen.
            count(
                &repo,
                &[
                    "rev-list",
                    "--count",
                    &wt.branch,
                    "--not",
                    &base,
                    "--remotes",
                ],
            ),
        )
    } else {
        (0, 0)
    };
    let will_commit = !dirty.is_empty() && opts.uncommitted == Some(Uncommitted::Commit);
    let unmerged_unpushed = (own > 0 && unpushed > 0) || will_commit;
    if unmerged_unpushed && opts.branch.is_none() {
        return Err(RemoveError::Unpushed {
            branch: wt.branch.clone(),
            base,
            commits: unpushed + u32::from(will_commit && unpushed == 0),
        });
    }

    let mut removed = Removed::default();
    if will_commit {
        let wt_git = identity(Git::new(&wt.path));
        wt_git.ok(&["add", "-A"])?;
        wt_git.ok(&[
            "commit",
            "--quiet",
            "--no-verify",
            "-m",
            "WIP: Stand beim Löschen der beton-Session",
        ])?;
        removed.wip_commit = Some(wt_git.text(&["rev-parse", "HEAD"])?);
    }
    if present {
        let path = wt.path.to_string_lossy().into_owned();
        let mut args = vec!["worktree", "remove"];
        if !dirty.is_empty() && opts.uncommitted == Some(Uncommitted::Discard) {
            args.push("--force");
        }
        args.push(&path);
        repo.ok(&args)?;
    }
    repo.ok(&["worktree", "prune"])?;
    if branch_exists {
        let delete = match opts.branch {
            Some(BranchAction::Delete) => true,
            Some(BranchAction::Keep) => false,
            // Keine eigenen Commits gegenüber der Base: nichts geht verloren.
            None => own == 0 && !will_commit,
        };
        // Ohne eigene Commits ist der Branch in der Base enthalten; `git branch -d` prüft
        // dagegen gegen den HEAD des Haupt-Checkouts und schlüge fehl, wenn dort ein anderer
        // Branch ausgecheckt ist. Die Prüfung gegen die Base ist hier die Bestätigung.
        if delete {
            repo.ok(&["branch", "-D", "--quiet", &wt.branch])?;
            removed.branch_deleted = true;
        }
    }
    Ok(removed)
}

/// Commit-Identität: die konfigurierte, sonst eine neutrale für WIP-Commits.
fn identity(git: Git) -> Git {
    let has = |key: &str| git.text(&["config", key]).is_ok_and(|v| !v.is_empty());
    if has("user.email") && has("user.name") {
        git
    } else {
        git.with_config("user.name=beton")
            .with_config("user.email=beton@localhost")
    }
}

/// `git worktree prune` im Repository.
pub fn prune(repo: &Path) -> Result<(), GitError> {
    Git::new(repo).ok(&["worktree", "prune"]).map(|_| ())
}

/// Ein Worktree-Verzeichnis ohne Session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    pub path: PathBuf,
    /// Größe in Bytes (Dateien, ohne Symlinks zu folgen).
    pub size: u64,
}

/// Größe eines Verzeichnisbaums in Bytes; folgt keinen Symlinks.
pub fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| dir_size(&e.path()))
                .sum()
        })
        .unwrap_or(0)
}

/// Worktree-Verzeichnisse unter `root` (`<repo>/<branch>`), die zu keiner bekannten Session
/// gehören.
pub fn orphans(root: &Path, known: &[PathBuf]) -> std::io::Result<Vec<Orphan>> {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let known: HashSet<PathBuf> = known.iter().map(|p| canon(p)).collect();
    let mut out = Vec::new();
    let Ok(repos) = std::fs::read_dir(root) else {
        return Ok(out);
    };
    for repo in repos.filter_map(Result::ok) {
        if !repo.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        for wt in std::fs::read_dir(repo.path())?.filter_map(Result::ok) {
            if !wt.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let path = canon(&wt.path());
            if !known.contains(&path) {
                out.push(Orphan {
                    size: dir_size(&path),
                    path,
                });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_follow_the_convention() {
        assert_eq!(
            default_branch(Some("Login-Fehler beheben!"), "AB12"),
            "beton/login-fehler-beheben-ab12"
        );
        assert_eq!(default_branch(None, "x9z0"), "beton/session-x9z0");
        assert_eq!(default_branch(Some("  ☃  "), "0000"), "beton/session-0000");
        assert_eq!(slug("a//b", 10), "a-b");
    }

    #[test]
    fn repo_dir_name_has_slug_and_hash() {
        let name = repo_dir_name(Path::new("/home/kim/Mein Projekt"));
        assert!(name.starts_with("mein-projekt-"), "{name}");
        assert_eq!(name.len(), "mein-projekt-".len() + 8);
        assert_ne!(name, repo_dir_name(Path::new("/other/Mein Projekt")));
    }
}
