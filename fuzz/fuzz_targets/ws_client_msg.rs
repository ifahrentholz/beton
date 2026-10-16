//! QA-009: Nachrichten-Decoder des Daemon-WebSockets (PROTO-005, PROTO-006).
//!
//! Die WS-Frames selbst dekodiert tungstenite (über axum, `beton-server/src/ws.rs`); beton
//! dekodiert den Inhalt jedes Text-Frames als `beton_proto::ws::ClientMsg`
//! (`serde_json::from_str`). Das ist die eigene Schicht über dem Frame-Decoder.
//!
//! Invarianten: kein Panic; eine dekodierte Nachricht übersteht Kodieren und erneutes
//! Dekodieren unverändert (ohne Gleitkommazahlen, die serde_json nicht bitgenau
//! zurückliest).

#![no_main]

use beton_proto::ws::ClientMsg;
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fn has_float(v: &Value) -> bool {
    match v {
        Value::Number(n) => n.is_f64(),
        Value::Array(a) => a.iter().any(has_float),
        Value::Object(o) => o.values().any(has_float),
        _ => false,
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(msg) = serde_json::from_str::<ClientMsg>(text) else {
        return;
    };
    let encoded = serde_json::to_string(&msg).expect("ClientMsg lässt sich kodieren");
    let again: ClientMsg =
        serde_json::from_str(&encoded).expect("kodierte ClientMsg lässt sich wieder lesen");
    let value: Value = serde_json::from_str(&encoded).expect("JSON");
    if !has_float(&value) {
        assert_eq!(msg, again, "ClientMsg übersteht Kodieren/Dekodieren nicht");
    }
});
