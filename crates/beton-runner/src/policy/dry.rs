//! Seiteneffektfreie Auswertung und Set-Snapshots des Policy-Gates (POL-027, AGT-007 AC3).
//!
//! - **`policy_query`:** wertet eine geplante Aktion genau wie einen echten Tool-Call aus
//!   (gleiche Normalisierung, gleicher Kontext, gleiche Grants, aktueller Stand der Dateien),
//!   aber ohne `policy.decision`, ohne Approval-Card, ohne Effects und ohne Call-Zähler. Die
//!   Antwort nennt Entscheidung, Regeln und Gründe, nicht die Bedingungen der Regeln (das
//!   Modell soll Policies nicht gezielt umgehen können).
//! - **Snapshots:** Vor der ersten Auswertung mit einer neuen Set-Version legt das Gate Manifest
//!   und Inhalte über den Server im Blob-Store der Session ab (`policy.snapshot`), damit
//!   `explain` später genau diese Version zeigt. Ein Fehler verhindert keine Auswertung; das
//!   Gate versucht es bei der nächsten erneut.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use beton_core::event::{EventPayload, Notice, NoticeLevel, PolicyOutcome, PolicyQueried};
use beton_harness::{GateOrigin, GateRequest};
use beton_policy::Phase;
use beton_policy::audit::subject_hash;
use beton_policy::engine::{EffectiveSet, Outcome};
use beton_policy::normalize::{harness_family, normalize_source};
use beton_policy::snapshot::Manifest;
use serde_json::{Value, json};

use super::{PolicyGate, USAGE_TIMEOUT, tool_vars};

/// Höchstzahl der `policy_query`-Abfragen je Turn (Security-Review B1).
pub(super) const QUERY_LIMIT: u32 = 20;

/// Abfragen im laufenden Turn.
#[derive(Debug, Default)]
pub(super) struct QueryBudget {
    turn: i64,
    used: u32,
}

/// Ablage der Set-Snapshots (im Runner: System-Call `policy.snapshot` über den Tunnel).
#[async_trait]
pub trait SnapshotSink: Send + Sync {
    /// Speichert Manifest und Inhalte (Version → Inhalt) eines effektiven Sets.
    async fn store(&self, manifest: &Manifest, sources: &[(String, Vec<u8>)])
    -> Result<(), String>;
}

/// Ziel der Snapshots und bereits gespeicherte Set-Hashes.
#[derive(Default)]
pub(super) struct Snapshots {
    sink: Option<Arc<dyn SnapshotSink>>,
    /// Gespeicherte oder gerade unterwegs befindliche Set-Hashes.
    stored: Arc<Mutex<HashSet<String>>>,
    /// Set aus `session_start`, gesendet erst nach dem Start ([`PolicyGate::flush_snapshot`]).
    deferred: Mutex<Option<Arc<EffectiveSet>>>,
}

/// Shell-Tool des Harness für eine Aktion ohne `tool`: (Name, MCP-Server, ACP-Klasse).
fn shell_tool(harness: &str, action: &str) -> (String, Option<String>, Option<String>) {
    match harness_family(harness) {
        "codex" => ("commandExecution".into(), None, None),
        "acp" => (action.to_owned(), None, Some("execute".into())),
        "direct" => ("shell_exec".into(), Some("beton-workspace".into()), None),
        _ => ("Bash".into(), None, None),
    }
}

fn denied(reason: &str) -> Value {
    json!({
        "decision": "deny",
        "engine": "beton-policy",
        "reason": reason,
        "rules": [],
        "explain": format!("Entscheidung: deny – {reason}"),
    })
}

#[async_trait]
impl crate::mcp::PolicyOracle for PolicyGate {
    async fn query(&self, args: &Value) -> Value {
        PolicyGate::query(self, args).await
    }
}

impl PolicyGate {
    /// Ziel für Set-Snapshots (POL-027 AC2).
    pub fn with_snapshots(self, sink: Arc<dyn SnapshotSink>) -> Self {
        self.rebuild(|i| i.snapshots.sink = Some(sink))
    }

    /// Hinterlegt das Set, falls diese Version noch nicht gespeichert ist.
    pub(super) fn ensure_snapshot(&self, set: &Arc<EffectiveSet>, phase: Phase) {
        if self.inner.snapshots.sink.is_none() {
            return;
        }
        // Vor dem Start ist der Tunnel noch nicht verbunden; ein System-Call würde ihn vorab
        // öffnen (#218). Das Set geht deshalb erst nach dem Start an den Server.
        if phase == Phase::SessionStart {
            if let Ok(mut d) = self.inner.snapshots.deferred.lock() {
                *d = Some(set.clone());
            }
            return;
        }
        self.send_snapshot(set);
    }

    /// Sendet das beim Start vorgemerkte Set (aufzurufen, sobald der Runner den Tunnel bedient).
    pub fn flush_snapshot(&self) {
        let deferred = self
            .inner
            .snapshots
            .deferred
            .lock()
            .ok()
            .and_then(|mut d| d.take());
        if let Some(set) = deferred {
            self.send_snapshot(&set);
        }
    }

    fn send_snapshot(&self, set: &EffectiveSet) {
        let Some(sink) = self.inner.snapshots.sink.clone() else {
            return;
        };
        let stored = self.inner.snapshots.stored.clone();
        let hash = set.hash().to_owned();
        // Neu: als unterwegs vormerken, damit parallele Auswertungen nicht doppelt senden.
        let fresh = stored
            .lock()
            .map(|mut s| s.insert(hash.clone()))
            .unwrap_or(false);
        if !fresh {
            return;
        }
        let manifest = set.manifest();
        let sources = set.sources();
        // Im Hintergrund: Die Auswertung wartet nie auf den Server (der Runner bedient den
        // Tunnel womöglich erst nach dem Start).
        tokio::spawn(async move {
            let ok = matches!(
                tokio::time::timeout(USAGE_TIMEOUT, sink.store(&manifest, &sources)).await,
                Ok(Ok(()))
            );
            if !ok {
                tracing::warn!(
                    "Policy: Set-Snapshot nicht gespeichert, nächster Versuch bei der nächsten Auswertung"
                );
                if let Ok(mut s) = stored.lock() {
                    s.remove(&hash);
                }
            }
        });
    }

    /// Effektives Set mit dem aktuellen Stand der Dateien, ohne Ereignisse (die meldet die
    /// nächste echte Auswertung).
    fn quiet_set(&self) -> Option<Arc<EffectiveSet>> {
        let mut copy = self.inner.store.lock().ok()?.clone();
        let _ = copy.refresh();
        Some(copy.effective())
    }

    /// Zählt eine Abfrage im laufenden Turn; `false`, wenn das Limit erreicht ist.
    fn count_query(&self) -> (bool, bool) {
        let turn = self.inner.settings.lock().map(|s| s.turns).unwrap_or(-1);
        let Ok(mut q) = self.inner.queries.lock() else {
            return (false, false);
        };
        if q.turn != turn {
            *q = QueryBudget { turn, used: 0 };
        }
        q.used = q.used.saturating_add(1);
        (q.used <= QUERY_LIMIT, q.used == QUERY_LIMIT + 1)
    }

    /// Protokolliert eine Abfrage ohne Klartext (`policy.queried`, Security-Review B1).
    fn log_query(
        &self,
        subject: &Value,
        tool: &str,
        kind: &str,
        outcome: PolicyOutcome,
        rules: Vec<String>,
        limited: bool,
    ) {
        self.emit(
            EventPayload::PolicyQueried(PolicyQueried {
                subject_hash: subject_hash(subject),
                tool: tool.to_owned(),
                kind: kind.to_owned(),
                outcome,
                rules,
                limited,
            }),
            None,
            None,
        );
    }

    /// `policy_query` (AGT-007 AC3, POL-027 AC3): `{action, tool?, server?, args?}` →
    /// Entscheidung wie bei tatsächlicher Ausführung, ohne Seiteneffekte auf Policies. Jede
    /// Abfrage steht als `policy.queried` im Log; je Turn sind höchstens [`QUERY_LIMIT`]
    /// erlaubt. Die Antwort nennt nur `deny`/`ask`-Gründe, damit das Modell Regeln nicht
    /// ausforschen kann. Fehler lehnen ab (fail closed).
    pub async fn query(&self, args: &Value) -> Value {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        let harness = self.inner.opts.harness.as_str().to_owned();
        let (tool, tool_args, mcp_server, acp_kind) = match args.get("tool").and_then(Value::as_str)
        {
            Some(t) => {
                let explicit = args
                    .get("server")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty() && *s != "builtin")
                    .map(str::to_owned);
                let (name, server) = match explicit {
                    Some(server) => (t.to_owned(), Some(server)),
                    None => split_mcp(&harness, t),
                };
                (
                    name,
                    args.get("args").cloned().unwrap_or_else(|| json!({})),
                    server,
                    None,
                )
            }
            None if !action.is_empty() => {
                let (name, server, acp) = shell_tool(&harness, action);
                (name, json!({ "command": action }), server, acp)
            }
            None => {
                self.log_query(args, "", "", PolicyOutcome::Deny, Vec::new(), false);
                return denied("`action` fehlt");
            }
        };
        let subject = json!({ "tool": tool, "server": mcp_server, "args": tool_args });
        let kind =
            beton_harness::tool_kind(&harness, &tool, acp_kind.as_deref(), mcp_server.as_deref());
        let (allowed, crossed) = self.count_query();
        if !allowed {
            if crossed {
                self.emit(
                    EventPayload::Notice(Notice {
                        level: NoticeLevel::Warn,
                        text: format!(
                            "policy_query: mehr als {QUERY_LIMIT} Abfragen in diesem Turn; weitere werden abgelehnt"
                        ),
                    }),
                    None,
                    None,
                );
            }
            self.log_query(
                &subject,
                &tool,
                &kind,
                PolicyOutcome::Deny,
                Vec::new(),
                true,
            );
            return denied(&format!(
                "Zu viele Abfragen in diesem Turn (höchstens {QUERY_LIMIT})"
            ));
        }
        let req = GateRequest {
            call_id: "policy_query".into(),
            tool,
            kind,
            args: tool_args,
            mcp_server,
            acp_kind,
            origin: GateOrigin::PreTool,
            ..GateRequest::default()
        };
        let Some(set) = self.quiet_set() else {
            self.log_query(
                &subject,
                &req.tool,
                &req.kind,
                PolicyOutcome::Deny,
                Vec::new(),
                false,
            );
            return denied("Policy-Zustand nicht lesbar");
        };
        let source = self.tool_source(&req);
        let mut n = match normalize_source(&source, &req.args) {
            Ok(n) => n,
            Err(e) => {
                self.log_query(
                    &subject,
                    &req.tool,
                    &req.kind,
                    PolicyOutcome::Deny,
                    Vec::new(),
                    false,
                );
                return denied(&format!("Tool-Call nicht prüfbar: {e}"));
            }
        };
        // Wie beim echten Call: dieser wäre der nächste.
        let call_index = self.inner.calls.load(std::sync::atomic::Ordering::SeqCst) + 1;
        let mut vars = self.base_vars("agent");
        if let Some(s) = vars.session.as_mut() {
            s.tool_calls = Some(call_index);
        }
        super::adapter_kind(&mut n, &req);
        vars.tool = Some(tool_vars(&n, Some(call_index)));
        self.with_usage_values(&set, Phase::ToolCall, &mut vars)
            .await;
        self.with_state_values(&set, Phase::ToolCall, &mut vars)
            .await;
        let decision = self
            .run_engine(&set, Phase::ToolCall, vars, Some(source))
            .await;
        let (verdict, outcome, reasons) = match &decision.outcome {
            Outcome::Allow => ("allow", PolicyOutcome::Allow, &[][..]),
            Outcome::Deny { reasons } => ("deny", PolicyOutcome::Deny, reasons.as_slice()),
            Outcome::Ask { reasons } => ("ask", PolicyOutcome::Ask, reasons.as_slice()),
        };
        // Nur die Regeln der Gründe von deny/ask; gegriffene allow-, modify- und
        // notify-Regeln bleiben verborgen.
        let mut rules: Vec<String> = Vec::new();
        for r in reasons {
            if !rules.contains(&r.rule.rule_id) {
                rules.push(r.rule.rule_id.clone());
            }
        }
        self.log_query(&subject, &req.tool, &n.kind, outcome, rules.clone(), false);
        let reason = reasons
            .iter()
            .map(|r| format!("{} ({})", r.reason, r.rule.rule_id))
            .collect::<Vec<_>>()
            .join("; ");
        let explain = if reason.is_empty() {
            format!("Entscheidung: {verdict}")
        } else {
            format!("Entscheidung: {verdict} – {reason}")
        };
        json!({
            "decision": verdict,
            "engine": "beton-policy",
            "action": action,
            "tool": { "name": n.name, "kind": n.kind },
            "rules": rules,
            "reason": reason,
            // Nur ob, nicht wie geändert würde.
            "modified": !decision.modified.is_empty(),
            "explain": explain,
        })
    }
}

/// MCP-Tool im Claude-Format (`mcp__<server>__<tool>`) für Harnesses, die den Server getrennt
/// melden (Codex, Direkt-API, ACP): Name und Server wie beim echten Call.
fn split_mcp(harness: &str, tool: &str) -> (String, Option<String>) {
    if harness_family(harness) != "claude"
        && harness_family(harness) != "fake"
        && let Some(rest) = tool.strip_prefix("mcp__")
        && let Some((server, name)) = rest.split_once("__")
        && !server.is_empty()
        && !name.is_empty()
    {
        return (name.to_owned(), Some(server.to_owned()));
    }
    (tool.to_owned(), None)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use beton_core::event::EventPayload;
    use beton_harness::{Gate, GateDecision, GateOrigin, GateRequest};
    use beton_policy::load::Scope;
    use beton_policy::snapshot::Manifest;
    use serde_json::json;
    use tokio::sync::mpsc;

    use super::super::{GateEvent, Human, PolicyGate, PolicyOptions, SnapshotSink};

    const PUSH: &str = "spec_version: 1
name: push
rules:
  - id: no-force
    on: tool_call
    when: 'shell.has_command(tool.argv, \"git\", \"push\") && shell.has_flag(tool.argv, \"--force\")'
    action: deny
    reason: Kein Force-Push
  - id: confirm-push
    on: tool_call
    when: 'shell.has_command(tool.argv, \"git\", \"push\")'
    action: ask
    reason: Push bestätigen
    effects:
      - { op: increment, scope: session, key: pushes }
  - id: count
    on: tool_call
    action: notify
    notify: { channels: [inbox, log] }
";

    struct Setup {
        gate: PolicyGate,
        rx: mpsc::UnboundedReceiver<GateEvent>,
        file: std::path::PathBuf,
        _dir: tempfile::TempDir,
    }

    fn setup(body: &str) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let pdir = dir.path().join("policies");
        std::fs::create_dir_all(&pdir).unwrap();
        let file = pdir.join("push.yaml");
        std::fs::write(&file, body).unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        let gate = PolicyGate::new(
            PolicyOptions {
                dirs: vec![(Scope::Project, pdir)],
                workdir: dir.path().to_path_buf(),
                ..PolicyOptions::default()
            },
            tx,
        );
        Setup {
            gate,
            rx,
            file,
            _dir: dir,
        }
    }

    fn bash(call: &str, command: &str) -> GateRequest {
        GateRequest {
            call_id: call.into(),
            tool: "Bash".into(),
            kind: "shell".into(),
            args: json!({ "command": command }),
            origin: GateOrigin::PreTool,
            ..GateRequest::default()
        }
    }

    /// Zahl der `policy.queried`; jedes andere Event ist ein Fehler.
    fn queried(events: &[EventPayload]) -> usize {
        for e in events {
            assert!(
                matches!(e, EventPayload::PolicyQueried(_)),
                "unerwartetes Event: {e:?}"
            );
        }
        events.len()
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<GateEvent>) -> Vec<EventPayload> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e.payload);
        }
        out
    }

    #[tokio::test]
    async fn agt_007_ac3_policy_query_gives_the_decision_of_the_real_call_for_git_push() {
        let mut s = setup(PUSH);
        // Abfragen: keine Events (auch kein `policy.set_changed`), keine Card, kein Zustand.
        let ask = s
            .gate
            .query(&json!({ "action": "git push origin main" }))
            .await;
        assert_eq!(ask["decision"], "ask", "{ask}");
        assert!(
            ask["rules"]
                .as_array()
                .unwrap()
                .contains(&json!("confirm-push")),
            "{ask}"
        );
        assert!(ask["explain"].as_str().unwrap().contains("confirm-push"));
        let deny = s
            .gate
            .query(&json!({ "action": "git push --force origin main" }))
            .await;
        assert_eq!(deny["decision"], "deny", "{deny}");
        assert!(deny["reason"].as_str().unwrap().contains("Kein Force-Push"));
        let allow = s
            .gate
            .query(&json!({ "action": "x", "tool": "Bash", "args": { "command": "ls" } }))
            .await;
        assert_eq!(allow["decision"], "allow", "{allow}");
        // Nur das Protokoll der Abfragen (Security-Review B1), keine Entscheidung, keine Card.
        assert_eq!(
            queried(&drain(&mut s.rx)),
            3,
            "Abfragen erzeugen nur `policy.queried`"
        );

        // Tatsächliche Ausführung: dieselben Entscheidungen.
        let real = s
            .gate
            .decide(bash("c1", "git push --force origin main"))
            .await;
        assert!(matches!(real, GateDecision::Deny { .. }), "{real:?}");
        let g = s.gate.clone();
        let call = tokio::spawn(async move { g.decide(bash("c2", "git push origin main")).await });
        let card = loop {
            let e = tokio::time::timeout(std::time::Duration::from_secs(10), s.rx.recv())
                .await
                .unwrap()
                .unwrap();
            if let EventPayload::ApprovalRequested(r) = e.payload {
                break r;
            }
        };
        assert_eq!(card.subject["reasons"][0]["rule_id"], "confirm-push");
        assert!(s.gate.resolve("c2", Human::default()));
        assert!(matches!(call.await.unwrap(), GateDecision::Deny { .. }));
        let real_allow = s.gate.decide(bash("c3", "ls")).await;
        assert!(matches!(real_allow, GateDecision::Allow { .. }));
    }

    #[tokio::test]
    async fn pol_027_ac3_query_changes_no_counter_state_or_grant() {
        let mut s = setup(PUSH);
        for _ in 0..3 {
            let _ = s
                .gate
                .query(&json!({ "action": "git push origin main" }))
                .await;
        }
        assert_eq!(queried(&drain(&mut s.rx)), 3);
        // Der erste echte Call ist weiterhin Call Nr. 1, ohne gemerkten Zustand.
        let g = s.gate.clone();
        let call = tokio::spawn(async move { g.decide(bash("c1", "git push origin main")).await });
        let mut decision = None;
        let mut card = None;
        while card.is_none() {
            let e = tokio::time::timeout(std::time::Duration::from_secs(10), s.rx.recv())
                .await
                .unwrap()
                .unwrap();
            match e.payload {
                EventPayload::PolicyDecision(d) => decision = Some(d),
                EventPayload::ApprovalRequested(r) => card = Some(r),
                _ => {}
            }
        }
        assert!(decision.is_some());
        assert!(s.gate.resolve(
            "c1",
            Human {
                allow: true,
                ..Human::default()
            }
        ));
        call.await.unwrap();
        let changes: Vec<_> = drain(&mut s.rx)
            .into_iter()
            .filter_map(|e| match e {
                EventPayload::PolicyStateChanged(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].value, json!(1), "{changes:?}");
    }

    #[tokio::test]
    async fn agt_007_ac3_query_fails_closed_without_action() {
        let s = setup(PUSH);
        let v = s.gate.query(&json!({})).await;
        assert_eq!(v["decision"], "deny", "{v}");
        let broken = setup(
            "spec_version: 1\nname: push\nrules:\n  - { id: x, on: tool_call, when: 'tool.args.n > 1', action: deny }\n",
        );
        let v = broken.gate.query(&json!({ "action": "ls" })).await;
        assert_eq!(
            v["decision"], "deny",
            "Fehler in einer deny-Regel: fail closed"
        );
    }

    #[derive(Default)]
    struct Sink {
        stored: Mutex<Vec<Manifest>>,
        fail: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl SnapshotSink for Sink {
        async fn store(
            &self,
            manifest: &Manifest,
            sources: &[(String, Vec<u8>)],
        ) -> Result<(), String> {
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("nicht erreichbar".into());
            }
            let versions: HashSet<&String> = manifest.sets.iter().map(|s| &s.version).collect();
            assert_eq!(versions.len(), sources.len());
            for (v, _) in sources {
                assert!(versions.contains(v));
            }
            self.stored.lock().unwrap().push(manifest.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn pol_027_ac2_gate_stores_one_snapshot_per_set_version() {
        let Setup {
            gate,
            rx: _rx,
            file,
            _dir,
        } = setup(PUSH);
        let sink = Arc::new(Sink::default());
        let gate = gate.with_snapshots(sink.clone());
        let settle = || tokio::time::sleep(std::time::Duration::from_millis(200));
        sink.fail.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = gate.decide(bash("c0", "ls")).await;
        settle().await;
        assert!(sink.stored.lock().unwrap().is_empty());
        // Nach einem Fehler versucht es das Gate bei der nächsten Auswertung erneut.
        sink.fail.store(false, std::sync::atomic::Ordering::SeqCst);
        let _ = gate.decide(bash("c1", "ls")).await;
        let _ = gate.decide(bash("c2", "ls -l")).await;
        settle().await;
        assert_eq!(sink.stored.lock().unwrap().len(), 1);
        std::fs::write(&file, PUSH.replace("Kein Force-Push", "Niemals Force-Push")).unwrap();
        let _ = gate.decide(bash("c3", "ls")).await;
        settle().await;
        let stored = sink.stored.lock().unwrap();
        assert_eq!(stored.len(), 2);
        assert_ne!(stored[0].policy_set_hash, stored[1].policy_set_hash);
        assert_eq!(stored[1].sets[0].origin, file.display().to_string());
    }

    #[tokio::test]
    async fn pol_027_ac2_snapshot_from_session_start_waits_for_the_runner_loop() {
        let Setup {
            gate,
            rx: _rx,
            _dir,
            ..
        } = setup(PUSH);
        let sink = Arc::new(Sink::default());
        let gate = gate.with_snapshots(sink.clone());
        let set = gate.refresh(None);
        gate.ensure_snapshot(&set, beton_policy::Phase::SessionStart);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            sink.stored.lock().unwrap().is_empty(),
            "vor dem Start kein System-Call"
        );
        gate.flush_snapshot();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(sink.stored.lock().unwrap().len(), 1);
        gate.flush_snapshot();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(sink.stored.lock().unwrap().len(), 1, "nur einmal");
    }

    const PROBE: &str = "spec_version: 1
name: probe
rules:
  - id: watch-secret
    on: tool_call
    when: 'tool.command.contains(\"geheim\")'
    action: notify
    notify: { channels: [inbox] }
  - id: patch
    on: tool_call
    when: 'tool.kind == \"shell\"'
    action: modify
    modify: { args: { timeout: 5 } }
  - id: allow-ls
    on: tool_call
    when: 'tool.command == \"ls\"'
    action: allow
  - id: no-rm
    on: tool_call
    when: 'tool.command.contains(\"rm\")'
    action: deny
    reason: Kein rm
";

    #[tokio::test]
    async fn agt_007_ac3_query_reveals_only_deny_and_ask_reasons() {
        let s = setup(PROBE);
        let v = s.gate.query(&json!({ "action": "cat geheim.txt" })).await;
        assert_eq!(v["decision"], "allow", "{v}");
        assert_eq!(
            v["rules"],
            json!([]),
            "allow/modify/notify verraten nichts: {v}"
        );
        assert_eq!(v["modified"], json!(true), "nur ob, nicht was: {v}");
        assert!(v.get("policy_set_hash").is_none(), "{v}");
        let text = v.to_string();
        for hidden in ["watch-secret", "patch", "allow-ls", "timeout"] {
            assert!(!text.contains(hidden), "`{hidden}` in {v}");
        }
        let v = s.gate.query(&json!({ "action": "rm geheim.txt" })).await;
        assert_eq!(v["decision"], "deny");
        assert_eq!(v["rules"], json!(["no-rm"]), "{v}");
        assert!(!v.to_string().contains("watch-secret"), "{v}");
    }

    #[tokio::test]
    async fn agt_007_ac3_every_query_is_logged_without_plaintext() {
        let mut s = setup(PROBE);
        let _ = s.gate.query(&json!({ "action": "rm geheim-4711" })).await;
        let events = drain(&mut s.rx);
        assert_eq!(queried(&events), 1);
        let EventPayload::PolicyQueried(q) = &events[0] else {
            unreachable!()
        };
        assert!(q.subject_hash.starts_with("sha256:"), "{q:?}");
        assert_eq!(q.outcome, beton_core::event::PolicyOutcome::Deny);
        assert_eq!(q.rules, vec!["no-rm".to_owned()]);
        assert_eq!(q.tool, "Bash");
        assert_eq!(q.kind, "shell");
        assert!(!q.limited);
        let text = serde_json::to_string(q).unwrap();
        assert!(!text.contains("geheim-4711"), "{text}");
    }

    #[tokio::test]
    async fn agt_007_ac3_queries_are_limited_per_turn() {
        let mut s = setup(PROBE);
        for i in 0..super::QUERY_LIMIT {
            let v = s.gate.query(&json!({ "action": "ls" })).await;
            assert_eq!(v["decision"], "allow", "Abfrage {i}: {v}");
        }
        let limited = s.gate.query(&json!({ "action": "ls" })).await;
        assert_eq!(limited["decision"], "deny", "{limited}");
        assert!(
            limited["reason"].as_str().unwrap().contains("Abfragen"),
            "{limited}"
        );
        let events = drain(&mut s.rx);
        let last = events
            .iter()
            .rev()
            .find_map(|e| match e {
                EventPayload::PolicyQueried(q) => Some(q),
                _ => None,
            })
            .unwrap();
        assert!(last.limited);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, EventPayload::Notice(n) if n.text.contains("policy_query"))),
            "Hinweis für Menschen: {events:?}"
        );
        // Ein neuer Turn setzt das Limit zurück.
        s.gate.note_turn();
        let v = s.gate.query(&json!({ "action": "ls" })).await;
        assert_eq!(v["decision"], "allow", "{v}");
    }

    fn codex(body: &str) -> Setup {
        let mut s = setup(body);
        let (tx, rx) = mpsc::unbounded_channel();
        let workdir = s._dir.path().to_path_buf();
        s.gate = PolicyGate::new(
            PolicyOptions {
                harness: "codex".parse().unwrap(),
                dirs: vec![(Scope::Project, workdir.join("policies"))],
                workdir,
                ..PolicyOptions::default()
            },
            tx,
        );
        s.rx = rx;
        s
    }

    #[tokio::test]
    async fn agt_007_ac3_query_classifies_mcp_tools_like_the_real_call() {
        let s = codex("spec_version: 1
name: mcp
rules:
  - id: no-github-pr
    on: tool_call
    when: 'tool.kind == \"mcp\" && tool.server == \"github\" && tool.name == \"create_pull_request\"'
    action: deny
    reason: Keine PRs
");
        let real = s
            .gate
            .decide(GateRequest {
                call_id: "m1".into(),
                tool: "create_pull_request".into(),
                mcp_server: Some("github".into()),
                args: json!({ "title": "x" }),
                origin: GateOrigin::PreTool,
                ..GateRequest::default()
            })
            .await;
        assert!(matches!(real, GateDecision::Deny { .. }), "{real:?}");
        for args in [
            json!({ "action": "PR anlegen", "tool": "create_pull_request", "server": "github", "args": { "title": "x" } }),
            json!({ "action": "PR anlegen", "tool": "mcp__github__create_pull_request", "args": { "title": "x" } }),
        ] {
            let v = s.gate.query(&args).await;
            assert_eq!(v["decision"], "deny", "{args}: {v}");
            assert_eq!(v["rules"], json!(["no-github-pr"]), "{v}");
        }
    }
}
