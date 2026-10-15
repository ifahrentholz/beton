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
use std::sync::Arc;

use beton_core::event::EgressSource;
use beton_harness::{StageEnv, ToolIsolation};
use beton_proxy::{
    BetonCa, Binding, CredentialAudit, CredentialKind, CredentialSource, Credentials, EgressEvents,
    EgressProxy, InstanceSpec, Passthrough, Placeholder, PrivateAllowList, ProxyOptions, RuleSet,
    Traffic,
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

/// Dienste der Stufe 0 für Credentials und Egress-Events (PRX-006, PRX-009): Werte und Audit
/// über den Tunnel zum Daemon, Events ins Session-Log.
#[derive(Clone)]
pub struct StageServices {
    pub source: Arc<dyn CredentialSource>,
    pub audit: Arc<dyn CredentialAudit>,
    pub events: Option<Arc<dyn EgressEvents>>,
}

impl std::fmt::Debug for StageServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StageServices")
    }
}

/// API-Key der Vendor-CLI bei `auth: api_key` (HAR-015 AC4): Die Variable in Stufe 1 bekommt
/// nur einen Platzhalter; die Hosts laufen statt Passthrough über MITM, damit der Proxy den
/// Wert einsetzen kann.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessKey {
    /// Binding-Name, z. B. `harness:claude`.
    pub binding: String,
    /// Variable, z. B. `ANTHROPIC_API_KEY`.
    pub var: String,
    pub hosts: Vec<String>,
}

/// Variable und Model-API-Hosts der Vendor-CLIs für `auth: api_key` (HAR-015, PRX-005).
pub fn vendor_key(harness: &str) -> Option<(&'static str, &'static [&'static str])> {
    match harness {
        "claude" => Some(("ANTHROPIC_API_KEY", &["api.anthropic.com"])),
        "codex" => Some(("OPENAI_API_KEY", &["api.openai.com"])),
        _ => None,
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
    /// Credentials und Egress-Events; ohne Dienste startet keine Session mit Credentials
    /// (fail closed).
    pub services: Option<StageServices>,
    /// API-Key der Vendor-CLI (`auth: api_key`, HAR-015 AC4).
    pub harness_key: Option<HarnessKey>,
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
    /// Platzhalter der Credentials der Stufe 2 je Binding (für `${secret:<name>}`, AGT-001).
    placeholders: BTreeMap<String, Placeholder>,
    _summaries: Option<AbortOnDrop>,
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

/// Beendet eine Aufgabe beim Fallenlassen.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl SessionStages {
    /// Platzhalter des Credentials `name` der Stufe 2 (`${secret:name}`, AGT-001, AGT-006).
    pub fn placeholder(&self, name: &str) -> Option<&Placeholder> {
        self.placeholders.get(name)
    }

    /// Passthrough-Ziele der Proxy-Instanz einer Stufe.
    pub fn passthrough(&self, stage: Stage) -> Vec<String> {
        match stage {
            Stage::Harness => self.passthrough[0].clone(),
            Stage::Tools => self.passthrough[1].clone(),
        }
    }
}

/// `start.sandbox` der Policy-Variablen (POL-004): `enabled` heißt, die Tools laufen in einer
/// Sandbox. Läuft ein Harness degraded (`tool_isolation: inherited`, Tools in Stufe 1), ist es
/// `false`, damit Policies, die eine Sandbox verlangen, greifen.
pub fn sandbox_vars(stages: Option<&SessionStages>) -> beton_policy::vars::StartSandboxVars {
    beton_policy::vars::StartSandboxVars {
        enabled: Some(stages.is_some_and(|s| !s.harness.policy().inherited_tools)),
        backend: Some(
            stages
                .map(|s| s.harness.policy().backend.as_str())
                .unwrap_or("none")
                .into(),
        ),
        profile: None,
    }
}

impl SessionStages {
    /// Kommandos, die der Exec-Broker bisher in Stufe 2 gestartet hat.
    pub fn broker_starts(&self) -> u64 {
        #[cfg(unix)]
        {
            self._broker.started()
        }
        #[cfg(not(unix))]
        {
            0
        }
    }
}

/// Integritätsprüfung für Harnesses mit `tool_isolation: shell_override` (Claude Code): Jeder
/// abgeschlossene Aufruf eines Shell-Tools muss über den Exec-Broker gestartet worden sein.
/// Ignoriert die CLI `CLAUDE_CODE_SHELL` (etwa nach einem Update), liefen ihre Kommandos in
/// Stufe 1; dann endet die Session (fail closed, Security-Review #228). Gezählt wird jeder
/// Abschluss außer `denied` (auch ein gescheitertes oder abgebrochenes Kommando ist gelaufen);
/// zusätzliche Broker-Starts (Env-Probe, Shell-Snapshot) schaden nicht. Der Zähler ist global,
/// nicht je Tool-Call; Code, der schon in Stufe 1 läuft, kann ihn erhöhen (#270).
#[derive(Debug, Default)]
pub struct ShellIntegrity {
    shell_tools: Vec<String>,
    open: std::collections::HashSet<String>,
    completed: u64,
}

impl ShellIntegrity {
    pub fn new(shell_tools: &[&str]) -> Self {
        Self {
            shell_tools: shell_tools.iter().map(|s| (*s).to_owned()).collect(),
            ..Self::default()
        }
    }

    /// Wertet ein Event aus; `Err` mit Begründung bei einem Verstoß.
    pub fn observe(
        &mut self,
        payload: &beton_core::event::EventPayload,
        broker_starts: u64,
    ) -> Result<(), String> {
        use beton_core::event::{EventPayload, ToolStatus};
        match payload {
            EventPayload::ToolCallRequested(r) if self.shell_tools.contains(&r.tool) => {
                self.open.insert(r.call_id.clone());
            }
            EventPayload::ToolCallCompleted(c) => {
                let shell = self.open.remove(&c.call_id);
                if shell && c.status != ToolStatus::Denied {
                    self.completed += 1;
                    if self.completed > broker_starts {
                        return Err(format!(
                            "Shell-Tool-Aufruf {} lief nicht über den Exec-Broker ({} \
                             abgeschlossene Aufrufe, {broker_starts} Starts in Stufe 2); die \
                             Session endet, weil Kommandos sonst in Stufe 1 liefen",
                            c.call_id, self.completed
                        ));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Stufe 2 darf vom Session-Verzeichnis nur ihren Scratch, den Austausch und das CA-Bundle
/// erreichen, nie Scratch und Sockets der Stufe 1 oder den Broker-Socket (fail closed, etwa
/// wenn `TMPDIR` in einer Freigabe liegt).
fn check_session_dir(
    root: &Path,
    allowed: &[&Path],
    tools: &beton_sandbox::ResolvedPolicy,
) -> Result<(), SandboxError> {
    for r in tools.read_roots.iter().chain(&tools.write_roots) {
        let covers_root = root.starts_with(r);
        let inside_other = r.starts_with(root) && !allowed.iter().any(|a| r.starts_with(a));
        if covers_root || inside_other {
            return Err(SandboxError::PathEscape {
                path: "sandbox".into(),
                message: format!(
                    "Stufe 2 erreicht das Session-Verzeichnis {} über {}",
                    root.display(),
                    r.display()
                ),
            });
        }
    }
    Ok(())
}

fn credential_kind(t: beton_sandbox::CredentialType) -> CredentialKind {
    use beton_sandbox::CredentialType as T;
    match t {
        T::Bearer => CredentialKind::Bearer,
        T::Basic => CredentialKind::Basic,
        T::GitHttps => CredentialKind::GitHttps,
        T::Github => CredentialKind::Github,
        T::Gitlab => CredentialKind::Gitlab,
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
    // Der Aufrufpfad selbst, wenn er ein Symlink ist (die Auflösung gibt den Link frei).
    if std::fs::symlink_metadata(&exe).is_ok_and(|m| m.file_type().is_symlink()) {
        out.push(exe.clone());
    }
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
    // Unter `session_base` (Linux: `/var/tmp`, nicht `TMPDIR`), damit die Symlink-Sperre im
    // Austausch greift; was Stufe 2 davon sieht, prüft `check_session_dir` (SBX-002).
    let dir = tempfile::Builder::new()
        .prefix("beton-sbx-")
        .tempdir_in(beton_sandbox::session_base())
        .map_err(|e| io("Sandbox-Verzeichnis", e))?;
    let root = dir
        .path()
        .canonicalize()
        .map_err(|e| io("Sandbox-Verzeichnis", e))?;
    let (harness_tmp, tools_tmp, xchg, exec, bin, ca_dir) = (
        root.join("harness"),
        root.join("tools"),
        root.join("xchg"),
        root.join("exec"),
        root.join("exec/bin"),
        root.join("ca"),
    );
    for d in [&root, &harness_tmp, &tools_tmp, &xchg, &exec, &bin, &ca_dir] {
        std::fs::create_dir_all(d).map_err(|e| io("Sandbox-Verzeichnis", e))?;
        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| io("Sandbox-Verzeichnis", e))?;
    }
    let grants = beton_sandbox::masks::harness_grants(input.harness);
    let harness_cfg = spec.harness.clone().unwrap_or_default();

    // Netz: Stufe 2 nach `allow_network`; Stufe 1 immer, wenn der Harness Vendor-Hosts
    // (Passthrough) oder `harness.egress_extra` hat, auch mit `allow_network: false` (z. B.
    // Preset `readonly`): Das gilt für die Tools, die Vendor-CLI braucht ihre API.
    let mut harness_pass: Vec<String> = grants
        .as_ref()
        .map(|g| g.passthrough.clone())
        .unwrap_or_default();
    harness_pass.extend(harness_cfg.egress_passthrough.clone().unwrap_or_default());
    let mut harness_extra = harness_cfg.egress_extra.clone().unwrap_or_default();
    let tools_net = spec.allow_network.unwrap_or(true);

    // Credentials (PRX-006): je Binding ein Platzhalter (160 bit, je Session neu); ohne
    // Dienste der Stufe 0 startet keine Session mit Credentials (fail closed).
    let tools_creds = spec.credentials.clone().unwrap_or_default();
    if (!tools_creds.is_empty() || opts.harness_key.is_some()) && opts.services.is_none() {
        return Err(unavailable(
            id,
            &[Cap::Net],
            "credentials_unavailable: keine Quelle für Credentials (Tunnel zum Daemon)",
        ));
    }
    let placeholder = || {
        Placeholder::generate().map_err(|e| SandboxError::SpawnFailed {
            message: e.to_string(),
            kind: std::io::ErrorKind::Other,
        })
    };
    let mut tools_bindings = Vec::new();
    let mut tools_env = BTreeMap::new();
    let mut placeholders = BTreeMap::new();
    for c in &tools_creds {
        let p = placeholder()?;
        for var in &c.env {
            tools_env.insert(var.clone(), p.to_string());
        }
        tools_bindings.push(Binding {
            name: c.name.clone(),
            kind: credential_kind(c.kind),
            hosts: c.hosts.clone().unwrap_or_default(),
            username: c.username.clone(),
            placeholder: p.clone(),
            implicit: true,
        });
        placeholders.insert(c.name.clone(), p);
    }
    // HAR-015 AC4: Der Key der Vendor-CLI steht in Stufe 1 nur als Platzhalter; ihre
    // Model-API-Hosts laufen dafür über MITM statt Passthrough (Präzisierung PRX-005).
    let mut harness_bindings = Vec::new();
    let mut harness_env = BTreeMap::new();
    if let Some(k) = &opts.harness_key {
        let p = placeholder()?;
        harness_env.insert(k.var.clone(), p.to_string());
        harness_pass.retain(|entry| {
            let host = entry.split(':').next().unwrap_or(entry);
            !k.hosts.iter().any(|h| h == host)
        });
        harness_extra.extend(k.hosts.iter().map(|h| format!("* {h}/**")));
        harness_bindings.push(Binding {
            name: k.binding.clone(),
            kind: CredentialKind::Bearer,
            hosts: k.hosts.clone(),
            username: None,
            placeholder: p,
            implicit: false,
        });
    }
    let harness_net = !harness_pass.is_empty() || !harness_extra.is_empty();
    let mut harness_spec = spec.clone();
    harness_spec.allow_network = Some(harness_net);

    // Proxy: je Stufe eine Instanz (PRX-001); ohne CA kein Netz (PRX-002, fail closed).
    let mut proxies = Vec::new();
    let mut endpoints: [Option<ProxyEndpoint>; 2] = [None, None];
    let mut passthrough: [Vec<String>; 2] = [Vec::new(), Vec::new()];
    // Erlaubter Verkehr der Session für `egress.summary` (PRX-009 AC3), höchstens einmal je
    // Minute.
    let traffic = Traffic::default();
    let summaries = opts
        .services
        .as_ref()
        .and_then(|s| s.events.clone())
        .map(|ev| {
            AbortOnDrop(beton_proxy::spawn_summaries(
                traffic.clone(),
                std::time::Duration::from_secs(60),
                move |summary| ev.summary(summary),
            ))
        });
    if tools_net || harness_net {
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
                harness_net,
                Stage::Harness,
                harness_extra,
                harness_pass,
                Vec::new(),
                "harness.egress_extra",
                std::mem::take(&mut harness_bindings),
                EgressSource::Harness,
            ),
            (
                tools_net,
                Stage::Tools,
                spec.egress_rules.clone().unwrap_or_default(),
                tools_pass,
                spec.egress_allow_private.clone().unwrap_or_default(),
                "egress_rules",
                std::mem::take(&mut tools_bindings),
                EgressSource::Tools,
            ),
        ];
        for (i, (needed, stage, rules, pass, private, field, bindings, source)) in
            specs.into_iter().enumerate()
        {
            if !needed {
                continue;
            }
            let credentials = match &opts.services {
                Some(sv) => Credentials::new(bindings, sv.source.clone(), sv.audit.clone())
                    .map_err(|e| invalid("credentials", e.to_string()))?,
                None => Credentials::default(),
            };
            let instance = proxy
                .start(InstanceSpec {
                    label: format!("{}/{}", input.session_id, stage.as_str()),
                    rules: RuleSet::compile(&rules).map_err(|e| invalid(field, e.to_string()))?,
                    allow_private: PrivateAllowList::parse(&private)
                        .map_err(|e| invalid("egress_allow_private", e.to_string()))?,
                    passthrough: Passthrough::parse(&pass)
                        .map_err(|e| invalid("egress_passthrough", e.to_string()))?,
                    credentials,
                    source,
                    events: opts.services.as_ref().and_then(|s| s.events.clone()),
                    traffic: Some(traffic.clone()),
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
    let mut c = ctx(Stage::Tools, &tools_tmp, tools_proxy);
    c.exchange = Some(xchg.clone());
    if tools_net {
        c.credential_env = tools_env;
    }
    // Unter Linux verdeckt ein leeres tmpfs das Session-Verzeichnis (Sockets der Stufe 1,
    // Broker-Socket; vor Landlock-ABI 9 sonst per `connect` erreichbar).
    c.session_dir = Some(root.clone());
    let tools_policy = provider.resolve(&spec, &c)?;
    check_session_dir(&root, &[&tools_tmp, &xchg, &ca_dir], &tools_policy)?;
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
    c.exchange = Some(xchg.clone());
    // Nichts im `PATH` der Stufe 1, was Stufe 2 schreiben kann (SBX-002).
    c.untrusted_path = tools.policy().write_roots.clone();
    c.programs = vec![exe];
    if let Some(program) = &input.program {
        for r in program_roots(program, input.host_env.get("PATH").map(String::as_str)) {
            if !c.programs.contains(&r) {
                c.programs.push(r);
            }
        }
    }
    c.credential_env = harness_env;
    let mut harness_policy = provider.resolve(&harness_spec, &c)?;
    harness_policy.inherited_tools = inherited;
    // PRX-006 AC4: Credential-Dateien liegen außerhalb aller Freigaben beider Stufen.
    for cred in &tools_creds {
        if let Some(file) = &cred.source.file {
            beton_sandbox::check_credential_file(
                Path::new(file),
                &input.home,
                &[tools.policy(), &harness_policy],
            )?;
        }
    }
    let harness = Sandbox::new(provider, harness_policy, input.events);
    Ok(SessionStages {
        harness,
        tools,
        env: StageEnv {
            shim_dir: bin,
            exchange: xchg,
            tools: None,
        },
        passthrough,
        placeholders,
        _summaries: summaries,
        _broker: broker,
        _proxies: proxies,
        _dir: dir,
    })
}
