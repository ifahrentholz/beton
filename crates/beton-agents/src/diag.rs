//! Befunde der Validierung (AGT-002 AC2): Datei, Feldpfad, Zeile, Spalte, Code, Meldung.

use std::fmt;

use serde::Serialize;

/// Schwere eines Befunds. Fehler machen den Agent ungültig, Warnungen nicht.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

/// Stabile Codes der Befunde. Neue Codes werden angehängt, bestehende nicht umbenannt.
pub mod code {
    pub const YAML_SYNTAX: &str = "yaml_syntax";
    pub const DUPLICATE_KEY: &str = "duplicate_key";
    pub const UNKNOWN_FIELD: &str = "unknown_field";
    pub const MISSING_FIELD: &str = "missing_field";
    pub const INVALID_VALUE: &str = "invalid_value";
    pub const UNKNOWN_HARNESS: &str = "unknown_harness";
    pub const UNSUPPORTED_SPEC_VERSION: &str = "unsupported_spec_version";
    pub const AGENT_NOT_FOUND: &str = "agent_not_found";
    pub const MISSING_AGENT_YAML: &str = "missing_agent_yaml";
    pub const NAME_MISMATCH: &str = "name_mismatch";
    pub const HARNESS_UNAVAILABLE: &str = "harness_unavailable";
    pub const EFFORT_MAPPED: &str = "effort_mapped";
    pub const FILE_NOT_FOUND: &str = "file_not_found";
    pub const PATH_OUTSIDE_AGENT: &str = "path_outside_agent";
    pub const SKILL_NOT_FOUND: &str = "skill_not_found";
    pub const UNKNOWN_SUBAGENT: &str = "unknown_subagent";
    pub const AGENT_CYCLE: &str = "agent_cycle";
    pub const CONFLICTING_FIELDS: &str = "conflicting_fields";
    pub const INVALID_PARAM: &str = "invalid_param";
}

/// Ein Befund. `path` ist der Feldpfad (`executor.harness`, `policies[1]`), leer für die
/// ganze Datei; `line`/`column` sind 1-basiert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// Datei relativ zum geprüften Agent (`agent.yaml`, `agents/reviewer/agent.yaml`).
    pub file: String,
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub code: &'static str,
    pub message: String,
    pub severity: Severity,
}

impl Diagnostic {
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}  {}  ",
            self.file, self.line, self.column, self.code
        )?;
        if !self.path.is_empty() {
            write!(f, "{}: ", self.path)?;
        }
        f.write_str(&self.message)
    }
}

/// Levenshtein-Abstand (für „Meintest du …?“).
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Ähnlichster Kandidat, sofern er plausibel ist (Abstand ≤ ein Drittel der Länge, mind. 2,
/// oder ein Präfix-Verhältnis wie `claude-code` → `claude`).
pub fn suggest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    candidates
        .into_iter()
        .map(|c| (distance(word, c), c))
        .filter(|(d, c)| {
            *d <= (word.chars().count().max(c.chars().count()) / 3).max(2)
                || (c.len() >= 4 && (word.starts_with(c) || c.starts_with(word)))
        })
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_find_typos_but_not_unrelated_words() {
        let fields = ["instructions", "executor", "params", "name"];
        assert_eq!(suggest("instruction", fields), Some("instructions"));
        assert_eq!(suggest("exector", fields), Some("executor"));
        assert_eq!(suggest("zzzzzz", fields), None);
        assert_eq!(
            suggest("claude-code", ["claude", "codex", "fake"]),
            Some("claude")
        );
    }
}
