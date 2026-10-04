//! DATA-002 AC2: Crash-Test. Ein Kindprozess hängt in einer Schleife Events an und wird
//! mitten darin hart beendet (SIGKILL bzw. TerminateProcess). Danach muss das Log lückenlos
//! sein und `head_seq` der höchsten `seq` entsprechen – über 1 000 Iterationen.
//!
//! Der Kindprozess ist dieselbe Test-Binary mit dem ignorierten Test `crash_child_writer`.

#![allow(clippy::unwrap_used)] // Testcode: Panics sind hier die Fehlermeldung.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use beton_core::event::{Actor, Event, EventPayload, Notice, SessionKind, SessionTrigger};
use beton_core::id::{OrgId, SessionId};
use beton_store::{Error, NewSession, Store, StoreOptions};

const DIR_ENV: &str = "BETON_CRASH_DIR";
const SESSION_ENV: &str = "BETON_CRASH_SESSION";

fn notice(session: SessionId, i: u64) -> Event {
    Event::new(
        session,
        0,
        Actor::default(),
        EventPayload::Notice(Notice {
            text: format!("Zeile {i}"),
            ..Notice::default()
        }),
    )
}

/// Kindprozess: schreibt endlos Batches, bis er getötet wird.
#[test]
#[ignore = "wird nur als Kindprozess von data_002_ac2_log_survives_kill gestartet"]
fn crash_child_writer() {
    let (Some(dir), Some(session)) = (std::env::var_os(DIR_ENV), std::env::var_os(SESSION_ENV))
    else {
        return;
    };
    let session: SessionId = session.to_str().unwrap().parse().unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let store = Store::open(Path::new(&dir), StoreOptions::default())
            .await
            .unwrap();
        println!("bereit");
        let mut i = 0;
        loop {
            let head = store.session(OrgId::LOCAL, session).await.unwrap().head_seq;
            let n = fastrand::u64(1..=8);
            let events = (0..n).map(|k| notice(session, i + k)).collect();
            match store.append(OrgId::LOCAL, session, head, 1, events).await {
                Ok(_) | Err(Error::SeqConflict { .. }) => i += n,
                Err(e) => panic!("{e}"),
            }
        }
    });
}

#[tokio::test]
async fn data_002_ac2_log_survives_kill() {
    let iterations: u32 = std::env::var("BETON_CRASH_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000);
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let local = store.ensure_local().await.unwrap();
    let session = store
        .create_session(
            local.org,
            NewSession {
                id: SessionId::new(),
                owner: local.user,
                kind: SessionKind::Main,
                harness: "fake".into(),
                cwd: "/".into(),
                model: None,
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

    let exe = std::env::current_exe().unwrap();
    let mut verified = 1u64;
    for iteration in 0..iterations {
        let mut child = Command::new(&exe)
            .args(["crash_child_writer", "--exact", "--ignored", "--nocapture"])
            .env(DIR_ENV, dir.path())
            .env(SESSION_ENV, session.id.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut lines = BufReader::new(stdout).lines();
        let ready = lines.any(|l| l.is_ok_and(|l| l.trim() == "bereit"));
        assert!(
            ready,
            "Iteration {iteration}: Kindprozess meldet sich nicht"
        );
        tokio::time::sleep(Duration::from_micros(fastrand::u64(0..8_000))).await;
        child.kill().unwrap();
        child.wait().unwrap();

        let head = store.session(local.org, session.id).await.unwrap().head_seq;
        assert!(
            head >= verified,
            "Iteration {iteration}: head_seq ist zurückgegangen"
        );
        let mut after = verified;
        while after < head {
            let events = store
                .events(local.org, session.id, after, 10_000)
                .await
                .unwrap();
            assert!(
                !events.is_empty(),
                "Iteration {iteration}: Lücke nach seq {after}"
            );
            for e in events {
                assert_eq!(e.seq, after + 1, "Iteration {iteration}: Lücke im Log");
                after = e.seq;
            }
        }
        assert_eq!(after, head, "Iteration {iteration}: head_seq ≠ max(seq)");
        let beyond = store.events(local.org, session.id, head, 10).await.unwrap();
        assert!(
            beyond.is_empty(),
            "Iteration {iteration}: Events hinter head_seq"
        );
        verified = head;
    }
    eprintln!("DATA-002 AC2: {iterations} Kills, {verified} Events, Log lückenlos");
    assert!(
        verified > u64::from(iterations),
        "zu wenig geschrieben: {verified}"
    );
}
