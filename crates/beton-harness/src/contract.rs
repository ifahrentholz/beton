//! Harness-Contract-Suite (QA-016): prüft einen Adapter gegen seine deklarierten Capabilities.
//!
//! Die Suite spielt Szenarien im Format des Fake-Harness (HAR-026) ab: eingebaute Adapter
//! über die Protokoll-Fake-CLIs (QA-002), der Fake-Harness im Prozess. Dieselbe Suite prüft
//! später Harness-Plugins (PLG-013); sie hängt deshalb nur am [`ContractSubject`].
//!
//! Prüfungen: Streaming, Usage-Reporting, Approval-Roundtrip (allow und deny), Interrupt,
//! Resume, Modellwechsel und die Fehlerpfade Turn-Fehler, Absturz und abgelaufener Login.
//! Eine Capability, die ein Adapter nicht deklariert, muss er mit `capability_unsupported`
//! ablehnen statt still zu ignorieren (HAR-002 AC3).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{ApprovalDecision, EventPayload, ToolStatus};
use tokio::sync::mpsc;

use crate::adapter::{
    AdapterContext, Gate, GateDecision, GateRequest, HarnessAdapter, HarnessError, HarnessSession,
    HostEnv, Mode, NormalizedEvent, SessionSpec, Shutdown, UserInput,
};
use crate::capabilities::{
    ApprovalMechanism, Capabilities, ResumeSupport, SwitchSupport, UsageReporting,
};
use crate::process::ProcessLauncher;

/// Wie lange die Suite höchstens auf ein Event wartet.
pub const EVENT_TIMEOUT: Duration = Duration::from_secs(15);
/// Nach `turn.interrupted` darf in dieser Zeit kein Delta des Turns mehr kommen (QA-016 AC1).
pub const QUIET_AFTER_INTERRUPT: Duration = Duration::from_millis(400);

/// Was die Suite prüft.
#[async_trait]
pub trait ContractSubject: Send + Sync {
    fn name(&self) -> String;
    /// Deklarierte Capabilities, wenn der Harness `scenario` abspielt.
    async fn capabilities(&self, scenario: &Path) -> Capabilities;
    /// Startet eine Session, die `scenario` abspielt.
    async fn start(
        &self,
        scenario: &Path,
        resume: Option<String>,
        gate: Arc<dyn Gate>,
    ) -> Result<Box<dyn HarnessSession>, HarnessError>;
}

/// Erzeugt die Umgebung, unter der ein Adapter ein Szenario abspielt, z. B.
/// `BETON_CODEX_PATH=beton-fake-cli --protocol app-server --scenario <datei>`.
pub type EnvForScenario = Arc<dyn Fn(&Path) -> HostEnv + Send + Sync>;

/// Ein [`HarnessAdapter`] als Prüfling. Das Szenario kommt über die Umgebung (Fake-CLI) und
/// über `SessionSpec::scenario` (Fake-Harness).
pub struct AdapterSubject {
    pub adapter: Arc<dyn HarnessAdapter>,
    pub env: EnvForScenario,
    pub launcher: Arc<dyn ProcessLauncher>,
    pub workdir: PathBuf,
}

impl std::fmt::Debug for AdapterSubject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdapterSubject")
            .field("adapter", &self.adapter.id())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ContractSubject for AdapterSubject {
    fn name(&self) -> String {
        self.adapter.id().to_string()
    }

    async fn capabilities(&self, scenario: &Path) -> Capabilities {
        let env = (self.env)(scenario);
        let probe = self.adapter.probe(&env).await;
        self.adapter.capabilities(Mode::Native, &probe)
    }

    async fn start(
        &self,
        scenario: &Path,
        resume: Option<String>,
        gate: Arc<dyn Gate>,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        let ctx = AdapterContext {
            gate,
            launcher: self.launcher.clone(),
            env: (self.env)(scenario),
        };
        self.adapter
            .start(
                SessionSpec {
                    workdir: self.workdir.clone(),
                    scenario: Some(scenario.to_path_buf()),
                    resume,
                    ..SessionSpec::default()
                },
                ctx,
            )
            .await
    }
}

/// Ergebnis einer Prüfung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    /// Capability nicht deklariert und korrekt abgelehnt bzw. nicht anwendbar.
    Skipped(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub check: &'static str,
    pub outcome: Outcome,
}

/// Bericht der Suite für einen Prüfling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContractReport {
    pub subject: String,
    pub results: Vec<CheckResult>,
}

impl ContractReport {
    pub fn failures(&self) -> Vec<&CheckResult> {
        self.results
            .iter()
            .filter(|r| matches!(r.outcome, Outcome::Failed(_)))
            .collect()
    }

    pub fn outcome(&self, check: &str) -> Option<&Outcome> {
        self.results
            .iter()
            .find(|r| r.check == check)
            .map(|r| &r.outcome)
    }

    pub fn is_ok(&self) -> bool {
        self.failures().is_empty()
    }

    /// Lesbare Zusammenfassung (für Testmeldungen).
    pub fn summary(&self) -> String {
        let mut out = format!("Contract-Suite für {}:\n", self.subject);
        for r in &self.results {
            let _ = match &r.outcome {
                Outcome::Passed => writeln!(out, "  ok    {}", r.check),
                Outcome::Skipped(why) => writeln!(out, "  skip  {} ({why})", r.check),
                Outcome::Failed(why) => writeln!(out, "  FAIL  {}: {why}", r.check),
            };
        }
        out
    }
}

/// Szenarien der Suite (Format HAR-026). Eingaben ohne `expect_input`, damit jeder Harness
/// sie unabhängig vom Wortlaut abspielt.
pub mod scenarios {
    pub const STREAMING_TEXT: &str = "Hallo Welt, hier spricht der Vertrag.";
    pub const STREAMING: &str = r#"
turns:
  - emit:
      - { message_delta: "Hallo Welt, hier spricht der Vertrag.", chunk: 6 }
      - { usage: { input_tokens: 100, output_tokens: 20 } }
  - emit:
      - { message: "Zweiter Turn." }
"#;
    pub const APPROVAL: &str = r#"
turns:
  - emit:
      - { tool_call: { name: Bash, kind: shell, args: { command: "git push origin main" } }, gate: true }
      - { on_gate: { allow: [{ tool_result: "gepusht" }, { message: "Erledigt." }], deny: [{ message: "Abgelehnt." }] } }
"#;
    pub const INTERRUPT: &str = r#"
turns:
  - emit:
      - { message_delta: "Ich arbeite gerade", chunk: 3 }
      - { hang: true }
  - emit:
      - { message: "Weiter geht's." }
"#;
    pub const TURN_ERROR: &str = r#"
turns:
  - emit:
      - { error: "Absichtlicher Fehler" }
"#;
    pub const CRASH: &str = r#"
turns:
  - emit:
      - { message: "Gleich stürze ich ab." }
      - { crash: 3 }
"#;
    pub const AUTH_EXPIRED: &str = r#"
turns:
  - emit:
      - { auth_expired: "Login abgelaufen" }
"#;
}

/// Gate mit fester Entscheidung, das Anfragen mitzählt.
#[derive(Debug)]
struct FixedGate {
    allow: bool,
    requests: Mutex<Vec<GateRequest>>,
}

impl FixedGate {
    fn new(allow: bool) -> Arc<Self> {
        Arc::new(Self {
            allow,
            requests: Mutex::default(),
        })
    }

    fn count(&self) -> usize {
        self.requests.lock().map(|r| r.len()).unwrap_or(0)
    }
}

#[async_trait]
impl Gate for FixedGate {
    async fn decide(&self, request: GateRequest) -> GateDecision {
        if let Ok(mut r) = self.requests.lock() {
            r.push(request);
        }
        if self.allow {
            GateDecision::Allow { updated_args: None }
        } else {
            GateDecision::Deny {
                reason: Some("Contract-Suite: abgelehnt".into()),
            }
        }
    }
}

fn is_turn_end(e: &EventPayload) -> bool {
    matches!(
        e,
        EventPayload::TurnCompleted(_)
            | EventPayload::TurnFailed(_)
            | EventPayload::TurnInterrupted(_)
            | EventPayload::HarnessExited(_)
    )
}

/// Events bis zum Turn-Ende; `Err`, wenn keins innerhalb der Frist kommt.
async fn until_turn_end(
    rx: &mut mpsc::Receiver<NormalizedEvent>,
) -> Result<Vec<EventPayload>, String> {
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(EVENT_TIMEOUT, rx.recv()).await {
            Err(_) => {
                return Err(format!(
                    "kein Turn-Ende innerhalb von {EVENT_TIMEOUT:?}; bisher: {:?}",
                    names(&out)
                ));
            }
            Ok(None) => return Ok(out),
            Ok(Some(e)) => {
                let end = is_turn_end(&e.payload);
                out.push(e.payload);
                if end {
                    return Ok(out);
                }
            }
        }
    }
}

fn names(events: &[EventPayload]) -> Vec<&'static str> {
    events.iter().map(EventPayload::type_name).collect()
}

fn count(events: &[EventPayload], name: &str) -> usize {
    events.iter().filter(|e| e.type_name() == name).count()
}

struct Started {
    session: Box<dyn HarnessSession>,
    rx: mpsc::Receiver<NormalizedEvent>,
}

async fn start(
    subject: &dyn ContractSubject,
    scenario: &Path,
    resume: Option<String>,
    gate: Arc<dyn Gate>,
) -> Result<Started, String> {
    let mut session = subject
        .start(scenario, resume, gate)
        .await
        .map_err(|e| format!("Start fehlgeschlagen: {e}"))?;
    let rx = session.events().ok_or("Event-Strom fehlt")?;
    Ok(Started { session, rx })
}

async fn stop(s: Started) {
    let _ = s
        .session
        .shutdown(Shutdown::Graceful {
            timeout: Duration::from_secs(2),
        })
        .await;
}

fn write_scenario(dir: &Path, name: &str, yaml: &str) -> Result<PathBuf, String> {
    let path = dir.join(format!("contract-{name}.yaml"));
    std::fs::write(&path, yaml).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Führt alle Prüfungen aus. `scratch` nimmt die Szenario-Dateien auf.
pub async fn run(subject: &dyn ContractSubject, scratch: &Path) -> ContractReport {
    let mut report = ContractReport {
        subject: subject.name(),
        results: Vec::new(),
    };
    let mut push = |check: &'static str, outcome: Outcome| {
        report.results.push(CheckResult { check, outcome });
    };
    let paths = (|| -> Result<_, String> {
        Ok((
            write_scenario(scratch, "streaming", scenarios::STREAMING)?,
            write_scenario(scratch, "approval", scenarios::APPROVAL)?,
            write_scenario(scratch, "interrupt", scenarios::INTERRUPT)?,
            write_scenario(scratch, "turn-error", scenarios::TURN_ERROR)?,
            write_scenario(scratch, "crash", scenarios::CRASH)?,
            write_scenario(scratch, "auth-expired", scenarios::AUTH_EXPIRED)?,
        ))
    })();
    let (streaming, approval, interrupt, turn_error, crash, auth) = match paths {
        Ok(p) => p,
        Err(e) => {
            push("setup", Outcome::Failed(e));
            return report;
        }
    };
    let caps = subject.capabilities(&streaming).await;

    let (stream_outcome, usage_outcome) = check_streaming(subject, &streaming, &caps).await;
    push("streaming", stream_outcome);
    push("usage_reporting", usage_outcome);
    push(
        "approval_allow",
        check_approval(subject, &approval, &caps, true).await,
    );
    push(
        "approval_deny",
        check_approval(subject, &approval, &caps, false).await,
    );
    push(
        "interrupt",
        check_interrupt(subject, &interrupt, &streaming, &caps).await,
    );
    push("resume", check_resume(subject, &streaming, &caps).await);
    push(
        "model_switch",
        check_model_switch(subject, &streaming, &caps).await,
    );
    push("turn_error", check_turn_error(subject, &turn_error).await);
    push("crash", check_crash(subject, &crash).await);
    push("auth_expired", check_auth_expired(subject, &auth).await);
    report
}

fn outcome(result: Result<(), String>) -> Outcome {
    match result {
        Ok(()) => Outcome::Passed,
        Err(e) => Outcome::Failed(e),
    }
}

async fn check_streaming(
    subject: &dyn ContractSubject,
    scenario: &Path,
    caps: &Capabilities,
) -> (Outcome, Outcome) {
    let mut s = match start(subject, scenario, None, FixedGate::new(true)).await {
        Ok(s) => s,
        Err(e) => return (Outcome::Failed(e.clone()), Outcome::Failed(e)),
    };
    let result = async {
        s.session
            .send(UserInput::from("Sag hallo"))
            .await
            .map_err(|e| format!("send: {e}"))?;
        until_turn_end(&mut s.rx).await
    }
    .await;
    stop(s).await;
    let events = match result {
        Ok(e) => e,
        Err(e) => return (Outcome::Failed(e.clone()), Outcome::Failed(e)),
    };
    let streaming = (|| {
        let deltas: Vec<(&str, &str)> = events
            .iter()
            .filter_map(|e| match e {
                EventPayload::MessageDelta(d) => Some((d.message_id.as_str(), d.text.as_str())),
                _ => None,
            })
            .collect();
        if deltas.len() < 2 {
            return Err(format!(
                "erwartet mehrere message.delta, erhalten {}: {:?}",
                deltas.len(),
                names(&events)
            ));
        }
        let text: String = deltas.iter().map(|(_, t)| *t).collect();
        if text != scenarios::STREAMING_TEXT {
            return Err(format!("Deltas ergeben {text:?}"));
        }
        let completed: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                EventPayload::MessageCompleted(m) => Some(m),
                _ => None,
            })
            .collect();
        let [message] = completed.as_slice() else {
            return Err(format!(
                "erwartet genau ein message.completed, erhalten {}",
                completed.len()
            ));
        };
        if deltas.iter().any(|(id, _)| *id != message.message_id) {
            return Err("Deltas und message.completed haben verschiedene IDs".into());
        }
        if message.content.first().and_then(|c| c["text"].as_str())
            != Some(scenarios::STREAMING_TEXT)
        {
            return Err(format!("message.completed: {:?}", message.content));
        }
        match events.last() {
            Some(EventPayload::TurnCompleted(_)) => Ok(()),
            other => Err(format!(
                "Turn endet mit {:?}",
                other.map(EventPayload::type_name)
            )),
        }
    })();
    let usage = if caps.usage_reporting == UsageReporting::None {
        if count(&events, "cost.delta") == 0 {
            Outcome::Skipped("usage_reporting: none".into())
        } else {
            Outcome::Failed("usage_reporting: none, aber cost.delta gesendet".into())
        }
    } else {
        let costs: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                EventPayload::CostDelta(c) => Some(c),
                _ => None,
            })
            .collect();
        outcome(match costs.as_slice() {
            [c] if c.input_tokens + c.cache_read_tokens == 100 && c.output_tokens == 20 => Ok(()),
            [c] => Err(format!(
                "cost.delta mit input {} (+{} Cache) / output {}, erwartet 100/20",
                c.input_tokens, c.cache_read_tokens, c.output_tokens
            )),
            other => Err(format!(
                "erwartet genau ein cost.delta je Turn, erhalten {}",
                other.len()
            )),
        })
    };
    (outcome(streaming), usage)
}

async fn check_approval(
    subject: &dyn ContractSubject,
    scenario: &Path,
    caps: &Capabilities,
    allow: bool,
) -> Outcome {
    if caps.approval == ApprovalMechanism::None {
        return Outcome::Skipped("approval: none".into());
    }
    let gate = FixedGate::new(allow);
    let mut s = match start(subject, scenario, None, gate.clone()).await {
        Ok(s) => s,
        Err(e) => return Outcome::Failed(e),
    };
    let result = async {
        s.session
            .send(UserInput::from("Bitte pushen"))
            .await
            .map_err(|e| format!("send: {e}"))?;
        until_turn_end(&mut s.rx).await
    }
    .await;
    stop(s).await;
    outcome((|| {
        let events = result?;
        if gate.count() != 1 {
            return Err(format!("Gate {} mal gefragt, erwartet 1", gate.count()));
        }
        if count(&events, "approval.requested") != 1 {
            return Err(format!(
                "erwartet genau ein approval.requested: {:?}",
                names(&events)
            ));
        }
        let resolved = events.iter().find_map(|e| match e {
            EventPayload::ApprovalResolved(r) => Some(r),
            _ => None,
        });
        let want = if allow {
            ApprovalDecision::Allow
        } else {
            ApprovalDecision::Deny
        };
        if resolved.map(|r| r.decision) != Some(want) {
            return Err(format!("approval.resolved fehlt oder ist nicht {want:?}"));
        }
        let call = events
            .iter()
            .find_map(|e| match e {
                EventPayload::ToolCallRequested(r) => Some(r.call_id.clone()),
                _ => None,
            })
            .ok_or("tool.call.requested fehlt")?;
        let started = events
            .iter()
            .any(|e| matches!(e, EventPayload::ToolCallStarted(s) if s.call_id == call));
        let completed = events.iter().find_map(|e| match e {
            EventPayload::ToolCallCompleted(c) if c.call_id == call => Some(c.status),
            _ => None,
        });
        if allow {
            if !started {
                return Err("freigegebener Tool-Call ohne tool.call.started".into());
            }
            if completed != Some(ToolStatus::Ok) {
                return Err(format!("tool.call.completed: {completed:?}, erwartet ok"));
            }
        } else {
            if started {
                return Err("abgelehnter Tool-Call wurde gestartet".into());
            }
            if completed.is_some_and(|s| s != ToolStatus::Denied) {
                return Err(format!(
                    "abgelehnter Tool-Call endet mit {completed:?}, erwartet denied"
                ));
            }
        }
        match events.last() {
            Some(EventPayload::TurnCompleted(_)) => Ok(()),
            other => Err(format!(
                "Turn endet mit {:?}",
                other.map(EventPayload::type_name)
            )),
        }
    })())
}

fn is_turn_output(e: &EventPayload) -> bool {
    matches!(
        e,
        EventPayload::MessageDelta(_)
            | EventPayload::ReasoningDelta(_)
            | EventPayload::MessageCompleted(_)
            | EventPayload::ToolCallRequested(_)
            | EventPayload::ToolCallStarted(_)
            | EventPayload::ToolCallOutputDelta(_)
    )
}

async fn check_interrupt(
    subject: &dyn ContractSubject,
    scenario: &Path,
    idle_scenario: &Path,
    caps: &Capabilities,
) -> Outcome {
    if !caps.interrupt {
        // Nicht deklariert: muss abgelehnt werden.
        let mut s = match start(subject, idle_scenario, None, FixedGate::new(true)).await {
            Ok(s) => s,
            Err(e) => return Outcome::Failed(e),
        };
        let result = s.session.interrupt().await;
        stop(s).await;
        return match result {
            Err(e) if e.code() == "capability_unsupported" => {
                Outcome::Skipped("interrupt: false, korrekt abgelehnt".into())
            }
            other => Outcome::Failed(format!(
                "interrupt: false deklariert, aber interrupt() liefert {other:?}"
            )),
        };
    }
    let mut s = match start(subject, scenario, None, FixedGate::new(true)).await {
        Ok(s) => s,
        Err(e) => return Outcome::Failed(e),
    };
    let result = async {
        s.session
            .send(UserInput::from("Arbeite"))
            .await
            .map_err(|e| format!("send: {e}"))?;
        // Warten, bis der Turn sichtbar läuft.
        loop {
            match tokio::time::timeout(EVENT_TIMEOUT, s.rx.recv()).await {
                Ok(Some(e)) if matches!(e.payload, EventPayload::MessageDelta(_)) => break,
                Ok(Some(e)) if is_turn_end(&e.payload) => {
                    return Err(format!(
                        "Turn endet vor dem Interrupt: {}",
                        e.payload.type_name()
                    ));
                }
                Ok(Some(_)) => {}
                Ok(None) => return Err("Event-Strom endet vor dem Interrupt".into()),
                Err(_) => return Err("kein message.delta vor dem Interrupt".into()),
            }
        }
        s.session
            .interrupt()
            .await
            .map_err(|e| format!("interrupt: {e}"))?;
        let events = until_turn_end(&mut s.rx).await?;
        match events.last() {
            Some(EventPayload::TurnInterrupted(_)) => {}
            other => {
                return Err(format!(
                    "erwartet turn.interrupted, erhalten {:?}",
                    other.map(EventPayload::type_name)
                ));
            }
        }
        // QA-016 AC1: Nach dem Interrupt kommt nichts mehr aus dem abgebrochenen Turn.
        let mut late = Vec::new();
        let deadline = tokio::time::Instant::now() + QUIET_AFTER_INTERRUPT;
        while let Ok(Some(e)) = tokio::time::timeout_at(deadline, s.rx.recv()).await {
            late.push(e.payload);
        }
        if let Some(e) = late.iter().find(|e| is_turn_output(e)) {
            return Err(format!(
                "nach turn.interrupted kam noch {} (Interrupt nicht wirksam)",
                e.type_name()
            ));
        }
        // Danach nimmt die Session eine neue Eingabe an.
        s.session
            .send(UserInput::from("Weiter"))
            .await
            .map_err(|e| format!("send nach Interrupt: {e}"))?;
        let events = until_turn_end(&mut s.rx).await?;
        match events.last() {
            Some(EventPayload::TurnCompleted(_)) => Ok(()),
            other => Err(format!(
                "Turn nach Interrupt endet mit {:?}",
                other.map(EventPayload::type_name)
            )),
        }
    }
    .await;
    stop(s).await;
    outcome(result)
}

async fn check_resume(
    subject: &dyn ContractSubject,
    scenario: &Path,
    caps: &Capabilities,
) -> Outcome {
    if caps.resume == ResumeSupport::None {
        return Outcome::Skipped("resume: none".into());
    }
    let first = async {
        let mut s = start(subject, scenario, None, FixedGate::new(true)).await?;
        let r = async {
            s.session
                .send(UserInput::from("Sag hallo"))
                .await
                .map_err(|e| format!("send: {e}"))?;
            until_turn_end(&mut s.rx).await?;
            s.session
                .native_session_ref()
                .ok_or_else(|| "keine native Session-Referenz nach dem ersten Turn".to_owned())
        }
        .await;
        stop(s).await;
        r
    }
    .await;
    let reference = match first {
        Ok(r) => r,
        Err(e) => return Outcome::Failed(e),
    };
    let resumed = async {
        let mut s = start(
            subject,
            scenario,
            Some(reference.clone()),
            FixedGate::new(true),
        )
        .await?;
        let r = async {
            s.session
                .send(UserInput::from("Sag hallo"))
                .await
                .map_err(|e| format!("send nach Resume: {e}"))?;
            let events = until_turn_end(&mut s.rx).await?;
            if !matches!(events.last(), Some(EventPayload::TurnCompleted(_))) {
                return Err(format!("Turn nach Resume: {:?}", names(&events)));
            }
            // `warm`: dieselbe native Session; `cold`: neue Session (Handover, HAR-018).
            match s.session.native_session_ref() {
                Some(r) if r == reference || caps.resume == ResumeSupport::Cold => Ok(()),
                other => Err(format!("Resume von {reference} liefert Referenz {other:?}")),
            }
        }
        .await;
        stop(s).await;
        r
    }
    .await;
    outcome(resumed)
}

async fn check_model_switch(
    subject: &dyn ContractSubject,
    scenario: &Path,
    caps: &Capabilities,
) -> Outcome {
    let mut s = match start(subject, scenario, None, FixedGate::new(true)).await {
        Ok(s) => s,
        Err(e) => return Outcome::Failed(e),
    };
    let model = caps
        .models
        .last()
        .cloned()
        .unwrap_or_else(|| "contract-model".into());
    let result = s.session.set_model(model.clone(), None).await;
    let checked = async {
        if caps.model_switch == SwitchSupport::None {
            return match result {
                Err(e) if e.code() == "capability_unsupported" => Ok(Some(
                    "model_switch: none, korrekt abgelehnt".to_owned(),
                )),
                other => Err(format!(
                    "model_switch: none deklariert, aber set_model liefert {other:?}"
                )),
            };
        }
        result.map_err(|e| format!("set_model: {e}"))?;
        s.session
            .send(UserInput::from("Sag hallo"))
            .await
            .map_err(|e| format!("send nach set_model: {e}"))?;
        let mut events = Vec::new();
        events.extend(until_turn_end(&mut s.rx).await?);
        let changed = events.iter().any(|e| {
            matches!(e, EventPayload::SessionSettingsChanged(c) if c.model.as_deref() == Some(model.as_str()))
        });
        if !changed {
            return Err("kein session.settings_changed mit dem neuen Modell".into());
        }
        match events.last() {
            Some(EventPayload::TurnCompleted(_)) => Ok(None),
            other => Err(format!(
                "Turn nach Modellwechsel endet mit {:?}",
                other.map(EventPayload::type_name)
            )),
        }
    }
    .await;
    stop(s).await;
    match checked {
        Ok(None) => Outcome::Passed,
        Ok(Some(skip)) => Outcome::Skipped(skip),
        Err(e) => Outcome::Failed(e),
    }
}

async fn check_turn_error(subject: &dyn ContractSubject, scenario: &Path) -> Outcome {
    let mut s = match start(subject, scenario, None, FixedGate::new(true)).await {
        Ok(s) => s,
        Err(e) => return Outcome::Failed(e),
    };
    let result = async {
        s.session
            .send(UserInput::from("Mach einen Fehler"))
            .await
            .map_err(|e| format!("send: {e}"))?;
        let events = until_turn_end(&mut s.rx).await?;
        match events.last() {
            Some(EventPayload::TurnFailed(_)) => Ok(()),
            other => Err(format!(
                "erwartet turn.failed, erhalten {:?}",
                other.map(EventPayload::type_name)
            )),
        }
    }
    .await;
    stop(s).await;
    outcome(result)
}

async fn check_crash(subject: &dyn ContractSubject, scenario: &Path) -> Outcome {
    let mut s = match start(subject, scenario, None, FixedGate::new(true)).await {
        Ok(s) => s,
        Err(e) => return Outcome::Failed(e),
    };
    let result = async {
        s.session
            .send(UserInput::from("Stürz ab"))
            .await
            .map_err(|e| format!("send: {e}"))?;
        let mut events = Vec::new();
        while let Ok(Some(e)) = tokio::time::timeout(EVENT_TIMEOUT, s.rx.recv()).await {
            let exited = matches!(e.payload, EventPayload::HarnessExited(_));
            events.push(e.payload);
            if exited {
                break;
            }
        }
        match events.iter().find_map(|e| match e {
            EventPayload::HarnessExited(x) => Some(x),
            _ => None,
        }) {
            Some(x) if x.code.is_some() || x.signal.is_some() => Ok(()),
            Some(x) => Err(format!("harness.exited ohne code/signal: {x:?}")),
            None => Err(format!("kein harness.exited: {:?}", names(&events))),
        }
    }
    .await;
    stop(s).await;
    outcome(result)
}

async fn check_auth_expired(subject: &dyn ContractSubject, scenario: &Path) -> Outcome {
    let mut s = match start(subject, scenario, None, FixedGate::new(true)).await {
        Ok(s) => s,
        Err(e) => return Outcome::Failed(e),
    };
    let result = async {
        s.session
            .send(UserInput::from("hallo"))
            .await
            .map_err(|e| format!("send: {e}"))?;
        let events = until_turn_end(&mut s.rx).await?;
        if count(&events, "harness.auth_required") != 1 {
            return Err(format!(
                "erwartet genau ein harness.auth_required: {:?}",
                names(&events)
            ));
        }
        match events.last() {
            Some(EventPayload::TurnFailed(_)) => Ok(()),
            other => Err(format!(
                "erwartet turn.failed, erhalten {:?}",
                other.map(EventPayload::type_name)
            )),
        }
    }
    .await;
    stop(s).await;
    outcome(result)
}

#[cfg(test)]
mod tests {
    use beton_core::event::TextDelta;

    use super::*;
    use crate::adapter::{ExitInfo, PermissionMode, SwitchOutcome};
    use crate::fake::FakeAdapter;
    use crate::process::RealLauncher;

    fn fake_subject(adapter: Arc<dyn HarnessAdapter>) -> AdapterSubject {
        AdapterSubject {
            adapter,
            env: Arc::new(|_| HostEnv::default()),
            launcher: Arc::new(RealLauncher),
            workdir: std::env::temp_dir(),
        }
    }

    #[tokio::test]
    async fn qa_016_fake_harness_passes_the_suite() {
        let dir = tempfile::tempdir().unwrap();
        let report = run(&fake_subject(Arc::new(FakeAdapter)), dir.path()).await;
        assert!(report.is_ok(), "{}", report.summary());
        assert_eq!(report.outcome("interrupt"), Some(&Outcome::Passed));
        assert_eq!(report.outcome("approval_deny"), Some(&Outcome::Passed));
    }

    /// Ein Adapter, der `interrupt: true` deklariert, aber nach dem Interrupt weiter
    /// Deltas sendet.
    struct Leaky;

    #[async_trait]
    impl HarnessAdapter for Leaky {
        fn id(&self) -> crate::HarnessId {
            FakeAdapter.id()
        }
        fn modes(&self) -> &[Mode] {
            &[Mode::Native]
        }
        fn capabilities(&self, mode: Mode, probe: &crate::ProbeReport) -> Capabilities {
            FakeAdapter.capabilities(mode, probe)
        }
        async fn probe(&self, env: &HostEnv) -> crate::ProbeReport {
            FakeAdapter.probe(env).await
        }
        async fn start(
            &self,
            spec: SessionSpec,
            ctx: AdapterContext,
        ) -> Result<Box<dyn HarnessSession>, HarnessError> {
            let mut inner = FakeAdapter.start(spec, ctx).await?;
            let mut inner_rx = inner.events().ok_or(HarnessError::Closed)?;
            let (tx, rx) = mpsc::channel(64);
            tokio::spawn(async move {
                while let Some(e) = inner_rx.recv().await {
                    let interrupted = matches!(e.payload, EventPayload::TurnInterrupted(_));
                    let turn = e.turn_id;
                    let _ = tx.send(e).await;
                    if interrupted {
                        let _ = tx
                            .send(NormalizedEvent::new(
                                EventPayload::MessageDelta(TextDelta {
                                    message_id: "zu_spaet".into(),
                                    text: " …und weiter".into(),
                                    snapshot: false,
                                }),
                                turn,
                            ))
                            .await;
                    }
                }
            });
            Ok(Box::new(LeakySession {
                inner,
                rx: Some(rx),
            }))
        }
    }

    struct LeakySession {
        inner: Box<dyn HarnessSession>,
        rx: Option<mpsc::Receiver<NormalizedEvent>>,
    }

    #[async_trait]
    impl HarnessSession for LeakySession {
        async fn send(&mut self, input: UserInput) -> Result<beton_core::id::TurnId, HarnessError> {
            self.inner.send(input).await
        }
        async fn steer(&mut self, input: UserInput) -> Result<(), HarnessError> {
            self.inner.steer(input).await
        }
        async fn interrupt(&mut self) -> Result<(), HarnessError> {
            self.inner.interrupt().await
        }
        async fn set_model(
            &mut self,
            model: String,
            effort: Option<String>,
        ) -> Result<SwitchOutcome, HarnessError> {
            self.inner.set_model(model, effort).await
        }
        async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), HarnessError> {
            self.inner.set_permission_mode(mode).await
        }
        async fn compact(&mut self) -> Result<(), HarnessError> {
            self.inner.compact().await
        }
        fn events(&mut self) -> Option<mpsc::Receiver<NormalizedEvent>> {
            self.rx.take()
        }
        fn native_session_ref(&self) -> Option<String> {
            self.inner.native_session_ref()
        }
        async fn shutdown(self: Box<Self>, how: Shutdown) -> Result<ExitInfo, HarnessError> {
            self.inner.shutdown(how).await
        }
    }

    #[tokio::test]
    async fn qa_016_ac1_deltas_after_interrupt_fail_the_suite() {
        let dir = tempfile::tempdir().unwrap();
        let report = run(&fake_subject(Arc::new(Leaky)), dir.path()).await;
        match report.outcome("interrupt") {
            Some(Outcome::Failed(why)) => assert!(why.contains("message.delta"), "{why}"),
            other => panic!(
                "Interrupt-Prüfung müsste scheitern: {other:?}\n{}",
                report.summary()
            ),
        }
        assert!(!report.is_ok());
    }
}
