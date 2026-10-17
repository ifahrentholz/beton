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
/// Nur im Test: ruft den Beobachter wie der Tunnel (nicht Teil der API, daher ohne OpenAPI).
const OBSERVE: &str = "/test/observe";

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
                max_cost_usd: None,
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
        "tz=Europe/Berln",
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
        |r| r.route(OBSERVE, axum::routing::post(observe)),
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
                t.authed("POST", OBSERVE)
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

/// Ziel des Hinweises „ohne Preis“ (USE-006 AC4): eigene Preise in Prüfreihenfolge mit
/// geschätzten Cache-Raten und der mitgelieferte Katalog (USE-002, USE-003).
#[tokio::test]
async fn use_006_pricing_lists_overrides_in_order_and_the_catalog() {
    let t = app().await;
    let res = get(&t, "/v1/usage/pricing").await;
    assert_eq!(res.status, 200);
    let view: crate::api_usage::PricingView = serde_json::from_slice(&res.body).unwrap();
    assert_eq!(
        view.catalog_version,
        beton_core::pricing::Catalog::embedded().version()
    );
    assert!(view.overrides.is_empty());
    assert!(view.catalog.iter().any(|m| m.model == "claude-opus-5-5"));
    let doc = get(&t, "/v1/openapi.json").await.json();
    assert!(doc["paths"]["/v1/usage/pricing"]["get"].is_object());

    let overrides = vec![
        beton_core::pricing::PriceOverride {
            matcher: beton_core::pricing::OverrideMatch {
                provider: Some("litellm-intern".into()),
                model: Some("llama-3.3-70b*".into()),
            },
            input_per_mtok: Some(0.4),
            output_per_mtok: Some(0.8),
            ..Default::default()
        },
        beton_core::pricing::PriceOverride {
            matcher: beton_core::pricing::OverrideMatch {
                provider: Some("ollama-local".into()),
                model: None,
            },
            free: true,
            ..Default::default()
        },
    ];
    let pricer =
        beton_core::pricing::Pricer::new(beton_core::pricing::Catalog::embedded(), overrides);
    let view = crate::api_usage::PricingView::new(&pricer);
    let first = &view.overrides[0];
    assert_eq!(first.position, 1);
    assert!(first.cache_estimated);
    assert!((first.cache_read_per_mtok - 0.04).abs() < 1e-9);
    assert!((first.cache_write_per_mtok - 0.5).abs() < 1e-9);
    assert!(view.overrides[1].free);
    assert_eq!(view.overrides[1].model, None);
}

// ------------------------------------------------------------- POL-004 AC4 (user.timezone)

fn metered() -> EventPayload {
    // 1,75 USD (1 Mio. Eingabe-Tokens zum Katalogpreis).
    cost("codex", "gpt-5.3-codex", AuthSource::ApiKey, 1_000_000, 0)
}

#[tokio::test]
async fn pol_004_ac4_daily_and_monthly_cost_in_the_users_timezone() {
    use beton_core::timezone::UserTimeZone;
    use time::macros::datetime;

    use crate::usage_policy::usage_values;

    let t = app().await;
    let s = session(&t, "codex").await;
    for ts in [
        "2026-06-30T22:30:00.000Z",
        "2026-07-01T21:30:00.000Z",
        "2026-07-01T22:30:00.000Z",
        "2026-12-01T22:30:00.000Z",
        "2026-12-01T23:30:00.000Z",
    ] {
        append(&t, &s, ts, vec![metered()]).await;
    }
    let berlin = UserTimeZone::parse("Europe/Berlin").unwrap();
    // Sommer (+02:00): der 2. Juli beginnt am 1. Juli um 22:00 UTC, der Juli am 30. Juni.
    let v = usage_values(
        &t.store,
        OrgId::LOCAL,
        s.id,
        &berlin,
        datetime!(2026-07-02 10:00 UTC),
    )
    .await
    .unwrap();
    assert_eq!(v["timezone"], "Europe/Berlin");
    assert_eq!(v["user_daily_cost_usd"], 1.75);
    assert_eq!(v["user_monthly_cost_usd"], 5.25);
    // Winter (+01:00, Versatz zum Auswertungszeitpunkt): 22:30 UTC ist noch der 1. Dezember.
    let v = usage_values(
        &t.store,
        OrgId::LOCAL,
        s.id,
        &berlin,
        datetime!(2026-12-02 10:00 UTC),
    )
    .await
    .unwrap();
    assert_eq!(v["user_daily_cost_usd"], 1.75);
    assert_eq!(v["user_monthly_cost_usd"], 3.5);
    // Default UTC: am 2. Juli um 10:00 UTC zählt nichts vom Vortag.
    let utc = UserTimeZone::utc();
    let v = usage_values(
        &t.store,
        OrgId::LOCAL,
        s.id,
        &utc,
        datetime!(2026-07-02 10:00 UTC),
    )
    .await
    .unwrap();
    assert_eq!(v["timezone"], "UTC");
    assert_eq!(v["user_daily_cost_usd"], 0.0);
    assert_eq!(v["user_monthly_cost_usd"], 3.5);
}

// ------------------------------------------------- POL-011/POL-012 (Budgets, ADR-0034)

async fn child(t: &TestApp, parent: &SessionRecord) -> SessionRecord {
    let local = t.store.ensure_local().await.unwrap();
    t.store
        .create_session(
            OrgId::LOCAL,
            NewSession {
                id: SessionId::new(),
                owner: UserId::LOCAL,
                kind: SessionKind::Subagent,
                harness: "codex".into(),
                cwd: "/tmp".into(),
                model: None,
                effort: None,
                permission_mode: None,
                agent_ref: None,
                project_id: None,
                parent_id: Some(parent.id),
                trigger: SessionTrigger::Spawn,
                home_node: local.node,
                harness_opts: Value::Null,
                max_cost_usd: None,
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn pol_011_ac3_subtree_costs_count_the_parent_and_both_children() {
    use crate::usage_policy::usage_values;
    use time::macros::datetime;

    let t = app().await;
    let parent = session(&t, "codex").await;
    let a = child(&t, &parent).await;
    let b = child(&t, &parent).await;
    let other = session(&t, "codex").await;
    for s in [&parent, &a, &b, &b, &other] {
        append(&t, s, "2026-07-01T10:00:00.000Z", vec![metered()]).await;
    }
    let utc = beton_core::timezone::UserTimeZone::utc();
    let now = datetime!(2026-07-01 12:00 UTC);
    let v = usage_values(&t.store, OrgId::LOCAL, parent.id, &utc, now)
        .await
        .unwrap();
    assert_eq!(v["session_cost_usd"], 1.75);
    // Parent 1,75 + Child A 1,75 + Child B 3,50; die fremde Session zählt nicht.
    assert_eq!(v["subtree_cost_usd"], 7.0);
    assert_eq!(v["subtree_equivalent_usd"], 0.0);
    let v = usage_values(&t.store, OrgId::LOCAL, b.id, &utc, now)
        .await
        .unwrap();
    assert_eq!(v["subtree_cost_usd"], 3.5);
}

#[tokio::test]
async fn pol_011_subscription_usage_counts_its_list_price_equivalent() {
    use crate::usage_policy::usage_values;
    use time::macros::datetime;

    let t = app().await;
    let s = session(&t, "claude").await;
    // 1 Mio. Eingabe-Tokens Sonnet 4.5 zum Listenpreis: 3 USD, aber kein Geld (ADR-0034).
    append(
        &t,
        &s,
        "2026-07-01T10:00:00.000Z",
        vec![cost(
            "claude",
            "claude-sonnet-4-5",
            AuthSource::VendorCli,
            1_000_000,
            0,
        )],
    )
    .await;
    let utc = beton_core::timezone::UserTimeZone::utc();
    let now = datetime!(2026-07-01 12:00 UTC);
    let v = usage_values(&t.store, OrgId::LOCAL, s.id, &utc, now)
        .await
        .unwrap();
    assert_eq!(v["session_cost_usd"], 0.0);
    assert_eq!(v["session_equivalent_usd"], 3.0);
    assert_eq!(v["user_daily_equivalent_usd"], 3.0);
    assert_eq!(v["subtree_equivalent_usd"], 3.0);
    assert_eq!(v["session_without_price"], 0);
    // Ein Modell ohne Katalogpreis: weder Geld noch Äquivalent, die Budgets wissen es nicht.
    append(
        &t,
        &s,
        "2026-07-01T10:05:00.000Z",
        vec![cost(
            "claude",
            "claude-unbekannt-9",
            AuthSource::VendorCli,
            1_000,
            0,
        )],
    )
    .await;
    let v = usage_values(&t.store, OrgId::LOCAL, s.id, &utc, now)
        .await
        .unwrap();
    assert_eq!(v["session_equivalent_usd"], 3.0);
    assert_eq!(v["session_without_price"], 1);
    assert_eq!(v["user_daily_without_price"], 1);
    assert_eq!(v["subtree_without_price"], 1);
    // Welche Modelle ohne Preis genutzt wurden (`on_unpriced`, Security-Review #284).
    for key in [
        "session_unpriced_models",
        "user_daily_unpriced_models",
        "subtree_unpriced_models",
    ] {
        assert_eq!(v[key], json!(["claude-unbekannt-9"]), "{key}: {v}");
    }
}

/// POL-012 AC3: Um 00:00 in der Zeitzone des Users ist das Tagesbudget wieder verfügbar –
/// auch an den Tagen mit 23 bzw. 25 Stunden (Sommerzeit, simulierte Uhr).
#[tokio::test]
async fn pol_012_ac3_the_daily_budget_is_available_again_at_local_midnight() {
    use std::sync::Arc;

    use beton_core::timezone::UserTimeZone;
    use beton_policy::Phase;
    use beton_policy::context::Activation;
    use beton_policy::engine::{EffectiveSet, EvalOptions, Outcome};
    use beton_policy::load::{Scope, parse_set};
    use time::macros::datetime;

    use crate::usage_policy::usage_values;

    let t = app().await;
    let s = session(&t, "codex").await;
    let berlin = UserTimeZone::parse("Europe/Berlin").unwrap();
    // 28.03.2026 23:30 MEZ (Tag vor der Umstellung) und 25.10.2026 23:30 MEZ (25-Stunden-Tag).
    append(&t, &s, "2026-03-28T22:30:00.000Z", vec![metered()]).await;
    append(&t, &s, "2026-10-25T22:30:00.000Z", vec![metered()]).await;
    let set = Arc::new(
        parse_set(
            Scope::User,
            "daily.yaml",
            "spec_version: 1\nname: daily\nrules:\n  - id: daily\n    type: daily_budget\n    params: { limit_usd: 1.5 }\n",
        )
        .unwrap(),
    );
    let effective = EffectiveSet::new(vec![set]);
    let check = |v: &Value| {
        let act = Activation::new(
            Phase::ModelRequest,
            json!({
                "user": { "daily_cost_usd": v["user_daily_cost_usd"],
                          "daily_equivalent_usd": v["user_daily_equivalent_usd"],
                          "daily_unpriced_models": v["user_daily_unpriced_models"],
                          "timezone": v["timezone"] },
                "request": { "model": "gpt-5.3-codex", "granularity": "turn" },
                "state": { "session": {}, "run": {}, "user_day": {}, "user": {} },
            }),
        )
        .unwrap();
        effective.evaluate(&act, &EvalOptions::default()).outcome
    };
    for (before, midnight) in [
        // 29.03.: 00:00 MEZ = 23:00 UTC am Vortag; der Tag hat 23 Stunden.
        (
            datetime!(2026-03-28 22:59:59 UTC),
            datetime!(2026-03-28 23:00 UTC),
        ),
        // 26.10.: 00:00 MEZ = 23:00 UTC; der 25.10. hatte 25 Stunden.
        (
            datetime!(2026-10-25 22:59:59 UTC),
            datetime!(2026-10-25 23:00 UTC),
        ),
    ] {
        let v = usage_values(&t.store, OrgId::LOCAL, s.id, &berlin, before)
            .await
            .unwrap();
        assert_eq!(v["user_daily_cost_usd"], 1.75, "{before}");
        assert!(matches!(check(&v), Outcome::Deny { .. }), "{before}");
        let v = usage_values(&t.store, OrgId::LOCAL, s.id, &berlin, midnight)
            .await
            .unwrap();
        assert_eq!(v["user_daily_cost_usd"], 0.0, "{midnight}");
        assert_eq!(check(&v), Outcome::Allow, "{midnight}");
    }
    // Der 29.03. endet nach 23 Stunden um 22:00 UTC, nicht um 23:00 UTC.
    append(&t, &s, "2026-03-29T21:30:00.000Z", vec![metered()]).await;
    let v = usage_values(
        &t.store,
        OrgId::LOCAL,
        s.id,
        &berlin,
        datetime!(2026-03-29 21:59 UTC),
    )
    .await
    .unwrap();
    assert_eq!(v["user_daily_cost_usd"], 1.75);
    let v = usage_values(
        &t.store,
        OrgId::LOCAL,
        s.id,
        &berlin,
        datetime!(2026-03-29 22:00 UTC),
    )
    .await
    .unwrap();
    assert_eq!(v["user_daily_cost_usd"], 0.0);
}

#[tokio::test]
async fn pol_004_ac4_usage_api_defaults_to_the_users_timezone() {
    let berlin = beton_core::timezone::UserTimeZone::parse("Europe/Berlin").unwrap();
    let t = app_full(
        |r| r,
        move |mut r| {
            r.timezone = berlin;
            r
        },
    )
    .await;
    let s = session(&t, "codex").await;
    // Um das Ende der Sommerzeit (25.10.2026): 22:30 UTC ist davor schon der nächste Tag.
    append(&t, &s, "2026-10-23T22:30:00.000Z", vec![metered()]).await;
    append(&t, &s, "2026-10-26T22:30:00.000Z", vec![metered()]).await;
    let days = |body: &Value| -> Vec<String> {
        body["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["day"].as_str().unwrap().to_owned())
            .collect()
    };
    let res = get(&t, "/v1/usage?group_by=day&from=2026-10-23&to=2026-10-27").await;
    assert_eq!(res.status, 200, "{}", String::from_utf8_lossy(&res.body));
    let body = res.json();
    assert_eq!(body["tz"], "Europe/Berlin");
    assert_eq!(body["from"], "2026-10-22T22:00:00.000Z");
    assert_eq!(body["to"], "2026-10-27T23:00:00.000Z");
    assert_eq!(days(&body), ["2026-10-24", "2026-10-26"]);
    // Ein fester Versatz bleibt möglich und gilt vor der Einstellung.
    let res = get(
        &t,
        "/v1/usage?group_by=day&from=2026-10-23&to=2026-10-27&tz=%2B02:00",
    )
    .await;
    let body = res.json();
    assert_eq!(body["tz"], "+02:00");
    assert_eq!(days(&body), ["2026-10-24", "2026-10-27"]);
    let res = get(&t, "/v1/usage?tz=Mars%2FOlympus").await;
    assert_eq!(res.status, 400);
    assert_problem(&res, "validation_failed");
}

/// #196 (POL-004): Session-Daten, die nur der Server kennt, kommen mit `policy.usage`:
/// Titel, Eltern-Session, Tiefe im Baum, Projekt und ob die Session asynchron läuft.
#[tokio::test]
async fn pol_004_session_title_parent_depth_and_async_for_policies() {
    use beton_core::event::{AgentSpawned, SessionTitleChanged, TitleSource};
    use time::macros::datetime;

    use crate::usage_policy::usage_values;

    let t = app().await;
    let parent = session(&t, "codex").await;
    let waited = child(&t, &parent).await;
    let background = child(&t, &parent).await;
    let grandchild = child(&t, &background).await;
    append(
        &t,
        &parent,
        "2026-07-01T10:00:00.000Z",
        vec![
            EventPayload::SessionTitleChanged(SessionTitleChanged {
                title: "Release vorbereiten".into(),
                source: TitleSource::User,
            }),
            EventPayload::AgentSpawned(AgentSpawned {
                child_session_id: waited.id,
                agent_ref: "builtin:review".into(),
                harness: "codex".into(),
                r#async: false,
            }),
            EventPayload::AgentSpawned(AgentSpawned {
                child_session_id: background.id,
                agent_ref: "builtin:review".into(),
                harness: "codex".into(),
                r#async: true,
            }),
        ],
    )
    .await;
    let utc = beton_core::timezone::UserTimeZone::utc();
    let now = datetime!(2026-07-01 12:00 UTC);
    let meta = |v: &Value| {
        (
            v["session_title"].clone(),
            v["session_parent_id"].clone(),
            v["session_depth"].clone(),
            v["session_is_async"].clone(),
        )
    };
    let v = usage_values(&t.store, OrgId::LOCAL, parent.id, &utc, now)
        .await
        .unwrap();
    assert_eq!(
        meta(&v),
        (
            json!("Release vorbereiten"),
            Value::Null,
            json!(0),
            json!(false)
        )
    );
    assert_eq!(v["session_project"], Value::Null);
    assert_eq!(v["run_trigger"], "interactive");
    let v = usage_values(&t.store, OrgId::LOCAL, waited.id, &utc, now)
        .await
        .unwrap();
    assert_eq!(
        meta(&v),
        (
            Value::Null,
            json!(parent.id.to_string()),
            json!(1),
            json!(false)
        )
    );
    let v = usage_values(&t.store, OrgId::LOCAL, background.id, &utc, now)
        .await
        .unwrap();
    assert_eq!(v["session_is_async"], true, "Spawn mit async: true");
    assert_eq!(v["run_trigger"], "spawn");
    // Ohne `agent.spawned` im Parent ist unbekannt, ob das Child asynchron läuft: `null`
    // (Regeln darauf scheitern, fail closed), nie stillschweigend `false`.
    let v = usage_values(&t.store, OrgId::LOCAL, grandchild.id, &utc, now)
        .await
        .unwrap();
    assert_eq!(v["session_depth"], 2);
    assert_eq!(v["session_parent_id"], json!(background.id.to_string()));
    assert_eq!(v["session_is_async"], Value::Null);

    // Ein Fork ist ein eigener Run mit `trigger = "fork"`.
    let forked = session(&t, "codex").await;
    append(
        &t,
        &forked,
        "2026-07-01T10:00:00.000Z",
        vec![EventPayload::SessionForked(
            beton_core::event::SessionForked {
                from_session: parent.id,
                at_seq: 1,
                reason: beton_core::event::ForkReason::User,
                harness: None,
                from_harness: None,
                history_mode: None,
                fallback_reason: None,
            },
        )],
    )
    .await;
    let v = usage_values(&t.store, OrgId::LOCAL, forked.id, &utc, now)
        .await
        .unwrap();
    assert_eq!(v["run_trigger"], "fork");
}
