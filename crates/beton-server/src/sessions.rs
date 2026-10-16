//! Session-Lebenszyklus (SES-001, SES-003, SES-005): anlegen, Runner starten, Eingaben
//! zustellen, unterbrechen, archivieren, löschen, fortsetzen. Worktree pro Session beim Anlegen
//! und kontrolliertes Entfernen beim Löschen (SES-015, SES-016).
//!
//! Der Server führt nie Agent-Code aus; er startet Runner über den `RunnerProvider` und
//! spricht mit ihnen über den Tunnel (`RunnerRegistry`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use beton_core::event::{
    Actor, Attachment, Empty, Event, EventPayload, SessionKind, SessionTitleChanged,
    SessionTrigger, TitleSource,
};
use beton_core::id::{InputId, OrgId, PrincipalId, RunnerId, SessionId, UserId};
use beton_host::{RunnerBoot, RunnerHandle, RunnerProvider, RunnerSpec, TerminateMode};
use beton_store::{DeleteAuthority, NewSession, SessionRecord};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::app::AppState;
use crate::problem::{Problem, ProblemCode};
use crate::queue::Accepted;

/// Wie eine Eingabe zugestellt wird (SES-004).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Deserialize,
    serde::Serialize,
    utoipa::ToSchema,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    /// Sofort, wenn kein Turn läuft; sonst in die Queue.
    #[default]
    Queue,
    /// In den laufenden Turn (Capability `steering`); ohne Turn wie `queue`.
    Steer,
}

/// Wie lange Eingaben auf einen frisch gestarteten Runner warten.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// Standard für [`SessionsConfig::start_budget`].
pub(crate) const START_BUDGET: Duration = Duration::from_secs(90);

/// Gestartete Runner dieses Knotens.
#[derive(Default)]
pub struct Launched {
    handles: Mutex<HashMap<SessionId, RunnerHandle>>,
    /// Tunnel-Token und Startzeit des zuletzt gestarteten Runners je Session. Ist das Token
    /// gesperrt (geordneter Stopp) oder das Start-Budget abgelaufen, startet dieser Runner nicht
    /// mehr, auch wenn sein Prozess noch läuft (#198).
    tokens: Mutex<HashMap<SessionId, (String, Instant)>>,
    /// Je Session höchstens ein Start gleichzeitig: Prüfen und Starten in `resume` laufen unter
    /// dieser Sperre, sonst starteten zwei gleichzeitige Aufrufe zwei Runner.
    starts: std::sync::Mutex<HashMap<SessionId, Arc<Mutex<()>>>>,
}

impl Launched {
    /// Sperre für Starts der Session.
    fn start_lock(&self, session: SessionId) -> Arc<Mutex<()>> {
        self.starts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(session)
            .or_default()
            .clone()
    }

    /// Prozess-ID des Runners einer Session (Diagnose, Tests von HAR-020).
    pub async fn pid(&self, session: SessionId) -> Option<u32> {
        self.handles.lock().await.get(&session).and_then(|h| h.pid)
    }
}

impl std::fmt::Debug for Launched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Launched")
    }
}

/// Wie Sessions gestartet werden.
#[derive(Clone)]
pub struct SessionsConfig {
    pub provider: Arc<dyn RunnerProvider>,
    /// Wie lange eine Eingabe bzw. `resume` auf die Anmeldung eines Runners wartet.
    pub connect_timeout: Duration,
    /// Wie lange ein gestarteter Runner als „startet gerade“ gilt; danach ersetzt ihn der
    /// nächste Start. Deutlich über dem längsten Harness-Handshake (Codex: 30 s).
    pub start_budget: Duration,
    pub tunnel_socket: Option<PathBuf>,
    /// Entwicklermodus: Fake-Harness erlaubt (HAR-026 AC3).
    pub dev: bool,
    pub launched: Arc<Launched>,
    /// `harnesses:` aus der User-Konfiguration; die Projekt-Konfiguration liest der Daemon
    /// beim Start aus dem Arbeitsverzeichnis der Session (HAR-003).
    pub harnesses_user: beton_harness::registry::HarnessesConfig,
    /// `providers:` aus der User-Konfiguration (Direkt-API-Harness, HAR-011); leer = kein
    /// `direct:*`-Harness.
    pub providers: std::collections::BTreeMap<String, Value>,
    /// Wurzel der Session-Worktrees, z. B. `~/.beton/worktrees` (SES-015).
    pub worktrees_root: PathBuf,
    /// Schatten-Repositories der Turn-Snapshots, z. B. `~/.beton/snapshots` (SES-018).
    pub snapshots_root: PathBuf,
    /// Fork-Pläne für den Runner-Start, z. B. `~/.beton/forks` (HAR-018, HAR-019).
    pub forks_root: PathBuf,
    /// Leere Workspaces von Forks mit `workspace: fresh`, z. B. `~/.beton/workspaces` (SES-006).
    pub fresh_root: PathBuf,
    /// Agent-Snapshots für den Runner-Start, z. B. `~/.beton/agent-snapshots` (AGT-004).
    pub agent_snapshots_root: PathBuf,
    /// `~/.beton` bzw. `BETON_HOME`: User-Agents unter `agents/` (AGT-003).
    pub beton_home: PathBuf,
    /// Umgebung, in der der Import die Verzeichnisse der Vendor-CLIs findet (`HOME`,
    /// `CLAUDE_CONFIG_DIR`, `CODEX_HOME`; SES-008). Enthält nur Pfade.
    pub vendor_env: beton_harness::HostEnv,
    /// Importe laufen nacheinander, damit der Dedup-Schlüssel nicht doppelt vergeben wird.
    pub import_lock: Arc<Mutex<()>>,
    /// Skill-Verzeichnisse des Users für das Slash-Menü (WEB-006, AGT-008).
    pub skill_paths: crate::session_skills::SkillPaths,
    /// `sandbox:` der User-Konfiguration (SBX-003); greift im Runner als Stufenmodell
    /// (SBX-002). Default „Sandbox erforderlich“ (SBX-006); ohne Isolation nur mit
    /// ausdrücklichem `backend: none` und Bestätigung je Session.
    pub sandbox: beton_sandbox::SandboxSpec,
    /// Probe des gewählten Backends für Vorprüfung und `GET /v1/info` (SBX-006).
    pub sandbox_probe: Arc<crate::sandbox_status::ProbeCache>,
    /// Launcher der Linux-Probe (`__sandbox-exec`): der Runner, falls er nicht das eigene
    /// Binary ist (eingebettete Daemons in Tests); sonst das eigene Binary.
    pub sandbox_launcher: Option<PathBuf>,
    /// `proxy.extra_ca_files` (PRX-002).
    pub proxy_extra_ca_files: Vec<PathBuf>,
    /// Lädt die beton-CA über die Proxy-Capability (PRX-002); `None`: keine Session mit Netz
    /// im Stufenmodell (fail closed).
    pub proxy_ca: Option<Arc<dyn ProxyCaSource>>,
    /// Werte und Audit der Credential-Bindings (PRX-006); `None`: Sessions mit Credentials
    /// bekommen keine Werte (fail closed).
    pub credentials: Option<Arc<crate::credentials::CredentialService>>,
    /// `beton secrets test` über den Proxy-Pfad (SEC-004, SEC-011 AC3).
    pub secret_tester: Option<Arc<crate::secret_test::SecretTester>>,
    /// Umgebung des Daemons beim Start: Quelle `env` der Credentials (PRX-006) und `HOME` für
    /// `~` in `file`. Geht nie an Runner oder Sandbox.
    pub daemon_env: Arc<std::collections::BTreeMap<String, String>>,
}

/// Quelle der beton-CA für den Runner einer Session (PRX-001, PRX-002, #191).
#[async_trait::async_trait]
pub trait ProxyCaSource: Send + Sync {
    /// Die CA in Übergabeform; jeder Abruf ist auditiert (SEC-012).
    async fn handover(&self, session: SessionId) -> Result<beton_proxy::CaHandover, String>;
}

/// Die CA aus dem Secret-Store, gelesen nur über die Proxy-Capability (SEC-001 AC2).
pub struct StoreCa {
    pub store: beton_proxy::CaStore,
    /// Geteilt mit den Credentials der Sessions (PRX-006); beide sind der Egress-Proxy.
    pub cap: Arc<beton_secrets::ProxyCap>,
    pub host_id: String,
}

#[async_trait::async_trait]
impl ProxyCaSource for StoreCa {
    async fn handover(&self, session: SessionId) -> Result<beton_proxy::CaHandover, String> {
        let ca = self
            .store
            .load_or_create(
                &self.cap,
                beton_secrets::UseContext {
                    session_id: Some(session),
                    host_id: self.host_id.clone(),
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        ca.handover().map_err(|e| e.to_string())
    }
}

/// Was der Runner für das Stufenmodell bekommt: immer die Konfiguration (auch ausdrücklich
/// `backend: none`, damit der Runner die Bestätigung prüft) und, bei Netz, die CA. Fehlt die
/// CA, startet die Session nicht (PRX-002, fail closed).
pub async fn stage_boot(
    cfg: &SessionsConfig,
    session: SessionId,
) -> Result<(Option<beton_host::SandboxBoot>, Option<beton_host::CaText>), Problem> {
    stage_boot_for(cfg, session, true).await
}

/// Wie [`stage_boot`]; `harness_net = false`: Der Harness läuft im Runner (Fake-Harness) und
/// braucht ohne Netz der Tools keinen Proxy, also auch keine CA.
pub async fn stage_boot_for(
    cfg: &SessionsConfig,
    session: SessionId,
    harness_net: bool,
) -> Result<(Option<beton_host::SandboxBoot>, Option<beton_host::CaText>), Problem> {
    let spec = beton_sandbox::SandboxSpec::merged(&[&cfg.sandbox]);
    let boot = beton_host::SandboxBoot {
        spec: cfg.sandbox.clone(),
        extra_ca_files: cfg.proxy_extra_ca_files.clone(),
    };
    if !beton_host::stages_active(&spec) {
        return Ok((Some(boot), None));
    }
    // Ohne Netz für die Tools braucht der Harness in Stufe 1 trotzdem seinen Proxy für die
    // Vendor-Hosts (Passthrough): Die CA geht mit, wenn es eine Quelle gibt; fehlt sie, entscheidet
    // der Runner (in-process Harnesses brauchen keine, alle anderen starten nicht).
    let tools_offline = spec.allow_network == Some(false);
    if tools_offline && !harness_net {
        return Ok((Some(boot), None));
    }
    let Some(source) = cfg.proxy_ca.as_ref() else {
        if tools_offline {
            return Ok((Some(boot), None));
        }
        return Err(Problem::new(ProblemCode::SandboxUnavailable)
            .detail("proxy_ca_missing: keine Quelle für die beton-CA"));
    };
    let ca = match source.handover(session).await {
        Ok(ca) => ca,
        Err(_) if tools_offline => return Ok((Some(boot), None)),
        Err(e) => {
            return Err(
                Problem::new(ProblemCode::SandboxUnavailable).detail(format!("beton-CA: {e}"))
            );
        }
    };
    Ok((
        Some(boot),
        Some(beton_host::CaText::new(ca.expose().to_owned())),
    ))
}

impl SessionsConfig {
    /// Schatten-Repository einer Session.
    pub fn snapshots_dir(&self, session: SessionId) -> PathBuf {
        self.snapshots_root.join(format!("{session}.git"))
    }

    /// Fork-Plan einer Session.
    pub fn fork_plan(&self, session: SessionId) -> PathBuf {
        self.forks_root.join(format!("{session}.json"))
    }

    /// Leerer Workspace eines Forks mit `workspace: fresh`.
    pub fn fresh_dir(&self, session: SessionId) -> PathBuf {
        self.fresh_root.join(session.to_string())
    }

    /// Agent-Snapshot einer Session für den Runner (AGT-004).
    pub fn agent_snapshot(&self, session: SessionId) -> PathBuf {
        self.agent_snapshots_root.join(format!("{session}.json"))
    }
}

impl std::fmt::Debug for dyn ProxyCaSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProxyCaSource")
    }
}

impl std::fmt::Debug for SessionsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionsConfig")
            .field("dev", &self.dev)
            .finish_non_exhaustive()
    }
}

/// System-Tools, die der Runner über den Tunnel an den Server gibt (`system.call`, AGT-007).
pub struct ServerSystemCalls(pub AppState);

#[async_trait::async_trait]
impl crate::tunnel::SystemCalls for ServerSystemCalls {
    async fn call(&self, session: SessionId, tool: &str, args: Value) -> Result<Value, Value> {
        let problem = |p: Problem| {
            let v = serde_json::to_value(&p).unwrap_or(Value::Null);
            json!({"code": v["code"], "detail": v["detail"]})
        };
        let m = self.0.sessions();
        let result = match tool {
            "session.spawn" => m.spawn_child(session, &args).await,
            "session.send" => m.send_child(session, &args).await,
            "session.wait" => m.wait_children(session, &args).await,
            "session.status" => m.child_status(session, &args).await,
            "session.list" => m.list_children(session).await,
            "session.cancel" => m.cancel_child(session, &args).await,
            // Verbrauchswerte für Policies (POL-004 AC3/AC4): `session.cost_usd`,
            // `user.daily_cost_usd` aus der Usage-Projektion.
            "policy.usage" => crate::usage_policy::policy_usage(&self.0, session).await,
            // Zustand und Grants (POL-009, POL-010), Änderungen in `user_day`/`user`.
            "policy.state" => crate::policy_state::policy_state(&self.0, session).await,
            "policy.state.apply" => {
                crate::policy_state::policy_state_apply(&self.0, session, &args).await
            }
            // Set-Snapshot je Version für Explain (POL-027 AC2).
            "policy.snapshot" => {
                crate::api_policies::policy_snapshot(&self.0, session, &args).await
            }
            // Agent-Inbox (`inbox_read`, AGT-007).
            "inbox.read" => crate::policy_state::inbox_read(&self.0, session).await,
            // Credentials der Proxy-Instanzen des Runners (PRX-006, PRX-009 AC4).
            "credential.resolve" | "credential.used" | "credential.misuse"
            | "egress.passthrough" => credential_call(&self.0, session, tool, &args).await,
            other => {
                return Err(
                    json!({"code": "unknown_command", "detail": format!("System-Tool {other} kennt der Server nicht")}),
                );
            }
        };
        result.map_err(problem)
    }
}

/// Variablen aus `sandbox.env.passthrough`, die der Runner erben muss, damit die Auflösung sie
/// in die Sandbox übernehmen kann (SBX-005): Namen bzw. Präfixe (`LC_*`) gegen die Umgebung
/// des Daemons, nie gesperrte (SSH_AUTH_SOCK, LD_PRELOAD, …) oder `BETON_*`.
pub fn passthrough_names(
    spec: &beton_sandbox::SandboxSpec,
    env: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    let patterns = spec
        .env
        .as_ref()
        .and_then(|e| e.passthrough.clone())
        .unwrap_or_default();
    let mut out: Vec<String> = env
        .keys()
        .filter(|k| {
            patterns.iter().any(|p| match p.strip_suffix('*') {
                Some(prefix) => k.starts_with(prefix),
                None => *k == p,
            })
        })
        .filter(|k| !beton_sandbox::spec::env_denied(k) && !k.starts_with("BETON_"))
        .cloned()
        .collect();
    out.sort();
    out
}

/// Geht der Loop des Direkt-API-Harness im Stufenmodell über den Proxy (https mit Key)?
/// Loopback-Endpunkte über `http` behalten den Key über stdin (HAR-011).
pub fn direct_via_proxy(
    harness: &str,
    providers: &std::collections::BTreeMap<String, Value>,
) -> bool {
    let Some(name) = harness.strip_prefix("direct:") else {
        return false;
    };
    providers
        .get(name)
        .and_then(|v| beton_harness_direct::config::parse_provider(name, v).ok())
        .is_some_and(|p| p.key.lookup().is_some() && p.base_url.scheme() == "https")
}

/// `credential.*` aus dem Runner einer Session; ohne Credential-Dienst nie ein Wert.
async fn credential_call(
    state: &AppState,
    session: SessionId,
    tool: &str,
    args: &Value,
) -> Result<Value, Problem> {
    let cfg = &state.runtime.sessions;
    let service = cfg.credentials.as_ref().ok_or_else(|| {
        Problem::new(ProblemCode::CredentialSourceError).detail("Credentials sind nicht verfügbar")
    })?;
    match tool {
        "credential.resolve" => {
            let record = state.store.session(state.local.org, session).await?;
            service.resolve(cfg, session, &record.harness, args).await
        }
        "credential.used" => service.used(session, args).await,
        "egress.passthrough" => service.passthrough(session, args).await,
        _ => service.misuse(session, args).await,
    }
}

/// Parameter für `POST /v1/sessions`.
#[derive(Debug, Clone, Default)]
pub struct CreateSession {
    /// Harness-ID; mit `agent` optional und dann ein Override von `executor.harness`.
    pub target: String,
    /// Agent-Ref (AGT-003); bestimmt Harness, Modell, Instructions und Tools (AGT-004).
    pub agent: Option<String>,
    /// Parameterwerte des Agents (AGT-010); Texte werden typgerecht umgewandelt.
    pub params: std::collections::BTreeMap<String, Value>,
    pub cwd: String,
    pub title: Option<String>,
    pub model: Option<String>,
    /// Reasoning-Effort ab dem ersten Turn (HAR-017).
    pub effort: Option<String>,
    /// Permission-Mode ab dem Start (HAR-027); `yolo` ohne Sandbox wird abgelehnt.
    pub permission_mode: Option<String>,
    pub harness_opts: Value,
    /// Eigener Worktree (SES-015).
    pub worktree: Option<WorktreeSpec>,
    /// Session-Budget in USD (API-006 `--max-cost`, POL-011).
    pub max_cost_usd: Option<f64>,
    /// Bestätigt `sandbox.backend: none` für genau diese Session (SBX-006).
    pub confirm_unsandboxed: bool,
}

/// Audit-Aktion einer bestätigten Session ohne Sandbox (SBX-006, SEC-012).
pub const UNSANDBOXED_ACTION: &str = "sandbox.backend_none";

/// `409 sandbox_confirmation_required` (SBX-006).
fn unconfirmed_problem() -> Problem {
    Problem::new(ProblemCode::SandboxConfirmationRequired).detail(
        "`sandbox.backend: none`: Diese Session liefe ohne Sandbox. Bestätigen mit `beton run \
         --unsandboxed` (`beton resume --unsandboxed`) bzw. `confirm_unsandboxed: true`; die \
         Bestätigung gilt nur für diese Session und steht im Audit-Log.",
    )
}

/// Ergebnis der Sandbox-Vorprüfung beim Anlegen (SBX-006).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SandboxStart {
    /// Schutz der Tools für `yolo` (HAR-027).
    pub status: beton_harness::SandboxStatus,
    /// Ausdrücklich `backend: none`, bestätigt.
    pub unsandboxed: bool,
}

/// Worktree-Wunsch beim Anlegen einer Session (SES-015).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeSpec {
    /// Branch-Name; sonst `beton/<titel-slug>-<id4>`.
    pub branch: Option<String>,
    /// Base; sonst `origin/HEAD`, sonst der aktuelle Branch.
    pub base: Option<String>,
    /// `git fetch` vor dem Anlegen (Default an).
    pub fetch: bool,
}

/// Entscheidungen beim Löschen einer Session mit Worktree (SES-016).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeleteOptions {
    pub worktree: beton_git::worktree::RemoveOptions,
}

/// Fehler von `beton-git` als Problem; Git-Meldungen können Pfade enthalten und gehen nur ins
/// Log (PROTO-011).
pub(crate) fn git_problem(e: &beton_git::GitError) -> Problem {
    match e {
        beton_git::GitError::NotInstalled => {
            Problem::new(ProblemCode::Unavailable).detail("git ist nicht installiert")
        }
        other => Problem::internal(other),
    }
}

fn worktree_problem(e: beton_git::worktree::WorktreeError) -> Problem {
    use beton_git::worktree::WorktreeError as E;
    match e {
        E::NotARepo => Problem::new(ProblemCode::NotAGitRepo).detail(
            "Das Arbeitsverzeichnis ist kein Git-Repository; ein Worktree ist nicht möglich.",
        ),
        E::BaseNotFound { base, available } => {
            Problem::new(ProblemCode::BaseNotFound).detail(format!(
                "Base `{base}` ist nicht auflösbar. Verfügbare Branches: {}",
                if available.is_empty() {
                    "keine".to_owned()
                } else {
                    available.join(", ")
                }
            ))
        }
        E::InvalidBranch(b) => Problem::new(ProblemCode::ValidationFailed)
            .detail(format!("Branch-Name `{b}` ist ungültig")),
        E::Exists(_) => Problem::new(ProblemCode::Conflict)
            .detail("Für diesen Branch existiert bereits ein Worktree-Verzeichnis."),
        E::Git(g) => git_problem(&g),
        E::Io(io) => Problem::internal(&io),
    }
}

fn remove_problem(e: beton_git::worktree::RemoveError) -> Problem {
    use beton_git::worktree::RemoveError as E;
    match e {
        E::Dirty { files } => Problem::new(ProblemCode::WorktreeDirty).detail(format!(
            "Der Worktree hat {} uncommittete Änderung(en). Erneut löschen mit \
             `uncommitted=commit` (WIP-Commit) oder `uncommitted=discard` (verwerfen).",
            files.len()
        )),
        E::Unpushed {
            branch,
            base,
            commits,
        } => Problem::new(ProblemCode::WorktreeUnpushed).detail(format!(
            "Branch `{branch}` hat {commits} ungepushte Commit(s), die nicht in `{base}` \
                 enthalten sind. Erneut löschen mit `branch=keep` (Branch behalten) oder \
                 `branch=delete` (Branch löschen)."
        )),
        E::Git(g) => git_problem(&g),
    }
}

pub struct SessionManager<'a> {
    state: &'a AppState,
}

fn user_actor(by: PrincipalId) -> Actor {
    Actor::User {
        id: by,
        device_id: None,
    }
}

impl<'a> SessionManager<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }

    pub(crate) fn org(&self) -> OrgId {
        self.state.local.org
    }

    pub(crate) fn state(&self) -> &AppState {
        self.state
    }

    pub(crate) fn cfg(&self) -> &SessionsConfig {
        &self.state.runtime.sessions
    }

    /// Fake-Harness erlaubt: Entwicklermodus oder Flag `fake_harness` (HAR-026 AC3, UX-007).
    pub fn fake_allowed(&self) -> bool {
        self.cfg().dev
            || self
                .state
                .runtime
                .features
                .is_active(beton_core::feature::FeatureFlag::FakeHarness)
    }

    /// Legt eine Session an und startet ihren Runner (SES-001 AC1). Mit Worktree entsteht er
    /// vor der Session; scheitert er, gibt es keine Session (SES-015 AC3).
    pub async fn create(&self, by: UserId, req: CreateSession) -> Result<SessionRecord, Problem> {
        if !std::path::Path::new(&req.cwd).is_dir() {
            return Err(Problem::new(ProblemCode::ValidationFailed)
                .detail(format!("Arbeitsverzeichnis {} existiert nicht", req.cwd)));
        }
        // Agent-Start (AGT-004): Agent auflösen, prüfen, Parameter und Snapshot vor der Session.
        let agent = match &req.agent {
            Some(reference) => {
                let target = Some(req.target.as_str()).filter(|t| !t.is_empty());
                Some(self.resolve_agent(
                    reference,
                    std::path::Path::new(&req.cwd),
                    target,
                    req.model.as_deref(),
                    &req.params,
                )?)
            }
            None if !req.params.is_empty() => {
                return Err(Problem::new(ProblemCode::ValidationFailed)
                    .detail("`params` gibt es nur zusammen mit `agent`"));
            }
            None => None,
        };
        let harness: beton_harness::HarnessId = match &agent {
            Some(a) => a.harness.clone(),
            None => req
                .target
                .parse()
                .map_err(|e| Problem::new(ProblemCode::ValidationFailed).detail(format!("{e}")))?,
        };
        if harness.as_str() == beton_harness::HarnessId::FAKE && !self.fake_allowed() {
            // UX-007 AC1: hinter einem nicht aktivierten Flag nicht erreichbar.
            return Err(Problem::new(ProblemCode::FeatureDisabled).detail(
                "Der Fake-Harness ist nur mit --dev oder BETON_FEATURES=fake_harness verfügbar.",
            ));
        }
        if let Some(a) = &agent {
            let (spec, _) = a
                .snapshot
                .agent()
                .map_err(|e| Problem::new(ProblemCode::AgentInvalid).detail(e.to_string()))?;
            self.check_agent_harness(&spec, &harness, std::path::Path::new(&req.cwd))?;
        }
        // Effort und Permission-Mode: Anfrage vor `executor` des Agents (AGT-004). Vor dem
        // Anlegen geprüft; `yolo` ohne Sandbox nie, auch nicht aus einem Agent (HAR-027).
        let (agent_effort, agent_mode) =
            crate::settings::executor_settings(agent.as_ref().map(|a| &a.snapshot));
        if let Some(a) = &agent {
            crate::settings::agent_permission_mode(
                &a.snapshot.source,
                agent_mode.as_deref(),
                req.permission_mode.as_deref(),
            )?;
        }
        let effort = req.effort.clone().or(agent_effort);
        let permission_mode = req.permission_mode.clone().or(agent_mode);
        let caps = self.harness_capabilities(&harness, Path::new(&req.cwd));
        // SBX-006: Sandbox erforderlich, ohne nur ausdrücklich und bestätigt.
        let sandbox = self
            .sandbox_start(caps.as_ref(), req.confirm_unsandboxed)
            .await?;
        crate::settings::validate_start(
            caps.as_ref(),
            effort.as_deref(),
            permission_mode.as_deref(),
            sandbox.status,
        )?;
        let id = SessionId::new();
        let worktree = match &req.worktree {
            Some(spec) => Some(self.create_worktree(id, &req, spec).await?),
            None => None,
        };
        let created = self
            .state
            .store
            .create_session(
                self.org(),
                NewSession {
                    id,
                    owner: by,
                    kind: SessionKind::Main,
                    harness: harness.to_string(),
                    cwd: req.cwd.clone(),
                    model: match &agent {
                        Some(a) => a.model.clone(),
                        None => req.model.clone(),
                    },
                    effort: effort.clone(),
                    permission_mode: permission_mode.clone(),
                    agent_ref: req.agent.clone(),
                    project_id: None,
                    parent_id: None,
                    trigger: SessionTrigger::Api,
                    home_node: self.state.local.node,
                    harness_opts: req.harness_opts.clone(),
                    max_cost_usd: req.max_cost_usd,
                },
            )
            .await;
        let session = match created {
            Ok(s) => s,
            Err(e) => {
                if let Some(wt) = worktree {
                    discard_worktree(wt).await;
                }
                return Err(e.into());
            }
        };
        if sandbox.unsandboxed {
            self.record_unsandboxed(session.id, PrincipalId::User(by), None)
                .await?;
        }
        if let Some(a) = &agent {
            self.record_agent(session.id, &a.snapshot, &a.notices)
                .await?;
        }
        if let Some(title) = req.title.filter(|t| !t.trim().is_empty()) {
            self.append(
                session.id,
                user_actor(PrincipalId::User(by)),
                EventPayload::SessionTitleChanged(SessionTitleChanged {
                    title,
                    source: TitleSource::User,
                }),
            )
            .await?;
        }
        if let Some(wt) = &worktree {
            self.log_worktree(session.id, wt).await?;
        }
        self.launch(&session, None).await?;
        Ok(self.state.store.session(self.org(), session.id).await?)
    }

    /// Sandbox des Hosts für Clients und Vorprüfungen (SBX-006).
    pub async fn sandbox_info(&self) -> crate::sandbox_status::SandboxInfo {
        let cfg = self.cfg();
        cfg.sandbox_probe
            .info_with(
                &cfg.sandbox,
                cfg.proxy_ca.is_some(),
                cfg.sandbox_launcher.as_deref(),
            )
            .await
    }

    /// Vorprüfung der Sandbox vor dem Anlegen einer Session (SBX-006, SBX-002 AC4): Ohne
    /// nutzbares Backend `503 sandbox_unavailable`; `backend: none` nur mit Bestätigung
    /// (`409 sandbox_confirmation_required`). Liefert, was für `yolo` gilt. Maßgeblich bleibt
    /// der Runner.
    pub(crate) async fn sandbox_start(
        &self,
        caps: Option<&beton_harness::Capabilities>,
        confirmed: bool,
    ) -> Result<SandboxStart, Problem> {
        let spec = beton_sandbox::SandboxSpec::merged(&[&self.cfg().sandbox]);
        if spec.unsandboxed() {
            if !confirmed {
                return Err(unconfirmed_problem());
            }
            return Ok(SandboxStart {
                status: beton_harness::SandboxStatus::NONE,
                unsandboxed: true,
            });
        }
        let info = self.sandbox_info().await;
        if !info.active {
            return Err(info.unavailable());
        }
        let inherited =
            caps.is_some_and(|c| c.tool_isolation == beton_harness::ToolIsolation::Inherited);
        if inherited && spec.require_tool_isolation != Some(false) {
            let provider = info
                .provider
                .clone()
                .unwrap_or_else(|| info.backend.clone());
            let hint = "Der Harness führt Tools selbst aus (tool_isolation: inherited) und \
                        kann Stufe 2 nicht erzwingen; mit sandbox.require_tool_isolation: false \
                        startet die Session als degraded";
            return Err(Problem::new(ProblemCode::SandboxUnavailable)
                .detail(format!("sandbox_unavailable: {provider}: {hint}"))
                .sandbox(provider, Vec::new(), Some(hint.into())));
        }
        Ok(SandboxStart {
            status: info.status(inherited),
            unsandboxed: false,
        })
    }

    /// Bestätigtes `backend: none` (SBX-006): Audit-Eintrag (SEC-012); im Log der Session
    /// zeigt es `sandbox.started` (`backend: none`, `degraded`). `from`: Quelle bzw. Parent,
    /// deren Bestätigung ein Fork oder Sub-Agent übernimmt.
    pub(crate) async fn record_unsandboxed(
        &self,
        session: SessionId,
        by: PrincipalId,
        from: Option<SessionId>,
    ) -> Result<(), Problem> {
        self.state
            .store
            .append_audit(
                self.org(),
                beton_store::NewAuditEntry {
                    at: beton_core::time::Timestamp::now(),
                    actor: by,
                    action: UNSANDBOXED_ACTION.into(),
                    target_kind: "session".into(),
                    target_id: session.to_string(),
                    session_id: Some(session),
                    host_id: None,
                    details: match from {
                        Some(f) => json!({"confirmed": "inherited", "from_session": f}),
                        None => json!({"confirmed": "request"}),
                    },
                },
            )
            .await?;
        Ok(())
    }

    /// Hat die Session `backend: none` bestätigt (sie selbst oder geerbt)?
    pub(crate) async fn unsandboxed_confirmed(&self, session: SessionId) -> Result<bool, Problem> {
        Ok(self
            .state
            .store
            .audit_has_session_entry(self.org(), UNSANDBOXED_ACTION, session)
            .await?)
    }

    /// Legt den Worktree einer neuen Session an (SES-015).
    pub(crate) async fn create_worktree(
        &self,
        id: SessionId,
        req: &CreateSession,
        spec: &WorktreeSpec,
    ) -> Result<beton_git::worktree::Created, Problem> {
        let id_text = id.to_string();
        let tag = id_text[id_text.len().saturating_sub(4)..].to_owned();
        let cwd = PathBuf::from(&req.cwd);
        let root = self.cfg().worktrees_root.clone();
        let (title, branch, base, fetch) = (
            req.title.clone(),
            spec.branch.clone(),
            spec.base.clone(),
            spec.fetch,
        );
        tokio::task::spawn_blocking(move || {
            beton_git::worktree::create(&beton_git::worktree::CreateOptions {
                cwd: &cwd,
                root: &root,
                base: base.as_deref(),
                branch: branch.as_deref(),
                title: title.as_deref(),
                tag: &tag,
                fetch,
                fetch_timeout: beton_git::worktree::FETCH_TIMEOUT,
            })
        })
        .await
        .map_err(|e| Problem::internal(&e))?
        .map_err(worktree_problem)
    }

    /// `git.worktree_created` (SES-015 AC4) und bei nicht erreichbarem Remote ein Hinweis
    /// (SES-015 AC5). Der Grund des Fetch-Fehlers bleibt draußen: er kann die Remote-URL samt
    /// Zugangsdaten enthalten.
    pub(crate) async fn log_worktree(
        &self,
        session: SessionId,
        wt: &beton_git::worktree::Created,
    ) -> Result<(), Problem> {
        let server = Actor::System {
            component: beton_core::event::SystemComponent::Server,
        };
        self.append(
            session,
            server.clone(),
            EventPayload::GitWorktreeCreated(beton_core::event::GitWorktreeCreated {
                path: wt.path.display().to_string(),
                branch: wt.branch.clone(),
                base: wt.base.clone(),
                base_sha: wt.base_sha.clone(),
            }),
        )
        .await?;
        if let beton_git::worktree::FetchOutcome::Failed { remote, branch, .. } = &wt.fetch {
            self.append(
                session,
                server,
                EventPayload::Notice(beton_core::event::Notice {
                    level: beton_core::event::NoticeLevel::Warn,
                    text: format!(
                        "`git fetch {remote} {branch}` nicht ausgeführt: Remote nicht erreichbar. \
                         Der Worktree basiert auf dem lokalen Stand von `{}`.",
                        wt.base
                    ),
                }),
            )
            .await?;
        }
        Ok(())
    }

    /// Workspace einer Session: ihr Worktree, sonst das Arbeitsverzeichnis aus
    /// `session.created`.
    ///
    /// Eine aus einer Exportdatei importierte Session hat hier keinen Workspace (SES-009):
    /// Runner, Fork und Workspace-API scheitern dann mit `imported_read_only`.
    pub async fn workspace_root(&self, session: &SessionRecord) -> Result<PathBuf, Problem> {
        crate::exports::ensure_runnable(&self.state.store, self.org(), session.id).await?;
        if let Some(wt) = &session.worktree {
            return Ok(PathBuf::from(&wt.path));
        }
        Ok(PathBuf::from(self.created(session.id).await?.cwd))
    }

    pub(crate) async fn append(
        &self,
        session: SessionId,
        actor: Actor,
        payload: EventPayload,
    ) -> Result<(), Problem> {
        let events = self.state.events();
        for _ in 0..5 {
            let record = self.state.store.session(self.org(), session).await?;
            let e = Event::new(session, 0, actor.clone(), payload.clone());
            match events
                .append(self.org(), session, record.head_seq, record.epoch, vec![e])
                .await
            {
                Err(beton_store::Error::SeqConflict { .. }) => continue,
                other => return other.map(|_| ()).map_err(Problem::from),
            }
        }
        Err(Problem::new(ProblemCode::SeqConflict))
    }

    /// Startoptionen aus `session.created`.
    pub async fn created(
        &self,
        session: SessionId,
    ) -> Result<beton_core::event::SessionCreated, Problem> {
        let first = self.state.store.events(self.org(), session, 0, 1).await?;
        match first.first().and_then(|e| e.payload().cloned()) {
            Some(EventPayload::SessionCreated(c)) => Ok(c),
            _ => Err(Problem::internal(&"session.created fehlt")),
        }
    }

    /// Letzte native Session-Referenz des Harness (für Resume, SES-003).
    pub(crate) async fn last_native_ref(
        &self,
        session: SessionId,
    ) -> Result<Option<String>, Problem> {
        let mut after = 0;
        let mut found = None;
        loop {
            let page = self
                .state
                .store
                .events(self.org(), session, after, 500)
                .await?;
            let Some(last) = page.last() else { break };
            after = last.seq;
            for e in &page {
                if let Some(EventPayload::HarnessReady(r)) = e.payload()
                    && r.harness_session_ref.is_some()
                {
                    found.clone_from(&r.harness_session_ref);
                }
            }
        }
        Ok(found)
    }

    /// Startet einen Runner für die Session.
    pub(crate) async fn launch(
        &self,
        session: &SessionRecord,
        resume: Option<String>,
    ) -> Result<(), Problem> {
        crate::exports::ensure_runnable(&self.state.store, self.org(), session.id).await?;
        let cfg = self.cfg();
        let socket = cfg.tunnel_socket.clone().ok_or_else(|| {
            Problem::new(ProblemCode::Unavailable).detail("Kein Tunnel-Socket konfiguriert")
        })?;
        let created = self.created(session.id).await?;
        let workspace = self.workspace_root(session).await?;
        let project = beton_harness::registry::HarnessesConfig::load_project(&workspace)
            .map_err(|e| Problem::new(ProblemCode::ValidationFailed).detail(e))?;
        // HAR-020 AC3: Was der vorige Runner offen ließ, endet jetzt im Log.
        self.close_interrupted(session.id).await?;
        // Modell, Effort und Mode laut letztem Stand, nicht nur `session.created` (HAR-020).
        let settings = crate::settings::current(&self.all_events(session.id).await?);
        let fork = self.fork_boot(session, resume.as_deref()).await?;
        let agent_snapshot = self.snapshot_file(session.id).await?;
        let (sandbox, proxy_ca) = stage_boot_for(
            cfg,
            session.id,
            session.harness != beton_harness::HarnessId::FAKE,
        )
        .await?;
        // Die Bestätigung von `backend: none` steht im Audit-Log und gilt für die ganze
        // Session, auch nach Resume (SBX-006).
        let unsandboxed_confirmed = self
            .state
            .store
            .audit_has_session_entry(self.org(), UNSANDBOXED_ACTION, session.id)
            .await?;
        if !beton_host::stages_active(&beton_sandbox::SandboxSpec::merged(&[&cfg.sandbox]))
            && !unsandboxed_confirmed
        {
            // Z. B. eine importierte Session: erst mit Bestätigung (Resume mit
            // `confirm_unsandboxed`), nie still ohne Sandbox (SBX-006).
            return Err(unconfirmed_problem());
        }
        let token = self
            .state
            .runtime
            .runners
            .mint_token(session.id)
            .map_err(|e| Problem::internal(&e))?;
        let boot_token = token.clone();
        let spec = RunnerSpec {
            runner_id: RunnerId::new(),
            session_id: session.id,
            harness: session.harness.clone(),
            workspace,
            // `sandbox.env.passthrough` (SBX-005): Der Runner erbt nur diese Variablen, damit
            // die Auflösung sie in die Sandbox übernehmen kann.
            env_allowlist: if sandbox.is_some() {
                passthrough_names(&cfg.sandbox, &cfg.daemon_env)
            } else {
                Vec::new()
            },
            // API-Keys für `direct:*` (HAR-011) gehen über stdin, nie in die Runner-Umgebung.
            // Im Stufenmodell nicht für https-Provider: Dort sieht der Loop nur einen
            // Platzhalter, den der Proxy ersetzt (HAR-011 AC2, PRX-006).
            secret_env: if sandbox.is_some() && direct_via_proxy(&session.harness, &cfg.providers) {
                Vec::new()
            } else {
                beton_harness_direct::key_vars(&session.harness, &cfg.providers)
            },
        };
        let provisioned = cfg
            .provider
            .provision(&spec)
            .await
            .map_err(|e| Problem::new(ProblemCode::Unavailable).detail(e.to_string()))?;
        let handle = cfg
            .provider
            .start(
                &provisioned,
                &RunnerBoot {
                    tunnel_socket: socket,
                    token: boot_token,
                    session_id: session.id,
                    epoch: session.epoch,
                    harness: session.harness.clone(),
                    scenario: created.harness_opts["scenario"].as_str().map(PathBuf::from),
                    model: settings.model,
                    effort: settings.effort,
                    permission_mode: settings.permission_mode,
                    dev: self.fake_allowed(),
                    resume,
                    agent_ref: created.agent_ref.clone(),
                    agent_snapshot,
                    harnesses: beton_harness::registry::HarnessLayers {
                        user: cfg.harnesses_user.clone(),
                        project,
                        user_file: None,
                        project_file: Some(
                            Path::new(&created.cwd).join(".beton").join("config.yaml"),
                        ),
                        providers: cfg.providers.clone(),
                    },
                    snapshots: Some(cfg.snapshots_dir(session.id)),
                    fork,
                    sandbox,
                    unsandboxed_confirmed,
                    proxy_ca,
                    compaction: self.state.runtime.compaction,
                    max_cost_usd: created.max_cost_usd,
                    spawned: created.trigger == SessionTrigger::Spawn,
                },
            )
            .await
            .map_err(|e| Problem::new(ProblemCode::Unavailable).detail(e.to_string()))?;
        cfg.launched.handles.lock().await.insert(session.id, handle);
        cfg.launched
            .tokens
            .lock()
            .await
            .insert(session.id, (token, Instant::now()));
        Ok(())
    }

    pub(crate) async fn wait_connected(&self, session: SessionId) -> Result<(), Problem> {
        let start = Instant::now();
        while !self.state.runtime.runners.connected(session) {
            if start.elapsed() > self.cfg().connect_timeout {
                // Der Runner startet weiter (Harness-Handshake, MCP-Server); der nächste
                // Versuch wartet auf ihn, ersetzt wird er erst nach dem Start-Budget.
                return Err(
                    Problem::new(ProblemCode::Unavailable).detail("Runner meldet sich nicht")
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }

    /// Räumt den vorigen Start vor einem neuen ab: Token sperren, Prozess beenden (falls er nach
    /// dem Start-Budget noch läuft). Nur unter der Start-Sperre der Session aufrufen; dann gehört
    /// der Eintrag sicher zur Generation vor dem neuen Start.
    async fn replace_start(&self, session: SessionId) {
        if let Some((token, _)) = self.cfg().launched.tokens.lock().await.remove(&session) {
            self.state.runtime.runners.revoke_token(&token);
        }
        if let Some(h) = self.cfg().launched.handles.lock().await.remove(&session) {
            let _ = self
                .cfg()
                .provider
                .terminate(&h, TerminateMode::Force)
                .await;
        }
    }

    /// Wie [`Self::resume`]; `confirm_unsandboxed` bestätigt `backend: none` für eine Session,
    /// die noch keine Bestätigung hat (z. B. importiert, SBX-006).
    pub async fn resume_confirmed(
        &self,
        session: SessionId,
        by: PrincipalId,
        confirm_unsandboxed: bool,
    ) -> Result<SessionRecord, Problem> {
        if confirm_unsandboxed
            && beton_sandbox::SandboxSpec::merged(&[&self.cfg().sandbox]).unsandboxed()
            && !self.unsandboxed_confirmed(session).await?
        {
            self.state.store.session(self.org(), session).await?;
            self.record_unsandboxed(session, by, None).await?;
        }
        self.resume(session).await
    }

    /// Setzt eine gestoppte Session mit neuem Runner fort (SES-003).
    pub async fn resume(&self, session: SessionId) -> Result<SessionRecord, Problem> {
        let record = self.state.store.session(self.org(), session).await?;
        {
            let starts = self.cfg().launched.start_lock(session);
            let _start = starts.lock().await;
            if !self.await_adopted(session).await && !self.runner_starting(session).await {
                self.replace_start(session).await;
                let reference = self.last_native_ref(session).await?;
                self.launch(&record, reference).await?;
            }
        }
        self.wait_connected(session).await?;
        Ok(self.state.store.session(self.org(), session).await?)
    }

    /// Wartet auf einen abgekoppelten Runner eines vorigen Daemons, der sich neu verbinden
    /// darf (POL-010 AC4), höchstens [`CONNECT_TIMEOUT`]. `true`, wenn er verbunden ist.
    pub(crate) async fn await_adopted(&self, session: SessionId) -> bool {
        let runners = &self.state.runtime.runners;
        let start = Instant::now();
        while runners.expected(session) && start.elapsed() < CONNECT_TIMEOUT {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if !runners.connected(session) {
            runners.forget_expected(session);
            return false;
        }
        true
    }

    /// Läuft (oder startet gerade) ein Runner für die Session?
    async fn runner_alive(&self, session: SessionId) -> bool {
        self.await_adopted(session).await || self.runner_starting(session).await
    }

    /// Ist der zuletzt gestartete Runner noch auf dem Weg zum Tunnel? Sein Prozess läuft, sein
    /// Token gilt, und das Start-Budget ist nicht abgelaufen. Nach einem geordneten Stopp ist
    /// das Token gesperrt: Ein Prozess, der sich nur noch beendet, meldet sich nicht wieder
    /// (#198).
    async fn runner_starting(&self, session: SessionId) -> bool {
        let launched = self
            .cfg()
            .launched
            .tokens
            .lock()
            .await
            .get(&session)
            .cloned();
        let budget = self.cfg().start_budget;
        if !launched.is_some_and(|(t, at)| {
            at.elapsed() < budget && self.state.runtime.runners.token_valid(&t)
        }) {
            return false;
        }
        let handle = self
            .cfg()
            .launched
            .handles
            .lock()
            .await
            .get(&session)
            .cloned();
        match &handle {
            Some(h) => matches!(
                self.cfg().provider.status(h).await,
                Ok(beton_host::RunnerStatus::Running)
            ),
            None => false,
        }
    }

    /// Startet bzw. verbindet den Runner, falls nötig (SES-003 AC3).
    pub(crate) async fn ensure_runner(&self, session: SessionId) -> Result<(), Problem> {
        if self.await_adopted(session).await {
            return Ok(());
        }
        // Ein startender Runner verbindet sich gleich; ein beendeter, geordnet gestoppter oder
        // über das Start-Budget hängender wird unter der Start-Sperre ersetzt.
        self.resume(session).await.map(|_| ())
    }

    /// Kann der Harness der Session Eingaben in den laufenden Turn nehmen? Laut den
    /// Capabilities aus dem letzten `session.started` (HAR-002).
    async fn can_steer(&self, session: SessionId) -> Result<bool, Problem> {
        self.capability(session, "steering").await
    }

    /// Boolesche Capability laut letztem `session.started` (HAR-002).
    async fn capability(&self, session: SessionId, name: &str) -> Result<bool, Problem> {
        let started = self
            .state
            .store
            .last_event_of_type(self.org(), session, "session.started")
            .await?;
        Ok(match started.as_ref().and_then(Event::payload) {
            Some(EventPayload::SessionStarted(s)) => s.capabilities[name] == true,
            _ => false,
        })
    }

    /// Eingabe zustellen (SES-004): ohne laufenden Turn sofort als neuer Turn, sonst in die
    /// Queue bzw. mit `steer` in den laufenden Turn. Eine gestoppte Session wird dafür
    /// fortgesetzt (SES-003 AC3).
    pub async fn input(
        &self,
        session: SessionId,
        text: String,
        by: PrincipalId,
        mode: InputMode,
    ) -> Result<Value, Problem> {
        self.input_as(session, text, Vec::new(), by, mode, user_actor(by))
            .await
    }

    /// Eingabe mit Anhängen (WEB-006).
    pub async fn input_with(
        &self,
        session: SessionId,
        text: String,
        attachments: Vec<Attachment>,
        by: PrincipalId,
        mode: InputMode,
    ) -> Result<Value, Problem> {
        self.input_as(session, text, attachments, by, mode, user_actor(by))
            .await
    }

    /// Wie [`Self::input`], mit eigenem Akteur der Nachricht (z. B. der Parent-Agent).
    pub(crate) async fn input_as(
        &self,
        session: SessionId,
        text: String,
        attachments: Vec<Attachment>,
        by: PrincipalId,
        mode: InputMode,
        actor: Actor,
    ) -> Result<Value, Problem> {
        if text.trim().is_empty() && attachments.is_empty() {
            return Err(Problem::new(ProblemCode::ValidationFailed).detail("text ist leer"));
        }
        let record = self.state.store.session(self.org(), session).await?;
        if record.archived {
            return Err(Problem::new(ProblemCode::Conflict)
                .detail("Archivierte Session; erst wiederherstellen"));
        }
        crate::exports::ensure_runnable(&self.state.store, self.org(), session).await?;
        if !attachments.is_empty() {
            let images = self.capability(session, "images").await?;
            crate::attachments::validate(self.state, session, &attachments, images).await?;
        }
        let mut q = self.state.queue().lock(session).await?;
        if q.busy() && !self.runner_alive(session).await {
            // Der Runner endete mitten im Turn (Absturz, Neustart): Turn und Freigaben im Log
            // abschließen, dann normal weiter (HAR-020 AC3).
            self.close_interrupted(session).await?;
        }
        if mode == InputMode::Steer && q.busy() && !attachments.is_empty() {
            return Err(Problem::new(ProblemCode::ValidationFailed).detail(
                "Anhänge gehen nur mit einer neuen Nachricht, nicht in den laufenden Turn",
            ));
        }
        if mode == InputMode::Steer && q.busy() {
            // SES-004 AC4: ohne Capability `steering` lehnt die API ab.
            if !self.can_steer(session).await? {
                return Err(Problem::new(ProblemCode::CapabilityUnsupported).detail(
                    "Dieser Harness nimmt keine Eingaben in den laufenden Turn (steering)",
                ));
            }
            if q.steer(&text, by).await? {
                return Ok(Accepted::Steered {
                    input_id: InputId::new(),
                }
                .to_json());
            }
        }
        if q.busy() || !q.is_empty() {
            let input_id = q.push(text, attachments, by).await?;
            if !q.busy() {
                self.ensure_runner(session).await?;
                q.drain().await?;
            }
            return Ok(Accepted::Queued { input_id }.to_json());
        }
        self.ensure_runner(session).await?;
        Ok(q.start_turn(InputId::new(), text, attachments, by, actor)
            .await?
            .to_json())
    }

    /// Einen eingereihten Input in den laufenden Turn einspeisen („Als Steer senden“).
    pub async fn steer_item(
        &self,
        session: SessionId,
        item: &str,
        by: PrincipalId,
    ) -> Result<Value, Problem> {
        if !self.can_steer(session).await? {
            return Err(Problem::new(ProblemCode::CapabilityUnsupported)
                .detail("Dieser Harness nimmt keine Eingaben in den laufenden Turn (steering)"));
        }
        let mut q = self.state.queue().lock(session).await?;
        if !q.busy() {
            return Err(Problem::new(ProblemCode::NoActiveTurn)
                .detail("Kein laufender Turn; der Eintrag läuft als Nächstes"));
        }
        let taken = q.take(item, by).await?;
        match q.steer(&taken.text, by).await {
            Ok(true) => Ok(Accepted::Steered {
                input_id: taken.id.parse().unwrap_or_else(|_| InputId::new()),
            }
            .to_json()),
            // Turn inzwischen vorbei bzw. Fehler: Der Eintrag kommt wieder nach vorn.
            other => {
                let input_id = taken.id.parse().unwrap_or_else(|_| InputId::new());
                q.put_front(taken, by).await?;
                other?;
                q.drain().await?;
                Ok(Accepted::Queued { input_id }.to_json())
            }
        }
    }

    /// Pausierte Queue fortsetzen (`queue.resume`, SES-005 AC2).
    pub async fn resume_queue(&self, session: SessionId, by: PrincipalId) -> Result<(), Problem> {
        self.state.store.session(self.org(), session).await?;
        let mut q = self.state.queue().lock(session).await?;
        if !q.busy() && !q.is_empty() {
            self.ensure_runner(session).await?;
        }
        q.resume(by).await
    }

    /// Bricht den laufenden Turn ab; ohne Turn bzw. Runner ein No-op (SES-005 AC3). Laufende
    /// Sub-Agents enden mit (`parent_cancelled`, AGT-009 AC5).
    pub async fn interrupt(&self, session: SessionId) -> Result<(), Problem> {
        self.state.store.session(self.org(), session).await?;
        self.cancel_children(session).await;
        if !self.state.runtime.runners.connected(session) {
            return Ok(());
        }
        self.state
            .runtime
            .runners
            .deliver(
                session,
                "turn.interrupt",
                Value::Null,
                self.state.runtime.tunnel.cmd_timeout,
            )
            .await
            .map(|_| ())
    }

    /// Kann der Harness der Session kompaktieren (Capability `compaction`, HAR-022)? Maßgeblich
    /// sind die Capabilities aus `session.started`, sonst die des Katalogs.
    async fn can_compact(&self, session: SessionId, harness: &str) -> Result<bool, Problem> {
        let started = self
            .state
            .store
            .last_event_of_type(self.org(), session, "session.started")
            .await?;
        if let Some(EventPayload::SessionStarted(s)) = started.as_ref().and_then(Event::payload) {
            return Ok(s.capabilities["compaction"]
                .as_str()
                .is_some_and(|c| c != "none"));
        }
        let Ok(id) = harness.parse::<beton_harness::HarnessId>() else {
            return Ok(false);
        };
        Ok(self.state.runtime.harnesses.get(&id).is_some_and(|a| {
            a.capabilities(
                beton_harness::Mode::Native,
                &beton_harness::ProbeReport::default(),
            )
            .check(beton_harness::Action::Compact)
            .is_ok()
        }))
    }

    /// Compaction anstoßen (SES-011): ohne Capability `compaction` 409
    /// `capability_unsupported` (HAR-022 AC2), ohne laufenden Runner oder während eines Turns
    /// 409 `conflict`. Die Events folgen asynchron vom Harness.
    pub async fn compact(&self, session: SessionId) -> Result<(), Problem> {
        let record = self.state.store.session(self.org(), session).await?;
        if !self.can_compact(session, &record.harness).await? {
            return Err(Problem::new(ProblemCode::CapabilityUnsupported)
                .detail("Dieser Harness kann nicht kompaktieren (compaction: none)"));
        }
        if !self.state.runtime.runners.connected(session) {
            return Err(
                Problem::new(ProblemCode::Conflict).detail("Session läuft nicht; erst fortsetzen")
            );
        }
        if self.state.queue().lock(session).await?.busy() {
            return Err(Problem::new(ProblemCode::Conflict)
                .detail("Ein Turn läuft; Compaction erst danach"));
        }
        self.state
            .runtime
            .runners
            .deliver(
                session,
                "session.compact",
                Value::Null,
                self.state.runtime.tunnel.cmd_timeout,
            )
            .await
            .map(|_| ())
    }

    /// Einstellungen ändern, z. B. Modell (HAR-002 AC3: ohne Capability `capability_unsupported`).
    pub async fn set(&self, session: SessionId, args: Value) -> Result<Value, Problem> {
        self.state.store.session(self.org(), session).await?;
        // Ungültige Werte und `yolo` ohne Sandbox erreichen den Runner nicht (HAR-027).
        crate::settings::precheck_switch(&args)?;
        if !self.state.runtime.runners.connected(session) {
            return Err(
                Problem::new(ProblemCode::Conflict).detail("Session läuft nicht; erst fortsetzen")
            );
        }
        self.state
            .runtime
            .runners
            .deliver(
                session,
                "session.set",
                args,
                self.state.runtime.tunnel.cmd_timeout,
            )
            .await
    }

    /// Entscheidung für eine offene Freigabe an den Runner (WEB-007, WEB-018, HAR-005).
    ///
    /// Geprüft wird gegen die offene Card (POL-010): `allow`/`deny` für Freigaben,
    /// `answer` (mit Text) oder `deny` für Fragen (`ask_user`); `remember: session` nur, wenn die
    /// Card `approve_session` anbietet. Der Runner erfährt, wer entschieden hat (`actor`).
    pub async fn resolve_approval(
        &self,
        session: SessionId,
        approval: &str,
        args: Value,
        by: PrincipalId,
    ) -> Result<(), Problem> {
        let open = self.state.store.open_approvals(self.org()).await?;
        let found = open
            .iter()
            .find(|a| a.session_id == session && a.id.to_string() == approval)
            .ok_or_else(|| {
                Problem::new(ProblemCode::NotFound).detail(format!("Freigabe {approval}"))
            })?;
        let invalid = |m: &str| Problem::new(ProblemCode::ValidationFailed).detail(m.to_owned());
        let question = found.kind == beton_core::event::ApprovalKind::Question;
        let decision = args["decision"].as_str().unwrap_or_default();
        match (decision, question) {
            ("allow" | "deny", false) | ("answer" | "deny", true) => {}
            ("answer", false) => return Err(invalid("`answer` gibt es nur für Fragen")),
            ("allow", true) => return Err(invalid("Fragen beantwortest du mit `answer`")),
            _ => return Err(invalid("`decision` muss allow, deny oder answer sein")),
        }
        if decision == "answer" {
            let answer = args["answer"].as_str().map(str::trim).unwrap_or_default();
            if answer.is_empty() {
                return Err(invalid("`answer` fehlt"));
            }
            if answer.chars().count() > 4096 {
                return Err(invalid("`answer` ist länger als 4096 Zeichen"));
            }
        }
        match args.get("remember").and_then(Value::as_str) {
            None | Some("none") => {}
            Some("session") if decision != "allow" => {
                return Err(invalid("`remember: session` gibt es nur mit `allow`"));
            }
            Some("session") => {
                if !found.options.iter().any(|o| o == "approve_session") {
                    return Err(Problem::new(ProblemCode::Conflict).detail(
                        "„Für diese Session erlauben“ bietet diese Freigabe nicht an".to_owned(),
                    ));
                }
            }
            Some(_) => return Err(invalid("`remember` muss none oder session sein")),
        }
        let call_id = found.subject["call_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        // Nach einem Neustart des Daemons verbindet sich der wartende Runner neu (POL-010 AC4).
        self.await_adopted(session).await;
        let mut payload = args;
        payload["call_id"] = Value::String(call_id);
        payload["actor"] = serde_json::to_value(user_actor(by)).unwrap_or(Value::Null);
        self.state
            .runtime
            .runners
            .deliver(
                session,
                "approval.resolve",
                payload,
                self.state.runtime.tunnel.cmd_timeout,
            )
            .await
            .map(|_| ())
    }

    /// Archivieren: Runner beenden, Session ausblenden (SES-001 AC2).
    /// Manuelle Nachredaktion (DATA-012): nur der Owner (lokal immer; Rollen ab M4). Das
    /// `event.redacted` geht an verbundene Clients, der Audit-Eintrag nennt Pfade und Grund.
    pub async fn redact_event(
        &self,
        session: SessionId,
        seq: u64,
        by: PrincipalId,
        paths: Vec<String>,
        reason: String,
    ) -> Result<beton_store::ManualRedactionOutcome, Problem> {
        let record = self.state().store.session(self.org(), session).await?;
        if PrincipalId::User(record.owner) != by {
            return Err(Problem::new(ProblemCode::Forbidden)
                .detail("Nur der Owner der Session darf Events schwärzen"));
        }
        let reason = reason.trim().to_owned();
        if reason.is_empty() || reason.chars().count() > 500 {
            return Err(Problem::new(ProblemCode::ValidationFailed)
                .detail("`reason` ist Pflicht (1 bis 500 Zeichen)"));
        }
        let mut details = serde_json::json!({
            "paths": paths,
            "whole_payload": paths.is_empty(),
            "reason": reason,
        });
        // Auch der Grund läuft durch die Pipeline, bevor er ins Audit-Log geht.
        self.state().store.redact_value(&mut details)?;
        let event = Event::new(
            session,
            0,
            user_actor(by),
            EventPayload::EventRedacted(beton_core::event::EventRedacted {
                target_seq: seq,
                by,
                reason,
            }),
        );
        let outcome = self
            .state()
            .store
            .redact_event(
                self.org(),
                session,
                beton_store::ManualRedaction {
                    seq,
                    paths,
                    record: event,
                    audit: beton_store::NewAuditEntry {
                        at: beton_core::time::Timestamp::now(),
                        actor: by,
                        action: "event.redacted".into(),
                        target_kind: "event".into(),
                        target_id: format!("{session}/{seq}"),
                        session_id: Some(session),
                        host_id: None,
                        details,
                    },
                },
            )
            .await?;
        self.state()
            .runtime
            .hub
            .publish_durable(session, std::slice::from_ref(&outcome.record));
        Ok(outcome)
    }

    pub async fn archive(&self, session: SessionId, by: PrincipalId) -> Result<(), Problem> {
        self.append(
            session,
            user_actor(by),
            EventPayload::SessionArchived(Empty {}),
        )
        .await?;
        self.stop(session, "archiviert").await;
        Ok(())
    }

    /// Titel ändern (CLI-003 `session rename`).
    pub async fn rename(
        &self,
        session: SessionId,
        title: &str,
        by: PrincipalId,
    ) -> Result<(), Problem> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Problem::new(ProblemCode::ValidationFailed).detail("Titel ist leer"));
        }
        self.append(
            session,
            user_actor(by),
            EventPayload::SessionTitleChanged(SessionTitleChanged {
                title: title.to_owned(),
                source: TitleSource::User,
            }),
        )
        .await
    }

    pub async fn unarchive(&self, session: SessionId, by: PrincipalId) -> Result<(), Problem> {
        self.append(
            session,
            user_actor(by),
            EventPayload::SessionUnarchived(Empty {}),
        )
        .await
    }

    /// Runner geordnet stoppen; die Session wird danach `stopped`. Laufende Sub-Agents enden
    /// mit (`parent_cancelled`, AGT-009 AC5).
    pub async fn stop(&self, session: SessionId, reason: &str) {
        self.cancel_children(session).await;
        let runners = &self.state.runtime.runners;
        if runners.stop_with_reason(session, 10, reason) {
            let start = Instant::now();
            while runners.connected(session) && start.elapsed() < Duration::from_secs(15) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        // Auch ein Runner, der sich noch nicht gemeldet hat, kommt danach nicht mehr in den
        // Tunnel (Security-Review zu #198).
        if let Some((token, _)) = self.cfg().launched.tokens.lock().await.remove(&session) {
            runners.revoke_token(&token);
        }
        if let Some(h) = self.cfg().launched.handles.lock().await.remove(&session) {
            let _ = self
                .cfg()
                .provider
                .terminate(
                    &h,
                    TerminateMode::Graceful {
                        grace: Duration::from_secs(5),
                    },
                )
                .await;
        }
    }

    /// Löschen nur durch den Owner (SES-001 AC3, DATA-008). Ein Worktree wird vorher
    /// kontrolliert entfernt (SES-016); verlangt das eine Rückfrage, bleibt alles unverändert.
    pub async fn delete(
        &self,
        session: SessionId,
        by: PrincipalId,
        opts: DeleteOptions,
    ) -> Result<(), Problem> {
        let record = self.state.store.session(self.org(), session).await?;
        if PrincipalId::User(record.owner) != by {
            return Err(Problem::new(ProblemCode::Forbidden).detail("Nur der Owner darf löschen"));
        }
        // Mit der Session verschwinden ihre Sub-Agents; laufende vorher beenden (AGT-009 AC5).
        self.cancel_children(session).await;
        if let Some(wt) = &record.worktree {
            let cwd = self.created(session).await?.cwd;
            let wt_ref = beton_git::worktree::WorktreeRef {
                repo: PathBuf::new(),
                path: PathBuf::from(&wt.path),
                branch: wt.branch.clone(),
                base: wt.base.clone(),
                base_sha: wt.base_sha.clone(),
            };
            let remove = opts.worktree;
            tokio::task::spawn_blocking(move || {
                let repo = beton_git::worktree::main_checkout(&wt_ref.path)
                    .or_else(|_| beton_git::worktree::main_checkout(Path::new(&cwd)))
                    .map_err(worktree_problem)?;
                beton_git::worktree::remove(
                    &beton_git::worktree::WorktreeRef { repo, ..wt_ref },
                    remove,
                )
                .map_err(remove_problem)
            })
            .await
            .map_err(|e| Problem::internal(&e))??;
        }
        self.state.runtime.runners.revoke_tokens(session);
        self.cfg().launched.tokens.lock().await.remove(&session);
        if let Some(h) = self.cfg().launched.handles.lock().await.remove(&session) {
            let _ = self
                .cfg()
                .provider
                .terminate(&h, TerminateMode::Force)
                .await;
        }
        self.state
            .store
            .delete_session(self.org(), session, by, DeleteAuthority::Owner)
            .await?;
        self.state.runtime.queues.forget(session);
        let _ = std::fs::remove_file(self.cfg().fork_plan(session));
        let _ = std::fs::remove_file(self.cfg().agent_snapshot(session));
        let fresh = self.cfg().fresh_dir(session);
        if fresh.is_dir()
            && let Err(e) = std::fs::remove_dir_all(&fresh)
        {
            tracing::warn!(session_id = %session, "Fork-Workspace nicht gelöscht: {e}");
        }
        let snapshots = self.cfg().snapshots_dir(session);
        if snapshots.is_dir()
            && let Err(e) = std::fs::remove_dir_all(&snapshots)
        {
            tracing::warn!(session_id = %session, "Turn-Snapshots nicht gelöscht: {e}");
        }
        Ok(())
    }
}

/// Entfernt einen frisch angelegten Worktree wieder (Session konnte nicht angelegt werden).
pub(crate) async fn discard_worktree(wt: beton_git::worktree::Created) {
    let _ = tokio::task::spawn_blocking(move || {
        beton_git::worktree::remove(
            &beton_git::worktree::WorktreeRef {
                repo: wt.repo,
                path: wt.path,
                branch: wt.branch,
                base: wt.base,
                base_sha: wt.base_sha,
            },
            beton_git::worktree::RemoveOptions {
                uncommitted: Some(beton_git::worktree::Uncommitted::Discard),
                branch: Some(beton_git::worktree::BranchAction::Delete),
            },
        )
    })
    .await;
}

/// Beim Herunterfahren alle Runner dieses Knotens beenden.
pub async fn shutdown_all(cfg: &SessionsConfig, runners: &crate::tunnel::RunnerRegistry) {
    let handles: Vec<(SessionId, RunnerHandle)> =
        cfg.launched.handles.lock().await.drain().collect();
    cfg.launched.tokens.lock().await.clear();
    // Runner mit offener Freigabe überdauern den Daemon (POL-010 AC4): abkoppeln statt beenden;
    // der nächste Daemon nimmt ihr Token bis zum Ablauf der Freigabe an.
    let mut candidates: Vec<SessionId> = runners.connected_sessions();
    candidates.extend(handles.iter().map(|(s, _)| *s));
    candidates.sort();
    candidates.dedup();
    let waiting: Vec<SessionId> = candidates
        .iter()
        .copied()
        .filter(|s| runners.connected(*s) && runners.has_open_approvals(*s))
        .collect();
    let detached = runners.detach(&waiting);
    if let Some(path) = cfg
        .tunnel_socket
        .as_deref()
        .and_then(crate::detached::path_for)
        && let Err(e) = crate::detached::save(&path, &detached)
    {
        tracing::warn!("Abgekoppelte Runner nicht gespeichert: {e}");
    }
    let kept: Vec<SessionId> = detached.iter().map(|d| d.session_id).collect();
    for (session, h) in &handles {
        if kept.contains(session) {
            tracing::info!(%session, "Runner wartet auf eine Freigabe und bleibt bestehen");
            let _ = cfg.provider.release(h).await;
            continue;
        }
        runners.revoke_tokens(*session);
        let _ = cfg
            .provider
            .terminate(
                h,
                TerminateMode::Graceful {
                    grace: Duration::from_secs(5),
                },
            )
            .await;
    }
    // Übernommene Runner ohne Handle dieses Daemons: über den Tunnel beenden.
    for session in runners.connected_sessions() {
        if !kept.contains(&session) && !handles.iter().any(|(s, _)| *s == session) {
            runners.stop(session, 5);
        }
    }
}

/// Wartung der Session-Worktrees (SES-016): beim Start und danach täglich `git worktree prune`
/// in allen Repositories mit Session-Worktrees und Meldung verwaister Worktree-Verzeichnisse.
pub async fn worktree_janitor(
    store: beton_store::Store,
    org: OrgId,
    cfg: SessionsConfig,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
    loop {
        if let Err(e) = tidy_worktrees(&store, org, &cfg).await {
            tracing::warn!("Worktree-Wartung: {e}");
        }
        tokio::select! {
            () = tokio::time::sleep(EVERY) => {}
            _ = stop.wait_for(|s| *s) => return,
        }
    }
}

async fn tidy_worktrees(
    store: &beton_store::Store,
    org: OrgId,
    cfg: &SessionsConfig,
) -> Result<(), beton_store::Error> {
    let mut known = Vec::new();
    let mut repos = Vec::new();
    for s in store.sessions(org, true).await? {
        let Some(wt) = &s.worktree else { continue };
        known.push(PathBuf::from(&wt.path));
        let cwd = match store
            .events(org, s.id, 0, 1)
            .await?
            .first()
            .and_then(|e| e.payload())
        {
            Some(EventPayload::SessionCreated(c)) => Some(PathBuf::from(&c.cwd)),
            _ => None,
        };
        repos.push((PathBuf::from(&wt.path), cwd));
    }
    let root = cfg.worktrees_root.clone();
    let orphans = tokio::task::spawn_blocking(move || {
        let mut done = std::collections::HashSet::new();
        for (path, cwd) in repos {
            let repo = beton_git::worktree::main_checkout(&path)
                .ok()
                .or_else(|| cwd.and_then(|c| beton_git::worktree::main_checkout(&c).ok()));
            if let Some(repo) = repo
                && done.insert(repo.clone())
                && let Err(e) = beton_git::worktree::prune(&repo)
            {
                tracing::debug!("git worktree prune: {e}");
            }
        }
        beton_git::worktree::orphans(&root, &known).unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    if !orphans.is_empty() {
        tracing::warn!(
            count = orphans.len(),
            "verwaiste Worktree-Verzeichnisse ohne Session; `beton doctor` listet sie"
        );
    }
    Ok(())
}

#[cfg(test)]
mod stage_tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn sbx_005_runner_inherits_only_configured_passthrough_variables() {
        let spec = beton_sandbox::SandboxSpec::from_yaml(
            "env:\n  passthrough: [RUST_LOG, \"LC_*\", NODE_OPTIONS]\n",
        )
        .unwrap();
        let env: std::collections::BTreeMap<String, String> = [
            ("RUST_LOG", "debug"),
            ("LC_ALL", "C"),
            ("LC_CTYPE", "C"),
            ("AWS_SECRET_ACCESS_KEY", "FIXTURE"),
            ("SSH_AUTH_SOCK", "/tmp/agent"),
            ("BETON_TOKEN", "x"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        assert_eq!(
            passthrough_names(&spec, &env),
            ["LC_ALL", "LC_CTYPE", "RUST_LOG"]
        );
        assert!(passthrough_names(&beton_sandbox::SandboxSpec::default(), &env).is_empty());
    }

    #[test]
    fn har_011_ac2_only_https_providers_with_key_go_via_the_proxy() {
        let providers: std::collections::BTreeMap<String, Value> = [
            ("or", json!({"kind": "openai", "base_url": "https://openrouter.ai/api/v1", "api_key_env": "OR_KEY"})),
            ("local", json!({"kind": "openai", "base_url": "http://127.0.0.1:4000/v1", "api_key_env": "L_KEY"})),
            ("ollama", json!({"kind": "openai", "base_url": "http://127.0.0.1:11434/v1", "api_key": "none"})),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        assert!(direct_via_proxy("direct:or", &providers));
        assert!(!direct_via_proxy("direct:local", &providers));
        assert!(!direct_via_proxy("direct:ollama", &providers));
        assert!(!direct_via_proxy("claude", &providers));
    }

    struct FixedCa(Result<(), String>);

    #[async_trait::async_trait]
    impl ProxyCaSource for FixedCa {
        async fn handover(&self, _: SessionId) -> Result<beton_proxy::CaHandover, String> {
            self.0.clone()?;
            beton_proxy::BetonCa::generate("stagetest", time::OffsetDateTime::now_utc())
                .unwrap()
                .handover()
                .map_err(|e| e.to_string())
        }
    }

    fn cfg(yaml: &str, ca: Option<Result<(), String>>) -> SessionsConfig {
        let tmp = std::env::temp_dir();
        SessionsConfig {
            provider: Arc::new(beton_host::LocalProvider::new(Vec::new(), tmp.clone())),
            tunnel_socket: None,
            dev: false,
            launched: Arc::default(),
            harnesses_user: Default::default(),
            providers: Default::default(),
            worktrees_root: tmp.clone(),
            snapshots_root: tmp.clone(),
            forks_root: tmp.clone(),
            fresh_root: tmp.clone(),
            agent_snapshots_root: tmp.clone(),
            beton_home: tmp,
            vendor_env: beton_harness::HostEnv::default(),
            import_lock: Arc::default(),
            skill_paths: crate::session_skills::SkillPaths::from_process(),
            sandbox: beton_sandbox::SandboxSpec::from_yaml(yaml).unwrap(),
            sandbox_probe: Arc::default(),
            sandbox_launcher: None,
            proxy_extra_ca_files: vec![PathBuf::from("/etc/firmen-ca.pem")],
            proxy_ca: ca.map(|r| Arc::new(FixedCa(r)) as Arc<dyn ProxyCaSource>),
            credentials: None,
            secret_tester: None,
            daemon_env: Arc::default(),
            connect_timeout: CONNECT_TIMEOUT,
            start_budget: START_BUDGET,
        }
    }

    #[tokio::test]
    async fn sbx_006_runner_gets_stage_config_by_default_and_backend_none_explicitly() {
        // SBX-006: Ohne `backend` gilt „Sandbox erforderlich“ (mit CA für den Proxy).
        let (sb, ca) = stage_boot(&cfg("{}\n", Some(Ok(()))), SessionId::new())
            .await
            .unwrap();
        assert!(sb.is_some() && ca.unwrap().ca().is_ok());
        // `backend: none` geht ausdrücklich an den Runner (er prüft die Bestätigung), ohne CA.
        let (sb, ca) = stage_boot(&cfg("backend: none\n", Some(Ok(()))), SessionId::new())
            .await
            .unwrap();
        assert_eq!(
            sb.unwrap().spec.backend,
            Some(beton_sandbox::BackendChoice::Provider(
                beton_sandbox::ProviderId::None
            ))
        );
        assert!(ca.is_none());
        let (sb, ca) = stage_boot(&cfg("backend: auto\n", Some(Ok(()))), SessionId::new())
            .await
            .unwrap();
        let sb = sb.unwrap();
        assert!(sb.spec.backend.is_some());
        assert_eq!(sb.extra_ca_files, vec![PathBuf::from("/etc/firmen-ca.pem")]);
        assert!(ca.unwrap().ca().is_ok());
    }

    #[tokio::test]
    async fn prx_002_session_with_network_but_without_ca_does_not_start() {
        for source in [None, Some(Err("secrets_locked".to_owned()))] {
            let err = stage_boot(&cfg("backend: auto\n", source), SessionId::new())
                .await
                .unwrap_err();
            assert_eq!(err.code, ProblemCode::SandboxUnavailable, "{err:?}");
        }
        // Ohne Netz braucht es keine CA.
        let (sb, ca) = stage_boot(
            &cfg("backend: auto\nallow_network: false\n", None),
            SessionId::new(),
        )
        .await
        .unwrap();
        assert!(sb.is_some() && ca.is_none());
    }
}
