//! POL-002: CEL-Engine & Erweiterungsfunktionen.

use std::sync::atomic::{AtomicU64, Ordering};

use beton_policy::Phase;
use beton_policy::cel::{Clock, EvalError, Limits, Meter, compile_bool, compile_expr};
use beton_policy::context::Activation;
use beton_policy::types::Ty;
use beton_policy::value::Value;
use serde_json::json;

fn tool_ctx(tool: serde_json::Value) -> Activation {
    Activation::new(
        Phase::ToolCall,
        json!({ "tool": tool, "session": { "cost_usd": 3.5 } }),
    )
    .expect("gültiger Kontext")
}

fn eval(src: &str, act: &Activation) -> Result<Value, EvalError> {
    let program = compile_expr(src, Phase::ToolCall, Ty::Dyn).unwrap_or_else(|e| panic!("{e}"));
    program.eval(act, &mut Meter::unlimited())
}

fn eval_bool(src: &str, act: &Activation) -> bool {
    let program = compile_bool(src, Phase::ToolCall).unwrap_or_else(|e| panic!("{src}: {e}"));
    program
        .eval_bool(act, &mut Meter::unlimited())
        .unwrap_or_else(|e| panic!("{src}: {e}"))
}

fn empty() -> Activation {
    tool_ctx(json!({ "name": "Read", "args": {} }))
}

#[test]
fn pol_002_ac1_type_error_is_rejected_at_load_with_position() {
    let err = compile_bool(r#"session.cost_usd > "5""#, Phase::ToolCall).unwrap_err();
    assert_eq!((err.line, err.column), (1, 18), "{err}");
    assert!(
        err.message.contains("double") && err.message.contains("string"),
        "{err}"
    );
}

#[test]
fn pol_002_ac1_position_on_later_line() {
    let src = "tool.kind == \"shell\" &&\n  tool.argv.size() + \"x\" > 2";
    let err = compile_bool(src, Phase::ToolCall).unwrap_err();
    assert_eq!(err.line, 2, "{err}");
}

#[test]
fn pol_002_ac1_other_load_errors() {
    for (src, needle) in [
        ("session.nope > 1.0", "nope"),
        ("unknown_var", "unknown_var"),
        ("tool.kind", "bool"),
        ("result.text == \"x\"", "tool_result"),
        ("tool.command.matches(\"(\")", "regex"),
        ("glob(tool.name)", "glob"),
        ("tool.kind ==", "syntax"),
        ("1 + 1.0 > 0", "int"),
    ] {
        let err = compile_bool(src, Phase::ToolCall).unwrap_err();
        assert!(
            err.message.to_lowercase().contains(needle),
            "{src}: {err} enthält nicht {needle}"
        );
    }
}

#[test]
fn pol_002_ac1_numeric_comparison_across_types_is_allowed() {
    let act = empty();
    assert!(eval_bool("session.cost_usd > 3", &act));
    assert!(eval_bool("session.cost_usd < 4.0", &act));
    assert!(eval_bool("1 == 1.0 && 2u > 1", &act));
}

#[test]
fn pol_002_ac2_missing_command_is_absorbed_to_false() {
    let src = r#"tool.name == "bash" && tool.args.command.matches("rm -rf")"#;
    let act = tool_ctx(json!({ "name": "Read", "args": { "file_path": "/x" } }));
    assert!(!eval_bool(src, &act));
    // kommutativ: der Fehler links wird vom `false` rechts absorbiert
    let src = r#"tool.args.command.matches("rm -rf") && tool.name == "bash""#;
    assert!(!eval_bool(src, &act));
    let src = r#"tool.name == "Read" || tool.args.command.matches("x")"#;
    assert!(eval_bool(src, &act));
}

#[test]
fn pol_002_ac2_unabsorbed_error_is_an_error() {
    let src = r#"tool.name == "Read" && tool.args.command.matches("rm -rf")"#;
    let act = tool_ctx(json!({ "name": "Read", "args": {} }));
    let program = compile_bool(src, Phase::ToolCall).unwrap();
    assert!(program.eval_bool(&act, &mut Meter::unlimited()).is_err());
}

#[test]
fn pol_002_ac2_matching_command_is_true() {
    let src = r#"tool.name == "bash" && tool.args.command.matches("rm -rf|git push")"#;
    let act = tool_ctx(json!({ "name": "bash", "args": { "command": "git push origin" } }));
    assert!(eval_bool(src, &act));
}

#[test]
fn pol_002_ac3_under() {
    let act = empty();
    assert!(!eval_bool(r#"under("/repo/../etc/passwd", "/repo")"#, &act));
    assert!(eval_bool(
        r#"under("/repo/src/../lib/a.rs", "/repo")"#,
        &act
    ));
    assert!(eval_bool(r#"under("/repo", "/repo/")"#, &act));
    assert!(!eval_bool(r#"under("/repository/x", "/repo")"#, &act));
    assert!(!eval_bool(r#"under("/../../etc", "/repo")"#, &act));
    // relative Pfade sind nicht entscheidbar: Fehler statt stiller Annahme
    let program = compile_bool(r#"under("a/b", "/repo")"#, Phase::ToolCall).unwrap();
    assert!(program.eval_bool(&act, &mut Meter::unlimited()).is_err());
}

#[test]
fn pol_002_ac3_glob() {
    let act = empty();
    assert!(eval_bool(
        r#"glob("a.b.example.com", "*.example.com")"#,
        &act
    ));
    assert!(!eval_bool(r#"glob("example.com", "*.example.com")"#, &act));
    assert!(eval_bool(r#"glob("/repo/src/a.rs", "/repo/**")"#, &act));
    assert!(eval_bool(r#"glob("/repo/src/a.rs", "/repo/*/a.rs")"#, &act));
    assert!(!eval_bool(
        r#"glob("/repo/src/x/a.rs", "/repo/*/a.rs")"#,
        &act
    ));
    assert!(eval_bool(r#"glob("/home/u/.env.local", "**/.env*")"#, &act));
    assert!(eval_bool(r#"glob("a+b(c)", "a+b(?)")"#, &act));
    assert!(!eval_bool(r#"glob("ab", "a")"#, &act));
}

#[test]
fn pol_002_ac3_domain_matches() {
    let act = empty();
    assert!(eval_bool(
        r#"domain_matches("a.b.example.com", "*.example.com")"#,
        &act
    ));
    assert!(eval_bool(
        r#"domain_matches("Docs.Example.COM.", "*.example.com")"#,
        &act
    ));
    assert!(!eval_bool(
        r#"domain_matches("example.com", "*.example.com")"#,
        &act
    ));
    assert!(!eval_bool(
        r#"domain_matches("evilexample.com", "*.example.com")"#,
        &act
    ));
    assert!(eval_bool(
        r#"domain_matches("example.com", "example.com")"#,
        &act
    ));
}

#[test]
fn pol_002_ac3_shell_functions() {
    let act = tool_ctx(json!({
        "name": "Bash",
        "argv": [["cd", "repo"], ["git", "-C", ".", "push", "-f", "origin", "main"]],
    }));
    assert!(eval_bool(r#"shell.has_command(tool.argv, "git")"#, &act));
    assert!(eval_bool(
        r#"shell.has_command(tool.argv, "git", "push")"#,
        &act
    ));
    assert!(!eval_bool(
        r#"shell.has_command(tool.argv, "git", "pull")"#,
        &act
    ));
    assert!(eval_bool(r#"shell.has_flag(tool.argv, "-f")"#, &act));
    assert!(!eval_bool(r#"shell.has_flag(tool.argv, "--force")"#, &act));
}

#[test]
fn pol_002_ac3_url_functions() {
    let act = empty();
    assert!(eval_bool(
        r#"url.host("https://User@Docs.rs:443/a/b?q=1") == "docs.rs""#,
        &act
    ));
    assert!(eval_bool(
        r#"url.path("https://docs.rs/a/b?q=1") == "/a/b""#,
        &act
    ));
    assert!(eval_bool(
        r#"url.host("http://evil.com\\@good.com/") == "evil.com""#,
        &act
    ));
    let program = compile_bool(r#"url.host("kein url") == """#, Phase::ToolCall).unwrap();
    assert!(program.eval_bool(&act, &mut Meter::unlimited()).is_err());
}

#[test]
fn pol_002_ac3_bytes_human() {
    let act = empty();
    assert_eq!(
        eval("bytes_human(512)", &act).unwrap(),
        Value::from("512 B")
    );
    assert_eq!(
        eval("bytes_human(1536)", &act).unwrap(),
        Value::from("1.5 KiB")
    );
    assert_eq!(
        eval("bytes_human(5242880)", &act).unwrap(),
        Value::from("5.0 MiB")
    );
}

#[test]
fn pol_002_standard_functions_and_macros() {
    let act = tool_ctx(json!({
        "name": "Bash",
        "command": "git push",
        "paths": ["/repo/a", "/etc/b"],
        "args": { "command": "git push", "n": 3 },
    }));
    for src in [
        r#"has(tool.args.command) && !has(tool.args.other)"#,
        r#"tool.paths.exists(p, p.startsWith("/etc"))"#,
        r#"!tool.paths.all(p, p.startsWith("/repo"))"#,
        r#"tool.paths.filter(p, p.endsWith("a")).size() == 1"#,
        r#"tool.paths.map(p, size(p)).exists(n, n == 6)"#,
        r#"tool.command.contains("push") && tool.command.startsWith("git")"#,
        r#"size(tool.paths) == 2 && "/etc/b" in tool.paths"#,
        r#"duration("1h30m") > duration("90s")"#,
        r#"timestamp("2026-01-01T12:00:00Z") + duration("1h") > timestamp("2026-01-01T12:30:00Z")"#,
        r#"int(tool.args.n) * 2 == 6 && string(1) == "1" && double(2) == 2.0"#,
        r#"tool.args["command"] == "git push""#,
        r#"{"a": 1}.a == 1 && [1, 2][1] == 2"#,
        r#"(tool.name == "Bash" ? 1 : 2) == 1"#,
        r#"tool.paths.exists_one(p, p.contains("etc"))"#,
        r#"matches(tool.command, "^git")"#,
        r#"tool.native_name == null"#,
    ] {
        assert!(eval_bool(src, &act), "{src}");
    }
}

#[test]
fn pol_002_runtime_errors_are_errors() {
    let act = tool_ctx(json!({ "name": "Bash", "args": { "n": 0 } }));
    for src in [
        "1 / int(tool.args.n) == 1",
        "[1][5] == 1",
        "9223372036854775807 + int(tool.args.n) + 1 > 0",
        r#"tool.args.missing == 1"#,
    ] {
        let program = compile_bool(src, Phase::ToolCall).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert!(
            program.eval_bool(&act, &mut Meter::unlimited()).is_err(),
            "{src}"
        );
    }
}

#[test]
fn pol_002_expression_size_limit() {
    let src = format!("tool.name == \"{}\"", "x".repeat(5000));
    let err = compile_bool(&src, Phase::ToolCall).unwrap_err();
    assert!(err.message.contains("4096"), "{err}");
}

#[test]
fn pol_002_cost_limit_turns_into_error() {
    let items: Vec<i64> = (0..200).collect();
    let act = tool_ctx(json!({ "name": "Bash", "args": { "items": items } }));
    let src = "tool.args.items.all(x, tool.args.items.all(y, x != y || x == y))";
    let program = compile_bool(src, Phase::ToolCall).unwrap();
    let mut meter = Meter::new(&Limits::default(), &SystemClockForTest);
    assert_eq!(
        program.eval_bool(&act, &mut meter),
        Err(EvalError::CostExceeded)
    );
    // dieselbe Regel ohne Verschachtelung bleibt unter dem Budget
    let program = compile_bool("tool.args.items.all(x, x >= 0)", Phase::ToolCall).unwrap();
    let mut meter = Meter::new(&Limits::default(), &SystemClockForTest);
    assert_eq!(program.eval_bool(&act, &mut meter), Ok(true));
}

/// Uhr, die bei jedem Ablesen um 1 ms weiterläuft (Fehlerinjektion für das Zeitbudget).
struct SlowClock(AtomicU64);

impl Clock for SlowClock {
    fn now_us(&self) -> u64 {
        self.0.fetch_add(1_000, Ordering::SeqCst)
    }
}

struct SystemClockForTest;

impl Clock for SystemClockForTest {
    fn now_us(&self) -> u64 {
        0
    }
}

#[test]
fn pol_002_time_budget_exceeded_is_an_error() {
    let items: Vec<i64> = (0..2000).collect();
    let act = tool_ctx(json!({ "name": "Bash", "args": { "items": items } }));
    let program = compile_bool("tool.args.items.all(x, x >= 0)", Phase::ToolCall).unwrap();
    let clock = SlowClock(AtomicU64::new(0));
    let limits = Limits {
        max_cost: u64::MAX,
        ..Limits::default()
    };
    let mut meter = Meter::new(&limits, &clock);
    assert_eq!(
        program.eval_bool(&act, &mut meter),
        Err(EvalError::Deadline)
    );
}

#[test]
fn pol_002_deep_or_long_expressions_are_load_errors_not_crashes() {
    let act = empty();
    // Tiefe Klammern scheitern beim Laden statt den Stack zu sprengen.
    for (src, needle) in [
        (
            format!("{}true{}", "(".repeat(2000), ")".repeat(2000)),
            "verschachtelt",
        ),
        (
            format!("{}1{}", "[".repeat(40), "]".repeat(40)),
            "verschachtelt",
        ),
        (format!("{} > 0", vec!["1"; 1000].join("+")), "tief"),
    ] {
        let src = &src[..src.len().min(4096)];
        let err = compile_bool(src, Phase::ToolCall).unwrap_err();
        assert!(err.message.contains(needle), "{err}");
    }
    // Klammern in Strings zählen nicht.
    let p = compile_bool(&format!("\"{}\" != \"\"", "(".repeat(100)), Phase::ToolCall).unwrap();
    assert!(p.eval_bool(&act, &mut Meter::unlimited()).unwrap());
    // Innerhalb der Grenzen: auf einem Thread mit kleinem Stack (wie ein Tokio-Worker)
    // auswertbar.
    let chain = vec!["true"; 60].join(" && ");
    let nested = format!("{}true{}", "(".repeat(30), ")".repeat(30));
    for src in [chain, nested] {
        let p = compile_bool(&src, Phase::ToolCall).unwrap();
        let act = act.clone();
        let ok = std::thread::Builder::new()
            .stack_size(256 << 10)
            .spawn(move || p.eval_bool(&act, &mut Meter::unlimited()))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(ok, Ok(true));
    }
}

/// #298: Integrationstests heben das Zeitbudget über eine Variable an, die nur Builds mit dem
/// Feature `test-budget` lesen (#304); andere Builds kennen weder die Variable noch
/// [`Limits::with_test_budget`].
#[cfg(feature = "test-budget")]
mod test_budget {
    use beton_policy::cel::{Limits, TEST_BUDGET_ENV};

    #[test]
    fn pol_002_test_budget_raises_only_the_time_budget() {
        let limits = Limits::default().with_test_budget(Some("1000"));
        assert_eq!(limits.eval_budget_us, 1_000_000);
        assert_eq!(limits.max_cost, Limits::default().max_cost);
        assert_eq!(limits.max_expr_bytes, Limits::default().max_expr_bytes);
    }

    #[test]
    fn pol_002_test_budget_never_lowers_and_ignores_invalid_values() {
        for raw in [
            None,
            Some(""),
            Some("1"),
            Some("0"),
            Some("-5"),
            Some("1s"),
            Some("abc"),
        ] {
            assert_eq!(
                Limits::default().with_test_budget(raw),
                Limits::default(),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn pol_002_test_budget_is_capped() {
        let limits = Limits::default().with_test_budget(Some("999999999999"));
        assert_eq!(limits.eval_budget_us, 60_000_000);
    }

    /// #304: Daemon und Runner warnen beim Start, wenn das Testbudget die 10 ms anhebt.
    #[test]
    fn pol_002_active_test_budget_is_reported_only_when_it_raises_the_budget() {
        let expected = std::env::var(TEST_BUDGET_ENV)
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|ms| *ms > 10)
            .map(|ms| ms.min(60_000));
        assert_eq!(Limits::active_test_budget_ms(), expected);
    }

    #[test]
    fn pol_002_runtime_limits_are_the_defaults_plus_the_test_budget() {
        assert_eq!(TEST_BUDGET_ENV, "BETON_TEST_POLICY_BUDGET_MS");
        let raw = std::env::var(TEST_BUDGET_ENV).ok();
        assert_eq!(
            Limits::runtime(),
            Limits::default().with_test_budget(raw.as_deref())
        );
    }
}

/// #304: Ohne das Feature `test-budget` gelten im Betrieb immer die Grenzen aus POL-002, auch
/// wenn `BETON_TEST_POLICY_BUDGET_MS` gesetzt ist (in Testläufen über `.cargo/config.toml`).
/// Das Feature schalten nur Dev-Dependencies ein; die CI prüft diesen Test zusätzlich im
/// Release-Build mit gesetzter Variable.
#[cfg(not(feature = "test-budget"))]
#[test]
fn pol_002_without_the_test_budget_feature_runtime_limits_are_the_defaults() {
    assert_eq!(Limits::runtime(), Limits::default());
    assert_eq!(Limits::active_test_budget_ms(), None);
}

#[test]
fn pol_002_default_time_budget_stays_10_ms() {
    assert_eq!(Limits::default().eval_budget_us, 10_000);
}
