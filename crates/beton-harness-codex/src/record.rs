//! Aufnahme von Golden-Transcripts mit der echten `codex`-CLI (HAR-025).
//!
//! Aufruf über `beton dev record-golden --harness codex [--scenario <name>]…`. Braucht eine
//! installierte, angemeldete CLI und verbraucht etwas Kontingent; nie in CI. Vor dem
//! Schreiben entfernt der Recorder Konto-, Pfad-, Rechner- und Benutzerdaten; danach greifen
//! Scrubbing und Secret-Scan (`golden::write_recording`). Die Erwartungen entstehen
//! anschließend per Replay.
//!
//! Bis zur ersten Aufnahme mit der echten CLI stammen die Fälle unter `tests/golden/` aus
//! der Fake-CLI (`recorded_with` in `meta.yaml`), gebaut nach dem exportierten
//! Protokoll-Schema.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use beton_harness::golden::{self, Meta, RecordingLauncher, Script, ScriptDecision, ScriptInput};
use beton_harness::process::RealLauncher;
use beton_harness::{HarnessAdapter, HostEnv};
use serde_json::Value;

use crate::CodexAdapter;

/// Ein Aufnahme-Szenario.
pub struct Scenario {
    pub name: &'static str,
    pub description: &'static str,
    inputs: Vec<ScriptInput>,
    gate: Vec<ScriptDecision>,
    files: &'static [(&'static str, &'static str)],
    model: Option<&'static str>,
}

fn send(text: &str) -> ScriptInput {
    ScriptInput {
        send: Some(text.into()),
        interrupt_after_events: None,
    }
}

/// Kleines Modell und niedriger Effort für Aufnahmen: Sie sollen das Protokoll abdecken,
/// nicht das Kontingent.
pub const RECORD_MODEL: &str = "gpt-5.6-luna";
pub const RECORD_EFFORT: &str = "low";

/// Alle Szenarien in Aufnahme-Reihenfolge (Mindestsatz HAR-025 AC3). Datei-Edit und
/// Reasoning-Zusammenfassung stammen bis auf Weiteres aus der Fake-CLI.
pub fn scenarios() -> Vec<Scenario> {
    use ScriptDecision::{Allow, Deny};
    vec![
        Scenario {
            name: "text",
            description: "Reiner Text-Turn",
            inputs: vec![send("Antworte nur mit dem Wort: Beton")],
            gate: vec![],
            files: &[],
            model: None,
        },
        Scenario {
            name: "command-tool",
            description: "Turn mit Shell-Kommando, Freigabe erteilt",
            inputs: vec![send(
                "Führe genau `mkdir beton-ordner` in der Shell aus und antworte danach nur mit: erledigt.",
            )],
            gate: vec![Allow, Allow],
            files: &[],
            model: None,
        },
        Scenario {
            name: "command-deny",
            description: "git push abgelehnt (HAR-006 AC2)",
            inputs: vec![send(
                "Führe genau `git push origin main` in der Shell aus. Wenn das abgelehnt wird, antworte nur mit: abgelehnt.",
            )],
            gate: vec![Deny, Deny],
            files: &[],
            model: None,
        },
        Scenario {
            name: "interrupt",
            description: "Interrupt während der Antwort",
            inputs: vec![ScriptInput {
                send: Some("Zähle von 1 bis 200, eine Zahl pro Zeile.".into()),
                interrupt_after_events: Some(6),
            }],
            gate: vec![],
            files: &[],
            model: None,
        },
        Scenario {
            name: "error",
            description: "Fehler-Turn (unbekanntes Modell)",
            inputs: vec![send("Antworte nur mit: ok")],
            gate: vec![],
            files: &[],
            model: Some("beton-gibt-es-nicht"),
        },
    ]
}

/// Notifications, die Rechner-, Konto- oder Konfigurationsdaten des Aufnehmenden tragen
/// (Hostname, Installations-ID, Namen eigener MCP-Server) und in Aufnahmen nichts verloren
/// haben.
const DROPPED: [&str; 5] = [
    "remoteControl/status/changed",
    "account/updated",
    "account/login/completed",
    "app/list/updated",
    "mcpServer/startupStatus/updated",
];

/// Entfernt Konto-, Benutzer-, Rechner- und Pfaddaten aus einer stdout-Zeile; `None` lässt
/// die Zeile weg.
fn sanitize(line: &str, replacements: &[(String, String)]) -> Option<String> {
    let mut text = line.to_owned();
    for (from, to) in replacements {
        text = text.replace(from.as_str(), to.as_str());
    }
    let Ok(mut v) = serde_json::from_str::<Value>(&text) else {
        return Some(text);
    };
    if v["method"].as_str().is_some_and(|m| DROPPED.contains(&m)) {
        return None;
    }
    if let Some(result) = v.get_mut("result").and_then(Value::as_object_mut)
        && result.contains_key("codexHome")
    {
        result.insert("codexHome".into(), Value::String("<lokal>".into()));
    }
    clean(&mut v);
    Some(v.to_string())
}

fn clean(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for key in [
                "email",
                "accountId",
                "userId",
                "installationId",
                "serverName",
            ] {
                map.remove(key);
            }
            // Tarif, Guthaben und eigene Instruktionsdateien des Aufnehmenden.
            if map.contains_key("planType") {
                map.insert("planType".into(), Value::String("<plan>".into()));
            }
            if map.contains_key("credits") {
                map.insert("credits".into(), Value::Null);
            }
            if map.contains_key("instructionSources") {
                map.insert("instructionSources".into(), Value::Array(Vec::new()));
            }
            map.values_mut().for_each(clean);
        }
        Value::Array(items) => items.iter_mut().for_each(clean),
        Value::String(s) if s.contains('@') && s.contains('.') && !s.contains(' ') => {
            *s = "<email>".into();
        }
        _ => {}
    }
}

/// Nimmt ein Szenario nach `<root>/<name>` auf und prüft es per Replay.
pub async fn record(s: &Scenario, root: &Path, version: &str) -> Result<(), String> {
    let work = tempfile::tempdir().map_err(|e| e.to_string())?;
    for (name, content) in s.files {
        std::fs::write(work.path().join(name), content).map_err(|e| e.to_string())?;
    }
    let workdir = work.path().to_path_buf();
    let canonical = workdir.canonicalize().map_err(|e| e.to_string())?;
    let home = std::env::var("HOME").unwrap_or_default();
    let mut replacements = vec![
        (
            canonical.display().to_string(),
            golden::GOLDEN_WORKDIR.to_owned(),
        ),
        (
            workdir.display().to_string(),
            golden::GOLDEN_WORKDIR.to_owned(),
        ),
    ];
    if !home.is_empty() {
        replacements.push((home, "/home/beton".into()));
    }
    let script = Script {
        model: Some(s.model.unwrap_or(RECORD_MODEL).to_owned()),
        inputs: s.inputs.clone(),
        gate: s.gate.clone(),
    };
    // Niedriger Effort über die Konfiguration der CLI (`-c`), nicht über beton.
    let codex = which("codex").ok_or("codex nicht in PATH")?;
    let mut env = HostEnv::from_process();
    env.vars.insert(
        "BETON_CODEX_PATH".into(),
        format!(
            "{} -c model_reasoning_effort={RECORD_EFFORT}",
            codex.display()
        ),
    );
    let launcher = RecordingLauncher::new(Arc::new(RealLauncher));
    let events = golden::drive(
        &CodexAdapter::default(),
        &script,
        Arc::new(launcher.clone()),
        env,
        workdir,
        Duration::from_secs(180),
    )
    .await
    .map_err(|e| format!("{}: {e}", s.name))?;
    tracing::info!(
        scenario = s.name,
        events = events.len(),
        lines = launcher.raw_lines().len(),
        "Szenario aufgenommen"
    );
    let raw: Vec<golden::RawLine> = launcher
        .raw_lines()
        .into_iter()
        .filter_map(|mut l| {
            l.out = sanitize(&l.out, &replacements)?;
            Some(l)
        })
        .collect();
    let meta = Meta {
        harness: "codex".into(),
        cli_version: version.into(),
        recorded_at: beton_core::time::Timestamp::now().to_string(),
        scenario: s.description.into(),
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        exit_code: 0,
        recorded_with: None,
    };
    let dir = root.join(s.name);
    let _ = std::fs::remove_dir_all(&dir);
    golden::write_recording(&dir, &meta, &script, &raw, &[]).map_err(|e| e.to_string())?;
    golden::check_case(&dir, &CodexAdapter::default(), true)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Version der installierten CLI.
pub async fn cli_version() -> Result<String, String> {
    CodexAdapter::default()
        .probe(&HostEnv::from_process())
        .await
        .version
        .ok_or_else(|| "codex --version liefert keine Version; ist die CLI installiert?".into())
}

fn which(cmd: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(cmd))
        .find(|p| p.is_file())
}

/// Ordner der Golden-Fälle im Repository.
pub fn default_root(repo: &Path) -> PathBuf {
    repo.join("crates/beton-harness-codex/tests/golden")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_drops_machine_and_account_data() {
        let r = vec![("/Users/x".to_owned(), "/home/beton".to_owned())];
        assert_eq!(
            sanitize(
                r#"{"method":"remoteControl/status/changed","params":{"serverName":"MB-1","installationId":"i"}}"#,
                &r
            ),
            None
        );
        let init = sanitize(
            r#"{"id":1,"result":{"userAgent":"beton/0.153.2","codexHome":"/Users/x/.codex"}}"#,
            &r,
        )
        .unwrap();
        assert!(
            init.contains("<lokal>") && !init.contains("/Users/x"),
            "{init}"
        );
        let limits = sanitize(
            r#"{"method":"account/rateLimits/updated","params":{"rateLimits":{"planType":"pro","credits":{"balance":"5"}}}}"#,
            &r,
        )
        .unwrap();
        assert!(
            !limits.contains("pro") && !limits.contains("balance"),
            "{limits}"
        );
    }
}
