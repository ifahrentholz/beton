//! Credentials der Sessions (PRX-006, SEC-001, SEC-011, SEC-012): Werte für die
//! Proxy-Instanzen der Runner und das Audit ihrer Nutzung.
//!
//! Der Runner fragt über den Tunnel (`system.call`) nach dem Wert eines Bindings seiner
//! Session (`credential.resolve`). Der Daemon sucht das Binding in **seiner** Konfiguration
//! (`sandbox.credentials` der User-Konfiguration, `harnesses.<id>` mit `auth: api_key`,
//! `providers.<name>` des Direkt-API-Harness) und liest die Quelle in Stufe 0:
//! - `secret`: Secret-Store, nur über die Proxy-Capability (SEC-001 AC2, auditiert);
//! - `env`: Umgebung des Daemons beim Start;
//! - `file`: Datei (die Lage außerhalb aller Freigaben prüft der Runner, PRX-006 AC4; hier
//!   nochmals Größe und Hardlinks);
//! - `command`: Kommando ohne Shell, Timeout 30 s, stdout ohne Leerraum am Rand, mit `ttl`.
//!
//! Jede Nutzung (`credential.used`) zählt im 5-min-Fenster je Binding, Session und Host;
//! Fehlgebrauch (`credential.misuse`) ist ein eigener Eintrag. Werte erscheinen nie in
//! Audit, Logs oder Fehlermeldungen.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use beton_core::id::{PrincipalId, SessionId};
use beton_core::time::Timestamp;
use beton_secrets::{ProxyCap, SecretKind, SecretRef, Secrets, UseContext};
use beton_store::{LocalIdentity, NewAuditEntry, Store, UsageKey};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::problem::{Problem, ProblemCode};
use crate::sessions::SessionsConfig;

/// Gültigkeit eines Werts aus dem Secret-Store im Runner (danach neu, z. B. nach Rotation).
pub const SECRET_TTL: Duration = Duration::from_secs(60);
/// Gültigkeit eines Kommando-Ergebnisses ohne `ttl`.
pub const COMMAND_TTL: Duration = Duration::from_secs(15 * 60);
/// Frist für `source.command`.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
/// Größter Wert aus Datei oder Kommando.
pub const MAX_VALUE: usize = 64 * 1024;

/// Präfix der Nutzungsfenster von Credentials im Audit (`audit_usage_windows.secret_ref`).
pub use beton_store::audit::CREDENTIAL_PREFIX;

/// Quelle eines Bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Secret(String),
    Env(String),
    File(String),
    Command { argv: Vec<String>, ttl: Option<u64> },
}

fn failed(detail: impl Into<String>) -> Problem {
    Problem::new(ProblemCode::CredentialSourceError).detail(detail)
}

/// Werte und Audit der Credentials (Stufe 0).
pub struct CredentialService {
    secrets: Secrets,
    cap: Arc<ProxyCap>,
    host_id: String,
    store: Store,
    local: LocalIdentity,
}

impl std::fmt::Debug for CredentialService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialService")
            .field("host_id", &self.host_id)
            .finish_non_exhaustive()
    }
}

/// Die Quelle eines Bindings der Session aus der Konfiguration des Daemons; ein Binding, das
/// die Session nicht hat, gibt es nicht (fail closed).
pub fn find_source(cfg: &SessionsConfig, harness: &str, binding: &str) -> Result<Source, Problem> {
    let unknown = || {
        Problem::new(ProblemCode::Forbidden).detail(format!(
            "Binding `{binding}` gehört nicht zu dieser Session"
        ))
    };
    if let Some(id) = binding.strip_prefix("harness:") {
        // HAR-015 AC4: nur der eigene Harness, nur mit `auth: api_key`.
        if id != harness
            || cfg.harnesses_user.auth(id) != beton_harness::registry::HarnessAuth::ApiKey
        {
            return Err(unknown());
        }
        let entry = cfg.harnesses_user.entries.get(id);
        if let Some(r) = entry.and_then(|e| e.api_key.clone()) {
            return Ok(Source::Secret(r));
        }
        let (var, _) = beton_host::vendor_key(id).ok_or_else(unknown)?;
        return Ok(Source::Env(var.to_owned()));
    }
    if let Some(name) = binding.strip_prefix("provider:") {
        // HAR-011 AC2: der Provider des Direkt-API-Harness dieser Session.
        if harness != format!("direct:{name}") {
            return Err(unknown());
        }
        let p = cfg.providers.get(name).ok_or_else(unknown)?;
        if let Some(r) = p.get("api_key").and_then(Value::as_str)
            && r.starts_with("secret://")
        {
            return Ok(Source::Secret(r.to_owned()));
        }
        if let Some(var) = p.get("api_key_env").and_then(Value::as_str) {
            return Ok(Source::Env(var.to_owned()));
        }
        return Err(unknown());
    }
    let c = cfg
        .sandbox
        .credentials
        .iter()
        .flatten()
        .find(|c| c.name == binding)
        .ok_or_else(unknown)?;
    let s = &c.source;
    Ok(match (&s.secret, &s.env, &s.file, &s.command) {
        (Some(r), None, None, None) => Source::Secret(r.clone()),
        (None, Some(v), None, None) => Source::Env(v.clone()),
        (None, None, Some(f), None) => Source::File(f.clone()),
        (None, None, None, Some(argv)) => Source::Command {
            argv: argv.clone(),
            ttl: s.ttl_secs().map_err(|e| failed(e))?,
        },
        _ => return Err(failed("Binding ohne eindeutige Quelle")),
    })
}

/// Der geheime Teil eines Secrets: Token roh, bei `basic` das Passwort, bei `oauth` das
/// Access-Token.
fn secret_part(kind: SecretKind, value: &[u8]) -> Result<Zeroizing<Vec<u8>>, Problem> {
    let field = match kind {
        SecretKind::Token => return Ok(Zeroizing::new(value.to_vec())),
        SecretKind::Basic => "password",
        SecretKind::Oauth => "access_token",
    };
    let json: Zeroizing<Vec<u8>> = Zeroizing::new(value.to_vec());
    let parsed: Value = serde_json::from_slice(&json)
        .map_err(|_| failed(format!("{}-Secret ist kein JSON", kind.as_str())))?;
    let v = parsed
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| failed(format!("{}-Secret ohne `{field}`", kind.as_str())))?;
    Ok(Zeroizing::new(v.as_bytes().to_vec()))
}

fn trimmed(bytes: &[u8]) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(bytes.trim_ascii().to_vec())
}

impl CredentialService {
    pub fn new(
        secrets: Secrets,
        cap: Arc<ProxyCap>,
        host_id: String,
        store: Store,
        local: LocalIdentity,
    ) -> Self {
        Self {
            secrets,
            cap,
            host_id,
            store,
            local,
        }
    }

    /// Wert einer Quelle und seine Gültigkeit in Sekunden (`None`: für die Session).
    pub async fn value(
        &self,
        session: Option<SessionId>,
        source: &Source,
        env: &BTreeMap<String, String>,
    ) -> Result<(Zeroizing<Vec<u8>>, Option<u64>), Problem> {
        let (value, ttl) = match source {
            Source::Secret(raw) => {
                let r: SecretRef = raw
                    .parse()
                    .map_err(|e: beton_secrets::Error| Problem::from(e))?;
                let meta = self
                    .secrets
                    .list_meta()
                    .await?
                    .into_iter()
                    .find(|m| m.secret_ref == r.to_string())
                    .ok_or_else(|| failed(format!("Secret {r} gibt es nicht")))?;
                let v = self
                    .cap
                    .get_for_use(
                        &self.secrets,
                        &r,
                        UseContext {
                            session_id: session,
                            host_id: self.host_id.clone(),
                        },
                    )
                    .await?;
                (
                    secret_part(meta.kind, v.expose())?,
                    Some(SECRET_TTL.as_secs()),
                )
            }
            Source::Env(var) => {
                let v = env
                    .get(var)
                    .ok_or_else(|| failed(format!("Variable {var} ist im Daemon nicht gesetzt")))?;
                (trimmed(v.as_bytes()), None)
            }
            Source::File(raw) => (read_file(raw, env)?, None),
            Source::Command { argv, ttl } => (
                run_command(argv, env).await?,
                Some(ttl.unwrap_or(COMMAND_TTL.as_secs())),
            ),
        };
        if value.is_empty() {
            return Err(failed("Die Quelle lieferte einen leeren Wert"));
        }
        Ok((value, ttl))
    }

    /// `credential.resolve {binding}` des Runners einer Session.
    pub async fn resolve(
        &self,
        cfg: &SessionsConfig,
        session: SessionId,
        harness: &str,
        args: &Value,
    ) -> Result<Value, Problem> {
        let binding = text_arg(args, "binding")?;
        let source = find_source(cfg, harness, binding)?;
        let (value, ttl) = self.value(Some(session), &source, &cfg.daemon_env).await?;
        let encoded = Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(&*value));
        Ok(json!({ "value_b64": encoded.as_str(), "ttl_s": ttl }))
    }

    /// `credential.used {binding, host}`: eine Nutzung im 5-min-Fenster (PRX-006 AC6).
    pub async fn used(&self, session: SessionId, args: &Value) -> Result<Value, Problem> {
        let binding = text_arg(args, "binding")?;
        let host = text_arg(args, "host")?;
        self.store
            .record_audit_usage(
                self.local.org,
                &UsageKey {
                    secret_ref: format!("{CREDENTIAL_PREFIX}{binding}"),
                    session_id: Some(session),
                    host_id: host.to_owned(),
                    actor: PrincipalId::User(self.local.user),
                    purpose: "proxy".into(),
                },
                Timestamp::now(),
            )
            .await?;
        Ok(Value::Null)
    }

    /// `credential.misuse {binding?, host, reason}` (PRX-006 AC2).
    pub async fn misuse(&self, session: SessionId, args: &Value) -> Result<Value, Problem> {
        let host = text_arg(args, "host")?;
        let reason = text_arg(args, "reason")?;
        let binding = match args.get("binding") {
            Some(Value::String(_)) => Some(text_arg(args, "binding")?),
            _ => None,
        };
        self.store
            .append_audit(
                self.local.org,
                NewAuditEntry {
                    at: Timestamp::now(),
                    actor: PrincipalId::User(self.local.user),
                    action: "credential.misuse".into(),
                    target_kind: "credential".into(),
                    target_id: binding.unwrap_or("unknown").to_owned(),
                    session_id: Some(session),
                    host_id: Some(host.to_owned()),
                    details: json!({ "reason": reason, "binding": binding }),
                },
            )
            .await?;
        Ok(Value::Null)
    }
}

/// Kurzer Text ohne Steuerzeichen (Namen, Hosts, Gründe; nie Werte).
fn text_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, Problem> {
    match args.get(key).and_then(Value::as_str) {
        Some(s) if !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control) => Ok(s),
        _ => Err(Problem::new(ProblemCode::ValidationFailed).detail(format!("`{key}` fehlt"))),
    }
}

fn read_file(raw: &str, env: &BTreeMap<String, String>) -> Result<Zeroizing<Vec<u8>>, Problem> {
    let home = env.get("HOME").map(String::as_str).unwrap_or_default();
    let path = match raw.strip_prefix("~/") {
        Some(rest) if !home.is_empty() => Path::new(home).join(rest),
        _ => Path::new(raw).to_path_buf(),
    };
    let meta = std::fs::metadata(&path)
        .map_err(|e| failed(format!("{}: {}", path.display(), e.kind())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if meta.nlink() > 1 {
            return Err(failed(format!("{} hat weitere Hardlinks", path.display())));
        }
    }
    if !meta.is_file() || meta.len() > MAX_VALUE as u64 {
        return Err(failed(format!(
            "{} ist keine Datei oder größer als 64 KiB",
            path.display()
        )));
    }
    let bytes = Zeroizing::new(
        std::fs::read(&path).map_err(|e| failed(format!("{}: {}", path.display(), e.kind())))?,
    );
    Ok(trimmed(&bytes))
}

/// `source.command` in Stufe 0: argv ohne Shell, Environment des Daemons, Timeout 30 s.
#[expect(
    clippy::disallowed_methods,
    reason = "Credential-Quelle `command` (PRX-006) läuft als beton-eigener Prozess in Stufe 0"
)]
async fn run_command(
    argv: &[String],
    env: &BTreeMap<String, String>,
) -> Result<Zeroizing<Vec<u8>>, Problem> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| failed("leeres Kommando"))?;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .env_clear()
        .envs(env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| failed(format!("Kommando `{program}`: {}", e.kind())))?;
    let out = tokio::time::timeout(COMMAND_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| failed(format!("Kommando `{program}`: Zeitüberschreitung (30 s)")))?
        .map_err(|e| failed(format!("Kommando `{program}`: {}", e.kind())))?;
    let stdout = Zeroizing::new(out.stdout);
    if !out.status.success() {
        return Err(failed(format!(
            "Kommando `{program}` endete mit {}",
            out.status
        )));
    }
    if stdout.len() > MAX_VALUE {
        return Err(failed(format!(
            "Kommando `{program}`: Ausgabe größer als 64 KiB"
        )));
    }
    Ok(trimmed(&stdout))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use beton_secrets::{NewSecret, SecretValue};
    use beton_store::StoreOptions;

    struct Fx {
        _dir: tempfile::TempDir,
        service: CredentialService,
        secrets: Secrets,
        store: Store,
        local: LocalIdentity,
        cfg: SessionsConfig,
        env: BTreeMap<String, String>,
        root: std::path::PathBuf,
    }

    async fn fx(sandbox: &str) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let store = Store::open(&root.join("data"), StoreOptions::default())
            .await
            .unwrap();
        let local = store.ensure_local().await.unwrap();
        let (secrets, caps) = Secrets::new(
            store.clone(),
            local,
            Arc::new(beton_secrets::backend::MemoryBackend::default()),
        );
        let service = CredentialService::new(
            secrets.clone(),
            Arc::new(caps.proxy),
            "hst_test".into(),
            store.clone(),
            local,
        );
        let mut cfg = crate::app::Runtime::new(tokio::sync::watch::channel(false).1).sessions;
        cfg.sandbox = beton_sandbox::SandboxSpec::from_yaml(sandbox).unwrap();
        let env = BTreeMap::from([
            ("HOME".to_owned(), root.display().to_string()),
            ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
            ("GH_PAT".to_owned(), " ghp_FIXTURE-ENV\n".to_owned()),
        ]);
        cfg.daemon_env = Arc::new(env.clone());
        Fx {
            _dir: dir,
            service,
            secrets,
            store,
            local,
            cfg,
            env,
            root,
        }
    }

    fn value(v: &Value) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(v["value_b64"].as_str().unwrap())
            .unwrap()
    }

    #[tokio::test]
    async fn prx_006_only_bindings_of_the_session_are_resolved() {
        let f = fx("credentials:\n  - { name: gh, type: github, source: { env: GH_PAT } }\n").await;
        let s = SessionId::new();
        let ok = f
            .service
            .resolve(&f.cfg, s, "claude", &json!({"binding": "gh"}))
            .await
            .unwrap();
        assert_eq!(value(&ok), b"ghp_FIXTURE-ENV");
        assert!(ok["ttl_s"].is_null());
        for binding in ["andere", "harness:claude", "harness:codex", "provider:x"] {
            let err = f
                .service
                .resolve(&f.cfg, s, "claude", &json!({ "binding": binding }))
                .await
                .unwrap_err();
            assert_eq!(err.code, ProblemCode::Forbidden, "{binding}");
        }
        // Fehlertexte nennen nie den Wert.
        let mut cfg = f.cfg.clone();
        cfg.daemon_env = Arc::new(BTreeMap::new());
        let err = f
            .service
            .resolve(&cfg, s, "claude", &json!({"binding": "gh"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, ProblemCode::CredentialSourceError);
        assert!(!format!("{err:?}").contains("FIXTURE"));
    }

    #[tokio::test]
    async fn prx_006_secret_file_and_command_sources() {
        let f = fx("{}\n").await;
        let r: SecretRef = "secret://user/github.com/me".parse().unwrap();
        f.secrets
            .put(NewSecret {
                secret_ref: r.clone(),
                kind: SecretKind::Token,
                value: SecretValue::from_bytes(b"ghp_FIXTURE-STORE".to_vec()),
            })
            .await
            .unwrap();
        let basic: SecretRef = "secret://user/registry.test/deploy".parse().unwrap();
        f.secrets
            .put(NewSecret {
                secret_ref: basic.clone(),
                kind: SecretKind::Basic,
                value: SecretValue::from_bytes(
                    br#"{"username":"deploy","password":"FIXTURE-PASS"}"#.to_vec(),
                ),
            })
            .await
            .unwrap();
        let s = Some(SessionId::new());
        let (v, ttl) = f
            .service
            .value(s, &Source::Secret(r.to_string()), &f.env)
            .await
            .unwrap();
        assert_eq!(&*v, b"ghp_FIXTURE-STORE");
        assert_eq!(ttl, Some(SECRET_TTL.as_secs()));
        let (v, _) = f
            .service
            .value(s, &Source::Secret(basic.to_string()), &f.env)
            .await
            .unwrap();
        assert_eq!(&*v, b"FIXTURE-PASS");
        // Datei unter `~`, ohne Leerraum am Rand; Hardlinks und zu große Dateien nicht.
        std::fs::write(f.root.join("token"), "FIXTURE-FILE\n").unwrap();
        let (v, ttl) = f
            .service
            .value(s, &Source::File("~/token".into()), &f.env)
            .await
            .unwrap();
        assert_eq!(&*v, b"FIXTURE-FILE");
        assert_eq!(ttl, None);
        std::fs::hard_link(f.root.join("token"), f.root.join("hard")).unwrap();
        assert!(
            f.service
                .value(s, &Source::File("~/token".into()), &f.env)
                .await
                .is_err()
        );
        // Kommando mit ttl; Fehler und leere Ausgabe sind Fehler, nie ein Wert.
        let cmd = |script: &str, ttl| Source::Command {
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
            ttl,
        };
        let (v, ttl) = f
            .service
            .value(s, &cmd("printf ' FIXTURE-CMD\\n'", Some(60)), &f.env)
            .await
            .unwrap();
        assert_eq!(&*v, b"FIXTURE-CMD");
        assert_eq!(ttl, Some(60));
        let (_, ttl) = f
            .service
            .value(s, &cmd("echo x", None), &f.env)
            .await
            .unwrap();
        assert_eq!(ttl, Some(COMMAND_TTL.as_secs()));
        for bad in ["exit 3", "printf ''", "echo FIXTURE-LEAK >&2; exit 1"] {
            let err = f
                .service
                .value(s, &cmd(bad, Some(60)), &f.env)
                .await
                .unwrap_err();
            assert_eq!(err.code, ProblemCode::CredentialSourceError, "{bad}");
            assert!(!format!("{err:?}").contains("FIXTURE"), "{err:?}");
        }
        // Die Nutzung über die Proxy-Capability steht im Audit (SEC-001 AC3), ohne Wert.
        f.secrets
            .seal_usage(Timestamp::from(
                Timestamp::now().as_offset_date_time() + time::Duration::minutes(6),
            ))
            .await
            .unwrap();
        let audit = f.store.audit_entries(f.local.org, 100).await.unwrap();
        assert!(audit.iter().any(|e| e.action == "secret.used"), "{audit:?}");
        assert!(!format!("{audit:?}").contains("FIXTURE"));
    }

    #[tokio::test]
    async fn prx_006_ac6_uses_and_misuse_are_audited_without_values() {
        let f = fx("{}\n").await;
        let s = SessionId::new();
        for _ in 0..3 {
            f.service
                .used(s, &json!({"binding": "gh", "host": "api.github.com"}))
                .await
                .unwrap();
        }
        f.service
            .misuse(
                s,
                &json!({"binding": "gh", "host": "evil.test", "reason": "host_mismatch"}),
            )
            .await
            .unwrap();
        f.service
            .misuse(
                s,
                &json!({"host": "evil.test", "reason": "unknown_placeholder"}),
            )
            .await
            .unwrap();
        assert!(
            f.service
                .used(s, &json!({"binding": "gh", "host": "a\nb"}))
                .await
                .is_err()
        );
        f.store
            .seal_audit_windows(
                f.local.org,
                Timestamp::from(
                    Timestamp::now().as_offset_date_time() + time::Duration::minutes(6),
                ),
            )
            .await
            .unwrap();
        let page = f
            .store
            .audit_page(
                f.local.org,
                &beton_store::AuditQuery {
                    target_kind: Some("credential".into()),
                    ..beton_store::AuditQuery::default()
                },
            )
            .await
            .unwrap();
        let actions: Vec<(&str, u64)> = page
            .items
            .iter()
            .map(|e| (e.action.as_str(), e.count))
            .collect();
        assert!(actions.contains(&("credential.used", 3)), "{actions:?}");
        assert_eq!(
            actions
                .iter()
                .filter(|(a, _)| *a == "credential.misuse")
                .count(),
            2
        );
        let misuse = page
            .items
            .iter()
            .find(|e| e.action == "credential.misuse" && e.target_id == "gh")
            .unwrap();
        assert_eq!(misuse.host_id.as_deref(), Some("evil.test"));
        assert_eq!(misuse.session_id, Some(s));
        assert_eq!(misuse.details["reason"], "host_mismatch");
    }
}
