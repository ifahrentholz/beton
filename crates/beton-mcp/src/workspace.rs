//! Der eingebaute MCP-Server `beton-workspace` mit den Coding-Tools des Direkt-API-Harness
//! (HAR-010): `fs_read`, `fs_write`, `fs_edit`, `fs_glob`, `fs_grep`, `shell_exec`.
//!
//! Der Server läuft im Runner-Prozess und wird über JSON-RPC wie jeder MCP-Server
//! angesprochen ([`WorkspaceServer::handle`]). Jeder Aufruf hat vorher das Gate des Runners
//! passiert; dieser Server prüft nur noch die Pfadgrenzen.
//!
//! Sicherheitsregel (fail closed): Pfade sind relativ zum Arbeitsverzeichnis der Session.
//! Absolute Pfade, `..`, `.git` und Pfade, die nach Auflösung aller Symlinks außerhalb des
//! Arbeitsverzeichnisses liegen, werden abgewiesen – auch wenn das Ziel nicht existiert.
//! `shell_exec` läuft im Arbeitsverzeichnis ohne interne `BETON_*`-Variablen; die
//! Tool-Sandbox Stufe 2 folgt mit SBX-002 (M2).

use std::path::{Path, PathBuf};
use std::time::Duration;

use beton_harness::process::{LaunchSpec, ProcessLauncher, RealLauncher};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use crate::protocol::{self, Kind, codes};

/// Name des Servers in Events (`mcp_server`).
pub const WORKSPACE_SERVER: &str = "beton-workspace";

/// Höchstens so viel Text liefert `fs_read`.
pub const READ_MAX_BYTES: usize = 256 * 1024;
/// Höchstens so viel Ausgabe liefert `shell_exec`.
pub const SHELL_MAX_BYTES: usize = 64 * 1024;
/// Standard- und Höchstdauer von `shell_exec`.
pub const SHELL_DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
pub const SHELL_MAX_TIMEOUT: Duration = Duration::from_secs(600);
/// Höchstzahl der Treffer von `fs_glob` bzw. `fs_grep`.
pub const GLOB_MAX: usize = 1000;
pub const GREP_MAX: usize = 200;

/// Namen der Tools in Übergabe-Reihenfolge.
pub const TOOLS: [&str; 6] = [
    "fs_read",
    "fs_write",
    "fs_edit",
    "fs_glob",
    "fs_grep",
    "shell_exec",
];

/// Kanonische Tool-Klasse (POL-005) eines Workspace-Tools.
pub fn tool_kind(tool: &str) -> &'static str {
    match tool {
        "fs_read" => "file_read",
        "fs_write" => "file_write",
        "fs_edit" => "file_edit",
        "fs_glob" | "fs_grep" => "search",
        "shell_exec" => "shell",
        _ => "other",
    }
}

/// Fehler eines Tool-Aufrufs, wie ihn das Modell sieht.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ToolError(pub String);

fn err(msg: impl Into<String>) -> ToolError {
    ToolError(msg.into())
}

/// Der Server einer Session.
#[derive(Debug, Clone)]
pub struct WorkspaceServer {
    root: PathBuf,
}

impl WorkspaceServer {
    /// Öffnet den Server für ein Arbeitsverzeichnis (wird kanonisiert).
    pub fn new(workdir: &Path) -> std::io::Result<Self> {
        let root = workdir.canonicalize()?;
        if !root.is_dir() {
            return Err(std::io::Error::other("Arbeitsverzeichnis fehlt"));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Tool-Definitionen (`tools/list`).
    pub fn definitions() -> Vec<Value> {
        let path = json!({"type": "string", "description": "Pfad relativ zum Arbeitsverzeichnis."});
        vec![
            json!({"name": "fs_read", "description": "Liest eine Textdatei im Arbeitsverzeichnis. Optional ab Zeile `offset` (1-basiert) höchstens `limit` Zeilen.",
                "inputSchema": {"type": "object", "properties": {"path": path, "offset": {"type": "integer", "minimum": 1}, "limit": {"type": "integer", "minimum": 1}}, "required": ["path"]}}),
            json!({"name": "fs_write", "description": "Schreibt eine Datei (legt Verzeichnisse an, überschreibt vorhandene Dateien).",
                "inputSchema": {"type": "object", "properties": {"path": path, "content": {"type": "string"}}, "required": ["path", "content"]}}),
            json!({"name": "fs_edit", "description": "Ersetzt `old_string` durch `new_string` in einer Datei. `old_string` muss genau einmal vorkommen, außer `replace_all` ist gesetzt.",
                "inputSchema": {"type": "object", "properties": {"path": path, "old_string": {"type": "string"}, "new_string": {"type": "string"}, "replace_all": {"type": "boolean"}}, "required": ["path", "old_string", "new_string"]}}),
            json!({"name": "fs_glob", "description": "Listet Dateien, die zu einem Glob passen (z. B. `src/**/*.rs`); `.gitignore` gilt.",
                "inputSchema": {"type": "object", "properties": {"pattern": {"type": "string"}}, "required": ["pattern"]}}),
            json!({"name": "fs_grep", "description": "Sucht einen regulären Ausdruck in Dateien; Treffer als `pfad:zeile: text`. Optional auf `glob` beschränkt.",
                "inputSchema": {"type": "object", "properties": {"pattern": {"type": "string"}, "glob": {"type": "string"}, "case_insensitive": {"type": "boolean"}}, "required": ["pattern"]}}),
            json!({"name": "shell_exec", "description": "Führt ein Shell-Kommando im Arbeitsverzeichnis aus und liefert Exit-Code und Ausgabe (stdout und stderr).",
                "inputSchema": {"type": "object", "properties": {"command": {"type": "string"}, "timeout_s": {"type": "integer", "minimum": 1, "maximum": 600}}, "required": ["command"]}}),
        ]
    }

    /// Verarbeitet eine JSON-RPC-Nachricht; `None` für Notifications und Antworten.
    pub async fn handle(&self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        match protocol::kind(msg) {
            Kind::Notification | Kind::Response => return None,
            Kind::Invalid => {
                return Some(protocol::error(
                    &id,
                    codes::INVALID_REQUEST,
                    "ungültige JSON-RPC-Nachricht",
                    None,
                ));
            }
            Kind::Request => {}
        }
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        Some(match msg["method"].as_str().unwrap_or_default() {
            "initialize" => protocol::result(
                &id,
                json!({
                    "protocolVersion": protocol::negotiate(params["protocolVersion"].as_str()),
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": WORKSPACE_SERVER, "version": env!("CARGO_PKG_VERSION")},
                }),
            ),
            "ping" => protocol::result(&id, json!({})),
            "tools/list" => protocol::result(&id, json!({"tools": Self::definitions()})),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                protocol::result(
                    &id,
                    match self.call(name, &args).await {
                        Ok(text) => protocol::text_result(text, false),
                        Err(e) => protocol::text_result(e.0, true),
                    },
                )
            }
            other => protocol::error(
                &id,
                codes::METHOD_NOT_FOUND,
                format!("Methode {other} gibt es nicht"),
                None,
            ),
        })
    }

    /// Führt ein Tool aus.
    pub async fn call(&self, name: &str, args: &Value) -> Result<String, ToolError> {
        let s = |k: &str| -> Result<&str, ToolError> {
            args[k]
                .as_str()
                .ok_or_else(|| err(format!("invalid_args: `{k}` fehlt")))
        };
        match name {
            "fs_read" => self.read(
                s("path")?,
                args["offset"].as_u64().unwrap_or(1),
                args["limit"].as_u64(),
            ),
            "fs_write" => self.write(s("path")?, s("content")?),
            "fs_edit" => self.edit(
                s("path")?,
                s("old_string")?,
                s("new_string")?,
                args["replace_all"] == true,
            ),
            "fs_glob" => self.glob(s("pattern")?),
            "fs_grep" => self.grep(
                s("pattern")?,
                args["glob"].as_str(),
                args["case_insensitive"] == true,
            ),
            "shell_exec" => {
                let timeout = args["timeout_s"]
                    .as_u64()
                    .map(Duration::from_secs)
                    .unwrap_or(SHELL_DEFAULT_TIMEOUT)
                    .min(SHELL_MAX_TIMEOUT);
                self.shell(s("command")?, timeout).await
            }
            other => Err(err(format!("tool_not_enabled: {other} gibt es nicht"))),
        }
    }

    // ------------------------------------------------------------------ Pfade

    fn contained(&self, canonical: &Path) -> bool {
        canonical.starts_with(&self.root)
            && !canonical
                .strip_prefix(&self.root)
                .unwrap_or(canonical)
                .components()
                .any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case(".git"))
    }

    fn components(raw: &str) -> Result<Vec<String>, ToolError> {
        let forbidden = || {
            err(format!(
                "forbidden: `{raw}` liegt außerhalb des Arbeitsverzeichnisses oder ist nicht erlaubt"
            ))
        };
        if raw.starts_with('/') || raw.contains('\0') {
            return Err(forbidden());
        }
        if cfg!(windows) && (raw.contains('\\') || raw.contains(':')) {
            return Err(forbidden());
        }
        let mut out = Vec::new();
        for part in raw.split('/') {
            match part {
                "" | "." => {}
                ".." => return Err(forbidden()),
                p if p.eq_ignore_ascii_case(".git") => return Err(forbidden()),
                p => out.push(p.to_owned()),
            }
        }
        Ok(out)
    }

    /// Kanonischer nächster existierender Vorfahr und Zahl der fehlenden Komponenten.
    fn existing_ancestor(
        &self,
        raw: &str,
        parts: &[String],
    ) -> Result<(PathBuf, usize), ToolError> {
        let forbidden = || {
            err(format!(
                "forbidden: `{raw}` liegt außerhalb des Arbeitsverzeichnisses oder ist nicht erlaubt"
            ))
        };
        for keep in (0..=parts.len()).rev() {
            let mut probe = self.root.clone();
            probe.extend(&parts[..keep]);
            if std::fs::symlink_metadata(&probe).is_err() {
                continue;
            }
            let canonical = probe.canonicalize().map_err(|_| forbidden())?;
            if !self.contained(&canonical) {
                return Err(forbidden());
            }
            return Ok((canonical, parts.len() - keep));
        }
        Err(forbidden())
    }

    fn resolve_existing(&self, raw: &str) -> Result<PathBuf, ToolError> {
        let parts = Self::components(raw)?;
        let (canonical, missing) = self.existing_ancestor(raw, &parts)?;
        if missing > 0 {
            return Err(err(format!("not_found: `{raw}` gibt es nicht")));
        }
        Ok(canonical)
    }

    fn resolve_for_write(&self, raw: &str) -> Result<PathBuf, ToolError> {
        let parts = Self::components(raw)?;
        if parts.is_empty() {
            return Err(err("invalid_args: Dateipfad fehlt"));
        }
        let (canonical, missing) = self.existing_ancestor(raw, &parts)?;
        if missing == 0 {
            if canonical.is_dir() {
                return Err(err(format!("invalid_args: `{raw}` ist ein Verzeichnis")));
            }
            return Ok(canonical);
        }
        let mut target = canonical;
        target.extend(&parts[parts.len() - missing..]);
        Ok(target)
    }

    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    }

    // ------------------------------------------------------------------ Tools

    fn read_text(&self, raw: &str) -> Result<(PathBuf, String), ToolError> {
        let path = self.resolve_existing(raw)?;
        if path.is_dir() {
            return Err(err(format!("invalid_args: `{raw}` ist ein Verzeichnis")));
        }
        let bytes = std::fs::read(&path).map_err(|e| err(format!("io: {raw}: {e}")))?;
        if bytes.iter().take(8000).any(|b| *b == 0) {
            return Err(err(format!("binary_file: `{raw}` ist keine Textdatei")));
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| err(format!("binary_file: `{raw}` ist kein UTF-8")))?;
        Ok((path, text))
    }

    fn read(&self, raw: &str, offset: u64, limit: Option<u64>) -> Result<String, ToolError> {
        let (_, text) = self.read_text(raw)?;
        let skip = usize::try_from(offset.max(1) - 1).unwrap_or(usize::MAX);
        let take = limit
            .and_then(|l| usize::try_from(l).ok())
            .unwrap_or(usize::MAX);
        let mut out = String::new();
        for line in text.split_inclusive('\n').skip(skip).take(take) {
            if out.len() + line.len() > READ_MAX_BYTES {
                out.push_str("\n[… gekürzt: Datei zu groß, `offset`/`limit` nutzen …]\n");
                break;
            }
            out.push_str(line);
        }
        Ok(out)
    }

    fn write(&self, raw: &str, content: &str) -> Result<String, ToolError> {
        let target = self.resolve_for_write(raw)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| err(format!("io: {raw}: {e}")))?;
        }
        std::fs::write(&target, content).map_err(|e| err(format!("io: {raw}: {e}")))?;
        Ok(format!("{} Bytes nach {raw} geschrieben", content.len()))
    }

    fn edit(&self, raw: &str, old: &str, new: &str, all: bool) -> Result<String, ToolError> {
        if old.is_empty() {
            return Err(err("invalid_args: `old_string` ist leer"));
        }
        let (path, text) = self.read_text(raw)?;
        // Schreibziel erneut prüfen (z. B. Datei unter einem Symlink).
        let path = self.resolve_for_write(&self.relative(&path))?;
        let count = text.matches(old).count();
        let updated = match (count, all) {
            (0, _) => {
                return Err(err(format!(
                    "not_found: `old_string` kommt in {raw} nicht vor"
                )));
            }
            (1, _) | (_, true) => text.replace(old, new),
            (n, false) => {
                return Err(err(format!(
                    "ambiguous: `old_string` kommt {n}-mal in {raw} vor; eindeutiger wählen oder `replace_all` setzen"
                )));
            }
        };
        std::fs::write(&path, updated).map_err(|e| err(format!("io: {raw}: {e}")))?;
        Ok(format!("{count} Stelle(n) in {raw} ersetzt"))
    }

    fn walker(&self, glob: Option<&str>) -> Result<ignore::Walk, ToolError> {
        let mut builder = ignore::WalkBuilder::new(&self.root);
        builder
            .follow_links(false)
            .hidden(true)
            .sort_by_file_name(std::cmp::Ord::cmp);
        if let Some(g) = glob {
            let mut o = ignore::overrides::OverrideBuilder::new(&self.root);
            o.add(g)
                .map_err(|e| err(format!("invalid_args: Glob `{g}`: {e}")))?;
            builder.overrides(
                o.build()
                    .map_err(|e| err(format!("invalid_args: Glob `{g}`: {e}")))?,
            );
        }
        Ok(builder.build())
    }

    fn glob(&self, pattern: &str) -> Result<String, ToolError> {
        let mut out = Vec::new();
        for entry in self.walker(Some(pattern))?.filter_map(Result::ok) {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            out.push(self.relative(entry.path()));
            if out.len() >= GLOB_MAX {
                out.push(format!("[… mehr als {GLOB_MAX} Treffer …]"));
                break;
            }
        }
        Ok(if out.is_empty() {
            "keine Treffer".into()
        } else {
            out.join("\n")
        })
    }

    fn grep(
        &self,
        pattern: &str,
        glob: Option<&str>,
        insensitive: bool,
    ) -> Result<String, ToolError> {
        let re = regex::RegexBuilder::new(pattern)
            .case_insensitive(insensitive)
            .size_limit(1 << 20)
            .build()
            .map_err(|e| err(format!("invalid_args: Muster: {e}")))?;
        let mut hits = Vec::new();
        'files: for entry in self.walker(glob)?.filter_map(Result::ok) {
            if !entry.file_type().is_some_and(|t| t.is_file())
                || entry.metadata().is_ok_and(|m| m.len() > 2 * 1024 * 1024)
            {
                continue;
            }
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            if bytes.iter().take(8000).any(|b| *b == 0) {
                continue;
            }
            let rel = self.relative(entry.path());
            for (i, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
                if re.is_match(line) {
                    let text: String = line.chars().take(300).collect();
                    hits.push(format!("{rel}:{}: {text}", i + 1));
                    if hits.len() >= GREP_MAX {
                        hits.push(format!("[… mehr als {GREP_MAX} Treffer …]"));
                        break 'files;
                    }
                }
            }
        }
        Ok(if hits.is_empty() {
            "keine Treffer".into()
        } else {
            hits.join("\n")
        })
    }

    async fn shell(&self, command: &str, timeout: Duration) -> Result<String, ToolError> {
        if command.trim().is_empty() {
            return Err(err("invalid_args: `command` ist leer"));
        }
        #[cfg(unix)]
        let (program, args) = (
            PathBuf::from("/bin/sh"),
            vec!["-c".to_owned(), format!("exec 2>&1\n{command}")],
        );
        #[cfg(not(unix))]
        let (program, args) = (
            PathBuf::from("cmd"),
            vec!["/C".to_owned(), format!("{command} 2>&1")],
        );
        // Interne Variablen des Runners gehen nicht an Kommandos des Modells.
        let env_remove = std::env::vars_os()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .filter(|k| k.starts_with("BETON_"))
            .collect();
        let mut process = RealLauncher
            .launch(LaunchSpec {
                program,
                args,
                env: Vec::new(),
                env_remove,
                clear_env: false,
                cwd: Some(self.root.clone()),
            })
            .await
            .map_err(|e| err(format!("io: Shell nicht startbar: {e}")))?;
        let io = process
            .take_io()
            .ok_or_else(|| err("io: keine Ausgabe der Shell"))?;
        drop(io.stdin);
        let mut stdout = io.stdout;
        let read = async {
            let mut out = Vec::new();
            let mut buf = [0u8; 8192];
            let mut truncated = false;
            loop {
                match stdout.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let room = SHELL_MAX_BYTES.saturating_sub(out.len());
                        out.extend_from_slice(&buf[..n.min(room)]);
                        truncated |= n > room;
                    }
                }
            }
            (out, truncated)
        };
        match tokio::time::timeout(timeout, read).await {
            Ok((out, truncated)) => {
                let exit = process.wait().await.unwrap_or_default();
                let mut text = String::from_utf8_lossy(&out).into_owned();
                if truncated {
                    text.push_str("\n[… Ausgabe gekürzt …]");
                }
                let code = exit.code.unwrap_or(-1);
                let report = format!("exit_code: {code}\n{text}");
                if code == 0 {
                    Ok(report)
                } else {
                    Err(ToolError(report))
                }
            }
            Err(_) => {
                let _ = process.kill().await;
                Err(err(format!(
                    "timeout: Kommando nach {} s abgebrochen",
                    timeout.as_secs()
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn ws() -> (tempfile::TempDir, WorkspaceServer) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        let server = WorkspaceServer::new(dir.path()).unwrap();
        (dir, server)
    }

    #[tokio::test]
    async fn har_010_workspace_paths_stay_inside_the_workdir() {
        let (dir, s) = ws();
        for bad in [
            "/etc/passwd",
            "../x",
            "src/../../x",
            ".git/config",
            "a/.GIT/hooks/x",
        ] {
            let e = s.call("fs_read", &json!({"path": bad})).await.unwrap_err();
            assert!(e.0.starts_with("forbidden"), "{bad}: {e}");
            let e = s
                .call("fs_write", &json!({"path": bad, "content": "x"}))
                .await
                .unwrap_err();
            assert!(e.0.starts_with("forbidden"), "{bad}: {e}");
        }
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), dir.path().join("raus")).unwrap();
            let e = s
                .call("fs_write", &json!({"path": "raus/x.txt", "content": "x"}))
                .await
                .unwrap_err();
            assert!(e.0.starts_with("forbidden"), "{e}");
            assert!(!outside.path().join("x.txt").exists());
        }
    }

    #[tokio::test]
    async fn har_010_workspace_tools_read_write_edit_glob_grep() {
        let (dir, s) = ws();
        let text = s
            .call("fs_read", &json!({"path": "src/lib.rs"}))
            .await
            .unwrap();
        assert_eq!(text, "fn a() {}\nfn b() {}\n");
        let text = s
            .call(
                "fs_read",
                &json!({"path": "src/lib.rs", "offset": 2, "limit": 1}),
            )
            .await
            .unwrap();
        assert_eq!(text, "fn b() {}\n");
        s.call(
            "fs_write",
            &json!({"path": "neu/datei.txt", "content": "hallo"}),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("neu/datei.txt")).unwrap(),
            "hallo"
        );
        s.call(
            "fs_edit",
            &json!({"path": "src/lib.rs", "old_string": "fn b()", "new_string": "fn c()"}),
        )
        .await
        .unwrap();
        let e = s
            .call(
                "fs_edit",
                &json!({"path": "src/lib.rs", "old_string": "fn", "new_string": "pub fn"}),
            )
            .await
            .unwrap_err();
        assert!(e.0.starts_with("ambiguous"), "{e}");
        assert_eq!(
            s.call("fs_glob", &json!({"pattern": "**/*.rs"}))
                .await
                .unwrap(),
            "src/lib.rs"
        );
        assert_eq!(
            s.call("fs_grep", &json!({"pattern": "fn c"}))
                .await
                .unwrap(),
            "src/lib.rs:2: fn c() {}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn har_010_shell_exec_runs_in_the_workdir_with_exit_code() {
        let (_dir, s) = ws();
        let ok = s
            .call("shell_exec", &json!({"command": "ls src; echo fehler >&2"}))
            .await
            .unwrap();
        assert!(ok.starts_with("exit_code: 0\n"), "{ok}");
        assert!(ok.contains("lib.rs") && ok.contains("fehler"), "{ok}");
        let failed = s
            .call("shell_exec", &json!({"command": "exit 3"}))
            .await
            .unwrap_err();
        assert!(failed.0.starts_with("exit_code: 3"), "{failed}");
        let slow = s
            .call(
                "shell_exec",
                &json!({"command": "sleep 30", "timeout_s": 1}),
            )
            .await
            .unwrap_err();
        assert!(slow.0.starts_with("timeout"), "{slow}");
    }

    #[tokio::test]
    async fn har_010_workspace_server_speaks_mcp() {
        let (_dir, s) = ws();
        let init = s
            .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}))
            .await
            .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], WORKSPACE_SERVER);
        let list = s
            .handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
            .await
            .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert_eq!(names, TOOLS);
        let call = s
            .handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "fs_read", "arguments": {"path": "../x"}}}))
            .await
            .unwrap();
        assert_eq!(call["result"]["isError"], true);
        assert_eq!(tool_kind("shell_exec"), "shell");
    }
}
