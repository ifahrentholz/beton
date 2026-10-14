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
use beton_harness::{GateOrigin, GateRequest};
use beton_policy::Phase;
use beton_policy::engine::{EffectiveSet, Outcome};
use beton_policy::normalize::{harness_family, normalize_source};
use beton_policy::snapshot::Manifest;
use serde_json::{Value, json};

use super::{PolicyGate, USAGE_TIMEOUT, tool_vars};

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

    /// `policy_query` (AGT-007 AC3, POL-027 AC3): `{action, tool?, args?}` → Entscheidung wie
    /// bei tatsächlicher Ausführung, ohne Seiteneffekte. Fehler lehnen ab (fail closed).
    pub async fn query(&self, args: &Value) -> Value {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        let (tool, tool_args, mcp_server, acp_kind) = match args.get("tool").and_then(Value::as_str)
        {
            Some(t) => (
                t.to_owned(),
                args.get("args").cloned().unwrap_or_else(|| json!({})),
                None,
                None,
            ),
            None if !action.is_empty() => {
                let (name, server, acp) = shell_tool(self.inner.opts.harness.as_str(), action);
                (name, json!({ "command": action }), server, acp)
            }
            None => return denied("`action` fehlt"),
        };
        let kind = beton_harness::tool_kind(
            self.inner.opts.harness.as_str(),
            &tool,
            acp_kind.as_deref(),
            mcp_server.as_deref(),
        );
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
            return denied("Policy-Zustand nicht lesbar");
        };
        let source = self.tool_source(&req);
        let mut n = match normalize_source(&source, &req.args) {
            Ok(n) => n,
            Err(e) => return denied(&format!("Tool-Call nicht prüfbar: {e}")),
        };
        // Wie beim echten Call: dieser wäre der nächste.
        let call_index = self.inner.calls.load(std::sync::atomic::Ordering::SeqCst) + 1;
        let mut vars = self.base_vars("agent");
        if let Some(s) = vars.session.as_mut() {
            s.tool_calls = Some(call_index);
        }
        vars.tool = Some(tool_vars(&mut n, &req.kind, call_index));
        self.with_usage_values(&set, Phase::ToolCall, &mut vars)
            .await;
        self.with_state_values(&set, Phase::ToolCall, &mut vars)
            .await;
        let decision = self
            .run_engine(&set, Phase::ToolCall, vars, Some(source))
            .await;
        let (verdict, reasons) = match &decision.outcome {
            Outcome::Allow => ("allow", &[][..]),
            Outcome::Deny { reasons } => ("deny", reasons.as_slice()),
            Outcome::Ask { reasons } => ("ask", reasons.as_slice()),
        };
        let mut rules: Vec<String> = reasons.iter().map(|r| r.rule.rule_id.clone()).collect();
        for m in &decision.matched {
            if !rules.contains(&m.rule.rule_id) {
                rules.push(m.rule.rule_id.clone());
            }
        }
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
        let mut out = json!({
            "decision": verdict,
            "engine": "beton-policy",
            "action": action,
            "tool": { "name": n.name, "kind": n.kind },
            "rules": rules,
            "reason": reason,
            "policy_set_hash": decision.policy_set_hash,
            "explain": explain,
        });
        if !decision.modified.is_empty() {
            out["modified"] = serde_json::to_value(&decision.modified).unwrap_or(Value::Null);
        }
        out
    }
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
        assert!(
            drain(&mut s.rx).is_empty(),
            "Abfragen erzeugen keine Events"
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
        assert!(drain(&mut s.rx).is_empty());
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
}
