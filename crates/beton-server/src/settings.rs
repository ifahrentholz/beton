//! Modell, Effort und Permission-Mode einer Session (HAR-017, HAR-027) sowie das Aufräumen
//! nach einem Runner-Neustart (HAR-020).
//!
//! Der Server prüft Einstellungen vor dem Anlegen (Session, Fork) und vor dem Weiterreichen an
//! den Runner: `yolo` ohne Sandbox lehnt er immer mit `sandbox_required` ab (fail closed), was
//! der Katalog des Harness nicht kennt, mit `capability_unsupported`. Die endgültige Prüfung
//! gegen die Capabilities der laufenden Session macht der Runner.

use beton_core::event::{
    Actor, ApprovalDecision, ApprovalResolved, Event, EventPayload, ResolvedVia, SystemComponent,
    TurnFailed,
};
use beton_core::id::{ApprovalId, SessionId, TurnId};
use beton_harness::{Capabilities, EFFORT_LEVELS, PermissionMode, SandboxStatus};
use serde_json::{Value, json};

use crate::problem::{Problem, ProblemCode};
use crate::sessions::SessionManager;

/// Wirksame Einstellungen einer Session laut Log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
}

/// `session.created` und alle späteren `session.settings_changed`, in Log-Reihenfolge.
/// Ein Neustart des Runners (HAR-020) startet mit diesem Stand, nicht mit dem ursprünglichen.
pub fn current<'a>(events: impl IntoIterator<Item = &'a Event>) -> Settings {
    let mut s = Settings::default();
    for e in events {
        match e.payload() {
            Some(EventPayload::SessionCreated(c)) => {
                s.model.clone_from(&c.model);
                s.effort.clone_from(&c.effort);
                s.permission_mode.clone_from(&c.permission_mode);
            }
            Some(EventPayload::SessionSettingsChanged(c)) => {
                if c.model.is_some() {
                    s.model.clone_from(&c.model);
                }
                if c.effort.is_some() {
                    s.effort.clone_from(&c.effort);
                }
                if c.permission_mode.is_some() {
                    s.permission_mode.clone_from(&c.permission_mode);
                }
            }
            _ => {}
        }
    }
    s
}

fn invalid(detail: String) -> Problem {
    Problem::new(ProblemCode::ValidationFailed).detail(detail)
}

/// Permission-Mode aus einer Anfrage; `yolo` ohne Sandbox und Egress-Proxy ist nie erlaubt.
pub fn permission_mode(raw: &str) -> Result<PermissionMode, Problem> {
    let mode: PermissionMode = raw.parse().map_err(|e| invalid(format!("{e}")))?;
    if mode == PermissionMode::Yolo && !SandboxStatus::current().allows_yolo() {
        return Err(Problem::new(ProblemCode::SandboxRequired).detail(
            "YOLO startet nur mit Tool-Sandbox (Stufe 2) und Egress-Proxy; beides ist auf diesem \
             Rechner nicht aktiv.",
        ));
    }
    Ok(mode)
}

/// Prüft Effort und Permission-Mode beim Anlegen einer Session bzw. eines Forks gegen den
/// Katalog des Harness (`caps`, falls bekannt).
pub fn validate_start(
    caps: Option<&Capabilities>,
    effort: Option<&str>,
    mode: Option<&str>,
) -> Result<(), Problem> {
    if let Some(raw) = mode {
        let mode = permission_mode(raw)?;
        if let Some(caps) = caps
            && !caps.permission_modes.contains(&mode)
        {
            return Err(
                Problem::new(ProblemCode::CapabilityUnsupported).detail(format!(
                    "Dieser Harness kennt den Permission-Mode `{mode}` nicht."
                )),
            );
        }
    }
    if let Some(effort) = effort {
        if !EFFORT_LEVELS.contains(&effort) {
            return Err(invalid(format!(
                "Effort `{effort}` gibt es nicht (low, medium, high, xhigh)"
            )));
        }
        if caps.is_some_and(|c| c.efforts.is_empty()) {
            return Err(Problem::new(ProblemCode::CapabilityUnsupported)
                .detail("Dieser Harness kennt keine Effort-Stufen."));
        }
    }
    Ok(())
}

/// Vorprüfung von `session.set` bzw. `PATCH /v1/sessions/{id}`, bevor etwas den Runner
/// erreicht: Werte müssen gültig sein, `yolo` scheitert ohne Sandbox.
pub fn precheck_switch(args: &Value) -> Result<(), Problem> {
    for key in ["model", "effort", "permission_mode"] {
        match &args[key] {
            Value::Null => {}
            Value::String(s) if !s.trim().is_empty() => {}
            _ => return Err(invalid(format!("`{key}` muss ein nicht leerer Text sein"))),
        }
    }
    if let Some(raw) = args["permission_mode"].as_str() {
        permission_mode(raw)?;
    }
    if let Some(effort) = args["effort"].as_str()
        && !EFFORT_LEVELS.contains(&effort)
    {
        return Err(invalid(format!(
            "Effort `{effort}` gibt es nicht (low, medium, high, xhigh)"
        )));
    }
    Ok(())
}

/// Was beim Ende eines Runners offen war: ein laufender Turn und unbeantwortete Freigaben.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Interrupted {
    pub turn: Option<TurnId>,
    pub approvals: Vec<ApprovalId>,
}

impl Interrupted {
    pub fn is_empty(&self) -> bool {
        self.turn.is_none() && self.approvals.is_empty()
    }
}

/// Offener Turn und offene Freigaben laut Log (HAR-020 AC3).
pub fn interrupted<'a>(events: impl IntoIterator<Item = &'a Event>) -> Interrupted {
    let mut out = Interrupted::default();
    for e in events {
        match e.payload() {
            Some(EventPayload::TurnStarted(t)) => out.turn = Some(t.turn_id),
            Some(
                EventPayload::TurnCompleted(_)
                | EventPayload::TurnFailed(_)
                | EventPayload::TurnInterrupted(_),
            ) => out.turn = None,
            Some(EventPayload::ApprovalRequested(a)) => out.approvals.push(a.approval_id),
            Some(EventPayload::ApprovalResolved(a)) => {
                out.approvals.retain(|id| *id != a.approval_id);
            }
            _ => {}
        }
    }
    out
}

/// Events, die einen abgebrochenen Runner im Log abschließen: offene Freigaben als
/// `approval.resolved {decision: abort, via: system}`, der laufende Turn als `turn.failed`
/// mit Problem `runner_restarted` (HAR-020 AC3).
pub fn closing_events(session: SessionId, open: &Interrupted) -> Vec<Event> {
    let system = Actor::System {
        component: SystemComponent::Server,
    };
    let mut out = Vec::new();
    for approval in &open.approvals {
        let mut e = Event::new(
            session,
            0,
            system.clone(),
            EventPayload::ApprovalResolved(ApprovalResolved {
                approval_id: *approval,
                decision: ApprovalDecision::Abort,
                answer: None,
                actor: system.clone(),
                via: ResolvedVia::System,
                remember: None,
                comment: Some("Runner neu gestartet; Freigabe verworfen".into()),
                on_timeout_applied: None,
            }),
        );
        e.turn_id = open.turn;
        out.push(e);
    }
    if let Some(turn) = open.turn {
        let mut e = Event::new(
            session,
            0,
            system,
            EventPayload::TurnFailed(TurnFailed {
                turn_id: turn,
                problem: json!({
                    "type": "urn:beton:problem:runner_restarted",
                    "code": "runner_restarted",
                    "title": "Runner neu gestartet; der laufende Turn wurde abgebrochen",
                }),
            }),
        );
        e.turn_id = Some(turn);
        out.push(e);
    }
    out
}

impl SessionManager<'_> {
    /// Capabilities des Harness laut Katalog (ohne Probe); `None`, wenn der Harness auf diesem
    /// Host unbekannt ist.
    pub(crate) fn catalog_caps(
        &self,
        target: &beton_harness::HarnessId,
        workdir: &std::path::Path,
    ) -> Option<Capabilities> {
        let probe = beton_harness::ProbeReport::default();
        let mode = beton_harness::Mode::Native;
        if target.as_str() == beton_harness::HarnessId::FAKE {
            return Some(beton_harness::fake::default_capabilities());
        }
        if let Some(a) = self.state().runtime.harnesses.get(target) {
            return Some(a.capabilities(mode, &probe));
        }
        // ACP-Agents (HAR-008) und Direkt-API-Provider (HAR-011) aus der Konfiguration.
        let layers = beton_harness::registry::HarnessLayers {
            user: self.cfg().harnesses_user.clone(),
            project: beton_harness::registry::HarnessesConfig::load_project(workdir)
                .unwrap_or_default(),
            user_file: None,
            project_file: None,
            // Direkt-API-Provider nur aus der User-Konfiguration (HAR-011).
            providers: self.cfg().providers.clone(),
        };
        let mut r =
            beton_harness::registry::Registry::new(beton_harness::registry::RegistryOptions {
                dev: false,
            });
        beton_harness_acp::register(&mut r, &layers);
        beton_harness_direct::register(
            &mut r,
            &layers.providers,
            &beton_harness_direct::DirectOptions::default(),
        );
        r.get(target).map(|a| a.capabilities(mode, &probe))
    }

    /// Schließt im Log ab, was ein beendeter Runner offen ließ (HAR-020 AC3), und gibt die
    /// Queue frei. Liefert `true`, wenn etwas offen war.
    pub(crate) async fn close_interrupted(&self, session: SessionId) -> Result<bool, Problem> {
        let events = self.all_events(session).await?;
        let open = interrupted(&events);
        if open.is_empty() {
            return Ok(false);
        }
        let batch = closing_events(session, &open);
        let store = &self.state().store;
        let written = loop {
            let record = store.session(self.org(), session).await?;
            match self
                .state()
                .events()
                .append(
                    self.org(),
                    session,
                    record.head_seq,
                    record.epoch,
                    batch.clone(),
                )
                .await
            {
                Err(beton_store::Error::SeqConflict { .. }) => continue,
                other => break other.map_err(Problem::from)?,
            }
        };
        self.state().queue().observe(session, &written);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use beton_core::event::{
        ApprovalKind, ApprovalRequested, SessionCreated, SessionKind, SessionSettingsChanged,
        SessionTrigger, TimeoutAction, TurnCompleted, TurnStarted,
    };
    use beton_core::id::{PrincipalId, UserId};
    use beton_core::time::Timestamp;

    use super::*;

    fn ev(payload: EventPayload) -> Event {
        Event::new(SessionId::new(), 0, Actor::default(), payload)
    }

    fn caps(modes: &[PermissionMode], efforts: &[&str]) -> Capabilities {
        let mut c = Capabilities::minimal(
            beton_harness::Mode::Native,
            beton_harness::Transport::Native,
        );
        c.permission_modes = modes.to_vec();
        c.efforts = efforts.iter().map(|e| (*e).to_owned()).collect();
        c
    }

    #[test]
    fn har_027_ac1_yolo_is_sandbox_required_at_create_and_switch() {
        let all = caps(&PermissionMode::ALL, &["low"]);
        let p = validate_start(Some(&all), None, Some("yolo")).unwrap_err();
        assert_eq!(p.code, ProblemCode::SandboxRequired);
        // Auch ohne bekannten Katalog und beim Wechsel.
        let p = validate_start(None, None, Some("yolo")).unwrap_err();
        assert_eq!(p.code, ProblemCode::SandboxRequired);
        let p = precheck_switch(&json!({"permission_mode": "yolo"})).unwrap_err();
        assert_eq!(p.code, ProblemCode::SandboxRequired);
        assert_eq!(p.code.status().as_u16(), 409);
    }

    #[test]
    fn har_027_unknown_or_unsupported_modes_are_rejected() {
        let p = validate_start(None, None, Some("bypassPermissions")).unwrap_err();
        assert_eq!(p.code, ProblemCode::ValidationFailed);
        let only_default = caps(&[PermissionMode::Default], &[]);
        let p = validate_start(Some(&only_default), None, Some("plan")).unwrap_err();
        assert_eq!(p.code, ProblemCode::CapabilityUnsupported);
        assert!(validate_start(Some(&only_default), None, Some("default")).is_ok());
    }

    #[test]
    fn har_017_effort_is_checked_against_the_catalog() {
        let none = caps(&[], &[]);
        let p = validate_start(Some(&none), Some("high"), None).unwrap_err();
        assert_eq!(p.code, ProblemCode::CapabilityUnsupported);
        let p = validate_start(None, Some("max"), None).unwrap_err();
        assert_eq!(p.code, ProblemCode::ValidationFailed);
        // Nicht unterstützte Stufe: der Runner mappt sie (HAR-017 AC3).
        assert!(validate_start(Some(&caps(&[], &["low"])), Some("xhigh"), None).is_ok());
    }

    #[test]
    fn har_020_settings_after_restart_are_the_latest() {
        let created = ev(EventPayload::SessionCreated(SessionCreated {
            owner: PrincipalId::User(UserId::LOCAL),
            kind: SessionKind::Main,
            harness: "fake".into(),
            model: Some("a".into()),
            effort: Some("low".into()),
            permission_mode: Some("plan".into()),
            cwd: "/".into(),
            trigger: SessionTrigger::Api,
            ..SessionCreated::default()
        }));
        let changed = ev(EventPayload::SessionSettingsChanged(
            SessionSettingsChanged {
                model: Some("b".into()),
                permission_mode: Some("accept_edits".into()),
                ..SessionSettingsChanged::default()
            },
        ));
        assert_eq!(
            current([&created, &changed]),
            Settings {
                model: Some("b".into()),
                effort: Some("low".into()),
                permission_mode: Some("accept_edits".into()),
            }
        );
    }

    #[test]
    fn har_020_ac3_open_turn_and_approvals_are_closed() {
        let turn = TurnId::new();
        let open = ApprovalId::new();
        let done = ApprovalId::new();
        let req = |id| {
            ev(EventPayload::ApprovalRequested(ApprovalRequested {
                approval_id: id,
                kind: ApprovalKind::Tool,
                subject: json!({}),
                options: vec![],
                expires_at: Timestamp::now(),
                on_timeout: TimeoutAction::Deny,
            }))
        };
        let events = vec![
            ev(EventPayload::TurnStarted(TurnStarted {
                turn_id: turn,
                input_id: None,
                author: PrincipalId::User(UserId::LOCAL),
            })),
            req(done),
            ev(EventPayload::ApprovalResolved(ApprovalResolved {
                approval_id: done,
                decision: ApprovalDecision::Allow,
                answer: None,
                actor: Actor::default(),
                via: ResolvedVia::User,
                remember: None,
                comment: None,
                on_timeout_applied: None,
            })),
            req(open),
        ];
        let found = interrupted(&events);
        assert_eq!(found.turn, Some(turn));
        assert_eq!(found.approvals, vec![open]);
        let closing = closing_events(SessionId::new(), &found);
        assert_eq!(closing.len(), 2);
        match closing[0].payload() {
            Some(EventPayload::ApprovalResolved(a)) => {
                assert_eq!(a.approval_id, open);
                assert_eq!(a.decision, ApprovalDecision::Abort);
                assert_eq!(a.via, ResolvedVia::System);
            }
            other => panic!("{other:?}"),
        }
        match closing[1].payload() {
            Some(EventPayload::TurnFailed(f)) => {
                assert_eq!(f.turn_id, turn);
                assert_eq!(f.problem["code"], "runner_restarted");
            }
            other => panic!("{other:?}"),
        }
        // Abgeschlossener Turn: nichts offen.
        let mut finished = events.clone();
        finished.push(ev(EventPayload::TurnCompleted(TurnCompleted {
            turn_id: turn,
            stop_reason: "end_turn".into(),
            usage_summary: json!({}),
        })));
        assert_eq!(interrupted(&finished).turn, None);
    }
}
