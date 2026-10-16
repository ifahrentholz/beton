//! Gemeinsame Helfer der Integrationstests mit Daemon und Runner.
//!
//! Diese Tests laufen ausdrücklich ohne Sandbox (`sandbox.backend: none`, SBX-006) und
//! bestätigen das für jede neue Session; die Sandbox prüfen eigene Tests (`stages.rs`,
//! `beton-sandbox`, `beton-cli/tests/m2_demo.rs`).

#![allow(dead_code)]

use serde_json::Value;

/// `sandbox:` der Test-Daemons: ausdrücklich `backend: none`.
pub fn unsandboxed() -> beton_sandbox::SandboxSpec {
    beton_sandbox::SandboxSpec::from_yaml("backend: none").expect("backend: none")
}

/// Ergänzt `POST /v1/sessions` und `POST /v1/sessions/{id}/fork` um die Bestätigung von
/// `backend: none` (SBX-006).
pub fn confirm_unsandboxed(method: &str, path: &str, body: Option<Value>) -> Option<Value> {
    let creates =
        path == "/v1/sessions" || (path.starts_with("/v1/sessions/") && path.ends_with("/fork"));
    match body {
        Some(Value::Object(mut b)) if method == "POST" && creates => {
            b.entry("confirm_unsandboxed").or_insert(Value::Bool(true));
            Some(Value::Object(b))
        }
        other => other,
    }
}

/// Bestätigt `backend: none` für eine direkt im Store angelegte (oder importierte) Session,
/// wie es `POST /v1/sessions` mit `confirm_unsandboxed` tut: Audit-Eintrag
/// `sandbox.backend_none` (SBX-006).
pub async fn confirm_stored(store: &beton_store::Store, session: beton_core::id::SessionId) {
    store
        .append_audit(
            beton_core::id::OrgId::LOCAL,
            beton_store::NewAuditEntry {
                at: beton_core::time::Timestamp::now(),
                actor: beton_core::id::PrincipalId::User(beton_core::id::UserId::LOCAL),
                action: beton_server::sessions::UNSANDBOXED_ACTION.into(),
                target_kind: "session".into(),
                target_id: session.to_string(),
                session_id: Some(session),
                host_id: None,
                details: serde_json::json!({"confirmed": "test"}),
            },
        )
        .await
        .expect("Audit-Eintrag");
}

/// beton-CA für Test-Daemons mit Stufenmodell (PRX-002): je Abruf neu erzeugt, nur im
/// Speicher (kein Secret-Store, kein Schlüsselbund).
pub struct TestCa;

#[async_trait::async_trait]
impl beton_server::sessions::ProxyCaSource for TestCa {
    async fn handover(
        &self,
        _: beton_core::id::SessionId,
    ) -> Result<beton_proxy::CaHandover, String> {
        beton_proxy::BetonCa::generate("runner-tests", time::OffsetDateTime::now_utc())
            .map_err(|e| e.to_string())?
            .handover()
            .map_err(|e| e.to_string())
    }
}
