//! Der eigene beton-Server einer Session heißt beim Harness `beton_<nonce>` (#252, M1 aus dem
//! Security-Review von #385).
//!
//! Definiert die Vendor-Konfiguration des Users selbst einen Server `beton`, ließe sich am
//! Namen nicht unterscheiden, welcher Server einen Aufruf bekommt. beton liest diese
//! Konfiguration nicht (ADR-0005), sondern bindet seinen Server je Session unter einem nicht
//! vorhersagbaren Namen ein (Nonce aus dem CSPRNG). Nur genau dieser Name ist der System-Server;
//! in Policies und im Log heißt er `beton`. Fremde Server mit reservierten Namen bleiben
//! `untrusted:<name>` (POL-005). Harnesses im Runner-Prozess (Direkt-API, Fake) haben keine
//! Vendor-Konfiguration und behalten `beton`.

use beton_core::event::EventPayload;
use beton_harness::{HarnessId, McpInjection};

/// Name des System-Servers in Policies und im Log.
pub const CANONICAL: &str = beton_mcp::config::SYSTEM_SERVER;

/// Ein neuer Name `beton_<16 Hex>` (64 Bit aus dem CSPRNG).
pub fn fresh_name() -> Result<String, String> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|e| format!("Zufallszahlen: {e}"))?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("{CANONICAL}_{hex}"))
}

/// Harnesses im Runner-Prozess ohne Vendor-Konfiguration (Direkt-API, Fake) behalten `beton`.
fn in_process(harness: &HarnessId) -> bool {
    matches!(
        beton_policy::normalize::harness_family(harness.as_str()),
        "direct" | "fake"
    )
}

/// Benennt den System-Server der Injektion für Harnesses mit eigener Konfiguration um und
/// liefert den neuen Namen. Der Relay startet weiter den internen Server (`mcp serve`).
pub fn expose(injection: &mut McpInjection, harness: &HarnessId) -> Result<Option<String>, String> {
    if in_process(harness) {
        return Ok(None);
    }
    let Some(server) = injection.servers.iter_mut().find(|s| s.name == CANONICAL) else {
        return Ok(None);
    };
    let name = fresh_name()?;
    server.name.clone_from(&name);
    Ok(Some(name))
}

/// Name eines Servers für Policies und Log: der eigene `beton`, fremde reservierte
/// `untrusted:<name>`, alle anderen unverändert.
pub fn canonical(server: &str, exposed: &str) -> String {
    if server == exposed {
        CANONICAL.to_owned()
    } else if beton_mcp::config::reserved_name(server) {
        format!("untrusted:{server}")
    } else {
        server.to_owned()
    }
}

/// Bildet Namen des Harness im Log ab: der eigene Server wird `beton` (`source: beton_mcp`,
/// Klasse `system`), fremde reservierte Server `untrusted:<name>`.
pub fn map_event(payload: &mut EventPayload, exposed: &str) {
    match payload {
        EventPayload::ToolCallRequested(r) => {
            let Some(server) = r.mcp_server.as_deref() else {
                return;
            };
            if server == exposed {
                r.mcp_server = Some(CANONICAL.to_owned());
                r.source = beton_core::event::ToolSource::BetonMcp;
                r.kind = Some("system".into());
            } else if beton_mcp::config::reserved_name(server) {
                r.mcp_server = Some(canonical(server, exposed));
                if r.source == beton_core::event::ToolSource::BetonMcp {
                    r.source = beton_core::event::ToolSource::Harness;
                }
                if r.kind.as_deref() == Some("system") {
                    r.kind = Some("mcp".into());
                }
            }
        }
        EventPayload::HarnessReady(h) => {
            for tool in &mut h.tools {
                if let Some(rest) = tool.strip_prefix("mcp__")
                    && let Some((server, name)) = rest.split_once("__")
                {
                    let mapped = canonical(server, exposed);
                    if mapped != server {
                        *tool = format!("mcp__{mapped}__{name}");
                    }
                }
            }
            for server in &mut h.mcp_servers {
                *server = canonical(server, exposed);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beton_core::event::{HarnessReady, ToolCallRequested, ToolSource};
    use beton_harness::McpLaunch;
    use serde_json::json;

    fn injection() -> McpInjection {
        let launch = |name: &str| McpLaunch {
            name: name.into(),
            command: "/bin/beton".into(),
            args: vec!["mcp".into(), "serve".into()],
        };
        McpInjection {
            servers: vec![launch("beton"), launch("github")],
            ..McpInjection::default()
        }
    }

    fn is_session_name(name: &str) -> bool {
        name.strip_prefix("beton_").is_some_and(|n| {
            n.len() == 16
                && n.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        })
    }

    #[test]
    fn pol_005_session_server_gets_an_unpredictable_name() {
        let a = fresh_name().unwrap();
        let b = fresh_name().unwrap();
        assert!(is_session_name(&a), "{a}");
        assert_ne!(a, b, "je Session neu");
        for harness in ["claude", "codex", "acp:gemini"] {
            let mut inj = injection();
            let name = expose(&mut inj, &harness.parse().unwrap())
                .unwrap()
                .unwrap();
            assert!(is_session_name(&name), "{harness}: {name}");
            assert_eq!(inj.names(), [name.clone(), "github".to_owned()]);
            // Der Relay startet weiter den internen Server (`mcp serve`).
            assert_eq!(inj.servers[0].args, ["mcp", "serve"]);
        }
        for harness in ["direct:anthropic", "fake"] {
            let mut inj = injection();
            assert_eq!(expose(&mut inj, &harness.parse().unwrap()).unwrap(), None);
            assert_eq!(inj.names(), ["beton", "github"]);
        }
    }

    #[test]
    fn pol_005_log_names_the_session_server_beton_and_foreign_ones_untrusted() {
        let own = "beton_0123456789abcdef";
        let requested = |server: &str| {
            EventPayload::ToolCallRequested(ToolCallRequested {
                call_id: "c".into(),
                tool: "session_list".into(),
                kind: Some("mcp".into()),
                mcp_server: Some(server.into()),
                args: json!({}),
                source: ToolSource::Harness,
                parent_call_id: None,
            })
        };
        let mut p = requested(own);
        map_event(&mut p, own);
        let EventPayload::ToolCallRequested(r) = &p else {
            unreachable!()
        };
        assert_eq!(r.mcp_server.as_deref(), Some("beton"));
        assert_eq!(r.source, ToolSource::BetonMcp);
        assert_eq!(r.kind.as_deref(), Some("system"));
        for foreign in ["beton", "beton_fedcba9876543210"] {
            let mut p = requested(foreign);
            if let EventPayload::ToolCallRequested(r) = &mut p {
                r.source = ToolSource::BetonMcp;
            }
            map_event(&mut p, own);
            let EventPayload::ToolCallRequested(r) = &p else {
                unreachable!()
            };
            assert_eq!(
                r.mcp_server.as_deref(),
                Some(format!("untrusted:{foreign}").as_str())
            );
            assert_eq!(r.source, ToolSource::Harness, "{foreign}");
        }
        let mut ready = EventPayload::HarnessReady(HarnessReady {
            harness_session_ref: None,
            tools: vec![
                format!("mcp__{own}__policy_query"),
                "mcp__beton__x".into(),
                "Bash".into(),
            ],
            mcp_servers: vec![own.into(), "beton".into(), "github".into()],
        });
        map_event(&mut ready, own);
        let EventPayload::HarnessReady(r) = &ready else {
            unreachable!()
        };
        assert_eq!(
            r.tools,
            [
                "mcp__beton__policy_query",
                "mcp__untrusted:beton__x",
                "Bash"
            ]
        );
        assert_eq!(r.mcp_servers, ["beton", "untrusted:beton", "github"]);
    }
}
