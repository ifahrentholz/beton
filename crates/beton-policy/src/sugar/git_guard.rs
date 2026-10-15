//! `git_guard` (POL-017): riskante Git-Operationen über Shell und MCP-Tools.
//!
//! Entzuckerung zu Regeln in Phase `tool_call`, je Operation mit ihrer Aktion (`allow` erzeugt
//! keine Regel; sie hebt also auch keinen Default der Ebene auf):
//!
//! | Regel | greift bei | Default |
//! |---|---|---|
//! | `<id>/force-push` | `-f`, `--force`, `--force-with-lease`, `+<refspec>`, `--mirror` | `deny` |
//! | `<id>/push` | jedem `git push` | `ask` |
//! | `<id>/protected` | Push auf bzw. Merge in einen geschützten Branch | `deny` |
//! | `<id>/merge` | `git merge` | `ask` |
//! | `<id>/delete-branch` | `git branch -d/-D`, `push --delete`, `push :<branch>` | `ask` |
//! | `<id>/history-rewrite` | `reset --hard`, `rebase`, `filter-branch`, `commit --amend` | `ask` |
//! | `<id>/uncertain` | nicht sicher parsebares Shell-Kommando mit `git` darin | `ask` |
//! | `<id>/mcp-<op>` | MCP-Tools aus `mcp_tools.<op>` (`server/tool`, Glob) | Aktion von `<op>` |
//!
//! Das Push-Ziel kommt aus der Refspec bzw. ohne Refspec aus `session.branch`; ist der Branch
//! unbekannt, scheitert die Regel und lehnt ab (fail closed). `commit --amend` zählt immer,
//! weil beton nicht erkennt, ob der Commit schon gepusht ist.

use serde_json::Value;

use super::{ParamError, Params, Sugar, SugarRule, check_glob, lit, strings_at};
use crate::Phase;
use crate::doc::Action;

const PARAMS: [&str; 8] = [
    "push",
    "force_push",
    "protected_branches",
    "protected_action",
    "merge",
    "delete_branch",
    "history_rewrite",
    "mcp_tools",
];

/// Operationen mit Regel-Teil, Default und Begründung.
const OPS: [(&str, &str, Action, &str); 5] = [
    ("force_push", "force-push", Action::Deny, "Force-Push"),
    ("push", "push", Action::Ask, "Push"),
    ("merge", "merge", Action::Ask, "Merge"),
    (
        "delete_branch",
        "delete-branch",
        Action::Ask,
        "Branch löschen",
    ),
    (
        "history_rewrite",
        "history-rewrite",
        Action::Ask,
        "Verlauf umschreiben (reset --hard, rebase, filter-branch, commit --amend)",
    ),
];

/// Regel-Teil der MCP-Regel je Operation.
fn mcp_part(op: &str) -> &'static str {
    match op {
        "force_push" => "mcp-force-push",
        "push" => "mcp-push",
        "merge" => "mcp-merge",
        "delete_branch" => "mcp-delete-branch",
        _ => "mcp-history-rewrite",
    }
}

/// Aktion eines Parameters (`allow`, `ask`, `deny`); `None` = `allow`.
fn action(params: &Params, name: &str, default: Action) -> Result<Option<Action>, ParamError> {
    match params.text(name)?.as_deref() {
        None => Ok(Some(default)),
        Some("allow") => Ok(None),
        Some("ask") => Ok(Some(Action::Ask)),
        Some("deny") => Ok(Some(Action::Deny)),
        Some(other) => Err(ParamError::new(
            name,
            format!("`{name}: {other}` ist ungültig (allow, ask, deny)"),
        )),
    }
}

/// `"x" in shell.git_ops(tool.argv)` für Shell-Calls.
fn shell_op(op: &str) -> String {
    format!(
        "tool.kind == \"shell\" && {} in shell.git_ops(tool.argv)",
        lit(op)
    )
}

pub(super) fn desugar(params: &Params) -> Result<Sugar, ParamError> {
    params.only(&PARAMS)?;
    let shell = || Some(vec!["shell".to_owned()]);
    let mut rules = Vec::new();
    let mut actions = Vec::new();
    for (op, part, default, what) in OPS {
        let act = action(params, op, default)?;
        actions.push((op, act));
        if let Some(a) = act {
            rules.push(SugarRule {
                part: Some(part),
                phase: Phase::ToolCall,
                when: shell_op(op),
                action: a,
                reason: format!("{what} (git_guard)"),
                kinds: shell(),
                notify: None,
            });
        }
    }

    // Geschützte Branches: Push-Ziel bzw. aktueller Branch beim Merge.
    let branches = strings_at(params.get("protected_branches"), "protected_branches")?
        .unwrap_or_else(|| vec!["main".into(), "master".into()]);
    for (i, b) in branches.iter().enumerate() {
        check_glob(b, &format!("protected_branches[{i}]"))?;
    }
    if let Some(a) = action(params, "protected_action", Action::Deny)?
        && !branches.is_empty()
    {
        let protected = |x: &str| {
            let parts: Vec<String> = branches
                .iter()
                .map(|b| format!("glob({x}, {})", lit(b)))
                .collect();
            format!("({})", parts.join(" || "))
        };
        rules.push(SugarRule {
                part: Some("protected"),
                phase: Phase::ToolCall,
                when: format!(
                    "tool.kind == \"shell\" && (shell.git_push_targets(tool.argv).exists(b, b == \"*\" || {} || (b == \"HEAD\" && {})) || (\"merge\" in shell.git_ops(tool.argv) && {}))",
                    protected("b"),
                    protected("session.branch"),
                    protected("session.branch"),
                ),
                action: a,
                reason: "Push bzw. Merge auf einen geschützten Branch (git_guard)".into(),
                kinds: shell(),
                notify: None,
        });
    }

    // Nicht sicher parsebar und `git` darin: nachfragen (POL-005).
    rules.push(SugarRule {
        part: Some("uncertain"),
        phase: Phase::ToolCall,
        when: "tool.kind == \"shell\" && tool.parse_uncertain && tool.command.contains(\"git\")"
            .into(),
        action: Action::Ask,
        reason: "Git-Kommando nicht sicher erkennbar (git_guard)".into(),
        kinds: shell(),
        notify: None,
    });

    // MCP-Tools je Operation.
    let mcp = mcp_tools(params)?;
    for (op, patterns) in mcp {
        let Some((_, Some(a))) = actions.iter().find(|(o, _)| *o == op) else {
            continue;
        };
        if patterns.is_empty() {
            continue;
        }
        let what = OPS
            .iter()
            .find(|(o, ..)| *o == op)
            .map_or("Git-Operation", |(.., w)| *w);
        let parts: Vec<String> = patterns
            .iter()
            .map(|p| format!("glob(tool.server + \"/\" + tool.name, {})", lit(p)))
            .collect();
        rules.push(SugarRule {
            part: Some(mcp_part(op)),
            phase: Phase::ToolCall,
            when: format!("tool.server != \"builtin\" && ({})", parts.join(" || ")),
            action: *a,
            reason: format!("{what} über ein MCP-Tool (git_guard)"),
            kinds: Some(vec!["mcp".into()]),
            notify: None,
        });
    }
    Ok(Sugar {
        rules,
        defaults: Vec::new(),
    })
}

/// `mcp_tools: { <op>: ["server/tool", …] }`; ohne Angabe `merge: [github/merge_pull_request]`.
fn mcp_tools(params: &Params) -> Result<Vec<(&'static str, Vec<String>)>, ParamError> {
    let Some(v) = params.get("mcp_tools") else {
        return Ok(vec![("merge", vec!["github/merge_pull_request".into()])]);
    };
    let Value::Object(map) = v else {
        return Err(ParamError::new(
            "mcp_tools",
            "Objekt `{ <operation>: [server/tool, …] }` erwartet",
        ));
    };
    let mut out = Vec::new();
    for (key, list) in map {
        let Some((op, ..)) = OPS.iter().find(|(o, ..)| o == key) else {
            return Err(ParamError::new(
                format!("mcp_tools.{key}"),
                format!(
                    "unbekannte Operation `{key}` (erlaubt: {})",
                    OPS.iter().map(|(o, ..)| *o).collect::<Vec<_>>().join(", ")
                ),
            ));
        };
        let field = format!("mcp_tools.{key}");
        let patterns = strings_at(Some(list), &field)?.unwrap_or_default();
        for (i, p) in patterns.iter().enumerate() {
            check_glob(p, &format!("{field}[{i}]"))?;
        }
        out.push((*op, patterns));
    }
    Ok(out)
}
