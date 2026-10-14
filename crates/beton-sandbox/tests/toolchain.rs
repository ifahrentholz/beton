//! Toolchains in Stufe 2 über den Egress-Proxy (PRX-008 AC1, AC2) und `preset: dev` mit npm
//! (SBX-012 AC1), gegen das Backend dieser Plattform (macOS: Seatbelt, Linux: Landlock bzw.
//! bwrap).
//!
//! Jedes Werkzeug läuft ohne eigene Proxy- oder CA-Optionen: Es bekommt nur das Environment
//! der Auflösung (`HTTPS_PROXY`, `SSL_CERT_FILE`, `NODE_EXTRA_CA_CERTS` …). Die Gegenstellen sind
//! ein lokaler TLS-Upstream mit Test-CA; der Proxy bildet die Namen über das Feature `test-net`
//! auf ihn ab, nichts erreicht ein echtes Netz (ADR-0033). In der CI (`CI` gesetzt) muss jedes
//! Werkzeug vorhanden sein; lokal wird ein fehlendes gemeldet.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use beton_sandbox::{
    BackendChoice, CommandSpec, ProxyEndpoint, Registry, ResolveCtx, ResolvedPolicy,
    SandboxProvider, SandboxSpec, Secret, Stage, Stdio,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// Namen, die der Test-Upstream bedient.
const HOSTS: [&str; 6] = [
    "registry.npmjs.org",
    "pypi.org",
    "files.pythonhosted.org",
    "index.crates.io",
    "git.test",
    "api.test",
];

const GIT_SHA: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

fn write(p: &Path, content: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

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

/// Ein Werkzeug aus dem `PATH` des Tests mit dem Verzeichnis, das die Sandbox lesen muss.
/// Fehlt es, scheitert der Test in der CI; lokal wird es gemeldet.
fn tool(name: &str) -> Option<(PathBuf, PathBuf)> {
    let path = std::env::var("PATH").unwrap_or_default();
    let found = path
        .split(':')
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file());
    let Some(found) = found else {
        assert!(
            std::env::var_os("CI").is_none(),
            "{name} fehlt in der CI (PRX-008 AC1 verlangt einen Test je Werkzeug)"
        );
        eprintln!("{name} fehlt lokal; Fall übersprungen");
        return None;
    };
    let mut exe = found.canonicalize().unwrap();
    // macOS: `/usr/bin/git` ist ein Xcode-Shim; das echte Werkzeug liefert `xcrun`.
    if cfg!(target_os = "macos") && exe.starts_with("/usr/bin") && name == "git" {
        let out = std::process::Command::new("/usr/bin/xcrun")
            .args(["-f", name])
            .output()
            .unwrap();
        exe = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
            .canonicalize()
            .unwrap();
        // …/Contents/Developer/usr/bin/git → …/Contents/Developer
        return Some((exe.clone(), exe.ancestors().nth(3).unwrap().to_path_buf()));
    }
    // rustup-Proxy: das echte Werkzeug der Toolchain.
    if exe.file_name().is_some_and(|n| n == "rustup") {
        let out = std::process::Command::new(&found)
            .args(["which", name])
            .output()
            .unwrap();
        exe = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
            .canonicalize()
            .unwrap();
    }
    // Präfix der Installation (…/bin/<werkzeug> → …), für Homebrew die ganze Wurzel.
    let prefix = exe.ancestors().nth(2).unwrap().to_path_buf();
    let root = if prefix.starts_with("/opt/homebrew") {
        PathBuf::from("/opt/homebrew")
    } else if prefix.starts_with("/usr/local") {
        PathBuf::from("/usr/local")
    } else {
        prefix
    };
    Some((exe, root))
}

/// Was der Upstream gesehen hat: (Methode, Host, Pfad).
type Seen = Arc<Mutex<Vec<(String, String, String)>>>;

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    ws: PathBuf,
    scratch: PathBuf,
    seen: Seen,
    proxy: beton_proxy::EgressProxy,
    bundle: PathBuf,
    _upstream: tokio::task::JoinHandle<()>,
}

/// Antwort des Upstreams auf `method path` (Host egal).
fn respond(method: &str, path: &str, files: &BTreeMap<String, Vec<u8>>) -> (u16, String, Vec<u8>) {
    let path = path.split('?').next().unwrap_or(path);
    if method != "GET" && method != "HEAD" {
        return (405, "text/plain".into(), b"nein".to_vec());
    }
    match files.get(path) {
        Some(body) => {
            let ct = if path.ends_with('/') {
                "text/html"
            } else if path.ends_with(".json") || !path.contains('.') {
                "application/json"
            } else {
                "application/octet-stream"
            };
            (200, ct.into(), body.clone())
        }
        None if path == "/ok" => (200, "text/plain".into(), b"upstream-ok\n".to_vec()),
        None => (404, "text/plain".into(), b"nicht gefunden".to_vec()),
    }
}

async fn upstream(
    files: BTreeMap<String, Vec<u8>>,
    seen: Seen,
) -> (
    SocketAddr,
    rustls::pki_types::CertificateDer<'static>,
    tokio::task::JoinHandle<()>,
) {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, PKCS_ECDSA_P256_SHA256,
    };
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut ca = CertificateParams::default();
    ca.distinguished_name
        .push(DnType::CommonName, "beton toolchain test upstream CA");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let leaf = CertificateParams::new(HOSTS.iter().map(|h| (*h).to_owned()).collect::<Vec<_>>())
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
    let files = Arc::new(files);
    let task = tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (acceptor, files, seen) = (acceptor.clone(), files.clone(), seen.clone());
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                loop {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match tls.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
                    let mut lines = head.lines();
                    let mut first = lines.next().unwrap_or_default().split_whitespace();
                    let method = first.next().unwrap_or_default().to_owned();
                    let path = first.next().unwrap_or_default().to_owned();
                    let header = |name: &str| {
                        head.lines().find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.trim()
                                .eq_ignore_ascii_case(name)
                                .then(|| v.trim().to_owned())
                        })
                    };
                    let host = header("host").unwrap_or_default();
                    // Body (z. B. npm publish) lesen und verwerfen.
                    let len: usize = header("content-length")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let mut have = buf.len() - end;
                    while have < len {
                        match tls.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => have += n,
                        }
                    }
                    seen.lock()
                        .unwrap()
                        .push((method.clone(), host, path.clone()));
                    let (status, ct, body) = respond(&method, &path, &files);
                    let mut res = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: {ct}\r\nContent-Length: {}\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    if method != "HEAD" {
                        res.extend_from_slice(&body);
                    }
                    if tls.write_all(&res).await.is_err() {
                        return;
                    }
                    let _ = tls.flush().await;
                }
            });
        }
    });
    (addr, ca_cert.der().clone(), task)
}

/// Inhalte des Upstreams: npm-Paket, PyPI-Index mit Wheel, Crates-Index, Git (dumb HTTP).
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut f = BTreeMap::new();
    // npm: Paket als Tarball (package/package.json).
    let pkg = root.join("npm-src/package");
    write(
        &pkg.join("package.json"),
        br#"{"name":"beton-test","version":"1.0.0","main":"index.js"}"#,
    );
    write(&pkg.join("index.js"), b"module.exports = 'beton';\n");
    let tgz = root.join("beton-test-1.0.0.tgz");
    let st = std::process::Command::new("tar")
        .args(["-czf"])
        .arg(&tgz)
        .args(["-C"])
        .arg(root.join("npm-src"))
        .arg("package")
        .status()
        .unwrap();
    assert!(st.success());
    let doc = serde_json::json!({
        "name": "beton-test",
        "dist-tags": {"latest": "1.0.0"},
        "versions": {"1.0.0": {
            "name": "beton-test",
            "version": "1.0.0",
            "dist": {"tarball": "https://registry.npmjs.org/beton-test/-/beton-test-1.0.0.tgz"}
        }}
    });
    f.insert("/beton-test".into(), doc.to_string().into_bytes());
    f.insert(
        "/beton-test/-/beton-test-1.0.0.tgz".into(),
        std::fs::read(&tgz).unwrap(),
    );
    // PyPI: Simple-Index und ein minimales Wheel.
    let whl = root.join("beton_test-1.0.0-py3-none-any.whl");
    let py = std::process::Command::new("python3")
        .args([
            "-c",
            "import sys, zipfile\n\
             z = zipfile.ZipFile(sys.argv[1], 'w')\n\
             d = 'beton_test-1.0.0.dist-info/'\n\
             z.writestr('beton_test/__init__.py', '')\n\
             z.writestr(d + 'METADATA', 'Metadata-Version: 2.1\\nName: beton-test\\nVersion: 1.0.0\\n')\n\
             z.writestr(d + 'WHEEL', 'Wheel-Version: 1.0\\nGenerator: beton\\nRoot-Is-Purelib: true\\nTag: py3-none-any\\n')\n\
             z.writestr(d + 'RECORD', '')\n\
             z.close()",
        ])
        .arg(&whl)
        .status();
    if let Ok(st) = py
        && st.success()
    {
        f.insert(
            "/packages/beton_test-1.0.0-py3-none-any.whl".into(),
            std::fs::read(&whl).unwrap(),
        );
    }
    f.insert(
        "/simple/beton-test/".into(),
        b"<html><body><a href=\"https://files.pythonhosted.org/packages/beton_test-1.0.0-py3-none-any.whl\">beton_test-1.0.0-py3-none-any.whl</a></body></html>".to_vec(),
    );
    // Crates (sparse): config.json und Index-Eintrag.
    f.insert(
        "/config.json".into(),
        br#"{"dl":"https://static.crates.io/crates","api":"https://crates.io"}"#.to_vec(),
    );
    f.insert(
        "/be/to/beton-test".into(),
        format!(
            "{{\"name\":\"beton-test\",\"vers\":\"1.0.0\",\"deps\":[],\"cksum\":\"{}\",\"features\":{{}},\"yanked\":false}}\n",
            "0".repeat(64)
        )
        .into_bytes(),
    );
    // Git (dumb HTTP): nur die Referenzen.
    f.insert(
        "/repo.git/info/refs".into(),
        format!("{GIT_SHA}\trefs/heads/main\n").into_bytes(),
    );
    f
}

async fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let home = root.join("home");
    let ws = root.join("ws");
    let scratch = root.join("scratch");
    for d in [&home, &ws, &scratch, &home.join(".npm"), &root.join("ca")] {
        std::fs::create_dir_all(d).unwrap();
    }
    let seen = Seen::default();
    let (addr, upstream_ca, task) = upstream(files(&root), seen.clone()).await;
    let ca =
        beton_proxy::BetonCa::generate("toolchain-test", time::OffsetDateTime::now_utc()).unwrap();
    let hosts: Vec<(&str, SocketAddr)> = HOSTS.iter().map(|h| (*h, addr)).collect();
    let proxy = beton_proxy::EgressProxy::with_test_hosts(ca, vec![upstream_ca], &hosts);
    let bundle = root.join("ca/bundle.pem");
    std::fs::write(&bundle, proxy.ca_bundle_pem()).unwrap();
    Fixture {
        _tmp: tmp,
        root,
        home,
        ws,
        scratch,
        seen,
        proxy,
        bundle,
        _upstream: task,
    }
}

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl std::fmt::Debug for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ok={} stdout={} stderr={}",
            self.ok, self.stdout, self.stderr
        )
    }
}

impl Fixture {
    /// Proxy-Instanz mit `rules` und Policy der Stufe 2 aus `yaml` plus Lese-Grants für
    /// `tools`.
    async fn stage_two(
        &self,
        rules: &[&str],
        yaml: &str,
        tools: &[&Path],
    ) -> (beton_proxy::ProxyInstance, ResolvedPolicy) {
        let instance = self
            .proxy
            .start(beton_proxy::InstanceSpec {
                label: "toolchain/tools".into(),
                rules: beton_proxy::RuleSet::compile(rules).unwrap(),
                ..beton_proxy::InstanceSpec::default()
            })
            .await
            .unwrap();
        let mut ctx = ResolveCtx::new(Stage::Tools, &self.ws, &self.home, &self.scratch);
        ctx.adapters = beton_sandbox::masks::builtin_harness_grants();
        ctx.host_env = std::env::vars()
            .filter(|(k, _)| {
                ["PATH", "LANG", "USER", "LOGNAME", "SHELL", "TERM"].contains(&k.as_str())
            })
            .collect();
        ctx.proxy = Some(ProxyEndpoint {
            port: instance.addr().port(),
            url: Some(Secret::new(instance.proxy_url().to_string())),
            ca_bundle: Some(self.bundle.clone()),
        });
        let mut spec = SandboxSpec::from_yaml(yaml).unwrap();
        let mut reads = spec.read_paths.clone().unwrap_or_default();
        reads.extend(tools.iter().map(|t| t.display().to_string()));
        spec.read_paths = Some(reads);
        let policy = provider().resolve(&spec, &ctx).unwrap();
        (instance, policy)
    }

    fn requests(&self) -> Vec<(String, String, String)> {
        self.seen.lock().unwrap().clone()
    }
}

async fn sh(policy: &ResolvedPolicy, script: &str) -> Run {
    let cmd = CommandSpec::new("/bin/sh")
        .args(["-c", script])
        .cwd(&policy.workspace)
        .stdio(Stdio::Null, Stdio::Piped, Stdio::Piped);
    let out = provider()
        .spawn(policy, cmd)
        .unwrap()
        .output(Duration::from_secs(180))
        .await
        .unwrap();
    Run {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prx_008_ac1_curl_reaches_an_allowed_host() {
    let fx = fixture().await;
    let Some((curl, root)) = tool("curl") else {
        return;
    };
    let (_i, policy) = fx.stage_two(&["GET api.test/**"], "{}\n", &[&root]).await;
    let out = sh(
        &policy,
        &format!("{} -sS --max-time 20 https://api.test/ok", curl.display()),
    )
    .await;
    assert!(out.ok && out.stdout.contains("upstream-ok"), "{out:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prx_008_ac1_git_reaches_an_allowed_host() {
    let fx = fixture().await;
    let Some((git, root)) = tool("git") else {
        return;
    };
    let (_i, policy) = fx.stage_two(&["GET git.test/**"], "{}\n", &[&root]).await;
    let out = sh(
        &policy,
        &format!(
            "GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0 {} ls-remote https://git.test/repo.git",
            git.display()
        ),
    )
    .await;
    assert!(out.ok && out.stdout.contains(GIT_SHA), "{out:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prx_008_ac1_node_fetch_reaches_an_allowed_host() {
    let fx = fixture().await;
    let Some((node, root)) = tool("node") else {
        return;
    };
    let (_i, policy) = fx.stage_two(&["GET api.test/**"], "{}\n", &[&root]).await;
    let out = sh(
        &policy,
        &format!(
            "{} -e \"fetch('https://api.test/ok').then(r => r.text()).then(t => console.log(t)).catch(e => {{ console.error(e); process.exit(1) }})\"",
            node.display()
        ),
    )
    .await;
    assert!(out.ok && out.stdout.contains("upstream-ok"), "{out:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prx_008_ac1_npm_reaches_an_allowed_host() {
    let fx = fixture().await;
    let (Some((npm, root)), Some((_, node_root))) = (tool("npm"), tool("node")) else {
        return;
    };
    let (_i, policy) = fx
        .stage_two(
            &["GET,HEAD registry.npmjs.org/**"],
            "write_paths: [\"~/.npm\"]\n",
            &[&root, &node_root],
        )
        .await;
    let out = sh(
        &policy,
        &format!(
            "PATH=\"{}:$PATH\" {} view beton-test version",
            node_root.join("bin").display(),
            npm.display()
        ),
    )
    .await;
    assert!(out.ok && out.stdout.contains("1.0.0"), "{out:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prx_008_ac1_pip_reaches_an_allowed_host() {
    let fx = fixture().await;
    let Some((python, root)) = tool("python3") else {
        return;
    };
    let (_i, policy) = fx
        .stage_two(
            &["GET pypi.org/** files.pythonhosted.org/**"],
            "{}\n",
            &[&root],
        )
        .await;
    let out = sh(
        &policy,
        &format!(
            "{} -m pip download --disable-pip-version-check --no-cache-dir --no-deps --index-url https://pypi.org/simple/ -d dl beton-test && ls dl",
            python.display()
        ),
    )
    .await;
    assert!(
        out.ok && out.stdout.contains("beton_test-1.0.0-py3-none-any.whl"),
        "{out:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prx_008_ac1_cargo_reaches_an_allowed_host() {
    let fx = fixture().await;
    let Some((cargo, root)) = tool("cargo") else {
        return;
    };
    write(
        &fx.ws.join("proj/Cargo.toml"),
        b"[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nbeton-test = \"1\"\n",
    );
    write(&fx.ws.join("proj/src/lib.rs"), b"");
    let cargo_home = fx.root.join("cargo-home");
    std::fs::create_dir_all(&cargo_home).unwrap();
    let (_i, policy) = fx
        .stage_two(
            &["GET index.crates.io/**"],
            &format!("write_paths: [\"{}\"]\n", cargo_home.display()),
            &[&root],
        )
        .await;
    let out = sh(
        &policy,
        &format!(
            "cd proj && CARGO_HOME=\"{}\" {} generate-lockfile && grep -A1 'name = \"beton-test\"' Cargo.lock",
            cargo_home.display(),
            cargo.display()
        ),
    )
    .await;
    assert!(out.ok && out.stdout.contains("1.0.0"), "{out:?}");
    assert!(
        fx.requests()
            .iter()
            .any(|(_, _, p)| p == "/be/to/beton-test"),
        "{:?}",
        fx.requests()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prx_008_ac2_tools_ignoring_the_proxy_variables_cannot_connect() {
    let fx = fixture().await;
    let Some((curl, root)) = tool("curl") else {
        return;
    };
    let (_i, policy) = fx.stage_two(&["GET api.test/**"], "{}\n", &[&root]).await;
    // Mit Proxy gelingt es (Kontrollgruppe in derselben Sandbox) …
    let out = sh(
        &policy,
        &format!("{} -sS --max-time 20 https://api.test/ok", curl.display()),
    )
    .await;
    assert!(out.ok, "{out:?}");
    let before = fx.requests().len();
    // … am Proxy vorbei nicht: weder per Name noch direkt an einer Adresse.
    for script in [
        format!(
            "{} -sS --noproxy '*' --max-time 5 https://api.test/ok",
            curl.display()
        ),
        format!(
            "env -u HTTPS_PROXY -u https_proxy -u ALL_PROXY -u all_proxy {} -sS --max-time 5 https://api.test/ok",
            curl.display()
        ),
        format!(
            "{} -sS --noproxy '*' --max-time 5 https://192.0.2.1/",
            curl.display()
        ),
    ] {
        let out = sh(&policy, &script).await;
        assert!(!out.ok, "{script}: {out:?}");
    }
    assert_eq!(fx.requests().len(), before, "{:?}", fx.requests());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sbx_012_ac1_dev_preset_installs_npm_packages_but_cannot_publish() {
    let fx = fixture().await;
    let (Some((npm, root)), Some((_, node_root))) = (tool("npm"), tool("node")) else {
        return;
    };
    // Nur die Regeln des Presets (keine eigenen).
    let rules: Vec<String> =
        SandboxSpec::merged(&[&SandboxSpec::from_yaml("preset: dev\n").unwrap()])
            .egress_rules
            .unwrap();
    let rules: Vec<&str> = rules.iter().map(String::as_str).collect();
    let (_i, policy) = fx
        .stage_two(&rules, "preset: dev\n", &[&root, &node_root])
        .await;
    assert!(policy.write_roots.contains(&fx.home.join(".npm")));
    write(
        &fx.ws.join("package.json"),
        br#"{"name":"app","version":"1.0.0"}"#,
    );
    let path = format!("PATH=\"{}:$PATH\"", node_root.join("bin").display());
    let out = sh(
        &policy,
        &format!(
            "{path} {} install beton-test --no-audit --no-fund && cat node_modules/beton-test/package.json",
            npm.display()
        ),
    )
    .await;
    assert!(out.ok && out.stdout.contains("\"beton-test\""), "{out:?}");
    // Veröffentlichen braucht PUT – das Preset erlaubt nur GET und HEAD.
    write(
        &fx.ws.join("pub/package.json"),
        br#"{"name":"beton-publish-test","version":"1.0.0"}"#,
    );
    write(
        &fx.ws.join("pub/.npmrc"),
        b"//registry.npmjs.org/:_authToken=FIXTURE-NPM-TOKEN\n",
    );
    let out = sh(
        &policy,
        &format!("cd pub && {path} {} publish", npm.display()),
    )
    .await;
    assert!(!out.ok, "{out:?}");
    assert!(
        fx.requests()
            .iter()
            .all(|(m, _, _)| m == "GET" || m == "HEAD"),
        "{:?}",
        fx.requests()
    );
}
