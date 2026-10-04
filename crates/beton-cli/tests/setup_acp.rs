//! `beton setup acp add` (HAR-008 AC1).

#![allow(clippy::unwrap_used)]

mod common;

use beton_harness::registry::{HarnessLayers, HarnessesConfig};
use beton_harness::{HarnessId, HostEnv};
use common::{beton, run, stderr};

#[tokio::test]
async fn har_008_ac1_setup_acp_add_writes_entry_and_agent_appears_in_catalog() {
    let home = tempfile::tempdir().unwrap();
    let out = run(beton(home.path()).args([
        "setup",
        "acp",
        "add",
        "mein-agent",
        "--command",
        "./agent",
        "--arg",
        "acp",
    ]));
    assert!(out.status.success(), "{}", stderr(&out));
    let file = home.path().join("config.yaml");
    let text = std::fs::read_to_string(&file).unwrap();
    #[derive(serde::Deserialize)]
    struct File {
        harnesses: HarnessesConfig,
    }
    let parsed: File = serde_yaml_ng::from_str(&text).unwrap();
    let entry = beton_harness_acp::config::parse_entry(
        "mein-agent",
        &parsed.harnesses.acp.agents["mein-agent"],
    )
    .unwrap();
    assert_eq!(entry.command, "./agent");
    assert_eq!(entry.args, ["acp"]);
    // Die Datei ist danach weiter gültig (`config get` liest sie).
    let get =
        run(beton(home.path()).args(["config", "get", "harnesses.acp.agents.mein-agent.command"]));
    assert!(get.status.success(), "{}", stderr(&get));

    let layers = HarnessLayers {
        user: parsed.harnesses,
        user_file: Some(file),
        ..HarnessLayers::default()
    };
    let registry = beton_runner::builtin_registry(&layers, false);
    let catalog = registry.catalog(&HostEnv::default()).await;
    let id: HarnessId = "acp:mein-agent".parse().unwrap();
    assert!(
        catalog.iter().any(|h| h.id == id),
        "acp:mein-agent fehlt im Katalog"
    );
}

#[test]
fn har_008_ac3_setup_acp_add_rejects_invalid_slug_without_writing() {
    let home = tempfile::tempdir().unwrap();
    let out =
        run(beton(home.path()).args(["setup", "acp", "add", "Mein_Agent", "--command", "./agent"]));
    assert!(!out.status.success());
    assert!(stderr(&out).contains("Slug"), "{}", stderr(&out));
    assert!(!home.path().join("config.yaml").exists());
}
