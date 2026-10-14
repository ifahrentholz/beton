//! SBX-012 (Presets) und die neuen Felder der Sandbox-Konfiguration aus WP-45 auf Ebene der
//! Auflösung (plattformunabhängig). Die Durchsetzung im Kernel prüfen `tests/seatbelt.rs` und
//! `tests/linux.rs`, `npm install`/`npm publish` mit `preset: dev` `tests/toolchain.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use beton_sandbox::{
    NetMode, Preset, ProviderId, ResolveCtx, SandboxSpec, Stage, WorkspaceAccess, resolve,
};

struct Env {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    ws: PathBuf,
    scratch: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let home = root.join("home");
    let ws = root.join("ws");
    let scratch = root.join("scratch");
    for d in [&home, &ws, &scratch] {
        std::fs::create_dir_all(d).unwrap();
    }
    for d in [
        ".cargo/registry",
        ".npm/_cacache",
        ".cache/pip",
        "go/pkg/mod",
    ] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    Env {
        _tmp: tmp,
        home,
        ws,
        scratch,
    }
}

fn spec(yaml: &str) -> SandboxSpec {
    SandboxSpec::from_yaml(yaml).unwrap()
}

#[test]
fn sbx_012_presets_are_parsed_and_unknown_names_rejected() {
    assert_eq!(spec("preset: dev\n").preset, Some(Preset::Dev));
    assert_eq!(spec("preset: readonly\n").preset, Some(Preset::Readonly));
    assert_eq!(spec("preset: default\n").preset, Some(Preset::Default));
    let e = SandboxSpec::from_yaml("preset: yolo\n").unwrap_err();
    assert_eq!(e.code(), "invalid_spec", "{e}");
}

#[test]
fn sbx_012_ac2_readonly_preset_makes_the_workspace_read_only_without_network() {
    let e = env();
    let merged = SandboxSpec::merged(&[&spec("preset: readonly\n")]);
    assert_eq!(merged.workspace, Some(WorkspaceAccess::Ro));
    assert_eq!(merged.allow_network, Some(false));
    let mut ctx = ResolveCtx::new(Stage::Tools, &e.ws, &e.home, &e.scratch);
    ctx.proxy = Some(beton_sandbox::ProxyEndpoint::port(4711));
    let p = resolve(ProviderId::Seatbelt, &merged, &ctx).unwrap();
    assert!(p.read_roots.contains(&e.ws), "{:?}", p.read_roots);
    assert!(!p.write_roots.contains(&e.ws), "{:?}", p.write_roots);
    assert_eq!(p.net, NetMode::None);
}

#[test]
fn sbx_012_dev_preset_allows_registries_read_only_and_caches() {
    let merged = SandboxSpec::merged(&[&spec("preset: dev\n")]);
    let rules = merged.egress_rules.clone().unwrap();
    let all = rules.join(" ");
    for host in [
        "registry.npmjs.org",
        "registry.yarnpkg.com",
        "pypi.org",
        "files.pythonhosted.org",
        "index.crates.io",
        "static.crates.io",
        "proxy.golang.org",
        "sum.golang.org",
        "codeload.github.com",
        "objects.githubusercontent.com",
    ] {
        assert!(all.contains(host), "{host} fehlt: {rules:?}");
    }
    // Nur lesend: jede Regel erlaubt höchstens GET und HEAD.
    for r in &rules {
        let methods = r.split_whitespace().next().unwrap();
        assert!(methods.split(',').all(|m| m == "GET" || m == "HEAD"), "{r}");
    }
    let write = merged.write_paths.clone().unwrap();
    for cache in [
        "~/.cargo/registry",
        "~/.npm",
        "~/.cache/pip",
        "~/go/pkg/mod",
    ] {
        assert!(write.iter().any(|w| w == cache), "{cache}: {write:?}");
    }
    assert_eq!(merged.workspace.unwrap_or_default(), WorkspaceAccess::Rw);
    let e = env();
    let ctx = ResolveCtx::new(Stage::Tools, &e.ws, &e.home, &e.scratch);
    let p = resolve(ProviderId::Seatbelt, &merged, &ctx).unwrap();
    assert!(p.write_roots.contains(&e.home.join(".cargo/registry")));
}

#[test]
fn sbx_012_default_preset_has_no_stage_two_network_rules() {
    let merged = SandboxSpec::merged(&[&spec("preset: default\n")]);
    assert!(merged.egress_rules.unwrap_or_default().is_empty());
    assert_eq!(merged.workspace.unwrap_or_default(), WorkspaceAccess::Rw);
    assert_ne!(merged.allow_network, Some(false));
}

#[test]
fn sbx_012_own_fields_override_or_extend_the_preset() {
    // Grants und Regeln der eigenen Ebene ersetzen die des Presets, Masken werden vereinigt.
    let user = spec(
        "preset: dev\negress_rules: [\"GET example.test/**\"]\nmasks: [secrets/]\nworkspace: ro\n",
    );
    let merged = SandboxSpec::merged(&[&user]);
    assert_eq!(merged.egress_rules.unwrap(), vec!["GET example.test/**"]);
    assert_eq!(merged.workspace, Some(WorkspaceAccess::Ro));
    // Nicht gesetzte Felder kommen aus dem Preset.
    assert!(merged.write_paths.unwrap().iter().any(|w| w == "~/.npm"));
    assert!(merged.masks.unwrap().contains(&"secrets/".to_owned()));
    // Das Preset der spezifischsten Ebene gilt.
    let agent = spec("preset: readonly\n");
    let merged = SandboxSpec::merged(&[&spec("preset: dev\n"), &agent]);
    assert_eq!(merged.preset, Some(Preset::Readonly));
    assert!(merged.egress_rules.unwrap_or_default().is_empty());
}

#[test]
fn sbx_012_ac3_explain_names_the_origin_of_each_value() {
    let user = spec("preset: dev\nmasks: [secrets/]\nworkspace: ro\n");
    let (merged, origins) = SandboxSpec::merged_with_origins(&[("user", &user)]);
    assert_eq!(
        origins.get("egress_rules").map(String::as_str),
        Some("preset:dev")
    );
    assert_eq!(
        origins.get("write_paths").map(String::as_str),
        Some("preset:dev")
    );
    assert_eq!(origins.get("workspace").map(String::as_str), Some("user"));
    assert_eq!(origins.get("preset").map(String::as_str), Some("user"));
    assert_eq!(
        origins.get("masks[secrets/]").map(String::as_str),
        Some("user")
    );
    let e = env();
    let ctx = ResolveCtx::new(Stage::Tools, &e.ws, &e.home, &e.scratch);
    let p = resolve(ProviderId::Seatbelt, &merged, &ctx).unwrap();
    let v = beton_sandbox::explain::with_origins(&p, &origins);
    assert_eq!(v["origins"]["egress_rules"], "preset:dev");
    assert_eq!(v["origins"]["workspace"], "user");
    // Werte des Environments erscheinen weiterhin nie.
    assert!(v["env"].is_array());
}

#[test]
fn sbx_002_require_tool_isolation_and_harness_overrides_merge_per_layer() {
    let user = spec(
        "require_tool_isolation: false\nharness:\n  egress_extra: [\"* mcp.example.test/**\"]\n  egress_passthrough: [\"api.vendor.test\"]\n",
    );
    let agent = spec("require_tool_isolation: true\n");
    let merged = SandboxSpec::merged(&[&user, &agent]);
    assert_eq!(merged.require_tool_isolation, Some(true));
    let h = merged.harness.unwrap();
    assert_eq!(h.egress_extra.unwrap(), vec!["* mcp.example.test/**"]);
    assert_eq!(h.egress_passthrough.unwrap(), vec!["api.vendor.test"]);
}

#[test]
fn prx_005_stage_two_passthrough_and_dev_ports_are_validated() {
    assert!(SandboxSpec::from_yaml("egress_passthrough: [\"api.vendor.test:443\"]\n").is_ok());
    for bad in [
        "egress_passthrough: [\"\"]\n",
        "egress_passthrough: [\"https://api.vendor.test\"]\n",
        "egress_passthrough: [\"*.vendor.test\"]\n",
        "harness: {egress_passthrough: [\"a b\"]}\n",
        "harness: {egress_extra: [\"\"]}\n",
        "dev_ports: [0]\n",
    ] {
        let e = SandboxSpec::from_yaml(bad).unwrap_err();
        assert_eq!(e.code(), "invalid_spec", "{bad}: {e}");
    }
    assert_eq!(
        SandboxSpec::from_yaml("dev_ports: [3000, 5173]\n")
            .unwrap()
            .dev_ports,
        Some(vec![3000, 5173])
    );
}
