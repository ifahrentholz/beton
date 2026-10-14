//! Policy-Engine von beton (03-policies.md).

pub mod audit;
pub mod cel;
pub mod context;
pub mod doc;
pub mod enforcement;
pub mod engine;
pub mod load;
pub mod normalize;
pub mod store;
pub mod types;
pub mod value;
pub mod vars;
mod yaml;

use serde::{Deserialize, Serialize};

/// Hook-Phase, an der Policies entscheiden (POL-003).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    SessionStart,
    ModelRequest,
    ToolCall,
    ToolResult,
    BrowserNavigate,
    BrowserAction,
}

impl Phase {
    pub const ALL: [Phase; 6] = [
        Phase::SessionStart,
        Phase::ModelRequest,
        Phase::ToolCall,
        Phase::ToolResult,
        Phase::BrowserNavigate,
        Phase::BrowserAction,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Phase::SessionStart => "session_start",
            Phase::ModelRequest => "model_request",
            Phase::ToolCall => "tool_call",
            Phase::ToolResult => "tool_result",
            Phase::BrowserNavigate => "browser_navigate",
            Phase::BrowserAction => "browser_action",
        }
    }
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
