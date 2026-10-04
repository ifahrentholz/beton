//! Rust-Client-SDK für die beton-API (API-005).
//!
//! CLI und TUI greifen ausschließlich über dieses Crate auf den Server zu. Der Umfang wächst
//! mit den Arbeitspaketen; heute: Daemon-Erkennung, Info, Session-Liste, Einmal-Codes.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Standard-Adresse des lokalen Daemons (AUTH-001).
pub const DEFAULT_LOCAL_ADDR: &str = "127.0.0.1:7420";

/// Laufzeitdatei des lokalen Daemons unter `<data_dir>/run/daemon.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    /// Erste gebundene TCP-Adresse, z. B. `127.0.0.1:7420`.
    pub http: String,
    pub version: String,
}

impl DaemonInfo {
    pub fn path_in(data_dir: &Path) -> PathBuf {
        data_dir.join("run").join("daemon.json")
    }

    pub fn read(data_dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(Self::path_in(data_dir)).ok()?;
        serde_json::from_str(&text).ok()
    }
}

/// Pfad der lokalen Token-Datei (AUTH-001).
pub fn local_token_path(data_dir: &Path) -> PathBuf {
    data_dir.join("auth").join("local.token")
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Server nicht erreichbar (CLI-Exit-Code 7).
    #[error("Server unter {url} nicht erreichbar: {reason}")]
    Unreachable { url: String, reason: String },
    /// Fehlerantwort als RFC-9457-Problem.
    #[error("{title} ({status}){}", detail.as_deref().map(|d| format!(": {d}")).unwrap_or_default())]
    Problem {
        status: u16,
        code: String,
        title: String,
        detail: Option<String>,
    },
    #[error("lokales Token nicht lesbar ({path}): {reason}")]
    Token { path: String, reason: String },
    #[error("Antwort nicht lesbar: {0}")]
    Decode(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Eine Seite einer Liste (PROTO-010).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Einmal-Code für die Browser-Anmeldung (AUTH-004).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginCode {
    pub code: String,
    pub redeem_url: String,
    pub expires_in_s: u64,
}

/// Client für einen beton-Server.
#[derive(Clone)]
pub struct Client {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Client für `base` (z. B. `http://127.0.0.1:7420`) mit Bearer-Token.
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            // Lokale Verbindungen gehen nie über einen Proxy.
            .no_proxy()
            .build()
            .map_err(|e| Error::Decode(e.to_string()))?;
        Ok(Self {
            base: base.into().trim_end_matches('/').to_owned(),
            token: token.into(),
            http,
        })
    }

    /// Client für den lokalen Daemon: Adresse aus `run/daemon.json` (sonst Port 7420), Token
    /// aus `auth/local.token`.
    pub fn local(data_dir: &Path) -> Result<Self> {
        let addr = DaemonInfo::read(data_dir)
            .map(|d| d.http)
            .unwrap_or_else(|| DEFAULT_LOCAL_ADDR.to_owned());
        let path = local_token_path(data_dir);
        let token = std::fs::read_to_string(&path).map_err(|e| Error::Token {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        Self::new(format!("http://{addr}"), token.trim())
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// `GET /v1/info`.
    pub async fn info(&self) -> Result<Value> {
        self.get("/v1/info").await
    }

    /// `GET /v1/sessions` (eine Seite).
    pub async fn sessions(
        &self,
        limit: Option<u32>,
        cursor: Option<&str>,
        include_archived: bool,
    ) -> Result<Page<Value>> {
        let mut query = Vec::new();
        if let Some(l) = limit {
            query.push(("limit", l.to_string()));
        }
        if let Some(c) = cursor {
            query.push(("cursor", c.to_owned()));
        }
        if include_archived {
            query.push(("include_archived", "true".to_owned()));
        }
        self.send(self.http.get(self.url("/v1/sessions")).query(&query))
            .await
    }

    /// Alle Sessions über alle Seiten.
    pub async fn all_sessions(&self, include_archived: bool) -> Result<Vec<Value>> {
        let mut items = Vec::new();
        let mut cursor = None;
        loop {
            let page = self
                .sessions(Some(200), cursor.as_deref(), include_archived)
                .await?;
            items.extend(page.items);
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => return Ok(items),
            }
        }
    }

    /// `POST /v1/sessions`: Session anlegen und Runner starten (SES-001).
    pub async fn create_session(&self, request: &Value) -> Result<Value> {
        self.send(self.http.post(self.url("/v1/sessions")).json(request))
            .await
    }

    /// `POST /v1/auth/local/codes`: Einmal-Code für den Browser (AUTH-004).
    pub async fn create_login_code(&self) -> Result<LoginCode> {
        self.send(self.http.post(self.url("/v1/auth/local/codes")))
            .await
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.send(self.http.get(self.url(path))).await
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn send<T: serde::de::DeserializeOwned>(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<T> {
        let res = req
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| Error::Unreachable {
                url: self.base.clone(),
                reason: root_cause(&e),
            })?;
        let status = res.status();
        let bytes = res.bytes().await.map_err(|e| Error::Unreachable {
            url: self.base.clone(),
            reason: root_cause(&e),
        })?;
        if !status.is_success() {
            let problem: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let text = |k: &str| problem.get(k).and_then(Value::as_str).map(str::to_owned);
            return Err(Error::Problem {
                status: status.as_u16(),
                code: text("code").unwrap_or_else(|| "unknown".into()),
                title: text("title")
                    .unwrap_or_else(|| status.canonical_reason().unwrap_or("Fehler").into()),
                detail: text("detail"),
            });
        }
        serde_json::from_slice(&bytes).map_err(|e| Error::Decode(e.to_string()))
    }
}

fn root_cause(e: &(dyn std::error::Error + 'static)) -> String {
    let mut cur = e;
    while let Some(next) = cur.source() {
        cur = next;
    }
    cur.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_info_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        assert!(DaemonInfo::read(dir.path()).is_none());
        let info = DaemonInfo {
            pid: 42,
            http: "127.0.0.1:1234".into(),
            version: "0.0.0".into(),
        };
        std::fs::create_dir_all(dir.path().join("run")).unwrap();
        std::fs::write(
            DaemonInfo::path_in(dir.path()),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();
        assert_eq!(DaemonInfo::read(dir.path()), Some(info));
    }

    #[tokio::test]
    async fn unreachable_server_is_reported_as_such() {
        // Port 9 (discard) ist auf Loopback praktisch nie offen.
        let c = Client::new("http://127.0.0.1:9", "x").unwrap();
        assert!(matches!(c.info().await, Err(Error::Unreachable { .. })));
    }

    #[test]
    fn missing_token_is_a_token_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            Client::local(dir.path()),
            Err(Error::Token { .. })
        ));
    }
}
