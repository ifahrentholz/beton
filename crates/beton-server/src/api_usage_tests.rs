//! HTTP-Tests der Usage-API (USE-005, USE-004, USE-007 AC2) und des Inbox-Hinweises zu
//! Abo-Fenstern (USE-004 AC3), mit echtem Store und der vollständigen App im Prozess.

#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Request, header};
use beton_core::event::{
    Actor, AuthSource, CostDelta, CostSource, Event, EventPayload, SessionKind, SessionTrigger,
    UsageSubscription,
};
use beton_core::id::{OrgId, SessionId, UserId};
use beton_store::{NewSession, SessionRecord, Store, StoreOptions};
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;

use crate::app::{self, AppParts, AppState};
use crate::local_auth::{BrowserLogins, LocalToken};
use crate::tunnel::EventObserver as _;

const HOST: &str = "127.0.0.1:7420";

struct TestApp {
    router: Router,
    token: String,
    store: Store,
    _dir: tempfile::TempDir,
}

struct Res {
    status: u16,
    body: Vec<u8>,
}

impl Res {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

impl TestApp {
    fn authed(&self, method: &str, path: &str) -> axum::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, HOST)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.token))
    }

    async fn send(&self, req: Request<Body>) -> Res {
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status().as_u16();
        let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
        Res { status, body }
    }
}

async fn app_full(
    extra: impl FnOnce(Router<AppState>) -> Router<AppState>,
    customize: impl FnOnce(app::Runtime) -> app::Runtime,
) -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let local = store.ensure_local().await.unwrap();
    let token = LocalToken::load_or_create(dir.path()).unwrap();
    let token_text = std::fs::read_to_string(token.path())
        .unwrap()
        .trim()
        .to_owned();
    let (router, doc) = app::routes();
    let router = app::layered(
        extra(router),
        &doc,
        AppParts {
            store: store.clone(),
            local,
            token: Arc::new(token),
            logins: Arc::new(BrowserLogins::default()),
            hosts: vec![HOST.into()],
            origins: vec![format!("http://{HOST}")],
            primary_host: HOST.into(),
            runtime: customize(
                app::Runtime::new(tokio::sync::watch::channel(false).1).with_default_commands(),
            ),
            web_dir: None,
            secrets: None,
        },
    )
    .layer(MockConnectInfo(
        "127.0.0.1:50000".parse::<SocketAddr>().unwrap(),
    ));
    TestApp {
        router,
        token: token_text,
        store,
        _dir: dir,
    }
}

async fn app() -> TestApp {
    app_full(|r| r, |r| r).await
}

fn assert_problem(res: &Res, code: &str) {
    let p: crate::Problem = serde_json::from_slice(&res.body).unwrap();
    assert_eq!(p.code.as_str(), code, "{p:?}");
}

async fn session(t: &TestApp, harness: &str) -> SessionRecord {
    let local = t.store.ensure_local().await.unwrap();
    t.store
        .create_session(
            OrgId::LOCAL,
            NewSession {
                id: SessionId::new(),
                owner: UserId::LOCAL,
                kind: SessionKind::Main,
                harness: harness.into(),
                cwd: "/tmp".into(),
                model: None,
                effort: None,
                permission_mode: None,
                agent_ref: None,
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::User,
                home_node: local.node,
                harness_opts: Value::Null,
            },
        )
        .await
        .unwrap()
}

fn cost(harness: &str, model: &str, auth: AuthSource, input: u64, output: u64) -> EventPayload {
    EventPayload::CostDelta(CostDelta {
        harness: harness.into(),
        model: model.into(),
        input_tokens: input,
        output_tokens: output,
        currency: "USD".into(),
        source: if auth == AuthSource::VendorCli {
            CostSource::Subscription
        } else {
            CostSource::Estimated
        },
        auth_source: auth,
        ..CostDelta::default()
    })
}

fn event(s: &SessionRecord, ts: &str, payload: EventPayload) -> Event {
    let mut e = Event::new(
        s.id,
        0,
        Actor::Agent {
            id: None,
            harness: "fake".into(),
            agent_ref: None,
        },
        payload,
    );
    e.ts = ts.parse().unwrap();
    e
}

async fn append(t: &TestApp, s: &SessionRecord, ts: &str, payloads: Vec<EventPayload>) {
    let head = t.store.session(s.org_id, s.id).await.unwrap().head_seq;
    let events = payloads.into_iter().map(|p| event(s, ts, p)).collect();
    t.store
        .append(s.org_id, s.id, head, s.epoch, events)
        .await
        .unwrap();
}

async fn get(t: &TestApp, path: &str) -> Res {
    t.send(t.authed("GET", path).body(Body::empty()).unwrap())
        .await
}

#[tokio::test]
async fn use_005_ac1_usage_by_day_and_harness() {
    let t = app().await;
    let claude = session(&t, "claude").await;
    let codex = session(&t, "codex").await;
    append(
        &t,
        &claude,
        "2026-10-01T09:00:00.000Z",
        vec![
            cost(
                "claude",
                "claude-opus-5-5",
                AuthSource::VendorCli,
                1_000_000,
                100,
            ),
            cost("claude", "claude-opus-5-5", AuthSource::VendorCli, 500, 10),
        ],
    )
    .await;
    append(
        &t,
        &codex,
        "2026-10-02T09:00:00.000Z",
        vec![
            cost("codex", "gpt-5.3-codex", AuthSource::ApiKey, 1_000_000, 0),
            cost("codex", "eigenes-modell", AuthSource::ApiKey, 10, 1),
        ],
    )
    .await;
    let res = get(
        &t,
        "/v1/usage?group_by=day,harness&from=2026-10-01&to=2026-10-02",
    )
    .await;
    assert_eq!(res.status, 200, "{}", String::from_utf8_lossy(&res.body));
    let body = res.json();
    assert_eq!(body["group_by"], json!(["day", "harness"]));
    assert_eq!(body["from"], "2026-10-01T00:00:00.000Z");
    assert_eq!(body["to"], "2026-10-03T00:00:00.000Z");
    let items = body["groups"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{body:#}");
    let sub = &items[0];
    assert_eq!(
        (&sub["day"], &sub["harness"], &sub["billing"]),
        (
            &json!("2026-10-01"),
            &json!("claude"),
            &json!("subscription")
        )
    );
    assert_eq!(sub["input_tokens"], 1_000_500);
    assert_eq!(sub["output_tokens"], 110);
    // Subscription: kein Geld, das Äquivalent nur informativ (USE-004 AC1).
    assert_eq!(sub["cost_usd"], Value::Null);
    assert!(sub["equivalent_usd"].as_f64().unwrap() > 0.0, "{sub}");
    assert_eq!(sub["unpriced_count"], 0);
    let api = &items[1];
    assert_eq!(api["billing"], "metered");
    assert_eq!(api["cost_usd"], 1.75);
    assert_eq!(api["equivalent_usd"], Value::Null);
    assert_eq!(api["unpriced_count"], 1);
    // Getrennte Summen, keine Gesamtsumme (CLI-010 AC2).
    assert_eq!(body["totals"]["metered"]["cost_usd"], 1.75);
    assert_eq!(body["totals"]["subscription"]["cost_usd"], Value::Null);
    assert_eq!(body["totals"]["subscription"]["input_tokens"], 1_000_500);
    assert!(body["totals"].get("all").is_none());
}

#[tokio::test]
async fn use_005_invalid_parameters_and_m4_dimensions_are_rejected() {
    let t = app().await;
    for q in [
        "group_by=user",
        "group_by=team",
        "team=t1",
        "group_by=farbe",
        "from=gestern",
        "tz=Europe/Berlin",
    ] {
        let res = get(&t, &format!("/v1/usage?{q}")).await;
        assert_eq!(res.status, 400, "{q}");
        assert_problem(&res, "validation_failed");
    }
    let res = get(&t, "/v1/usage?group_by=user").await;
    assert!(
        res.json()["detail"].as_str().unwrap().contains("ab M4"),
        "{}",
        res.json()
    );
}

#[tokio::test]
async fn use_007_ac2_session_usage_per_turn_with_price_source() {
    let t = app().await;
    let s = session(&t, "codex").await;
    append(
        &t,
        &s,
        "2026-10-02T09:00:00.000Z",
        vec![
            cost("codex", "gpt-5.3-codex", AuthSource::ApiKey, 1_000_000, 0),
            cost("codex", "eigenes-modell", AuthSource::ApiKey, 10, 1),
        ],
    )
    .await;
    let res = get(&t, &format!("/v1/sessions/{}/usage", s.id)).await;
    assert_eq!(res.status, 200);
    let body = res.json();
    let records = body["records"].as_array().unwrap();
    let rows: Vec<_> = records
        .iter()
        .map(|r| {
            (
                r["model"].clone(),
                r["cost_usd"].clone(),
                r["cost_source"].clone(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (json!("gpt-5.3-codex"), json!(1.75), json!("catalog")),
            (json!("eigenes-modell"), Value::Null, json!("none")),
        ]
    );
    assert_eq!(body["totals"]["metered"]["turns"], 2);
    assert_eq!(body["totals"]["metered"]["unpriced_count"], 1);
    let res = get(&t, &format!("/v1/sessions/{}/usage", SessionId::new())).await;
    assert_eq!(res.status, 404);
}

#[tokio::test]
async fn use_004_ac2_and_ac4_windows_and_reporting() {
    let t = app().await;
    let claude = session(&t, "claude").await;
    let _acp = session(&t, "acp:gemini").await;
    let now = beton_core::time::Timestamp::now().to_string();
    let soon = beton_core::time::Timestamp::from(
        time::OffsetDateTime::now_utc() + time::Duration::hours(2),
    );
    append(
        &t,
        &claude,
        &now,
        vec![
            cost("claude", "claude-opus-5-5", AuthSource::VendorCli, 10, 1),
            EventPayload::UsageSubscription(UsageSubscription {
                vendor: "anthropic".into(),
                window: "5h".into(),
                used_pct: 72.0,
                resets_at: soon,
            }),
        ],
    )
    .await;
    let res = get(&t, "/v1/usage?group_by=harness").await;
    let body = res.json();
    let windows = body["subscriptions"].as_array().unwrap();
    assert_eq!(windows.len(), 1, "{body:#}");
    assert_eq!(windows[0]["harness"], "claude");
    assert_eq!(windows[0]["window"], "5h");
    assert_eq!(windows[0]["used_pct"], 72.0);
    assert_eq!(windows[0]["resets_at"], soon.to_string());
    assert_eq!(windows[0]["expired"], false);
    assert_eq!(
        body["reporting"],
        json!([
            {"harness": "acp:gemini", "sessions": 1, "tokens": false, "rate_limits": false},
            {"harness": "claude", "sessions": 1, "tokens": true, "rate_limits": true},
        ])
    );
}

/// Ruft den Beobachter wie der Tunnel nach dem Speichern auf.
async fn observe(State(state): State<AppState>, body: String) -> &'static str {
    let (session, events): (SessionId, Vec<Event>) = serde_json::from_str(&body).unwrap();
    crate::usage_notice::UsageNoticeObserver(state).observe(session, &events);
    "ok"
}

async fn notices(t: &TestApp) -> Vec<Value> {
    let res = get(t, "/v1/inbox?kind=notice").await;
    res.json()["items"].as_array().cloned().unwrap_or_default()
}

async fn rate_limit_case(enabled: bool) -> TestApp {
    let t = app_full(
        |r| r.route("/test/observe", axum::routing::post(observe)),
        move |mut r| {
            r.usage.rate_limit_notice = enabled;
            r
        },
    )
    .await;
    let s = session(&t, "claude").await;
    let resets: beton_core::time::Timestamp = "2026-10-01T12:00:00.000Z".parse().unwrap();
    let window = |used: f64| {
        EventPayload::UsageSubscription(UsageSubscription {
            vendor: "anthropic".into(),
            window: "5h".into(),
            used_pct: used,
            resets_at: resets,
        })
    };
    // Zwei Meldungen über der Schwelle in derselben Periode, eine darunter.
    for used in [89.0, 92.0, 96.0] {
        let e = event(&s, "2026-10-01T10:00:00.000Z", window(used));
        let res = t
            .send(
                t.authed("POST", "/test/observe")
                    .body(Body::from(serde_json::to_string(&(s.id, vec![e])).unwrap()))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status, 200);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while enabled && notices(&t).await.is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Weitere (doppelte) Hinweise kämen innerhalb dieser Zeit.
    tokio::time::sleep(Duration::from_millis(200)).await;
    t
}

#[tokio::test]
async fn use_004_ac3_inbox_notice_from_90_percent_once_per_period() {
    let t = rate_limit_case(true).await;
    let items = notices(&t).await;
    assert_eq!(items.len(), 1, "{items:#?}");
    let item = &items[0];
    assert_eq!(item["status"], "info");
    assert_eq!(item["detail"]["notification"], "rate_limit_high");
    assert_eq!(item["detail"]["window"], "5h");
    assert_eq!(item["detail"]["used_pct"], 92.0);
    assert_eq!(item["detail"]["resets_at"], "2026-10-01T12:00:00.000Z");
    // Hinweise zählen nicht als offene Einträge (UX-001 AC3).
    assert_eq!(get(&t, "/v1/inbox").await.json()["open_count"], 0);
}

#[tokio::test]
async fn use_004_ac3_notice_can_be_switched_off() {
    let t = rate_limit_case(false).await;
    assert!(notices(&t).await.is_empty());
}

#[tokio::test]
async fn use_005_openapi_documents_the_usage_routes() {
    let t = app().await;
    let doc = get(&t, "/v1/openapi.json").await.json();
    assert!(doc["paths"]["/v1/usage"]["get"].is_object());
    assert!(doc["paths"]["/v1/sessions/{id}/usage"]["get"].is_object());
}
