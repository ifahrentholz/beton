//! Skills einer Session für das Slash-Menü im Composer (WEB-006 AC3).
//!
//! Dieselbe Discovery wie beim Start des Runners (AGT-008, `beton_mcp::skills`): Agent-Ordner
//! `skills/`, dann Projekt (`.beton/skills`, `.claude/skills`, `.agents/skills`), dann User;
//! Auswahl laut `skills:` des Agents. Gezeigt werden nur Skills, die der Mensch aufrufen darf
//! (`user-invocable`).

use std::path::{Path, PathBuf};

use axum::Extension;
use axum::extract::{Path as UrlPath, State};
use beton_mcp::skills::{self, Origin};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

use crate::app::AppState;
use crate::problem::{ApiResult, Problem, ProblemCode};
use crate::security::Authenticated;

/// Ein Skill im Slash-Menü.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
pub struct SessionSkill {
    /// Aufruf als `/<name>`.
    pub name: String,
    pub description: String,
    /// Herkunft: `agent`, `project`, `user` oder `builtin`.
    pub origin: String,
}

/// Skills einer Session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
pub struct SessionSkills {
    /// Name des aktiven Agents, falls die Session einen hat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub agent: Option<String>,
    pub items: Vec<SessionSkill>,
    /// Immer leer: die Liste ist vollständig.
    pub next_cursor: Option<String>,
}

fn origin(o: Origin) -> &'static str {
    match o {
        Origin::Agent => "agent",
        Origin::Project => "project",
        Origin::User => "user",
        Origin::Builtin => "builtin",
    }
}

/// Wo die Skills des Users liegen: `BETON_HOME` bzw. `~/.beton` und `HOME`.
#[derive(Debug, Clone)]
pub struct SkillPaths {
    pub beton_home: PathBuf,
    pub home: Option<PathBuf>,
}

impl SkillPaths {
    pub fn from_process() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let beton_home = std::env::var_os("BETON_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".beton")))
            .unwrap_or_else(|| PathBuf::from(".beton"));
        Self { beton_home, home }
    }
}

/// Discovery für einen Workspace und optional einen Agent (blockierend).
pub fn discover(
    workdir: &Path,
    agent_ref: Option<&str>,
    paths: &SkillPaths,
) -> Result<SessionSkills, String> {
    let agent = match agent_ref {
        Some(r) => Some(beton_mcp::plan::load_agent(
            r,
            workdir,
            &paths.beton_home,
            beton_agents::Builtins::embedded(),
        )?),
        None => None,
    };
    // Built-in-Agents bringen ihre Skills eingebettet mit; kurz auspacken, nur zum Lesen.
    let scratch =
        std::env::temp_dir().join(format!("beton-skills-{}", beton_core::id::RunnerId::new()));
    let agent_skills = match &agent {
        Some(a) => beton_mcp::plan::agent_skills_dir(a, &scratch).map_err(|e| e.to_string())?,
        None => None,
    };
    let roots = skills::roots(
        agent_skills.as_deref(),
        workdir,
        &paths.beton_home,
        paths.home.as_deref(),
    );
    let found = skills::discover(&roots);
    let _ = std::fs::remove_dir_all(&scratch);
    let selected = skills::select(
        &found.skills,
        agent.as_ref().and_then(|a| a.spec.skills.as_ref()),
    );
    Ok(SessionSkills {
        agent: agent.map(|a| a.spec.name.to_string()),
        items: selected
            .into_iter()
            .filter(|s| s.user_invocable)
            .map(|s| SessionSkill {
                name: s.name,
                description: s.description,
                origin: origin(s.origin).into(),
            })
            .collect(),
        next_cursor: None,
    })
}

/// Skills des aktiven Agents und des Workspace für das Slash-Menü (WEB-006 AC3).
#[utoipa::path(get, path = "/v1/sessions/{id}/skills", tag = "sessions",
    params(("id" = String, Path), crate::extract::PageQuery),
    responses((status = 200, description = "Aufrufbare Skills mit Beschreibung", body = SessionSkills)))]
pub async fn list(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    UrlPath(id): UrlPath<String>,
    // Die Liste ist immer vollständig; `limit` und `cursor` gibt es der Einheitlichkeit halber.
    crate::extract::ApiQuery(_page): crate::extract::ApiQuery<crate::extract::PageQuery>,
) -> ApiResult<axum::Json<SessionSkills>> {
    let session = id
        .parse()
        .map_err(|_| Problem::new(ProblemCode::NotFound).detail(format!("Session {id}")))?;
    let record = state.store.session(state.local.org, session).await?;
    if !state.runtime.authorizer.can_read(auth, &record) {
        return Err(Problem::new(ProblemCode::Forbidden));
    }
    let sessions = state.sessions();
    let created = sessions.created(session).await?;
    let workdir = sessions.workspace_root(&record).await?;
    let paths = state.runtime.sessions.skill_paths.clone();
    let found = tokio::task::spawn_blocking(move || {
        discover(&workdir, created.agent_ref.as_deref(), &paths)
    })
    .await
    .map_err(|e| Problem::internal(&e))?
    .map_err(|e| Problem::new(ProblemCode::ValidationFailed).detail(format!("Agent: {e}")))?;
    Ok(axum::Json(found))
}
