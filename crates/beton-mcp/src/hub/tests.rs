#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;

use async_trait::async_trait;
use beton_agents::spec::SystemTool;
use tokio::net::UnixStream;

use super::*;
use crate::config::{McpFile, merge, parse};
use crate::skills::SkillSet;
use crate::system::{SpawnTarget, SystemBackend, ToolFailure};

struct Echo;

#[async_trait]
impl SystemBackend for Echo {
    async fn policy_query(&self, _args: &Value) -> Result<Value, ToolFailure> {
        Ok(json!({"decision": "ask", "from": "runner"}))
    }
    async fn session_spawn(&self, _t: &SpawnTarget, _a: &Value) -> Result<Value, ToolFailure> {
        Err(ToolFailure::new("not_supported", "Test"))
    }
    async fn session_tool(
        &self,
        _tool: beton_agents::spec::SystemTool,
        _a: &Value,
    ) -> Result<Value, ToolFailure> {
        Err(ToolFailure::new("not_supported", "Test"))
    }
}

fn private_dir() -> tempfile::TempDir {
    // Kurzer Pfad: Unix-Sockets vertragen keine langen Pfade.
    let dir = tempfile::Builder::new()
        .prefix("bt")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

fn system() -> Arc<SystemServer> {
    Arc::new(SystemServer::new(
        vec![SystemTool::PolicyQuery],
        SkillSet::default(),
        Vec::new(),
        Arc::new(Echo),
    ))
}

fn servers(yaml: &str) -> Vec<ConfiguredServer> {
    let f: McpFile = parse(yaml, Path::new("mcp.yaml")).unwrap();
    merge(&McpFile::default(), &f, None)
}

async fn hub(
    dir: &Path,
    session: SessionId,
    servers: Vec<ConfiguredServer>,
    system: Option<Arc<SystemServer>>,
) -> (Hub, mpsc::UnboundedReceiver<EventPayload>) {
    Hub::start(HubOptions {
        session_id: session,
        run_dir: dir.to_owned(),
        workdir: dir.to_owned(),
        servers,
        system,
        relay: RelayCommand {
            program: PathBuf::from("/opt/beton/bin/beton"),
            prefix: Vec::new(),
        },
    })
    .await
    .unwrap()
}

struct Client {
    r: BufReader<tokio::net::unix::OwnedReadHalf>,
    w: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    async fn connect(
        socket: &Path,
        session: &str,
        token: &str,
        server: &str,
    ) -> (Self, HandshakeReply) {
        let (r, w) = UnixStream::connect(socket).await.unwrap().into_split();
        let mut c = Self {
            r: BufReader::new(r),
            w,
        };
        let h = json!({"relay": RELAY_VERSION, "session_id": session, "token": token, "server": server});
        c.send(&h).await;
        let reply = c.recv().await.unwrap();
        (c, serde_json::from_value(reply).unwrap())
    }

    async fn send(&mut self, v: &Value) {
        self.w.write_all(format!("{v}\n").as_bytes()).await.unwrap();
    }

    async fn recv(&mut self) -> Option<Value> {
        let line = tokio::time::timeout(Duration::from_secs(10), read_line(&mut self.r, MAX_LINE))
            .await
            .expect("Antwort innerhalb von 10 s")
            .unwrap()?;
        Some(serde_json::from_str(&line).unwrap())
    }
}

fn token(h: &Hub) -> String {
    std::fs::read_to_string(h.token_file()).unwrap()
}

#[tokio::test]
async fn har_009_ac3_token_of_another_session_is_rejected() {
    let dir = private_dir();
    let a = SessionId::new();
    let b = SessionId::new();
    let (hub_a, _ea) = hub(dir.path(), a, Vec::new(), Some(system())).await;
    let (hub_b, _eb) = hub(dir.path(), b, Vec::new(), Some(system())).await;
    let (ta, tb) = (token(&hub_a), token(&hub_b));
    assert_ne!(ta, tb);
    let (sa, sb) = (a.to_string(), b.to_string());
    // Token und Session von B am Hub von A, gemischte Kombinationen: alle abgelehnt, ohne
    // Hinweis, welcher Teil nicht passte.
    for (session, tok) in [(&sb, &tb), (&sa, &tb), (&sb, &ta), (&sa, &String::new())] {
        let (mut c, reply) = Client::connect(hub_a.socket(), session, tok, "beton").await;
        assert!(!reply.ok, "{session}/{tok} müsste abgelehnt werden");
        assert_eq!(reply.error.as_deref(), Some("unauthorized"));
        // Danach schließt der Hub die Verbindung.
        assert!(c.recv().await.is_none());
    }
    // Mit eigenem Token und eigener Session: policy_query beantwortet der Runner.
    let (mut c, reply) = Client::connect(hub_a.socket(), &sa, &ta, "beton").await;
    assert!(reply.ok, "{reply:?}");
    c.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "policy_query", "arguments": {"action": "git push"}}}))
        .await;
    let r = c.recv().await.unwrap();
    assert_eq!(r["result"]["structuredContent"]["from"], "runner", "{r}");
}

#[tokio::test]
async fn har_009_handshake_must_be_well_formed() {
    let dir = private_dir();
    let s = SessionId::new();
    let (h, _e) = hub(dir.path(), s, Vec::new(), Some(system())).await;
    let (r, mut w) = UnixStream::connect(h.socket()).await.unwrap().into_split();
    w.write_all(b"kein json\n").await.unwrap();
    let mut r = BufReader::new(r);
    let reply: HandshakeReply =
        serde_json::from_str(&read_line(&mut r, 4096).await.unwrap().unwrap()).unwrap();
    assert_eq!(reply.error.as_deref(), Some("bad_handshake"));
    // Falsche Relay-Version.
    let (r, mut w) = UnixStream::connect(h.socket()).await.unwrap().into_split();
    let bad =
        json!({"relay": 99, "session_id": s.to_string(), "token": token(&h), "server": "beton"});
    w.write_all(format!("{bad}\n").as_bytes()).await.unwrap();
    let mut r = BufReader::new(r);
    let reply: HandshakeReply =
        serde_json::from_str(&read_line(&mut r, 4096).await.unwrap().unwrap()).unwrap();
    assert_eq!(reply.error.as_deref(), Some("relay_version"));
}

#[tokio::test]
async fn har_009_ac2_without_bridge_the_system_server_is_not_offered() {
    let dir = private_dir();
    let s = SessionId::new();
    let (h, _e) = hub(
        dir.path(),
        s,
        servers("servers:\n  gh:\n    command: gh-mcp\n"),
        None,
    )
    .await;
    assert_eq!(h.names(), ["gh"]);
    assert!(h.launches().iter().all(|l| l.name != "beton"));
    let (_, reply) = Client::connect(h.socket(), &s.to_string(), &token(&h), "beton").await;
    assert_eq!(reply.error.as_deref(), Some("unknown_server"));
}

#[tokio::test]
async fn har_009_files_are_private_and_launches_carry_no_secrets() {
    let dir = private_dir();
    let s = SessionId::new();
    let (h, _e) = hub(
        dir.path(),
        s,
        servers("servers:\n  gh:\n    command: gh-mcp\n    env: { GITHUB_TOKEN: ghp_supersecretvalue }\n"),
        Some(system()),
    )
    .await;
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(h.token_file()), 0o600);
    assert_eq!(mode(h.socket()), 0o600);
    let tok = token(&h);
    assert_eq!(tok.len(), 64);
    let launches = h.launches();
    assert_eq!(
        launches.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
        ["beton", "gh"]
    );
    let text = format!("{launches:?}");
    assert!(!text.contains(&tok), "Token in der Harness-Konfiguration");
    assert!(
        !text.contains("ghp_supersecretvalue"),
        "Env-Wert in der Harness-Konfiguration"
    );
    assert_eq!(launches[0].command, "/opt/beton/bin/beton");
    assert_eq!(
        launches[1].args[..4],
        ["mcp", "proxy", "--server", "gh"].map(str::to_owned)
    );
    assert!(
        launches[0]
            .args
            .contains(&h.token_file().display().to_string())
    );
    // Aufräumen beim Drop.
    let (sock, tf) = (h.socket().to_owned(), h.token_file().to_owned());
    drop(h);
    assert!(!sock.exists() && !tf.exists());
}

#[tokio::test]
async fn har_009_run_dir_must_be_private() {
    let dir = private_dir();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = Hub::start(HubOptions {
        session_id: SessionId::new(),
        run_dir: dir.path().to_owned(),
        workdir: dir.path().to_owned(),
        servers: Vec::new(),
        system: Some(system()),
        relay: RelayCommand {
            program: "beton".into(),
            prefix: Vec::new(),
        },
    })
    .await
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
}

#[tokio::test]
async fn har_009_ac4_unstartable_server_reports_failure_and_answers_with_error() {
    let dir = private_dir();
    let s = SessionId::new();
    let (h, mut events) = hub(
        dir.path(),
        s,
        servers("servers:\n  kaputt:\n    command: /nicht/vorhanden/mcp-server\n  ungueltig:\n    command: x\n    url: http://a\n"),
        Some(system()),
    )
    .await;
    // Ungültige Einträge melden sich sofort und gehen nicht an den Harness.
    let first = events.recv().await.unwrap();
    let EventPayload::McpServerFailed(f) = first else {
        panic!("{first:?}")
    };
    assert_eq!(f.name, "ungueltig");
    assert!(!h.names().contains(&"ungueltig".to_owned()));
    let (mut c, reply) = Client::connect(h.socket(), &s.to_string(), &token(&h), "kaputt").await;
    assert!(reply.ok);
    c.send(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {}}))
        .await;
    let r = c.recv().await.unwrap();
    assert_eq!(r["error"]["code"], codes::SERVER_UNAVAILABLE, "{r}");
    let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    let EventPayload::McpServerFailed(f) = ev else {
        panic!("{ev:?}")
    };
    assert_eq!(f.name, "kaputt");
    assert!(f.error.contains("Start fehlgeschlagen"), "{}", f.error);
    // Der Server `beton` bleibt nutzbar.
    let (_, reply) = Client::connect(h.socket(), &s.to_string(), &token(&h), "beton").await;
    assert!(reply.ok);
}

#[tokio::test]
async fn har_009_ac4_server_exiting_before_initialize_is_reported() {
    let dir = private_dir();
    let s = SessionId::new();
    let (h, mut events) = hub(
        dir.path(),
        s,
        servers("servers:\n  stirbt:\n    command: sh\n    args: [\"-c\", \"exit 3\"]\n"),
        Some(system()),
    )
    .await;
    let (mut c, _) = Client::connect(h.socket(), &s.to_string(), &token(&h), "stirbt").await;
    c.send(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {}}))
        .await;
    let r = c.recv().await.unwrap();
    assert_eq!(r["error"]["code"], codes::SERVER_UNAVAILABLE, "{r}");
    let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    let EventPayload::McpServerFailed(f) = ev else {
        panic!("{ev:?}")
    };
    assert_eq!(f.name, "stirbt");
    assert!(f.error.contains("vor initialize"), "{}", f.error);
}

#[test]
fn agt_006_ac1_filter_hides_and_blocks_tools_outside_allow() {
    let server = servers("servers:\n  gh:\n    command: x\n    allow: [get_pull_request]\n")
        .pop()
        .unwrap();
    let f = Filter::default();
    let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    assert!(f.outbound(&server, &list).is_ok());
    let mut answer = json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": [
        {"name": "get_pull_request"}, {"name": "merge_pull_request"}
    ]}});
    assert_eq!(
        f.inbound(&server, &mut answer).as_deref(),
        Some("tools/list")
    );
    assert_eq!(
        answer["result"]["tools"],
        json!([{"name": "get_pull_request"}])
    );
    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "merge_pull_request"}});
    let reply = f.outbound(&server, &call).unwrap_err();
    assert_eq!(reply["error"]["data"]["code"], "tool_not_enabled");
    assert_eq!(reply["id"], 2);
    // Server-Anfragen (z. B. sampling) laufen ungefiltert durch.
    let mut req = json!({"jsonrpc": "2.0", "id": 9, "method": "roots/list"});
    assert_eq!(f.inbound(&server, &mut req), None);
}

#[tokio::test]
async fn agt_006_ac1_proxy_over_http_filters_tools() {
    use axum::routing::post;
    let app = axum::Router::new().route(
        "/mcp",
        post(|headers: axum::http::HeaderMap, body: String| async move {
            let msg: Value = serde_json::from_str(&body).unwrap();
            assert_eq!(headers["authorization"], "Bearer test-only");
            let id = msg["id"].clone();
            let result = match msg["method"].as_str().unwrap_or_default() {
                "initialize" => json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "h", "version": "1"}}),
                "tools/list" => json!({"tools": [{"name": "a"}, {"name": "b"}]}),
                "tools/call" => json!({"content": [{"type": "text", "text": "ok"}]}),
                _ => return (axum::http::StatusCode::ACCEPTED, [("mcp-session-id", "s1")], String::new()),
            };
            // tools/list als SSE, der Rest als JSON.
            if msg["method"] == "tools/list" {
                let data = json!({"jsonrpc": "2.0", "id": id, "result": result});
                return (axum::http::StatusCode::OK, [("content-type", "text/event-stream")], format!("event: message\ndata: {data}\n\n"));
            }
            (axum::http::StatusCode::OK, [("content-type", "application/json")], json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string())
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let dir = private_dir();
    let s = SessionId::new();
    let (h, _e) = hub(
        dir.path(),
        s,
        servers(&format!(
            "servers:\n  web:\n    url: http://{addr}/mcp\n    headers: {{ Authorization: \"Bearer test-only\" }}\n    allow: [a]\n"
        )),
        None,
    )
    .await;
    let (mut c, reply) = Client::connect(h.socket(), &s.to_string(), &token(&h), "web").await;
    assert!(reply.ok);
    c.send(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}))
        .await;
    assert_eq!(c.recv().await.unwrap()["result"]["serverInfo"]["name"], "h");
    c.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await;
    c.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .await;
    let list = c.recv().await.unwrap();
    assert_eq!(list["result"]["tools"], json!([{"name": "a"}]), "{list}");
    c.send(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "b"}}))
        .await;
    let denied = c.recv().await.unwrap();
    assert_eq!(denied["error"]["data"]["code"], "tool_not_enabled");
}

#[tokio::test]
async fn har_009_ac4_unreachable_http_server_reports_failure() {
    let dir = private_dir();
    let s = SessionId::new();
    // Port 9 auf Loopback: niemand hört zu.
    let (h, mut events) = hub(
        dir.path(),
        s,
        servers("servers:\n  weg:\n    url: http://127.0.0.1:9/mcp\n"),
        None,
    )
    .await;
    let (mut c, _) = Client::connect(h.socket(), &s.to_string(), &token(&h), "weg").await;
    c.send(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {}}))
        .await;
    let r = c.recv().await.unwrap();
    assert_eq!(r["error"]["code"], codes::SERVER_UNAVAILABLE, "{r}");
    let ev = events.recv().await.unwrap();
    let EventPayload::McpServerFailed(f) = ev else {
        panic!("{ev:?}")
    };
    assert_eq!(f.name, "weg");
}

#[tokio::test]
async fn har_009_ac3_relay_uses_the_token_file_and_is_bound_to_its_session() {
    use crate::relay::{RelayArgs, RelayError, run};
    let dir = private_dir();
    let a = SessionId::new();
    let b = SessionId::new();
    let (hub_a, _ea) = hub(dir.path(), a, Vec::new(), Some(system())).await;
    let (hub_b, _eb) = hub(dir.path(), b, Vec::new(), Some(system())).await;

    // Relay mit dem Token von B am Hub von A: abgelehnt.
    let args = RelayArgs {
        server: "beton".into(),
        session: b.to_string(),
        socket: hub_a.socket().to_owned(),
        token_file: hub_b.token_file().to_owned(),
    };
    let err = run(&args, tokio::io::empty(), tokio::io::sink())
        .await
        .unwrap_err();
    assert!(
        matches!(err, RelayError::Rejected(ref e) if e == "unauthorized"),
        "{err}"
    );
    assert!(!err.to_string().contains(&token(&hub_b)));

    // Eigenes Token: das Relay reicht MCP durch.
    let args = RelayArgs {
        server: "beton".into(),
        session: a.to_string(),
        socket: hub_a.socket().to_owned(),
        token_file: hub_a.token_file().to_owned(),
    };
    let (mut harness_w, relay_in) = tokio::io::duplex(1 << 16);
    let (relay_out, harness_r) = tokio::io::duplex(1 << 16);
    let relay = tokio::spawn(async move { run(&args, relay_in, relay_out).await });
    harness_w
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
        .await
        .unwrap();
    let mut harness_r = BufReader::new(harness_r);
    let line = tokio::time::timeout(Duration::from_secs(10), read_line(&mut harness_r, MAX_LINE))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let v: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["result"]["tools"][0]["name"], "policy_query");
    drop(harness_w);
    drop(hub_a);
    let _ = tokio::time::timeout(Duration::from_secs(5), relay).await;
}
