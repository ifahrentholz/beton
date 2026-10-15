//! Usage-Projektion (USE-001, USE-002 AC1, DATA-005 AC2).

use std::sync::Arc;

use beton_core::event::{
    Actor, AuthSource, CostDelta, CostSource, Event, EventPayload, PriceSource, SessionKind,
    SessionTrigger,
};
use beton_core::id::{OrgId, SessionId};
use beton_core::pricing::{Catalog, EMBEDDED_CATALOG, OverrideMatch, PriceOverride, Pricer};
use beton_core::time::Timestamp;
use proptest::prelude::*;

use crate::testutil::{TestStore, org, store, store_with};
use crate::{NewSession, SessionRecord, Store, StoreOptions, UsageScope, UsageTotals};

fn new_session(t: &TestStore) -> NewSession {
    NewSession {
        id: SessionId::new(),
        owner: t.local.user,
        kind: SessionKind::Main,
        harness: "fake".into(),
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
    }
}

fn cost(harness: &str, model: &str, auth: AuthSource) -> CostDelta {
    CostDelta {
        harness: harness.into(),
        model: model.into(),
        currency: "USD".into(),
        source: CostSource::Estimated,
        auth_source: auth,
        ..CostDelta::default()
    }
}

fn agent() -> Actor {
    Actor::Agent {
        id: None,
        harness: "fake".into(),
        agent_ref: None,
    }
}

async fn append(store: &Store, s: &SessionRecord, deltas: Vec<CostDelta>) -> Vec<Event> {
    let head = store.session(s.org_id, s.id).await.unwrap().head_seq;
    let events = deltas
        .into_iter()
        .map(|c| Event::new(s.id, 0, agent(), EventPayload::CostDelta(c)))
        .collect();
    store
        .append(s.org_id, s.id, head, s.epoch, events)
        .await
        .unwrap()
}

async fn session(t: &TestStore) -> SessionRecord {
    t.store
        .create_session(org(t), new_session(t))
        .await
        .unwrap()
}

fn catalog_with_input_price(version: &str, gpt5_input: &str) -> Arc<Catalog> {
    let text = EMBEDDED_CATALOG
        .replacen(
            &format!(
                "catalog_version = \"{}\"",
                Catalog::embedded().version()
            ),
            &format!("catalog_version = \"{version}\""),
            1,
        )
        .replace(
            "id = \"gpt-5\"\naliases = [\"gpt-5-2025-08-07\"]\nprovider = \"openai\"\ncontext_window = 400000\ninput_per_mtok = 1.25",
            &format!(
                "id = \"gpt-5\"\naliases = [\"gpt-5-2025-08-07\"]\nprovider = \"openai\"\ncontext_window = 400000\ninput_per_mtok = {gpt5_input}"
            ),
        );
    let catalog = Catalog::parse(&text).unwrap();
    assert_eq!(catalog.version(), version);
    Arc::new(catalog)
}

#[tokio::test]
async fn use_001_ac1_one_record_per_cost_delta_with_tokens_and_cost_source() {
    // Zusätzlich DATA-005 AC2: sichtbar unmittelbar nach dem Append (gleiche Transaktion).
    let t = store().await;
    let s = session(&t).await;
    let mut c = cost("fake", "fake-mini", AuthSource::None);
    c.input_tokens = 1200;
    c.output_tokens = 80;
    let written = append(&t.store, &s, vec![c]).await;
    let records = t.store.usage_records(org(&t), s.id).await.unwrap();
    assert_eq!(records.len(), 1);
    let r = &records[0];
    assert_eq!((r.input_tokens, r.output_tokens), (1200, 80));
    assert_eq!(
        r.cost_source,
        PriceSource::None,
        "fake-mini steht nicht im Katalog"
    );
    assert_eq!(r.cost_micro, None);
    assert_eq!(r.seq, written[0].seq);
    assert_eq!(r.ts, written[0].ts);
    assert_eq!(r.day, written[0].ts.to_string()[..10]);
    assert_eq!(r.user_id, t.local.user);
    assert_eq!(
        (r.harness.as_str(), r.auth_source),
        ("fake", AuthSource::None)
    );
    // Das Event im Log trägt die Bewertung (auch für Clients auf dem Draht).
    let EventPayload::CostDelta(logged) = written[0].payload().unwrap() else {
        panic!("cost.delta erwartet")
    };
    assert_eq!(logged.cost_source, Some(PriceSource::None));
}

#[tokio::test]
async fn use_001_ac1_cost_source_follows_design_order() {
    let pricer = Pricer::new(
        Catalog::embedded(),
        vec![PriceOverride {
            matcher: OverrideMatch {
                provider: Some("litellm-intern".into()),
                model: Some("llama-3.3-70b*".into()),
            },
            input_per_mtok: Some(0.40),
            output_per_mtok: Some(0.80),
            ..PriceOverride::default()
        }],
    );
    let t = store_with(StoreOptions {
        pricer: Arc::new(pricer),
        ..StoreOptions::default()
    })
    .await;
    let s = session(&t).await;
    let mut overridden = cost(
        "direct:litellm-intern",
        "llama-3.3-70b-instruct",
        AuthSource::Gateway,
    );
    overridden.input_tokens = 1_000_000;
    let mut reported = cost("claude", "claude-sonnet-4-5", AuthSource::ApiKey);
    reported.cost_micro = Some(4_200);
    reported.source = CostSource::Reported;
    let mut catalog = cost("codex", "gpt-5-codex", AuthSource::ApiKey);
    catalog.input_tokens = 1_000_000;
    let unknown = cost("codex", "gpt-unbekannt", AuthSource::ApiKey);
    append(&t.store, &s, vec![overridden, reported, catalog, unknown]).await;
    let records = t.store.usage_records(org(&t), s.id).await.unwrap();
    let got: Vec<_> = records
        .iter()
        .map(|r| (r.cost_source, r.cost_micro, r.pricing_version.as_deref()))
        .collect();
    let version = Catalog::embedded().version().to_owned();
    assert_eq!(
        got,
        vec![
            (PriceSource::Override, Some(400_000), None),
            (PriceSource::Harness, Some(4_200), None),
            (
                PriceSource::Catalog,
                Some(1_250_000),
                Some(version.as_str())
            ),
            (PriceSource::None, None, None),
        ]
    );
    let totals = t
        .store
        .usage_totals(org(&t), UsageScope::Session(s.id))
        .await
        .unwrap();
    assert_eq!(totals.cost_micro, 1_654_200);
    assert_eq!(totals.unpriced, 1);
    assert_eq!(
        t.store.session(org(&t), s.id).await.unwrap().cost_micro,
        1_654_200
    );
}

#[tokio::test]
async fn use_001_ac3_vendor_cli_records_never_count_towards_money() {
    let t = store().await;
    let s = session(&t).await;
    let mut sub = cost("claude", "claude-opus-5-5", AuthSource::VendorCli);
    sub.input_tokens = 1_000_000;
    sub.source = CostSource::Subscription;
    // Ein fehlerhaft gemeldeter Betrag darf trotzdem nicht als Ausgabe zählen.
    sub.cost_micro = Some(999_999);
    let mut api = cost("codex", "gpt-5", AuthSource::ApiKey);
    api.output_tokens = 100_000;
    append(&t.store, &s, vec![sub, api]).await;

    let records = t.store.usage_records(org(&t), s.id).await.unwrap();
    assert_eq!(records[0].cost_source, PriceSource::Subscription);
    assert_eq!(records[0].cost_micro, None);
    assert_eq!(
        records[0].equivalent_micro,
        Some(4_000_000),
        "nur informativ"
    );
    let totals = t
        .store
        .usage_totals(org(&t), UsageScope::Session(s.id))
        .await
        .unwrap();
    assert_eq!(totals.cost_micro, 1_000_000, "nur der API-Key-Turn");
    assert_eq!(totals.equivalent_micro, 4_000_000);
    assert_eq!(totals.input_tokens, 1_000_000, "Tokens zählen weiter");
    assert_eq!(
        t.store.session(org(&t), s.id).await.unwrap().cost_micro,
        1_000_000
    );
    let daily = t.store.usage_daily(org(&t), "0000", "9999").await.unwrap();
    assert_eq!(daily.iter().map(|d| d.cost_micro).sum::<i64>(), 1_000_000);

    // Auch ältere Events ohne Bewertung (vor M2 bzw. importiert) zählen nicht.
    let legacy = CostDelta {
        harness: "claude".into(),
        model: "claude-opus-5-5".into(),
        cost_micro: Some(77),
        auth_source: AuthSource::VendorCli,
        ..CostDelta::default()
    };
    sqlx::query("UPDATE events SET payload = ? WHERE org_id = ? AND session_id = ? AND seq = 2")
        .bind(serde_json::to_string(&legacy).unwrap())
        .bind(org(&t).to_string())
        .bind(s.id.to_string())
        .execute(&t.store.pool)
        .await
        .unwrap();
    t.store
        .rebuild_projections(org(&t), Some(s.id))
        .await
        .unwrap();
    let totals = t
        .store
        .usage_totals(org(&t), UsageScope::Session(s.id))
        .await
        .unwrap();
    assert_eq!(totals.cost_micro, 1_000_000);
}

#[tokio::test]
async fn use_002_ac1_record_keeps_pricing_version_after_catalog_update() {
    let dir = tempfile::tempdir().unwrap();
    let old = Pricer::new(catalog_with_input_price("2026-01-01", "1.25"), Vec::new());
    let store = Store::open(
        dir.path(),
        StoreOptions {
            pricer: Arc::new(old),
            ..StoreOptions::default()
        },
    )
    .await
    .unwrap();
    let local = store.ensure_local().await.unwrap();
    let t = TestStore { store, dir, local };
    let s = session(&t).await;
    let mut c = cost("codex", "gpt-5", AuthSource::ApiKey);
    c.input_tokens = 1_000_000;
    append(&t.store, &s, vec![c.clone()]).await;
    let before = t.store.usage_records(org(&t), s.id).await.unwrap();
    assert_eq!(before[0].pricing_version.as_deref(), Some("2026-01-01"));
    assert_eq!(before[0].cost_micro, Some(1_250_000));
    t.store.close().await;

    // Neues Release mit neuem Katalog: alte Records bleiben, auch nach einem Rebuild.
    let new = Pricer::new(catalog_with_input_price("2026-02-01", "2.50"), Vec::new());
    let store = Store::open(
        t.dir.path(),
        StoreOptions {
            pricer: Arc::new(new),
            ..StoreOptions::default()
        },
    )
    .await
    .unwrap();
    store.rebuild_projections(org(&t), None).await.unwrap();
    assert_eq!(store.usage_records(org(&t), s.id).await.unwrap(), before);
    append(&store, &s, vec![c]).await;
    let after = store.usage_records(org(&t), s.id).await.unwrap();
    assert_eq!(after[0], before[0]);
    assert_eq!(after[1].pricing_version.as_deref(), Some("2026-02-01"));
    assert_eq!(after[1].cost_micro, Some(2_500_000));
}

#[tokio::test]
async fn use_001_totals_per_user_and_day_window() {
    let t = store().await;
    let s = session(&t).await;
    let mut c = cost("codex", "gpt-5", AuthSource::ApiKey);
    c.input_tokens = 1_000_000;
    let written = append(&t.store, &s, vec![c]).await;
    let ts = written[0].ts.as_offset_date_time();
    let window = |from: time::OffsetDateTime, to: time::OffsetDateTime| {
        UsageScope::UserBetween(t.local.user, Timestamp::from(from), Timestamp::from(to))
    };
    let hour = time::Duration::hours(1);
    let inside = t
        .store
        .usage_totals(org(&t), window(ts - hour, ts + hour))
        .await
        .unwrap();
    assert_eq!(inside.cost_micro, 1_250_000);
    let outside = t
        .store
        .usage_totals(org(&t), window(ts + hour, ts + hour * 2))
        .await
        .unwrap();
    assert_eq!(outside, UsageTotals::default());
    let org_totals = t
        .store
        .usage_totals(org(&t), UsageScope::Org)
        .await
        .unwrap();
    assert_eq!(org_totals.records, 1);
}

#[tokio::test]
async fn use_001_migration_backfill_matches_rebuild() {
    let t = store().await;
    let s = session(&t).await;
    let mut api = cost("claude", "sonnet", AuthSource::ApiKey);
    api.cost_micro = Some(12);
    api.purpose = Some("title".into());
    let mut sub = cost("claude", "sonnet", AuthSource::VendorCli);
    sub.input_tokens = 5;
    let none = cost("fake", "fake-mini", AuthSource::None);
    append(&t.store, &s, vec![api, sub, none]).await;
    // Stand vor M2: Events ohne Bewertung, keine Records.
    sqlx::query(
        "UPDATE events SET payload = json_remove(payload, '$.cost_source', '$.pricing_version', \
         '$.equivalent_micro') WHERE type = 'cost.delta'",
    )
    .execute(&t.store.pool)
    .await
    .unwrap();
    t.store.rebuild_projections(org(&t), None).await.unwrap();
    let rebuilt = t.store.projection_dump(org(&t)).await.unwrap();
    sqlx::query("DELETE FROM usage_records")
        .execute(&t.store.pool)
        .await
        .unwrap();
    let sql = crate::migrate::MIGRATIONS
        .iter()
        .find(|m| m.name == "usage_records")
        .unwrap()
        .sql;
    let backfill = &sql[sql.find("INSERT INTO usage_records").unwrap()..];
    sqlx::raw_sql(backfill)
        .execute(&t.store.pool)
        .await
        .unwrap();
    assert_eq!(t.store.projection_dump(org(&t)).await.unwrap(), rebuilt);
    let sources: Vec<_> = t
        .store
        .usage_records(org(&t), s.id)
        .await
        .unwrap()
        .iter()
        .map(|r| (r.cost_source, r.cost_micro))
        .collect();
    assert_eq!(
        sources,
        vec![
            (PriceSource::Harness, Some(12)),
            (PriceSource::Subscription, None),
            (PriceSource::None, None),
        ]
    );
}

// ---------------------------------------------------------------------------
// USE-001 AC2: Rebuild ergibt identische Summen (Property-Test)
// ---------------------------------------------------------------------------

fn auth() -> impl Strategy<Value = AuthSource> {
    prop_oneof![
        Just(AuthSource::VendorCli),
        Just(AuthSource::ApiKey),
        Just(AuthSource::Gateway),
        Just(AuthSource::None),
    ]
}

fn delta() -> impl Strategy<Value = CostDelta> {
    (
        prop_oneof![
            Just(("codex", "gpt-5")),
            Just(("claude", "claude-sonnet-4-5-20250929")),
            Just(("direct:gw", "llama-3.3-70b-instruct")),
            Just(("fake", "fake-mini")),
        ],
        auth(),
        (0u64..2_000_000, 0u64..200_000, 0u64..500_000, 0u64..100_000),
        proptest::option::of(0i64..1_000_000),
    )
        .prop_map(|((harness, model), auth, (i, o, cr, cw), reported)| {
            let mut c = cost(harness, model, auth);
            (c.input_tokens, c.output_tokens) = (i, o);
            (c.cache_read_tokens, c.cache_write_tokens) = (cr, cw);
            c.cost_micro = reported;
            c
        })
}

/// Erwartete Summen, unabhängig von der Projektion aus den bewerteten Events gerechnet.
fn expected(events: &[Event]) -> UsageTotals {
    let mut t = UsageTotals::default();
    for e in events {
        let Some(EventPayload::CostDelta(c)) = e.payload() else {
            continue;
        };
        t.records += 1;
        t.input_tokens += c.input_tokens;
        t.output_tokens += c.output_tokens;
        t.cache_read_tokens += c.cache_read_tokens;
        t.cache_write_tokens += c.cache_write_tokens;
        if c.auth_source != AuthSource::VendorCli {
            t.cost_micro += c.cost_micro.unwrap_or(0);
        }
        t.equivalent_micro += c.equivalent_micro.unwrap_or(0);
        t.unpriced += u64::from(c.cost_source == Some(PriceSource::None));
        // Weder Geld noch Listenpreis-Äquivalent (POL-011).
        t.without_price += u64::from(if c.auth_source == AuthSource::VendorCli {
            c.equivalent_micro.is_none()
        } else {
            c.cost_micro.is_none()
        });
    }
    t
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    #[test]
    fn use_001_ac2_rebuild_yields_identical_sums(
        sessions in prop::collection::vec(prop::collection::vec(delta(), 0..8), 1..4),
    ) {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let pricer = Pricer::new(
                Catalog::embedded(),
                vec![PriceOverride {
                    matcher: OverrideMatch { provider: Some("gw".into()), model: None },
                    input_per_mtok: Some(0.4),
                    output_per_mtok: Some(0.8),
                    ..PriceOverride::default()
                }],
            );
            let t = store_with(StoreOptions { pricer: Arc::new(pricer), ..StoreOptions::default() }).await;
            let mut all = Vec::new();
            for deltas in sessions {
                let s = session(&t).await;
                // In mehreren Appends, wie Runner-Batches.
                for chunk in deltas.chunks(3) {
                    all.extend(append(&t.store, &s, chunk.to_vec()).await);
                }
            }
            let org: OrgId = org(&t);
            let before = t.store.usage_totals(org, UsageScope::Org).await.unwrap();
            prop_assert_eq!(&before, &expected(&all));
            let dump = t.store.projection_dump(org).await.unwrap();
            t.store.rebuild_projections(org, None).await.unwrap();
            prop_assert_eq!(t.store.usage_totals(org, UsageScope::Org).await.unwrap(), before);
            prop_assert_eq!(t.store.projection_dump(org).await.unwrap(), dump);
            Ok(())
        })?;
    }
}
