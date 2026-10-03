//! Client-Kommandos über WebSocket (PROTO-006).
//!
//! Jedes Kommando hat einen REST-Zwilling mit identischer Semantik und Autorisierung
//! (AC3). Die Kommandos selbst (`input.submit`, `turn.interrupt`, `approval.resolve` …)
//! kommen mit dem Session-Lebenszyklus (WP-09); hier liegt der Rahmen.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use beton_core::id::SessionId;
use serde_json::Value;

use crate::problem::Problem;
use crate::security::Authenticated;

/// Kontext eines Kommandos.
#[derive(Clone)]
pub struct CommandCtx {
    pub state: crate::app::AppState,
    pub auth: Authenticated,
}

#[async_trait]
pub trait Command: Send + Sync {
    fn name(&self) -> &'static str;
    /// REST-Zwilling als (Methode, OpenAPI-Pfad), z. B. `("post", "/v1/sessions/{id}/input")`.
    fn rest_twin(&self) -> (&'static str, &'static str);
    async fn run(
        &self,
        ctx: &CommandCtx,
        session: Option<SessionId>,
        args: Value,
    ) -> Result<Value, Problem>;
}

/// Alle bekannten Kommandos.
#[derive(Clone, Default)]
pub struct CommandRegistry {
    commands: HashMap<&'static str, Arc<dyn Command>>,
}

impl std::fmt::Debug for CommandRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.commands.keys()).finish()
    }
}

impl CommandRegistry {
    pub fn register(&mut self, command: Arc<dyn Command>) {
        self.commands.insert(command.name(), command);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Command>> {
        self.commands.get(name)
    }

    /// PROTO-006 AC3: Zu jedem Kommando existiert der REST-Zwilling im OpenAPI-Dokument.
    pub fn missing_rest_twins(&self, openapi: &Value) -> Vec<String> {
        let mut missing: Vec<String> = self
            .commands
            .values()
            .filter_map(|c| {
                let (method, path) = c.rest_twin();
                openapi["paths"][path][method]
                    .is_null()
                    .then(|| format!("{} → {} {path}", c.name(), method.to_uppercase()))
            })
            .collect();
        missing.sort();
        missing
    }
}
