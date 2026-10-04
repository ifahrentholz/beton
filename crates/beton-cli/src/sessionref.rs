//! Session-Referenzen (CLI-003): vollständige ID, eindeutiges Präfix oder `last`.

use beton_sdk::Client;
use serde_json::Value;

use crate::exit::{CliError, CliResult, Exit};

/// Löst eine Referenz in eine Session-ID auf. Ein mehrdeutiges Präfix listet die Kandidaten
/// und endet mit Exit-Code 2 (CLI-003 AC1).
pub async fn resolve(client: &Client, reference: &str) -> CliResult<String> {
    let reference = reference.trim();
    if reference.parse::<beton_core::id::SessionId>().is_ok() {
        return Ok(reference.to_owned());
    }
    let sessions = client.all_sessions(true).await?;
    if reference == "last" {
        return latest(sessions.iter().filter(|s| s["archived"] != true)).ok_or_else(|| {
            CliError::new(Exit::General, anyhow::anyhow!("Es gibt noch keine Session"))
        });
    }
    let matches: Vec<&Value> = sessions
        .iter()
        .filter(|s| {
            let id = s["id"].as_str().unwrap_or_default();
            id.starts_with(reference)
                || id
                    .strip_prefix("ses_")
                    .is_some_and(|r| r.starts_with(reference))
        })
        .collect();
    match matches.as_slice() {
        [] => Err(CliError::new(
            Exit::General,
            anyhow::anyhow!("Keine Session passt zu `{reference}`"),
        )),
        [one] => Ok(one["id"].as_str().unwrap_or_default().to_owned()),
        many => {
            let list: Vec<String> = many
                .iter()
                .map(|s| {
                    format!(
                        "  {}  {}  {}",
                        s["id"].as_str().unwrap_or_default(),
                        s["status"].as_str().unwrap_or_default(),
                        s["title"].as_str().unwrap_or_default()
                    )
                })
                .collect();
            Err(CliError::usage(format!(
                "`{reference}` ist mehrdeutig ({} Sessions):\n{}",
                many.len(),
                list.join("\n")
            )))
        }
    }
}

/// Session mit der jüngsten Aktivität.
pub fn latest<'a>(sessions: impl Iterator<Item = &'a Value>) -> Option<String> {
    sessions
        .max_by(|a, b| {
            let key = |s: &Value| {
                s["last_activity_at"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            };
            key(a).cmp(&key(b))
        })
        .and_then(|s| s["id"].as_str().map(str::to_owned))
}

/// Arbeitsverzeichnis einer Session aus ihrem `session.created`-Event.
pub async fn cwd_of(client: &Client, id: &str) -> CliResult<Option<String>> {
    let page = client.events(id, 0, 1).await?;
    Ok(page
        .items
        .first()
        .filter(|e| e["type"] == "session.created")
        .and_then(|e| e["payload"]["cwd"].as_str().map(str::to_owned)))
}

/// Zuletzt genutzte, nicht archivierte Session mit diesem Arbeitsverzeichnis (CLI-002 AC2).
pub async fn last_in_dir(client: &Client, cwd: &str) -> CliResult<Option<String>> {
    let mut sessions: Vec<Value> = client
        .all_sessions(false)
        .await?
        .into_iter()
        .filter(|s| s["archived"] != true)
        .collect();
    sessions.sort_by(|a, b| {
        let key = |s: &Value| {
            s["last_activity_at"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        };
        key(b).cmp(&key(a))
    });
    for s in sessions {
        let Some(id) = s["id"].as_str() else { continue };
        if cwd_of(client, id).await?.as_deref() == Some(cwd) {
            return Ok(Some(id.to_owned()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn latest_picks_most_recent_activity() {
        let sessions = [
            json!({"id": "ses_a", "last_activity_at": "2026-10-01T10:00:00Z"}),
            json!({"id": "ses_b", "last_activity_at": "2026-10-03T10:00:00Z"}),
            json!({"id": "ses_c", "last_activity_at": "2026-10-02T10:00:00Z"}),
        ];
        assert_eq!(latest(sessions.iter()).as_deref(), Some("ses_b"));
        assert_eq!(latest([].iter()), None);
    }
}
