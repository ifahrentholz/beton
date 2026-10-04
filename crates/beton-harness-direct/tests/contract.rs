//! Harness-Contract-Suite (QA-016) für den Direkt-API-Harness: dieselben Szenarien wie für
//! Claude, Codex und ACP, abgespielt vom lokalen Mock-Server in beiden Wire-Formaten.

#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use beton_harness::contract::{self, ContractSubject, Outcome};
use beton_harness::process::RealLauncher;
use beton_harness::scenario::Scenario;
use beton_harness::{
    AdapterContext, Capabilities, Gate, HarnessAdapter, HarnessError, HarnessSession, HostEnv,
    Mode, ProbeReport, SessionSpec,
};
use beton_harness_direct::config::{KeyRef, ModelConfig, Provider, WireKind};
use beton_harness_direct::mock::{self, MockServer};
use beton_harness_direct::tools::{ToolHost, ToolInfo, ToolOutcome};
use beton_harness_direct::{DirectAdapter, DirectOptions};
use serde_json::{Value, json};

/// Führt jedes Tool der Szenarien aus (`Bash` & Co.) und meldet Erfolg.
struct ScenarioTools;

#[async_trait]
impl ToolHost for ScenarioTools {
    fn tools(&self) -> Vec<ToolInfo> {
        vec![ToolInfo {
            model_name: "Bash".into(),
            tool: "Bash".into(),
            server: "contract".into(),
            description: "Shell".into(),
            schema: json!({"type": "object"}),
            kind: "shell".into(),
            source: beton_core::event::ToolSource::Harness,
        }]
    }
    async fn call(&self, _name: &str, _args: Value) -> ToolOutcome {
        ToolOutcome {
            ok: true,
            text: "gepusht".into(),
        }
    }
}

struct DirectSubject {
    wire: WireKind,
    servers: Mutex<Vec<MockServer>>,
}

fn adapter(wire: WireKind, base: &str) -> DirectAdapter {
    DirectAdapter::new(
        Provider {
            name: "contract".into(),
            wire,
            base_url: base.parse().unwrap(),
            key: KeyRef::None,
            prompt_caching: false,
            models: vec![ModelConfig {
                id: "mock-model".into(),
                context_window: None,
                pricing: None,
            }],
        },
        &DirectOptions::default(),
    )
}

#[async_trait]
impl ContractSubject for DirectSubject {
    fn name(&self) -> String {
        format!("direct:contract ({:?})", self.wire)
    }

    async fn capabilities(&self, _scenario: &Path) -> Capabilities {
        adapter(self.wire, "http://127.0.0.1:9").capabilities(Mode::Native, &ProbeReport::default())
    }

    async fn start(
        &self,
        scenario: &Path,
        _resume: Option<String>,
        gate: Arc<dyn Gate>,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        let sc = Scenario::load(scenario).map_err(|e| HarnessError::StartRefused(e.to_string()))?;
        let server = MockServer::start().await?;
        for reply in mock::from_scenario(self.wire, &sc) {
            server.push(mock::messages_path(self.wire), reply);
        }
        let a = adapter(self.wire, &server.base_url(mock::base_suffix(self.wire)));
        self.servers.lock().unwrap().push(server);
        a.start_with_tools(
            SessionSpec {
                workdir: std::env::temp_dir(),
                ..SessionSpec::default()
            },
            AdapterContext {
                gate,
                launcher: Arc::new(RealLauncher),
                env: HostEnv::default(),
            },
            Arc::new(ScenarioTools),
            Vec::new(),
            Vec::new(),
        )
        .await
    }
}

async fn passes(wire: WireKind) {
    let subject = DirectSubject {
        wire,
        servers: Mutex::default(),
    };
    let dir = tempfile::tempdir().unwrap();
    let report = contract::run(&subject, dir.path()).await;
    assert!(report.is_ok(), "{}", report.summary());
    for check in [
        "streaming",
        "usage_reporting",
        "approval_allow",
        "approval_deny",
        "interrupt",
        "model_switch",
        "turn_error",
        "auth_expired",
    ] {
        assert_eq!(
            report.outcome(check),
            Some(&Outcome::Passed),
            "{check}: {}",
            report.summary()
        );
    }
    // Ohne Resume (M1) und ohne eigenen Prozess: korrekt übersprungen.
    assert!(matches!(
        report.outcome("resume"),
        Some(Outcome::Skipped(_))
    ));
    assert!(matches!(report.outcome("crash"), Some(Outcome::Skipped(_))));
}

#[tokio::test]
async fn qa_016_ac2_direct_adapter_passes_the_suite_anthropic_wire() {
    passes(WireKind::Anthropic).await;
}

#[tokio::test]
async fn qa_016_ac2_direct_adapter_passes_the_suite_openai_wire() {
    passes(WireKind::Openai).await;
}
