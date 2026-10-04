//! HAR-011 AC5 (Sicherheit): Der Wert eines API-Keys erscheint weder in Events noch in
//! Logs – auch nicht, wenn der Anbieter ihn in Fehlermeldungen zurückspiegelt. Eigenes
//! Test-Binary mit globalem Log-Subscriber (TRACE), damit kein paralleler Test die
//! Erfassung stört.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::EventPayload;
use beton_harness::process::RealLauncher;
use beton_harness::{
    AdapterContext, Gate, GateDecision, GateRequest, HarnessAdapter, HostEnv, NormalizedEvent,
    SessionSpec, Shutdown, UserInput,
};
use beton_harness_direct::config::{KeyRef, ModelConfig, Provider, WireKind};
use beton_harness_direct::http::HttpOptions;
use beton_harness_direct::mock::{self, MockServer, Reply, Say};
use beton_harness_direct::secret::{ApiKey, KeyMap};
use beton_harness_direct::{DirectAdapter, DirectOptions};
use serde_json::json;
use tokio::sync::mpsc;

/// Offensichtlich falscher Test-Key (kein echtes Key-Muster).
const MARKER: &str = "bt-fake-key-MARKER-7f3a9c";
const VAR: &str = "OPENROUTER_API_KEY";

struct Allow;

#[async_trait]
impl Gate for Allow {
    async fn decide(&self, _request: GateRequest) -> GateDecision {
        GateDecision::Allow { updated_args: None }
    }
}

async fn until_end(rx: &mut mpsc::Receiver<NormalizedEvent>) -> Vec<EventPayload> {
    let mut out = Vec::new();
    loop {
        let e = tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .unwrap()
            .unwrap();
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

/// Sammelt alle Log-Ausgaben (TRACE) in einen Puffer.
#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn har_011_ac5_key_value_never_reaches_events_or_logs() {
    let logs = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(logs.clone())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();
    tracing::info!("Test: Logs werden erfasst");
    let server = MockServer::start().await.unwrap();
    let path = "/v1/chat/completions";
    // 1) Fehler, dessen Text den Key zurückspiegelt.
    server.push(
        path,
        Reply::error(401, &format!("Incorrect API key provided: {MARKER}")),
    );
    // 2) 5xx mit Key im Body, danach Erfolg mit Tool-Call.
    server.push(
        path,
        Reply::error(500, &format!("upstream failed for key {MARKER}")),
    );
    server.push(
        path,
        mock::openai(
            "chatcmpl-1",
            &[Say::Tool {
                id: "call_mock000000000001".into(),
                name: "fs_glob".into(),
                input: json!({"pattern": "*"}),
            }],
        ),
    );
    server.push(
        path,
        mock::openai("chatcmpl-2", &[Say::Text("ok".into(), 1)]),
    );
    let work = tempfile::tempdir().unwrap();
    let mut keys = BTreeMap::new();
    keys.insert(VAR.to_owned(), ApiKey::new(MARKER).unwrap());
    let provider = Provider {
        name: "openrouter".into(),
        wire: WireKind::Openai,
        base_url: server.base_url("/v1").parse().unwrap(),
        key: KeyRef::Env(VAR.into()),
        prompt_caching: false,
        models: vec![ModelConfig {
            id: "mock-model".into(),
            context_window: None,
            pricing: None,
        }],
    };
    let mut a = DirectAdapter::new(
        provider,
        &DirectOptions {
            keys: Arc::new(KeyMap::new(keys)),
            discover_models: false,
        },
    );
    a.http = HttpOptions {
        backoff_base: Duration::from_millis(10),
        ..HttpOptions::default()
    };
    let ctx = AdapterContext {
        gate: Arc::new(Allow),
        launcher: Arc::new(RealLauncher),
        env: HostEnv::default(),
    };
    let spec = SessionSpec {
        workdir: work.path().to_path_buf(),
        ..SessionSpec::default()
    };
    let mut s = a.start(spec, ctx).await.unwrap();
    let mut rx = s.events().unwrap();
    s.send(UserInput::from("eins")).await.unwrap();
    let mut events = until_end(&mut rx).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, EventPayload::HarnessAuthRequired(h) if h.hint.contains(VAR))),
        "{events:?}"
    );
    s.send(UserInput::from("zwei")).await.unwrap();
    events.extend(until_end(&mut rx).await);
    s.shutdown(Shutdown::Kill).await.unwrap();
    // Der Key ging an den Anbieter …
    assert!(
        server
            .requests()
            .iter()
            .all(|r| r.header("authorization") == Some(format!("Bearer {MARKER}").as_str()))
    );
    // … aber in kein Event (inkl. Debug-Ausgabe) und in kein Log.
    for e in &events {
        let json = serde_json::to_string(e).unwrap();
        assert!(!json.contains(MARKER), "Key im Event: {json}");
        assert!(!format!("{e:?}").contains(MARKER));
    }
    let failed = events
        .iter()
        .find_map(|e| match e {
            EventPayload::TurnFailed(f) => Some(f.problem.to_string()),
            _ => None,
        })
        .unwrap();
    assert!(failed.contains("[REDACTED]"), "{failed}");
    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(
        text.contains("Model-Request fehlgeschlagen"),
        "Logs der Fehlerpfade erfasst: {text}"
    );
    assert!(!text.contains(MARKER), "Key im Log: {text}");
}
