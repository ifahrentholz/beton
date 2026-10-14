//! Der Startplan zwischen Runner und Launcher und die daraus abgeleiteten Mounts und
//! Landlock-Regeln. Alles hier ist plattformunabhängig und ohne Seiteneffekte (außer Lesen des
//! Dateisystems), damit es auf jeder Plattform getestet werden kann.
//!
//! Der Plan geht als JSON über eine Pipe an den Launcher, nie über argv oder Environment: Er
//! enthält das Environment des Ziels (Platzhalter, Proxy-URL), das in `/proc/<pid>/cmdline`
//! sichtbar wäre (SBX-005 AC4, SBX-009 AC3).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::command::CommandSpec;
use crate::error::SandboxError;
use crate::resolve::{GitProtection, HiddenDotfiles, Mask, NetMode, ResolvedPolicy, Stage};
use crate::spec::{Limits, env_denied};

/// System-Pfade, die jede Sandbox lesen und ausführen darf.
pub const SYSTEM_READ: [&str; 9] = [
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    "/etc",
    "/sys/devices/system/cpu",
];

/// Geräte, die jede Sandbox lesen und schreiben darf.
pub const DEVICES_RW: [&str; 7] = [
    "/dev/null",
    "/dev/zero",
    "/dev/full",
    "/dev/random",
    "/dev/urandom",
    "/dev/tty",
    "/dev/ptmx",
];

/// Verzeichnisse, die in der Sandbox privat und leer (tmpfs) sind: Host-Sockets in `/run` (z. B.
/// Docker, D-Bus, `$XDG_RUNTIME_DIR`) und gemeinsamer Speicher sind nicht erreichbar.
pub const PRIVATE_TMPFS: [&str; 3] = ["/tmp", "/run", "/dev/shm"];

/// Höchstzahl der Einträge, die die Suche nach Muster-Masken prüft (fail closed darüber).
pub const MAX_MASK_WALK: usize = 1_000_000;

/// Wie der Launcher die Namespaces und Mounts herstellt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchMode {
    /// Eigene unprivilegierte Namespaces (SBX-008).
    Landlock,
    /// Namespaces und Mounts über bubblewrap, Landlock und seccomp im Launcher (SBX-009).
    Bwrap { bwrap: PathBuf },
}

/// Alles, was der Launcher für einen Start braucht.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchPlan {
    pub version: u32,
    pub mode: LaunchMode,
    pub stage: Stage,
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Vollständiges Environment des Ziels.
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub workspace: PathBuf,
    pub scratch: PathBuf,
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
    pub masks: Vec<Mask>,
    pub hidden: Vec<HiddenDotfiles>,
    pub git: Option<GitProtection>,
    pub net: NetMode,
    pub limits: Limits,
    /// Höchste genutzte Landlock-ABI (Tests).
    pub abi_cap: Option<u8>,
    /// cgroup-v2-Wurzel mit Delegation, unter der der Launcher eine cgroup anlegt.
    pub cgroup: Option<PathBuf>,
    /// Leere Datei ohne Rechte (0000) als Quelle für Datei-Masken unter bwrap.
    pub mask_file: Option<PathBuf>,
    /// Der Launcher läuft schon innerhalb von bwrap (Namespaces und Mounts stehen).
    #[serde(default)]
    pub inner: bool,
}

impl LaunchPlan {
    /// Plan für `cmd` unter `policy`. Das Environment ist das der Policy plus die erlaubten
    /// Variablen des Aufrufs (gesperrte Namen wie `LD_PRELOAD` fallen weg, SBX-005).
    pub fn new(policy: &ResolvedPolicy, cmd: &CommandSpec, mode: LaunchMode) -> Self {
        let mut env = policy.env.clone();
        for (k, v) in &cmd.env.set {
            if !env_denied(k) {
                env.insert(k.clone(), v.clone());
            }
        }
        for k in &cmd.env.remove {
            env.remove(k);
        }
        Self {
            version: super::PLAN_VERSION,
            mode,
            stage: policy.stage,
            program: cmd.program.clone(),
            args: cmd.args.clone(),
            env: env.into_iter().collect(),
            cwd: cmd.cwd.clone().unwrap_or_else(|| policy.workspace.clone()),
            workspace: policy.workspace.clone(),
            scratch: policy.scratch.clone(),
            read_roots: policy.read_roots.clone(),
            write_roots: policy.write_roots.clone(),
            masks: policy.masks.clone(),
            hidden: policy.hidden.clone(),
            git: policy.git.clone(),
            net: policy.net,
            limits: policy.limits,
            abi_cap: None,
            cgroup: None,
            mask_file: None,
            inner: false,
        }
    }

    /// Alle Wurzeln (Lesen und Schreiben).
    pub fn roots(&self) -> impl Iterator<Item = &PathBuf> {
        self.read_roots.iter().chain(&self.write_roots)
    }

    /// Liegt `p` in einem sichtbaren Bereich (Freigaben oder System-Pfade)? Unter den privaten
    /// tmpfs (`/tmp`, `/run`) ist nur sichtbar, was in einer Freigabe liegt.
    pub fn visible(&self, p: &Path) -> bool {
        self.roots().any(|r| p.starts_with(r)) || SYSTEM_READ.iter().any(|s| p.starts_with(s))
    }
}

/// Was der Launcher maskieren und neu einbinden muss.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaskTargets {
    /// Verzeichnisse: leeres, schreibgeschütztes tmpfs ohne Rechte (0000).
    pub dirs: Vec<PathBuf>,
    /// Dateien und andere Nicht-Verzeichnisse: eine leere Datei ohne Rechte darüber.
    pub files: Vec<PathBuf>,
    /// Masken, die es noch nicht gibt, in einer Schreib-Wurzel: Sie dürfen nicht entstehen
    /// (Landlock-Grant-Expansion, siehe [`landlock_rules`]).
    pub missing_in_write: Vec<PathBuf>,
    /// Wurzeln unter einem privaten tmpfs oder einer Maske, die danach wieder eingebunden
    /// werden (in der Reihenfolge der Tiefe).
    pub rebind: Vec<PathBuf>,
}

fn glob_match(glob: &str, name: &str) -> bool {
    fn rec(g: &[char], n: &[char]) -> bool {
        match (g.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => rec(&g[1..], n) || (!n.is_empty() && rec(g, &n[1..])),
            (Some('?'), Some(_)) => rec(&g[1..], &n[1..]),
            (Some(a), Some(b)) if a == b => rec(&g[1..], &n[1..]),
            _ => false,
        }
    }
    let g: Vec<char> = glob.chars().collect();
    let n: Vec<char> = name.chars().collect();
    rec(&g, &n)
}

#[derive(Default)]
struct Collector {
    dirs: BTreeSet<PathBuf>,
    files: BTreeSet<PathBuf>,
    missing: BTreeSet<PathBuf>,
}

impl Collector {
    /// Nimmt `p` als Maske auf. Symlinks werden aufgelöst: Ein Mount auf dem Link landet auf
    /// dem Ziel; liegt das Ziel außerhalb der Freigaben, sperrt Landlock es ohnehin.
    fn add(&mut self, plan: &LaunchPlan, p: &Path) {
        let Ok(meta) = std::fs::symlink_metadata(p) else {
            if plan.write_roots.iter().any(|w| p.starts_with(w)) {
                self.missing.insert(p.to_path_buf());
            }
            return;
        };
        let (target, meta) = if meta.file_type().is_symlink() {
            match (std::fs::canonicalize(p), std::fs::metadata(p)) {
                (Ok(t), Ok(m)) => (t, m),
                _ => return,
            }
        } else {
            (p.to_path_buf(), meta)
        };
        if !plan.visible(&target) {
            return;
        }
        if meta.is_dir() {
            self.dirs.insert(target);
        } else {
            self.files.insert(target);
        }
    }
}

/// Durchsucht `root` nach Einträgen, deren Name `glob` entspricht (ohne Symlinks zu folgen).
fn walk(
    plan: &LaunchPlan,
    root: &Path,
    glob: &str,
    any_depth: bool,
    out: &mut Collector,
    budget: &mut usize,
) -> Result<(), SandboxError> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Ok(());
    };
    for e in entries.flatten() {
        if *budget == 0 {
            return Err(SandboxError::invalid(
                "masks",
                format!(
                    "mehr als {MAX_MASK_WALK} Einträge beim Suchen der Muster-Masken unter {}",
                    root.display()
                ),
            ));
        }
        *budget -= 1;
        let path = e.path();
        let name = e.file_name();
        if glob_match(glob, &name.to_string_lossy()) {
            out.add(plan, &path);
            continue;
        }
        if any_depth
            && let Ok(ft) = e.file_type()
            && ft.is_dir()
            && !ft.is_symlink()
        {
            walk(plan, &path, glob, any_depth, out, budget)?;
        }
    }
    Ok(())
}

/// Masken, die beim Start existieren, sowie die danach wieder einzubindenden Wurzeln.
pub fn mask_targets(plan: &LaunchPlan) -> Result<MaskTargets, SandboxError> {
    let mut c = Collector::default();
    let mut budget = MAX_MASK_WALK;
    for m in &plan.masks {
        match m {
            Mask::Path { path, .. } => c.add(plan, path),
            Mask::Pattern {
                root,
                glob,
                any_depth,
                ..
            } => {
                if plan.visible(root) {
                    walk(plan, root, glob, *any_depth, &mut c, &mut budget)?;
                }
            }
        }
    }
    for h in &plan.hidden {
        let Ok(entries) = std::fs::read_dir(&h.root) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !h.allow.contains(&name) {
                c.add(plan, &e.path());
            }
        }
    }
    // Eine ausdrücklich freigegebene Wurzel (Workspace, Scratch, Pfade des Harness in Stufe 1)
    // gewinnt gegen Dotfile- und Muster-Masken, wie unter macOS; Pfad-Masken über Wurzeln
    // lehnt schon die Auflösung ab.
    let roots: Vec<&PathBuf> = plan.roots().collect();
    c.dirs.retain(|d| !roots.contains(&d));
    c.files.retain(|f| !roots.contains(&f));
    let dirs: Vec<PathBuf> = c.dirs.iter().cloned().collect();
    let mut shadows: Vec<PathBuf> = PRIVATE_TMPFS.iter().map(PathBuf::from).collect();
    shadows.extend(dirs.iter().cloned());
    let mut rebind: Vec<PathBuf> = roots
        .iter()
        .filter(|r| shadows.iter().any(|s| r.starts_with(s) && **r != s))
        .map(|r| (*r).clone())
        .collect();
    rebind.sort();
    rebind.dedup();
    // Sichtbar bleibt ein Pfad nur, wenn jeder ihn verdeckende Mount von einer tieferen
    // Wiedereinbindung aufgehoben wird (eine Maske in `~/.config` verschwindet mit der Maske von
    // `~/.config`, auch wenn `~` wieder eingebunden wird).
    let reachable = |p: &Path| {
        shadows
            .iter()
            .filter(|s| p != *s && p.starts_with(s))
            .all(|s| {
                rebind
                    .iter()
                    .any(|r| p.starts_with(r) && r.starts_with(s) && r != s)
            })
    };
    Ok(MaskTargets {
        dirs: dirs.iter().filter(|d| reachable(d)).cloned().collect(),
        files: c.files.iter().filter(|f| reachable(f)).cloned().collect(),
        missing_in_write: c.missing.into_iter().collect(),
        rebind,
    })
}

/// Ein Mount-Schritt im Mount-Namespace der Sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountOp {
    /// Privates, leeres tmpfs (`/tmp`, `/run`, `/dev/shm`) mit diesem Modus.
    Tmpfs { path: PathBuf, mode: u32 },
    /// Maskiertes Verzeichnis: leeres, schreibgeschütztes tmpfs mit Modus 0000.
    MaskDir { path: PathBuf },
    /// Zwischenebene unter einem privaten tmpfs für Wiedereinbindungen (z. B. `/tmp/x` für die
    /// Wurzel `/tmp/x/ws`): eigenes tmpfs, am Ende read-only. Sonst ließen sich neben den
    /// wieder eingebundenen Wurzeln Dateien anlegen, die wie Teile des Hosts aussehen.
    Anchor { path: PathBuf },
    /// Maskierte Datei: leere Datei mit Modus 0000, schreibgeschützt darübergebunden.
    MaskFile { path: PathBuf },
    /// Eine Wurzel, die unter einem tmpfs oder einer Maske liegt, aus der ursprünglichen Sicht
    /// wieder einbinden.
    Rebind { path: PathBuf, write: bool },
    /// Auf sich selbst schreibgeschützt binden (Git-Hooks und -Konfiguration).
    ReadOnly { path: PathBuf },
    /// Auf sich selbst binden; ein Mountpunkt lässt sich weder umbenennen noch ersetzen.
    /// `writable`: danach beschreibbar (unter einem read-only gebundenen Vorfahren), sonst mit
    /// dem Zustand des Elternteils.
    Pin { path: PathBuf, writable: bool },
}

impl MountOp {
    pub fn path(&self) -> &Path {
        match self {
            MountOp::Tmpfs { path, .. }
            | MountOp::MaskDir { path }
            | MountOp::Anchor { path }
            | MountOp::MaskFile { path }
            | MountOp::Rebind { path, .. }
            | MountOp::ReadOnly { path }
            | MountOp::Pin { path, .. } => path,
        }
    }

    fn order(&self) -> u8 {
        match self {
            MountOp::Tmpfs { .. } => 0,
            MountOp::MaskDir { .. } | MountOp::MaskFile { .. } | MountOp::Anchor { .. } => 1,
            MountOp::Rebind { .. } => 2,
            MountOp::Pin { .. } => 3,
            MountOp::ReadOnly { .. } => 4,
        }
    }
}

/// Alle Mount-Schritte in der Reihenfolge, in der sie gelten müssen: nach Tiefe des Pfads, bei
/// gleicher Tiefe tmpfs vor Masken vor Wiedereinbindungen. Jeder Schritt wirkt nur auf seinen
/// Teilbaum; so liegt eine Maske im Workspace auf dessen Wiedereinbindung und nicht darunter.
pub fn mount_ops(plan: &LaunchPlan) -> Result<(Vec<MountOp>, MaskTargets), SandboxError> {
    let targets = mask_targets(plan)?;
    let mut ops = vec![
        MountOp::Tmpfs {
            path: "/tmp".into(),
            mode: 0o1777,
        },
        MountOp::Tmpfs {
            path: "/run".into(),
            mode: 0o755,
        },
        MountOp::Tmpfs {
            path: "/dev/shm".into(),
            mode: 0o1777,
        },
    ];
    ops.extend(
        targets
            .dirs
            .iter()
            .map(|p| MountOp::MaskDir { path: p.clone() }),
    );
    ops.extend(
        targets
            .files
            .iter()
            .map(|p| MountOp::MaskFile { path: p.clone() }),
    );
    let mut anchors = BTreeSet::new();
    for r in &targets.rebind {
        for s in ["/tmp", "/run"] {
            let Ok(rel) = r.strip_prefix(s) else { continue };
            let Some(first) = rel.components().next() else {
                continue;
            };
            let anchor = Path::new(s).join(first);
            if anchor != *r && !targets.rebind.contains(&anchor) {
                anchors.insert(anchor);
            }
        }
    }
    ops.extend(anchors.into_iter().map(|path| MountOp::Anchor { path }));
    for r in &targets.rebind {
        ops.push(MountOp::Rebind {
            path: r.clone(),
            write: plan.write_roots.contains(r),
        });
    }
    // Eine Maske, die es noch nicht gibt, darf in einer Schreib-Wurzel nicht entstehen: Ihr
    // tiefster vorhandener Vorfahre wird read-only, seine Einträge einzeln beschreibbar
    // (Mount-Gegenstück zur Landlock-Grant-Expansion, wirkt auch unter dem privaten `/tmp`).
    let masked: Vec<&PathBuf> = targets.dirs.iter().chain(&targets.files).collect();
    let mut frozen = BTreeSet::new();
    for m in &targets.missing_in_write {
        let Some(w) = plan.write_roots.iter().find(|w| m.starts_with(w)) else {
            continue;
        };
        let Some(e) = m
            .ancestors()
            .skip(1)
            .find(|a| a.starts_with(w) && a.is_dir())
        else {
            continue;
        };
        if !frozen.insert(e.to_path_buf()) {
            continue;
        }
        ops.push(MountOp::ReadOnly {
            path: e.to_path_buf(),
        });
        for c in std::fs::read_dir(e).into_iter().flatten().flatten() {
            let c = c.path();
            if !masked.contains(&&c) {
                ops.push(MountOp::Pin {
                    path: c,
                    writable: true,
                });
            }
        }
    }
    let git = git_mounts(plan);
    ops.extend(git.pinned.into_iter().map(|path| MountOp::Pin {
        path,
        writable: false,
    }));
    ops.extend(
        git.read_only
            .into_iter()
            .map(|path| MountOp::ReadOnly { path }),
    );
    sort_ops(&mut ops);
    Ok((ops, targets))
}

/// Sortiert Mount-Schritte nach Tiefe, dann Art, dann Pfad (siehe [`mount_ops`]).
pub fn sort_ops(ops: &mut Vec<MountOp>) {
    ops.sort_by(|a, b| {
        let depth = |o: &MountOp| o.path().components().count();
        depth(a)
            .cmp(&depth(b))
            .then(a.order().cmp(&b.order()))
            .then(a.path().cmp(b.path()))
    });
    ops.dedup();
}

/// Mounts zum Schutz von Git (SBX-003 AC4): read-only gebundene Hooks und Konfiguration,
/// `.git` als eigener Mount (nicht umbenennbar oder ersetzbar).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitMounts {
    /// Read-only auf sich selbst binden.
    pub read_only: Vec<PathBuf>,
    /// Beschreibbar auf sich selbst binden (Mountpunkte lassen sich nicht umbenennen).
    pub pinned: Vec<PathBuf>,
}

/// Hooks-Verzeichnisse, die vor dem Start existieren müssen (sonst könnte die Sandbox sie
/// anlegen). Legt sie an; `git init` tut dasselbe.
pub fn ensure_git_hooks(git: &GitProtection) -> std::io::Result<()> {
    let mut dirs = vec![git.common_dir.clone()];
    if git.git_dir != git.common_dir {
        dirs.push(git.git_dir.clone());
    }
    for d in dirs {
        let hooks = d.join("hooks");
        if !hooks.exists() && d.is_dir() {
            std::fs::create_dir(&hooks)?;
        }
    }
    Ok(())
}

fn modules(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let name = e.file_name();
        if name == "hooks" || name == "config" {
            out.push(p);
        } else if ft.is_dir() {
            modules(&p, out, depth + 1);
        }
    }
}

/// Die Git-Mounts eines Plans.
pub fn git_mounts(plan: &LaunchPlan) -> GitMounts {
    let mut m = GitMounts::default();
    let Some(g) = &plan.git else {
        return m;
    };
    let mut dirs = vec![g.common_dir.clone()];
    if g.git_dir != g.common_dir {
        dirs.push(g.git_dir.clone());
    }
    let mut ro = Vec::new();
    for d in &dirs {
        for name in ["hooks", "config", "config.worktree", "commondir", "gitdir"] {
            ro.push(d.join(name));
        }
        modules(&d.join("modules"), &mut ro, 0);
        if let Ok(entries) = std::fs::read_dir(d.join("worktrees")) {
            for e in entries.flatten() {
                ro.push(e.path().join("config.worktree"));
            }
        }
    }
    if g.dot_git_is_file {
        ro.push(g.dot_git.clone());
    } else {
        m.pinned.push(g.dot_git.clone());
    }
    m.read_only = ro
        .into_iter()
        .filter(|p| std::fs::symlink_metadata(p).is_ok_and(|m| !m.file_type().is_symlink()))
        .collect();
    m
}

/// Rechte einer Landlock-Regel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    /// Verzeichnis lesen/auflisten, ohne Dateien zu lesen (z. B. `/dev`).
    ListDir,
    /// Lesen und Ausführen.
    ReadExec,
    /// Gerätedateien lesen und schreiben.
    Device,
    /// Alles (Lesen, Schreiben, Anlegen, Löschen, Umbenennen, Ausführen).
    Write,
}

/// Die Landlock-Regeln eines Plans (nach den Mounts im Mount-Namespace aufzulösen).
///
/// Eine Schreib-Wurzel mit einer noch nicht vorhandenen Maske (z. B. `~/.docker/config.json`
/// bei `write_paths: [~/.docker]`) wird expandiert: Die Wurzel selbst ist nur lesbar, ihre
/// vorhandenen Einträge sind einzeln beschreibbar, und auf dem Weg zur Maske wiederholt sich das.
/// So kann die Maske nicht neu entstehen; neue Einträge direkt in diesen Verzeichnissen sind
/// dann ebenfalls gesperrt (fail closed).
pub fn landlock_rules(plan: &LaunchPlan, missing_in_write: &[PathBuf]) -> Vec<(PathBuf, Access)> {
    let mut rules: Vec<(PathBuf, Access)> = Vec::new();
    for s in SYSTEM_READ {
        rules.push((PathBuf::from(s), Access::ReadExec));
    }
    rules.push(("/proc".into(), Access::ReadExec));
    rules.push(("/dev".into(), Access::ListDir));
    for d in DEVICES_RW {
        rules.push((PathBuf::from(d), Access::Device));
    }
    rules.push(("/dev/pts".into(), Access::Device));
    rules.push(("/dev/shm".into(), Access::Write));
    rules.push(("/tmp".into(), Access::Write));
    for r in &plan.read_roots {
        rules.push((r.clone(), Access::ReadExec));
    }
    for w in &plan.write_roots {
        expand_write(w, missing_in_write, &mut rules);
    }
    rules
}

fn expand_write(w: &Path, missing: &[PathBuf], rules: &mut Vec<(PathBuf, Access)>) {
    let relevant: Vec<&PathBuf> = missing
        .iter()
        .filter(|m| m.starts_with(w) && *m != w)
        .collect();
    if relevant.is_empty() {
        rules.push((w.to_path_buf(), Access::Write));
        return;
    }
    rules.push((w.to_path_buf(), Access::ReadExec));
    let Ok(entries) = std::fs::read_dir(w) else {
        return;
    };
    for e in entries.flatten() {
        let child = e.path();
        let on_path = relevant.iter().any(|m| m.starts_with(&child));
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        if on_path && is_dir {
            let missing: Vec<PathBuf> = relevant.iter().map(|m| (*m).clone()).collect();
            expand_write(&child, &missing, rules);
        } else if !on_path {
            rules.push((child, Access::Write));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::resolve::{ResolveCtx, resolve};
    use crate::spec::SandboxSpec;
    use crate::{CommandSpec, ProviderId};

    fn write(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x").unwrap();
    }

    fn plan_for(root: &Path, yaml: &str) -> LaunchPlan {
        let home = root.join("home");
        let ws = root.join("ws");
        let scratch = root.join("scratch");
        write(&home.join(".ssh/id_ed25519"));
        write(&home.join(".config/gh/hosts.yml"));
        write(&home.join(".netrc"));
        write(&home.join("Documents/a.txt"));
        write(&ws.join(".env"));
        write(&ws.join("a/.env.local"));
        write(&ws.join("src/main.rs"));
        std::fs::create_dir_all(&scratch).unwrap();
        let mut ctx = ResolveCtx::new(Stage::Tools, &ws, &home, &scratch);
        ctx.adapters = crate::masks::builtin_harness_grants();
        let spec = SandboxSpec::from_yaml(yaml).unwrap();
        let policy = resolve(ProviderId::Landlock, &spec, &ctx).unwrap();
        LaunchPlan::new(
            &policy,
            &CommandSpec::new("/bin/true"),
            LaunchMode::Landlock,
        )
    }

    #[test]
    fn sbx_004_mask_targets_cover_dotfiles_patterns_and_skip_nested_masks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let plan = plan_for(&root, "read_paths: [\"~\"]\n");
        let t = mask_targets(&plan).unwrap();
        let home = root.join("home");
        assert!(t.dirs.contains(&home.join(".ssh")), "{t:?}");
        assert!(t.dirs.contains(&home.join(".config")), "{t:?}");
        // Unter einer Maske nichts weiter maskieren (der Pfad existiert dort nicht mehr).
        assert!(!t.dirs.contains(&home.join(".config/gh")), "{t:?}");
        assert!(t.files.contains(&home.join(".netrc")), "{t:?}");
        assert!(t.files.contains(&root.join("ws/.env")), "{t:?}");
        assert!(t.files.contains(&root.join("ws/a/.env.local")), "{t:?}");
        assert!(!t.files.iter().any(|f| f.ends_with("main.rs")), "{t:?}");
        // Wurzeln unter `/tmp` (Fixtures unter Linux) kommen wieder hinein.
        for r in plan.roots() {
            assert_eq!(
                t.rebind.contains(r),
                r.starts_with("/tmp") || r.starts_with("/run"),
                "{r:?}: {t:?}"
            );
        }
    }

    #[test]
    fn sbx_004_mount_ops_apply_parents_before_children() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let plan = plan_for(&root, "read_paths: [\"~\"]\n");
        let (ops, _) = mount_ops(&plan).unwrap();
        for (i, op) in ops.iter().enumerate() {
            for later in &ops[i + 1..] {
                let shadows = matches!(later, MountOp::Tmpfs { .. } | MountOp::MaskDir { .. });
                assert!(
                    !(shadows && op.path().starts_with(later.path()) && op.path() != later.path()),
                    "{op:?} vor {later:?}"
                );
            }
        }
        assert!(matches!(ops[0], MountOp::Tmpfs { .. }));
    }

    #[test]
    fn sbx_004_missing_mask_in_write_root_expands_the_grant() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("home/.docker/buildx")).unwrap();
        write(&root.join("home/.docker/daemon.json"));
        let plan = plan_for(
            &root,
            "read_paths: [\"~\"]\nwrite_paths: [\"~/.docker\"]\nallow_hidden: [.docker]\n",
        );
        let t = mask_targets(&plan).unwrap();
        let docker = root.join("home/.docker");
        assert!(
            t.missing_in_write.contains(&docker.join("config.json")),
            "{t:?}"
        );
        let rules = landlock_rules(&plan, &t.missing_in_write);
        // Die Wurzel selbst ist nur lesbar, ihre Einträge einzeln beschreibbar.
        assert!(
            rules.contains(&(docker.clone(), Access::ReadExec)),
            "{rules:?}"
        );
        assert!(
            !rules.contains(&(docker.clone(), Access::Write)),
            "{rules:?}"
        );
        assert!(
            rules.contains(&(docker.join("buildx"), Access::Write)),
            "{rules:?}"
        );
        assert!(
            rules.contains(&(docker.join("daemon.json"), Access::Write)),
            "{rules:?}"
        );
        // Ohne fehlende Maske bleibt der Workspace eine ganze Schreib-Wurzel.
        assert!(
            rules.contains(&(root.join("ws"), Access::Write)),
            "{rules:?}"
        );
        // Dasselbe als Mounts: `.docker` read-only, Einträge einzeln beschreibbar.
        let (ops, _) = mount_ops(&plan).unwrap();
        assert!(
            ops.contains(&MountOp::ReadOnly {
                path: docker.clone()
            }),
            "{ops:?}"
        );
        assert!(
            ops.contains(&MountOp::Pin {
                path: docker.join("buildx"),
                writable: true
            }),
            "{ops:?}"
        );
        assert!(
            ops.contains(&MountOp::Pin {
                path: docker.join("daemon.json"),
                writable: true
            }),
            "{ops:?}"
        );
    }

    #[test]
    fn sbx_003_git_mounts_pin_dot_git_and_protect_hooks_and_config() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let mut plan = plan_for(&root, "{}\n");
        let git = root.join("ws/.git");
        std::fs::create_dir_all(git.join("modules/sub/hooks")).unwrap();
        write(&git.join("config"));
        write(&git.join("modules/sub/config"));
        plan.git = Some(GitProtection {
            dot_git: git.clone(),
            dot_git_is_file: false,
            git_dir: git.clone(),
            common_dir: git.clone(),
        });
        ensure_git_hooks(plan.git.as_ref().unwrap()).unwrap();
        assert!(git.join("hooks").is_dir());
        let m = git_mounts(&plan);
        assert_eq!(m.pinned, vec![git.clone()]);
        for p in ["hooks", "config", "modules/sub/hooks", "modules/sub/config"] {
            assert!(m.read_only.contains(&git.join(p)), "{p}: {m:?}");
        }
        // Nicht vorhandene Dateien werden nicht gebunden.
        assert!(!m.read_only.contains(&git.join("commondir")), "{m:?}");
    }

    #[test]
    fn globs_match_one_segment() {
        assert!(glob_match(".env", ".env"));
        assert!(glob_match(".env.*", ".env.local"));
        assert!(!glob_match(".env.*", ".env"));
        assert!(glob_match("credentials*", "credentials.toml"));
        assert!(glob_match("?b", "ab"));
        assert!(!glob_match("?b", "abb"));
    }
}
