//! Registry der Backends pro Plattform (SBX-001) und die Spawn-Fassade, die zu jedem
//! erfolgreichen Start genau ein `sandbox.started` meldet (AC4).

use std::collections::BTreeMap;
use std::sync::Arc;

use beton_core::event::{SandboxStarted, SandboxViolation};

use crate::child::SandboxedChild;
use crate::command::CommandSpec;
use crate::error::SandboxError;
use crate::none::NoneProvider;
use crate::provider::{ProbeReport, ProviderId, SandboxProvider};
use crate::resolve::ResolvedPolicy;
use crate::spec::BackendChoice;

/// Betriebssystem (für `backend: auto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Linux,
    Windows,
    Other,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Other
        }
    }
}

/// Bevorzugtes Backend für `auto` (SBX-001 AC3). Linux: `landlock`; ob stattdessen `bwrap`
/// greift, entscheidet [`Registry::auto_choice`] anhand der Probe (SBX-009).
pub fn auto_backend(os: Os) -> Option<ProviderId> {
    match os {
        Os::MacOs => Some(ProviderId::Seatbelt),
        Os::Linux => Some(ProviderId::Landlock),
        Os::Windows => Some(ProviderId::Windows),
        Os::Other => None,
    }
}

/// Nur Debug-Builds (Tests): Backends, die die Registry als nicht verfügbar meldet (`all`
/// oder IDs mit Komma, z. B. `landlock,bwrap`).
pub const TEST_UNAVAILABLE_ENV: &str = "BETON_TEST_SANDBOX_UNAVAILABLE";

/// Ein registriertes, auf dieser Plattform (noch) nicht nutzbares Backend.
#[derive(Debug, Clone)]
pub struct UnavailableProvider {
    pub id: ProviderId,
    pub reason: String,
}

impl SandboxProvider for UnavailableProvider {
    fn id(&self) -> ProviderId {
        self.id
    }

    fn probe(&self) -> ProbeReport {
        ProbeReport::unavailable(self.id, self.reason.clone())
    }

    fn spawn(
        &self,
        policy: &ResolvedPolicy,
        _cmd: CommandSpec,
    ) -> Result<SandboxedChild, SandboxError> {
        Err(SandboxError::Unavailable {
            provider: self.id,
            missing: policy.required.clone(),
            hint: self.reason.clone(),
        })
    }
}

/// Alle registrierten Backends.
#[derive(Clone)]
pub struct Registry {
    os: Os,
    providers: BTreeMap<ProviderId, Arc<dyn SandboxProvider>>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("os", &self.os)
            .field("providers", &self.providers.keys().collect::<Vec<_>>())
            .finish()
    }
}

fn unavailable(id: ProviderId, reason: &str) -> Arc<dyn SandboxProvider> {
    Arc::new(UnavailableProvider {
        id,
        reason: reason.to_owned(),
    })
}

impl Registry {
    /// Leere Registry (Tests).
    pub fn empty(os: Os) -> Self {
        Self {
            os,
            providers: BTreeMap::new(),
        }
    }

    /// Die Backends dieser Plattform. Was hier (noch) nicht isolieren kann, ist registriert,
    /// meldet aber `unavailable` mit Grund: kein stiller Fallback (SBX-006). Unter Linux ist
    /// das laufende Programm (`beton`) der Launcher (`beton __sandbox-exec`).
    pub fn platform() -> Self {
        let exe = std::env::current_exe().unwrap_or_default();
        Self::with_launcher(&exe)
    }

    /// Wie [`Registry::platform`], mit ausdrücklichem Linux-Launcher (Tests).
    pub fn with_launcher(launcher: &std::path::Path) -> Self {
        let os = Os::current();
        let mut r = Self::empty(os);
        r.register(crate::seatbelt::provider());
        #[cfg(target_os = "linux")]
        {
            let (landlock, bwrap) = crate::linux::sys::provider::LinuxProvider::pair(
                crate::linux::LinuxConfig::new(launcher),
            );
            r.register(Arc::new(landlock));
            r.register(Arc::new(bwrap));
        }
        #[cfg(not(target_os = "linux"))]
        {
            let cfg = crate::linux::LinuxConfig::new(launcher);
            r.register(crate::linux::landlock_provider(cfg.clone()));
            r.register(crate::linux::bwrap_provider(cfg));
        }
        r.register(unavailable(
            ProviderId::Windows,
            if os == Os::Windows {
                "Windows-Backend (Beta) folgt mit SBX-016 (M5)"
            } else {
                "nur unter Windows"
            },
        ));
        for id in [ProviderId::Docker, ProviderId::Podman] {
            r.register(unavailable(id, "Container-Backend folgt mit SBX-015 (M5)"));
        }
        r.register(Arc::new(NoneProvider));
        #[cfg(debug_assertions)]
        r.disable_for_tests();
        r
    }

    /// Nur Debug-Builds: [`TEST_UNAVAILABLE_ENV`] schaltet Backends ab (`all` bzw. IDs mit
    /// Komma), z. B. „Linux ohne Landlock und bwrap“ für SBX-006 AC1. Der Schalter macht die
    /// Sandbox nie verfügbarer als sie ist; `none` bleibt, wie es ist.
    #[cfg(debug_assertions)]
    fn disable_for_tests(&mut self) {
        let Ok(list) = std::env::var(TEST_UNAVAILABLE_ENV) else {
            return;
        };
        let ids: Vec<ProviderId> = self
            .providers
            .keys()
            .copied()
            .filter(|id| *id != ProviderId::None)
            .filter(|id| {
                list.split(',')
                    .map(str::trim)
                    .any(|w| w == "all" || w == id.as_str())
            })
            .collect();
        for id in ids {
            self.register(unavailable(
                id,
                &format!("abgeschaltet über {TEST_UNAVAILABLE_ENV} (Test)"),
            ));
        }
    }

    /// Das Backend für `auto` auf diesem Rechner. Linux: `landlock`; sind unprivilegierte
    /// User-Namespaces gesperrt, aber bwrap nutzbar, `bwrap` (SBX-009 AC1). Ist keins nutzbar,
    /// bleibt es bei `landlock`, dessen Start dann mit Grund scheitert (kein Fallback auf
    /// `none`).
    pub fn auto_choice(&self) -> Option<ProviderId> {
        let first = auto_backend(self.os)?;
        if first == ProviderId::Landlock {
            let usable = |id| self.get(id).is_some_and(|p| p.probe().available);
            if !usable(ProviderId::Landlock) && usable(ProviderId::Bwrap) {
                return Some(ProviderId::Bwrap);
            }
        }
        Some(first)
    }

    pub fn register(&mut self, provider: Arc<dyn SandboxProvider>) {
        self.providers.insert(provider.id(), provider);
    }

    pub fn os(&self) -> Os {
        self.os
    }

    pub fn get(&self, id: ProviderId) -> Option<Arc<dyn SandboxProvider>> {
        self.providers.get(&id).cloned()
    }

    /// Backend zu `choice`. `auto` wählt nach Betriebssystem; ein nicht registriertes Backend
    /// ist [`SandboxError::Unavailable`]. `none` wird nie automatisch gewählt.
    pub fn select(&self, choice: BackendChoice) -> Result<Arc<dyn SandboxProvider>, SandboxError> {
        let id = match choice {
            BackendChoice::Auto => self
                .auto_choice()
                .ok_or_else(|| SandboxError::Unavailable {
                    provider: ProviderId::None,
                    missing: crate::caps::required_default(),
                    hint: "für dieses Betriebssystem gibt es kein Sandbox-Backend".into(),
                })?,
            BackendChoice::Provider(id) => id,
        };
        self.get(id).ok_or_else(|| SandboxError::Unavailable {
            provider: id,
            missing: crate::caps::required_default(),
            hint: "Backend ist nicht registriert".into(),
        })
    }

    /// Probe aller registrierten Backends (sortiert nach ID).
    pub fn probe_all(&self) -> Vec<ProbeReport> {
        self.providers.values().map(|p| p.probe()).collect()
    }
}

/// Empfänger der `sandbox.started`-Events.
pub type EventSink = Arc<dyn Fn(SandboxStarted) + Send + Sync>;

/// Empfänger der `sandbox.violation`-Events (SBX-013 AC1, SBX-014).
pub type ViolationSink = Arc<dyn Fn(SandboxViolation) + Send + Sync>;

/// Eine aufgelöste Sandbox einer Stufe: Provider, Policy und Event-Empfänger. Jeder
/// erfolgreiche [`Sandbox::spawn`] meldet genau ein `sandbox.started`, ein gescheiterter
/// keines.
#[derive(Clone)]
pub struct Sandbox {
    provider: Arc<dyn SandboxProvider>,
    policy: ResolvedPolicy,
    events: Option<EventSink>,
    violations: Option<ViolationSink>,
}

impl std::fmt::Debug for Sandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sandbox")
            .field("provider", &self.provider.id())
            .field("stage", &self.policy.stage)
            .finish_non_exhaustive()
    }
}

impl Sandbox {
    pub fn new(
        provider: Arc<dyn SandboxProvider>,
        policy: ResolvedPolicy,
        events: Option<EventSink>,
    ) -> Self {
        Self {
            provider,
            policy,
            events,
            violations: None,
        }
    }

    /// Meldet erkannte Verstöße (z. B. ein erreichtes Limit) als `sandbox.violation`.
    pub fn with_violations(mut self, sink: ViolationSink) -> Self {
        self.violations = Some(sink);
        self
    }

    /// Ohne Isolation (`backend: none`, ausdrücklich), z. B. bis SBX-006 scharf ist.
    pub fn unsandboxed(stage: crate::Stage, events: Option<EventSink>) -> Self {
        Self::new(
            Arc::new(NoneProvider),
            ResolvedPolicy::unsandboxed(stage),
            events,
        )
    }

    pub fn policy(&self) -> &ResolvedPolicy {
        &self.policy
    }

    /// Das Event, das ein erfolgreicher Start dieser Sandbox meldet.
    pub fn started_event(&self) -> SandboxStarted {
        let caps: Vec<&str> = self
            .provider
            .probe()
            .caps
            .iter()
            .map(|c| c.as_str())
            .collect();
        SandboxStarted {
            stage: self.policy.stage.event(),
            backend: self.policy.backend.as_str().to_owned(),
            caps: serde_json::json!(caps),
            degraded: !self.policy.degraded.is_empty() || self.policy.inherited_tools,
        }
    }

    pub fn spawn(&self, cmd: CommandSpec) -> Result<SandboxedChild, SandboxError> {
        let child = self
            .provider
            .spawn_observed(&self.policy, cmd, self.violations.clone())?;
        if let Some(events) = &self.events {
            events(self.started_event());
        }
        Ok(child)
    }
}
