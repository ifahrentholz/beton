//! Tests der Session-Traces (OBS-003).

use beton_core::event::{
    Actor, ApprovalRequested, ApprovalResolved, CostDelta, EventPayload, MessageCompleted,
    MessageRole, PolicyDecision, PolicyOutcome, ToolCallCompleted, ToolCallRequested, ToolStatus,
    TurnCompleted, TurnStarted,
};
use beton_core::id::{ApprovalId, PrincipalId, TurnId, UserId};
use beton_store::StoreOptions;
use serde_json::json;

use super::*;

const MARKER: &str = "MARKER-PROMPT-4711";

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    org: OrgId,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let org = store.ensure_local().await.unwrap().org;
    Fixture {
        _dir: dir,
        store,
        org,
    }
}

fn at(e: &mut Event, ms: i64) {
    e.ts = beton_core::time::Timestamp::from(
        time::OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap()
            + time::Duration::milliseconds(ms),
    );
}

fn ev(session: SessionId, turn: Option<TurnId>, ms: i64, payload: EventPayload) -> Event {
    let mut e = Event::new(session, 0, Actor::default(), payload);
    e.turn_id = turn;
    at(&mut e, ms);
    e
}

/// Ein Turn mit Eingabe, Entscheidungen, Tool-Call samt Freigabe und Kosten.
fn turn_events(session: SessionId, turn: TurnId, base: i64) -> Vec<Event> {
    let approval = ApprovalId::new();
    vec![
        ev(
            session,
            Some(turn),
            base,
            EventPayload::MessageCompleted(MessageCompleted {
                message_id: "m1".into(),
                role: MessageRole::User,
                content: vec![json!({ "type": "text", "text": MARKER })],
                author: Some(PrincipalId::User(UserId::LOCAL)),
            }),
        ),
        ev(
            session,
            Some(turn),
            base + 1,
            EventPayload::TurnStarted(TurnStarted {
                turn_id: turn,
                input_id: None,
                author: PrincipalId::User(UserId::LOCAL),
            }),
        ),
        ev(
            session,
            Some(turn),
            base + 2,
            EventPayload::PolicyDecision(PolicyDecision {
                decision_id: format!("dec_turn_{base}"),
                phase: "turn".into(),
                outcome: PolicyOutcome::Allow,
                eval_us: 300,
                ..PolicyDecision::default()
            }),
        ),
        ev(
            session,
            Some(turn),
            base + 10,
            EventPayload::ToolCallRequested(ToolCallRequested {
                call_id: format!("call_{base}"),
                tool: "Bash".into(),
                kind: Some("shell".into()),
                args: json!({ "command": format!("echo {MARKER}") }),
                ..ToolCallRequested::default()
            }),
        ),
        ev(
            session,
            Some(turn),
            base + 11,
            EventPayload::PolicyDecision(PolicyDecision {
                decision_id: format!("dec_tool_{base}"),
                phase: "tool_call".into(),
                subject_ref: json!({ "call_id": format!("call_{base}") }),
                outcome: PolicyOutcome::Ask,
                matched: vec![json!({ "rule_id": "shell-ask" })],
                approval_id: Some(approval),
                eval_us: 500,
                ..PolicyDecision::default()
            }),
        ),
        ev(
            session,
            Some(turn),
            base + 12,
            EventPayload::ApprovalRequested(ApprovalRequested {
                approval_id: approval,
                subject: json!({ "call_id": format!("call_{base}"), "args": MARKER }),
                ..ApprovalRequested::default()
            }),
        ),
        ev(
            session,
            Some(turn),
            base + 40,
            EventPayload::ApprovalResolved(ApprovalResolved {
                approval_id: approval,
                ..ApprovalResolved::default()
            }),
        ),
        ev(
            session,
            Some(turn),
            base + 50,
            EventPayload::ToolCallCompleted(ToolCallCompleted {
                call_id: format!("call_{base}"),
                status: ToolStatus::Ok,
                result: Some(json!({ "stdout": MARKER })),
                result_ref: None,
                duration_ms: 9,
            }),
        ),
        ev(
            session,
            Some(turn),
            base + 60,
            EventPayload::CostDelta(CostDelta {
                harness: "fake".into(),
                model: "fake-1".into(),
                input_tokens: 100,
                output_tokens: 20,
                cost_micro: Some(1500),
                currency: "USD".into(),
                ..CostDelta::default()
            }),
        ),
        ev(
            session,
            Some(turn),
            base + 70,
            EventPayload::TurnCompleted(TurnCompleted {
                turn_id: turn,
                stop_reason: "end_turn".into(),
                usage_summary: json!({}),
            }),
        ),
    ]
}

async fn spans_of(capture_content: bool, turns: usize) -> (SessionId, Vec<SpanData>) {
    let f = fixture().await;
    let (tx, mut rx) = mpsc::channel(1024);
    let tracer = SessionTracer::with_sink(capture_content, tx);
    let session = SessionId::new();
    for i in 0..turns {
        let events = turn_events(session, TurnId::new(), (i as i64) * 1000);
        tracer.observe(&f.store, f.org, session, &events).await;
    }
    drop(tracer);
    let mut out = Vec::new();
    while let Some(s) = rx.recv().await {
        out.push(s);
    }
    (session, out)
}

fn named<'a>(spans: &'a [SpanData], name: &str) -> Vec<&'a SpanData> {
    spans.iter().filter(|s| s.name == name).collect()
}

fn attr<'a>(s: &'a SpanData, key: &str) -> Option<&'a AttrValue> {
    s.attributes.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
}

#[tokio::test]
async fn obs_003_ac1_spans_follow_the_event_log_in_hierarchy() {
    let (session, spans) = spans_of(false, 1).await;
    let turn = named(&spans, "turn");
    assert_eq!(turn.len(), 1, "{spans:#?}");
    let turn = turn[0];
    assert_eq!(turn.parent, None, "Turn ist Wurzel");
    assert_eq!(
        turn.end_ns - turn.start_ns,
        69_000_000,
        "turn.started bis turn.completed"
    );
    assert_eq!(
        attr(turn, "beton.session.id"),
        Some(&AttrValue::Str(session.to_string()))
    );

    let tool = named(&spans, "tool.call");
    assert_eq!(tool.len(), 1);
    let tool = tool[0];
    assert_eq!(tool.parent, Some(turn.span_id));
    assert_eq!(
        attr(tool, "beton.tool.name"),
        Some(&AttrValue::Str("Bash".into()))
    );
    assert_eq!(
        attr(tool, "beton.tool.status"),
        Some(&AttrValue::Str("ok".into()))
    );

    let policy = named(&spans, "policy.evaluate");
    assert_eq!(policy.len(), 2);
    let on_turn = policy
        .iter()
        .find(|s| attr(s, "beton.policy.phase") == Some(&AttrValue::Str("turn".into())))
        .unwrap();
    assert_eq!(on_turn.parent, Some(turn.span_id));
    let on_tool = policy
        .iter()
        .find(|s| attr(s, "beton.policy.phase") == Some(&AttrValue::Str("tool_call".into())))
        .unwrap();
    assert_eq!(
        on_tool.parent,
        Some(tool.span_id),
        "Entscheidung unter dem Tool-Call"
    );
    assert_eq!(
        attr(on_tool, "beton.policy.decision"),
        Some(&AttrValue::Str("ask".into()))
    );
    assert_eq!(
        attr(on_tool, "beton.policy.rule_ids"),
        Some(&AttrValue::Str("shell-ask".into()))
    );
    assert_eq!(on_tool.end_ns - on_tool.start_ns, 500_000);

    let approval = named(&spans, "approval.wait");
    assert_eq!(approval.len(), 1);
    assert_eq!(approval[0].parent, Some(tool.span_id));
    assert_eq!(approval[0].end_ns - approval[0].start_ns, 28_000_000);

    let model = named(&spans, "model.request");
    assert_eq!(model.len(), 1);
    assert_eq!(model[0].parent, Some(turn.span_id));
    assert_eq!(
        attr(model[0], "gen_ai.request.model"),
        Some(&AttrValue::Str("fake-1".into()))
    );
    assert_eq!(
        attr(model[0], "gen_ai.usage.input_tokens"),
        Some(&AttrValue::Int(100))
    );
    let cost = named(&spans, "cost");
    assert_eq!(cost.len(), 1);
    assert_eq!(cost[0].parent, Some(turn.span_id));
    assert_eq!(
        attr(cost[0], "beton.cost.usd"),
        Some(&AttrValue::Float(0.0015))
    );

    // Alle Spans in einem Trace.
    assert!(spans.iter().all(|s| s.trace_id == trace_id(session)));
}

#[tokio::test]
async fn obs_003_ac2_all_turns_of_a_session_share_the_trace_id() {
    let (session, spans) = spans_of(false, 3).await;
    assert_eq!(named(&spans, "turn").len(), 3);
    let id = trace_id(session);
    assert_ne!(id, [0; 16]);
    assert!(spans.iter().all(|s| s.trace_id == id));
    assert_eq!(trace_id(session), id, "stabil");
    assert_ne!(trace_id(SessionId::new()), id);
    let mut span_ids: Vec<_> = spans.iter().map(|s| s.span_id).collect();
    span_ids.sort_unstable();
    span_ids.dedup();
    assert_eq!(span_ids.len(), spans.len(), "Span-IDs sind eindeutig");
}

#[tokio::test]
async fn obs_003_ac3_without_capture_content_no_prompt_or_args_in_spans() {
    let (_, spans) = spans_of(false, 2).await;
    let text = format!("{spans:?}");
    assert!(!text.contains(MARKER), "{text}");
    // Mit `capture_content` stehen Prompt und Argumente drin.
    let (_, spans) = spans_of(true, 1).await;
    let turn = named(&spans, "turn")[0];
    assert!(matches!(attr(turn, "gen_ai.prompt"), Some(AttrValue::Str(s)) if s.contains(MARKER)));
    let tool = named(&spans, "tool.call")[0];
    assert!(matches!(attr(tool, "beton.tool.args"), Some(AttrValue::Str(s)) if s.contains(MARKER)));
}

#[test]
fn obs_003_endpoint_only_when_configured() {
    let none = |_: &str| None;
    let headers = BTreeMap::new();
    assert_eq!(OtelConfig::resolve(None, &headers, false, &none), Ok(None));
    assert_eq!(
        OtelConfig::resolve(Some(""), &headers, false, &none),
        Ok(None)
    );

    let c = OtelConfig::resolve(Some("http://127.0.0.1:4318"), &headers, true, &none)
        .unwrap()
        .unwrap();
    assert_eq!(c.endpoint, "http://127.0.0.1:4318/v1/traces");
    assert!(c.capture_content);

    let env = |k: &str| match k {
        "OTEL_EXPORTER_OTLP_ENDPOINT" => Some("http://collector:4318/".to_owned()),
        "OTEL_EXPORTER_OTLP_HEADERS" => Some("x-api-key=abc%3D,tenant=t1".to_owned()),
        _ => None,
    };
    let c = OtelConfig::resolve(None, &headers, false, &env)
        .unwrap()
        .unwrap();
    assert_eq!(c.endpoint, "http://collector:4318/v1/traces");
    assert_eq!(
        c.headers,
        vec![
            ("x-api-key".to_owned(), "abc=".to_owned()),
            ("tenant".to_owned(), "t1".to_owned())
        ]
    );
    // Der Traces-Endpoint gilt unverändert; die Konfiguration geht der Umgebung vor.
    let traces = |k: &str| {
        (k == "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").then(|| "http://t:1/custom".to_owned())
    };
    assert_eq!(
        OtelConfig::resolve(None, &headers, false, &traces)
            .unwrap()
            .unwrap()
            .endpoint,
        "http://t:1/custom"
    );
    assert_eq!(
        OtelConfig::resolve(Some("http://cfg:2"), &headers, false, &traces)
            .unwrap()
            .unwrap()
            .endpoint,
        "http://cfg:2/v1/traces"
    );
    // Nur http/json; ein anderes Protokoll ist ein Konfigurationsfehler statt stiller Fehlschläge.
    let grpc = |k: &str| match k {
        "OTEL_EXPORTER_OTLP_ENDPOINT" => Some("http://c:4317".to_owned()),
        "OTEL_EXPORTER_OTLP_PROTOCOL" => Some("grpc".to_owned()),
        _ => None,
    };
    assert!(OtelConfig::resolve(None, &headers, false, &grpc).is_err());
    assert!(OtelConfig::resolve(Some("ftp://x"), &headers, false, &none).is_err());
}

#[test]
fn obs_003_otlp_json_has_the_export_shape() {
    let span = SpanData {
        trace_id: [1; 16],
        span_id: [2; 8],
        parent: Some([3; 8]),
        name: "turn",
        start_ns: 1_000,
        end_ns: 2_000,
        attributes: vec![
            ("a", AttrValue::Str("x".into())),
            ("b", AttrValue::Int(7)),
            ("c", AttrValue::Float(0.5)),
            ("d", AttrValue::Bool(true)),
        ],
        error: true,
    };
    let body = otlp_json(&[span]);
    let rs = &body["resourceSpans"][0];
    assert_eq!(
        rs["resource"]["attributes"][0],
        json!({ "key": "service.name", "value": { "stringValue": "beton" } })
    );
    let s = &rs["scopeSpans"][0]["spans"][0];
    assert_eq!(s["traceId"], "01".repeat(16));
    assert_eq!(s["spanId"], "02".repeat(8));
    assert_eq!(s["parentSpanId"], "03".repeat(8));
    assert_eq!(s["name"], "turn");
    assert_eq!(s["startTimeUnixNano"], "1000");
    assert_eq!(s["endTimeUnixNano"], "2000");
    assert_eq!(s["status"]["code"], 2);
    assert_eq!(
        s["attributes"][1],
        json!({ "key": "b", "value": { "intValue": "7" } })
    );
    assert_eq!(s["attributes"][2]["value"]["doubleValue"], 0.5);
    assert_eq!(s["attributes"][3]["value"]["boolValue"], true);
}
