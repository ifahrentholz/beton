//! Agent-Loop, Provider und Sicherheit des Direkt-API-Harness (HAR-010, HAR-011) gegen den
//! lokalen Mock-Server auf Loopback. Kein echter Anbieter, kein echter Key, kein Internet.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use beton_core::event::EventPayload;
use beton_harness::golden::{BLESS_ENV, Normalizer, line_diff};
use beton_harness::process::RealLauncher;
use beton_harness::{
    AdapterContext, Gate, GateDecision, GateRequest, HarnessAdapter, HostEnv, Mode,
    NormalizedEvent, ProbeReport, SessionSpec, Shutdown, UserInput,
};
use beton_harness_direct::config::{KeyRef, ModelConfig, Provider, WireKind};
use beton_harness_direct::http::HttpOptions;
use beton_harness_direct::mock::{self, MockServer, Reply, Say};
use beton_harness_direct::secret::{ApiKey, KeyMap};
use beton_harness_direct::tools::{ToolHost, ToolInfo, ToolOutcome};
use beton_harness_direct::{DirectAdapter, DirectOptions};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// Offensichtlich falscher Test-Key (kein echtes Key-Muster).
const MARKER: &str = "bt-fake-key-MARKER-7f3a9c";
const VAR: &str = "OPENROUTER_API_KEY";

fn provider(name: &str, wire: WireKind, base: &str, key: KeyRef, window: Option<u64>) -> Provider {
    Provider {
        name: name.into(),
        wire,
        base_url: base.parse().unwrap(),
        key,
        prompt_caching: true,
        models: vec![ModelConfig {
            id: "mock-model".into(),
            context_window: window,
            pricing: None,
        }],
    }
}

fn keys() -> DirectOptions {
    let mut map = BTreeMap::new();
    map.insert(VAR.to_owned(), ApiKey::new(MARKER).unwrap());
    DirectOptions {
        keys: Arc::new(KeyMap::new(map)),
        discover_models: false,
    }
}

fn adapter(p: Provider) -> DirectAdapter {
    let mut a = DirectAdapter::new(p, &keys());
    a.http = HttpOptions {
        backoff_base: Duration::from_millis(10),
        ..HttpOptions::default()
    };
    a
}

/// Gate mit fester Entscheidung, zeichnet Anfragen auf.
#[derive(Default)]
struct Recorder {
    deny: bool,
    seen: Mutex<Vec<GateRequest>>,
}

#[async_trait]
impl Gate for Recorder {
    async fn decide(&self, request: GateRequest) -> GateDecision {
        self.seen.lock().unwrap().push(request);
        if self.deny {
            GateDecision::Deny {
                reason: Some("nein".into()),
            }
        } else {
            GateDecision::Allow { updated_args: None }
        }
    }
}

fn ctx(gate: Arc<dyn Gate>) -> AdapterContext {
    AdapterContext {
        gate,
        launcher: Arc::new(RealLauncher),
        env: HostEnv::default(),
    }
}

fn spec(workdir: &Path) -> SessionSpec {
    SessionSpec {
        workdir: workdir.to_path_buf(),
        ..SessionSpec::default()
    }
}

async fn until_end(rx: &mut mpsc::Receiver<NormalizedEvent>) -> Vec<EventPayload> {
    let mut out = Vec::new();
    loop {
        let e = tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .expect("Turn-Ende fehlt")
            .expect("Event-Strom zu Ende");
        let end = matches!(
            e.payload,
            EventPayload::TurnCompleted(_)
                | EventPayload::TurnFailed(_)
                | EventPayload::TurnInterrupted(_)
        );
        out.push(e.payload);
        if end {
            return out;
        }
    }
}

fn names(events: &[EventPayload]) -> Vec<&'static str> {
    events.iter().map(EventPayload::type_name).collect()
}

fn tool(id: &str, name: &str, input: Value) -> Say {
    Say::Tool {
        id: id.into(),
        name: name.into(),
        input,
    }
}

// ---------------------------------------------------------------------------- HAR-010 AC1

/// Golden-Vergleich: Typ und normalisierte Nutzlast je Event; `duration_ms` ist Laufzeit.
fn golden(case: &str, workdir: &Path, events: &[EventPayload]) {
    let mut norm = Normalizer::new(&workdir.canonicalize().unwrap().display().to_string());
    let mut lines = String::new();
    for e in events {
        // Serialisiert als `{"type", "payload"}`; `duration_ms` ist Laufzeit.
        let mut v = serde_json::to_value(e).unwrap();
        if v["payload"].get("duration_ms").is_some() {
            v["payload"]["duration_ms"] = json!(0);
        }
        let v = norm.value(v);
        lines.push_str(&v.to_string());
        lines.push('\n');
    }
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(case)
        .join("expected.events.jsonl");
    if std::env::var_os(BLESS_ENV).is_some() {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, &lines).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&file)
        .unwrap_or_else(|_| panic!("{} fehlt; mit {BLESS_ENV}=1 erzeugen", file.display()));
    assert!(expected == lines, "{}", line_diff(case, &expected, &lines));
}

async fn three_tool_calls(wire: WireKind, case: &str) {
    let server = MockServer::start().await.unwrap();
    let path = mock::messages_path(wire);
    server.push(
        path,
        mock::reply(
            wire,
            "msg_mock000000000001",
            &[
                Say::Text("Ich lege die Datei an.".into(), 6),
                tool(
                    "toolu_mock000000000001",
                    "fs_write",
                    json!({"path": "notiz.txt", "content": "hallo"}),
                ),
                Say::Usage {
                    input: 120,
                    output: 30,
                },
            ],
        ),
    );
    server.push(
        path,
        mock::reply(
            wire,
            "msg_mock000000000002",
            &[
                tool(
                    "toolu_mock000000000002",
                    "fs_read",
                    json!({"path": "notiz.txt"}),
                ),
                Say::Usage {
                    input: 180,
                    output: 20,
                },
            ],
        ),
    );
    server.push(
        path,
        mock::reply(
            wire,
            "msg_mock000000000003",
            &[
                tool(
                    "toolu_mock000000000003",
                    "shell_exec",
                    json!({"command": "cat notiz.txt"}),
                ),
                Say::Usage {
                    input: 220,
                    output: 20,
                },
            ],
        ),
    );
    server.push(
        path,
        mock::reply(
            wire,
            "msg_mock000000000004",
            &[
                Say::Text("Fertig: notiz.txt enthält hallo.".into(), 8),
                Say::Usage {
                    input: 260,
                    output: 15,
                },
            ],
        ),
    );
    let work = tempfile::tempdir().unwrap();
    let a = adapter(provider(
        "mock",
        wire,
        &server.base_url(mock::base_suffix(wire)),
        KeyRef::Env(VAR.into()),
        Some(100_000),
    ));
    let gate = Arc::new(Recorder::default());
    let mut s = a.start(spec(work.path()), ctx(gate.clone())).await.unwrap();
    let mut rx = s.events().unwrap();
    s.send(UserInput::from(
        "Lege notiz.txt mit hallo an und prüfe sie.",
    ))
    .await
    .unwrap();
    let mut events = until_end(&mut rx).await;
    // `harness.ready` kommt vor dem Turn.
    assert!(
        matches!(events[0], EventPayload::HarnessReady(_)),
        "{:?}",
        names(&events)
    );
    events.remove(0);
    golden(case, work.path(), &events);
    // Drei Tool-Calls, jeder durch das Gate, alle ausgeführt.
    let kinds: Vec<String> = gate
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.kind.clone())
        .collect();
    assert_eq!(kinds, ["file_write", "file_read", "shell"]);
    assert_eq!(
        std::fs::read_to_string(work.path().join("notiz.txt")).unwrap(),
        "hallo"
    );
    let reqs = server.requests_to(path);
    assert_eq!(reqs.len(), 4);
    let last = reqs[3].body.to_string();
    assert!(last.contains("exit_code: 0\\nhallo"), "{last}");
    s.shutdown(Shutdown::Kill).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn har_010_ac1_three_tool_calls_anthropic_wire_match_golden() {
    three_tool_calls(WireKind::Anthropic, "anthropic-three-tools").await;
}

#[cfg(unix)]
#[tokio::test]
async fn har_010_ac1_three_tool_calls_openai_wire_match_golden() {
    three_tool_calls(WireKind::Openai, "openai-three-tools").await;
}

// ---------------------------------------------------------------------------- HAR-010 AC2

#[tokio::test]
async fn har_010_ac2_max_turns_ends_the_turn_with_stop_reason() {
    let server = MockServer::start().await.unwrap();
    let wire = WireKind::Anthropic;
    for i in 1..=5 {
        server.push(
            mock::messages_path(wire),
            mock::anthropic(
                &format!("msg_mock00000000000{i}"),
                &[tool(
                    &format!("toolu_mock00000000000{i}"),
                    "fs_glob",
                    json!({"pattern": "*"}),
                )],
            ),
        );
    }
    let work = tempfile::tempdir().unwrap();
    let a = adapter(provider(
        "mock",
        wire,
        &server.base_url(""),
        KeyRef::None,
        None,
    ));
    let mut s = a
        .start(
            SessionSpec {
                max_turns: Some(2),
                ..spec(work.path())
            },
            ctx(Arc::new(Recorder::default())),
        )
        .await
        .unwrap();
    let mut rx = s.events().unwrap();
    s.send(UserInput::from("Suche ewig.")).await.unwrap();
    let events = until_end(&mut rx).await;
    let Some(EventPayload::TurnCompleted(done)) = events.last() else {
        panic!("{:?}", names(&events))
    };
    assert_eq!(done.stop_reason, "max_turns");
    assert_eq!(server.requests_to("/v1/messages").len(), 2);
    // Der zweite Call wird nicht mehr ausgeführt, sondern abgebrochen.
    let cancelled = events.iter().any(|e| matches!(e, EventPayload::ToolCallCompleted(c) if c.status == beton_core::event::ToolStatus::Cancelled));
    assert!(cancelled, "{:?}", names(&events));
}

// ---------------------------------------------------------------------------- HAR-010 AC3

#[tokio::test]
async fn har_010_ac3_retry_after_is_respected() {
    let server = MockServer::start().await.unwrap();
    server.push(
        "/v1/messages",
        Reply::Status {
            status: 429,
            headers: vec![("retry-after".into(), "2".into())],
            body: json!({"error": {"type": "rate_limit_error", "message": "zu schnell"}})
                .to_string(),
        },
    );
    server.push(
        "/v1/messages",
        mock::anthropic("msg_mock000000000001", &[Say::Text("ok".into(), 1)]),
    );
    let work = tempfile::tempdir().unwrap();
    let a = adapter(provider(
        "mock",
        WireKind::Anthropic,
        &server.base_url(""),
        KeyRef::None,
        None,
    ));
    let mut s = a
        .start(spec(work.path()), ctx(Arc::new(Recorder::default())))
        .await
        .unwrap();
    let mut rx = s.events().unwrap();
    let started = Instant::now();
    s.send(UserInput::from("hallo")).await.unwrap();
    let events = until_end(&mut rx).await;
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert!(
        matches!(events.last(), Some(EventPayload::TurnCompleted(_))),
        "{:?}",
        names(&events)
    );
    assert_eq!(server.requests_to("/v1/messages").len(), 2);
}

#[tokio::test]
async fn har_010_ac3_five_failures_fail_the_turn_with_problem() {
    let server = MockServer::start().await.unwrap();
    for _ in 0..6 {
        server.push("/v1/messages", Reply::error(503, "überlastet"));
    }
    let work = tempfile::tempdir().unwrap();
    let a = adapter(provider(
        "mock",
        WireKind::Anthropic,
        &server.base_url(""),
        KeyRef::None,
        None,
    ));
    let mut s = a
        .start(spec(work.path()), ctx(Arc::new(Recorder::default())))
        .await
        .unwrap();
    let mut rx = s.events().unwrap();
    s.send(UserInput::from("hallo")).await.unwrap();
    let events = until_end(&mut rx).await;
    let Some(EventPayload::TurnFailed(f)) = events.last() else {
        panic!("{:?}", names(&events))
    };
    let detail = f.problem["detail"].as_str().unwrap();
    assert!(
        detail.contains("nach 5 Versuchen")
            && detail.contains("503")
            && detail.contains("überlastet"),
        "{detail}"
    );
    assert_eq!(f.problem["code"], "provider_retries_exhausted");
    assert_eq!(server.requests_to("/v1/messages").len(), 5);
}

// ---------------------------------------------------------------------------- HAR-010 AC4

#[tokio::test]
async fn har_010_ac4_compaction_runs_before_the_next_request_above_80_percent() {
    let server = MockServer::start().await.unwrap();
    // 850 von 1000 Tokens belegt → vor dem zweiten Request kompaktieren.
    server.push(
        "/v1/messages",
        mock::anthropic(
            "msg_mock000000000001",
            &[
                tool("toolu_mock000000000001", "fs_glob", json!({"pattern": "*"})),
                Say::Usage {
                    input: 840,
                    output: 10,
                },
            ],
        ),
    );
    server.push(
        "/v1/messages",
        mock::anthropic(
            "msg_mock000000000002",
            &[
                Say::Text("fertig".into(), 3),
                Say::Usage {
                    input: 100,
                    output: 5,
                },
            ],
        ),
    );
    let work = tempfile::tempdir().unwrap();
    let a = adapter(provider(
        "mock",
        WireKind::Anthropic,
        &server.base_url(""),
        KeyRef::None,
        Some(1000),
    ));
    let mut s = a
        .start(spec(work.path()), ctx(Arc::new(Recorder::default())))
        .await
        .unwrap();
    let mut rx = s.events().unwrap();
    s.send(UserInput::from("los")).await.unwrap();
    let events = until_end(&mut rx).await;
    let n = names(&events);
    let pos = |t: &str| {
        n.iter()
            .position(|x| *x == t)
            .unwrap_or_else(|| panic!("{t} fehlt: {n:?}"))
    };
    assert!(
        pos("tool.call.completed") < pos("compaction.started"),
        "{n:?}"
    );
    assert!(pos("compaction.started") < pos("compaction.completed"));
    assert!(pos("compaction.completed") < pos("message.delta"), "{n:?}");
    let done = events.iter().find_map(|e| match e {
        EventPayload::CompactionCompleted(c) => Some(c.clone()),
        _ => None,
    });
    let done = done.unwrap();
    assert_eq!(done.before_tokens, 850);
    assert!(done.after_tokens.unwrap() < 850);
    // SES-011 AC1: genau ein `context.usage` am Ende des Turns (der Runner bündelt).
    assert!(matches!(
        events.last(),
        Some(EventPayload::TurnCompleted(_))
    ));
}

#[tokio::test]
async fn har_022_direct_compact_summarizes_older_turns() {
    let server = MockServer::start().await.unwrap();
    server.push(
        "/v1/messages",
        mock::anthropic(
            "msg_mock000000000001",
            &[
                Say::Text("Antwort eins".into(), 4),
                Say::Usage {
                    input: 50,
                    output: 5,
                },
            ],
        ),
    );
    server.push(
        "/v1/messages",
        mock::anthropic(
            "msg_mock000000000002",
            &[
                Say::Text("Ziel: X. Stand: Y.".into(), 50),
                Say::Usage {
                    input: 60,
                    output: 8,
                },
            ],
        ),
    );
    let work = tempfile::tempdir().unwrap();
    let a = adapter(provider(
        "mock",
        WireKind::Anthropic,
        &server.base_url(""),
        KeyRef::None,
        Some(1000),
    ));
    let mut s = a
        .start(spec(work.path()), ctx(Arc::new(Recorder::default())))
        .await
        .unwrap();
    let mut rx = s.events().unwrap();
    s.send(UserInput::from("erste Frage")).await.unwrap();
    until_end(&mut rx).await;
    s.compact().await.unwrap();
    let mut got = Vec::new();
    while !got
        .iter()
        .any(|e| matches!(e, EventPayload::CompactionCompleted(_)))
    {
        got.push(
            tokio::time::timeout(Duration::from_secs(10), rx.recv())
                .await
                .unwrap()
                .unwrap()
                .payload,
        );
    }
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
        got.push(e.payload);
    }
    let n = names(&got);
    assert_eq!(
        n,
        [
            "compaction.started",
            "compaction.completed",
            "context.usage",
            "cost.delta"
        ],
        "{n:?}"
    );
    let summary = &server.requests_to("/v1/messages")[1].body;
    assert!(
        summary["messages"].to_string().contains("erste Frage"),
        "{summary}"
    );
    assert!(summary["tools"].is_null());
}

// ---------------------------------------------------------------------------- HAR-011

#[tokio::test]
async fn har_011_ac1_openrouter_runs_against_openai_wire_with_tools_and_streaming() {
    let server = MockServer::start().await.unwrap();
    let path = "/api/v1/chat/completions";
    server.push(
        path,
        mock::openai(
            "chatcmpl-1",
            &[
                Say::Text("Ich schaue nach.".into(), 4),
                tool(
                    "call_mock000000000001",
                    "fs_glob",
                    json!({"pattern": "*.txt"}),
                ),
                Say::Usage {
                    input: 40,
                    output: 12,
                },
            ],
        ),
    );
    server.push(
        path,
        mock::openai(
            "chatcmpl-2",
            &[
                Say::Text("Eine Datei: a.txt".into(), 5),
                Say::Usage {
                    input: 60,
                    output: 6,
                },
            ],
        ),
    );
    let work = tempfile::tempdir().unwrap();
    std::fs::write(work.path().join("a.txt"), "x").unwrap();
    let mut p = provider(
        "openrouter",
        WireKind::Openai,
        &server.base_url("/api/v1"),
        KeyRef::Env(VAR.into()),
        Some(262_144),
    );
    p.models[0].id = "qwen/qwen3-coder".into();
    let a = adapter(p);
    assert_eq!(a.id().as_str(), "direct:openrouter");
    let mut s = a
        .start(
            SessionSpec {
                model: Some("qwen/qwen3-coder".into()),
                ..spec(work.path())
            },
            ctx(Arc::new(Recorder::default())),
        )
        .await
        .unwrap();
    let mut rx = s.events().unwrap();
    s.send(UserInput::from("Welche Textdateien gibt es?"))
        .await
        .unwrap();
    let events = until_end(&mut rx).await;
    let deltas = events
        .iter()
        .filter(|e| matches!(e, EventPayload::MessageDelta(_)))
        .count();
    assert!(deltas >= 4, "Streaming: {:?}", names(&events));
    let result = events.iter().find_map(|e| match e {
        EventPayload::ToolCallCompleted(c) => Some(c.result.clone().unwrap()),
        _ => None,
    });
    assert_eq!(result.unwrap(), json!("a.txt"));
    let cost = events
        .iter()
        .find_map(|e| match e {
            EventPayload::CostDelta(c) => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!((cost.input_tokens, cost.output_tokens), (100, 18));
    assert_eq!(cost.harness, "direct:openrouter");
    let reqs = server.requests_to(path);
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].body["model"], "qwen/qwen3-coder");
    assert_eq!(reqs[0].body["stream"], true);
    assert!(
        reqs[0].body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["function"]["name"] == "fs_glob")
    );
    assert_eq!(
        reqs[0].header("authorization"),
        Some(format!("Bearer {MARKER}").as_str())
    );
    assert_eq!(reqs[1].body["messages"][3]["role"], "tool");
}

#[tokio::test]
async fn har_011_ac3_model_picker_falls_back_to_last_known_good_list() {
    let server = MockServer::start().await.unwrap();
    server.push(
        "/v1/models",
        Reply::Json(json!({"data": [{"id": "m-b"}, {"id": "m-a"}]})),
    );
    server.push("/v1/models", Reply::error(503, "weg"));
    let p = provider(
        "gw",
        WireKind::Openai,
        &server.base_url("/v1"),
        KeyRef::None,
        None,
    );
    let a = DirectAdapter::new(
        p,
        &DirectOptions {
            discover_models: true,
            ..DirectOptions::default()
        },
    );
    let t0 = Instant::now();
    a.refresh_models_at(t0).await;
    let caps = a.capabilities(Mode::Native, &ProbeReport::default());
    assert_eq!(caps.models, ["mock-model", "m-a", "m-b"]);
    assert!(!caps.models_stale);
    a.refresh_models_at(t0 + Duration::from_secs(600)).await;
    let caps = a.capabilities(Mode::Native, &ProbeReport::default());
    assert_eq!(
        caps.models,
        ["mock-model", "m-a", "m-b"],
        "zuletzt erfolgreiche Liste"
    );
    assert!(caps.models_stale, "als veraltet gekennzeichnet");
    assert_eq!(server.requests_to("/v1/models").len(), 2);
}

#[tokio::test]
async fn har_011_ac5_missing_key_variable_refuses_the_start_with_a_hint() {
    let p = provider(
        "openrouter",
        WireKind::Openai,
        "https://openrouter.example/api/v1",
        KeyRef::Env("NICHT_GESETZT_KEY".into()),
        None,
    );
    let a = adapter(p);
    let work = tempfile::tempdir().unwrap();
    let err = a
        .start(spec(work.path()), ctx(Arc::new(Recorder::default())))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), "start_refused");
    let text = err.to_string();
    assert!(
        text.contains("NICHT_GESETZT_KEY") && text.contains("providers.openrouter.api_key_env"),
        "{text}"
    );
}

#[tokio::test]
async fn har_011_redirects_are_not_followed() {
    let elsewhere = MockServer::start().await.unwrap();
    let server = MockServer::start().await.unwrap();
    server.push(
        "/v1/messages",
        Reply::Status {
            status: 307,
            headers: vec![(
                "location".into(),
                format!("{}/v1/messages", elsewhere.base_url("")),
            )],
            body: String::new(),
        },
    );
    let work = tempfile::tempdir().unwrap();
    let mut p = provider(
        "mock",
        WireKind::Anthropic,
        &server.base_url(""),
        KeyRef::Env(VAR.into()),
        None,
    );
    p.prompt_caching = false;
    let a = adapter(p);
    let mut s = a
        .start(spec(work.path()), ctx(Arc::new(Recorder::default())))
        .await
        .unwrap();
    let mut rx = s.events().unwrap();
    s.send(UserInput::from("hallo")).await.unwrap();
    let events = until_end(&mut rx).await;
    let Some(EventPayload::TurnFailed(f)) = events.last() else {
        panic!("{:?}", names(&events))
    };
    assert_eq!(f.problem["code"], "provider_redirect");
    assert!(
        elsewhere.requests().is_empty(),
        "keine stille Weiterleitung"
    );
    assert_eq!(server.requests()[0].header("x-api-key"), Some(MARKER));
}

// ------------------------------------------------------------------- Gate und Parallelität

#[tokio::test]
async fn har_010_denied_tool_call_is_not_executed_and_the_model_learns_why() {
    let server = MockServer::start().await.unwrap();
    server.push(
        "/v1/messages",
        mock::anthropic(
            "msg_mock000000000001",
            &[tool(
                "toolu_mock000000000001",
                "fs_write",
                json!({"path": "x.txt", "content": "x"}),
            )],
        ),
    );
    server.push(
        "/v1/messages",
        mock::anthropic("msg_mock000000000002", &[Say::Text("Schade.".into(), 7)]),
    );
    let work = tempfile::tempdir().unwrap();
    let a = adapter(provider(
        "mock",
        WireKind::Anthropic,
        &server.base_url(""),
        KeyRef::None,
        None,
    ));
    let gate = Arc::new(Recorder {
        deny: true,
        ..Recorder::default()
    });
    let mut s = a.start(spec(work.path()), ctx(gate)).await.unwrap();
    let mut rx = s.events().unwrap();
    s.send(UserInput::from("schreib")).await.unwrap();
    let events = until_end(&mut rx).await;
    assert!(!work.path().join("x.txt").exists());
    assert!(!names(&events).contains(&"tool.call.started"));
    let second = &server.requests_to("/v1/messages")[1].body;
    let result = &second["messages"][2]["content"][0];
    assert_eq!(result["is_error"], true);
    assert!(
        result["content"]
            .as_str()
            .unwrap()
            .starts_with("Abgelehnt von beton: nein"),
        "{result}"
    );
}

/// Zählt gleichzeitige Aufrufe.
#[derive(Default)]
struct Slow {
    now: std::sync::atomic::AtomicUsize,
    max: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl ToolHost for Slow {
    fn tools(&self) -> Vec<ToolInfo> {
        vec![ToolInfo {
            model_name: "warte".into(),
            tool: "warte".into(),
            server: "test".into(),
            description: "wartet".into(),
            schema: json!({"type": "object"}),
            kind: "other".into(),
            source: beton_core::event::ToolSource::Harness,
        }]
    }
    async fn call(&self, _name: &str, _args: Value) -> ToolOutcome {
        use std::sync::atomic::Ordering;
        let n = self.now.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(n, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(200)).await;
        self.now.fetch_sub(1, Ordering::SeqCst);
        ToolOutcome {
            ok: true,
            text: "ok".into(),
        }
    }
}

#[tokio::test]
async fn har_010_parallel_tool_calls_run_at_most_four_at_a_time() {
    let server = MockServer::start().await.unwrap();
    let calls: Vec<Say> = (1..=6)
        .map(|i| tool(&format!("toolu_mock00000000000{i}"), "warte", json!({})))
        .collect();
    server.push(
        "/v1/messages",
        mock::anthropic("msg_mock000000000001", &calls),
    );
    server.push(
        "/v1/messages",
        mock::anthropic("msg_mock000000000002", &[Say::Text("ok".into(), 2)]),
    );
    let work = tempfile::tempdir().unwrap();
    let a = adapter(provider(
        "mock",
        WireKind::Anthropic,
        &server.base_url(""),
        KeyRef::None,
        None,
    ));
    let host = Arc::new(Slow::default());
    let mut s = a
        .start_with_tools(
            spec(work.path()),
            ctx(Arc::new(Recorder::default())),
            host.clone(),
            Vec::new(),
            Vec::new(),
        )
        .await
        .unwrap();
    let mut rx = s.events().unwrap();
    let started = Instant::now();
    s.send(UserInput::from("warte sechsmal")).await.unwrap();
    until_end(&mut rx).await;
    assert_eq!(host.max.load(std::sync::atomic::Ordering::SeqCst), 4);
    assert!(
        started.elapsed() < Duration::from_millis(1100),
        "parallel statt nacheinander"
    );
    // Ergebnisse in der Reihenfolge der Calls.
    let second = &server.requests_to("/v1/messages")[1].body;
    let ids: Vec<&str> = second["messages"][2]["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["tool_use_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        (1..=6)
            .map(|i| format!("toolu_mock00000000000{i}"))
            .collect::<Vec<_>>()
    );
}
