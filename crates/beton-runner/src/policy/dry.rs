//! Seiteneffektfreie Auswertung und Set-Snapshots des Policy-Gates (POL-027, AGT-007 AC3).

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
        assert!(drain(&mut s.rx).is_empty(), "Abfragen erzeugen keine Events");

        // Tatsächliche Ausführung: dieselben Entscheidungen.
        let real = s.gate.decide(bash("c1", "git push --force origin main")).await;
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
        assert!(s.gate.resolve("c1", Human { allow: true, ..Human::default() }));
        call.await.unwrap();
        let changes: Vec<_> = drain(&mut s.rx)
            .into_iter()
            .filter_map(|e| match e {
                EventPayload::PolicyStateChanged(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].value, Some(json!(1)), "{changes:?}");
    }

    #[tokio::test]
    async fn agt_007_ac3_query_fails_closed_without_action() {
        let s = setup(PUSH);
        let v = s.gate.query(&json!({})).await;
        assert_eq!(v["decision"], "deny", "{v}");
        let broken = setup("spec_version: 1\nname: push\nrules:\n  - { id: x, on: tool_call, when: 'tool.args.n > 1', action: deny }\n");
        let v = broken
            .gate
            .query(&json!({ "action": "ls" }))
            .await;
        assert_eq!(v["decision"], "deny", "Fehler in einer deny-Regel: fail closed");
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
        sink.fail.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = gate.decide(bash("c0", "ls")).await;
        assert!(sink.stored.lock().unwrap().is_empty());
        // Nach einem Fehler versucht es das Gate bei der nächsten Auswertung erneut.
        sink.fail.store(false, std::sync::atomic::Ordering::SeqCst);
        let _ = gate.decide(bash("c1", "ls")).await;
        let _ = gate.decide(bash("c2", "ls -l")).await;
        assert_eq!(sink.stored.lock().unwrap().len(), 1);
        std::fs::write(&file, PUSH.replace("Kein Force-Push", "Niemals Force-Push")).unwrap();
        let _ = gate.decide(bash("c3", "ls")).await;
        let stored = sink.stored.lock().unwrap();
        assert_eq!(stored.len(), 2);
        assert_ne!(stored[0].policy_set_hash, stored[1].policy_set_hash);
        assert_eq!(stored[1].sets[0].origin, file.display().to_string());
    }
}
