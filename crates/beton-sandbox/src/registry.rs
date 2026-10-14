//! Registry der Backends pro Plattform (SBX-001) und die Spawn-Fassade, die zu jedem
//! erfolgreichen Start genau ein `sandbox.started` meldet (AC4).

use std::collections::BTreeMap;
use std::sync::Arc;

use beton_core::event::SandboxStarted;

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

/// Backend für `auto` (SBX-001 AC3). Linux: `landlock`; ob stattdessen `bwrap` greift,
/// entscheidet SBX-009 anhand der Probe.
pub fn auto_backend(os: Os) -> Option<ProviderId> {
        todo!("SBX-001")
    }

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

    fn spawn(&self, policy: &ResolvedPolicy, _cmd: CommandSpec) -> Result<SandboxedChild, SandboxError> {
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
    /// meldet aber `unavailable` mit Grund: kein stiller Fallback (SBX-006).
    pub fn platform() -> Self {
        let os = Os::current();
        let mut r = Self::empty(os);
        r.register(crate::seatbelt::provider());
        let linux = |what: &str| -> String {
            if os == Os::Linux {
                format!("noch nicht umgesetzt ({what})")
            } else {
                "nur unter Linux".to_owned()
            }
        };
        r.register(unavailable(ProviderId::Landlock, &linux("SBX-008")));
        r.register(unavailable(ProviderId::Bwrap, &linux("SBX-009")));
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
        r
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
        todo!("SBX-001")
    }

    /// Probe aller registrierten Backends (sortiert nach ID).
    pub fn probe_all(&self) -> Vec<ProbeReport> {
        todo!("SBX-001")
    }
}

/// Empfänger der `sandbox.started`-Events.
pub type EventSink = Arc<dyn Fn(SandboxStarted) + Send + Sync>;

/// Eine aufgelöste Sandbox einer Stufe: Provider, Policy und Event-Empfänger. Jeder
/// erfolgreiche [`Sandbox::spawn`] meldet genau ein `sandbox.started`, ein gescheiterter
/// keines.
#[derive(Clone)]
pub struct Sandbox {
    provider: Arc<dyn SandboxProvider>,
    policy: ResolvedPolicy,
    events: Option<EventSink>,
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
        }
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
        todo!("SBX-001")
    }

    pub fn spawn(&self, cmd: CommandSpec) -> Result<SandboxedChild, SandboxError> {
        let _ = &self.events;
        self.provider.spawn(&self.policy, cmd)
    }
}
