//! Zweistufige Sandbox einer Session (SBX-002, PRX-001, PRX-005, PRX-008, #191): Proxy-Instanz
//! je Stufe, Stufe-2-Sandbox mit Exec-Broker, Stufe-1-Sandbox für den Harness.
//!
//! ```text
//!   Stufe 0 (Runner)                     Stufe 1 (Harness)          Stufe 2 (Tools)
//!   EgressProxy ─ Instanz harness ◄────── claude (Passthrough: api.anthropic.com …)
//!               └ Instanz tools   ◄────────────────────────────────── npm, git, curl …
//!   Broker  ◄──── bt-exec ─────────────── Bash-Tool ─── Sandbox::spawn ──► /bin/zsh -c …
//! ```
//!
//! Bis WP-49 (SBX-006) greift das Modell nur, wenn `sandbox.backend` ausdrücklich gesetzt und
//! nicht `none` ist; sonst laufen Harness und Tools wie bisher über `backend: none`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use beton_harness::{StageEnv, ToolIsolation};
use beton_proxy::{
    BetonCa, EgressProxy, InstanceSpec, Passthrough, PrivateAllowList, ProxyOptions, RuleSet,
};
use beton_sandbox::registry::EventSink;
use beton_sandbox::{
    BackendChoice, Cap, ProviderId, ProxyEndpoint, Registry, ResolveCtx, Sandbox, SandboxError,
    SandboxProvider, SandboxSpec, Secret, Stage,
};

/// `sandbox:` und `proxy.extra_ca_files` vom Daemon (Env `BETON_RUNNER_SANDBOX`, nur Pfade
/// und Regeln, keine Secrets).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SandboxBoot {
    #[serde(default)]
    pub spec: SandboxSpec,
    #[serde(default)]
    pub extra_ca_files: Vec<PathBuf>,
}

/// Übergabeform der beton-CA vom Daemon (dritte stdin-Zeile, nie Env oder argv); `Debug`
/// zeigt nichts, der Speicher wird überschrieben.
#[derive(Clone)]
pub struct CaText(zeroize::Zeroizing<String>);

impl CaText {
    pub fn new(text: String) -> Self {
        Self(zeroize::Zeroizing::new(text))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Die CA; kaputt oder abgelaufen → Fehler (fail closed, PRX-002).
    pub fn ca(&self) -> Result<BetonCa, beton_proxy::CaError> {
        BetonCa::from_handover(&self.0)
    }
}

impl std::fmt::Debug for CaText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CaText(***)")
    }
}

/// Einstellungen des Daemons für das Stufenmodell.
pub struct StageOptions {
    /// `sandbox:` der User-Konfiguration (Projekt- und Agent-Ebene folgen mit #200).
    pub spec: SandboxSpec,
    /// beton-CA vom Daemon (über stdin); ohne CA gibt es keinen Proxy und damit keine Session
    /// mit Netz (PRX-002, fail closed).
    pub proxy_ca: Option<BetonCa>,
    /// `proxy.extra_ca_files` u. a.
    pub proxy: ProxyOptions,
    /// beton selbst: `bt-exec` in Stufe 1 und Launcher unter Linux.
    pub exe: PathBuf,
}

impl std::fmt::Debug for StageOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StageOptions")
            .field("spec", &self.spec)
            .field("proxy_ca", &self.proxy_ca.is_some())
            .finish_non_exhaustive()
    }
}

/// Was der Runner über die Session weiß.
pub struct StageInput<'a> {
    pub session_id: String,
    pub harness: &'a str,
    pub tool_isolation: ToolIsolation,
    pub workdir: &'a Path,
    pub home: PathBuf,
    pub host_env: BTreeMap<String, String>,
    /// Programm des Harness (aufgelöst), für die Lese-Grants der Stufe 1.
    pub program: Option<PathBuf>,
    pub events: Option<EventSink>,
}

/// Die laufenden Stufen einer Session; alles endet mit `Drop` (Proxy-Instanzen, Broker,
/// Verzeichnisse).
pub struct SessionStages {
    pub harness: Sandbox,
    pub tools: Sandbox,
    pub env: StageEnv,
    passthrough: [Vec<String>; 2],
    #[cfg(unix)]
    _broker: beton_sandbox::broker::Broker,
    _proxies: Vec<beton_proxy::ProxyInstance>,
    _dir: tempfile::TempDir,
}

impl std::fmt::Debug for SessionStages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionStages")
            .field("harness", &self.harness)
            .field("tools", &self.tools)
            .finish_non_exhaustive()
    }
}

impl SessionStages {
    /// Passthrough-Ziele der Proxy-Instanz einer Stufe.
    pub fn passthrough(&self, stage: Stage) -> Vec<String> {
        match stage {
            Stage::Harness => self.passthrough[0].clone(),
            Stage::Tools => self.passthrough[1].clone(),
        }
    }
}

/// Ist das Stufenmodell konfiguriert? Bis WP-49 nur mit ausdrücklichem Backend außer `none`.
pub fn active(spec: &SandboxSpec) -> bool {
    matches!(
        spec.backend,
        Some(b) if b != BackendChoice::Provider(ProviderId::None)
    )
}

fn unavailable(provider: ProviderId, missing: &[Cap], hint: impl Into<String>) -> SandboxError {
    SandboxError::Unavailable {
        provider,
        missing: missing.iter().copied().collect(),
        hint: hint.into(),
    }
}

fn io(step: &str, e: std::io::Error) -> SandboxError {
    SandboxError::SpawnFailed {
        message: format!("{step}: {e}"),
        kind: e.kind(),
    }
}

/// Verzeichnisse, die die Stufe 1 zum Ausführen des Harness lesen muss: das Installations-
/// verzeichnis des (kanonischen) Programms und bei Node-Skripten das von `node`.
fn program_roots(program: &Path, path: Option<&str>) -> Vec<PathBuf> {
    fn root(exe: &Path) -> Option<PathBuf> {
        let exe = exe.canonicalize().ok()?;
        let r = exe.ancestors().nth(2)?.to_path_buf();
        let system = ["/", "/usr", "/bin", "/sbin", "/usr/local", "/opt/homebrew"];
        if system.iter().any(|s| r == Path::new(s)) {
            // Systempfade liest jede Sandbox; bei Präfixen wie Homebrew nur das Programm.
            return (!r.starts_with("/usr") && r != Path::new("/")).then_some(exe);
        }
        Some(r)
    }
    let find = |name: &str| -> Option<PathBuf> {
        if name.contains('/') {
            return Some(PathBuf::from(name));
        }
        path?
            .split(':')
            .map(|d| Path::new(d).join(name))
            .find(|p| p.is_file())
    };
    let Some(exe) = find(&program.to_string_lossy()) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = root(&exe).into_iter().collect();
    let head = std::fs::read(&exe)
        .ok()
        .map(|b| b.into_iter().take(128).collect::<Vec<u8>>())
        .unwrap_or_default();
    if head.starts_with(b"#!")
        && String::from_utf8_lossy(&head).contains("node")
        && let Some(node) = find("node").and_then(|n| root(&n))
        && !out.contains(&node)
    {
        out.push(node);
    }
    out
}

/// Baut die Stufen der Session. `Ok(None)`: kein Stufenmodell konfiguriert (bis WP-49).
pub async fn compose(
    input: StageInput<'_>,
    opts: StageOptions,
) -> Result<Option<SessionStages>, SandboxError> {
    let spec = SandboxSpec::merged(&[&opts.spec]);
    if !active(&spec) {
        return Ok(None);
    }
    let choice = spec.backend.unwrap_or(BackendChoice::Auto);
    let registry = Registry::with_launcher(&opts.exe);
    let provider = registry.select(choice)?;
    let id = provider.id();
    // SBX-002 AC4: Ein Harness, der Tools selbst ausführt, nur mit ausdrücklichem Opt-out.
    let inherited = input.tool_isolation == ToolIsolation::Inherited;
    if inherited && spec.require_tool_isolation != Some(false) {
        return Err(unavailable(
            id,
            &[],
            format!(
                "Harness `{}` führt Tools selbst aus (tool_isolation: inherited) und kann Stufe 2 \
                 nicht erzwingen; mit sandbox.require_tool_isolation: false startet die Session \
                 als degraded",
                input.harness
            ),
        ));
    }
    #[cfg(not(unix))]
    {
        let _ = (input, opts, inherited);
        return Err(unavailable(
            id,
            &[Cap::Proc],
            "Exec-Broker gibt es unter Windows erst mit SBX-016 (M5)",
        ));
    }
    #[cfg(unix)]
    {
        compose_unix(input, opts, spec, provider, inherited)
            .await
            .map(Some)
    }
}

#[cfg(unix)]
async fn compose_unix(
    input: StageInput<'_>,
    opts: StageOptions,
    spec: SandboxSpec,
    provider: std::sync::Arc<dyn SandboxProvider>,
    inherited: bool,
) -> Result<SessionStages, SandboxError> {
    use std::os::unix::fs::PermissionsExt as _;

    let id = provider.id();
    let dir = tempfile::Builder::new()
        .prefix("beton-sbx-")
        .tempdir()
        .map_err(|e| io("Sandbox-Verzeichnis", e))?;
    let root = dir.path().to_path_buf();
    let (harness_tmp, tools_tmp, exec, bin, ca_dir) = (
        root.join("harness"),
        root.join("tools"),
        root.join("exec"),
        root.join("exec/bin"),
        root.join("ca"),
    );
    for d in [&root, &harness_tmp, &tools_tmp, &exec, &bin, &ca_dir] {
        std::fs::create_dir_all(d).map_err(|e| io("Sandbox-Verzeichnis", e))?;
        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| io("Sandbox-Verzeichnis", e))?;
    }
    let grants = beton_sandbox::masks::harness_grants(input.harness);
    let harness_cfg = spec.harness.clone().unwrap_or_default();

    // Proxy: je Stufe eine Instanz (PRX-001); ohne CA kein Netz (PRX-002, fail closed).
    let mut proxies = Vec::new();
    let mut endpoints: [Option<ProxyEndpoint>; 2] = [None, None];
    let mut passthrough: [Vec<String>; 2] = [Vec::new(), Vec::new()];
    if spec.allow_network.unwrap_or(true) {
        let ca = opts.proxy_ca.ok_or_else(|| {
            unavailable(
                id,
                &[Cap::Net],
                "proxy_ca_missing: Der Daemon hat keine beton-CA übergeben; ohne Proxy keine \
                 Session mit Netz (oder allow_network: false)",
            )
        })?;
        let proxy = EgressProxy::new(ca, &opts.proxy)
            .map_err(|e| unavailable(id, &[Cap::Net], e.to_string()))?;
        let bundle = ca_dir.join("bundle.pem");
        std::fs::write(&bundle, proxy.ca_bundle_pem()).map_err(|e| io("CA-Bundle", e))?;
        let invalid = |field: &str, e: String| SandboxError::InvalidSpec {
            path: field.into(),
            message: e,
        };
        let mut harness_pass: Vec<String> = grants
            .as_ref()
            .map(|g| g.passthrough.clone())
            .unwrap_or_default();
        harness_pass.extend(harness_cfg.egress_passthrough.clone().unwrap_or_default());
        let tools_pass = spec.egress_passthrough.clone().unwrap_or_default();
        if !tools_pass.is_empty() {
            // Ausdrücklich konfiguriert (PRX-005); ohne TLS-Terminierung und ohne
            // Credential-Injection.
            tracing::warn!(
                hosts = ?tools_pass,
                "Passthrough in Stufe 2 (sandbox.egress_passthrough)"
            );
        }
        let specs = [
            (
                Stage::Harness,
                harness_cfg.egress_extra.clone().unwrap_or_default(),
                harness_pass,
                Vec::new(),
                "harness.egress_extra",
            ),
            (
                Stage::Tools,
                spec.egress_rules.clone().unwrap_or_default(),
                tools_pass,
                spec.egress_allow_private.clone().unwrap_or_default(),
                "egress_rules",
            ),
        ];
        for (i, (stage, rules, pass, private, field)) in specs.into_iter().enumerate() {
            let instance = proxy
                .start(InstanceSpec {
                    label: format!("{}/{}", input.session_id, stage.as_str()),
                    rules: RuleSet::compile(&rules).map_err(|e| invalid(field, e.to_string()))?,
                    allow_private: PrivateAllowList::parse(&private)
                        .map_err(|e| invalid("egress_allow_private", e.to_string()))?,
                    passthrough: Passthrough::parse(&pass)
                        .map_err(|e| invalid("egress_passthrough", e.to_string()))?,
                })
                .await
                .map_err(|e| unavailable(id, &[Cap::Net], e.to_string()))?;
            passthrough[i] = Passthrough::parse(&pass)
                .map(|p| p.entries())
                .unwrap_or_default();
            endpoints[i] = Some(ProxyEndpoint {
                port: instance.addr().port(),
                url: Some(Secret::new(instance.proxy_url().to_string())),
                ca_bundle: Some(bundle.clone()),
            });
            proxies.push(instance);
        }
    }

    let git = beton_sandbox::GitLayout::discover(input.workdir)?;
    let ctx = |stage: Stage, scratch: &Path, proxy: Option<ProxyEndpoint>| {
        let mut c = ResolveCtx::new(stage, input.workdir, &input.home, scratch);
        c.git.clone_from(&git);
        c.adapters = beton_sandbox::masks::builtin_harness_grants();
        c.proxy = proxy;
        c.host_env.clone_from(&input.host_env);
        c.session_id = Some(input.session_id.clone());
        c
    };
    let [harness_proxy, tools_proxy] = endpoints;

    // Stufe 2 und der Broker.
    let tools_policy = provider.resolve(&spec, &ctx(Stage::Tools, &tools_tmp, tools_proxy))?;
    let tools = Sandbox::new(provider.clone(), tools_policy, input.events.clone());
    let broker = beton_sandbox::broker::Broker::start(&exec, tools.clone())
        .await
        .map_err(|e| io("Exec-Broker", e))?;
    let exe = opts.exe.canonicalize().map_err(|e| io("beton-Binary", e))?;
    for name in ["bt-exec", "bt-exec-zsh", "bt-exec-bash", "bt-exec-sh"] {
        std::os::unix::fs::symlink(&exe, bin.join(name)).map_err(|e| io("bt-exec", e))?;
    }

    // Stufe 1.
    let mut c = ctx(Stage::Harness, &harness_tmp, harness_proxy);
    c.harness = grants;
    c.exec = Some(broker.access(bin.clone()));
    c.shared = vec![tools_tmp.clone()];
    c.programs = vec![exe];
    if let Some(program) = &input.program {
        for r in program_roots(program, input.host_env.get("PATH").map(String::as_str)) {
            if !c.programs.contains(&r) {
                c.programs.push(r);
            }
        }
    }
    let mut harness_policy = provider.resolve(&spec, &c)?;
    harness_policy.inherited_tools = inherited;
    let harness = Sandbox::new(provider, harness_policy, input.events);
    Ok(SessionStages {
        harness,
        tools,
        env: StageEnv {
            shim_dir: bin,
            exchange: tools_tmp,
            tools: None,
        },
        passthrough,
        _broker: broker,
        _proxies: proxies,
        _dir: dir,
    })
}
