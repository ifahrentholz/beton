//! `tool_allowlist` (POL-015): erlaubt nur gelistete Tools.
//!
//! Entzuckerung: `defaults.tool_call: deny` der Ebene plus eine `allow`-Regel, die greift,
//! wenn das Tool auf der Liste steht. Weil ein Default nur durch `allow` derselben Ebene
//! aufgehoben wird, wirken Allowlists verschiedener Ebenen als Schnittmenge (POL-007).
//!
//! - `allow`: Globs gegen `tool.name` eingebauter Tools (`tool.server == "builtin"`, nie
//!   Klasse `mcp`, nicht bei ACP, wo `tool.name` der Titel des Agents ist) bzw. gegen
//!   `tool.kind`; `mcp:<server>/<tool>` gegen MCP-Tools. Ein Name ohne `mcp:` erlaubt
//!   nie ein gleichnamiges MCP-Tool (ein fremder Server könnte sonst `Read` anbieten).
//! - `allow_kinds`: Tool-Klassen (POL-005), `allow_servers`: MCP-Server.
//! - `deny_reason`: Begründung der Ablehnung (Default der Ebene).

use super::{ParamError, Params, Sugar, SugarRule, check_glob, check_kinds, list, lit};
use crate::Phase;
use crate::doc::{Action, DefaultAction, RuleDoc};

const PARAMS: [&str; 4] = ["allow", "allow_kinds", "allow_servers", "deny_reason"];

pub(super) fn desugar(rule: &RuleDoc, params: &Params, shadow: bool) -> Result<Sugar, ParamError> {
    params.only(&PARAMS)?;
    let id = &rule.id;
    let allow = params.strings("allow")?.unwrap_or_default();
    let kinds = params.strings("allow_kinds")?.unwrap_or_default();
    let servers = params.strings("allow_servers")?.unwrap_or_default();
    let deny_reason = params
        .text("deny_reason")?
        .or_else(|| rule.reason.clone())
        .unwrap_or_else(|| {
            format!("Das Tool steht nicht auf der Allowlist `{id}` (tool_allowlist)")
        });
    check_kinds(&kinds, "allow_kinds")?;

    let mut parts: Vec<String> = Vec::new();
    for (i, pattern) in allow.iter().enumerate() {
        check_glob(pattern, &format!("allow[{i}]"))?;
        match pattern.strip_prefix("mcp:") {
            Some(_) => parts.push(format!(
                "(tool.server != \"builtin\" && glob(\"mcp:\" + tool.server + \"/\" + tool.name, {}))",
                lit(pattern)
            )),
            None => parts.push(format!(
                // Bei ACP ist `tool.name` der Titel des Agents (Review #285 M2).
                "(tool.server == \"builtin\" && tool.kind != \"mcp\" && !session.harness.startsWith(\"acp\") && glob(tool.name, {p})) || glob(tool.kind, {p})",
                p = lit(pattern)
            )),
        }
    }
    if !kinds.is_empty() {
        parts.push(format!("tool.kind in {}", list(&kinds)));
    }
    if !servers.is_empty() {
        parts.push(format!("tool.server in {}", list(&servers)));
    }
    let allowed = if parts.is_empty() {
        "false".to_owned()
    } else {
        parts.join(" || ")
    };

    if shadow {
        // Im Schattenbetrieb kein scharfer Default: die gleichwertige Ablehnung als Regel
        // meldet `would_deny` (POL-027).
        return Ok(Sugar {
            rules: vec![SugarRule {
                part: None,
                phase: Phase::ToolCall,
                when: format!("!({allowed})"),
                action: Action::Deny,
                reason: deny_reason,
                kinds: None,
                notify: None,
                enforcement: None,
            }],
            defaults: Vec::new(),
        });
    }
    Ok(Sugar {
        rules: vec![SugarRule {
            part: None,
            phase: Phase::ToolCall,
            when: allowed,
            action: Action::Allow,
            reason: format!("Auf der Allowlist `{id}` (tool_allowlist)"),
            kinds: None,
            notify: None,
            enforcement: None,
        }],
        defaults: vec![(Phase::ToolCall, DefaultAction::Deny, deny_reason)],
    })
}
