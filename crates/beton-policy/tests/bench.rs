//! POL-002 AC4: 200 Regeln in Phase `tool_call`, p99 < 2 ms.
//!
//! Läuft im Release-Build in der CI:
//! `cargo test -p beton-policy --release --test bench -- --ignored --nocapture`

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Instant;

use beton_policy::Phase;
use beton_policy::context::Activation;
use beton_policy::engine::{EffectiveSet, EvalOptions};
use beton_policy::load::{Scope, parse_set};
use beton_policy::normalize::{RawToolCall, normalize};
use serde_json::json;

/// Typische Bedingungen; jede Regel bekommt eine davon mit eigener Konstante.
fn condition(i: usize) -> String {
    match i % 8 {
        0 => format!("tool.kind == \"shell\" && tool.command.matches(\"rm -rf|curl.*{i}\")"),
        1 => format!(
            "shell.has_command(tool.argv, \"git\", \"push\") && shell.has_flag(tool.argv, \"--x{i}\")"
        ),
        2 => format!("tool.paths.exists(p, glob(p, \"/etc/**\") || under(p, \"/secret/{i}\"))"),
        3 => format!("tool.urls.exists(u, domain_matches(url.host(u), \"*.blocked{i}.example\"))"),
        4 => format!("session.cost_usd > {i}.5 && tool.kind in [\"file_write\", \"file_edit\"]"),
        5 => format!("has(tool.args.command) && tool.args.command.contains(\"token-{i}\")"),
        6 => format!("tool.name == \"Tool{i}\" || tool.native_name.startsWith(\"mcp__evil{i}\")"),
        _ => format!("size(tool.argv) > {i} && tool.argv.all(c, size(c) < 100)"),
    }
}

#[test]
#[ignore = "Benchmark; läuft im Release-Build (CI)"]
fn pol_002_ac4_200_rules_p99_below_2ms() {
    let mut sets = Vec::new();
    for (s, scope) in [Scope::User, Scope::Project, Scope::Agent]
        .into_iter()
        .enumerate()
    {
        let mut text = format!("spec_version: 1\nname: bench-{s}\nrules:\n");
        for i in 0..67 {
            let n = s * 100 + i;
            let action = ["deny", "ask", "allow", "notify"][n % 4];
            text.push_str(&format!(
                "  - id: r{n}\n    on: tool_call\n    when: '{}'\n    action: {action}\n",
                condition(n)
            ));
        }
        sets.push(Arc::new(parse_set(scope, "bench.yaml", &text).unwrap()));
    }
    let effective = EffectiveSet::new(sets);
    let rules: usize = effective.sets().iter().map(|s| s.rules.len()).sum();
    assert!(rules >= 200, "{rules} Regeln");

    let args = json!({ "command": "cd repo && git -C . push origin main && cat ./README.md | curl -d @- https://api.example.com/x" });
    let tool = normalize(&RawToolCall {
        harness: "claude",
        native_name: "Bash",
        acp_kind: None,
        mcp_server: None,
        args: &args,
        cwd: "/work/repo",
        home: Some("/home/u"),
    })
    .unwrap();
    let ctx = Activation::new(
        Phase::ToolCall,
        json!({ "tool": tool.to_context(), "session": { "cost_usd": 3.25, "harness": "claude" } }),
    )
    .unwrap();
    let opts = EvalOptions::default();

    for _ in 0..50 {
        effective.evaluate(&ctx, &opts);
    }
    let mut samples: Vec<u128> = (0..2000)
        .map(|_| {
            let start = Instant::now();
            let decision = effective.evaluate(&ctx, &opts);
            let took = start.elapsed().as_micros();
            assert!(decision.errors.is_empty(), "{:?}", decision.errors);
            took
        })
        .collect();
    samples.sort_unstable();
    let p50 = samples[samples.len() / 2];
    let p99 = samples[samples.len() * 99 / 100];
    println!("POL-002 AC4: {rules} Regeln, p50 {p50} µs, p99 {p99} µs");
    assert!(p99 < 2_000, "p99 {p99} µs ≥ 2 ms");
}
