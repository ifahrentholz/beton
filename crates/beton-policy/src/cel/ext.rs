//! Platzhalter.
pub trait PathResolver: Send + Sync { fn resolve(&self, p: &str) -> Result<String, String>; }
pub struct LexicalResolver;
impl PathResolver for LexicalResolver { fn resolve(&self, p: &str) -> Result<String, String> { Ok(p.into()) } }
pub fn parse_duration(_s: &str) -> Result<time::Duration, String> { Err("todo".into()) }
