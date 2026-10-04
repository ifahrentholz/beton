//! Beispiel (API-005 AC2): legt eine Session mit dem Fake-Harness an, schickt eine Eingabe
//! und gibt die Text-Deltas aus.
//!
//! ```text
//! beton serve --dev            # Daemon mit Fake-Harness (Debug-Builds: immer)
//! cargo run -p beton-sdk --example stream
//! ```
//!
//! Der Daemon wird über `BETON_HOME` (sonst `~/.beton`) gefunden.

use std::io::Write as _;
use std::path::PathBuf;

use beton_core::event::{EventPayload, SessionStatus};
use beton_sdk::Client;
use beton_sdk::ws::Update;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var_os("BETON_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".beton")))
        .ok_or("weder BETON_HOME noch HOME gesetzt")?;
    let client = Client::local(&home)?;

    let work = tempfile::tempdir()?;
    let scenario = work.path().join("hallo.yaml");
    std::fs::write(
        &scenario,
        "turns:\n  - expect_input: \"hallo\"\n    emit:\n      - { message_delta: \"Hallo aus dem Fake-Harness\", chunk: 5 }\n",
    )?;
    let created = client
        .create_session(&json!({
            "target": "fake",
            "cwd": work.path(),
            "harness_opts": { "scenario": scenario },
        }))
        .await?;
    let id = created["id"].as_str().ok_or("Antwort ohne ID")?.to_owned();
    eprintln!("Session {id}");

    let mut sub = client.subscribe(id.parse()?, 0).await?;
    let mut sent = false;
    loop {
        let Update::Events(events) = sub.next().await? else {
            continue;
        };
        for event in events {
            match event.payload() {
                Some(EventPayload::SessionStatus(s))
                    if s.status == SessionStatus::Idle && !sent =>
                {
                    client.input(&id, "hallo").await?;
                    sent = true;
                }
                Some(EventPayload::MessageDelta(d)) => {
                    print!("{}", d.text);
                    std::io::stdout().flush()?;
                }
                Some(EventPayload::TurnCompleted(_)) => {
                    println!();
                    return Ok(());
                }
                _ => {}
            }
        }
    }
}
