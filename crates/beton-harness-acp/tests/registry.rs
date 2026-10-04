//! ACP-Agent-Registrierung und Presets (HAR-008 AC2, AC3).

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use beton_harness::process::RealLauncher;
use beton_harness::registry::{HarnessLayers, HarnessesConfig, Registry, RegistryOptions};
use beton_harness::{AdapterContext, AllowAll, HarnessId, HostEnv, SessionSpec};

#[tokio::test]
async fn har_008_ac2_preset_without_binary_is_listed_but_not_startable() {
    let mut registry = Registry::new(RegistryOptions::default());
    beton_harness_acp::register(&mut registry, &HarnessLayers::default());
    let empty = tempfile::tempdir().unwrap();
    let env = HostEnv {
        path: Some(empty.path().as_os_str().to_owned()),
        ..HostEnv::default()
    };
    let catalog = registry.catalog(&env).await;
    for preset in ["acp:gemini", "acp:goose", "acp:qwen"] {
        let entry = catalog
            .iter()
            .find(|h| h.id.as_str() == preset)
            .unwrap_or_else(|| panic!("{preset} fehlt im Katalog"));
        assert!(!entry.probe.installed, "{preset}");
        assert!(!entry.capabilities.is_empty(), "vollständige Capabilities");
    }
    let id: HarnessId = "acp:gemini".parse().unwrap();
    let ctx = AdapterContext {
        gate: Arc::new(AllowAll),
        launcher: Arc::new(RealLauncher),
        env,
    };
    let err = registry
        .start(
            &id,
            SessionSpec {
                workdir: empty.path().to_path_buf(),
                ..SessionSpec::default()
            },
            ctx,
        )
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), "harness_not_found", "{err}");
}

#[test]
fn har_008_ac3_invalid_entries_are_reported_with_file_and_line_and_skipped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".beton")).unwrap();
    let file = dir.path().join(".beton/config.yaml");
    std::fs::write(
        &file,
        "harnesses:\n  claude:\n    command: claude\n  acp:\n    agents:\n      gut:\n        command: ./agent\n        args: [acp]\n      ohne-command:\n        args: [acp]\n      Mein_Agent:\n        command: ./x\n",
    )
    .unwrap();
    let project = HarnessesConfig::load_project(dir.path()).unwrap();
    assert_eq!(project.entries["claude"].command.as_deref(), Some("claude"));
    let layers = HarnessLayers {
        project,
        project_file: Some(file.clone()),
        ..HarnessLayers::default()
    };
    let mut registry = Registry::new(RegistryOptions::default());
    let problems = beton_harness_acp::register(&mut registry, &layers);
    let text: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert_eq!(problems.len(), 2, "{text:#?}");
    let missing = problems
        .iter()
        .find(|p| p.key.ends_with("ohne-command"))
        .unwrap();
    assert_eq!(missing.file.as_deref(), Some(file.as_path()));
    assert_eq!(missing.line, Some(9));
    assert!(missing.message.contains("command"), "{}", missing.message);
    let slug = problems
        .iter()
        .find(|p| p.key.ends_with("Mein_Agent"))
        .unwrap();
    assert_eq!(slug.line, Some(11));
    assert!(text[0].contains("config.yaml:"), "{text:?}");
    // Der gültige Eintrag und die Presets sind registriert; nichts anderes ist betroffen.
    let ids: Vec<&str> = registry.ids().map(HarnessId::as_str).collect();
    assert_eq!(ids, ["acp:gemini", "acp:goose", "acp:gut", "acp:qwen"]);
}
