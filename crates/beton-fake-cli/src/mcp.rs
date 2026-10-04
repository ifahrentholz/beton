//! MCP auf Seiten der Fake-CLI (HAR-009): ein minimaler stdio-Client, wie ihn die echten CLIs
//! für injizierte Server haben, und ein Test-MCP-Server (`--protocol mcp-server`).
//!
//! Der Client startet jeden übergebenen Server (bei beton immer ein Relay `beton mcp …`),
//! führt `initialize` und `tools/list` aus und ruft Tools für den Szenario-Schritt `mcp_call`
//! auf. Argumente können frühere Ergebnisse dieses Prozesses verwenden: `"${mcp.<n>.<pfad>}"`
//! steht für den Wert unter `<pfad>` (Punkte trennen Felder bzw. Indizes) im
//! `structuredContent` des n-ten erfolgreichen `mcp_call` (ab 0), z. B. die `session_id` eines
//! gestarteten Sub-Agents (AGT-009). Steht der Ausdruck allein, wird der Wert eingesetzt, sonst
//! als Text.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

/// Ein Server aus der Konfiguration des Harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

struct Conn {
    name: String,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
    tools: Vec<String>,
}

impl Drop for Conn {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Conn {
    fn rpc(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{msg}").map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        loop {
            let mut line = String::new();
            if self
                .stdout
                .read_line(&mut line)
                .map_err(|e| e.to_string())?
                == 0
            {
                return Err(format!("MCP-Server {} beendet", self.name));
            }
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if v.get("method").is_some() {
                // Anfragen des Servers (sampling, roots) beantwortet die Fake-CLI nicht.
                if let Some(rid) = v.get("id") {
                    let reply = json!({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "nicht unterstützt"}});
                    let _ = writeln!(self.stdin, "{reply}");
                }
                continue;
            }
            if v["id"] == json!(id) {
                if let Some(err) = v.get("error") {
                    return Err(err["message"].as_str().unwrap_or("Fehler").to_owned());
                }
                return Ok(v["result"].clone());
            }
        }
    }

    fn notify(&mut self, method: &str) {
        let _ = writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "method": method})
        );
        let _ = self.stdin.flush();
    }
}

/// Verbundene MCP-Server einer Session.
#[derive(Default)]
pub struct Clients {
    conns: Vec<Conn>,
    /// `structuredContent` der erfolgreichen Aufrufe in Reihenfolge.
    results: Vec<Value>,
    /// Server, deren Start scheiterte, mit Grund.
    pub failed: Vec<(String, String)>,
}

impl Clients {
    /// Startet alle Server, führt `initialize` und `tools/list` aus.
    pub fn connect(configs: &[ServerConfig]) -> Self {
        let mut out = Self::default();
        for c in configs {
            match connect_one(c) {
                Ok(conn) => out.conns.push(conn),
                Err(e) => out.failed.push((c.name.clone(), e)),
            }
        }
        out
    }

    /// Namen der Tools wie bei Claude Code: `mcp__<server>__<tool>`.
    pub fn claude_tool_names(&self) -> Vec<String> {
        self.conns
            .iter()
            .flat_map(|c| c.tools.iter().map(move |t| format!("mcp__{}__{t}", c.name)))
            .collect()
    }

    /// Status je Server wie im `system/init` von Claude Code.
    pub fn status(&self) -> Vec<Value> {
        let mut out: Vec<Value> = self
            .conns
            .iter()
            .map(|c| json!({"name": c.name, "status": "connected"}))
            .collect();
        out.extend(
            self.failed
                .iter()
                .map(|(n, _)| json!({"name": n, "status": "failed"})),
        );
        out
    }

    pub fn connected(&self) -> Vec<String> {
        self.conns.iter().map(|c| c.name.clone()).collect()
    }

    /// `tools/call`; `Ok` ist das `CallToolResult`, `Err` ein Protokoll- oder Startfehler.
    pub fn call(&mut self, server: &str, tool: &str, args: &Value) -> Result<Value, String> {
        let conn = self
            .conns
            .iter_mut()
            .find(|c| c.name == server)
            .ok_or_else(|| format!("MCP-Server {server} nicht verbunden"))?;
        let result = conn.rpc(
            "tools/call",
            json!({"name": tool, "arguments": args.clone()}),
        )?;
        if result["isError"] != true {
            self.results.push(result["structuredContent"].clone());
        }
        Ok(result)
    }

    /// Setzt `${mcp.<n>.<pfad>}` in Argumenten ein (siehe Modul-Doku).
    pub fn resolve(&self, args: &Value) -> Value {
        match args {
            Value::String(s) => self.resolve_text(s),
            Value::Array(a) => Value::Array(a.iter().map(|v| self.resolve(v)).collect()),
            Value::Object(m) => Value::Object(
                m.iter()
                    .map(|(k, v)| (k.clone(), self.resolve(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    fn lookup(&self, expr: &str) -> Value {
        let mut parts = expr.split('.');
        let Some(n) = parts.next().and_then(|n| n.parse::<usize>().ok()) else {
            return Value::Null;
        };
        let mut v = self.results.get(n).cloned().unwrap_or(Value::Null);
        for p in parts {
            v = match p.parse::<usize>() {
                Ok(i) if v.is_array() => v[i].clone(),
                _ => v[p].clone(),
            };
        }
        v
    }

    fn resolve_text(&self, s: &str) -> Value {
        if let Some(expr) = s.strip_prefix("${mcp.").and_then(|r| r.strip_suffix('}'))
            && !expr.contains('}')
        {
            return self.lookup(expr);
        }
        let mut out = String::new();
        let mut rest = s;
        while let Some(start) = rest.find("${mcp.") {
            out.push_str(&rest[..start]);
            let tail = &rest[start + 6..];
            let Some(end) = tail.find('}') else {
                out.push_str(&rest[start..]);
                rest = "";
                break;
            };
            match self.lookup(&tail[..end]) {
                Value::String(t) => out.push_str(&t),
                other => out.push_str(&other.to_string()),
            }
            rest = &tail[end + 1..];
        }
        out.push_str(rest);
        Value::String(out)
    }
}

fn connect_one(c: &ServerConfig) -> Result<Conn, String> {
    let mut child = Command::new(&c.command)
        .args(&c.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("Start fehlgeschlagen: {e}"))?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err("keine Pipes".into());
    };
    let mut conn = Conn {
        name: c.name.clone(),
        child,
        stdin,
        stdout: BufReader::new(stdout),
        next: 0,
        tools: Vec::new(),
    };
    conn.rpc(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "beton-fake-cli", "version": env!("CARGO_PKG_VERSION")}}),
    )?;
    conn.notify("notifications/initialized");
    let list = conn.rpc("tools/list", json!({}))?;
    conn.tools = list["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect();
    Ok(conn)
}

/// Text eines `CallToolResult` (Textblöcke aneinandergehängt).
pub fn result_text(result: &Value) -> String {
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Server aus `--mcp-config` (Claude): JSON-Text oder Pfad einer Datei mit `mcpServers`.
pub fn from_claude_config(arg: &str) -> Vec<ServerConfig> {
    let text = if arg.trim_start().starts_with('{') {
        arg.to_owned()
    } else {
        std::fs::read_to_string(arg).unwrap_or_default()
    };
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    from_map(&v["mcpServers"])
}

/// Server aus `thread/start.config.mcp_servers` (Codex).
pub fn from_codex_config(config: &Value) -> Vec<ServerConfig> {
    from_map(&config["mcp_servers"])
}

fn from_map(map: &Value) -> Vec<ServerConfig> {
    map.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, s)| {
            Some(ServerConfig {
                name: name.clone(),
                command: s["command"].as_str()?.to_owned(),
                args: strings(&s["args"]),
            })
        })
        .collect()
}

/// Server aus `session/new.mcpServers` (ACP).
pub fn from_acp(servers: &Value) -> Vec<ServerConfig> {
    servers
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            Some(ServerConfig {
                name: s["name"].as_str()?.to_owned(),
                command: s["command"].as_str()?.to_owned(),
                args: strings(&s["args"]),
            })
        })
        .collect()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a.as_str().map(str::to_owned))
        .collect()
}

/// Test-MCP-Server: bietet die Tools `tools` an; `tools/call` antwortet mit
/// `<tool>: <argumente>`.
pub fn serve(tools: &[String]) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let reply = match msg["method"].as_str().unwrap_or_default() {
            "initialize" => json!({"jsonrpc": "2.0", "id": id, "result": {
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "beton-fake-mcp", "version": env!("CARGO_PKG_VERSION")},
            }}),
            "tools/list" => {
                json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tools.iter().map(|t| json!({
                "name": t, "description": format!("Test-Tool {t}"), "inputSchema": {"type": "object"},
            })).collect::<Vec<_>>()}})
            }
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or_default();
                json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": format!("{name}: {}", msg["params"]["arguments"])}]}})
            }
            "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            m => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("Methode {m} gibt es nicht")}})
            }
        };
        writeln!(out, "{reply}")?;
        out.flush()?;
    }
    Ok(())
}
