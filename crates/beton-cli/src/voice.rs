//! `beton voice models list|pull|import|remove` (VOI-002). Braucht keinen Daemon; Modelle
//! liegen in `<BETON_HOME>/models/whisper/`. Netzwerk nur bei `pull` nach Zustimmung, und nur
//! zu den gepinnten Adressen (VOI-008); `import` übernimmt eine lokale Datei ohne Netzwerk
//! (ADR-0033).

use std::io::{IsTerminal as _, Write as _};

use anyhow::anyhow;
use beton_voice::catalog::Catalog;
use beton_voice::download::{self, Consent, DownloadError};
use beton_voice::store::{ModelStore, StoreError};
use serde_json::json;

use crate::cli::{VoiceCommand, VoiceModelsCommand, VoiceModelsPullArgs};
use crate::commands::{Ctx, print_json};
use crate::exit::{CliError, CliResult, Exit};

pub async fn run(ctx: &Ctx, cmd: VoiceCommand) -> CliResult {
    if ctx.global.server.is_some() {
        return Err(CliError::usage(
            "Im zentralen Betrieb liegen die Whisper-Modelle auf dem Server; `beton voice \
             models` dort ausführen",
        ));
    }
    let store = ModelStore::in_home(&ctx.home);
    let catalog = Catalog::pinned();
    let VoiceCommand::Models(cmd) = cmd;
    match cmd {
        VoiceModelsCommand::List => list(ctx, &store, &catalog),
        VoiceModelsCommand::Pull(args) => pull(ctx, &store, &catalog, args).await,
        VoiceModelsCommand::Import(args) => match store.import(&catalog, &args.file) {
            Ok(entry) => {
                if ctx.global.json {
                    print_json(&json!({
                        "name": entry.name,
                        "path": store.path(&entry).display().to_string(),
                        "sha256": entry.sha256,
                    }))
                } else {
                    println!(
                        "{} importiert nach {} (SHA-256 geprüft)",
                        entry.name,
                        store.path(&entry).display()
                    );
                    Ok(())
                }
            }
            Err(e) => Err(store_error(e)),
        },
        VoiceModelsCommand::Remove(args) => {
            store.remove(&catalog, &args.name).map_err(store_error)?;
            if !ctx.global.quiet {
                eprintln!("{} entfernt", args.name);
            }
            Ok(())
        }
    }
}

fn store_error(e: StoreError) -> CliError {
    match e {
        StoreError::UnknownModel(_) | StoreError::NotInstalled(_) => CliError::usage(e),
        _ => CliError::new(Exit::General, e),
    }
}

fn list(ctx: &Ctx, store: &ModelStore, catalog: &Catalog) -> CliResult {
    let models = store.list(catalog, beton_voice::default_model());
    if ctx.global.json {
        return print_json(&serde_json::to_value(&models).map_err(anyhow::Error::from)?);
    }
    println!(
        "{:<28} {:>8}  {:<11} SHA-256",
        "MODELL", "GRÖSSE", "INSTALLIERT"
    );
    for m in &models {
        let name = if m.recommended {
            format!("{} (empfohlen)", m.name)
        } else {
            m.name.clone()
        };
        println!(
            "{name:<28} {:>8}  {:<11} {}",
            beton_voice::format_size(m.size_bytes),
            if m.installed { "ja" } else { "nein" },
            m.sha256
        );
    }
    if !ctx.global.quiet {
        eprintln!("Verzeichnis: {}", store.dir().display());
    }
    Ok(())
}

/// Fragt am Terminal nach der Zustimmung zum Download (VOI-002 AC3).
fn ask(name: &str, size: u64, store: &ModelStore) -> bool {
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        return false;
    }
    eprint!(
        "Modell {name} (≈ {}) von huggingface.co (feste Adresse im Programm) nach {} laden? [j/N] ",
        beton_voice::format_size(size),
        store.dir().display()
    );
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(
        line.trim().to_lowercase().as_str(),
        "j" | "ja" | "y" | "yes"
    )
}

async fn pull(
    ctx: &Ctx,
    store: &ModelStore,
    catalog: &Catalog,
    args: VoiceModelsPullArgs,
) -> CliResult {
    let Some(entry) = catalog.get(&args.name) else {
        let names: Vec<&str> = catalog.entries().iter().map(|e| e.name.as_str()).collect();
        return Err(CliError::usage(format!(
            "unbekanntes Modell „{}“; verfügbar: {}",
            args.name,
            names.join(", ")
        )));
    };
    let path = store.path(entry);
    if !store.is_installed(entry) {
        // Ohne Zustimmung kein Download (VOI-002 AC3, ADR-0033).
        if !args.yes && !ask(&entry.name, entry.size_bytes, store) {
            return Err(CliError::usage(format!(
                "Download von {} (≈ {}) braucht deine Zustimmung: `beton voice models pull {} \
                 --yes`; ohne Netzwerk: `beton voice models import <datei>`",
                entry.name,
                beton_voice::format_size(entry.size_bytes),
                entry.name
            )));
        }
        let quiet = ctx.global.quiet || ctx.global.json;
        let mut last = u64::MAX;
        let result = download::pull(catalog, store, &entry.name, Consent::given_by_user(), |p| {
            if quiet || p.total == 0 {
                return;
            }
            let pct = p.downloaded * 100 / p.total;
            if pct != last && (pct % 5 == 0 || p.downloaded == p.total) {
                last = pct;
                let resumed = if p.resumed_from > 0 {
                    format!(
                        ", fortgesetzt ab {}",
                        beton_voice::format_size(p.resumed_from)
                    )
                } else {
                    String::new()
                };
                eprintln!(
                    "{}: {pct:>3} % ({} von {}{resumed})",
                    entry.name,
                    beton_voice::format_size(p.downloaded),
                    beton_voice::format_size(p.total)
                );
            }
        })
        .await;
        match result {
            Ok(_) => {}
            Err(e @ DownloadError::UnknownModel(_)) => return Err(CliError::usage(e)),
            Err(e) => return Err(CliError::new(Exit::General, anyhow!("{}: {e}", entry.name))),
        }
    } else if !ctx.global.quiet && !ctx.global.json {
        eprintln!("{} ist bereits installiert", entry.name);
    }
    if ctx.global.json {
        print_json(&json!({
            "name": entry.name,
            "path": path.display().to_string(),
            "sha256": entry.sha256,
            "size_bytes": entry.size_bytes,
        }))
    } else {
        println!(
            "{} bereit: {} (SHA-256 geprüft)",
            entry.name,
            path.display()
        );
        Ok(())
    }
}
