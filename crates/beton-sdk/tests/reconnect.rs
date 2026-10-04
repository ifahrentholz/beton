//! Reconnect des SDK nach einem Server-Neustart (PROTO-009 AC2, PROTO-005).
//!
//! Der Server läuft in-process; nach `shutdown` (Close 4503) startet er auf demselben Port
//! neu. Alle 100 Clients müssen binnen 35 s wieder verbunden sein, und kein 100-ms-Fenster
//! darf mehr als 20 Verbindungsversuche enthalten.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use beton_core::event::{SessionKind, SessionTrigger};
use beton_core::id::{OrgId, SessionId, UserId};
use beton_sdk::Client;
use beton_sdk::ws::Update;
use beton_server::{ServerConfig, start};
use beton_store::{NewSession, Store, StoreOptions};
use tokio::sync::mpsc;

enum Seen {
    Live(usize, Instant),
    Attempt(Instant),
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proto_009_ac2_hundred_clients_reconnect_after_restart_spread_out() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let mut cfg = ServerConfig::local(dir.path().to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.socket = None;
    cfg.tunnel_socket = None;
    let daemon = start(cfg.clone(), store.clone()).await.unwrap();
    let port = daemon.addrs[0].port();
    let local = store.ensure_local().await.unwrap();
    let session = store
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
            },
        )
        .await
        .unwrap();
    let token = std::fs::read_to_string(dir.path().join("auth/local.token")).unwrap();
    let client = Client::new(format!("http://127.0.0.1:{port}"), token.trim()).unwrap();

    const CLIENTS: usize = 100;
    let (tx, mut rx) = mpsc::unbounded_channel();
    for i in 0..CLIENTS {
        let client = client.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut sub = client.subscribe(session.id, 0).await.unwrap();
            loop {
                match sub.next().await {
                    Ok(Update::Live { .. }) => {
                        let _ = tx.send(Seen::Live(i, Instant::now()));
                    }
                    Ok(Update::Reconnecting { delay, .. }) => {
                        let _ = tx.send(Seen::Attempt(Instant::now() + delay));
                    }
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        });
    }
    let mut live = [false; CLIENTS];
    while live.iter().any(|l| !l) {
        if let Some(Seen::Live(i, _)) = rx.recv().await {
            live[i] = true;
        }
    }

    // Geordnetes Herunterfahren (4503), kurz darauf Neustart auf demselben Port.
    let t0 = Instant::now();
    daemon.shutdown().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    cfg.listen = vec![format!("127.0.0.1:{port}").parse().unwrap()];
    let restarted = start(cfg, store.clone()).await.unwrap();

    let mut back = [None; CLIENTS];
    let mut attempts = Vec::new();
    let deadline = t0 + Duration::from_secs(35);
    while back.iter().any(Option::is_none) {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(Seen::Live(i, at))) => back[i] = Some(at),
            Ok(Some(Seen::Attempt(at))) => attempts.push(at),
            Ok(None) | Err(_) => break,
        }
    }
    let missing = back.iter().filter(|b| b.is_none()).count();
    assert_eq!(
        missing, 0,
        "{missing} Clients nach 35 s nicht wieder verbunden"
    );

    let mut windows: BTreeMap<u128, usize> = BTreeMap::new();
    for at in &attempts {
        *windows
            .entry(at.saturating_duration_since(t0).as_millis() / 100)
            .or_default() += 1;
    }
    let busiest = windows.values().copied().max().unwrap_or(0);
    assert!(
        busiest <= 20,
        "{busiest} Verbindungsversuche im selben 100-ms-Fenster: {windows:?}"
    );
    restarted.shutdown().await;
}

fn notice(session: SessionId, i: usize) -> beton_core::event::Event {
    beton_core::event::Event::new(
        session,
        0,
        beton_core::event::Actor::default(),
        beton_core::event::EventPayload::Notice(beton_core::event::Notice {
            text: format!("n{i}"),
            ..beton_core::event::Notice::default()
        }),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proto_005_resume_after_restart_has_no_gaps_or_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let mut cfg = ServerConfig::local(dir.path().to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.socket = None;
    cfg.tunnel_socket = None;
    let daemon = start(cfg.clone(), store.clone()).await.unwrap();
    let port = daemon.addrs[0].port();
    let local = store.ensure_local().await.unwrap();
    let session = store
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
            },
        )
        .await
        .unwrap();
    let token = std::fs::read_to_string(dir.path().join("auth/local.token")).unwrap();
    let client = Client::new(format!("http://127.0.0.1:{port}"), token.trim()).unwrap();
    let append = |from: usize, n: usize| {
        let store = store.clone();
        async move {
            let head = store
                .session(OrgId::LOCAL, session.id)
                .await
                .unwrap()
                .head_seq;
            store
                .append(
                    OrgId::LOCAL,
                    session.id,
                    head,
                    1,
                    (from..from + n).map(|i| notice(session.id, i)).collect(),
                )
                .await
                .unwrap();
        }
    };
    append(0, 50).await;
    let mut sub = client.subscribe(session.id, 0).await.unwrap();
    let mut seqs = Vec::new();
    let head = store
        .session(OrgId::LOCAL, session.id)
        .await
        .unwrap()
        .head_seq;
    collect(&mut sub, &mut seqs, head).await;

    // Neustart; währenddessen kommen weitere Events hinzu.
    daemon.shutdown().await;
    append(50, 50).await;
    cfg.listen = vec![format!("127.0.0.1:{port}").parse().unwrap()];
    let restarted = start(cfg, store.clone()).await.unwrap();
    let head = store
        .session(OrgId::LOCAL, session.id)
        .await
        .unwrap()
        .head_seq;
    tokio::time::timeout(Duration::from_secs(30), collect(&mut sub, &mut seqs, head))
        .await
        .expect("Resume nach Neustart");
    assert_eq!(
        seqs,
        (1..=head).collect::<Vec<_>>(),
        "keine Lücken, keine Duplikate"
    );
    restarted.shutdown().await;
}

async fn collect(sub: &mut beton_sdk::ws::Subscription, seqs: &mut Vec<u64>, until: u64) {
    while seqs.last().copied().unwrap_or(0) < until {
        if let Update::Events(events) = sub.next().await.unwrap() {
            seqs.extend(events.iter().filter(|e| !e.transient).map(|e| e.seq));
        }
    }
}
