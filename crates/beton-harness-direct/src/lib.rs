//! Direkt-API-/Gateway-Harness (HAR-010, HAR-011): ein eigener Agent-Loop in Rust für
//! API-Keys und Gateways, als **zusätzliche Option** neben den Subscription-Harnesses. Kein
//! anderes Feature setzt ihn voraus (ADR-0034); ohne `providers:` in der User-Konfiguration
//! gibt es keinen `direct:*`-Harness und keine Netzverbindung (ADR-0033, Registry
//! `harness.direct`).
//!
//! - [`config`]: `providers:` prüfen (Pfad und Grund je Fehler, keine Klartext-Keys).
//! - [`secret`]: API-Keys, die ihren Wert nie preisgeben.
//! - [`wire`]: Anthropic Messages und OpenAI Chat Completions mit Streaming und Tool-Calls.
//! - [`http`]: Client ohne Weiterleitungen und impliziten Proxy, mit Retries.
//! - [`tools`]: Ausführung über MCP (`beton-workspace` im Prozess, Relays aus der Session).
//! - [`session`]: der Agent-Loop mit Gate je Tool-Call und eigener Compaction.
//! - [`models`]: Modell-Discovery mit Last-Known-Good-Cache.
//! - `mock` (Feature `mock`): lokaler HTTP/SSE-Mock-Server für Tests.

pub mod config;
pub mod http;
#[cfg(feature = "mock")]
pub mod mock;
pub mod models;
pub mod secret;
pub mod session;
pub mod tools;
pub mod wire;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use async_trait::async_trait;
use beton_core::event::{AuthSource, EventPayload, HarnessReady};
use beton_harness::registry::Registry;
use beton_harness::{
    AdapterContext, ApprovalMechanism, AuthStatus, Capabilities, CompactionSupport, ForkHistory,
    HarnessAdapter, HarnessError, HarnessId, HarnessSession, HostEnv, InstructionsDelivery, Mode,
    NormalizedEvent, ProbeReport, ResumeSupport, SessionSpec, Subagents, SwitchSupport,
    ToolCallGate, Transport, UsageReporting,
};
use serde_json::Value;
use tokio::sync::{Mutex, mpsc};

use crate::config::{ConfigProblem, KeyRef, Provider};
use crate::http::{Client, HttpOptions};
use crate::models::ModelCache;
use crate::secret::{ApiKey, KeySource};
use crate::session::{Conversation, DirectSession, LoopSettings, Shared};
use crate::tools::{McpTools, ToolHost};

/// Der Adapter für einen Provider (`direct:<name>`).
#[derive(Clone)]
pub struct DirectAdapter {
    pub provider: Provider,
    keys: Arc<dyn KeySource>,
    /// Modell-Discovery (nur im Daemon); `None` im Runner.
    discovery: Option<Arc<ModelCache>>,
    pub http: HttpOptions,
    pub settings: LoopSettings,
}

impl std::fmt::Debug for DirectAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectAdapter")
            .field("provider", &self.provider.name)
            .finish_non_exhaustive()
    }
}

/// Optionen beim Registrieren.
#[derive(Clone)]
pub struct DirectOptions {
    pub keys: Arc<dyn KeySource>,
    /// `GET /models` im Katalog (Daemon).
    pub discover_models: bool,
}

impl Default for DirectOptions {
    fn default() -> Self {
        Self {
            keys: Arc::new(secret::ProcessEnv),
            discover_models: false,
        }
    }
}

impl std::fmt::Debug for DirectOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectOptions")
            .field("discover_models", &self.discover_models)
            .finish_non_exhaustive()
    }
}

impl DirectAdapter {
    pub fn new(provider: Provider, options: &DirectOptions) -> Self {
        Self {
            provider,
            keys: options.keys.clone(),
            discovery: options
                .discover_models
                .then(|| Arc::new(ModelCache::default())),
            http: HttpOptions::default(),
            settings: LoopSettings::default(),
        }
    }

    fn auth_source(&self) -> AuthSource {
        match self.provider.key {
            KeyRef::Env(_) => AuthSource::ApiKey,
            KeyRef::None => AuthSource::Gateway,
        }
    }

    fn key(&self) -> Option<ApiKey> {
        match &self.provider.key {
            KeyRef::Env(var) => self.keys.get(var),
            KeyRef::None => None,
        }
    }

    /// Modelle für den Picker: konfigurierte zuerst, dann entdeckte; `stale`, wenn die
    /// Discovery gerade scheitert (HAR-011 AC3).
    pub fn models(&self) -> (Vec<String>, bool) {
        let mut out: Vec<String> = self.provider.models.iter().map(|m| m.id.clone()).collect();
        let (found, stale) = self
            .discovery
            .as_ref()
            .map(|c| c.snapshot())
            .unwrap_or_default();
        for m in found {
            if !out.contains(&m) {
                out.push(m);
            }
        }
        (out, stale)
    }

    /// Fragt `GET /models` ab, wenn fällig (Daemon, Katalog).
    pub async fn refresh_models(&self) {
        self.refresh_models_at(Instant::now()).await;
    }

    /// Wie [`Self::refresh_models`] mit vorgegebener Zeit (Tests).
    pub async fn refresh_models_at(&self, now: Instant) {
        let Some(cache) = &self.discovery else {
            return;
        };
        if !cache.due(now) {
            return;
        }
        let result = match Client::new(self.http) {
            Ok(client) => client
                .list_models(
                    &self.provider,
                    self.key().as_ref(),
                    models::DISCOVERY_TIMEOUT,
                )
                .await
                .map_err(|e| e.to_string()),
            Err(e) => Err(e),
        };
        cache.record(now, result);
    }

    /// Startet eine Session mit eigener Tool-Ausführung (Tests, Contract-Suite).
    pub async fn start_with_tools(
        &self,
        spec: SessionSpec,
        ctx: AdapterContext,
        tools: Arc<dyn ToolHost>,
        servers: Vec<String>,
        failures: Vec<EventPayload>,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        // HAR-027: nur der Default-Modus; andere werden abgelehnt statt ignoriert.
        if let Some(mode) = spec.permission_mode
            && mode != beton_harness::PermissionMode::Default
        {
            return Err(beton_harness::CapabilityUnsupported(
                beton_harness::Action::PermissionMode,
            )
            .into());
        }
        let key = match &self.provider.key {
            KeyRef::None => None,
            KeyRef::Env(var) => Some(self.keys.get(var).ok_or_else(|| {
                HarnessError::StartRefused(format!(
                    "API-Key fehlt: Die Umgebungsvariable {var} des Daemons ist nicht gesetzt \
                     (providers.{}.api_key_env).",
                    self.provider.name
                ))
            })?),
        };
        let model = spec
            .model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .or_else(|| self.provider.models.first().map(|m| m.id.clone()))
            .ok_or_else(|| {
                HarnessError::StartRefused(format!(
                    "kein Modell: providers.{}.models ist leer und die Session nennt keins",
                    self.provider.name
                ))
            })?;
        let client = Client::new(self.http).map_err(HarnessError::StartRefused)?;
        let (tx, rx) = mpsc::channel(4096);
        let settings = LoopSettings {
            max_turns: spec.max_turns.unwrap_or(self.settings.max_turns).max(1),
            ..self.settings
        };
        let capabilities = self.capabilities(Mode::Native, &ProbeReport::default());
        let shared = Arc::new(Shared {
            harness: self.provider.harness_id(),
            provider: self.provider.clone(),
            key,
            client,
            tools: tools.clone(),
            gate: ctx.gate.clone(),
            tx: tx.clone(),
            settings,
            system: session::system_prompt(&spec.workdir.display().to_string()),
            model: std::sync::Mutex::new(model),
            conv: Mutex::new(Conversation::default()),
            running: AtomicBool::new(false),
            auth_source: self.auth_source(),
            capabilities,
        });
        for f in failures {
            let _ = tx.send(NormalizedEvent::new(f, None)).await;
        }
        let _ = tx
            .send(NormalizedEvent::new(
                EventPayload::HarnessReady(HarnessReady {
                    harness_session_ref: None,
                    tools: tools.tools().into_iter().map(|t| t.model_name).collect(),
                    mcp_servers: servers,
                }),
                None,
            ))
            .await;
        Ok(Box::new(DirectSession {
            shared,
            rx: Some(rx),
            cancel: None,
        }))
    }
}

#[async_trait]
impl HarnessAdapter for DirectAdapter {
    fn id(&self) -> HarnessId {
        self.provider
            .harness_id()
            .parse()
            .unwrap_or_else(|_| unreachable!("Provider-Namen sind geprüft"))
    }

    fn modes(&self) -> &[Mode] {
        &[Mode::Native]
    }

    fn capabilities(&self, _mode: Mode, _probe: &ProbeReport) -> Capabilities {
        let (models, models_stale) = self.models();
        let priced = self.provider.models.iter().any(|m| m.pricing.is_some());
        Capabilities {
            mode: Mode::Native,
            transport: Transport::InProc,
            version_range: None,
            auth_sources: vec![self.auth_source()],
            // beton fragt vor jedem Tool-Call selbst das Gate (kein Vendor-Mechanismus).
            approval: ApprovalMechanism::NativeRequest,
            tool_call_gate: ToolCallGate::Full,
            model_switch: SwitchSupport::Live,
            effort_switch: SwitchSupport::None,
            resume: ResumeSupport::None,
            fork_history: ForkHistory::Preamble,
            interrupt: true,
            steering: false,
            subagents: Subagents::None,
            usage_reporting: if priced {
                UsageReporting::TokensAndCost
            } else {
                UsageReporting::Tokens
            },
            // Der Loop kompaktiert selbst (HAR-010, HAR-022).
            compaction: CompactionSupport::Native,
            instructions_delivery: InstructionsDelivery::SystemPrompt,
            mcp_injection: true,
            images: false,
            transcript_import: false,
            models,
            models_stale,
            efforts: Vec::new(),
            // Jeder Tool-Call geht durch das Gate; weitere Modi folgen mit der Policy-Engine.
            permission_modes: vec![beton_harness::PermissionMode::Default],
            // Kleinstes konfiguriertes Fenster: das Handover-Budget (HAR-018) passt dann auf
            // jedes Modell des Providers; ohne Angabe gilt der Default.
            context_window: self
                .provider
                .models
                .iter()
                .filter_map(|m| m.context_window)
                .min(),
        }
    }

    async fn probe(&self, _env: &HostEnv) -> ProbeReport {
        self.refresh_models().await;
        ProbeReport {
            installed: true,
            path: Some(self.provider.base_url.to_string()),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            auth_status: self.auth_status_now(),
            probe_failed: None,
        }
    }

    async fn auth_status(&self, _env: &HostEnv) -> AuthStatus {
        self.auth_status_now()
    }

    async fn start(
        &self,
        spec: SessionSpec,
        ctx: AdapterContext,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        // Key und Modell vor dem Start der MCP-Server prüfen (fail closed, ohne Seiteneffekte).
        if let KeyRef::Env(var) = &self.provider.key
            && self.keys.get(var).is_none()
        {
            return Err(HarnessError::StartRefused(format!(
                "API-Key fehlt: Die Umgebungsvariable {var} des Daemons ist nicht gesetzt \
                 (providers.{}.api_key_env).",
                self.provider.name
            )));
        }
        let (tools, failures) =
            McpTools::connect(&spec.workdir, &spec.mcp.servers, ctx.launcher.as_ref()).await;
        let servers = tools.server_names();
        let tools: Arc<dyn ToolHost> = Arc::new(tools);
        match self
            .start_with_tools(spec, ctx, tools.clone(), servers, failures)
            .await
        {
            Ok(s) => Ok(s),
            Err(e) => {
                tools.shutdown().await;
                Err(e)
            }
        }
    }
}

impl DirectAdapter {
    fn auth_status_now(&self) -> AuthStatus {
        match &self.provider.key {
            KeyRef::None => AuthStatus::NotApplicable,
            KeyRef::Env(var) => {
                if self.keys.get(var).is_some() {
                    AuthStatus::LoggedIn
                } else {
                    AuthStatus::LoggedOut
                }
            }
        }
    }
}

/// Registriert je gültigem Provider einen Harness `direct:<name>`; ungültige Einträge werden
/// mit Pfad und Grund gemeldet und übersprungen (HAR-011 AC4).
pub fn register(
    registry: &mut Registry,
    providers: &BTreeMap<String, Value>,
    options: &DirectOptions,
) -> Vec<ConfigProblem> {
    let (ok, problems) = config::parse_providers(providers);
    for p in ok {
        registry.register(Arc::new(DirectAdapter::new(p, options)));
    }
    problems
}

/// Variablen, deren Werte der Daemon einer Session dieses Harness mitgeben muss (über
/// stdin, nie über Env oder argv).
pub fn key_vars(harness: &str, providers: &BTreeMap<String, Value>) -> Vec<String> {
    let Some(name) = harness.strip_prefix("direct:") else {
        return Vec::new();
    };
    match providers.get(name).map(|v| config::parse_provider(name, v)) {
        Some(Ok(Provider {
            key: KeyRef::Env(var),
            ..
        })) => vec![var],
        _ => Vec::new(),
    }
}
