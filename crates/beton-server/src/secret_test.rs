//! `beton secrets test` (SEC-004, SEC-011 AC3): Prüft ein Git-Provider-Token beim Provider
//! über den Proxy-Pfad, ohne den Wert auszugeben.
//!
//! Der Daemon startet dafür eine eigene Proxy-Instanz (Stufe 0) mit genau einer Regel für die
//! API des Providers und einem Binding für das Secret. Sein HTTP-Client sendet nur einen
//! Platzhalter, vertraut nur der beton-CA, und der Proxy setzt den Wert ein (PRX-006): Der
//! Wert verlässt den Proxy nur zum gebundenen Host und kommt in keiner Antwort zurück
//! (PRX-010). Ausgewertet werden Status, `login` und die Scopes (GitHub `X-OAuth-Scopes`,
//! GitLab `scopes` des Tokens). Netzverkehr entsteht nur auf ausdrücklichen Aufruf
//! (ADR-0033: Git-Provider nur wenn konfiguriert bzw. angefragt).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use beton_proxy::credentials::BoxFuture;
use beton_proxy::{
    Binding, CredentialAudit, CredentialKind, CredentialSource, Credentials, EgressProxy,
    InstanceSpec, Placeholder, ProxyOptions, RuleSet, SecretBytes,
};
use beton_secrets::{ProxyCap, SecretRef, UseContext};
use serde_json::{Value, json};

use crate::credentials::{CredentialService, Source};
use crate::problem::{Problem, ProblemCode};

/// Frist der Prüfung beim Provider.
pub const TEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Art des Providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Github,
    Gitlab,
}

/// Was geprüft wird: Host der API, Pfad und Header mit dem Platzhalter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub kind: ProviderKind,
    pub api_host: String,
    pub path: &'static str,
}

impl Probe {
    /// Aus Provider der SecretRef (`github.com`, `gitlab.com`, GHE- bzw. GitLab-Host) und
    /// optionaler Art.
    pub fn new(provider: &str, kind: Option<&str>) -> Result<Self, Problem> {
        let kind = match kind {
            Some("github") => ProviderKind::Github,
            Some("gitlab") => ProviderKind::Gitlab,
            Some(other) => {
                return Err(Problem::new(ProblemCode::ValidationFailed)
                    .detail(format!("unbekannte Art `{other}` (github oder gitlab)")));
            }
            None if provider.contains("github") => ProviderKind::Github,
            None if provider.contains("gitlab") => ProviderKind::Gitlab,
            None => {
                return Err(Problem::new(ProblemCode::ValidationFailed).detail(
                    "Art des Providers unbekannt; --kind github oder --kind gitlab angeben",
                ));
            }
        };
        let valid = provider.contains('.')
            && provider
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
        if !valid {
            return Err(Problem::new(ProblemCode::ValidationFailed)
                .detail("Der Provider der SecretRef muss ein Host sein, z. B. github.com"));
        }
        Ok(match (kind, provider) {
            (ProviderKind::Github, "github.com") => Self {
                kind,
                api_host: "api.github.com".into(),
                path: "/user",
            },
            (ProviderKind::Github, host) => Self {
                kind,
                api_host: host.into(),
                path: "/api/v3/user",
            },
            (ProviderKind::Gitlab, host) => Self {
                kind,
                api_host: host.into(),
                path: "/api/v4/personal_access_tokens/self",
            },
        })
    }
}

/// Fehlende Scopes für Git und API (`None`: nicht prüfbar, z. B. fine-grained PAT).
pub fn missing_scopes(kind: ProviderKind, scopes: Option<&[String]>) -> Vec<String> {
    let Some(scopes) = scopes else {
        return Vec::new();
    };
    let has = |s: &str| scopes.iter().any(|x| x == s);
    match kind {
        ProviderKind::Github if !has("repo") => vec!["repo".into()],
        ProviderKind::Gitlab if !(has("api") || (has("read_api") && has("write_repository"))) => {
            vec!["api".into()]
        }
        _ => Vec::new(),
    }
}

struct OneValue(Arc<tokio::sync::Mutex<Option<SecretBytes>>>);

impl CredentialSource for OneValue {
    fn value<'a>(&'a self, _binding: &'a str) -> BoxFuture<'a, Result<SecretBytes, String>> {
        Box::pin(async move {
            self.0
                .lock()
                .await
                .take()
                .ok_or_else(|| "Wert schon verwendet".to_owned())
        })
    }
}

/// Die Nutzung auditiert schon der Abruf über die Proxy-Capability (SEC-001 AC3).
struct NoAudit;

impl CredentialAudit for NoAudit {
    fn used<'a>(&'a self, _b: &'a str, _h: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn misuse<'a>(&'a self, _b: Option<&'a str>, _h: &'a str, _r: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// Prüfung von Secrets über den Proxy-Pfad des Daemons.
pub struct SecretTester {
    cas: Option<(beton_proxy::CaStore, Arc<ProxyCap>, String)>,
    extra_ca_files: Vec<PathBuf>,
    proxy: tokio::sync::OnceCell<EgressProxy>,
}

impl std::fmt::Debug for SecretTester {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretTester")
    }
}

impl SecretTester {
    /// Mit der beton-CA aus dem Secret-Store (nur über die Proxy-Capability).
    pub fn new(
        cas: beton_proxy::CaStore,
        cap: Arc<ProxyCap>,
        host_id: String,
        extra_ca_files: Vec<PathBuf>,
    ) -> Self {
        Self {
            cas: Some((cas, cap, host_id)),
            extra_ca_files,
            proxy: tokio::sync::OnceCell::new(),
        }
    }

    /// Mit einem fertigen Proxy (Tests: Test-Resolver und lokale Upstreams, ADR-0033).
    pub fn with_proxy(proxy: EgressProxy) -> Self {
        let cell = tokio::sync::OnceCell::new();
        let _ = cell.set(proxy);
        Self {
            cas: None,
            extra_ca_files: Vec::new(),
            proxy: cell,
        }
    }

    async fn proxy(&self) -> Result<&EgressProxy, Problem> {
        self.proxy
            .get_or_try_init(|| async {
                let (cas, cap, host_id) = self.cas.as_ref().ok_or_else(|| {
                    Problem::new(ProblemCode::SandboxUnavailable).detail("keine beton-CA")
                })?;
                let ca = cas
                    .load_or_create(
                        cap,
                        UseContext {
                            session_id: None,
                            host_id: host_id.clone(),
                        },
                    )
                    .await
                    .map_err(|e| {
                        Problem::new(ProblemCode::SandboxUnavailable)
                            .detail(format!("beton-CA: {e}"))
                    })?;
                EgressProxy::new(
                    ca,
                    &ProxyOptions {
                        extra_ca_files: self.extra_ca_files.clone(),
                    },
                )
                .map_err(|e| Problem::new(ProblemCode::SandboxUnavailable).detail(e.to_string()))
            })
            .await
    }

    /// Prüft `secret_ref` beim Provider; das Ergebnis enthält nie den Wert.
    pub async fn test(
        &self,
        credentials: &CredentialService,
        env: &BTreeMap<String, String>,
        secret_ref: &str,
        kind: Option<&str>,
    ) -> Result<Value, Problem> {
        let r: SecretRef = secret_ref
            .parse()
            .map_err(|e: beton_secrets::Error| Problem::from(e))?;
        let probe = Probe::new(&r.provider, kind)?;
        let (value, _) = credentials
            .value(None, &Source::Secret(r.to_string()), env)
            .await?;
        let proxy = self.proxy().await?;
        let placeholder = Placeholder::generate().map_err(|e| Problem::internal(&e))?;
        let binding = Binding {
            name: "secrets-test".into(),
            kind: match probe.kind {
                ProviderKind::Github => CredentialKind::Github,
                ProviderKind::Gitlab => CredentialKind::Gitlab,
            },
            hosts: vec![probe.api_host.clone()],
            username: None,
            placeholder: placeholder.clone(),
            implicit: false,
        };
        let source = OneValue(Arc::new(tokio::sync::Mutex::new(Some(SecretBytes::new(
            value.to_vec(),
        )))));
        drop(value);
        let instance = proxy
            .start(InstanceSpec {
                label: "secrets-test".into(),
                rules: RuleSet::compile(&[format!("GET {}{}", probe.api_host, probe.path)])
                    .map_err(|e| Problem::internal(&e))?,
                credentials: Credentials::new(vec![binding], Arc::new(source), Arc::new(NoAudit))
                    .map_err(|e| Problem::internal(&e))?,
                ..InstanceSpec::default()
            })
            .await
            .map_err(|e| Problem::internal(&e))?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .proxy(
                reqwest::Proxy::all(instance.proxy_url().as_str())
                    .map_err(|_| Problem::internal(&"Proxy-URL"))?,
            )
            .tls_built_in_root_certs(false)
            .add_root_certificate(
                reqwest::Certificate::from_pem(proxy.ca_pem().as_bytes())
                    .map_err(|_| Problem::internal(&"beton-CA"))?,
            )
            .timeout(TEST_TIMEOUT)
            .build()
            .map_err(|e| Problem::internal(&e.without_url()))?;
        let url = format!("https://{}{}", probe.api_host, probe.path);
        let req = match probe.kind {
            ProviderKind::Github => client
                .get(&url)
                .header("authorization", format!("token {placeholder}"))
                .header("accept", "application/vnd.github+json"),
            ProviderKind::Gitlab => client
                .get(&url)
                .header("private-token", placeholder.as_str()),
        };
        let res = req.send().await.map_err(|e| {
            Problem::new(ProblemCode::Unavailable)
                .detail(format!("Provider nicht erreichbar: {}", e.without_url()))
        })?;
        let status = res.status().as_u16();
        let header_scopes: Option<Vec<String>> = res
            .headers()
            .get("x-oauth-scopes")
            .and_then(|v| v.to_str().ok())
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect()
            });
        let body: Value = res.json().await.unwrap_or(Value::Null);
        drop(instance);
        let scopes = match probe.kind {
            ProviderKind::Github => header_scopes,
            ProviderKind::Gitlab => body["scopes"].as_array().map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            }),
        };
        let missing = if status == 200 {
            missing_scopes(probe.kind, scopes.as_deref())
        } else {
            Vec::new()
        };
        let hint = match status {
            200 if scopes.is_none() => Some(
                "Scopes nicht prüfbar (z. B. fine-grained PAT); Berechtigungen beim Provider prüfen",
            ),
            200 if !missing.is_empty() => Some("Token mit den fehlenden Scopes neu ausstellen"),
            401 | 403 => Some("Token ungültig, abgelaufen oder ohne Zugriff"),
            200 => None,
            _ => Some("Unerwartete Antwort des Providers"),
        };
        Ok(json!({
            "secret_ref": r.to_string(),
            "provider": probe.api_host,
            "kind": match probe.kind { ProviderKind::Github => "github", ProviderKind::Gitlab => "gitlab" },
            "status": status,
            "ok": status == 200 && missing.is_empty(),
            "login": body.get("login").or_else(|| body.get("name")).cloned(),
            "scopes": scopes,
            "missing": missing,
            "hint": hint,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec_011_probes_and_scopes() {
        let gh = Probe::new("github.com", None).unwrap();
        assert_eq!((gh.api_host.as_str(), gh.path), ("api.github.com", "/user"));
        let ghe = Probe::new("ghe.example.test", Some("github")).unwrap();
        assert_eq!(
            (ghe.api_host.as_str(), ghe.path),
            ("ghe.example.test", "/api/v3/user")
        );
        let gl = Probe::new("gitlab.example.test", None).unwrap();
        assert_eq!(gl.path, "/api/v4/personal_access_tokens/self");
        assert!(Probe::new("npm", None).is_err());
        assert!(Probe::new("x/y", Some("github")).is_err());
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            missing_scopes(ProviderKind::Github, Some(&s(&["read:org"]))),
            ["repo"]
        );
        assert!(missing_scopes(ProviderKind::Github, Some(&s(&["repo", "read:org"]))).is_empty());
        assert!(missing_scopes(ProviderKind::Github, None).is_empty());
        assert_eq!(
            missing_scopes(ProviderKind::Gitlab, Some(&s(&["read_api"]))),
            ["api"]
        );
        assert!(
            missing_scopes(
                ProviderKind::Gitlab,
                Some(&s(&["read_api", "write_repository"]))
            )
            .is_empty()
        );
    }
}
