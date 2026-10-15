//! Git und GitHub-API in Stufe 2 nur über den Platzhalter (PRX-006 AC1, SEC-011 AC1,
//! SBX-005 AC3) gegen das Backend dieser Plattform.
//!
//! Gegenstelle ist ein lokaler TLS-Upstream mit Test-CA, der sich als `github.com` (Git über
//! Smart-HTTP mit `git http-backend`) und `api.github.com` ausgibt; der Proxy bildet die Namen
//! über das Feature `test-net` auf ihn ab (ADR-0033, nie gegen github.com). Der Upstream nimmt
//! nur den echten Token an; die Sandbox sieht nur `bt_cred_*`. `gh` läuft nur unter Linux
//! (unter macOS prüft Go gegen den System-Verifier, PRX-008).

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use beton_proxy::credentials::BoxFuture;
use beton_sandbox::{
    BackendChoice, CommandSpec, ProxyEndpoint, Registry, ResolveCtx, SandboxProvider, SandboxSpec,
    Secret, Stage, Stdio,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const TOKEN: &str = "ghp_FIXTUREgitTOKEN0123456789abcdefABCD";

fn launcher() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_beton-sandbox-exec"))
}

fn provider() -> Arc<dyn SandboxProvider> {
    #[cfg(target_os = "linux")]
    let registry = Registry::with_launcher(&launcher());
    #[cfg(not(target_os = "linux"))]
    let registry = {
        let _ = launcher();
        Registry::platform()
    };
    let p = registry.select(BackendChoice::Auto).unwrap();
    assert!(p.probe().available, "{:?}", p.probe());
    p
}

/// Werkzeug aus dem `PATH` und die Wurzel, die die Sandbox lesen muss; fehlt es, scheitert
/// der Test in der CI.
fn tool(name: &str) -> Option<(PathBuf, PathBuf)> {
    let path = std::env::var("PATH").unwrap_or_default();
    let Some(found) = path
        .split(':')
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file())
    else {
        assert!(std::env::var_os("CI").is_none(), "{name} fehlt in der CI");
        eprintln!("{name} fehlt lokal; Fall übersprungen");
        return None;
    };
    let mut exe = found.canonicalize().unwrap();
    if cfg!(target_os = "macos") && exe.starts_with("/usr/bin") && name == "git" {
        let out = std::process::Command::new("/usr/bin/xcrun")
            .args(["-f", name])
            .output()
            .unwrap();
        exe = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
            .canonicalize()
            .unwrap();
        return Some((exe.clone(), exe.ancestors().nth(3).unwrap().to_path_buf()));
    }
    let prefix = exe.ancestors().nth(2).unwrap().to_path_buf();
    if ["/", "/usr", "/bin"].iter().any(|p| prefix == Path::new(p)) {
        return Some((exe, PathBuf::from("/usr/bin")));
    }
    let root = if prefix.starts_with("/opt/homebrew") {
        PathBuf::from("/opt/homebrew")
    } else if prefix.starts_with("/usr/local") {
        PathBuf::from("/usr/local")
    } else {
        prefix
    };
    Some((exe, root))
}

fn git(git: &Path, dir: &Path, args: &[&str]) {
    let out = std::process::Command::new(git)
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// Was der Upstream an `Authorization`-Headern gesehen hat.
type Seen = Arc<Mutex<Vec<String>>>;

struct Request {
    method: String,
    host: String,
    path: String,
    query: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

async fn read_request(
    tls: &mut (impl tokio::io::AsyncRead + Unpin),
    buf: &mut Vec<u8>,
) -> Option<Request> {
    let mut chunk = [0u8; 8192];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match tls.read(&mut chunk).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.trim().to_owned(), v.trim().to_owned()))
        })
        .collect();
    let mut rest = buf[end..].to_vec();
    buf.clear();
    let get = |n: &str| {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(n))
            .map(|(_, v)| v.clone())
    };
    let body = if get("transfer-encoding").is_some_and(|v| v.contains("chunked")) {
        // Chunked: bis zum Null-Chunk lesen.
        loop {
            if let Some(pos) = rest.windows(5).position(|w| w == b"0\r\n\r\n") {
                let raw = rest[..pos].to_vec();
                rest.drain(..pos + 5);
                *buf = rest;
                break decode_chunked(&raw);
            }
            match tls.read(&mut chunk).await {
                Ok(0) | Err(_) => return None,
                Ok(n) => rest.extend_from_slice(&chunk[..n]),
            }
        }
    } else {
        let len: usize = get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        while rest.len() < len {
            match tls.read(&mut chunk).await {
                Ok(0) | Err(_) => return None,
                Ok(n) => rest.extend_from_slice(&chunk[..n]),
            }
        }
        *buf = rest.split_off(len);
        rest
    };
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_owned(), q.to_owned()),
        None => (target, String::new()),
    };
    Some(Request {
        method,
        host: get("host").unwrap_or_default(),
        path,
        query,
        headers,
        body,
    })
}

fn decode_chunked(mut raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(pos) = raw.windows(2).position(|w| w == b"\r\n") {
        let size =
            usize::from_str_radix(String::from_utf8_lossy(&raw[..pos]).trim(), 16).unwrap_or(0);
        raw = &raw[pos + 2..];
        if size == 0 || raw.len() < size {
            break;
        }
        out.extend_from_slice(&raw[..size]);
        raw = &raw[(size + 2).min(raw.len())..];
    }
    out
}

/// Antwort `(status, header, body)`.
fn respond(req: &Request, repos: &Path, git_exe: &Path) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let auth = req.header("authorization").unwrap_or_default().to_owned();
    if req.host.starts_with("api.github.com") {
        let ok = auth == format!("Bearer {TOKEN}") || auth == format!("token {TOKEN}");
        if !ok {
            return (401, vec![], br#"{"message":"Bad credentials"}"#.to_vec());
        }
        let json = vec![("Content-Type".into(), "application/json".into())];
        return match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/user") => (200, json, br#"{"login":"me","id":1}"#.to_vec()),
            ("POST", "/graphql") => (
                200,
                json,
                br#"{"data":{"repository":{"pullRequests":{"totalCount":0,"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}"#
                    .to_vec(),
            ),
            _ => (404, json, br#"{"message":"Not Found"}"#.to_vec()),
        };
    }
    // github.com: Git über Smart-HTTP, nur mit x-access-token:<token>.
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{TOKEN}"))
    );
    if auth != expected {
        return (
            401,
            vec![("WWW-Authenticate".into(), "Basic realm=\"GitHub\"".into())],
            b"Authentifizierung erforderlich".to_vec(),
        );
    }
    let mut cmd = std::process::Command::new(git_exe);
    cmd.arg("http-backend")
        .env_clear()
        .env("GIT_PROJECT_ROOT", repos)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REMOTE_USER", "me")
        .env("REMOTE_ADDR", "127.0.0.1")
        .env("REQUEST_METHOD", &req.method)
        .env("PATH_INFO", &req.path)
        .env("QUERY_STRING", &req.query)
        .env(
            "CONTENT_TYPE",
            req.header("content-type").unwrap_or_default(),
        )
        .env("CONTENT_LENGTH", req.body.len().to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if let Some(enc) = req.header("content-encoding") {
        cmd.env("HTTP_CONTENT_ENCODING", enc);
    }
    let mut child = cmd.spawn().unwrap();
    {
        use std::io::Write as _;
        child.stdin.take().unwrap().write_all(&req.body).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let raw = out.stdout;
    let (sep, skip) = match raw.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(p) => (p, 4),
        None => (raw.windows(2).position(|w| w == b"\n\n").unwrap_or(0), 2),
    };
    let head = String::from_utf8_lossy(&raw[..sep]).into_owned();
    let mut status = 200;
    let mut headers = Vec::new();
    for l in head.lines() {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        if k.eq_ignore_ascii_case("status") {
            status = v.trim()[..3].parse().unwrap_or(500);
        } else {
            headers.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }
    (status, headers, raw[sep + skip..].to_vec())
}

async fn upstream(
    repos: PathBuf,
    git_exe: PathBuf,
    seen: Seen,
) -> (SocketAddr, rustls::pki_types::CertificateDer<'static>) {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, PKCS_ECDSA_P256_SHA256,
    };
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut ca = CertificateParams::default();
    ca.distinguished_name
        .push(DnType::CommonName, "beton git test upstream CA");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let leaf = CertificateParams::new(vec!["github.com".to_owned(), "api.github.com".to_owned()])
        .unwrap()
        .signed_by(&key, &Issuer::from_params(&ca, &ca_key))
        .unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone(), ca_cert.der().clone()],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
    )
    .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (acceptor, repos, git_exe, seen) = (
                acceptor.clone(),
                repos.clone(),
                git_exe.clone(),
                seen.clone(),
            );
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = Vec::new();
                while let Some(req) = read_request(&mut tls, &mut buf).await {
                    seen.lock().unwrap().push(format!(
                        "{} {}{} {}",
                        req.method,
                        req.host,
                        req.path,
                        req.header("authorization").unwrap_or("-")
                    ));
                    let (repos, git_exe) = (repos.clone(), git_exe.clone());
                    let (status, headers, body) =
                        tokio::task::spawn_blocking(move || respond(&req, &repos, &git_exe))
                            .await
                            .unwrap();
                    let mut res = format!("HTTP/1.1 {status} X\r\n");
                    for (k, v) in headers {
                        res.push_str(&format!("{k}: {v}\r\n"));
                    }
                    res.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
                    let mut bytes = res.into_bytes();
                    bytes.extend_from_slice(&body);
                    if tls.write_all(&bytes).await.is_err() {
                        return;
                    }
                    let _ = tls.flush().await;
                }
            });
        }
    });
    (addr, ca_cert.der().clone())
}

/// Quelle mit dem Fixture-Token.
struct Source;

impl beton_proxy::CredentialSource for Source {
    fn value<'a>(&'a self, _b: &'a str) -> BoxFuture<'a, Result<beton_proxy::SecretBytes, String>> {
        Box::pin(async { Ok(beton_proxy::SecretBytes::new(TOKEN.as_bytes().to_vec())) })
    }
}

#[derive(Default)]
struct Audit(Mutex<Vec<String>>);

impl beton_proxy::CredentialAudit for Audit {
    fn used<'a>(&'a self, binding: &'a str, host: &'a str) -> BoxFuture<'a, Result<(), String>> {
        self.0
            .lock()
            .unwrap()
            .push(format!("used {binding} {host}"));
        Box::pin(async { Ok(()) })
    }

    fn misuse<'a>(
        &'a self,
        binding: Option<&'a str>,
        host: &'a str,
        reason: &'a str,
    ) -> BoxFuture<'a, ()> {
        self.0
            .lock()
            .unwrap()
            .push(format!("misuse {binding:?} {host} {reason}"));
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prx_006_ac1_sec_011_ac1_git_clone_push_and_github_api_only_via_the_placeholder() {
    let Some((git_exe, git_root)) = tool("git") else {
        return;
    };
    let Some((curl, curl_root)) = tool("curl") else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let (home, ws, scratch, repos) = (
        root.join("home"),
        root.join("ws"),
        root.join("scratch"),
        root.join("repos"),
    );
    for d in [&home, &ws, &scratch, &repos.join("me"), &root.join("work")] {
        std::fs::create_dir_all(d).unwrap();
    }
    // Privates Repository mit einem Commit (bare, Smart-HTTP über `git http-backend`).
    let bare = repos.join("me/private.git");
    git(
        &git_exe,
        &root,
        &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    let work = root.join("work");
    git(&git_exe, &work, &["init", "-q", "-b", "main"]);
    std::fs::write(work.join("README"), "geheim\n").unwrap();
    git(&git_exe, &work, &["add", "README"]);
    git(&git_exe, &work, &["commit", "-qm", "eins"]);
    git(
        &git_exe,
        &work,
        &["push", "-q", bare.to_str().unwrap(), "main"],
    );

    let seen = Seen::default();
    let (addr, upstream_ca) = upstream(repos.clone(), git_exe.clone(), seen.clone()).await;
    let ca = beton_proxy::BetonCa::generate("git-test", time::OffsetDateTime::now_utc()).unwrap();
    let proxy = beton_proxy::EgressProxy::with_test_hosts(
        ca,
        vec![upstream_ca],
        &[("github.com", addr), ("api.github.com", addr)],
    );
    let bundle = root.join("bundle.pem");
    std::fs::write(&bundle, proxy.ca_bundle_pem()).unwrap();
    let placeholder = beton_proxy::Placeholder::generate().unwrap();
    let audit = Arc::new(Audit::default());
    let credentials = beton_proxy::Credentials::new(
        vec![beton_proxy::Binding {
            name: "github".into(),
            kind: beton_proxy::CredentialKind::Github,
            hosts: Vec::new(),
            username: None,
            placeholder: placeholder.clone(),
            implicit: true,
        }],
        Arc::new(Source),
        audit.clone(),
    )
    .unwrap();
    let instance = proxy
        .start(beton_proxy::InstanceSpec {
            label: "ses_git/tools".into(),
            rules: beton_proxy::RuleSet::compile(&["* github.com/**", "* api.github.com/**"])
                .unwrap(),
            credentials,
            ..beton_proxy::InstanceSpec::default()
        })
        .await
        .unwrap();

    let mut ctx = ResolveCtx::new(Stage::Tools, &ws, &home, &scratch);
    ctx.adapters = beton_sandbox::masks::builtin_harness_grants();
    ctx.host_env = [("PATH", "/usr/bin:/bin"), ("LANG", "C")]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    ctx.proxy = Some(ProxyEndpoint {
        port: instance.addr().port(),
        url: Some(Secret::new(instance.proxy_url().to_string())),
        ca_bundle: Some(bundle.clone()),
    });
    ctx.credential_env = [
        ("GH_TOKEN", placeholder.as_str()),
        ("GITHUB_TOKEN", placeholder.as_str()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let gh = if cfg!(target_os = "linux") {
        tool("gh")
    } else {
        None
    };
    let mut reads: Vec<String> = [&git_root, &curl_root]
        .into_iter()
        .filter(|r| !r.starts_with("/usr/bin"))
        .map(|r| r.display().to_string())
        .collect();
    if let Some((_, r)) = &gh
        && !r.starts_with("/usr/bin")
    {
        reads.push(r.display().to_string());
    }
    let mut spec = SandboxSpec::default();
    spec.read_paths = Some(reads);
    let policy = provider().resolve(&spec, &ctx).unwrap();
    assert_eq!(policy.env["GH_TOKEN"], placeholder.as_str());

    let path_dirs: Vec<String> = [&git_exe, &curl]
        .into_iter()
        .chain(gh.as_ref().map(|(e, _)| e))
        .map(|e| e.parent().unwrap().display().to_string())
        .collect();
    let gh_part = match &gh {
        Some((gh, _)) => format!(
            "export GH_CONFIG_DIR=\"$TMPDIR/gh\" GH_NO_UPDATE_NOTIFIER=1 GH_PROMPT_DISABLED=1; \
             echo \"gh-user=$({} api user --jq .login)\"; \
             {} pr list -R me/private --json number && echo gh-pr-ok; ",
            gh.display(),
            gh.display()
        ),
        None => String::new(),
    };
    let script = format!(
        "set -e; export PATH=\"{path}:$PATH\" GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null \
         GIT_TERMINAL_PROMPT=0 GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t \
         GIT_COMMITTER_EMAIL=t@t; \
         echo \"token=$GH_TOKEN\"; \
         git clone -q https://github.com/me/private.git repo; cat repo/README; \
         cd repo && git commit -q --allow-empty -m zwei && git push -q origin HEAD:main && echo push-ok; \
         curl -sS --max-time 20 -H \"Authorization: Bearer $GH_TOKEN\" https://api.github.com/user; echo; \
         {gh_part}\
         env | grep -c ghp_ || true",
        path = path_dirs.join(":"),
    );
    let cmd = CommandSpec::new("/bin/sh")
        .args(["-c", &script])
        .cwd(&ws)
        .stdio(Stdio::Null, Stdio::Piped, Stdio::Piped);
    let out = provider()
        .spawn(&policy, cmd)
        .unwrap()
        .output(Duration::from_secs(180))
        .await
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{stdout}\n{stderr}");
    // Die Sandbox sieht nur den Platzhalter …
    assert!(stdout.contains(&format!("token={placeholder}")), "{stdout}");
    assert!(
        !stdout.contains(TOKEN) && !stderr.contains(TOKEN),
        "{stdout}{stderr}"
    );
    assert!(
        stdout.trim_end().ends_with('0'),
        "kein ghp_ im Env: {stdout}"
    );
    // … und trotzdem gelingen clone, push und API-Aufrufe.
    assert!(stdout.contains("geheim"), "{stdout}");
    assert!(stdout.contains("push-ok"), "{stdout}");
    assert!(stdout.contains("\"login\":\"me\""), "{stdout}");
    if gh.is_some() {
        assert!(stdout.contains("gh-user=me"), "{stdout}{stderr}");
        assert!(stdout.contains("gh-pr-ok"), "{stdout}{stderr}");
    }
    // Der Commit ist im Repository angekommen.
    let log = std::process::Command::new(&git_exe)
        .args(["log", "--oneline", "main"])
        .current_dir(&bare)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&log.stdout).contains("zwei"));
    // Kein Platzhalter hat den Upstream erreicht; jede Nutzung ist auditiert.
    let seen = seen.lock().unwrap().clone();
    assert!(!seen.iter().any(|s| s.contains("bt_cred_")), "{seen:?}");
    let audit = audit.0.lock().unwrap().clone();
    assert!(
        audit.iter().any(|a| a == "used github github.com"),
        "{audit:?}"
    );
    assert!(
        audit.iter().any(|a| a == "used github api.github.com"),
        "{audit:?}"
    );
    assert!(!audit.iter().any(|a| a.starts_with("misuse")), "{audit:?}");
    drop(instance);
}
