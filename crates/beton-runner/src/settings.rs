//! Modell-, Effort- und Permission-Mode-Wechsel im Runner (HAR-017, HAR-027).
//!
//! Der Runner prüft jede Änderung gegen die Capabilities der Session, mappt den Effort auf
//! eine Stufe des Harness, verschiebt Wechsel während eines Turns auf dessen Ende und schreibt
//! `session.settings_changed`. Den Mechanismus (live oder Neustart mit Resume) bestimmen die
//! Capabilities. Fail closed: Was der Harness nicht abbilden kann, wird abgelehnt statt still
//! ignoriert; `yolo` braucht Sandbox und Egress-Proxy.

use beton_core::event::{SessionSettingsChanged, SettingsMechanism};
use beton_core::id::TurnId;
use beton_harness::{
    Action, Capabilities, CapabilityUnsupported, EFFORT_LEVELS, HarnessError, PermissionMode,
    ResumeSupport, SandboxStatus, SwitchSupport, check_permission_mode, map_effort,
};
use serde_json::{Value, json};

/// Eine geprüfte Änderung der Session-Einstellungen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    pub model: Option<String>,
    /// Effektiver Effort (gemappt).
    pub effort: Option<String>,
    /// Angefragter Effort, wenn er gemappt wurde (HAR-017 AC3).
    pub requested_effort: Option<String>,
    pub permission_mode: Option<PermissionMode>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.model.is_none() && self.effort.is_none() && self.permission_mode.is_none()
    }

    /// Eine spätere Änderung überschreibt die Felder, die sie setzt.
    pub fn merge(&mut self, later: Change) {
        if later.model.is_some() {
            self.model = later.model;
        }
        if later.effort.is_some() {
            self.effort = later.effort;
            self.requested_effort = later.requested_effort;
        }
        if later.permission_mode.is_some() {
            self.permission_mode = later.permission_mode;
        }
    }

    /// Übernimmt die Änderung in die Startparameter (für einen späteren Neustart).
    pub fn apply_to(&self, spec: &mut beton_harness::SessionSpec) {
        if self.model.is_some() {
            spec.model.clone_from(&self.model);
        }
        if self.effort.is_some() {
            spec.effort.clone_from(&self.effort);
        }
        if self.permission_mode.is_some() {
            spec.permission_mode = self.permission_mode;
        }
    }

    /// Live oder per Neustart mit Resume (Capabilities `model_switch`, `effort_switch`).
    pub fn mechanism(&self, caps: &Capabilities) -> SettingsMechanism {
        let restart = (self.model.is_some() && caps.model_switch == SwitchSupport::Restart)
            || (self.effort.is_some() && caps.effort_switch == SwitchSupport::Restart);
        if restart {
            SettingsMechanism::Restart
        } else {
            SettingsMechanism::Live
        }
    }

    /// `session.settings_changed` für diese Änderung.
    pub fn event(
        &self,
        mechanism: Option<SettingsMechanism>,
        effective_from_turn: Option<TurnId>,
    ) -> SessionSettingsChanged {
        SessionSettingsChanged {
            model: self.model.clone(),
            effort: self.effort.clone(),
            requested_effort: self.requested_effort.clone(),
            permission_mode: self.permission_mode.map(|m| m.as_str().to_owned()),
            mechanism,
            effective_from_turn,
        }
    }
}

/// Eine abgelehnte Änderung als Problem für `cmd.result`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub code: &'static str,
    pub detail: String,
}

impl Rejected {
    fn invalid(detail: impl Into<String>) -> Self {
        Self {
            code: "validation_failed",
            detail: detail.into(),
        }
    }

    pub fn to_problem(&self) -> Value {
        json!({"code": self.code, "detail": self.detail})
    }
}

impl From<HarnessError> for Rejected {
    fn from(e: HarnessError) -> Self {
        Self {
            code: e.code(),
            detail: e.to_string(),
        }
    }
}

fn unsupported(action: Action, detail: &str) -> Rejected {
    let e: HarnessError = CapabilityUnsupported(action).into();
    Rejected {
        code: e.code(),
        detail: detail.to_owned(),
    }
}

/// Effort prüfen und mappen: `(effektiv, angefragt falls abweichend)`.
pub fn effort(requested: &str, caps: &Capabilities) -> Result<(String, Option<String>), Rejected> {
    if !EFFORT_LEVELS.contains(&requested) {
        return Err(Rejected::invalid(format!(
            "Effort `{requested}` gibt es nicht (low, medium, high, xhigh)"
        )));
    }
    if caps.effort_switch == SwitchSupport::None || caps.efforts.is_empty() {
        return Err(unsupported(
            Action::EffortSwitch,
            "Dieser Harness kennt keine Effort-Stufen",
        ));
    }
    let mapped = map_effort(requested, &caps.efforts).ok_or_else(|| {
        unsupported(
            Action::EffortSwitch,
            "Dieser Harness kennt keine passende Effort-Stufe",
        )
    })?;
    let requested = (mapped != requested).then(|| requested.to_owned());
    Ok((mapped, requested))
}

/// Prüft `session.set {model?, effort?, permission_mode?}` für eine laufende Session.
pub fn parse(
    args: &Value,
    caps: &Capabilities,
    sandbox: SandboxStatus,
) -> Result<Change, Rejected> {
    let text = |key: &str| -> Result<Option<String>, Rejected> {
        match &args[key] {
            Value::Null => Ok(None),
            Value::String(s) if !s.trim().is_empty() => Ok(Some(s.trim().to_owned())),
            _ => Err(Rejected::invalid(format!(
                "`{key}` muss ein nicht leerer Text sein"
            ))),
        }
    };
    let mut change = Change::default();
    if let Some(mode) = text("permission_mode")? {
        let mode: PermissionMode = mode
            .parse()
            .map_err(|e| Rejected::invalid(format!("{e}")))?;
        check_permission_mode(mode, caps, sandbox)?;
        change.permission_mode = Some(mode);
    }
    if let Some(model) = text("model")? {
        match caps.model_switch {
            SwitchSupport::None => {
                return Err(unsupported(
                    Action::ModelSwitch,
                    "Dieser Harness kann das Modell nicht während der Session wechseln",
                ));
            }
            // Ein Neustart ohne Warm-Resume verlöre den Verlauf.
            SwitchSupport::Restart if caps.resume != ResumeSupport::Warm => {
                return Err(unsupported(
                    Action::ModelSwitch,
                    "Modellwechsel per Neustart braucht Warm-Resume",
                ));
            }
            _ => {}
        }
        change.model = Some(model);
    }
    if let Some(requested) = text("effort")? {
        if caps.effort_switch == SwitchSupport::Restart && caps.resume != ResumeSupport::Warm {
            return Err(unsupported(
                Action::EffortSwitch,
                "Effort-Wechsel per Neustart braucht Warm-Resume",
            ));
        }
        let (effective, asked) = effort(&requested, caps)?;
        change.effort = Some(effective);
        change.requested_effort = asked;
    }
    Ok(change)
}

/// Bindet der Agent die Session an den Plan-Modus (AGT-011, HAR-027)? Ja, wenn sein
/// `executor.permission_mode` `plan` ist und die Session in `plan` startet, also kein Override
/// (CLI-Flag, Anfrage, Fork) einen anderen Modus gesetzt hat. `plan` ohne Agent wählt der
/// Nutzer selbst; dann darf er `ExitPlanMode` freigeben.
pub fn plan_bound(agent: Option<PermissionMode>, start: Option<PermissionMode>) -> bool {
    agent == Some(PermissionMode::Plan) && start == Some(PermissionMode::Plan)
}

/// Einstellungen beim Start einer Session (aus `session.created` bzw. den letzten
/// `session.settings_changed`). Der Effort wird gegen die Stufen des Harness gemappt; ob der
/// Harness den Permission-Mode abbilden kann, prüft der Adapter beim Start (manche kennen ihre
/// Modi erst danach, z. B. ACP), `yolo` scheitert hier schon ohne Sandbox (fail closed).
/// Liefert die Startwerte und – falls der Effort gemappt wurde – das Event dazu.
pub fn at_start(
    model: Option<String>,
    effort: Option<&str>,
    permission_mode: Option<&str>,
    caps: &Capabilities,
    sandbox: SandboxStatus,
) -> Result<(Change, Option<SessionSettingsChanged>), Rejected> {
    let mut change = Change {
        model,
        ..Change::default()
    };
    if let Some(mode) = permission_mode {
        let mode: PermissionMode = mode
            .parse()
            .map_err(|e| Rejected::invalid(format!("{e}")))?;
        if mode == PermissionMode::Yolo && !sandbox.allows_yolo() {
            return Err(HarnessError::SandboxRequired(
                "YOLO startet nur mit Tool-Sandbox (Stufe 2) und Egress-Proxy".into(),
            )
            .into());
        }
        change.permission_mode = Some(mode);
    }
    let mut mapped = None;
    if let Some(requested) = effort {
        if !EFFORT_LEVELS.contains(&requested) {
            return Err(Rejected::invalid(format!(
                "Effort `{requested}` gibt es nicht (low, medium, high, xhigh)"
            )));
        }
        if caps.efforts.is_empty() {
            return Err(unsupported(
                Action::EffortSwitch,
                "Dieser Harness kennt keine Effort-Stufen",
            ));
        }
        let effective = map_effort(requested, &caps.efforts).ok_or_else(|| {
            unsupported(
                Action::EffortSwitch,
                "Dieser Harness kennt keine passende Effort-Stufe",
            )
        })?;
        if effective != requested {
            mapped = Some(SessionSettingsChanged {
                effort: Some(effective.clone()),
                requested_effort: Some(requested.to_owned()),
                ..SessionSettingsChanged::default()
            });
        }
        change.effort = Some(effective);
    }
    Ok((change, mapped))
}

/// Start-Fehler als Harness-Fehler (für `report_start_failure`).
pub fn start_error(r: &Rejected) -> HarnessError {
    match r.code {
        "sandbox_required" => HarnessError::SandboxRequired(r.detail.clone()),
        "capability_unsupported" => CapabilityUnsupported(Action::EffortSwitch).into(),
        _ => HarnessError::StartRefused(r.detail.clone()),
    }
}

#[cfg(test)]
mod tests {
    use beton_harness::{Mode, Transport};

    use super::*;

    fn caps() -> Capabilities {
        let mut c = Capabilities::minimal(Mode::Native, Transport::Native);
        c.model_switch = SwitchSupport::Live;
        c.effort_switch = SwitchSupport::Live;
        c.resume = ResumeSupport::Warm;
        c.efforts = vec!["low".into(), "medium".into(), "high".into()];
        c.permission_modes = PermissionMode::ALL.to_vec();
        c
    }

    fn sandbox() -> SandboxStatus {
        SandboxStatus::current()
    }

    #[test]
    fn har_017_ac3_unsupported_effort_is_mapped_and_reported() {
        let change = parse(&json!({"effort": "xhigh"}), &caps(), sandbox()).unwrap();
        assert_eq!(change.effort.as_deref(), Some("high"));
        assert_eq!(change.requested_effort.as_deref(), Some("xhigh"));
        let e = change.event(Some(SettingsMechanism::Live), None);
        assert_eq!(e.effort.as_deref(), Some("high"));
        assert_eq!(e.requested_effort.as_deref(), Some("xhigh"));
        // Unterstützt: kein `requested_effort`.
        let exact = parse(&json!({"effort": "medium"}), &caps(), sandbox()).unwrap();
        assert_eq!(exact.requested_effort, None);
    }

    #[test]
    fn har_017_effort_without_levels_or_unknown_is_rejected() {
        let mut c = caps();
        c.efforts.clear();
        let e = parse(&json!({"effort": "high"}), &c, sandbox()).unwrap_err();
        assert_eq!(e.code, "capability_unsupported");
        let e = parse(&json!({"effort": "max"}), &caps(), sandbox()).unwrap_err();
        assert_eq!(e.code, "validation_failed");
    }

    #[test]
    fn har_002_ac3_model_switch_none_is_capability_unsupported() {
        let mut c = caps();
        c.model_switch = SwitchSupport::None;
        let e = parse(&json!({"model": "opus"}), &c, sandbox()).unwrap_err();
        assert_eq!(e.code, "capability_unsupported");
    }

    #[test]
    fn har_017_ac2_restart_needs_warm_resume_and_sets_mechanism() {
        let mut c = caps();
        c.model_switch = SwitchSupport::Restart;
        let change = parse(&json!({"model": "big"}), &c, sandbox()).unwrap();
        assert_eq!(change.mechanism(&c), SettingsMechanism::Restart);
        c.resume = ResumeSupport::Cold;
        let e = parse(&json!({"model": "big"}), &c, sandbox()).unwrap_err();
        assert_eq!(e.code, "capability_unsupported");
        // Permission-Modes wechseln immer live.
        let mode = parse(&json!({"permission_mode": "plan"}), &caps(), sandbox()).unwrap();
        assert_eq!(mode.mechanism(&c), SettingsMechanism::Live);
    }

    #[test]
    fn har_027_ac1_switch_to_yolo_is_sandbox_required() {
        let e = parse(&json!({"permission_mode": "yolo"}), &caps(), sandbox()).unwrap_err();
        assert_eq!(e.code, "sandbox_required");
        // Auch zusammen mit anderen Feldern: nichts wird übernommen.
        let e = parse(
            &json!({"model": "opus", "permission_mode": "yolo"}),
            &caps(),
            sandbox(),
        )
        .unwrap_err();
        assert_eq!(e.code, "sandbox_required");
    }

    #[test]
    fn har_027_unknown_or_vendor_mode_names_are_rejected() {
        for bad in ["bypassPermissions", "acceptEdits", "auto"] {
            let e = parse(&json!({"permission_mode": bad}), &caps(), sandbox()).unwrap_err();
            assert_eq!(e.code, "validation_failed", "{bad}");
        }
        let mut c = caps();
        c.permission_modes = vec![PermissionMode::Default];
        let e = parse(&json!({"permission_mode": "plan"}), &c, sandbox()).unwrap_err();
        assert_eq!(e.code, "capability_unsupported");
    }

    #[test]
    fn har_027_ac3_mode_change_event_carries_mode_and_mechanism() {
        let change = parse(
            &json!({"permission_mode": "accept_edits"}),
            &caps(),
            sandbox(),
        )
        .unwrap();
        let e = change.event(Some(change.mechanism(&caps())), None);
        assert_eq!(e.permission_mode.as_deref(), Some("accept_edits"));
        assert_eq!(e.mechanism, Some(SettingsMechanism::Live));
    }

    #[test]
    fn har_017_ac4_later_change_overrides_pending_fields() {
        let mut pending =
            parse(&json!({"model": "a", "effort": "low"}), &caps(), sandbox()).unwrap();
        pending.merge(parse(&json!({"model": "b"}), &caps(), sandbox()).unwrap());
        assert_eq!(pending.model.as_deref(), Some("b"));
        assert_eq!(pending.effort.as_deref(), Some("low"));
    }

    #[test]
    fn har_027_ac1_start_with_yolo_is_sandbox_required() {
        let e = at_start(None, None, Some("yolo"), &caps(), sandbox()).unwrap_err();
        assert_eq!(e.code, "sandbox_required");
        let (change, mapped) = at_start(
            Some("m".into()),
            Some("xhigh"),
            Some("plan"),
            &caps(),
            sandbox(),
        )
        .unwrap();
        assert_eq!(change.permission_mode, Some(PermissionMode::Plan));
        assert_eq!(change.effort.as_deref(), Some("high"));
        assert_eq!(mapped.unwrap().requested_effort.as_deref(), Some("xhigh"));
    }

    #[test]
    fn agt_011_plan_from_the_agent_is_binding_unless_overridden() {
        let plan = Some(PermissionMode::Plan);
        // `executor.permission_mode: plan` des Agents, Session startet in `plan`.
        assert!(plan_bound(plan, plan));
        // Ein Override (CLI-Flag, Anfrage, Fork) auf einen anderen Modus löst die Bindung.
        assert!(!plan_bound(plan, Some(PermissionMode::Default)));
        // `plan` ohne Agent bzw. ohne Agent-Modus: Der Nutzer darf `ExitPlanMode` freigeben.
        assert!(!plan_bound(None, plan));
        assert!(!plan_bound(Some(PermissionMode::AcceptEdits), plan));
    }
}
