//! Snapshot-Tests der Hilfetexte (CLI-001 AC3).
//!
//! Jedes sichtbare Kommando hat eine Datei `tests/snapshots/help/<pfad>.txt`.
//! Neu schreiben: `BETON_BLESS=1 cargo nextest run -p beton-cli --test help`.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use beton_cli::cli::Cli;
use clap::CommandFactory as _;

fn collect(cmd: &clap::Command, path: &str, out: &mut BTreeMap<String, String>) {
    let mut cmd = cmd.clone().term_width(100).color(clap::ColorChoice::Never);
    let help = cmd.render_long_help().to_string();
    // Die Version ändert sich mit jedem Release; sie gehört nicht in den Snapshot.
    let help = help.replace(beton_cli::VERSION, "<version>");
    out.insert(path.to_owned(), help);
    for sub in cmd.get_subcommands() {
        if sub.is_hide_set() {
            continue;
        }
        collect(sub, &format!("{path}-{}", sub.get_name()), out);
    }
}

fn snapshot_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/help")
}

#[test]
fn cli_001_ac3_every_command_help_matches_snapshot() {
    let mut cmd = Cli::command();
    cmd.build();
    let mut helps = BTreeMap::new();
    collect(&cmd, "beton", &mut helps);
    assert!(helps.contains_key("beton-config-set"));
    assert!(helps.contains_key("beton-serve"));

    let dir = snapshot_dir();
    let bless = std::env::var_os("BETON_BLESS").is_some();
    if bless {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
    }
    let mut failures = Vec::new();
    for (name, help) in &helps {
        let file = dir.join(format!("{name}.txt"));
        if bless {
            std::fs::write(&file, help).unwrap();
            continue;
        }
        match std::fs::read_to_string(&file) {
            Ok(expected) if expected == *help => {}
            Ok(expected) => failures.push(format!(
                "{name}: Hilfetext weicht ab\n--- erwartet\n{expected}\n--- aktuell\n{help}"
            )),
            Err(_) => failures.push(format!("{name}: Snapshot fehlt ({})", file.display())),
        }
    }
    // Keine verwaisten Snapshots.
    for entry in std::fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        let stem = name.trim_end_matches(".txt");
        if !helps.contains_key(stem) {
            failures.push(format!("{name}: kein Kommando mehr dazu"));
        }
    }
    assert!(
        failures.is_empty(),
        "{}\n\nNeu schreiben mit BETON_BLESS=1",
        failures.join("\n\n")
    );
}
