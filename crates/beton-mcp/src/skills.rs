//! Skills im `SKILL.md`-Format (AGT-008).
//!
//! Ein Skill ist ein Ordner mit `SKILL.md` (YAML-Frontmatter `name`, `description`, optional
//! `user-invocable`, `disable-model-invocation`, `allowed-tools`) und optionalen `scripts/`,
//! `references/`, `assets/`. Discovery-Reihenfolge, der erste Treffer pro Name gewinnt:
//! Agent-Verzeichnis `skills/` → Projekt `.beton/skills/`, `.claude/skills/`, `.agents/skills/`
//! → User `~/.beton/skills/`, `~/.claude/skills/`, `~/.agents/skills/` → Built-in-Skills.
//!
//! Auslieferung: Harnesses mit nativen Skills bekommen ein session-spezifisches
//! Plugin-Verzeichnis ([`materialize`]); alle Harnesses können Inhalte über die System-Tools
//! `skill_load`/`skill_read_file` laden. `skill_read_file` liefert nur Dateien innerhalb des
//! Skill-Ordners, auch nicht über Symlinks nach außen.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

/// Datei, die einen Skill-Ordner ausmacht.
pub const SKILL_MD: &str = "SKILL.md";
/// Name des Plugins, unter dem Claude Code die Session-Skills lädt (`beton:<skill>`).
pub const PLUGIN_NAME: &str = "beton";
/// Höchstgröße einer Datei für `skill_read_file` (Kontext des Modells schonen).
pub const MAX_READ_BYTES: u64 = 256 * 1024;

/// Ebene, aus der ein Skill stammt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Origin {
    Agent,
    Project,
    User,
    Builtin,
}

/// Vendor-Verzeichnis, das eine CLI selbst liest (dann liefert beton den Skill dort nicht
/// noch einmal nativ aus).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VendorDir {
    /// `.claude/skills` – liest Claude Code selbst.
    Claude,
    /// `.agents/skills` – liest Codex selbst.
    Agents,
}

/// Ein Discovery-Verzeichnis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRoot {
    pub dir: PathBuf,
    pub origin: Origin,
    pub vendor: Option<VendorDir>,
}

/// Ein gefundener Skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Ordner des Skills (wie gefunden, nicht kanonisiert).
    pub dir: PathBuf,
    pub origin: Origin,
    pub vendor: Option<VendorDir>,
    /// Im Composer über `/name` aufrufbar (Default `true`).
    pub user_invocable: bool,
    /// Das Modell darf den Skill nicht selbst laden (nur der Mensch über `/name`).
    pub disable_model_invocation: bool,
    pub allowed_tools: Vec<String>,
}

/// Ergebnis der Discovery: Skills (erster Treffer pro Name) und übersprungene Einträge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    pub skills: Vec<Skill>,
    pub warnings: Vec<String>,
}

/// Discovery-Verzeichnisse in Reihenfolge (AGT-008). `agent_skills` ist `<agent>/skills`.
pub fn roots(
    agent_skills: Option<&Path>,
    project: &Path,
    beton_home: &Path,
    home: Option<&Path>,
) -> Vec<SkillRoot> {
    let root = |dir: PathBuf, origin, vendor| SkillRoot {
        dir,
        origin,
        vendor,
    };
    let mut out = Vec::new();
    if let Some(a) = agent_skills {
        out.push(root(a.to_owned(), Origin::Agent, None));
    }
    out.push(root(project.join(".beton/skills"), Origin::Project, None));
    out.push(root(
        project.join(".claude/skills"),
        Origin::Project,
        Some(VendorDir::Claude),
    ));
    out.push(root(
        project.join(".agents/skills"),
        Origin::Project,
        Some(VendorDir::Agents),
    ));
    out.push(root(beton_home.join("skills"), Origin::User, None));
    if let Some(home) = home {
        out.push(root(
            home.join(".claude/skills"),
            Origin::User,
            Some(VendorDir::Claude),
        ));
        out.push(root(
            home.join(".agents/skills"),
            Origin::User,
            Some(VendorDir::Agents),
        ));
    }
    // Built-in-Skills: noch keine (folgen mit den Built-in-Agents, AGT-011/AGT-012).
    out
}

/// Gültige Skill-Namen: `[a-z0-9-]`, 1–64 Zeichen, nicht mit `-` beginnend. Der Name wird
/// Verzeichnisname im Session-Skill-Verzeichnis und darf daher keine Pfadteile enthalten.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[derive(Debug, Default, Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(rename = "user-invocable")]
    user_invocable: Option<bool>,
    #[serde(rename = "disable-model-invocation")]
    disable_model_invocation: Option<bool>,
    #[serde(rename = "allowed-tools")]
    allowed_tools: Option<serde_json::Value>,
}

/// Zerlegt `SKILL.md` in Frontmatter-Text und Body. Ohne Frontmatter ist sie leer.
pub fn split_frontmatter(text: &str) -> (&str, &str) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return ("", text);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            let front = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return (front, body);
        }
        offset += line.len();
    }
    ("", text)
}

/// Body ohne Frontmatter (für `skill_load`, AGT-008 AC2).
pub fn body(text: &str) -> &str {
    let (_, body) = split_frontmatter(text);
    body.strip_prefix("\r\n")
        .or_else(|| body.strip_prefix('\n'))
        .unwrap_or(body)
}

fn parse_skill(dir: &Path, root: &SkillRoot) -> Result<Skill, String> {
    let file = dir.join(SKILL_MD);
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let (front, _) = split_frontmatter(&text);
    let fm: Frontmatter = if front.trim().is_empty() {
        Frontmatter::default()
    } else {
        serde_yaml_ng::from_str(front)
            .map_err(|e| format!("{}: Frontmatter ungültig: {e}", file.display()))?
    };
    let name = fm
        .name
        .filter(|n| !n.trim().is_empty())
        .ok_or_else(|| format!("{}: `name` fehlt im Frontmatter", file.display()))?;
    let description = fm
        .description
        .filter(|d| !d.trim().is_empty())
        .ok_or_else(|| format!("{}: `description` fehlt im Frontmatter", file.display()))?;
    if !valid_name(&name) {
        return Err(format!(
            "{}: ungültiger Skill-Name „{name}“ (erlaubt: a-z, 0-9, -)",
            file.display()
        ));
    }
    let allowed_tools = match fm.allowed_tools {
        Some(serde_json::Value::String(s)) => s
            .split([',', ' '])
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect(),
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    };
    Ok(Skill {
        name,
        description: description.trim().to_owned(),
        dir: dir.to_owned(),
        origin: root.origin,
        vendor: root.vendor,
        user_invocable: fm.user_invocable.unwrap_or(true),
        disable_model_invocation: fm.disable_model_invocation.unwrap_or(false),
        allowed_tools,
    })
}

/// Sucht Skills in den Verzeichnissen; der erste Treffer pro Name gewinnt (AGT-008 AC1).
/// Eine `SKILL.md` ohne `name`/`description` wird mit Warnung übersprungen (AC4).
pub fn discover(roots: &[SkillRoot]) -> Discovery {
    let mut out = Discovery::default();
    let mut seen = BTreeSet::new();
    for root in roots {
        let Ok(read) = std::fs::read_dir(&root.dir) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = read
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.join(SKILL_MD).is_file())
            .collect();
        dirs.sort();
        for dir in dirs {
            match parse_skill(&dir, root) {
                Ok(skill) => {
                    if seen.insert(skill.name.clone()) {
                        out.skills.push(skill);
                    }
                }
                Err(warning) => {
                    tracing::warn!("Skill übersprungen: {warning}");
                    out.warnings.push(warning);
                }
            }
        }
    }
    out
}

/// Auswahl laut `skills:` im Agent: `all` (Default), `none` oder eine Namensliste.
pub fn select(
    skills: &[Skill],
    selection: Option<&beton_agents::spec::SkillSelection>,
) -> Vec<Skill> {
    use beton_agents::spec::{SkillMode, SkillSelection};
    match selection {
        None | Some(SkillSelection::Mode(SkillMode::All)) => skills.to_vec(),
        Some(SkillSelection::Mode(SkillMode::None)) => Vec::new(),
        Some(SkillSelection::List(names)) => skills
            .iter()
            .filter(|s| names.contains(&s.name))
            .cloned()
            .collect(),
    }
}

/// Fehler von `skill_load` / `skill_read_file`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SkillError {
    #[error("skill_not_found: Skill „{0}“ ist in dieser Session nicht verfügbar")]
    NotFound(String),
    #[error("skill_not_model_invocable: Skill „{0}“ ist nur über /{0} aufrufbar")]
    NotModelInvocable(String),
    #[error("path_outside_skill: „{0}“ liegt außerhalb des Skill-Ordners")]
    OutsideSkill(String),
    #[error("file_not_found: {0}")]
    FileNotFound(String),
    #[error("file_too_large: {0} ist größer als {MAX_READ_BYTES} Bytes")]
    TooLarge(String),
    #[error("io: {0}")]
    Io(String),
}

impl SkillError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "skill_not_found",
            Self::NotModelInvocable(_) => "skill_not_model_invocable",
            Self::OutsideSkill(_) => "path_outside_skill",
            Self::FileNotFound(_) => "file_not_found",
            Self::TooLarge(_) => "file_too_large",
            Self::Io(_) => "io",
        }
    }
}

/// Skills einer Session für `skill_load`/`skill_read_file`.
#[derive(Debug, Clone, Default)]
pub struct SkillSet {
    skills: Vec<Skill>,
}

impl SkillSet {
    pub fn new(skills: Vec<Skill>) -> Self {
        Self { skills }
    }

    pub fn all(&self) -> &[Skill] {
        &self.skills
    }

    /// Skills, die das Modell selbst laden darf.
    pub fn model_invocable(&self) -> impl Iterator<Item = &Skill> {
        self.skills.iter().filter(|s| !s.disable_model_invocation)
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    fn get(&self, name: &str) -> Result<&Skill, SkillError> {
        let skill = self
            .skills
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| SkillError::NotFound(name.to_owned()))?;
        if skill.disable_model_invocation {
            return Err(SkillError::NotModelInvocable(name.to_owned()));
        }
        Ok(skill)
    }

    /// Body der `SKILL.md` ohne Frontmatter (AGT-008 AC2).
    pub fn load(&self, name: &str) -> Result<String, SkillError> {
        let skill = self.get(name)?;
        let text = std::fs::read_to_string(skill.dir.join(SKILL_MD))
            .map_err(|e| SkillError::Io(e.to_string()))?;
        Ok(body(&text).to_owned())
    }

    /// Eine Datei relativ zum Skill-Ordner. Absolute Pfade, `..` und Symlinks, deren Ziel
    /// außerhalb des Skill-Ordners liegt, werden abgelehnt (AGT-008 AC3).
    pub fn read_file(&self, name: &str, rel: &str) -> Result<Vec<u8>, SkillError> {
        let skill = self.get(name)?;
        let target = contained(&skill.dir, rel)?;
        let meta = std::fs::metadata(&target).map_err(|_| SkillError::FileNotFound(rel.into()))?;
        if !meta.is_file() {
            return Err(SkillError::FileNotFound(rel.into()));
        }
        if meta.len() > MAX_READ_BYTES {
            return Err(SkillError::TooLarge(rel.into()));
        }
        std::fs::read(&target).map_err(|e| SkillError::Io(e.to_string()))
    }
}

/// Löst `rel` im Ordner `dir` auf und stellt sicher, dass das (symlink-aufgelöste) Ziel darin
/// liegt.
fn contained(dir: &Path, rel: &str) -> Result<PathBuf, SkillError> {
    let outside = || SkillError::OutsideSkill(rel.to_owned());
    let rel_path = Path::new(rel);
    if rel.is_empty() || rel_path.is_absolute() || rel.contains('\0') {
        return Err(outside());
    }
    for c in rel_path.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(outside());
            }
        }
    }
    let base = dir
        .canonicalize()
        .map_err(|e| SkillError::Io(e.to_string()))?;
    let target = base.join(rel_path);
    // Symlinks auflösen; existiert die Datei nicht, ist sie auch nicht lesbar.
    let resolved = target
        .canonicalize()
        .map_err(|_| SkillError::FileNotFound(rel.to_owned()))?;
    if !resolved.starts_with(&base) {
        return Err(outside());
    }
    Ok(resolved)
}

/// Legt das Session-Skill-Verzeichnis an: ein Claude-Code-Plugin `beton` mit
/// `.claude-plugin/plugin.json` und `skills/<name>/…` (Codex nutzt `<dir>/skills` als
/// zusätzliche Skill-Wurzel). Kopiert nur Dateien innerhalb des jeweiligen Skill-Ordners;
/// Symlinks nach außen werden ausgelassen.
pub fn materialize(skills: &[Skill], dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir.join(".claude-plugin"))?;
    let manifest = serde_json::json!({
        "name": PLUGIN_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Skills dieser beton-Session (AGT-008)",
    });
    std::fs::write(
        dir.join(".claude-plugin/plugin.json"),
        serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
    )?;
    let skills_dir = dir.join("skills");
    std::fs::create_dir_all(&skills_dir)?;
    for skill in skills {
        if !valid_name(&skill.name) {
            continue;
        }
        let base = skill.dir.canonicalize()?;
        copy_tree(&base, &base, &skills_dir.join(&skill.name))?;
    }
    Ok(())
}

fn copy_tree(base: &Path, from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    let mut entries: Vec<_> = std::fs::read_dir(from)?.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let Ok(resolved) = entry.path().canonicalize() else {
            continue;
        };
        if !resolved.starts_with(base) {
            tracing::warn!(
                "Skill-Datei ausgelassen (Symlink nach außen): {}",
                entry.path().display()
            );
            continue;
        }
        let dest = to.join(entry.file_name());
        if resolved.is_dir() {
            copy_tree(base, &resolved, &dest)?;
        } else if resolved.is_file() {
            std::fs::copy(&resolved, &dest)?;
        }
    }
    Ok(())
}

/// Skill-Index für die Beschreibung von `skill_load` (Name + Beschreibung).
pub fn index(skills: &SkillSet) -> String {
    skills
        .model_invocable()
        .map(|s| format!("- {}: {}", s.name, s.description.replace('\n', " ")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn skill(dir: &Path, name: &str, front: &str, body: &str) -> PathBuf {
        let d = dir.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(SKILL_MD), format!("---\n{front}---\n{body}")).unwrap();
        d
    }

    fn named(dir: &Path, name: &str, description: &str, body: &str) -> PathBuf {
        skill(
            dir,
            name,
            &format!("name: {name}\ndescription: {description}\n"),
            body,
        )
    }

    struct Layout {
        _tmp: tempfile::TempDir,
        agent: PathBuf,
        project: PathBuf,
        beton_home: PathBuf,
        home: PathBuf,
    }

    fn layout() -> Layout {
        let tmp = tempfile::tempdir().unwrap();
        let p = |s: &str| {
            let d = tmp.path().join(s);
            std::fs::create_dir_all(&d).unwrap();
            d
        };
        Layout {
            agent: p("agent/skills"),
            project: p("project"),
            beton_home: p("home/.beton"),
            home: p("home"),
            _tmp: tmp,
        }
    }

    impl Layout {
        fn roots(&self) -> Vec<SkillRoot> {
            roots(
                Some(&self.agent),
                &self.project,
                &self.beton_home,
                Some(&self.home),
            )
        }
    }

    #[test]
    fn agt_008_ac1_agent_skill_shadows_project_skill() {
        let l = layout();
        named(&l.project.join(".beton/skills"), "fix-ci", "Projekt", "P");
        named(&l.agent, "fix-ci", "Agent", "A");
        named(&l.project.join(".claude/skills"), "only-claude", "C", "c");
        named(&l.home.join(".agents/skills"), "only-user", "U", "u");
        // Projekt verschattet User.
        named(&l.home.join(".claude/skills"), "only-claude", "User", "u");
        let found = discover(&l.roots());
        let by_name = |n: &str| found.skills.iter().find(|s| s.name == n).unwrap();
        assert_eq!(by_name("fix-ci").origin, Origin::Agent);
        assert_eq!(by_name("fix-ci").description, "Agent");
        assert_eq!(by_name("only-claude").origin, Origin::Project);
        assert_eq!(by_name("only-claude").vendor, Some(VendorDir::Claude));
        assert_eq!(by_name("only-user").origin, Origin::User);
        assert_eq!(found.skills.len(), 3);
    }

    #[test]
    fn agt_008_discovery_order_matches_spec() {
        let l = layout();
        let dirs: Vec<PathBuf> = l.roots().into_iter().map(|r| r.dir).collect();
        assert_eq!(
            dirs,
            [
                l.agent.clone(),
                l.project.join(".beton/skills"),
                l.project.join(".claude/skills"),
                l.project.join(".agents/skills"),
                l.beton_home.join("skills"),
                l.home.join(".claude/skills"),
                l.home.join(".agents/skills"),
            ]
        );
    }

    #[test]
    fn agt_008_ac2_load_returns_body_without_frontmatter() {
        let l = layout();
        named(
            &l.agent,
            "fix-ci",
            "Behebt CI",
            "# Fix CI\n\n1. Logs lesen\n",
        );
        let set = SkillSet::new(discover(&l.roots()).skills);
        assert_eq!(set.load("fix-ci").unwrap(), "# Fix CI\n\n1. Logs lesen\n");
        assert_eq!(
            set.load("fehlt").unwrap_err(),
            SkillError::NotFound("fehlt".into())
        );
    }

    #[test]
    fn agt_008_ac3_read_file_rejects_paths_outside_the_skill() {
        let l = layout();
        let dir = named(&l.agent, "fix-ci", "Behebt CI", "b");
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(dir.join("scripts/run.sh"), "echo ok\n").unwrap();
        let secret = l.project.join("geheim.txt");
        std::fs::write(&secret, "geheim").unwrap();
        // Symlink nach innen ist erlaubt, nach außen nicht.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&secret, dir.join("raus.txt")).unwrap();
            std::os::unix::fs::symlink(dir.join("scripts/run.sh"), dir.join("rein.sh")).unwrap();
            std::os::unix::fs::symlink(&l.project, dir.join("projekt")).unwrap();
        }
        let set = SkillSet::new(discover(&l.roots()).skills);
        assert_eq!(
            set.read_file("fix-ci", "scripts/run.sh").unwrap(),
            b"echo ok\n"
        );
        assert_eq!(
            set.read_file("fix-ci", "./scripts/run.sh").unwrap(),
            b"echo ok\n"
        );
        for bad in [
            "../../project/geheim.txt",
            "scripts/../../fix-ci/SKILL.md",
            "/etc/passwd",
            secret.to_str().unwrap(),
            "",
        ] {
            assert_eq!(
                set.read_file("fix-ci", bad).unwrap_err().code(),
                "path_outside_skill",
                "{bad}"
            );
        }
        #[cfg(unix)]
        {
            assert_eq!(set.read_file("fix-ci", "rein.sh").unwrap(), b"echo ok\n");
            assert_eq!(
                set.read_file("fix-ci", "raus.txt").unwrap_err().code(),
                "path_outside_skill"
            );
            assert_eq!(
                set.read_file("fix-ci", "projekt/geheim.txt")
                    .unwrap_err()
                    .code(),
                "path_outside_skill"
            );
        }
        assert_eq!(
            set.read_file("fix-ci", "scripts/fehlt.sh")
                .unwrap_err()
                .code(),
            "file_not_found"
        );
    }

    #[test]
    fn agt_008_ac4_skill_without_name_or_description_is_skipped_with_warning() {
        let l = layout();
        skill(&l.agent, "ohne-name", "description: d\n", "b");
        skill(&l.agent, "ohne-desc", "name: ohne-desc\n", "b");
        let d = l.agent.join("ohne-front");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(SKILL_MD), "nur Text").unwrap();
        skill(&l.agent, "boese", "name: ../boese\ndescription: d\n", "b");
        named(&l.agent, "gut", "d", "b");
        let found = discover(&l.roots());
        assert_eq!(
            found
                .skills
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["gut"]
        );
        assert_eq!(found.warnings.len(), 4, "{:?}", found.warnings);
        assert!(found.warnings.iter().any(|w| w.contains("`name` fehlt")));
        assert!(
            found
                .warnings
                .iter()
                .any(|w| w.contains("`description` fehlt"))
        );
        assert!(
            found
                .warnings
                .iter()
                .any(|w| w.contains("ungültiger Skill-Name"))
        );
    }

    #[test]
    fn selection_filters_and_model_invocation_is_respected() {
        let l = layout();
        named(&l.agent, "a", "A", "a");
        named(&l.agent, "b", "B", "b");
        skill(
            &l.agent,
            "nur-mensch",
            "name: nur-mensch\ndescription: M\ndisable-model-invocation: true\nallowed-tools: Read, Grep\n",
            "m",
        );
        let all = discover(&l.roots()).skills;
        use beton_agents::spec::{SkillMode, SkillSelection};
        assert_eq!(select(&all, None).len(), 3);
        assert!(select(&all, Some(&SkillSelection::Mode(SkillMode::None))).is_empty());
        let picked = select(&all, Some(&SkillSelection::List(vec!["b".into()])));
        assert_eq!(picked.len(), 1);
        let set = SkillSet::new(all);
        assert_eq!(
            set.load("nur-mensch").unwrap_err().code(),
            "skill_not_model_invocable"
        );
        let idx = index(&set);
        assert!(idx.contains("- a: A") && idx.contains("- b: B"), "{idx}");
        assert!(!idx.contains("nur-mensch"));
        let m = set.all().iter().find(|s| s.name == "nur-mensch").unwrap();
        assert_eq!(m.allowed_tools, ["Read", "Grep"]);
    }

    #[test]
    fn agt_008_materialize_builds_a_claude_plugin_and_skips_outside_symlinks() {
        let l = layout();
        let dir = named(&l.agent, "fix-ci", "Behebt CI", "b");
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(dir.join("scripts/run.sh"), "x").unwrap();
        #[cfg(unix)]
        {
            let secret = l.project.join("geheim.txt");
            std::fs::write(&secret, "geheim").unwrap();
            std::os::unix::fs::symlink(&secret, dir.join("raus.txt")).unwrap();
        }
        let set = discover(&l.roots()).skills;
        let out = l.project.join("session-skills");
        materialize(&set, &out).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join(".claude-plugin/plugin.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["name"], PLUGIN_NAME);
        assert!(out.join("skills/fix-ci/SKILL.md").is_file());
        assert!(out.join("skills/fix-ci/scripts/run.sh").is_file());
        assert!(!out.join("skills/fix-ci/raus.txt").exists());
    }

    #[test]
    fn frontmatter_split_handles_crlf_and_missing_end() {
        assert_eq!(body("---\r\nname: a\r\n---\r\nText"), "Text");
        assert_eq!(body("ohne"), "ohne");
        assert_eq!(body("---\nname: a\nkein ende"), "---\nname: a\nkein ende");
    }
}
