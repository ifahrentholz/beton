//! POL-024 AC2: Die Enforcement-Matrix entsteht aus den Capabilities (HAR-002), nicht aus
//! einer festen Tabelle je Harness.

#![allow(clippy::unwrap_used)]

use beton_harness::fake::default_capabilities;
use beton_harness::{Capabilities, Mode, ModelRequestGate, SwitchSupport, ToolCallGate, Transport};
use beton_policy::Phase;
use beton_policy::enforcement::{KINDS, Level};

fn caps(gate: ToolCallGate, approval_kinds: &[&str], mcp: bool) -> Capabilities {
    let mut c = Capabilities::minimal(Mode::Native, Transport::Native);
    c.tool_call_gate = gate;
    c.approval_kinds = approval_kinds.iter().map(|k| (*k).to_owned()).collect();
    c.mcp_injection = mcp;
    c.model_request_gate = ModelRequestGate::Turn;
    c
}

#[test]
fn pol_024_ac2_full_gate_enforces_every_class() {
    let m = caps(ToolCallGate::Full, &[], true).enforcement();
    for kind in KINDS {
        assert_eq!(m.level(Phase::ToolCall, Some(kind)), Level::Full, "{kind}");
    }
    assert_eq!(m.level(Phase::SessionStart, None), Level::Full);
    assert_eq!(m.level(Phase::ModelRequest, None), Level::Full);
}

#[test]
fn pol_024_ac2_approval_only_covers_only_the_asked_classes() {
    let m = caps(ToolCallGate::ApprovalOnly, &["shell", "file_edit"], true).enforcement();
    assert_eq!(m.level(Phase::ToolCall, Some("shell")), Level::ApprovalOnly);
    assert_eq!(
        m.level(Phase::ToolCall, Some("file_edit")),
        Level::ApprovalOnly
    );
    assert_eq!(
        m.level(Phase::ToolCall, Some("file_read")),
        Level::ObserveOnly
    );
    assert_eq!(m.level(Phase::ToolCall, Some("mcp")), Level::ObserveOnly);
    // System- und Browser-Tools laufen über den beton-MCP-Server: voll (Hub).
    assert_eq!(m.level(Phase::ToolCall, Some("system")), Level::Full);
    assert_eq!(m.level(Phase::ToolCall, Some("browser")), Level::Full);
    // Ohne MCP-Injektion gibt es diesen Weg nicht.
    let m = caps(ToolCallGate::ApprovalOnly, &["shell"], false).enforcement();
    assert_eq!(m.level(Phase::ToolCall, Some("system")), Level::ObserveOnly);
    // Freigaben können keine Argumente ändern.
    assert!(!m.capabilities(Phase::ToolCall).args);
}

#[test]
fn pol_024_ac2_model_request_and_result_levels_come_from_capabilities() {
    let mut c = caps(ToolCallGate::Full, &[], true);
    c.model_request_gate = ModelRequestGate::ObserveOnly;
    c.tool_result_gate = ToolCallGate::ObserveOnly;
    let m = c.enforcement();
    assert_eq!(m.level(Phase::ModelRequest, None), Level::ObserveOnly);
    assert_eq!(
        m.level(Phase::ToolResult, Some("shell")),
        Level::ObserveOnly
    );
    assert!(
        m.capabilities(Phase::ToolCall).args,
        "voller Gate: args änderbar"
    );

    // Modellwechsel pro Turn nur mit Live-Wechsel.
    c.model_request_gate = ModelRequestGate::Turn;
    c.model_switch = SwitchSupport::None;
    assert!(!c.enforcement().capabilities(Phase::ModelRequest).model);
    c.model_switch = SwitchSupport::Live;
    c.effort_switch = SwitchSupport::Live;
    let m = c.enforcement();
    assert!(m.capabilities(Phase::ModelRequest).model);
    assert!(m.capabilities(Phase::ModelRequest).reasoning_effort);
    // Start: Modell, Effort und Permission-Mode setzt der Runner vor dem Start.
    assert!(m.capabilities(Phase::SessionStart).permission_mode);
}

#[test]
fn pol_024_ac2_fake_harness_is_fully_enforceable_and_overridable() {
    let m = default_capabilities().enforcement();
    for kind in KINDS {
        assert_eq!(m.level(Phase::ToolCall, Some(kind)), Level::Full);
        assert_eq!(m.level(Phase::ToolResult, Some(kind)), Level::Full);
    }
    let mut c = default_capabilities();
    c.tool_call_gate = ToolCallGate::ApprovalOnly;
    c.approval_kinds = vec!["shell".into()];
    assert_eq!(
        c.enforcement().level(Phase::ToolCall, Some("file_edit")),
        Level::ObserveOnly
    );
}

/// #283 (POL-013, HAR-017): ACP kennt keinen Start-Parameter für das Modell; ohne
/// `session/set_model` (`model_switch: live`) ist eine Modell-Modifikation beim Start nicht
/// anwendbar (`on_unsupported` greift, Default `deny`), statt still mit dem Default des
/// Agents zu laufen. Ebenso der Effort ohne Stufen bzw. ohne Live-Wechsel bei ACP.
#[test]
fn pol_013_acp_start_model_is_modifiable_only_with_set_model() {
    let mut acp = Capabilities::minimal(Mode::Native, Transport::Acp);
    acp.model_switch = SwitchSupport::None;
    acp.efforts = vec!["low".into(), "high".into()];
    let start = acp.enforcement().capabilities(Phase::SessionStart);
    assert!(!start.model, "ACP ohne set_model");
    assert!(!start.reasoning_effort, "ACP ohne Effort-Wechsel");
    assert!(start.permission_mode, "Modus über session/set_mode");
    acp.model_switch = SwitchSupport::Live;
    acp.effort_switch = SwitchSupport::Live;
    let start = acp.enforcement().capabilities(Phase::SessionStart);
    assert!(start.model && start.reasoning_effort, "mit Live-Wechsel");

    // Native Harnesses setzen das Modell per Startparameter; Effort nur mit Stufen.
    let mut native = Capabilities::minimal(Mode::Native, Transport::Native);
    native.model_switch = SwitchSupport::None;
    let start = native.enforcement().capabilities(Phase::SessionStart);
    assert!(start.model, "Startparameter --model");
    assert!(
        !start.reasoning_effort,
        "ohne Effort-Stufen nicht abbildbar"
    );
    native.efforts = vec!["low".into()];
    assert!(
        native
            .enforcement()
            .capabilities(Phase::SessionStart)
            .reasoning_effort
    );
}
