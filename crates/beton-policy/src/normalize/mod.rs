//! Tool-Normalisierung (POL-005). Platzhalter.

pub mod shell;

#[derive(Debug, Clone, Copy)]
pub struct RawToolCall<'a> {
    pub harness: &'a str,
    pub native_name: &'a str,
    pub acp_kind: Option<&'a str>,
    pub mcp_server: Option<&'a str>,
    pub args: &'a serde_json::Value,
    pub cwd: &'a str,
    pub home: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct NormalizeError(pub String);

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedTool {
    pub name: String,
    pub native_name: String,
    pub server: String,
    pub kind: String,
    pub args: serde_json::Value,
    pub command: Option<String>,
    pub argv: Vec<Vec<String>>,
    pub cwd: String,
    pub paths: Vec<String>,
    pub urls: Vec<String>,
    pub is_subagent_spawn: bool,
    pub parse_uncertain: bool,
}

impl NormalizedTool {
    pub fn to_context(&self) -> serde_json::Value { serde_json::Value::Null }
}

pub struct ToolMap;
impl ToolMap {
    pub fn kind_of(&self, _h: &str, _n: &str, _k: Option<&str>, _s: Option<&str>) -> Option<&str> { None }
}

pub fn tool_map() -> Result<&'static ToolMap, NormalizeError> { Err(NormalizeError("todo".into())) }
pub fn normalize(_raw: &RawToolCall) -> Result<NormalizedTool, NormalizeError> { Err(NormalizeError("todo".into())) }
