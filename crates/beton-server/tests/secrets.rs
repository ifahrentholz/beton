//! REST-API für Secrets und Audit-Log (SEC-004, SEC-012, SEC-003).

#![allow(clippy::unwrap_used)]

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::header;
use beton_secrets::backend::{Backend, FileBackend, KdfParams, MemoryBackend};
use common::{Res, TestApp, app_runtime};
use serde_json::{Value, json};

const MARKER: &str = "MARKER-ghp_0123456789abcdefghijklmnopqrstuvwxyz";

async fn with_backend(backend: Arc<dyn Backend>) -> TestApp {
    app_runtime(move |mut r| {
        r.secret_backend = backend;
        r
    })
    .await
}

async fn memory_app() -> TestApp {
    with_backend(Arc::new(MemoryBackend::default())).await
}

async fn call(t: &TestApp, method: &str, path: &str, body: Option<Value>) -> Res {
    let req = t.authed(method, path);
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    t.send(req).await
}

fn create(value: &str) -> Value {
    json!({ "secret_ref": "secret://user/github.com/ifahrentholz", "kind": "token", "value": value })
}

fn assert_no_marker(res: &Res) {
    assert!(
        !String::from_utf8_lossy(&res.body).contains("MARKER"),
        "Wert in der Antwort: {}",
        String::from_utf8_lossy(&res.body)
    );
}

#[tokio::test]
async fn sec_004_create_list_rotate_delete_without_values_in_responses() {
    let t = memory_app().await;
    let res = call(&t, "POST", "/v1/secrets", Some(create(MARKER))).await;
    assert_eq!(res.status, 201, "{}", String::from_utf8_lossy(&res.body));
    assert_no_marker(&res);
    let meta = res.json();
    assert_eq!(meta["secret_ref"], "secret://user/github.com/ifahrentholz");
    assert_eq!(meta["kind"], "token");
    assert_eq!(meta["provider"], "github.com");
    assert_eq!(meta["account"], "ifahrentholz");
    assert!(meta.get("value").is_none());

    let res = call(&t, "POST", "/v1/secrets", Some(create(MARKER))).await;
    res.assert_problem("conflict");
    assert_no_marker(&res);

    let res = call(&t, "GET", "/v1/secrets", None).await;
    assert_eq!(res.status, 200);
    assert_no_marker(&res);
    let list = res.json();
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_eq!(list["store"]["backend"], "memory");
    assert_eq!(list["store"]["locked"], false);
    assert!(list["items"][0]["last_used_at"].is_null());

    let res = call(
        &t,
        "PUT",
        "/v1/secrets",
        Some(json!({ "secret_ref": "secret://user/github.com/ifahrentholz", "value": format!("{MARKER}-2") })),
    )
    .await;
    assert_eq!(res.status, 200, "{}", String::from_utf8_lossy(&res.body));
    assert_no_marker(&res);

    let res = call(
        &t,
        "DELETE",
        "/v1/secrets?secret_ref=secret://user/github.com/ifahrentholz",
        None,
    )
    .await;
    assert_eq!(res.status, 204);
    let res = call(
        &t,
        "DELETE",
        "/v1/secrets?secret_ref=secret://user/github.com/ifahrentholz",
        None,
    )
    .await;
    res.assert_problem("not_found");
    assert!(
        call(&t, "GET", "/v1/secrets", None).await.json()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn sec_004_invalid_input_is_rejected_without_echoing_the_value() {
    let t = memory_app().await;
    let res = call(
        &t,
        "POST",
        "/v1/secrets",
        Some(json!({ "secret_ref": "github", "kind": "token", "value": MARKER })),
    )
    .await;
    res.assert_problem("invalid_secret_ref");
    assert_no_marker(&res);
    let res = call(
        &t,
        "POST",
        "/v1/secrets",
        Some(json!({ "secret_ref": "secret://user/npm/x", "kind": "basic", "value": MARKER })),
    )
    .await;
    res.assert_problem("validation_failed");
    assert_no_marker(&res);
    // Falscher Typ des Werts: der Parser darf ihn nicht zitieren.
    let res = call(
        &t,
        "POST",
        "/v1/secrets",
        Some(json!({ "secret_ref": "secret://user/npm/x", "kind": "token", "value": ["MARKER"] })),
    )
    .await;
    res.assert_problem("validation_failed");
    assert_no_marker(&res);
}

#[tokio::test]
async fn sec_004_secret_requests_are_never_stored_for_idempotency() {
    let t = memory_app().await;
    let req = t
        .authed("POST", "/v1/secrets")
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", "k1")
        .body(Body::from(create(MARKER).to_string()))
        .unwrap();
    assert_eq!(t.send(req).await.status, 201);
    assert!(
        t.store
            .idempotency_lookup(beton_core::id::OrgId::LOCAL, "k1")
            .await
            .unwrap()
            .is_none(),
        "kein Hash des Bodys mit dem Wert in der Datenbank"
    );
}

#[tokio::test]
async fn sec_003_locked_file_store_fails_closed_and_unlocks() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(FileBackend::new(
        dir.path().join("store.v1"),
        KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
        },
    ));
    let t = with_backend(backend).await;
    let res = call(&t, "GET", "/v1/secrets", None).await;
    assert_eq!(res.json()["store"]["locked"], true);
    let res = call(&t, "POST", "/v1/secrets", Some(create("wert"))).await;
    res.assert_problem("secrets_locked");

    // Erstes Entsperren legt den Store an.
    let res = call(
        &t,
        "POST",
        "/v1/secrets/unlock",
        Some(json!({ "passphrase": "richtig" })),
    )
    .await;
    assert_eq!(res.status, 204, "{}", String::from_utf8_lossy(&res.body));
    assert_eq!(
        call(&t, "POST", "/v1/secrets", Some(create("wert")))
            .await
            .status,
        201
    );
    // Falsche Passphrase: gesperrt, ohne Hinweis.
    let res = call(
        &t,
        "POST",
        "/v1/secrets/unlock",
        Some(json!({ "passphrase": "falsch" })),
    )
    .await;
    res.assert_problem("secrets_locked");
    // Passphrase-Wechsel.
    let res = call(
        &t,
        "POST",
        "/v1/secrets/unlock",
        Some(json!({ "passphrase": "richtig", "new_passphrase": "neu" })),
    )
    .await;
    assert_eq!(res.status, 204);
    let res = call(
        &t,
        "POST",
        "/v1/secrets/unlock",
        Some(json!({ "passphrase": "richtig" })),
    )
    .await;
    res.assert_problem("secrets_locked");
}

#[tokio::test]
async fn sec_012_audit_list_and_verify_over_the_api() {
    let t = memory_app().await;
    call(&t, "POST", "/v1/secrets", Some(create(MARKER))).await;
    call(
        &t,
        "DELETE",
        "/v1/secrets?secret_ref=secret://user/github.com/ifahrentholz",
        None,
    )
    .await;
    let res = call(&t, "GET", "/v1/audit?limit=1", None).await;
    assert_eq!(res.status, 200, "{}", String::from_utf8_lossy(&res.body));
    assert_no_marker(&res);
    let page = res.json();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["action"], "secret.deleted");
    assert_eq!(page["items"][0]["seq"], 2);
    let cursor = page["next_cursor"].as_str().unwrap().to_owned();
    let res = call(&t, "GET", &format!("/v1/audit?cursor={cursor}"), None).await;
    let page = res.json();
    assert_eq!(page["items"][0]["action"], "secret.created");
    assert!(page["next_cursor"].is_null());
    let res = call(&t, "GET", "/v1/audit?action=secret.created", None).await;
    assert_eq!(res.json()["items"].as_array().unwrap().len(), 1);

    let res = call(&t, "GET", "/v1/audit/verify", None).await;
    assert_eq!(res.status, 200);
    let v = res.json();
    assert_eq!(v["ok"], true);
    assert_eq!(v["entries"], 2);
    assert!(v["first_invalid"].is_null());

    let res = call(&t, "GET", "/v1/audit?cursor=kaputt", None).await;
    res.assert_problem("invalid_cursor");
}

/// SEC-004 AC3: Keine Antwort der API enthält ein Feld `value` (OpenAPI-Snapshot).
#[test]
fn sec_004_ac3_no_response_schema_has_a_value_field() {
    let doc = serde_json::to_value(beton_server::openapi()).unwrap();
    let schemas = doc["components"]["schemas"].clone();
    let mut checked = 0;
    for (path, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            let Some(responses) = op["responses"].as_object() else {
                continue;
            };
            for (status, response) in responses {
                for (_, media) in response["content"].as_object().into_iter().flatten() {
                    let mut seen = Vec::new();
                    let mut fields = Vec::new();
                    collect_fields(&media["schema"], &schemas, &mut seen, &mut fields);
                    checked += 1;
                    for f in &fields {
                        assert!(
                            !matches!(f.as_str(), "value" | "passphrase" | "new_passphrase"),
                            "{method} {path} {status}: Antwort hat ein Feld `{f}`"
                        );
                    }
                }
            }
        }
    }
    assert!(checked > 50, "nur {checked} Antworten geprüft");
    // Die Secret-Routen sind dokumentiert, und ihr Request hat sehr wohl einen Wert.
    assert!(doc["paths"]["/v1/secrets"]["post"].is_object());
    assert!(doc["paths"]["/v1/audit"]["get"].is_object());
    assert!(schemas["CreateSecretRequest"]["properties"]["value"].is_object());
}

/// Feldnamen eines Schemas samt aller referenzierten Schemas.
fn collect_fields(schema: &Value, schemas: &Value, seen: &mut Vec<String>, out: &mut Vec<String>) {
    match schema {
        Value::Object(map) => {
            if let Some(r) = map.get("$ref").and_then(Value::as_str) {
                let name = r.trim_start_matches("#/components/schemas/").to_owned();
                if !seen.contains(&name) {
                    seen.push(name.clone());
                    collect_fields(&schemas[&name], schemas, seen, out);
                }
            }
            if let Some(props) = map.get("properties").and_then(Value::as_object) {
                out.extend(props.keys().cloned());
            }
            for v in map.values() {
                collect_fields(v, schemas, seen, out);
            }
        }
        Value::Array(items) => {
            for v in items {
                collect_fields(v, schemas, seen, out);
            }
        }
        _ => {}
    }
}
