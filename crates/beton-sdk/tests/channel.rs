//! Binärkanäle im Rust-SDK gegen einen echten Server (PROTO-007): Keyframe in Fragmenten,
//! laufende Ausgabe, Eingaben mit `INPUT`, Ende des Kanals. Quelle ist ein synthetischer Kanal
//! wie in `beton-server/tests/channels.rs`.

#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use beton_core::event::{SessionKind, SessionTrigger};
use beton_core::id::{OrgId, SessionId, UserId};
use beton_proto::channel::{ChannelAccess, ChannelKind, Frame, flags};
use beton_sdk::Client;
use beton_sdk::channel::{Channel, ChannelEvent};
use beton_server::channels::ChannelBackend;
use beton_server::{ServerConfig, start_with};
use beton_store::{NewSession, Store, StoreOptions};
use serde_json::Value;

#[derive(Default)]
struct Source {
    input: Mutex<Vec<u8>>,
}

impl ChannelBackend for Source {
    fn credit(&self, _: u64) {}
    fn keyframe(&self) {}
    fn input(&self, payload: Vec<u8>) -> bool {
        self.input.lock().unwrap().extend(payload);
        true
    }
}

async fn next(ch: &mut Channel) -> ChannelEvent {
    tokio::time::timeout(Duration::from_secs(10), ch.next())
        .await
        .expect("Zeitüberschreitung")
        .expect("Kanal zu")
}

#[tokio::test]
async fn proto_007_rust_channel_reads_keyframes_and_sends_input() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let mut cfg = ServerConfig::local(dir.path().to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.socket = None;
    let daemon = start_with(cfg, store.clone(), |r| r).await.unwrap();
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
                harness_opts: Value::Null,
                max_cost_usd: None,
            },
        )
        .await
        .unwrap();
    let source = Arc::new(Source::default());
    let relay = daemon.runtime.channels.register(
        session.id,
        ChannelKind::Terminal,
        "term_1",
        source.clone(),
    );
    let token = std::fs::read_to_string(dir.path().join("auth/local.token")).unwrap();
    let client = Client::new(format!("http://{}", daemon.addrs[0]), token.trim()).unwrap();
    let mut ch = Channel::open(&client, session.id, ChannelKind::Terminal, "term_1");
    assert_eq!(
        next(&mut ch).await,
        ChannelEvent::Opened {
            access: ChannelAccess::Drive
        }
    );
    // Keyframe in zwei Fragmenten, danach laufende Ausgabe.
    relay.push(Frame::new(1, flags::KEYFRAME, b"\x1bcTEIL1".to_vec()));
    relay.push(Frame::new(
        1,
        flags::KEYFRAME | flags::FIN,
        b"TEIL2".to_vec(),
    ));
    relay.push(Frame::new(1, flags::FIN, b"$ ".to_vec()));
    let mut data = Vec::new();
    while data.len() < 3 {
        if let ChannelEvent::Data { bytes, keyframe } = next(&mut ch).await {
            data.push((bytes, keyframe));
        }
    }
    assert_eq!(
        data[0],
        (b"\x1bcTEIL1".to_vec(), true),
        "erstes Fragment ersetzt den Zustand"
    );
    assert_eq!(data[1], (b"TEIL2".to_vec(), false));
    assert_eq!(data[2], (b"$ ".to_vec(), false));
    // Eingaben kommen in Reihenfolge bei der Quelle an.
    ch.send(b"ls".to_vec());
    ch.send(b"\r".to_vec());
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while source.input.lock().unwrap().as_slice() != b"ls\r" {
        assert!(std::time::Instant::now() < deadline, "Eingabe fehlt");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    daemon
        .runtime
        .channels
        .close(session.id, ChannelKind::Terminal, "term_1", "closed");
    loop {
        if let ChannelEvent::Closed { reason } = next(&mut ch).await {
            assert_eq!(reason, "closed");
            break;
        }
    }
    // Unbekanntes Terminal: der Kanal endet sofort.
    let mut gone = Channel::open(&client, session.id, ChannelKind::Terminal, "term_x");
    assert!(matches!(next(&mut gone).await, ChannelEvent::Closed { .. }));
}
