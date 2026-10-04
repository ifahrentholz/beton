//! Was eine Session an Tools bekommt (HAR-009, AGT-006, AGT-007, AGT-008): MCP-Server nach
//! dem Merge, sichtbare System-Tools, ausgewählte Skills und die Sub-Agents für
//! `session_spawn`. Reine Funktion über Dateien; der Runner startet damit den Relay-Hub.

use std::path::{Path, PathBuf};

use beton_agents::dir::AgentDir;
use beton_agents::resolve::{AgentRef, SearchPath};
use beton_agents::spec::SystemTool;
use beton_agents::{AgentSpec, Builtins};

use crate::config::{self, ConfiguredServer, McpFile};
use crate::skills::{self, Skill, VendorDir};
use crate::system::{self, SpawnTarget};

/// Ein geladener Agent.
#[derive(Debug, Clone)]
pub struct LoadedAgent {
    pub spec: AgentSpec,
    pub dir: AgentDir,
}

/// Wo die Dateien einer Session liegen.
#[derive(Debug, Clone)]
pub struct PlanInput<'a> {
    /// Arbeitsverzeichnis der Session (Projekt: `.beton/mcp.yaml`, `.beton/skills`, …).
    pub workdir: &'a Path,
    /// `~/.beton` bzw. `BETON_HOME`.
    pub beton_home: &'a Path,
    /// Home des Nutzers für `~/.claude/skills` und `~/.agents/skills`.
    pub home: Option<&'a Path>,
    pub agent: Option<&'a LoadedAgent>,
    /// Skill-Verzeichnis des Agents im Dateisystem (bei Built-ins vorher ausgepackt).
    pub agent_skills: Option<&'a Path>,
    /// `mcp_bridge` laut Harness-Konfiguration (`harnesses.<id>` bzw. ACP-Agent).
    pub harness_bridge: bool,
}

/// Ergebnis für eine Session.
#[derive(Debug, Clone, Default)]
pub struct SessionPlan {
    pub servers: Vec<ConfiguredServer>,
    /// Bekommt der Harness den Server `beton`? (`mcp_bridge`, HAR-009 AC2)
    pub bridge: bool,
    pub system_tools: Vec<SystemTool>,
    pub skills: Vec<Skill>,
    pub spawn: Vec<SpawnTarget>,
    /// Hinweise für das Event-Log (fehlerhafte Konfiguration, übersprungene Skills).
    pub notices: Vec<String>,
}

/// Lädt einen Agent über den Suchpfad (AGT-003). Fehler: fail closed, die Session startet
/// nicht.
pub fn load_agent(
    reference: &str,
    workdir: &Path,
    beton_home: &Path,
    builtins: Builtins,
) -> Result<LoadedAgent, String> {
    let r = AgentRef::parse(reference)?;
    let search = SearchPath {
        project: Some(workdir.join(".beton/agents")),
        user: Some(beton_home.join("agents")),
        builtins,
    };
    let located = search.resolve(&r, workdir).map_err(|e| e.to_string())?;
    load_dir(located.dir)
}

fn load_dir(dir: AgentDir) -> Result<LoadedAgent, String> {
    let text = dir
        .read(beton_agents::dir::AGENT_YAML)
        .ok_or_else(|| format!("{}: agent.yaml fehlt", dir.display()))?;
    let text = String::from_utf8_lossy(&text);
    let file = format!("{}/agent.yaml", dir.display());
    let parsed = beton_agents::load::parse_agent_yaml(&text, &file);
    if let Some(d) = parsed.diagnostics.iter().find(|d| d.is_error()) {
        return Err(format!("{}:{}:{}: {}", d.file, d.line, d.column, d.message));
    }
    let spec = parsed
        .spec
        .ok_or_else(|| format!("{file}: Agent ungültig"))?;
    Ok(LoadedAgent { spec, dir })
}

/// `<agent>/skills` im Dateisystem; Built-ins werden nach `scratch` ausgepackt.
pub fn agent_skills_dir(agent: &LoadedAgent, scratch: &Path) -> std::io::Result<Option<PathBuf>> {
    match &agent.dir {
        AgentDir::Fs(dir) => Ok(Some(dir.join("skills"))),
        AgentDir::Builtin { .. } => {
            let mut any = false;
            for (rel, bytes) in agent.dir.files() {
                let Some(inner) = rel.strip_prefix("skills/") else {
                    continue;
                };
                // Eingebettete Pfade sind normalisiert; trotzdem keine Ausbrüche zulassen.
                let Some(norm) = beton_agents::dir::normalize_relative("", inner) else {
                    continue;
                };
                let dest = scratch.join(&norm);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(dest, bytes)?;
                any = true;
            }
            Ok(any.then(|| scratch.to_owned()))
        }
    }
}

/// Sub-Agents aus `spawn.agents` mit ihrem Harness (inline `executor` oder `ref`).
fn spawn_targets(agent: &LoadedAgent, notices: &mut Vec<String>) -> Vec<SpawnTarget> {
    let Some(spawn) = &agent.spec.spawn else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for name in &spawn.agents {
        let Some(sub) = agent.spec.agents.get(name) else {
            notices.push(format!("spawn.agents: „{name}“ fehlt unter `agents`"));
            continue;
        };
        let (executor, description) = match (&sub.executor, &sub.reference) {
            (Some(e), _) => (Some(e.clone()), sub.description.clone()),
            (None, Some(reference)) => match resolve_ref(agent, reference) {
                Ok(child) => (
                    Some(child.spec.executor.clone()),
                    sub.description.clone().or(child.spec.description.clone()),
                ),
                Err(e) => {
                    notices.push(format!("Sub-Agent „{name}“: {e}"));
                    (None, None)
                }
            },
            (None, None) => (None, None),
        };
        let Some(executor) = executor else {
            notices.push(format!("Sub-Agent „{name}“ ohne executor"));
            continue;
        };
        out.push(SpawnTarget {
            name: name.clone(),
            description,
            harness: executor.harness.to_string(),
            model: executor.model.clone(),
        });
    }
    out
}

fn resolve_ref(agent: &LoadedAgent, reference: &str) -> Result<LoadedAgent, String> {
    match AgentRef::parse(reference)? {
        AgentRef::Path(p) if p.is_relative() => {
            let rel = p.to_string_lossy().replace('\\', "/");
            let dir = agent
                .dir
                .join(&rel)
                .ok_or_else(|| format!("{reference} liegt außerhalb des Agents"))?;
            load_dir(dir)
        }
        other => {
            let search = SearchPath {
                project: None,
                user: None,
                builtins: Builtins::embedded(),
            };
            let base = match &agent.dir {
                AgentDir::Fs(d) => d.clone(),
                AgentDir::Builtin { .. } => PathBuf::from("."),
            };
            let located = search.resolve(&other, &base).map_err(|e| e.to_string())?;
            load_dir(located.dir)
        }
    }
}

fn load_layer(path: &Path, notices: &mut Vec<String>) -> McpFile {
    config::load(path).unwrap_or_else(|e| {
        // Fehlerhafte Datei: keiner ihrer Server startet (fail closed), die Session schon.
        notices.push(format!("MCP-Konfiguration ignoriert: {e}"));
        McpFile::default()
    })
}

/// Stellt den Plan einer Session zusammen.
pub fn build(input: &PlanInput<'_>) -> SessionPlan {
    let mut notices = Vec::new();
    let user = load_layer(&input.beton_home.join("mcp.yaml"), &mut notices);
    let project = load_layer(&input.workdir.join(".beton/mcp.yaml"), &mut notices);
    let spec = input.agent.map(|a| &a.spec);
    let servers = config::merge(&user, &project, spec.and_then(|s| s.tools.as_ref()));

    let roots = skills::roots(
        input.agent_skills,
        input.workdir,
        input.beton_home,
        input.home,
    );
    let found = skills::discover(&roots);
    notices.extend(
        found
            .warnings
            .iter()
            .map(|w| format!("Skill übersprungen: {w}")),
    );
    let selected = skills::select(&found.skills, spec.and_then(|s| s.skills.as_ref()));

    let bridge = input.harness_bridge && spec.and_then(|s| s.executor.mcp_bridge) != Some(false);
    let spawn = input
        .agent
        .map(|a| spawn_targets(a, &mut notices))
        .unwrap_or_default();
    let system_tools = if bridge {
        system::enabled_tools(spec, !selected.is_empty())
    } else {
        Vec::new()
    };
    SessionPlan {
        servers,
        bridge,
        system_tools,
        skills: selected,
        spawn,
        notices,
    }
}

impl SessionPlan {
    /// Skills, die ein Harness mit nativen Skills zusätzlich im Session-Skill-Verzeichnis
    /// bekommt: alle ausgewählten außer denen aus dem Vendor-Verzeichnis, das er selbst liest.
    pub fn native_skills(&self, reads_itself: Option<VendorDir>) -> Vec<Skill> {
        self.skills
            .iter()
            .filter(|s| reads_itself.is_none_or(|v| s.vendor != Some(v)))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    struct Fx {
        _tmp: tempfile::TempDir,
        work: PathBuf,
        beton: PathBuf,
        home: PathBuf,
    }

    fn fx() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        let home = tmp.path().join("home");
        let beton = home.join(".beton");
        for d in [&work, &home, &beton] {
            std::fs::create_dir_all(d).unwrap();
        }
        Fx {
            _tmp: tmp,
            work,
            beton,
            home,
        }
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn input<'a>(
        f: &'a Fx,
        agent: Option<&'a LoadedAgent>,
        skills: Option<&'a Path>,
    ) -> PlanInput<'a> {
        PlanInput {
            workdir: &f.work,
            beton_home: &f.beton,
            home: Some(&f.home),
            agent,
            agent_skills: skills,
            harness_bridge: true,
        }
    }

    #[test]
    fn interactive_session_gets_user_and_project_servers_and_default_tools() {
        let f = fx();
        write(
            &f.beton.join("mcp.yaml"),
            "servers:\n  u:\n    command: u\n",
        );
        write(
            &f.work.join(".beton/mcp.yaml"),
            "servers:\n  p:\n    command: p\n",
        );
        let plan = build(&input(&f, None, None));
        let names: Vec<_> = plan.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["p", "u"]);
        assert!(plan.bridge);
        assert_eq!(plan.system_tools, [SystemTool::PolicyQuery]);
    }

    #[test]
    fn broken_config_is_ignored_with_notice() {
        let f = fx();
        write(
            &f.work.join(".beton/mcp.yaml"),
            "servers:\n  p:\n    comand: p\n",
        );
        let plan = build(&input(&f, None, None));
        assert!(plan.servers.is_empty());
        assert!(plan.notices[0].contains("comand"), "{:?}", plan.notices);
    }

    #[test]
    fn har_009_ac2_mcp_bridge_false_removes_the_system_server() {
        let f = fx();
        let mut i = input(&f, None, None);
        i.harness_bridge = false;
        let plan = build(&i);
        assert!(!plan.bridge);
        assert!(plan.system_tools.is_empty());

        let dir = f.work.join(".beton/agents/a");
        write(
            &dir.join("agent.yaml"),
            "spec_version: 1\nname: a\nexecutor: { harness: claude, mcp_bridge: false }\n",
        );
        let agent = load_agent("a", &f.work, &f.beton, Builtins::default()).unwrap();
        let plan = build(&input(&f, Some(&agent), None));
        assert!(!plan.bridge);
    }

    #[test]
    fn agent_plan_resolves_spawn_targets_and_selects_skills() {
        let f = fx();
        let dir = f.work.join(".beton/agents/lead");
        write(
            &dir.join("agent.yaml"),
            "spec_version: 1\nname: lead\nexecutor: { harness: claude }\nskills: [fix-ci]\nagents:\n  quick-check:\n    description: Reviewt\n    executor: { harness: codex, model: gpt-5-codex }\n  reviewer:\n    ref: ./agents/reviewer\n  unused:\n    executor: { harness: codex }\nspawn: { agents: [quick-check, reviewer] }\ntools:\n  mcp:\n    gh: { command: gh-mcp, allow: [get] }\n",
        );
        write(
            &dir.join("agents/reviewer/agent.yaml"),
            "spec_version: 1\nname: reviewer\ndescription: Prüft Diffs\nexecutor: { harness: codex }\n",
        );
        write(
            &dir.join("skills/fix-ci/SKILL.md"),
            "---\nname: fix-ci\ndescription: CI\n---\nb\n",
        );
        write(
            &dir.join("skills/other/SKILL.md"),
            "---\nname: other\ndescription: O\n---\nb\n",
        );
        write(
            &f.beton.join("mcp.yaml"),
            "servers:\n  u:\n    command: u\n",
        );
        let agent = load_agent("lead", &f.work, &f.beton, Builtins::default()).unwrap();
        let skills_dir = agent_skills_dir(&agent, &f.work).unwrap().unwrap();
        let plan = build(&input(&f, Some(&agent), Some(&skills_dir)));
        // Ohne inherit nur Agent-Server.
        assert_eq!(plan.servers.len(), 1);
        assert_eq!(plan.servers[0].name, "gh");
        let spawn: Vec<_> = plan
            .spawn
            .iter()
            .map(|t| (t.name.as_str(), t.harness.as_str()))
            .collect();
        assert_eq!(spawn, [("quick-check", "codex"), ("reviewer", "codex")]);
        assert_eq!(plan.spawn[1].description.as_deref(), Some("Prüft Diffs"));
        assert_eq!(
            plan.skills
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["fix-ci"]
        );
        assert_eq!(
            plan.system_tools,
            [
                SystemTool::PolicyQuery,
                SystemTool::SessionSpawn,
                SystemTool::SkillLoad,
                SystemTool::SkillReadFile
            ]
        );
    }

    #[test]
    fn invalid_agent_fails_closed() {
        let f = fx();
        write(
            &f.work.join(".beton/agents/bad/agent.yaml"),
            "spec_version: 1\nname: bad\nexecutor: { harness: claude }\ninstruction: x\n",
        );
        let err = load_agent("bad", &f.work, &f.beton, Builtins::default()).unwrap_err();
        assert!(err.contains("instruction"), "{err}");
        assert!(load_agent("fehlt", &f.work, &f.beton, Builtins::default()).is_err());
    }

    #[test]
    fn builtin_agent_skills_are_unpacked() {
        let f = fx();
        let builtins = Builtins::from_files([
            (
                "b/agent.yaml",
                "spec_version: 1\nname: b\nexecutor: { harness: claude }\n".as_bytes(),
            ),
            (
                "b/skills/s/SKILL.md",
                "---\nname: s\ndescription: d\n---\nx\n".as_bytes(),
            ),
        ]);
        let agent = load_agent("builtin:b", &f.work, &f.beton, builtins).unwrap();
        let scratch = f.work.join("scratch");
        let dir = agent_skills_dir(&agent, &scratch).unwrap().unwrap();
        assert!(dir.join("s/SKILL.md").is_file());
    }

    #[test]
    fn native_skills_skip_the_vendor_dir_the_harness_reads_itself() {
        let f = fx();
        write(
            &f.work.join(".claude/skills/c/SKILL.md"),
            "---\nname: c\ndescription: d\n---\n",
        );
        write(
            &f.work.join(".agents/skills/a/SKILL.md"),
            "---\nname: a\ndescription: d\n---\n",
        );
        write(
            &f.work.join(".beton/skills/b/SKILL.md"),
            "---\nname: b\ndescription: d\n---\n",
        );
        let plan = build(&input(&f, None, None));
        let names = |v: Vec<Skill>| v.into_iter().map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(
            names(plan.native_skills(Some(VendorDir::Claude))),
            ["b", "a"]
        );
        assert_eq!(
            names(plan.native_skills(Some(VendorDir::Agents))),
            ["b", "c"]
        );
    }
}
