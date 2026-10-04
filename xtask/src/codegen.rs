//! Generierte Artefakte aus den Rust-Typen (PROTO-013, QA-006).
//!
//! Rust-Typen sind die einzige Quelle. `cargo xtask codegen` schreibt:
//! - `schemas/v1/events.schema.json` (JSON-Schema via `schemars`)
//! - `schemas/v1/ws.schema.json` (WebSocket-Nachrichten, PROTO-004 ff.)
//! - `schemas/v1/harness-catalog.schema.json` (Harness-Katalog mit Capabilities, HAR-002 AC4)
//! - `schemas/v1/config.schema.json` (`config.yaml`, CLI-008)
//! - `schemas/v1/agent.schema.json` (`agent.yaml`, Agent-Format v1, AGT-002)
//! - `schemas/v1/doctor.schema.json` (`beton doctor --json`, OBS-005 AC2)
//! - `schemas/v1/setup-check.schema.json` (`beton setup --check --json`, HAR-016 AC2)
//! - `packages/sdk-ts/src/gen/*.ts` (TypeScript via `ts-rs`) plus `index.ts`
//! - `docs/generated/er-diagram.md` (ER-Diagramm aus den SQLite-Migrationen, DATA-001 AC1)
//! - `openapi/v1.json` (OpenAPI 3.1 via `utoipa`, API-001 AC1, PROTO-013 AC3)
//! - `docs/generated/problem-codes.md` (Fehlercodes, PROTO-011)
//!
//! `cargo xtask codegen --check` erzeugt alles in ein temporäres Verzeichnis und vergleicht
//! mit dem eingecheckten Stand; jede Abweichung lässt die CI fehlschlagen.
//! WebSocket-, Tunnel- und OpenAPI-Schemas kommen mit ihren Arbeitspaketen dazu.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use beton_core::event::Event;
use beton_harness::registry::HarnessInfo;
use beton_proto::ws::{ClientMsg, ServerMsg};

/// Beide Richtungen des WebSocket-Protokolls in einem Schema (PROTO-013).
#[derive(schemars::JsonSchema)]
#[allow(dead_code)]
struct WsMessages {
    client: ClientMsg,
    server: ServerMsg,
}
use ts_rs::TS;

/// Verzeichnisse, die vollständig generiert werden (veraltete Dateien werden entfernt).
const GENERATED_DIRS: [&str; 4] = [
    "schemas/v1",
    "packages/sdk-ts/src/gen",
    "docs/generated",
    "openapi",
];

const TS_HEADER: &str =
    "// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.\n";

/// Alle generierten Dateien: Pfad relativ zur Repo-Wurzel → Inhalt.
pub fn generate() -> Result<BTreeMap<PathBuf, String>> {
    let mut files = BTreeMap::new();

    let schema = schemars::schema_for!(Event);
    let mut json = serde_json::to_string_pretty(&schema)?;
    json.push('\n');
    files.insert(PathBuf::from("schemas/v1/events.schema.json"), json);
    let ws = schemars::schema_for!(WsMessages);
    let mut json = serde_json::to_string_pretty(&ws)?;
    json.push('\n');
    files.insert(PathBuf::from("schemas/v1/ws.schema.json"), json);
    let catalog = schemars::schema_for!(Vec<HarnessInfo>);
    let mut json = serde_json::to_string_pretty(&catalog)?;
    json.push('\n');
    files.insert(
        PathBuf::from("schemas/v1/harness-catalog.schema.json"),
        json,
    );
    let config = schemars::schema_for!(beton_cli::config::Settings);
    let mut json = serde_json::to_string_pretty(&config)?;
    json.push('\n');
    files.insert(PathBuf::from("schemas/v1/config.schema.json"), json);
    files.insert(
        PathBuf::from("schemas/v1/agent.schema.json"),
        beton_agents::schema_json(),
    );
    for (path, schema) in [
        (
            "schemas/v1/doctor.schema.json",
            schemars::schema_for!(beton_cli::doctor::DoctorReport),
        ),
        (
            "schemas/v1/setup-check.schema.json",
            schemars::schema_for!(beton_cli::setup::SetupReport),
        ),
    ] {
        let mut json = serde_json::to_string_pretty(&schema)?;
        json.push('\n');
        files.insert(PathBuf::from(path), json);
    }

    let tmp = tempfile::tempdir()?;
    let cfg = ts_rs::Config::new()
        .with_out_dir(tmp.path())
        .with_large_int("number");
    Event::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
    HarnessInfo::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
    ClientMsg::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
    ServerMsg::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
    // REST-Modelle (API-004): Anfragen und Antworten der Session-API.
    {
        use beton_server::api::{Info, LoginCode, SessionPage, SessionSummary};
        use beton_server::api_sessions::{
            ApprovalPage, CreateSessionRequest, EventPage, InputAccepted, InputRequest, PinRequest,
            QueueEditRequest, QueueMoveRequest, QueueView, ReadStateRequest,
            ResolveApprovalRequest, SessionSettings,
        };
        QueueView::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        QueueEditRequest::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        QueueMoveRequest::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        ReadStateRequest::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        PinRequest::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        Info::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        LoginCode::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        SessionPage::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        SessionSummary::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        ApprovalPage::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        CreateSessionRequest::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        EventPage::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        InputAccepted::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        InputRequest::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        ResolveApprovalRequest::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
        SessionSettings::export_all(&cfg).context("TypeScript-Export fehlgeschlagen")?;
    }
    let mut names = Vec::new();
    for path in walk(tmp.path())? {
        let rel = path.strip_prefix(tmp.path())?.to_path_buf();
        let Some(rel_str) = rel.to_str().map(|s| s.replace('\\', "/")) else {
            continue;
        };
        let Some(stem) = rel_str.strip_suffix(".ts").map(str::to_owned) else {
            continue;
        };
        let body = fs::read_to_string(&path)?;
        // ts-rs schreibt einen eigenen Kopf; wir setzen einen einheitlichen.
        let body: String = body
            .lines()
            .filter(|l| !l.starts_with("// This file was generated by"))
            .collect::<Vec<_>>()
            .join("\n");
        files.insert(
            PathBuf::from("packages/sdk-ts/src/gen").join(&rel),
            format!("{TS_HEADER}{}\n", body.trim_start()),
        );
        names.push(stem);
    }
    names.sort();
    let index: String = names
        .iter()
        .map(|n| format!("export type * from './{n}'\n"))
        .collect();
    files.insert(
        PathBuf::from("packages/sdk-ts/src/gen/index.ts"),
        format!("{TS_HEADER}{index}"),
    );

    files.insert(PathBuf::from("docs/generated/er-diagram.md"), er_diagram()?);
    let mut openapi = beton_server::openapi().to_pretty_json()?;
    openapi.push('\n');
    files.insert(PathBuf::from("openapi/v1.json"), openapi);
    files.insert(
        PathBuf::from("docs/generated/problem-codes.md"),
        problem_codes(),
    );
    Ok(files)
}

/// Referenz der Fehlercodes aus `ProblemCode` (PROTO-011).
fn problem_codes() -> String {
    use std::fmt::Write as _;
    let mut out = String::from(
        "<!-- Generiert von `cargo xtask codegen` aus beton_server::ProblemCode. \
         Nicht von Hand ändern. -->\n\n# Fehlercodes (RFC 9457)\n\n\
         Jede Fehlerantwort ist ein Problem-Objekt mit `type: urn:beton:problem:<code>`. \
         Spezifikation: PROTO-011 in \
         [06-data-sync-protocol.md](../spec/06-data-sync-protocol.md#proto-011--fehlerformat-rfc-9457).\n\n\
         | Code | Status | Titel |\n| --- | --- | --- |\n",
    );
    for code in beton_server::ProblemCode::ALL {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} |",
            code.as_str(),
            code.status().as_u16(),
            code.title()
        );
    }
    out
}

/// ER-Diagramm der lokalen Datenbank, erzeugt aus den eingebetteten SQLite-Migrationen.
fn er_diagram() -> Result<String> {
    let schema = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(beton_store::schema::sqlite_schema())
        .context("Schema aus den Migrationen lesen")?;
    Ok(format!(
        "<!-- Generiert von `cargo xtask codegen` aus crates/beton-store/migrations/sqlite. \
         Nicht von Hand ändern. -->\n\n\
         # Datenmodell (lokal, SQLite)\n\n\
         Schema-Version {}. Spezifikation: DATA-001 in \
         [06-data-sync-protocol.md](../spec/06-data-sync-protocol.md#data-001--datenmodell--entitäten).\n\n\
         ```mermaid\n{}```\n",
        beton_store::SCHEMA_VERSION,
        schema.to_mermaid()
    ))
}

/// Alle Dateien unter `dir`, rekursiv.
fn walk(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            out.extend(walk(&path)?);
        } else {
            out.push(path);
        }
    }
    Ok(out)
}

/// Schreibt die generierten Dateien und entfernt veraltete in den generierten Verzeichnissen.
pub fn write(root: &Path, files: &BTreeMap<PathBuf, String>) -> Result<()> {
    for dir in GENERATED_DIRS {
        let dir = root.join(dir);
        if dir.exists() {
            for path in walk(&dir)? {
                let rel = path.strip_prefix(root)?.to_path_buf();
                if !files.contains_key(&rel) {
                    fs::remove_file(&path)?;
                }
            }
        }
    }
    for (rel, content) in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }
    Ok(())
}

/// Abweichungen zwischen generiertem und eingechecktem Stand.
pub fn drift(root: &Path, files: &BTreeMap<PathBuf, String>) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    for (rel, content) in files {
        match fs::read_to_string(root.join(rel)) {
            Ok(existing) if &existing == content => {}
            Ok(_) => problems.push(format!("veraltet: {}", rel.display())),
            Err(_) => problems.push(format!("fehlt: {}", rel.display())),
        }
    }
    for dir in GENERATED_DIRS {
        let dir = root.join(dir);
        if !dir.exists() {
            continue;
        }
        for path in walk(&dir)? {
            let rel = path.strip_prefix(root)?.to_path_buf();
            if !files.contains_key(&rel) {
                problems.push(format!("überzählig: {}", rel.display()));
            }
        }
    }
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proto_013_ac1_committed_artifacts_are_current() {
        let files = generate().unwrap();
        let problems = drift(&crate::repo_root(), &files).unwrap();
        assert!(
            problems.is_empty(),
            "Generierte Dateien sind veraltet – `cargo xtask codegen` ausführen:\n{}",
            problems.join("\n")
        );
    }

    #[test]
    fn proto_013_ac1_drift_is_detected() {
        let files = generate().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), &files).unwrap();
        assert!(drift(tmp.path(), &files).unwrap().is_empty());

        // Feldänderung ohne Neugenerierung: eingecheckte Datei weicht ab.
        let schema = tmp.path().join("schemas/v1/events.schema.json");
        let changed = fs::read_to_string(&schema)
            .unwrap()
            .replace("\"seq\"", "\"sequence\"");
        fs::write(&schema, changed).unwrap();
        // Zusätzliche, nicht mehr erzeugte Datei.
        fs::write(
            tmp.path().join("packages/sdk-ts/src/gen/Alt.ts"),
            "export type Alt = string\n",
        )
        .unwrap();

        let problems = drift(tmp.path(), &files).unwrap();
        assert!(
            problems
                .iter()
                .any(|p| p.contains("veraltet: schemas/v1/events.schema.json")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("überzählig")),
            "{problems:?}"
        );
    }

    #[test]
    fn qa_006_ac1_typescript_contains_event_union() {
        let files = generate().unwrap();
        let ts = |name: &str| {
            files
                .get(&PathBuf::from(format!("packages/sdk-ts/src/gen/{name}")))
                .unwrap_or_else(|| panic!("{name} fehlt"))
        };
        let payload = ts("EventPayload.ts");
        let event_type = ts("EventType.ts");
        for (name, _) in beton_core::event::CATALOG {
            assert!(
                payload.contains(&format!("\"type\": \"{name}\"")),
                "{name} fehlt in EventPayload.ts"
            );
            assert!(
                event_type.contains(&format!("\"{name}\"")),
                "{name} fehlt in EventType.ts"
            );
        }
        assert!(ts("Event.ts").contains("EventPayload | OffloadedPayload"));
        assert!(files.contains_key(&PathBuf::from("packages/sdk-ts/src/gen/index.ts")));
    }

    #[test]
    fn har_002_ac4_capability_schema_is_snapshotted() {
        let files = generate().unwrap();
        let schema = &files[&PathBuf::from("schemas/v1/harness-catalog.schema.json")];
        for field in [
            "approval",
            "tool_call_gate",
            "model_switch",
            "fork_history",
            "probe",
        ] {
            assert!(schema.contains(&format!("\"{field}\"")), "{field} fehlt");
        }
        assert!(files.contains_key(&PathBuf::from("packages/sdk-ts/src/gen/Capabilities.ts")));
        // Drift erkennt `proto_013_ac1_committed_artifacts_are_current`.
    }

    #[test]
    fn api_001_ac1_openapi_is_snapshotted() {
        let files = generate().unwrap();
        let doc = &files[&PathBuf::from("openapi/v1.json")];
        assert!(doc.contains("\"openapi\": \"3.1"));
        assert!(doc.contains("/v1/sessions"));
        // Drift erkennt `proto_013_ac1_committed_artifacts_are_current`.
        let codes = &files[&PathBuf::from("docs/generated/problem-codes.md")];
        assert!(codes.contains("| `idempotency_key_reused` | 422 |"));
    }

    #[test]
    fn data_001_ac1_er_diagram_is_generated_from_migrations() {
        let files = generate().unwrap();
        let er = &files[&PathBuf::from("docs/generated/er-diagram.md")];
        assert!(er.contains("```mermaid\nerDiagram\n"));
        assert!(er.contains("    events {"));
        assert!(er.contains("sessions ||--o{ events : \"session_id\""));
    }
}
