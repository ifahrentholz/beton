//! Claude-Code-Adapter über das stream-json-Protokoll der offiziellen `claude`-CLI
//! (HAR-004, HAR-005, HAR-015, HAR-021).
//!
//! Ein langlebiger Prozess pro Session. Subscriptions laufen ausschließlich über die CLI
//! selbst: beton liest, speichert oder injiziert keine Anthropic-Tokens (ADR-0005) und
//! entfernt bei `auth: subscription` API-Key-Variablen aus der Umgebung.

pub mod import;
pub mod mapping;
pub mod permission;
pub mod rebuild;
pub mod record;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{
    Actor, ApprovalDecision, ApprovalKind, ApprovalRequested, ApprovalResolved, AuthSource,
    EventPayload, HarnessExited, RawJson, ResolvedVia, TimeoutAction, ToolCallStarted, TurnStarted,
};
use beton_core::id::{ApprovalId, PrincipalId, TurnId, UserId};
use beton_core::time::Timestamp;
use beton_harness::process::{LaunchSpec, ProcessHandle, ShutdownTimeouts};
use beton_harness::registry::{VersionProbe, resolve_binary};
use beton_harness::{
    Action, AdapterContext, ApprovalMechanism, AuthStatus, Capabilities, CompactionSupport,
    ExitInfo, ForkHistory, Gate, GateDecision, GateRequest, HarnessAdapter, HarnessError,
    HarnessId, HarnessSession, HostEnv, InstructionsDelivery, Mode, NormalizedEvent, OneShotReply,
    OneShotRequest, PermissionMode, ProbeReport, ResumeSupport, SessionSpec, Shutdown, Subagents,
    SwitchOutcome, SwitchSupport, ToolCallGate, Transport, UsageReporting, UserInput,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc};

use crate::mapping::{MapState, map_line, unmapped};
use crate::permission::{ModeGuard, vendor_mode};

/// Getestete CLI-Versionen (Golden-Transcripts unter `tests/golden/`).
pub const TESTED_VERSIONS: [&str; 1] = ["2.1.285"];
/// Unterstützter Bereich; die Fake-CLI meldet 2.1.0.
pub const VERSION_RANGE: &str = ">=2.1.0, <3.0.0";
/// Umgebungsvariablen, die bei `auth: subscription` nicht in den Prozess dürfen (HAR-015).
pub const SUBSCRIPTION_ENV_REMOVE: [&str; 2] = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"];
/// Höchstens so lange wartet der Adapter auf das Gate; danach `deny` (HAR-005 AC4).
pub const GATE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Höchstdauer für `claude auth status`.
pub const AUTH_STATUS_TIMEOUT: Duration = Duration::from_secs(5);

/// Wertet nur `loggedIn` aus `claude auth status --json` aus; Konto-Felder (E-Mail,
/// Organisation) werden verworfen (HAR-016).
pub fn parse_auth_status(stdout: &[u8]) -> AuthStatus {
    serde_json::from_slice::<Value>(stdout)
        .ok()
        .and_then(|v| v.get("loggedIn")?.as_bool())
        .map_or(AuthStatus::Unknown, |logged_in| {
            if logged_in {
                AuthStatus::LoggedIn
            } else {
                AuthStatus::LoggedOut
            }
        })
}

/// Der Claude-Code-Adapter.
#[derive(Debug, Clone)]
pub struct ClaudeAdapter {
    pub probe: VersionProbe,
    /// Auth-Herkunft laut `harnesses.claude.auth` (Default: Subscription).
    pub auth: AuthSource,
    /// Benutzer-Anpassungen der CLI (Hooks, Skills, Plugins, MCP) abschalten.
    pub isolated: bool,
    pub gate_timeout: Duration,
}

impl Default for ClaudeAdapter {
    fn default() -> Self {
        Self {
            probe: VersionProbe::default(),
            auth: AuthSource::VendorCli,
            isolated: false,
            gate_timeout: GATE_TIMEOUT,
        }
    }
}

impl ClaudeAdapter {
    /// Variablen, die bei `auth: subscription` nicht in den CLI-Prozess dürfen (HAR-015).
    pub fn env_remove(&self) -> Vec<String> {
        if self.auth == AuthSource::VendorCli {
            SUBSCRIPTION_ENV_REMOVE
                .iter()
                .map(|s| (*s).to_owned())
                .collect()
        } else {
            Vec::new()
        }
    }
}

pub fn capabilities() -> Capabilities {
    Capabilities {
        mode: Mode::Native,
        transport: Transport::Native,
        version_range: Some(VERSION_RANGE.into()),
        auth_sources: vec![AuthSource::VendorCli, AuthSource::ApiKey],
        approval: ApprovalMechanism::NativeRequest,
        // Vollständig erst mit dem PreToolUse-Hook (HAR-005, ab M2).
        tool_call_gate: ToolCallGate::ApprovalOnly,
        // `set_model` bzw. `apply_flag_settings {effortLevel}` (HAR-017, gegen 2.1.285 geprüft).
        model_switch: SwitchSupport::Live,
        effort_switch: SwitchSupport::Live,
        resume: ResumeSupport::Warm,
        fork_history: ForkHistory::Rebuild,
        interrupt: true,
        steering: false,
        subagents: Subagents::Native,
        usage_reporting: UsageReporting::TokensAndCost,
        // `/compact` als Nachricht; Ergebnis über `compact_boundary` und `result` (HAR-022).
        compaction: CompactionSupport::Native,
        instructions_delivery: InstructionsDelivery::AppendSystemPrompt,
        mcp_injection: true,
        images: true,
        // Import vorhandener Chats aus `~/.claude/projects` (HAR-023).
        transcript_import: true,
        // Aliase der CLI (`--model`); die genaue Liste hängt am Konto.
        models: vec!["sonnet".into(), "opus".into(), "haiku".into()],
        models_stale: false,
        // `--effort` kennt zusätzlich `max`; beton bietet die gemeinsamen Stufen an.
        efforts: vec!["low".into(), "medium".into(), "high".into(), "xhigh".into()],
        // `--permission-mode` bzw. `set_permission_mode` (HAR-027).
        permission_modes: PermissionMode::ALL.to_vec(),
        // Standard-Kontextfenster der Claude-Modelle (Handover-Budget, HAR-018).
        context_window: Some(200_000),
        // Claude Code liest `CLAUDE.md` selbst, `AGENTS.md` nicht (AGT-005).
        native_project_files: vec!["CLAUDE.md".into()],
    }
}

/// Name der Instructions-Datei im privaten Verzeichnis der Session.
pub const INSTRUCTIONS_FILE: &str = "instructions.md";

/// Schreibt die Instructions des Agents (AGT-005) in ein privates Verzeichnis (0700, Datei
/// 0600), das so lange lebt wie die Session; Claude liest sie über
/// `--append-system-prompt-file` (HAR-004 AC5).
pub fn write_instructions(text: &str) -> std::io::Result<(tempfile::TempDir, std::path::PathBuf)> {
    let dir = tempfile::Builder::new().prefix("beton-claude-").tempdir()?;
    let path = dir.path().join(INSTRUCTIONS_FILE);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    std::io::Write::write_all(&mut file, text.as_bytes())?;
    Ok((dir, path))
}

fn harness_id() -> HarnessId {
    HarnessId::CLAUDE.parse().unwrap_or_else(|_| unreachable!())
}

/// Neue Session-ID (UUID v4) für `--session-id`: So kennt beton die native Referenz schon vor
/// dem ersten Turn (HAR-020 AC2); die CLI meldet sie sonst erst mit `system/init`.
pub fn new_session_id() -> String {
    let mut b = fastrand::u128(..).to_be_bytes();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Kommandozeile für eine Session (HAR-004, gegen 2.1.285 verifiziert). `instructions` ist
/// die Datei mit den Agent-Instructions (AGT-005).
pub fn command_args(
    spec: &SessionSpec,
    isolated: bool,
    instructions: Option<&std::path::Path>,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-prompt-tool",
        "stdio",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    if let Some(model) = &spec.model {
        args.extend(["--model".into(), model.clone()]);
    }
    if let Some(effort) = &spec.effort {
        args.extend(["--effort".into(), effort.clone()]);
    }
    // Immer ausdrücklich: Ohne Flag gilt `permissions.defaultMode` aus `.claude/settings.json`
    // des Repositorys (HAR-027).
    args.extend([
        "--permission-mode".into(),
        vendor_mode(spec.permission_mode.unwrap_or(PermissionMode::Default)).into(),
    ]);
    if let Some(resume) = &spec.resume {
        args.extend(["--resume".into(), resume.clone()]);
        if spec.fork_session {
            // Native History übernehmen, die Quelle aber nicht fortschreiben (HAR-019).
            args.push("--fork-session".into());
        }
    }
    if let Some(file) = instructions {
        // Hängt an den System-Prompt von Claude Code an (HAR-004 AC5, AGT-005).
        args.extend([
            "--append-system-prompt-file".into(),
            file.display().to_string(),
        ]);
    }
    if isolated {
        // `--safe-mode` schaltet auch per `--mcp-config` und `--plugin-dir` übergebene Server
        // und Skills ab (gegen 2.1.285 verifiziert); daher keine Injektion (HAR-009).
        args.extend(["--safe-mode".into(), "--strict-mcp-config".into()]);
    } else {
        if let Some(config) = mcp_config(spec) {
            args.extend(["--mcp-config".into(), config]);
        }
        if let Some(dir) = &spec.mcp.skills_dir {
            args.extend(["--plugin-dir".into(), dir.display().to_string()]);
        }
    }
    args
}

/// Kleinstes Modell für Einmal-Aufrufe wie Session-Titel (SES-010); ein Alias der CLI.
pub const ONE_SHOT_MODEL: &str = "haiku";

/// Kommandozeile des Einmal-Modus (SES-010, Flags gegen 2.1.285 verifiziert): Antwort als ein
/// JSON-Objekt, keine Tools (auch keine MCP-Server), Permission-Mode `dontAsk`, keine
/// gespeicherte Session. Der Inhalt kommt über stdin, nicht
/// über argv; die Anweisung ersetzt den System-Prompt der CLI (spart Tokens des Kontingents).
pub fn one_shot_args(req: &OneShotRequest, isolated: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--output-format",
        "json",
        "--no-session-persistence",
        "--tools",
        "",
        // Keine MCP-Server aus Nutzer- oder Projekt-Konfiguration (deren Tools zählen nicht zu
        // `--tools`), und der Modus ausdrücklich, damit `.claude/settings.json` ihn nicht lockert:
        // `dontAsk` lehnt alles ab, was nicht vorab erlaubt ist (HAR-027).
        "--strict-mcp-config",
        "--permission-mode",
        "dontAsk",
        "--model",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    args.push(
        req.model
            .clone()
            .unwrap_or_else(|| ONE_SHOT_MODEL.to_owned()),
    );
    if !req.instructions.is_empty() {
        args.extend(["--system-prompt".into(), req.instructions.clone()]);
    }
    if isolated {
        args.push("--safe-mode".into());
    }
    args
}

/// Wertet die Antwort von `claude -p --output-format json` aus (SES-010).
pub fn parse_one_shot(stdout: &[u8], auth: AuthSource) -> Result<OneShotReply, HarnessError> {
    let v: Value = serde_json::from_slice(stdout)
        .map_err(|e| HarnessError::Protocol(format!("Antwort von claude -p ist kein JSON: {e}")))?;
    if v["type"] != "result" || v["is_error"] == true || v["subtype"] != "success" {
        let why = v["subtype"].as_str().unwrap_or("unbekannt");
        return Err(HarnessError::Protocol(format!(
            "claude -p ohne Ergebnis ({why})"
        )));
    }
    let text = v["result"].as_str().unwrap_or_default().to_owned();
    let model = mapping::result_model(&v).unwrap_or_default();
    Ok(OneShotReply {
        text,
        model,
        cost: Some(mapping::result_cost(&v, auth)),
    })
}

/// Inhaltsblöcke einer Eingabe für stream-json: Text, Bilder als `image`, PDF als `document`
/// (WEB-006; Format der Messages-API, das stream-json übernimmt).
pub fn user_content(input: &UserInput) -> Vec<Value> {
    let mut content = Vec::with_capacity(input.attachments.len() + 1);
    for a in &input.attachments {
        let source = json!({"type": "base64", "media_type": a.mime, "data": a.data_base64});
        if a.mime == "application/pdf" {
            content.push(json!({"type": "document", "source": source, "title": a.name}));
        } else {
            content.push(json!({"type": "image", "source": source}));
        }
    }
    if !input.text.is_empty() || content.is_empty() {
        content.push(json!({"type": "text", "text": input.text}));
    }
    content
}

/// `--mcp-config` als JSON-Text (HAR-009): nur Relay-Kommandos, keine Env-Werte oder Tokens.
pub fn mcp_config(spec: &SessionSpec) -> Option<String> {
    if spec.mcp.servers.is_empty() {
        return None;
    }
    let servers: serde_json::Map<String, Value> = spec
        .mcp
        .servers
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                json!({"type": "stdio", "command": s.command, "args": s.args}),
            )
        })
        .collect();
    Some(json!({"mcpServers": servers}).to_string())
}

#[async_trait]
impl HarnessAdapter for ClaudeAdapter {
    fn id(&self) -> HarnessId {
        harness_id()
    }

    fn modes(&self) -> &[Mode] {
        &[Mode::Native]
    }

    fn capabilities(&self, _mode: Mode, _probe: &ProbeReport) -> Capabilities {
        let caps = capabilities();
        Capabilities {
            // Isoliert (`--safe-mode`) kommen keine MCP-Server an (HAR-009), und Claude liest
            // auch `CLAUDE.md` nicht (laut `claude --help` 2.1.285).
            mcp_injection: !self.isolated,
            native_project_files: if self.isolated {
                Vec::new()
            } else {
                caps.native_project_files.clone()
            },
            ..caps
        }
    }

    async fn probe(&self, env: &HostEnv) -> ProbeReport {
        let Some(bin) = resolve_binary(&harness_id(), "claude", env) else {
            return ProbeReport::default();
        };
        let (version, failed) = match self.probe.version(&bin.program).await {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };
        ProbeReport {
            installed: true,
            path: Some(bin.program.display().to_string()),
            version,
            // Der Login-Status kommt erst beim Start (harness.auth_required); Token-Dateien
            // liest beton nie (HAR-015).
            auth_status: AuthStatus::Unknown,
            probe_failed: failed,
        }
    }

    async fn auth_status(&self, env: &HostEnv) -> AuthStatus {
        let Some(bin) = resolve_binary(&harness_id(), "claude", env) else {
            return AuthStatus::Unknown;
        };
        let remove: &[&str] = if self.auth == AuthSource::VendorCli {
            &SUBSCRIPTION_ENV_REMOVE
        } else {
            &[]
        };
        match beton_harness::registry::run_status(
            &bin.program,
            &["auth", "status", "--json"],
            remove,
            AUTH_STATUS_TIMEOUT,
        )
        .await
        {
            Ok(out) => parse_auth_status(&out.stdout),
            Err(_) => AuthStatus::Unknown,
        }
    }

    async fn rebuild_history(
        &self,
        request: &beton_harness::RebuildRequest,
        env: &HostEnv,
    ) -> Result<String, HarnessError> {
        let request = request.clone();
        let env = env.clone();
        tokio::task::spawn_blocking(move || rebuild::rebuild(&request, &env))
            .await
            .map_err(|e| HarnessError::Protocol(e.to_string()))?
    }

    fn transcript_importer(&self) -> Option<Arc<dyn beton_harness::import::TranscriptImporter>> {
        Some(Arc::new(import::ClaudeImporter::default()))
    }

    /// `claude -p` mit der Anmeldung der CLI (SES-010, ADR-0034): bei `auth: subscription`
    /// ohne API-Key-Variablen in der Umgebung (HAR-015).
    async fn one_shot(
        &self,
        request: &OneShotRequest,
        ctx: &AdapterContext,
    ) -> Result<OneShotReply, HarnessError> {
        let (program, mut args) = match resolve_binary(&harness_id(), "claude", &ctx.env) {
            Some(bin) => (bin.program, bin.args),
            None => ("claude".into(), Vec::new()),
        };
        args.extend(one_shot_args(request, self.isolated));
        let launch = LaunchSpec {
            program,
            args,
            env: Vec::new(),
            env_remove: self.env_remove(),
            clear_env: false,
            cwd: Some(request.workdir.clone()),
        };
        let out = beton_harness::process::run_once(
            ctx.launcher.as_ref(),
            launch,
            request.prompt.as_bytes(),
            request.timeout,
        )
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::TimedOut => HarnessError::Timeout(e.to_string()),
            _ => HarnessError::Io(e),
        })?;
        if out.exit.code != Some(0) && out.stdout.is_empty() {
            return Err(HarnessError::Protocol(format!(
                "claude -p endete mit {:?}",
                out.exit.code
            )));
        }
        parse_one_shot(&out.stdout, self.auth)
    }

    async fn start(
        &self,
        spec: SessionSpec,
        ctx: AdapterContext,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        // Installiert-Prüfung macht `Registry::start` über den Probe; ohne Auflösung bleibt
        // der Name, damit Replays (Golden-Tests) ohne CLI laufen.
        let (program, mut args) = match resolve_binary(&harness_id(), "claude", &ctx.env) {
            Some(bin) => (bin.program, bin.args),
            None => ("claude".into(), Vec::new()),
        };
        let instructions = match spec.instructions.as_deref() {
            Some(text) => Some(write_instructions(text).map_err(|e| {
                HarnessError::StartRefused(format!("Instructions-Datei nicht geschrieben: {e}"))
            })?),
            None => None,
        };
        args.extend(command_args(
            &spec,
            self.isolated,
            instructions.as_ref().map(|(_, p)| p.as_path()),
        ));
        // Eigene Session-ID für neue Sessions; beim Fortsetzen bleibt die ID der CLI (ohne
        // `--fork-session`), beim Abzweigen vergibt die CLI eine neue.
        let session_ref = match (&spec.resume, spec.fork_session) {
            (None, _) => {
                let id = new_session_id();
                args.extend(["--session-id".into(), id.clone()]);
                Some(id)
            }
            (Some(id), false) => Some(id.clone()),
            (Some(_), true) => None,
        };
        let launch = LaunchSpec {
            program,
            args,
            env: Vec::new(),
            env_remove: self.env_remove(),
            clear_env: false,
            cwd: Some(spec.workdir.clone()),
        };
        let mut process = ctx.launcher.launch(launch).await?;
        let io = process.take_io().ok_or(HarnessError::Closed)?;
        let stdin: Writer = Arc::new(Mutex::new(Some(io.stdin)));
        let state = Arc::new(Mutex::new(MapState {
            auth_source: Some(self.auth),
            session_ref,
            expected_mode: ModeGuard::new(vendor_mode(
                spec.permission_mode.unwrap_or(PermissionMode::Default),
            )),
            ..MapState::default()
        }));
        let (tx, rx) = mpsc::channel(4096);
        let process = Arc::new(Mutex::new(process));
        let closing = Arc::new(std::sync::atomic::AtomicBool::new(false));

        write_json(
            &stdin,
            &json!({"type": "control_request", "request_id": "beton_init", "request": {"subtype": "initialize"}}),
        )
        .await?;

        tokio::spawn(read_loop(
            io.stdout,
            Reader {
                stdin: stdin.clone(),
                state: state.clone(),
                tx: tx.clone(),
                gate: ctx.gate.clone(),
                gate_timeout: self.gate_timeout,
                process: process.clone(),
                closing: closing.clone(),
            },
        ));
        Ok(Box::new(ClaudeSession {
            stdin,
            state,
            tx,
            rx: Some(rx),
            process,
            closing,
            requests: 0,
            _instructions: instructions.map(|(dir, _)| dir),
        }))
    }
}

type Writer = Arc<Mutex<Option<Box<dyn AsyncWrite + Send + Unpin>>>>;

async fn write_json(stdin: &Writer, v: &Value) -> Result<(), HarnessError> {
    let mut guard = stdin.lock().await;
    let w = guard.as_mut().ok_or(HarnessError::Closed)?;
    w.write_all(format!("{v}\n").as_bytes()).await?;
    w.flush().await?;
    Ok(())
}

struct Reader {
    stdin: Writer,
    state: Arc<Mutex<MapState>>,
    tx: mpsc::Sender<NormalizedEvent>,
    gate: Arc<dyn Gate>,
    gate_timeout: Duration,
    process: Arc<Mutex<Box<dyn ProcessHandle>>>,
    closing: Arc<std::sync::atomic::AtomicBool>,
}

async fn read_loop(stdout: Box<dyn tokio::io::AsyncRead + Send + Unpin>, r: Reader) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let raw = RawJson::from_string(line.clone()).ok();
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            tracing::warn!("claude: ungültige stdout-Zeile");
            send(&r.tx, unmapped(&Value::String(line)), None, None).await;
            continue;
        };
        // HAR-027: Meldet die CLI einen freizügigeren Modus als gesetzt, endet die Session.
        if let Some(reported) = reported_mode(&v) {
            let verdict = r.state.lock().await.expected_mode.check(reported);
            if let Err(reason) = verdict {
                // Nichts mehr verarbeiten, was die CLI schon ausgegeben hat (auch keine
                // Freigabe-Anfragen); danach meldet die Schleife `harness.exited`.
                fail_closed(&r, &reason).await;
                break;
            }
        }
        match v["type"].as_str() {
            Some("control_request") => handle_control(&r, &v, raw).await,
            Some("control_response") => {
                // Antwort auf `set_permission_mode`: Wechsel bestätigt oder abgelehnt.
                if let Some(id) = v["response"]["request_id"].as_str() {
                    let ok = v["response"]["subtype"] == "success";
                    r.state.lock().await.expected_mode.answered(id, ok);
                }
            }
            _ => {
                let (events, turn) = {
                    let mut st = r.state.lock().await;
                    let turn = st.turn;
                    (map_line(&v, &mut st), turn)
                };
                if events
                    .iter()
                    .any(|e| matches!(e, EventPayload::HarnessUnmapped(_)))
                {
                    tracing::warn!(kind = ?v["type"], "claude: unbekannte Nachricht");
                }
                let mut raw = raw;
                for e in events {
                    // `raw` hängt am ersten Event der Zeile (HAR-001 AC3).
                    send(&r.tx, e, raw.take(), turn).await;
                }
            }
        }
    }
    // stdout zu: Prozess endet. Unerwartet → harness.exited (HAR-001 AC1).
    if !r.closing.load(std::sync::atomic::Ordering::SeqCst) {
        let exit = r.process.lock().await.wait().await.unwrap_or_default();
        send(
            &r.tx,
            EventPayload::HarnessExited(HarnessExited {
                code: exit.code,
                signal: exit.signal,
                stderr_tail: exit.stderr_tail,
            }),
            None,
            None,
        )
        .await;
    }
}

/// Permission-Mode, den eine stdout-Zeile meldet: `current_permission_mode` der
/// Initialize-Antwort oder `permissionMode` einer `system`-Zeile (`init`, `status`).
fn reported_mode(v: &Value) -> Option<&str> {
    match v["type"].as_str()? {
        "control_response" if v["response"]["request_id"] == "beton_init" => {
            v["response"]["response"]["current_permission_mode"].as_str()
        }
        "system" => v["permissionMode"].as_str(),
        _ => None,
    }
}

/// Fail closed (HAR-027): Fehler melden und die CLI sofort beenden; der Runner setzt die
/// Session über `harness.exited` auf `failed`.
async fn fail_closed(r: &Reader, reason: &str) {
    tracing::warn!("claude: Permission-Mode weicht ab, Session wird beendet");
    let turn = r.state.lock().await.turn;
    send(
        &r.tx,
        EventPayload::Error(beton_core::event::ErrorEvent {
            problem: json!({
                "type": "urn:beton:problem:permission_mode_mismatch",
                "code": "permission_mode_mismatch",
                "title": "Permission-Mode weicht ab; Session aus Sicherheitsgründen beendet",
                "detail": reason,
            }),
        }),
        None,
        turn,
    )
    .await;
    // Ohne stdin nimmt die CLI nichts mehr an; danach der ganze Prozessbaum.
    r.stdin.lock().await.take();
    let _ = r.process.lock().await.kill().await;
}

async fn send(
    tx: &mpsc::Sender<NormalizedEvent>,
    payload: EventPayload,
    raw: Option<RawJson>,
    turn: Option<TurnId>,
) {
    let _ = tx
        .send(NormalizedEvent {
            payload,
            raw,
            turn_id: turn,
        })
        .await;
}

/// Permission-Bridge (HAR-005): `can_use_tool` → Gate → `control_response`.
async fn handle_control(r: &Reader, v: &Value, raw: Option<RawJson>) {
    let req = &v["request"];
    let request_id = v["request_id"].clone();
    let turn = r.state.lock().await.turn;
    if req["subtype"] != "can_use_tool" {
        send(&r.tx, unmapped(v), raw, turn).await;
        let _ = write_json(
            &r.stdin,
            &json!({"type": "control_response", "response": {"subtype": "error", "request_id": request_id, "error": "nicht unterstützt"}}),
        )
        .await;
        return;
    }
    let tool = req["tool_name"].as_str().unwrap_or_default().to_owned();
    let input = req["input"].clone();
    let call_id = req["tool_use_id"].as_str().unwrap_or_default().to_owned();
    let approval_id = ApprovalId::new();
    let expires = time::OffsetDateTime::now_utc() + r.gate_timeout;
    send(
        &r.tx,
        EventPayload::ApprovalRequested(ApprovalRequested {
            approval_id,
            kind: ApprovalKind::Tool,
            subject: json!({"tool": tool, "args": input, "call_id": call_id}),
            options: vec!["allow".into(), "deny".into()],
            expires_at: Timestamp::from(expires),
            on_timeout: TimeoutAction::Deny,
        }),
        raw,
        turn,
    )
    .await;
    let decision = tokio::time::timeout(
        r.gate_timeout,
        r.gate.decide(GateRequest {
            turn_id: turn,
            call_id: call_id.clone(),
            tool: tool.clone(),
            kind: tool_kind(&tool).into(),
            args: input.clone(),
        }),
    )
    .await;
    let (response, decision, via, modified, comment) = match decision {
        Ok(GateDecision::Allow { updated_args }) => {
            let args = updated_args.clone().unwrap_or_else(|| input.clone());
            (
                json!({"behavior": "allow", "updatedInput": args}),
                ApprovalDecision::Allow,
                ResolvedVia::User,
                updated_args,
                None,
            )
        }
        Ok(GateDecision::Deny { reason }) => (
            json!({"behavior": "deny", "message": reason.clone().unwrap_or_else(|| "Von beton abgelehnt.".into())}),
            ApprovalDecision::Deny,
            ResolvedVia::User,
            None,
            // Die Begründung sehen auch andere Clients (WEB-018 AC1).
            reason,
        ),
        // Fail closed (HAR-005 AC4).
        Err(_) => (
            json!({"behavior": "deny", "message": "Keine Entscheidung erhalten; aus Sicherheitsgründen abgelehnt."}),
            ApprovalDecision::Deny,
            ResolvedVia::Timeout,
            None,
            None,
        ),
    };
    if decision == ApprovalDecision::Deny {
        r.state.lock().await.denied.insert(call_id.clone());
    }
    // Ein freigegebenes `ExitPlanMode` verlässt `plan` zum Modus davor (HAR-027); das Log
    // erfährt es, und der Wächter erwartet den neuen Modus.
    let left_plan = if decision == ApprovalDecision::Allow && tool == "ExitPlanMode" {
        r.state.lock().await.expected_mode.plan_exit_approved()
    } else {
        None
    };
    let _ = write_json(
        &r.stdin,
        &json!({"type": "control_response", "response": {"subtype": "success", "request_id": request_id, "response": response}}),
    )
    .await;
    if let Some(mode) = left_plan.as_deref().and_then(permission::beton_mode) {
        send(
            &r.tx,
            EventPayload::SessionSettingsChanged(beton_core::event::SessionSettingsChanged {
                permission_mode: Some(mode.as_str().to_owned()),
                mechanism: Some(beton_core::event::SettingsMechanism::Live),
                ..beton_core::event::SessionSettingsChanged::default()
            }),
            None,
            turn,
        )
        .await;
    }
    send(
        &r.tx,
        EventPayload::ApprovalResolved(ApprovalResolved {
            approval_id,
            decision,
            answer: None,
            actor: if via == ResolvedVia::Timeout {
                Actor::System {
                    component: beton_core::event::SystemComponent::Runner,
                }
            } else {
                Actor::User {
                    id: PrincipalId::User(UserId::LOCAL),
                    device_id: None,
                }
            },
            via,
            remember: None,
            comment,
            on_timeout_applied: (via == ResolvedVia::Timeout).then_some(TimeoutAction::Deny),
        }),
        None,
        turn,
    )
    .await;
    if decision == ApprovalDecision::Allow {
        send(
            &r.tx,
            EventPayload::ToolCallStarted(ToolCallStarted {
                call_id,
                sandbox_stage: None,
                args: modified,
            }),
            None,
            turn,
        )
        .await;
    }
}

/// Kanonische Tool-Klasse (POL-005) für die bekannten Claude-Tools.
pub fn tool_kind(tool: &str) -> &'static str {
    match tool {
        "Bash" | "BashOutput" | "KillShell" => "shell",
        "Read" | "NotebookRead" => "file_read",
        "Write" => "file_write",
        "Edit" | "MultiEdit" | "NotebookEdit" => "file_edit",
        "Glob" | "Grep" => "search",
        "WebFetch" | "WebSearch" => "web_fetch",
        t if t.starts_with("mcp__beton__") => "system",
        t if t.starts_with("mcp__") => "mcp",
        _ => "other",
    }
}

/// Eine laufende Claude-Code-Session.
pub struct ClaudeSession {
    stdin: Writer,
    state: Arc<Mutex<MapState>>,
    tx: mpsc::Sender<NormalizedEvent>,
    rx: Option<mpsc::Receiver<NormalizedEvent>>,
    process: Arc<Mutex<Box<dyn ProcessHandle>>>,
    closing: Arc<std::sync::atomic::AtomicBool>,
    requests: u64,
    /// Privates Verzeichnis der Instructions-Datei; wird mit der Session entfernt.
    _instructions: Option<tempfile::TempDir>,
}

impl ClaudeSession {
    fn next_request_id(&mut self) -> String {
        self.requests += 1;
        format!("beton_{}", self.requests)
    }

    async fn control(&mut self, request: Value) -> Result<(), HarnessError> {
        let id = self.next_request_id();
        self.control_with_id(&id, request).await
    }

    async fn control_with_id(&mut self, id: &str, request: Value) -> Result<(), HarnessError> {
        write_json(
            &self.stdin,
            &json!({"type": "control_request", "request_id": id, "request": request}),
        )
        .await
    }
}

#[async_trait]
impl HarnessSession for ClaudeSession {
    async fn send(&mut self, input: UserInput) -> Result<TurnId, HarnessError> {
        let turn = input.turn_id.unwrap_or_default();
        self.state.lock().await.turn = Some(turn);
        let _ = self
            .tx
            .send(NormalizedEvent::new(
                EventPayload::TurnStarted(TurnStarted {
                    turn_id: turn,
                    input_id: None,
                    author: PrincipalId::User(UserId::LOCAL),
                }),
                Some(turn),
            ))
            .await;
        write_json(
            &self.stdin,
            &json!({"type": "user", "message": {"role": "user", "content": user_content(&input)}}),
        )
        .await?;
        Ok(turn)
    }

    async fn steer(&mut self, _input: UserInput) -> Result<(), HarnessError> {
        capabilities().check(Action::Steer)?;
        Ok(())
    }

    async fn interrupt(&mut self) -> Result<(), HarnessError> {
        self.state.lock().await.interrupt_requested = true;
        self.control(json!({"subtype": "interrupt"})).await
    }

    async fn set_model(
        &mut self,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<SwitchOutcome, HarnessError> {
        // HAR-017: beides live über Control-Requests, wirksam ab dem nächsten Request (gegen
        // 2.1.285 ohne Modellaufruf geprüft: `apply_flag_settings` setzt `effortLevel`).
        if let Some(effort) = &effort {
            capabilities().check(Action::EffortSwitch)?;
            if !capabilities().efforts.contains(effort) {
                return Err(beton_harness::CapabilityUnsupported(Action::EffortSwitch).into());
            }
        }
        if let Some(model) = model {
            self.control(json!({"subtype": "set_model", "model": model}))
                .await?;
        }
        if let Some(effort) = effort {
            self.control(
                json!({"subtype": "apply_flag_settings", "settings": {"effortLevel": effort}}),
            )
            .await?;
        }
        Ok(SwitchOutcome::Live)
    }

    async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), HarnessError> {
        if !capabilities().permission_modes.contains(&mode) {
            return Err(beton_harness::CapabilityUnsupported(Action::PermissionMode).into());
        }
        let vendor = vendor_mode(mode);
        // Bis zur Antwort darf die CLI den alten oder den neuen Modus melden (`ModeGuard`).
        let id = self.next_request_id();
        self.state.lock().await.expected_mode.allow(&id, vendor);
        self.control_with_id(
            &id,
            json!({"subtype": "set_permission_mode", "mode": vendor}),
        )
        .await
    }

    /// HAR-022: `/compact` als Nutzernachricht; die CLI meldet `compact_boundary` und ein
    /// `result`, das die Mapping-Schicht als `compaction.completed` statt als Turn-Ende liest.
    async fn compact(&mut self) -> Result<(), HarnessError> {
        capabilities().check(Action::Compact)?;
        let before = {
            let mut st = self.state.lock().await;
            if st.turn.is_some() || st.compacting {
                return Err(HarnessError::Busy(
                    "Compaction erst nach dem laufenden Turn".into(),
                ));
            }
            st.compacting = true;
            st.compact_before = st.last_context;
            st.last_context.unwrap_or(0)
        };
        let _ = self
            .tx
            .send(NormalizedEvent::new(
                EventPayload::CompactionStarted(beton_core::event::Compaction {
                    before_tokens: before,
                    after_tokens: None,
                }),
                None,
            ))
            .await;
        let sent = write_json(
            &self.stdin,
            &json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "/compact"}]}}),
        )
        .await;
        if sent.is_err() {
            self.state.lock().await.compacting = false;
        }
        sent
    }

    fn events(&mut self) -> Option<mpsc::Receiver<NormalizedEvent>> {
        self.rx.take()
    }

    fn native_session_ref(&self) -> Option<String> {
        self.state
            .try_lock()
            .ok()
            .and_then(|s| s.session_ref.clone())
    }

    async fn shutdown(self: Box<Self>, how: Shutdown) -> Result<ExitInfo, HarnessError> {
        self.closing
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // stdin schließen ist das protokolleigene Ende.
        self.stdin.lock().await.take();
        let mut process = self.process.lock().await;
        let exit = match how {
            Shutdown::Kill => process.kill().await?,
            Shutdown::Graceful { timeout } => {
                process
                    .terminate(ShutdownTimeouts {
                        graceful: timeout,
                        ..ShutdownTimeouts::default()
                    })
                    .await?
            }
        };
        Ok(exit)
    }
}

#[cfg(test)]
mod auth_tests {
    use super::*;

    #[test]
    fn har_016_auth_status_reads_only_logged_in() {
        assert_eq!(
            parse_auth_status(br#"{"loggedIn":true,"email":"x@y.z","orgName":"O"}"#),
            AuthStatus::LoggedIn
        );
        assert_eq!(
            parse_auth_status(br#"{"loggedIn":false}"#),
            AuthStatus::LoggedOut
        );
        assert_eq!(parse_auth_status(b"Logged in as x"), AuthStatus::Unknown);
        assert_eq!(parse_auth_status(b""), AuthStatus::Unknown);
    }
}

#[cfg(test)]
mod mcp_tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use beton_harness::{McpInjection, McpLaunch};

    fn spec() -> SessionSpec {
        SessionSpec {
            mcp: McpInjection {
                servers: vec![
                    McpLaunch {
                        name: "beton".into(),
                        command: "/bin/beton".into(),
                        args: vec![
                            "mcp".into(),
                            "serve".into(),
                            "--token-file".into(),
                            "/run/t".into(),
                        ],
                    },
                    McpLaunch {
                        name: "gh".into(),
                        command: "/bin/beton".into(),
                        args: vec!["mcp".into(), "proxy".into(), "--server".into(), "gh".into()],
                    },
                ],
                skills_dir: Some("/run/skills".into()),
            },
            ..SessionSpec::default()
        }
    }

    fn arg_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    }

    #[test]
    fn har_009_claude_gets_relays_via_mcp_config_and_skills_via_plugin_dir() {
        let args = command_args(&spec(), false, None);
        let config: Value =
            serde_json::from_str(arg_after(&args, "--mcp-config").unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["beton"]["type"], "stdio");
        assert_eq!(config["mcpServers"]["beton"]["command"], "/bin/beton");
        assert_eq!(config["mcpServers"]["gh"]["args"][3], "gh");
        // Nur Relay-Kommandos, keine Env-Blöcke.
        assert!(config["mcpServers"]["gh"].get("env").is_none());
        assert_eq!(arg_after(&args, "--plugin-dir"), Some("/run/skills"));
        // Ohne Injektion keine Flags.
        let plain = command_args(&SessionSpec::default(), false, None);
        assert!(
            !plain
                .iter()
                .any(|a| a == "--mcp-config" || a == "--plugin-dir")
        );
    }

    #[test]
    fn har_027_permission_mode_is_always_explicit() {
        // Ohne Angabe ausdrücklich `default`: `.claude/settings.json` im Repository kann den
        // Modus sonst lockern.
        let args = command_args(&SessionSpec::default(), false, None);
        assert_eq!(arg_after(&args, "--permission-mode"), Some("default"));
        for (mode, vendor) in [
            (PermissionMode::Plan, "plan"),
            (PermissionMode::AcceptEdits, "acceptEdits"),
        ] {
            let args = command_args(
                &SessionSpec {
                    permission_mode: Some(mode),
                    ..SessionSpec::default()
                },
                false,
                None,
            );
            assert_eq!(arg_after(&args, "--permission-mode"), Some(vendor));
        }
    }

    #[test]
    fn har_017_start_effort_and_model_go_to_the_cli() {
        let args = command_args(
            &SessionSpec {
                model: Some("sonnet".into()),
                effort: Some("high".into()),
                ..SessionSpec::default()
            },
            false,
            None,
        );
        assert_eq!(arg_after(&args, "--model"), Some("sonnet"));
        assert_eq!(arg_after(&args, "--effort"), Some("high"));
        assert!(
            !command_args(&SessionSpec::default(), false, None)
                .iter()
                .any(|a| a == "--effort")
        );
    }

    #[test]
    fn har_020_new_session_id_is_a_uuid() {
        let id = new_session_id();
        assert_eq!(id.len(), 36);
        assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);
        assert_eq!(&id[14..15], "4", "Version 4");
        assert_ne!(id, new_session_id());
    }

    #[test]
    fn har_004_ac5_instructions_go_via_append_system_prompt_file() {
        let (dir, path) = write_instructions("Du behebst CI-Fehler.\n\nBranch: develop").unwrap();
        // Der Test prüft den generierten Dateiinhalt (HAR-004 AC5).
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "Du behebst CI-Fehler.\n\nBranch: develop"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let args = command_args(&SessionSpec::default(), false, Some(&path));
        assert_eq!(
            arg_after(&args, "--append-system-prompt-file"),
            Some(path.to_str().unwrap())
        );
        // Auch isoliert (`--safe-mode`) kommen die Instructions an.
        let isolated = command_args(&SessionSpec::default(), true, Some(&path));
        assert!(isolated.iter().any(|a| a == "--append-system-prompt-file"));
        assert!(
            !command_args(&SessionSpec::default(), false, None)
                .iter()
                .any(|a| a == "--append-system-prompt-file")
        );
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn har_009_isolated_mode_has_no_mcp_injection() {
        let args = command_args(&spec(), true, None);
        assert!(args.iter().any(|a| a == "--safe-mode"));
        assert!(
            !args
                .iter()
                .any(|a| a == "--mcp-config" || a == "--plugin-dir")
        );
        let isolated = ClaudeAdapter {
            isolated: true,
            ..ClaudeAdapter::default()
        };
        assert!(
            !isolated
                .capabilities(Mode::Native, &ProbeReport::default())
                .mcp_injection
        );
        assert!(
            ClaudeAdapter::default()
                .capabilities(Mode::Native, &ProbeReport::default())
                .mcp_injection
        );
    }

    #[test]
    fn agt_007_system_tools_are_classified_as_system() {
        assert_eq!(tool_kind("mcp__beton__policy_query"), "system");
        assert_eq!(tool_kind("mcp__gh__get"), "mcp");
        assert_eq!(
            mapping::split_mcp_name("mcp__beton__session_spawn"),
            ("session_spawn".to_owned(), Some("beton".to_owned()))
        );
        assert_eq!(mapping::split_mcp_name("Bash"), ("Bash".to_owned(), None));
        assert_eq!(
            mapping::split_mcp_name("mcp__x"),
            ("mcp__x".to_owned(), None)
        );
        let mut st = mapping::MapState::default();
        let out = mapping::map_line(
            &json!({"type": "assistant", "message": {"id": "m", "content": [
                {"type": "tool_use", "id": "t1", "name": "mcp__beton__policy_query", "input": {"action": "x"}}
            ]}}),
            &mut st,
        );
        let EventPayload::ToolCallRequested(r) = &out[0] else {
            panic!("{out:?}")
        };
        assert_eq!(r.tool, "policy_query");
        assert_eq!(r.mcp_server.as_deref(), Some("beton"));
        assert_eq!(r.source, beton_core::event::ToolSource::BetonMcp);
    }
}

#[cfg(test)]
mod attachment_tests {
    use super::*;

    #[test]
    fn web_006_attachments_become_image_and_document_blocks() {
        let input = UserInput {
            text: "Was steht hier?".into(),
            turn_id: None,
            attachments: vec![
                beton_harness::InputAttachment {
                    name: "shot.png".into(),
                    mime: "image/png".into(),
                    data_base64: "iVBORw0K".into(),
                },
                beton_harness::InputAttachment {
                    name: "bericht.pdf".into(),
                    mime: "application/pdf".into(),
                    data_base64: "JVBERi0x".into(),
                },
            ],
        };
        let c = user_content(&input);
        assert_eq!(c.len(), 3);
        assert_eq!(c[0]["type"], "image");
        assert_eq!(c[0]["source"]["media_type"], "image/png");
        assert_eq!(c[0]["source"]["data"], "iVBORw0K");
        assert_eq!(c[1]["type"], "document");
        assert_eq!(c[1]["title"], "bericht.pdf");
        assert_eq!(c[2], json!({"type": "text", "text": "Was steht hier?"}));
        assert_eq!(
            user_content(&UserInput::from("nur Text")),
            vec![json!({"type": "text", "text": "nur Text"})]
        );
    }
}
