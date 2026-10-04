//! Tool-Ausführung über MCP (HAR-010, HAR-009): ein In-Process-MCP-Client für den
//! eingebauten Server `beton-workspace` und für die stdio-Relays aus `SessionSpec.mcp`
//! (`beton mcp serve|proxy`, also der Server `beton` mit den System-Tools und alle externen
//! Server über den Relay-Hub des Runners).
//!
//! Das Modell sieht Workspace-Tools unter ihrem Namen (`fs_read` …) und MCP-Tools als
//! `mcp__<server>__<tool>`. Relay-Kommandos enthalten nur Pfade, nie Tokens (HAR-009).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{EventPayload, McpServerFailed, ToolSource};
use beton_harness::McpLaunch;
use beton_harness::process::{LaunchSpec, ProcessHandle, ProcessLauncher};
use beton_mcp::workspace::{self, WORKSPACE_SERVER, WorkspaceServer};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, oneshot};

/// Höchstdauer des MCP-Handshakes je Server.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// Höchstens so viel Tool-Ergebnis geht an das Modell.
pub const RESULT_MAX_CHARS: usize = 100_000;

/// Ein Tool, wie es das Modell sieht, mit seiner Herkunft für Events und Gate.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInfo {
    /// Name für das Modell, z. B. `fs_read` oder `mcp__beton__policy_query`.
    pub model_name: String,
    /// Name des Tools beim Server.
    pub tool: String,
    pub server: String,
    pub description: String,
    pub schema: Value,
    /// Kanonische Klasse (POL-005).
    pub kind: String,
    pub source: ToolSource,
}

/// Ergebnis eines Aufrufs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    pub ok: bool,
    pub text: String,
}

/// Führt die Tools einer Session aus.
#[async_trait]
pub trait ToolHost: Send + Sync {
    fn tools(&self) -> Vec<ToolInfo>;
    async fn call(&self, model_name: &str, args: Value) -> ToolOutcome;
    /// Beendet gestartete Server.
    async fn shutdown(&self) {}
}

/// Ein Tool-Name, den Anthropic und OpenAI akzeptieren (`[a-zA-Z0-9_-]{1,64}`).
pub fn valid_model_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Text aus einem MCP-`tools/call`-Ergebnis.
pub fn result_text(result: &Value) -> String {
    let mut parts = Vec::new();
    for c in result["content"].as_array().into_iter().flatten() {
        match c["type"].as_str() {
            Some("text") => parts.push(c["text"].as_str().unwrap_or_default().to_owned()),
            Some(other) => parts.push(format!("[{other}-Inhalt ausgelassen]")),
            None => {}
        }
    }
    if parts.is_empty()
        && let Some(s) = result.get("structuredContent")
    {
        parts.push(s.to_string());
    }
    let text = parts.join("\n");
    if text.chars().count() > RESULT_MAX_CHARS {
        let cut: String = text.chars().take(RESULT_MAX_CHARS).collect();
        format!("{cut}\n[… Ergebnis gekürzt …]")
    } else {
        text
    }
}

type Writer = Arc<Mutex<Box<dyn AsyncWrite + Send + Unpin>>>;
type Pending = Arc<std::sync::Mutex<HashMap<u64, oneshot::Sender<Result<Value, Value>>>>>;

/// Ein MCP-Server über stdio.
struct StdioServer {
    name: String,
    stdin: Writer,
    pending: Pending,
    next: std::sync::atomic::AtomicU64,
    process: Mutex<Box<dyn ProcessHandle>>,
}

impl StdioServer {
    async fn start(
        launch: &McpLaunch,
        launcher: &dyn ProcessLauncher,
        workdir: &Path,
    ) -> Result<(Self, Vec<Value>), String> {
        let mut process = launcher
            .launch(LaunchSpec {
                program: launch.command.clone().into(),
                args: launch.args.clone(),
                env: Vec::new(),
                env_remove: Vec::new(),
                clear_env: false,
                cwd: Some(workdir.to_path_buf()),
            })
            .await
            .map_err(|e| format!("Start fehlgeschlagen: {e}"))?;
        let io = process
            .take_io()
            .ok_or_else(|| "Start fehlgeschlagen: kein stdio".to_owned())?;
        let pending: Pending = Arc::default();
        let reader_pending = pending.clone();
        let stdin: Writer = Arc::new(Mutex::new(io.stdin));
        let reply_to = stdin.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(io.stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                match beton_mcp::protocol::kind(&msg) {
                    beton_mcp::protocol::Kind::Response => {
                        let Some(id) = msg["id"].as_u64() else {
                            continue;
                        };
                        let tx = reader_pending.lock().ok().and_then(|mut p| p.remove(&id));
                        if let Some(tx) = tx {
                            let _ = tx.send(match msg.get("error") {
                                Some(e) => Err(e.clone()),
                                None => Ok(msg["result"].clone()),
                            });
                        }
                    }
                    // Anfragen des Servers (roots, sampling …) unterstützt der Client nicht.
                    beton_mcp::protocol::Kind::Request => {
                        let reply = beton_mcp::protocol::error(
                            &msg["id"],
                            beton_mcp::protocol::codes::METHOD_NOT_FOUND,
                            "vom Client nicht unterstützt",
                            None,
                        );
                        let mut w = reply_to.lock().await;
                        let _ = w.write_all(format!("{reply}\n").as_bytes()).await;
                        let _ = w.flush().await;
                    }
                    _ => {}
                }
            }
            // Server beendet: offene Aufrufe scheitern (fail closed).
            if let Ok(mut p) = reader_pending.lock() {
                for (_, tx) in p.drain() {
                    let _ = tx.send(Err(json!({"message": "MCP-Server beendet"})));
                }
            }
        });
        let server = Self {
            name: launch.name.clone(),
            stdin,
            pending,
            next: std::sync::atomic::AtomicU64::new(0),
            process: Mutex::new(process),
        };
        let handshake = async {
            server
                .request(
                    "initialize",
                    json!({
                        "protocolVersion": beton_mcp::protocol::PROTOCOL_VERSIONS[1],
                        "capabilities": {},
                        "clientInfo": {"name": "beton-direct", "version": env!("CARGO_PKG_VERSION")},
                    }),
                )
                .await
                .map_err(|e| format!("initialize: {}", e["message"].as_str().unwrap_or("Fehler")))?;
            server
                .notify("notifications/initialized", json!({}))
                .await
                .map_err(|e| format!("initialized: {e}"))?;
            let mut tools = Vec::new();
            let mut cursor: Option<String> = None;
            for _ in 0..50 {
                let params = match &cursor {
                    Some(c) => json!({"cursor": c}),
                    None => json!({}),
                };
                let page = server.request("tools/list", params).await.map_err(|e| {
                    format!("tools/list: {}", e["message"].as_str().unwrap_or("Fehler"))
                })?;
                tools.extend(page["tools"].as_array().cloned().unwrap_or_default());
                cursor = page["nextCursor"].as_str().map(str::to_owned);
                if cursor.is_none() {
                    break;
                }
            }
            Ok::<_, String>(tools)
        };
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await {
            Ok(Ok(tools)) => Ok((server, tools)),
            Ok(Err(e)) => {
                server.kill().await;
                Err(e)
            }
            Err(_) => {
                server.kill().await;
                Err(format!(
                    "keine Antwort auf initialize innerhalb von {} s",
                    HANDSHAKE_TIMEOUT.as_secs()
                ))
            }
        }
    }

    async fn write(&self, msg: &Value) -> std::io::Result<()> {
        let mut w = self.stdin.lock().await;
        w.write_all(format!("{msg}\n").as_bytes()).await?;
        w.flush().await
    }

    async fn notify(&self, method: &str, params: Value) -> std::io::Result<()> {
        self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(id, tx);
        }
        if let Err(e) = self
            .write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await
        {
            return Err(json!({"message": format!("MCP-Server nicht erreichbar: {e}")}));
        }
        rx.await
            .unwrap_or_else(|_| Err(json!({"message": "MCP-Server beendet"})))
    }

    async fn kill(&self) {
        let _ = self.process.lock().await.kill().await;
    }
}

/// Workspace-Tools und MCP-Server einer Session.
pub struct McpTools {
    workspace: Option<WorkspaceServer>,
    servers: Vec<Arc<StdioServer>>,
    tools: Vec<ToolInfo>,
}

impl std::fmt::Debug for McpTools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpTools")
            .field("tools", &self.tools.len())
            .finish_non_exhaustive()
    }
}

impl McpTools {
    /// Startet die Relays und fragt ihre Tools ab. Server, die nicht starten, melden je
    /// genau ein `mcp.server_failed` (HAR-009 AC4); die Session läuft weiter.
    pub async fn connect(
        workdir: &Path,
        servers: &[McpLaunch],
        launcher: &dyn ProcessLauncher,
    ) -> (Self, Vec<EventPayload>) {
        let mut failures = Vec::new();
        let mut tools = Vec::new();
        let workspace = match WorkspaceServer::new(workdir) {
            Ok(ws) => {
                for def in WorkspaceServer::definitions() {
                    let name = def["name"].as_str().unwrap_or_default().to_owned();
                    tools.push(ToolInfo {
                        model_name: name.clone(),
                        kind: workspace::tool_kind(&name).into(),
                        tool: name,
                        server: WORKSPACE_SERVER.into(),
                        description: def["description"].as_str().unwrap_or_default().into(),
                        schema: def["inputSchema"].clone(),
                        source: ToolSource::Harness,
                    });
                }
                Some(ws)
            }
            Err(e) => {
                failures.push(EventPayload::McpServerFailed(McpServerFailed {
                    name: WORKSPACE_SERVER.into(),
                    error: format!("Arbeitsverzeichnis nicht nutzbar: {e}"),
                }));
                None
            }
        };
        let mut started = Vec::new();
        for launch in servers {
            match StdioServer::start(launch, launcher, workdir).await {
                Ok((server, defs)) => {
                    for def in defs {
                        let tool = def["name"].as_str().unwrap_or_default().to_owned();
                        let model_name = format!("mcp__{}__{tool}", launch.name);
                        if !valid_model_name(&model_name) {
                            tracing::warn!(server = %launch.name, "MCP-Tool mit ungültigem Namen übersprungen");
                            continue;
                        }
                        let system = launch.name == beton_mcp::config::SYSTEM_SERVER;
                        tools.push(ToolInfo {
                            model_name,
                            tool,
                            server: launch.name.clone(),
                            description: def["description"].as_str().unwrap_or_default().into(),
                            schema: if def["inputSchema"].is_object() {
                                def["inputSchema"].clone()
                            } else {
                                json!({"type": "object"})
                            },
                            kind: if system { "system" } else { "mcp" }.into(),
                            source: if system {
                                ToolSource::BetonMcp
                            } else {
                                ToolSource::Harness
                            },
                        });
                    }
                    started.push(Arc::new(server));
                }
                Err(e) => failures.push(EventPayload::McpServerFailed(McpServerFailed {
                    name: launch.name.clone(),
                    error: e,
                })),
            }
        }
        (
            Self {
                workspace,
                servers: started,
                tools,
            },
            failures,
        )
    }

    /// Namen der verbundenen Server.
    pub fn server_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.workspace.is_some() {
            out.push(WORKSPACE_SERVER.to_owned());
        }
        out.extend(self.servers.iter().map(|s| s.name.clone()));
        out
    }
}

#[async_trait]
impl ToolHost for McpTools {
    fn tools(&self) -> Vec<ToolInfo> {
        self.tools.clone()
    }

    async fn call(&self, model_name: &str, args: Value) -> ToolOutcome {
        let Some(info) = self.tools.iter().find(|t| t.model_name == model_name) else {
            return ToolOutcome {
                ok: false,
                text: format!("tool_not_enabled: {model_name} gibt es in dieser Session nicht"),
            };
        };
        if info.server == WORKSPACE_SERVER {
            let Some(ws) = &self.workspace else {
                return ToolOutcome {
                    ok: false,
                    text: "unavailable: beton-workspace nicht verfügbar".into(),
                };
            };
            return match ws.call(&info.tool, &args).await {
                Ok(text) => ToolOutcome { ok: true, text },
                Err(e) => ToolOutcome {
                    ok: false,
                    text: e.0,
                },
            };
        }
        let Some(server) = self.servers.iter().find(|s| s.name == info.server) else {
            return ToolOutcome {
                ok: false,
                text: format!("unavailable: MCP-Server {} nicht verbunden", info.server),
            };
        };
        match server
            .request("tools/call", json!({"name": info.tool, "arguments": args}))
            .await
        {
            Ok(result) => ToolOutcome {
                ok: result["isError"] != true,
                text: result_text(&result),
            },
            Err(e) => {
                let code = e["data"]["code"].as_str();
                let msg = e["message"].as_str().unwrap_or("MCP-Fehler");
                ToolOutcome {
                    ok: false,
                    text: match code {
                        Some(c) if !msg.starts_with(c) => format!("{c}: {msg}"),
                        _ => msg.to_owned(),
                    },
                }
            }
        }
    }

    async fn shutdown(&self) {
        for s in &self.servers {
            s.kill().await;
        }
    }
}
