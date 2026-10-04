//! Das Nötigste aus dem Model Context Protocol (JSON-RPC 2.0, zeilenweise über stdio):
//! Nachrichten klassifizieren, Antworten und Fehler bauen, Protokollversion aushandeln.

use serde_json::{Value, json};

/// Protokollversionen, die der `beton`-Server spricht (neueste zuerst). Claude Code 2.1.285
/// fragt `2025-11-25`, Codex 0.153.2 `2025-06-18` an (verifiziert ohne Modellaufruf).
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// JSON-RPC-Fehlercodes.
pub mod codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL: i64 = -32603;
    /// Server nicht erreichbar bzw. Start gescheitert (implementierungsdefiniert).
    pub const SERVER_UNAVAILABLE: i64 = -32000;
}

/// Art einer JSON-RPC-Nachricht.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Request,
    Notification,
    Response,
    Invalid,
}

pub fn kind(msg: &Value) -> Kind {
    let has_id = msg.get("id").is_some_and(|id| !id.is_null());
    let has_method = msg.get("method").and_then(Value::as_str).is_some();
    match (has_id, has_method) {
        (true, true) => Kind::Request,
        (false, true) => Kind::Notification,
        (true, false) if msg.get("result").is_some() || msg.get("error").is_some() => {
            Kind::Response
        }
        _ => Kind::Invalid,
    }
}

pub fn result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

pub fn error(id: &Value, code: i64, message: impl Into<String>, data: Option<Value>) -> Value {
    let mut err = json!({"code": code, "message": message.into()});
    if let Some(d) = data {
        err["data"] = d;
    }
    json!({"jsonrpc": "2.0", "id": id, "error": err})
}

/// Die vom Client angefragte Version, falls unterstützt, sonst die neueste eigene.
pub fn negotiate(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|r| PROTOCOL_VERSIONS.iter().find(|v| **v == r))
        .copied()
        .unwrap_or(PROTOCOL_VERSIONS[0])
}

/// Ergebnis eines `tools/call` mit Textinhalt.
pub fn text_result(text: impl Into<String>, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text.into()}], "isError": is_error})
}

/// Ergebnis eines `tools/call` mit strukturiertem Inhalt (zusätzlich als JSON-Text).
pub fn structured_result(value: &Value) -> Value {
    json!({
        "content": [{"type": "text", "text": value.to_string()}],
        "structuredContent": value,
        "isError": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_messages() {
        assert_eq!(
            kind(&json!({"jsonrpc":"2.0","id":1,"method":"x"})),
            Kind::Request
        );
        assert_eq!(
            kind(&json!({"jsonrpc":"2.0","method":"x"})),
            Kind::Notification
        );
        assert_eq!(
            kind(&json!({"jsonrpc":"2.0","id":1,"result":{}})),
            Kind::Response
        );
        assert_eq!(
            kind(&json!({"jsonrpc":"2.0","id":1,"error":{}})),
            Kind::Response
        );
        assert_eq!(kind(&json!({"id":1})), Kind::Invalid);
        assert_eq!(kind(&json!(5)), Kind::Invalid);
    }

    #[test]
    fn negotiates_versions() {
        assert_eq!(negotiate(Some("2025-06-18")), "2025-06-18");
        assert_eq!(negotiate(Some("1999-01-01")), PROTOCOL_VERSIONS[0]);
        assert_eq!(negotiate(None), PROTOCOL_VERSIONS[0]);
    }
}
