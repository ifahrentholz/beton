//! Eingebaute Regeltypen aus WP-42 (POL-015 bis POL-018): Entzuckerung zu CEL-Regeln und ihre
//! Wirkung in der Engine. Die mitgelieferten YAML-Policy-Tests je Regeltyp liegen unter
//! `policies/tests/rule-types/` (POL-026 AC4).

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use beton_policy::Phase;
use beton_policy::context::Activation;
use beton_policy::doc::{Action, DefaultAction};
use beton_policy::engine::{Decision, EffectiveSet, EvalOptions, Outcome};
use beton_policy::load::{LoadedSet, RuleKind, Scope, parse_set};
use beton_policy::policytest::activation;
use beton_policy::store::only_tightens;
use serde_json::{Map, Value, json};

fn set(scope: Scope, name: &str, rules: &str) -> Arc<LoadedSet> {
    let text = format!("spec_version: 1\nname: {name}\nrules:\n{rules}");
    Arc::new(
        parse_set(scope, &format!("{name}.yaml"), &text).unwrap_or_else(|e| panic!("{e}\n{text}")),
    )
}

fn load_err(rules: &str) -> String {
    let text = format!("spec_version: 1\nname: t\nrules:\n{rules}");
    match parse_set(Scope::Project, "t.yaml", &text) {
        Ok(_) => panic!("Ladefehler erwartet:\n{text}"),
        Err(e) => e.to_string(),
    }
}

/// Normalisierter Tool-Call wie im Runner (POL-005), Harness `claude`.
fn call(name: &str, args: Value) -> Activation {
    call_with(name, args, json!({}))
}

fn call_with(name: &str, args: Value, fixtures: Value) -> Activation {
    let mut input = Map::new();
    input.insert("tool".into(), json!({ "name": name, "args": args }));
    activation(
        Phase::ToolCall,
        &fixtures,
        &input,
        Some("claude"),
        Some("/home/u"),
    )
    .unwrap()
}

fn eval(sets: Vec<Arc<LoadedSet>>, act: &Activation) -> Decision {
    EffectiveSet::new(sets).evaluate(act, &EvalOptions::default())
}

fn outcome(d: &Decision) -> &'static str {
    match d.outcome {
        Outcome::Allow => "allow",
        Outcome::Deny { .. } => "deny",
        Outcome::Ask { .. } => "ask",
    }
}

fn reasons(d: &Decision) -> Vec<(String, String)> {
    match &d.outcome {
        Outcome::Allow => Vec::new(),
        Outcome::Deny { reasons } | Outcome::Ask { reasons } => reasons
            .iter()
            .map(|r| (r.rule.rule_id.clone(), r.reason.clone()))
            .collect(),
    }
}

// ---------------------------------------------------------------------------------------
// POL-015 tool_allowlist
// ---------------------------------------------------------------------------------------

const READ_ONLY: &str = "  - id: read-only\n    type: tool_allowlist\n    params: { allow_kinds: [system, file_read, search], deny_reason: \"Nur lesende Tools\" }\n";

#[test]
fn pol_015_ac1_allow_kinds_deny_bash_and_edit_but_allow_read_and_session_spawn() {
    let s = set(Scope::Project, "p", READ_ONLY);
    for (name, args) in [
        ("Bash", json!({ "command": "ls" })),
        (
            "Edit",
            json!({ "file_path": "/repo/a", "old_string": "a", "new_string": "b" }),
        ),
    ] {
        let d = eval(vec![s.clone()], &call(name, args));
        assert_eq!(outcome(&d), "deny", "{name}: {d:?}");
        let r = reasons(&d);
        assert_eq!(r[0].0, "defaults.tool_call", "{r:?}");
        assert!(r[0].1.contains("Nur lesende Tools"), "{r:?}");
    }
    for (name, args) in [
        ("Read", json!({ "file_path": "/repo/a" })),
        ("mcp__beton__session_spawn", json!({ "agent": "x" })),
    ] {
        let d = eval(vec![s.clone()], &call(name, args));
        assert_eq!(outcome(&d), "allow", "{name}: {d:?}");
        assert!(
            d.matched.iter().any(|m| m.rule.rule_id == "read-only"),
            "{name}: {:?}",
            d.matched
        );
    }
}

#[test]
fn pol_015_ac2_allowlists_on_project_and_agent_intersect() {
    let project = set(
        Scope::Project,
        "p",
        "  - id: p-list\n    type: tool_allowlist\n    params: { allow_kinds: [file_read, shell] }\n",
    );
    let agent = set(
        Scope::Agent,
        "a",
        "  - id: a-list\n    type: tool_allowlist\n    params: { allow_kinds: [file_read, file_edit] }\n",
    );
    let sets = vec![project, agent];
    let read = eval(
        sets.clone(),
        &call("Read", json!({ "file_path": "/repo/a" })),
    );
    assert_eq!(outcome(&read), "allow", "{read:?}");
    let bash = eval(sets.clone(), &call("Bash", json!({ "command": "ls" })));
    assert_eq!(outcome(&bash), "deny", "{bash:?}");
    assert_eq!(reasons(&bash)[0].0, "defaults.tool_call");
    let edit = eval(
        sets,
        &call(
            "Edit",
            json!({ "file_path": "/repo/a", "old_string": "a", "new_string": "b" }),
        ),
    );
    assert_eq!(outcome(&edit), "deny", "{edit:?}");
}

/// Namensmuster gelten nur für eingebaute Tools; MCP-Tools nennt man `mcp:<server>/<tool>`
/// bzw. über `allow_servers` (sonst erlaubte `allow: [Read]` auch ein MCP-Tool `Read`).
#[test]
fn pol_015_name_patterns_cover_builtins_and_mcp_patterns_cover_servers() {
    let s = set(
        Scope::User,
        "u",
        "  - id: list\n    type: tool_allowlist\n    params: { allow: [\"Read\", \"Gr*\", \"mcp:github/*\"], allow_servers: [beton] }\n",
    );
    for (name, args, want) in [
        ("Read", json!({ "file_path": "/repo/a" }), "allow"),
        ("Grep", json!({ "pattern": "x", "path": "/repo" }), "allow"),
        ("mcp__github__get_issue", json!({ "n": 1 }), "allow"),
        ("mcp__beton__session_spawn", json!({}), "allow"),
        ("mcp__evil__Read", json!({ "file_path": "/repo/a" }), "deny"),
        ("mcp__gitlab__get_issue", json!({}), "deny"),
        (
            "Write",
            json!({ "file_path": "/repo/a", "content": "x" }),
            "deny",
        ),
    ] {
        let d = eval(vec![s.clone()], &call(name, args));
        assert_eq!(outcome(&d), want, "{name}: {d:?}");
    }
}

#[test]
fn pol_015_desugars_to_layer_default_deny_plus_allow_rule() {
    let s = set(Scope::Project, "p", READ_ONLY);
    assert_eq!(s.defaults.get(&Phase::ToolCall), Some(&DefaultAction::Deny));
    assert_eq!(s.rules.len(), 1);
    let rule = &s.rules[0];
    assert_eq!(rule.id, "read-only");
    match &rule.kind {
        RuleKind::Cel {
            phase,
            when: Some(when),
            action,
        } => {
            assert_eq!(*phase, Phase::ToolCall);
            assert_eq!(*action, Action::Allow);
            assert!(when.source().contains("tool.kind in"), "{}", when.source());
        }
        other => panic!("CEL-Regel erwartet: {other:?}"),
    }
}

#[test]
fn pol_015_dry_run_allowlist_only_reports() {
    let s = set(
        Scope::Project,
        "p",
        "  - id: shadow-list\n    type: tool_allowlist\n    dry_run: true\n    params: { allow_kinds: [file_read] }\n",
    );
    assert!(!s.defaults.contains_key(&Phase::ToolCall));
    let d = eval(vec![s.clone()], &call("Bash", json!({ "command": "ls" })));
    assert_eq!(outcome(&d), "allow", "{d:?}");
    assert!(
        d.would.iter().any(|m| m.rule.rule_id == "shadow-list"),
        "{:?}",
        d.would
    );
    let d = eval(vec![s], &call("Read", json!({ "file_path": "/repo/a" })));
    assert!(d.would.is_empty(), "{:?}", d.would);
}

#[test]
fn pol_015_params_are_checked_when_loading() {
    for (rules, needle) in [
        (
            "  - id: x\n    type: tool_allowlist\n    params: { allow_kind: [shell] }\n",
            "allow_kind",
        ),
        (
            "  - id: x\n    type: tool_allowlist\n    params: { allow_kinds: [shel] }\n",
            "shel",
        ),
        (
            "  - id: x\n    type: tool_allowlist\n    params: { allow: Read }\n",
            "Liste",
        ),
        (
            "  - id: x\n    type: tool_allowlist\n    params: { deny_reason: [x] }\n",
            "Text",
        ),
    ] {
        let e = load_err(rules);
        assert!(e.contains(needle), "{e}");
    }
}

/// Hot-Reload (POL-001, #253): Eine neue Allowlist bringt `allow`-Regeln mit, die Defaults
/// anderer Sets derselben Ebene aufheben können; sie gilt deshalb erst in einer neuen Session.
#[test]
fn pol_001_new_tool_allowlist_counts_as_loosening() {
    let base = set(
        Scope::Project,
        "p",
        "  - id: no-shell\n    on: tool_call\n    when: tool.kind == \"shell\"\n    action: deny\n",
    );
    let with_list = set(
        Scope::Project,
        "p",
        &format!(
            "  - id: no-shell\n    on: tool_call\n    when: tool.kind == \"shell\"\n    action: deny\n{READ_ONLY}"
        ),
    );
    assert!(only_tightens(Some(&base), &with_list).is_err());
    assert!(only_tightens(Some(&with_list), &with_list).is_ok());
    // Entfernen lockert ebenfalls.
    assert!(only_tightens(Some(&with_list), &base).is_err());
}

// ---------------------------------------------------------------------------------------
// POL-016 path_guard
// ---------------------------------------------------------------------------------------

/// Kontext mit Worktree `/repo`, Home `/home/u` und Projektwurzel `/repo`.
fn ws() -> Value {
    json!({ "session": { "worktree": "/repo", "project_root": "/repo" }, "host": { "home": "/home/u" } })
}

/// Pfadauflösung wie im Runner, aber mit fester Symlink-Tabelle (ohne Dateisystem).
struct Links(Vec<(&'static str, &'static str)>);

impl beton_policy::cel::ext::PathResolver for Links {
    fn resolve(&self, path: &str) -> Result<String, String> {
        let lexical = beton_policy::cel::ext::normalize_path(path).ok_or("nicht absolut")?;
        for (link, target) in &self.0 {
            if lexical == *link {
                return Ok((*target).to_owned());
            }
            if let Some(rest) = lexical.strip_prefix(&format!("{link}/")) {
                return Ok(format!("{target}/{rest}"));
            }
        }
        Ok(lexical)
    }
}

const GUARD: &str = "  - id: paths\n    type: path_guard\n";

#[test]
fn pol_016_shell_redirect_targets_are_write_paths() {
    let mut input = Map::new();
    input.insert(
        "tool".into(),
        json!({ "name": "Bash", "args": { "command": "echo x > /etc/hosts && cp /usr/share/a ./b | tee -a ../log" } }),
    );
    let (tool, _) = beton_policy::policytest::tool_context(
        input["tool"].as_object().unwrap(),
        "claude",
        "/repo",
        Some("/home/u"),
    )
    .unwrap();
    assert!(
        tool["paths"]
            .as_array()
            .unwrap()
            .contains(&json!("/etc/hosts")),
        "{tool}"
    );
    assert_eq!(
        tool["write_paths"],
        json!(["/etc/hosts", "/repo/b", "/log"]),
        "{tool}"
    );
    let (write, _) = beton_policy::policytest::tool_context(
        json!({ "name": "Write", "args": { "file_path": "/repo/a", "content": "x" } })
            .as_object()
            .unwrap(),
        "claude",
        "/repo",
        None,
    )
    .unwrap();
    assert_eq!(write["write_paths"], json!(["/repo/a"]));
    let (read, _) = beton_policy::policytest::tool_context(
        json!({ "name": "Read", "args": { "file_path": "/repo/a" } })
            .as_object()
            .unwrap(),
        "claude",
        "/repo",
        None,
    )
    .unwrap();
    assert_eq!(read["write_paths"], json!([]));
}

#[test]
fn pol_016_ac1_write_outside_the_worktree_is_denied_after_normalization() {
    let s = set(Scope::Project, "p", GUARD);
    let d = eval(
        vec![s.clone()],
        &call_with(
            "Write",
            json!({ "file_path": "/repo/../other/x", "content": "x" }),
            ws(),
        ),
    );
    assert_eq!(outcome(&d), "deny", "{d:?}");
    assert_eq!(reasons(&d)[0].0, "paths/write");
    let d = eval(
        vec![s],
        &call_with(
            "Write",
            json!({ "file_path": "/repo/src/../x", "content": "x" }),
            ws(),
        ),
    );
    assert_eq!(outcome(&d), "allow", "{d:?}");
}

#[test]
fn pol_016_ac2_symlink_into_ssh_is_denied_when_reading() {
    let s = set(Scope::Project, "p", GUARD);
    let act = call_with("Read", json!({ "file_path": "/repo/notes.txt" }), ws()).with_resolver(
        Arc::new(Links(vec![("/repo/notes.txt", "/home/u/.ssh/id_ed25519")])),
    );
    let d = eval(vec![s.clone()], &act);
    assert_eq!(outcome(&d), "deny", "{d:?}");
    assert_eq!(reasons(&d)[0].0, "paths/read");
    // Ohne Symlink ist dieselbe Datei frei lesbar.
    let d = eval(
        vec![s],
        &call_with("Read", json!({ "file_path": "/repo/notes.txt" }), ws()),
    );
    assert_eq!(outcome(&d), "allow", "{d:?}");
}

/// Dasselbe mit einem echten Symlink und einer Auflösung über das Dateisystem (wie
/// `FsResolver` im Runner).
#[test]
fn pol_016_ac2_real_symlink_is_resolved_before_matching() {
    struct Fs;
    impl beton_policy::cel::ext::PathResolver for Fs {
        fn resolve(&self, path: &str) -> Result<String, String> {
            let lexical = beton_policy::cel::ext::normalize_path(path).ok_or("nicht absolut")?;
            Ok(std::fs::canonicalize(&lexical)
                .map(|p| p.display().to_string())
                .unwrap_or(lexical))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let home = root.join("home");
    let wt = root.join("wt");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::write(home.join(".ssh/id_ed25519"), "x").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(home.join(".ssh/id_ed25519"), wt.join("key")).unwrap();
    #[cfg(not(unix))]
    return;
    let fixtures = json!({
        "session": { "worktree": wt.display().to_string() },
        "host": { "home": home.display().to_string() },
    });
    let mut input = Map::new();
    input.insert(
        "tool".into(),
        json!({ "name": "Read", "args": { "file_path": wt.join("key").display().to_string() } }),
    );
    let act = activation(
        Phase::ToolCall,
        &fixtures,
        &input,
        Some("claude"),
        Some(&home.display().to_string()),
    )
    .unwrap()
    .with_resolver(Arc::new(Fs));
    let d = eval(vec![set(Scope::User, "u", GUARD)], &act);
    assert_eq!(outcome(&d), "deny", "{d:?}");
}

#[test]
fn pol_016_ac3_shell_redirect_to_etc_hosts_is_denied() {
    let s = set(Scope::Project, "p", GUARD);
    let act = call_with("Bash", json!({ "command": "echo x > /etc/hosts" }), ws());
    let d = eval(vec![s.clone()], &act);
    assert_eq!(outcome(&d), "deny", "{d:?}");
    assert_eq!(reasons(&d)[0].0, "paths/write");
    // Lesen außerhalb des Worktrees ist ohne `read.allow` frei, Schreiben im Worktree auch.
    for command in ["cat /etc/hosts", "echo x > ./out.txt", "cargo test"] {
        let d = eval(
            vec![s.clone()],
            &call_with("Bash", json!({ "command": command }), ws()),
        );
        assert_eq!(outcome(&d), "allow", "{command}: {d:?}");
    }
}

#[test]
fn pol_016_sensitive_patterns_are_denied_by_default() {
    let s = set(Scope::Project, "p", GUARD);
    for (name, args) in [
        ("Read", json!({ "file_path": "/repo/.env" })),
        ("Read", json!({ "file_path": "/repo/sub/.env.local" })),
        ("Read", json!({ "file_path": "/repo/.git/config" })),
        ("Read", json!({ "file_path": "/home/u/.aws/credentials" })),
        ("Bash", json!({ "command": "cat ~/.ssh/id_rsa" })),
        (
            "Edit",
            json!({ "file_path": "/repo/.env", "old_string": "a", "new_string": "b" }),
        ),
    ] {
        let d = eval(vec![s.clone()], &call_with(name, args.clone(), ws()));
        assert_eq!(outcome(&d), "deny", "{name} {args}: {d:?}");
    }
}

#[test]
fn pol_016_custom_lists_variables_and_on_violation_ask() {
    let s = set(
        Scope::User,
        "u",
        "  - id: strict\n    type: path_guard\n    params:\n      read: { allow: [\"${project_root}/**\", \"~/notes/*.md\"], deny: [\"**/secret*\"] }\n      write: { allow: [\"${worktree}/src/**\"] }\n      on_violation: ask\n",
    );
    for (name, args, want) in [
        ("Read", json!({ "file_path": "/repo/README.md" }), "allow"),
        (
            "Read",
            json!({ "file_path": "/home/u/notes/a.md" }),
            "allow",
        ),
        (
            "Read",
            json!({ "file_path": "/home/u/notes/sub/a.md" }),
            "ask",
        ),
        ("Read", json!({ "file_path": "/repo/secret.txt" }), "ask"),
        ("Read", json!({ "file_path": "/etc/hosts" }), "ask"),
        (
            "Write",
            json!({ "file_path": "/repo/src/a.rs", "content": "x" }),
            "allow",
        ),
        (
            "Write",
            json!({ "file_path": "/repo/README.md", "content": "x" }),
            "ask",
        ),
    ] {
        let d = eval(vec![s.clone()], &call_with(name, args.clone(), ws()));
        assert_eq!(outcome(&d), want, "{name} {args}: {d:?}");
    }
}

#[test]
fn pol_016_params_are_checked_when_loading() {
    for (params, needle) in [
        ("{ reed: {} }", "reed"),
        ("{ read: { allow: [\"src/**\"] } }", "absolut"),
        ("{ read: { allow: [\"${repo}/**\"] } }", "repo"),
        ("{ write: { deny: x } }", "Liste"),
        ("{ write: { block: [] } }", "block"),
        ("{ on_violation: allow }", "on_violation"),
    ] {
        let e = load_err(&format!(
            "  - id: x\n    type: path_guard\n    params: {params}\n"
        ));
        assert!(e.contains(needle), "{params}: {e}");
    }
}

/// Ohne Worktree bzw. Home lässt sich nicht prüfen: ablehnen (fail closed).
#[test]
fn pol_016_missing_worktree_fails_closed() {
    let s = set(Scope::Project, "p", GUARD);
    let d = eval(
        vec![s],
        &call_with(
            "Write",
            json!({ "file_path": "/repo/a", "content": "x" }),
            json!({ "session": { "worktree": null }, "host": { "home": "/home/u" } }),
        ),
    );
    assert_eq!(outcome(&d), "deny", "{d:?}");
}

/// `resolve(path)` (POL-002, für path_guard): normalisiert und löst Symlinks über die
/// Pfadauflösung des Kontexts auf; relative Pfade sind ein Fehler (fail closed).
#[test]
fn pol_002_resolve_uses_the_path_resolution() {
    let s = set(
        Scope::Project,
        "p",
        "  - id: r\n    on: tool_call\n    when: 'resolve(tool.cwd + \"/x/../link\") == \"/target\"'\n    action: deny\n",
    );
    let plain = call("Read", json!({ "file_path": "/repo/a" }));
    assert_eq!(outcome(&eval(vec![s.clone()], &plain)), "allow");
    let linked = plain
        .clone()
        .with_resolver(Arc::new(Links(vec![("/repo/link", "/target")])));
    assert_eq!(outcome(&eval(vec![s], &linked)), "deny");
    let relative = set(
        Scope::Project,
        "p",
        "  - id: r\n    on: tool_call\n    when: 'resolve(\"rel\") == \"\"'\n    action: deny\n",
    );
    let d = eval(vec![relative], &plain);
    assert_eq!(outcome(&d), "deny");
    assert!(!d.errors.is_empty(), "{d:?}");
}

// ---------------------------------------------------------------------------------------
// POL-017 git_guard
// ---------------------------------------------------------------------------------------

/// Wertet `expr` für ein Shell-Kommando aus (über `notify.message`, die Werte als JSON zeigt).
fn shell_value(expr: &str, command: &str) -> String {
    let s = set(
        Scope::Project,
        "p",
        &format!(
            "  - id: show\n    on: tool_call\n    action: notify\n    notify: {{ channels: [log], message: \"{{{{ {expr} }}}}\" }}\n"
        ),
    );
    let d = eval(vec![s], &call("Bash", json!({ "command": command })));
    assert!(d.errors.is_empty(), "{command}: {:?}", d.errors);
    d.notifications[0].message.clone().unwrap()
}

#[test]
fn pol_017_git_ops_classify_subcommands() {
    for (command, ops) in [
        ("git status", "[]"),
        ("git push", r#"["push"]"#),
        ("git push --force", r#"["push","force_push"]"#),
        ("git push origin +main", r#"["push","force_push"]"#),
        ("git -C x push -f", r#"["push","force_push"]"#),
        (
            "cd a && git push --force-with-lease",
            r#"["push","force_push"]"#,
        ),
        (
            "git push --force-with-lease=main:abc origin feature",
            r#"["push","force_push"]"#,
        ),
        ("git push --mirror backup", r#"["push","force_push"]"#),
        (
            "git -c core.x=y push -uf origin x",
            r#"["push","force_push"]"#,
        ),
        ("sudo git push --force", r#"["push","force_push"]"#),
        ("git push origin feature && rm -f x", r#"["push"]"#),
        (
            "git push origin --delete old",
            r#"["push","delete_branch"]"#,
        ),
        ("git push origin :old", r#"["push","delete_branch"]"#),
        ("git branch -D old", r#"["delete_branch"]"#),
        ("git branch --delete old", r#"["delete_branch"]"#),
        ("git branch new", "[]"),
        ("git merge feature", r#"["merge"]"#),
        ("git merge --abort", "[]"),
        ("git reset --hard HEAD~1", r#"["history_rewrite"]"#),
        ("git reset HEAD~1", "[]"),
        ("git rebase main", r#"["history_rewrite"]"#),
        ("git rebase --abort", "[]"),
        ("git filter-branch --all", r#"["history_rewrite"]"#),
        ("git commit --amend --no-edit", r#"["history_rewrite"]"#),
        ("git commit -m x", "[]"),
    ] {
        assert_eq!(
            shell_value("shell.git_ops(tool.argv)", command),
            ops,
            "{command}"
        );
    }
}

#[test]
fn pol_017_git_push_targets_follow_refspecs() {
    for (command, targets) in [
        ("git push", r#"["HEAD"]"#),
        ("git push origin", r#"["HEAD"]"#),
        ("git push -u origin feature/x", r#"["feature/x"]"#),
        ("git push origin +main", r#"["main"]"#),
        ("git push origin HEAD:main", r#"["main"]"#),
        ("git push origin a:refs/heads/master b", r#"["master","b"]"#),
        ("git push -o ci.skip origin x", r#"["x"]"#),
        ("git push origin :old", r#"["old"]"#),
        ("git push --all origin", r#"["*"]"#),
        ("git push --tags", "[]"),
        ("git status", "[]"),
    ] {
        assert_eq!(
            shell_value("shell.git_push_targets(tool.argv)", command),
            targets,
            "{command}"
        );
    }
}

const GIT: &str = "  - id: git\n    type: git_guard\n";

fn git_call(command: &str, branch: &str) -> Activation {
    call_with(
        "Bash",
        json!({ "command": command }),
        json!({ "session": { "branch": branch } }),
    )
}

fn rule_ids(d: &Decision) -> Vec<String> {
    reasons(d).into_iter().map(|(id, _)| id).collect()
}

#[test]
fn pol_017_ac1_force_push_variants_are_denied() {
    let s = set(Scope::User, "u", GIT);
    for command in [
        "git push --force",
        "git push origin +main",
        "git push origin +feature",
        "git -C x push -f",
        "cd a && git push --force-with-lease",
        "git push --mirror backup",
    ] {
        let d = eval(vec![s.clone()], &git_call(command, "feature"));
        assert_eq!(outcome(&d), "deny", "{command}: {d:?}");
        assert!(
            rule_ids(&d).contains(&"git/force-push".to_owned()),
            "{command}: {d:?}"
        );
    }
}

#[test]
fn pol_017_ac2_push_asks_and_protected_branches_are_denied() {
    let s = set(Scope::User, "u", GIT);
    let d = eval(
        vec![s.clone()],
        &git_call("git push origin feature/x", "feature/x"),
    );
    assert_eq!(outcome(&d), "ask", "{d:?}");
    assert_eq!(rule_ids(&d), ["git/push"]);
    for (command, branch) in [
        ("git push origin main", "feature/x"),
        ("git push origin HEAD:master", "feature/x"),
        ("git push", "main"),
        ("git push origin", "master"),
        ("git push --all origin", "feature/x"),
        ("git merge feature/x", "main"),
    ] {
        let d = eval(vec![s.clone()], &git_call(command, branch));
        assert_eq!(outcome(&d), "deny", "{command} auf {branch}: {d:?}");
        assert!(
            rule_ids(&d).contains(&"git/protected".to_owned()),
            "{command}: {d:?}"
        );
    }
    // Ohne bekannten Branch lässt sich das Ziel nicht prüfen: ablehnen (fail closed).
    let d = eval(
        vec![s],
        &call_with(
            "Bash",
            json!({ "command": "git push" }),
            json!({ "session": { "branch": null } }),
        ),
    );
    assert_eq!(outcome(&d), "deny", "{d:?}");
}

#[test]
fn pol_017_ac3_mcp_merge_follows_the_merge_setting() {
    let pr = || {
        call(
            "mcp__github__merge_pull_request",
            json!({ "pull_number": 1 }),
        )
    };
    let d = eval(vec![set(Scope::User, "u", GIT)], &pr());
    assert_eq!(outcome(&d), "ask", "{d:?}");
    assert_eq!(rule_ids(&d), ["git/mcp-merge"]);
    for (merge, want) in [("deny", "deny"), ("allow", "allow")] {
        let s = set(
            Scope::User,
            "u",
            &format!("{GIT}    params: {{ merge: {merge} }}\n"),
        );
        assert_eq!(outcome(&eval(vec![s], &pr())), want, "merge: {merge}");
    }
    // Eigene MCP-Tools je Operation.
    let s = set(
        Scope::User,
        "u",
        &format!("{GIT}    params: {{ mcp_tools: {{ push: [\"gitea/push_*\"] }} }}\n"),
    );
    let d = eval(vec![s.clone()], &call("mcp__gitea__push_files", json!({})));
    assert_eq!(rule_ids(&d), ["git/mcp-push"]);
    // Ohne `merge`-Liste kein MCP-Merge mehr.
    let d = eval(vec![s], &pr());
    assert_eq!(outcome(&d), "allow", "{d:?}");
}

#[test]
fn pol_017_ac4_uncertain_command_with_git_asks() {
    let s = set(Scope::User, "u", GIT);
    let d = eval(vec![s.clone()], &git_call("eval \"git status\"", "feature"));
    assert_eq!(outcome(&d), "ask", "{d:?}");
    assert_eq!(rule_ids(&d), ["git/uncertain"]);
    // Ohne `git` bleibt ein unsicheres Kommando Sache anderer Regeln.
    let d = eval(vec![s], &git_call("eval \"ls\"", "feature"));
    assert_eq!(outcome(&d), "allow", "{d:?}");
}

#[test]
fn pol_017_other_operations_and_settings() {
    let s = set(Scope::User, "u", GIT);
    for (command, want, rule) in [
        ("git merge feature", "ask", "git/merge"),
        ("git branch -D old", "ask", "git/delete-branch"),
        ("git push origin --delete old", "ask", "git/delete-branch"),
        ("git reset --hard HEAD~1", "ask", "git/history-rewrite"),
        ("git commit --amend", "ask", "git/history-rewrite"),
        ("git status", "allow", ""),
        ("git push origin feature && rm -f x", "ask", "git/push"),
    ] {
        let d = eval(vec![s.clone()], &git_call(command, "feature"));
        assert_eq!(outcome(&d), want, "{command}: {d:?}");
        if !rule.is_empty() {
            assert!(rule_ids(&d).contains(&rule.to_owned()), "{command}: {d:?}");
        }
    }
    // Demo (POL-017): `force_push: ask` macht aus dem Force-Push eine Approval-Card.
    let s = set(
        Scope::User,
        "u",
        &format!("{GIT}    params: {{ force_push: ask, push: allow }}\n"),
    );
    let d = eval(vec![s.clone()], &git_call("git push --force", "feature"));
    assert_eq!(outcome(&d), "ask", "{d:?}");
    assert_eq!(rule_ids(&d), ["git/force-push"]);
    let d = eval(vec![s], &git_call("git push origin feature", "feature"));
    assert_eq!(outcome(&d), "allow", "{d:?}");
    // Eigene geschützte Branches und Aktion.
    let s = set(
        Scope::User,
        "u",
        &format!(
            "{GIT}    params: {{ protected_branches: [\"release/*\"], protected_action: ask }}\n"
        ),
    );
    let d = eval(
        vec![s.clone()],
        &git_call("git push origin release/1.0", "x"),
    );
    assert_eq!(outcome(&d), "ask", "{d:?}");
    assert!(rule_ids(&d).contains(&"git/protected".to_owned()));
    let d = eval(vec![s], &git_call("git push origin main", "x"));
    assert_eq!(rule_ids(&d), ["git/push"]);
}

#[test]
fn pol_017_params_are_checked_when_loading() {
    for (params, needle) in [
        ("{ pushes: ask }", "pushes"),
        ("{ push: maybe }", "push"),
        ("{ force_push: [deny] }", "force_push"),
        ("{ protected_branches: main }", "Liste"),
        ("{ mcp_tools: { rebase: [x] } }", "rebase"),
        ("{ mcp_tools: [x] }", "mcp_tools"),
    ] {
        let e = load_err(&format!(
            "  - id: x\n    type: git_guard\n    params: {params}\n"
        ));
        assert!(e.contains(needle), "{params}: {e}");
    }
}

// ---------------------------------------------------------------------------------------
// POL-018 loop_detection
// ---------------------------------------------------------------------------------------

fn counted(repeat: i64, errors: Option<i64>) -> Activation {
    let mut input = Map::new();
    let mut tool =
        json!({ "name": "Bash", "args": { "command": "npm test" }, "repeat_count": repeat });
    if let Some(e) = errors {
        tool["consecutive_errors"] = json!(e);
    }
    input.insert("tool".into(), tool);
    activation(Phase::ToolCall, &json!({}), &input, Some("claude"), None).unwrap()
}

#[test]
fn pol_018_ac1_third_identical_call_asks() {
    let s = set(Scope::User, "u", "  - id: loop\n    type: loop_detection\n");
    assert_eq!(
        outcome(&eval(vec![s.clone()], &counted(2, Some(0)))),
        "allow"
    );
    let d = eval(vec![s], &counted(3, Some(0)));
    assert_eq!(outcome(&d), "ask", "{d:?}");
    assert_eq!(rule_ids(&d), ["loop/repeat"]);
}

#[test]
fn pol_018_ac2_consecutive_errors_trigger_the_configured_action() {
    let s = set(
        Scope::User,
        "u",
        "  - id: loop\n    type: loop_detection\n    params: { consecutive_errors: 5, action: deny }\n",
    );
    assert_eq!(
        outcome(&eval(vec![s.clone()], &counted(1, Some(4)))),
        "allow"
    );
    let d = eval(vec![s], &counted(1, Some(5)));
    assert_eq!(outcome(&d), "deny", "{d:?}");
    assert_eq!(rule_ids(&d), ["loop/errors"]);
}

#[test]
fn pol_018_notify_reports_without_blocking() {
    let s = set(
        Scope::User,
        "u",
        "  - id: loop\n    type: loop_detection\n    params: { identical_threshold: 2, action: notify }\n",
    );
    let d = eval(vec![s], &counted(2, Some(0)));
    assert_eq!(outcome(&d), "allow", "{d:?}");
    assert_eq!(d.notifications.len(), 1, "{d:?}");
    assert_eq!(d.notifications[0].rule.rule_id, "loop/repeat");
    assert!(
        d.notifications[0]
            .message
            .as_deref()
            .is_some_and(|m| m.contains('2')),
        "{:?}",
        d.notifications
    );
}

/// Ohne Zähler (z. B. `beton policy eval` ohne Session) greift die Erkennung nicht; sie
/// ist eine Heuristik gegen festgefahrene Agents, keine Schutzgrenze.
#[test]
fn pol_018_without_counters_the_rule_does_not_fire() {
    let s = set(Scope::User, "u", "  - id: loop\n    type: loop_detection\n");
    let d = eval(vec![s], &call("Bash", json!({ "command": "ls" })));
    assert_eq!(outcome(&d), "allow", "{d:?}");
    assert!(d.errors.is_empty(), "{:?}", d.errors);
}

#[test]
fn pol_018_params_are_checked_when_loading() {
    for (params, needle) in [
        ("{ window: 20 }", "window"),
        ("{ identical_threshold: 1 }", "identical_threshold"),
        ("{ identical_threshold: x }", "Zahl"),
        ("{ consecutive_errors: 0 }", "consecutive_errors"),
        ("{ action: allow }", "action"),
        ("{ threshold: 3 }", "threshold"),
    ] {
        let e = load_err(&format!(
            "  - id: x\n    type: loop_detection\n    params: {params}\n"
        ));
        assert!(e.contains(needle), "{params}: {e}");
    }
}

// ---------------------------------------------------------------------------------------
// User-Defaults von `beton setup` (POL-001)
// ---------------------------------------------------------------------------------------

fn user_defaults() -> String {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../policies/user-defaults.yaml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `beton setup` legt `~/.beton/policies/defaults.yaml` an: `git_guard` (Demo: `force_push:
/// ask`) und `loop_detection` mit `id: loop`, Schema als lokale Datei (ADR-0033).
#[test]
fn pol_001_user_defaults_have_git_guard_and_loop_detection_with_local_schema() {
    let text = user_defaults();
    assert!(
        text.lines().any(|l| l == beton_policy::doc::SCHEMA_COMMENT),
        "{text}"
    );
    assert!(!text.contains("http"), "nie eine URL (ADR-0033)");
    let s = parse_set(Scope::User, "defaults.yaml", &text).unwrap();
    let docs: Vec<_> = s
        .rules
        .iter()
        .map(|r| (r.doc.id.as_str(), r.doc.rule_type))
        .collect();
    assert!(docs.contains(&("git", Some(beton_policy::doc::RuleType::GitGuard))));
    assert!(docs.contains(&("loop", Some(beton_policy::doc::RuleType::LoopDetection))));
    let d = eval(
        vec![Arc::new(s)],
        &git_call("git push --force", "feature/x"),
    );
    assert_eq!(outcome(&d), "ask", "{d:?}");
}

/// POL-026: `expect.rules` mit der ID einer Typ-Regel (`loop`) trifft ihre entzuckerten
/// Teilregeln (`loop/repeat`), wie im Beispiel der Spec; eine fremde ID mit demselben Anfang
/// (`loo`) nicht.
#[test]
fn pol_026_expected_rule_id_of_a_rule_type_matches_its_parts() {
    use beton_policy::policytest::{Summary, TestOptions, run_paths};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("loop.yaml"),
        "spec_version: 1\nname: l\nrules:\n  - id: loop\n    type: loop_detection\n",
    )
    .unwrap();
    let case = |rules: &str| {
        format!(
            "spec_version: 1\nlayers: {{ user: [loop.yaml] }}\ncases:\n  - name: x\n    phase: tool_call\n    input: {{ tool: {{ name: Bash, args: {{ command: ls }}, repeat_count: 3 }} }}\n    expect: {{ outcome: ask, rules: [{rules}] }}\n"
        )
    };
    let run = |rules: &str| {
        let file = dir.path().join("t.policytest.yaml");
        std::fs::write(&file, case(rules)).unwrap();
        Summary::of(&run_paths(&[file], &TestOptions::default()).unwrap()).exit_code()
    };
    assert_eq!(run("loop/repeat"), 0);
    assert_eq!(run("loop"), 0);
    assert_eq!(run("loo"), 1);
    assert_eq!(run("loop/errors"), 1);
}

/// Hot-Reload (POL-001): Neue `path_guard`-, `git_guard`- und `loop_detection`-Regeln erzeugen
/// nur `deny`/`ask`/`notify` und gelten sofort; eine geänderte bzw. entfernte erst in einer
/// neuen Session.
#[test]
fn pol_001_new_guards_count_as_tightening_and_changes_as_loosening() {
    let base = set(
        Scope::User,
        "u",
        "  - id: keep\n    on: tool_call\n    action: notify\n",
    );
    for added in [
        GUARD.to_owned(),
        GIT.to_owned(),
        format!("{GIT}    params: {{ force_push: ask }}\n"),
        "  - id: loop\n    type: loop_detection\n    params: { action: notify }\n".to_owned(),
    ] {
        let rules = format!("  - id: keep\n    on: tool_call\n    action: notify\n{added}");
        let new = set(Scope::User, "u", &rules);
        assert!(only_tightens(Some(&base), &new).is_ok(), "{added}");
        assert!(only_tightens(Some(&new), &base).is_err(), "{added}");
    }
    let strict = set(Scope::User, "u", GIT);
    let lax = set(
        Scope::User,
        "u",
        &format!("{GIT}    params: {{ push: allow }}\n"),
    );
    assert!(only_tightens(Some(&strict), &lax).is_err());
}

/// POL-016: Auch geschriebene Pfade unterliegen den Leseregeln (`mv` liest die Quelle, bevor
/// es sie entfernt); sonst verschöbe ein Kommando eine sensible Datei an den Leseregeln
/// vorbei.
#[test]
fn pol_016_read_rules_apply_to_written_paths_too() {
    let s = set(
        Scope::User,
        "u",
        "  - id: paths\n    type: path_guard\n    params: { write: { allow: [\"**\"], deny: [] } }\n",
    );
    let d = eval(
        vec![s.clone()],
        &call_with("Bash", json!({ "command": "mv ~/.ssh/id_rsa ./x" }), ws()),
    );
    assert_eq!(outcome(&d), "deny", "{d:?}");
    assert_eq!(rule_ids(&d), ["paths/read"]);
    let d = eval(
        vec![s],
        &call_with("Bash", json!({ "command": "mv ./a ./b" }), ws()),
    );
    assert_eq!(outcome(&d), "allow", "{d:?}");
}

/// POL-017: Wechselt dasselbe Kommando vorher den Branch (`git checkout`, `git switch`), ist
/// der aktuelle Branch bei der Auswertung nicht der beim Push bzw. Merge: geschützt.
#[test]
fn pol_017_branch_switch_in_the_same_command_counts_as_protected() {
    let s = set(
        Scope::User,
        "u",
        &format!("{GIT}    params: {{ push: allow, merge: allow }}\n"),
    );
    for command in [
        "git checkout main && git push",
        "git switch main && git merge feature",
        "git -C . checkout main; git push origin",
    ] {
        let d = eval(vec![s.clone()], &git_call(command, "feature"));
        assert_eq!(outcome(&d), "deny", "{command}: {d:?}");
        assert_eq!(rule_ids(&d), ["git/protected"], "{command}");
    }
    // Mit ausdrücklichem Ziel zählt das Ziel.
    let d = eval(
        vec![s],
        &git_call("git switch -c topic && git push -u origin topic", "feature"),
    );
    assert_eq!(outcome(&d), "allow", "{d:?}");
}
