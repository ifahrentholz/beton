//! `beton usage` gegen einen echten Daemon und ohne Daemon (USE-004, USE-005, USE-007,
//! CLI-010). Die Vendor-CLIs sind die Fake-CLI mit Subscription-Login (keine `*_API_KEY`).

#![allow(
    clippy::disallowed_methods,
    reason = "Testaufbau startet Prozesse direkt (SBX-001 gilt für Harness und Tools)"
)]
#![allow(clippy::unwrap_used)]
#![cfg(unix)]

mod common;

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use beton_sdk::DaemonInfo;
use beton_server::api_usage::UsageReportView;
use common::{Serve, beton, fake_cli, run, stderr, stdout};
use serde_json::Value;

const CLAUDE: &str = r#"
turns:
  - emit:
      - { rate_limit: { window: 5h, used_pct: 95, resets_in_s: 5400 } }
      - { rate_limit: { window: 7d, used_pct: 40, resets_in_s: 200000 } }
      - { message: "Hallo aus Claude." }
      - { usage: { input_tokens: 1200, output_tokens: 80, cache_read_tokens: 300, cost_usd: 0.42 } }
"#;

const CODEX: &str = r#"
turns:
  - emit:
      - { message: "Hallo aus Codex." }
      - { usage: { input_tokens: 500, output_tokens: 20 } }
      - { rate_limit: { window: 5h, used_pct: 12 } }
"#;

/// Ein Kommando ohne `*_API_KEY` in der Umgebung (ADR-0034), Zeitzone UTC.
fn cli(home: &Path, work: &Path, env: &[(&str, String)]) -> Command {
    let mut c = beton(home);
    c.current_dir(work).env("TZ", "UTC");
    for (k, _) in std::env::vars() {
        if k.ends_with("_API_KEY") {
            c.env_remove(k);
        }
    }
    for (k, v) in env {
        c.env(k, v);
    }
    c
}

fn json(out: &std::process::Output) -> Value {
    assert!(out.status.success(), "{}", stderr(out));
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", stdout(out)))
}

/// Ohne die Felder, die von der Uhrzeit des Aufrufs abhängen.
fn stable(mut v: Value) -> Value {
    for w in v["subscriptions"].as_array_mut().into_iter().flatten() {
        w.as_object_mut().unwrap().remove("expired");
    }
    v
}

/// Ganzer Ablauf mit echtem Daemon und Fake-CLIs, danach ohne Daemon. Deckt USE-007 AC1,
/// USE-007 AC2, USE-007 AC3, CLI-010 AC1, CLI-010 AC2, USE-004 AC1, USE-004 AC2 und
/// USE-004 AC3 ab.
#[tokio::test]
async fn use_007_ac3_usage_with_and_without_daemon() {
    let vendor = tempfile::tempdir().unwrap();
    let claude_s = vendor.path().join("claude.yaml");
    let codex_s = vendor.path().join("codex.yaml");
    std::fs::write(&claude_s, CLAUDE).unwrap();
    std::fs::write(&codex_s, CODEX).unwrap();
    // Ein Link namens `codex`, damit der Versions-Probe die Codex-Version liest.
    let link = vendor.path().join("bin/codex");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(fake_cli(), &link).unwrap();
    let env: Vec<(&str, String)> = vec![
        (
            "BETON_CLAUDE_PATH",
            format!(
                "{} --protocol stream-json --scenario {}",
                fake_cli().display(),
                claude_s.display()
            ),
        ),
        (
            "BETON_CODEX_PATH",
            format!(
                "{} --protocol app-server --scenario {}",
                link.display(),
                codex_s.display()
            ),
        ),
    ];
    let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mut serve = Serve::start_without_api_keys(&pairs);
    let home = serve.home().to_path_buf();
    let work = serve.work.path().to_path_buf();

    for harness in ["claude", "codex"] {
        let out =
            run(cli(&home, &work, &env).args(["run", "--unsandboxed", harness, "-p", "Hallo"]));
        assert!(out.status.success(), "{harness}: {}", stderr(&out));
    }
    let client = serve.client();
    let sessions = client.all_sessions(false).await.unwrap();
    let claude_id = sessions.iter().find(|s| s["harness"] == "claude").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // USE-004 AC3: ab 90 % ein Hinweis in der Inbox, einmal.
    let deadline = Instant::now() + Duration::from_secs(20);
    let notices = loop {
        let store = beton_store::Store::open_read_only(&home)
            .await
            .unwrap()
            .unwrap();
        let items = store
            .inbox(
                beton_core::id::OrgId::LOCAL,
                &beton_store::InboxQuery {
                    kind: Some("notice".into()),
                    ..beton_store::InboxQuery::default()
                },
            )
            .await
            .unwrap();
        store.close().await;
        if !items.is_empty() || Instant::now() > deadline {
            break items;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(notices.len(), 1, "{notices:#?}");
    assert_eq!(notices[0].detail["notification"], "rate_limit_high");
    assert_eq!(notices[0].detail["window"], "5h");
    assert_eq!(notices[0].detail["used_pct"], 95.0);

    // USE-007 AC1 / CLI-010 AC1: JSON nach Modell; dieselben Summen wie GET /v1/usage.
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let by_model = [
        "usage", "--by", "model", "--from", &today, "--to", &today, "--json",
    ];
    let with_daemon = json(&run(cli(&home, &work, &env).args(by_model)));
    let api = client
        .usage(&[
            ("from", today.clone()),
            ("to", today.clone()),
            ("tz", "+00:00".into()),
            ("group_by", "model".into()),
        ])
        .await
        .unwrap();
    assert_eq!(stable(with_daemon.clone()), stable(api));
    let items = with_daemon["groups"].as_array().unwrap();
    let claude = items
        .iter()
        .find(|i| i["model"] == "fake-model" || i["billing"] == "subscription")
        .unwrap();
    // USE-004 AC1, CLI-010 AC2: Tokens statt Geld, nichts zusammengerechnet.
    assert_eq!(claude["billing"], "subscription");
    assert_eq!(claude["cost_usd"], Value::Null);
    // Dazu kommen die Einmal-Aufrufe für Session-Titel (`purpose: title`, SES-010).
    let input = with_daemon["totals"]["subscription"]["input_tokens"]
        .as_u64()
        .unwrap();
    assert!(input >= 1700, "{input}");
    assert_eq!(
        with_daemon["totals"]["subscription"]["cost_usd"],
        Value::Null
    );
    assert_eq!(with_daemon["totals"]["metered"]["turns"], 0);
    // USE-004 AC2: Abo-Fenster beider CLIs.
    let mut windows: Vec<(String, String, f64)> = with_daemon["subscriptions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            (
                w["harness"].as_str().unwrap().to_owned(),
                w["window"].as_str().unwrap().to_owned(),
                w["used_pct"].as_f64().unwrap(),
            )
        })
        .collect();
    windows.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        windows,
        [
            ("claude".into(), "5h".into(), 95.0),
            ("claude".into(), "7d".into(), 40.0),
            ("codex".into(), "5h".into(), 12.0),
        ]
    );
    // USE-007 AC1: Die Ausgabe ist genau der Typ hinter dem veröffentlichten Schema
    // (verlustfreier Round-Trip), und das Schema ist aktuell.
    let typed: UsageReportView = serde_json::from_value(with_daemon.clone()).unwrap();
    assert_eq!(serde_json::to_value(&typed).unwrap(), with_daemon);
    let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1/usage.schema.json");
    let published: Value = serde_json::from_str(&std::fs::read_to_string(schema).unwrap()).unwrap();
    let current = serde_json::to_value(schemars::schema_for!(UsageReportView)).unwrap();
    assert_eq!(published, current, "cargo xtask codegen ausführen");

    // Tabelle: Subscriptions mit Balken und Reset-Uhrzeit, ohne Geldbetrag.
    let table = run(cli(&home, &work, &env).args(["usage", "--by", "harness", "--since", "1d"]));
    assert!(table.status.success(), "{}", stderr(&table));
    let text = stdout(&table);
    assert!(
        text.contains("Subscriptions · Tokens, kein Geldbetrag"),
        "{text}"
    );
    assert!(!text.contains("Nach Verbrauch"), "{text}");
    assert!(!text.contains('$'), "kein Geld bei Subscriptions: {text}");
    assert!(text.contains("██████████  95 %  Reset "), "{text}");
    assert!(text.contains("█░░░░░░░░░  12 %"), "{text}");
    assert!(
        stderr(&table).contains("nur informativ"),
        "Hinweis auf stderr: {}",
        stderr(&table)
    );

    // USE-007 AC2: je Turn mit Preisquelle.
    let turns = run(cli(&home, &work, &env).args(["usage", "--session", &claude_id]));
    assert!(turns.status.success(), "{}", stderr(&turns));
    let text = stdout(&turns);
    assert!(text.contains("PREISQUELLE"), "{text}");
    assert!(text.contains("subscription"), "{text}");
    assert!(text.contains("1.200"), "{text}");
    let session_json = json(&run(cli(&home, &work, &env).args([
        "usage",
        "--session",
        &claude_id,
        "--json",
    ])));
    assert_eq!(session_json["records"][0]["cost_source"], "subscription");
    assert_eq!(session_json["records"][0]["cost_usd"], Value::Null);

    // CSV: eine Kopfzeile, eine Zeile je Gruppe.
    let csv =
        run(cli(&home, &work, &env).args(["usage", "--by", "harness", "--since", "1d", "--csv"]));
    let csv = stdout(&csv);
    let mut lines = csv.lines();
    assert!(
        lines.next().unwrap().starts_with("harness,billing,turns"),
        "{csv}"
    );
    assert_eq!(lines.count(), 2, "{csv}");

    // USE-007 AC3: ohne Daemon dieselben Zahlen, direkt aus der Datenbank; kein Start.
    serve.terminate();
    let offline = run(cli(&home, &work, &env).args(by_model));
    assert!(offline.status.success(), "{}", stderr(&offline));
    assert!(
        stderr(&offline).contains("Kein Daemon aktiv"),
        "{}",
        stderr(&offline)
    );
    assert_eq!(stable(json(&offline)), stable(with_daemon));
    assert!(DaemonInfo::read(&home).is_none(), "kein Daemon gestartet");
    let offline_turns = json(&run(cli(&home, &work, &env).args([
        "usage",
        "--session",
        &claude_id,
        "--json",
    ])));
    assert_eq!(offline_turns, session_json);
}

#[test]
fn use_007_invalid_flags_are_usage_errors() {
    let home = tempfile::tempdir().unwrap();
    for args in [
        vec!["usage", "--by", "farbe"],
        vec!["usage", "--by", "user"],
        vec!["usage", "--since", "gestern"],
        vec!["usage", "--json", "--csv"],
        vec!["usage", "--since", "1d", "--from", "2026-10-01"],
    ] {
        let out = run(beton(home.path()).args(&args));
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
        assert!(stdout(&out).is_empty());
    }
    // Ohne Datenbank: leere Übersicht, nichts angelegt.
    let out = run(beton(home.path()).args(["usage"]));
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("Kein Verbrauch im Zeitraum."));
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
}
