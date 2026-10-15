//! Usage-Auswertung (USE-004, USE-005, USE-007 AC3).

use beton_core::event::{
    Actor, AuthSource, CostDelta, CostSource, Event, EventPayload, SessionKind, SessionTrigger,
    UsageRateLimitHigh, UsageSubscription,
};
use beton_core::id::SessionId;
use beton_core::time::Timestamp;

use crate::testutil::{TestStore, org, store};
use crate::{
    Billing, NewSession, SessionRecord, Store, StoreOptions, UsageDimension, UsageGroup, UsageQuery,
};

async fn session(t: &TestStore, harness: &str) -> SessionRecord {
    t.store
        .create_session(
            org(t),
            NewSession {
                id: SessionId::new(),
                owner: t.local.user,
                kind: SessionKind::Main,
                harness: harness.into(),
                cwd: "/tmp/projekt".into(),
                model: None,
                effort: None,
                permission_mode: None,
                agent_ref: None,
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::User,
                home_node: t.local.node,
                harness_opts: serde_json::Value::Null,
            },
        )
        .await
        .unwrap()
}

fn cost(harness: &str, model: &str, auth: AuthSource, input: u64, output: u64) -> CostDelta {
    CostDelta {
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
    }
}

fn window(vendor: &str, window: &str, used: f64, resets: &str) -> EventPayload {
    EventPayload::UsageSubscription(UsageSubscription {
        vendor: vendor.into(),
        window: window.into(),
        used_pct: used,
        resets_at: resets.parse().unwrap(),
    })
}

async fn append_at(store: &Store, s: &SessionRecord, ts: &str, payloads: Vec<EventPayload>) {
    let head = store.session(s.org_id, s.id).await.unwrap().head_seq;
    let events = payloads
        .into_iter()
        .map(|p| {
            let mut e = Event::new(
                s.id,
                0,
                Actor::Agent {
                    id: None,
                    harness: "fake".into(),
                    agent_ref: None,
                },
                p,
            );
            e.ts = ts.parse().unwrap();
            e
        })
        .collect();
    store
        .append(s.org_id, s.id, head, s.epoch, events)
        .await
        .unwrap();
}

fn query(from: &str, to: &str, group_by: &[UsageDimension]) -> UsageQuery {
    UsageQuery {
        from: from.parse().unwrap(),
        to: to.parse().unwrap(),
        utc_offset_s: 0,
        offset_changes: Vec::new(),
        group_by: group_by.to_vec(),
        project: None,
        harness: None,
        user: None,
        session: None,
    }
}

fn short(g: &UsageGroup) -> (Option<&str>, Option<&str>, Billing, u64, Option<i64>, u64) {
    (
        g.day.as_deref(),
        g.harness.as_deref(),
        g.billing.unwrap(),
        g.input_tokens,
        g.cost_micro,
        g.unpriced,
    )
}

#[tokio::test]
async fn use_005_ac1_groups_by_day_and_harness_with_money_tokens_and_unpriced() {
    let t = store().await;
    let claude = session(&t, "claude").await;
    let api = session(&t, "codex").await;
    append_at(
        &t.store,
        &claude,
        "2026-10-01T10:00:00.000Z",
        vec![
            EventPayload::CostDelta(cost(
                "claude",
                "claude-opus-5-5",
                AuthSource::VendorCli,
                1_000_000,
                0,
            )),
            EventPayload::CostDelta(cost(
                "claude",
                "claude-opus-5-5",
                AuthSource::VendorCli,
                1_000,
                10,
            )),
        ],
    )
    .await;
    append_at(
        &t.store,
        &api,
        "2026-10-02T09:00:00.000Z",
        vec![
            // Katalogpreis (gpt-5.3-codex: 1,75 $ je 1 Mio. Eingabe).
            EventPayload::CostDelta(cost(
                "codex",
                "gpt-5.3-codex",
                AuthSource::ApiKey,
                1_000_000,
                0,
            )),
            // Ohne Preis.
            EventPayload::CostDelta(cost("codex", "eigenes-modell", AuthSource::ApiKey, 50, 5)),
        ],
    )
    .await;
    let report = t
        .store
        .usage_report(
            org(&t),
            &query(
                "2026-10-01T00:00:00.000Z",
                "2026-10-03T00:00:00.000Z",
                &[UsageDimension::Day, UsageDimension::Harness],
            ),
        )
        .await
        .unwrap();
    let groups: Vec<_> = report.groups.iter().map(short).collect();
    assert_eq!(
        groups,
        [
            (
                Some("2026-10-01"),
                Some("claude"),
                Billing::Subscription,
                1_001_000,
                None,
                0
            ),
            (
                Some("2026-10-02"),
                Some("codex"),
                Billing::Metered,
                1_000_050,
                Some(1_750_000),
                1
            ),
        ]
    );
    let sub = &report.groups[0];
    // Subscription: kein Geld, nur das informative Äquivalent (USE-004 AC1).
    assert_eq!(sub.turns, 2);
    assert_eq!(sub.sessions, 1);
    assert!(sub.equivalent_micro.unwrap() > 0, "{sub:?}");
    assert_eq!(report.groups[1].equivalent_micro, None);
    // Summen je Abrechnungsart, nie zusammengerechnet (CLI-010 AC2).
    let totals: Vec<_> = report
        .totals
        .iter()
        .map(|g| (g.billing.unwrap(), g.input_tokens, g.cost_micro))
        .collect();
    assert_eq!(
        totals,
        [
            (Billing::Metered, 1_000_050, Some(1_750_000)),
            (Billing::Subscription, 1_001_000, None),
        ]
    );
}

#[tokio::test]
async fn use_005_day_follows_the_users_utc_offset_and_filters_apply() {
    let t = store().await;
    let s = session(&t, "claude").await;
    append_at(
        &t.store,
        &s,
        "2026-10-01T23:30:00.000Z",
        vec![EventPayload::CostDelta(cost(
            "claude",
            "m",
            AuthSource::VendorCli,
            7,
            1,
        ))],
    )
    .await;
    let mut q = query(
        "2026-10-01T00:00:00.000Z",
        "2026-10-03T00:00:00.000Z",
        &[UsageDimension::Day],
    );
    let day = |r: &crate::UsageReport| r.groups[0].day.clone().unwrap();
    assert_eq!(
        day(&t.store.usage_report(org(&t), &q).await.unwrap()),
        "2026-10-01"
    );
    q.utc_offset_s = 2 * 3600;
    assert_eq!(
        day(&t.store.usage_report(org(&t), &q).await.unwrap()),
        "2026-10-02"
    );
    q.harness = Some("codex".into());
    assert!(
        t.store
            .usage_report(org(&t), &q)
            .await
            .unwrap()
            .groups
            .is_empty()
    );
    q.harness = None;
    q.session = Some(s.id);
    q.group_by = vec![UsageDimension::Session, UsageDimension::Model];
    let r = t.store.usage_report(org(&t), &q).await.unwrap();
    assert_eq!(r.groups[0].session_id, Some(s.id));
    assert_eq!(r.groups[0].model.as_deref(), Some("m"));
    // Außerhalb des Zeitraums.
    q.from = "2026-10-02T00:00:00.000Z".parse().unwrap();
    assert!(
        t.store
            .usage_report(org(&t), &q)
            .await
            .unwrap()
            .groups
            .is_empty()
    );
}

#[tokio::test]
async fn pol_004_ac4_days_follow_the_offset_changes_of_the_users_timezone() {
    // Europe/Berlin um das Ende der Sommerzeit (25.10.2026, 01:00 UTC): vorher +02:00,
    // danach +01:00. 22:30 UTC ist davor schon der nächste Tag, danach noch derselbe.
    let t = store().await;
    let s = session(&t, "codex").await;
    for ts in ["2026-10-23T22:30:00.000Z", "2026-10-26T22:30:00.000Z"] {
        append_at(
            &t.store,
            &s,
            ts,
            vec![EventPayload::CostDelta(cost(
                "codex",
                "m",
                AuthSource::ApiKey,
                1,
                0,
            ))],
        )
        .await;
    }
    let mut q = query(
        "2026-10-22T22:00:00.000Z",
        "2026-10-27T23:00:00.000Z",
        &[UsageDimension::Day],
    );
    q.utc_offset_s = 7200;
    q.offset_changes = vec![("2026-10-25T01:00:00.000Z".parse().unwrap(), 3600)];
    let days: Vec<String> = t
        .store
        .usage_report(org(&t), &q)
        .await
        .unwrap()
        .groups
        .iter()
        .map(|g| g.day.clone().unwrap())
        .collect();
    assert_eq!(days, ["2026-10-24", "2026-10-26"]);
}

#[tokio::test]
async fn use_004_ac2_latest_window_per_harness_and_vendor_with_unified_names() {
    let t = store().await;
    let a = session(&t, "claude").await;
    let b = session(&t, "claude").await;
    let c = session(&t, "codex").await;
    append_at(
        &t.store,
        &a,
        "2026-10-01T10:00:00.000Z",
        // Vor der Vereinheitlichung gespeichert: `five_hour`.
        vec![window(
            "anthropic",
            "five_hour",
            40.0,
            "2026-10-01T12:00:00.000Z",
        )],
    )
    .await;
    append_at(
        &t.store,
        &b,
        "2026-10-01T11:00:00.000Z",
        vec![
            window("anthropic", "5h", 72.0, "2026-10-01T12:00:00.000Z"),
            window("anthropic", "7d", 13.0, "2026-10-05T00:00:00.000Z"),
        ],
    )
    .await;
    append_at(
        &t.store,
        &c,
        "2026-10-01T09:00:00.000Z",
        vec![window("openai", "300m", 5.0, "2026-10-01T13:00:00.000Z")],
    )
    .await;
    let windows: Vec<_> = t
        .store
        .subscription_windows(org(&t), None)
        .await
        .unwrap()
        .into_iter()
        .map(|w| (w.harness, w.vendor, w.window, w.used_pct, w.session_id))
        .collect();
    assert_eq!(
        windows,
        [
            ("claude".into(), "anthropic".into(), "5h".into(), 72.0, b.id),
            ("claude".into(), "anthropic".into(), "7d".into(), 13.0, b.id),
            ("codex".into(), "openai".into(), "5h".into(), 5.0, c.id),
        ]
    );
    // Projektion: nach dem Rebuild identisch (DATA-005).
    let before = t.store.projection_dump(org(&t)).await.unwrap();
    t.store.rebuild_projections(org(&t), None).await.unwrap();
    assert_eq!(t.store.projection_dump(org(&t)).await.unwrap(), before);
    assert!(
        before.iter().any(|l| l.contains("|anthropic|5h|72")),
        "{before:#?}"
    );
}

#[tokio::test]
async fn use_004_ac4_harness_without_reporting_is_marked() {
    let t = store().await;
    let now = Timestamp::now().to_string();
    let reporting = session(&t, "claude").await;
    let _silent = session(&t, "acp:gemini").await;
    append_at(
        &t.store,
        &reporting,
        &now,
        vec![
            EventPayload::CostDelta(cost("claude", "m", AuthSource::VendorCli, 1, 1)),
            window("anthropic", "5h", 1.0, &now),
        ],
    )
    .await;
    let from = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    let to = time::OffsetDateTime::now_utc() + time::Duration::days(1);
    let mut q = query(&now, &now, &[UsageDimension::Harness]);
    q.from = from.into();
    q.to = to.into();
    let r = t.store.usage_report(org(&t), &q).await.unwrap();
    let rep: Vec<_> = r
        .reporting
        .iter()
        .map(|h| (h.harness.as_str(), h.sessions, h.tokens, h.rate_limits))
        .collect();
    assert_eq!(
        rep,
        [("acp:gemini", 1, false, false), ("claude", 1, true, true)]
    );
}

#[tokio::test]
async fn use_004_ac3_rate_limit_notice_lands_in_the_inbox_once() {
    let t = store().await;
    let s = session(&t, "claude").await;
    let resets: Timestamp = "2026-10-01T12:00:00.000Z".parse().unwrap();
    assert!(
        !t.store
            .rate_limit_notice_exists(org(&t), "anthropic", "5h", resets)
            .await
            .unwrap()
    );
    append_at(
        &t.store,
        &s,
        "2026-10-01T10:00:00.000Z",
        vec![EventPayload::UsageRateLimitHigh(UsageRateLimitHigh {
            vendor: "anthropic".into(),
            window: "5h".into(),
            used_pct: 92.0,
            resets_at: resets,
            threshold_pct: 90.0,
        })],
    )
    .await;
    let items = t
        .store
        .inbox(org(&t), &crate::InboxQuery::default())
        .await
        .unwrap();
    let [item] = items.as_slice() else {
        panic!("{items:?}")
    };
    assert_eq!(
        (item.kind.as_str(), item.status.as_str()),
        ("notice", "info")
    );
    assert_eq!(
        item.title,
        "Subscription-Kontingent fast aufgebraucht: claude, Fenster 5h zu 92 %"
    );
    assert_eq!(item.detail["notification"], "rate_limit_high");
    assert_eq!(item.detail["resets_at"], "2026-10-01T12:00:00.000Z");
    // Hinweise zählen nicht als offen (UX-001 AC3).
    assert_eq!(t.store.inbox_open_count(org(&t)).await.unwrap(), 0);
    assert!(
        t.store
            .rate_limit_notice_exists(org(&t), "anthropic", "5h", resets)
            .await
            .unwrap()
    );
    assert!(
        !t.store
            .rate_limit_notice_exists(org(&t), "anthropic", "7d", resets)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn use_007_ac3_read_only_open_creates_nothing_and_checks_the_schema() {
    let dir = tempfile::tempdir().unwrap();
    assert!(Store::open_read_only(dir.path()).await.unwrap().is_none());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let local = store.ensure_local().await.unwrap();
    store.close().await;
    for side in ["beton.db-wal", "beton.db-shm"] {
        let _ = std::fs::remove_file(dir.path().join(side));
    }
    let names = || {
        let mut v: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    };
    let before = names();
    let ro = Store::open_read_only(dir.path()).await.unwrap().unwrap();
    let r = ro
        .usage_report(
            local.org,
            &query("2026-01-01T00:00:00.000Z", "2027-01-01T00:00:00.000Z", &[]),
        )
        .await
        .unwrap();
    assert!(r.groups.is_empty());
    ro.close().await;
    assert_eq!(names(), before, "keine Dateien angelegt");

    // Älteres Schema: ablehnen statt migrieren.
    let rw = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    sqlx::query("DELETE FROM schema_migrations WHERE version = ?")
        .bind(crate::SCHEMA_VERSION)
        .execute(&rw.pool)
        .await
        .unwrap();
    rw.close().await;
    let err = Store::open_read_only(dir.path()).await.unwrap_err();
    assert_eq!(err.code(), "schema_outdated", "{err}");
}
