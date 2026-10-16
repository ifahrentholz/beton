//! Badge und Benachrichtigungen gegen einen echten Server im Prozess (nur Loopback).

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use beton_core::event::{
    Actor, ApprovalDecision, ApprovalKind, ApprovalRequested, ApprovalResolved, Event,
    EventPayload, MessageCompleted, MessageRole, ResolvedVia, SessionKind, SessionTitleChanged,
    SessionTrigger, TitleSource, TurnCompleted,
};
use beton_core::id::{ApprovalId, OrgId, SessionId, UserId};
use beton_core::notifications::NotificationEvent;
use beton_desktop::notify::Focus;
use beton_desktop::poller::{Poller, Shared, Update};
use beton_sdk::Client;
use beton_server::{ServerConfig, start};
use beton_store::{NewSession, SessionRecord, Store, StoreOptions};
use serde_json::json;
use tokio::sync::mpsc;

struct Env {
    _dir: tempfile::TempDir,
    store: Store,
    client: Client,
    _daemon: beton_server::Daemon,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let mut cfg = ServerConfig::local(dir.path().to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.socket = None;
    cfg.tunnel_socket = None;
    let daemon = start(cfg, store.clone()).await.unwrap();
    let port = daemon.addrs[0].port();
    let token = std::fs::read_to_string(dir.path().join("auth/local.token")).unwrap();
    let client = Client::new(format!("http://127.0.0.1:{port}"), token.trim()).unwrap();
    Env {
        _dir: dir,
        store,
        client,
        _daemon: daemon,
    }
}

async fn session(store: &Store, title: &str) -> SessionRecord {
    let local = store.ensure_local().await.unwrap();
    let s = store
        .create_session(
            OrgId::LOCAL,
            NewSession {
                id: SessionId::new(),
                owner: UserId::LOCAL,
                kind: SessionKind::Main,
                harness: "fake".into(),
                cwd: "/".into(),
                model: None,
                effort: None,
                permission_mode: None,
                agent_ref: None,
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::User,
                home_node: local.node,
                harness_opts: serde_json::Value::Null,
                max_cost_usd: None,
            },
        )
        .await
        .unwrap();
    append(
        store,
        &s,
        vec![EventPayload::SessionTitleChanged(SessionTitleChanged {
            title: title.into(),
            source: TitleSource::User,
        })],
    )
    .await;
    s
}

async fn append(store: &Store, s: &SessionRecord, payloads: Vec<EventPayload>) {
    let head = store.session(OrgId::LOCAL, s.id).await.unwrap().head_seq;
    let events = payloads
        .into_iter()
        .map(|p| Event::new(s.id, 0, Actor::default(), p))
        .collect();
    store
        .append(OrgId::LOCAL, s.id, head, s.epoch, events)
        .await
        .unwrap();
}

fn requested(id: ApprovalId) -> EventPayload {
    EventPayload::ApprovalRequested(ApprovalRequested {
        approval_id: id,
        kind: ApprovalKind::Policy,
        subject: json!({"title": "Shell-Kommando bestätigen", "summary": "git push origin main"}),
        options: vec!["approve".into(), "deny".into()],
        ..ApprovalRequested::default()
    })
}

fn resolved(id: ApprovalId) -> EventPayload {
    EventPayload::ApprovalResolved(ApprovalResolved {
        approval_id: id,
        decision: ApprovalDecision::Allow,
        via: ResolvedVia::User,
        ..ApprovalResolved::default()
    })
}

fn spawn(env: &Env, shared: Arc<Shared>) -> mpsc::Receiver<Update> {
    let (tx, rx) = mpsc::channel(64);
    let mut poller = Poller::new(env.client.clone(), shared);
    poller.period = Duration::from_millis(100);
    poller.settings_every = 1;
    poller.clock = || 12 * 60;
    tokio::spawn(poller.run(tx));
    rx
}

/// Wartet auf das erste Update, das `want` erfüllt, und liefert die Wartezeit.
async fn until(
    rx: &mut mpsc::Receiver<Update>,
    limit: Duration,
    want: impl Fn(&Update) -> bool,
) -> Duration {
    let started = Instant::now();
    loop {
        let left = limit.saturating_sub(started.elapsed());
        let update = tokio::time::timeout(left, rx.recv())
            .await
            .unwrap_or_else(|_| panic!("kein passendes Update binnen {limit:?}"))
            .unwrap();
        if want(&update) {
            return started.elapsed();
        }
    }
}

/// Sammelt alle Updates bis `quiet` lang nichts mehr kommt.
async fn drain(rx: &mut mpsc::Receiver<Update>, quiet: Duration) -> Vec<Update> {
    let mut out = Vec::new();
    while let Ok(Some(u)) = tokio::time::timeout(quiet, rx.recv()).await {
        out.push(u);
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ux_001_ac5_desk_004_ac1_badge_follows_the_inbox_within_one_second() {
    let env = env().await;
    let s = session(&env.store, "Rate-Limiter").await;
    let head = env
        .store
        .session(OrgId::LOCAL, s.id)
        .await
        .unwrap()
        .head_seq;
    env.store
        .mark_read(OrgId::LOCAL, UserId::LOCAL, s.id, head)
        .await
        .unwrap();
    let mut rx = spawn(&env, Arc::new(Shared::default()));
    until(&mut rx, Duration::from_secs(5), |u| *u == Update::Badge(0)).await;

    let a = ApprovalId::new();
    append(&env.store, &s, vec![requested(a)]).await;
    let up = until(&mut rx, Duration::from_secs(2), |u| *u == Update::Badge(1)).await;
    assert!(
        up <= Duration::from_secs(1),
        "neue Freigabe nach {up:?} im Badge"
    );
    let inbox = env.client.inbox(Some("open"), 1).await.unwrap();
    assert_eq!(inbox["open_count"], 1, "Badge = Zähler der Inbox");

    append(&env.store, &s, vec![resolved(a)]).await;
    let down = until(&mut rx, Duration::from_secs(2), |u| *u == Update::Badge(0)).await;
    assert!(
        down <= Duration::from_secs(1),
        "entschieden nach {down:?} aus dem Badge"
    );

    // Mit Kanal „Dock-Badge“ für „Agent ist fertig“ zählen ungelesene Sessions mit (DESK-004).
    let mut settings = env.client.notification_settings().await.unwrap();
    settings.events.turn_completed.badge = true;
    env.client
        .set_notification_settings(&settings)
        .await
        .unwrap();
    until(&mut rx, Duration::from_secs(2), |u| *u == Update::Badge(1)).await;
    let head = env
        .store
        .session(OrgId::LOCAL, s.id)
        .await
        .unwrap()
        .head_seq;
    env.store
        .mark_read(OrgId::LOCAL, UserId::LOCAL, s.id, head)
        .await
        .unwrap();
    let read = until(&mut rx, Duration::from_secs(2), |u| *u == Update::Badge(0)).await;
    assert!(read <= Duration::from_secs(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn desk_005_ac1_turn_end_notifies_once_only_when_the_session_is_not_in_view() {
    let env = env().await;
    let mut settings = env.client.notification_settings().await.unwrap();
    settings.events.turn_completed.desktop = true;
    env.client
        .set_notification_settings(&settings)
        .await
        .unwrap();
    let s = session(&env.store, "Review: Rate-Limiter").await;
    let shared = Arc::new(Shared::default());
    *shared.focus.lock().unwrap() = Focus {
        visible: [s.id.to_string()].into(),
    };
    let mut rx = spawn(&env, shared.clone());
    until(&mut rx, Duration::from_secs(5), |u| {
        matches!(u, Update::Badge(_))
    })
    .await;

    let turn = |text: &str| {
        vec![
            EventPayload::MessageCompleted(MessageCompleted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                content: vec![json!({"type": "text", "text": text})],
                ..MessageCompleted::default()
            }),
            EventPayload::TurnCompleted(TurnCompleted::default()),
        ]
    };
    // Sichtbar im fokussierten Fenster: keine Mitteilung.
    append(&env.store, &s, turn("Zwei Anmerkungen")).await;
    let seen = drain(&mut rx, Duration::from_millis(800)).await;
    assert!(
        !seen.iter().any(|u| matches!(u, Update::Notify(_))),
        "{seen:?}"
    );

    // Fenster unfokussiert: genau eine.
    *shared.focus.lock().unwrap() = Focus::default();
    append(&env.store, &s, turn("Alles grün")).await;
    let seen = drain(&mut rx, Duration::from_millis(800)).await;
    let notes: Vec<_> = seen
        .iter()
        .filter_map(|u| match u {
            Update::Notify(n) => Some(n.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(notes.len(), 1, "{seen:?}");
    assert_eq!(notes[0].event, NotificationEvent::TurnCompleted);
    assert_eq!(notes[0].title, "Review: Rate-Limiter");
    assert_eq!(notes[0].body, "Fertig: Alles grün");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn desk_005_auto_resolved_approval_never_notifies() {
    // #153: Ein Adapter, der selbst entscheidet, schreibt requested und resolved direkt
    // hintereinander; das ist keine Rückfrage an den Menschen.
    let env = env().await;
    let s = session(&env.store, "Plan-Modus").await;
    let mut rx = spawn(&env, Arc::new(Shared::default()));
    until(&mut rx, Duration::from_secs(5), |u| {
        matches!(u, Update::Badge(_))
    })
    .await;
    let a = ApprovalId::new();
    append(&env.store, &s, vec![requested(a), resolved(a)]).await;
    let seen = drain(&mut rx, Duration::from_millis(800)).await;
    assert!(
        !seen.iter().any(|u| matches!(u, Update::Notify(_))),
        "{seen:?}"
    );

    let b = ApprovalId::new();
    append(&env.store, &s, vec![requested(b)]).await;
    let seen = drain(&mut rx, Duration::from_millis(800)).await;
    let n = seen
        .iter()
        .find_map(|u| match u {
            Update::Notify(n) => Some(n.clone()),
            _ => None,
        })
        .expect("offene Freigabe meldet sich");
    assert_eq!(n.event, NotificationEvent::ApprovalRequested);
    assert!(
        n.route.starts_with(&format!("/s/{}?seq=", s.id)),
        "{}",
        n.route
    );
}
