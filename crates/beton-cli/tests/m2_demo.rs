//! M2-Demo als automatisierter End-to-End-Test (Milestone „M2 — Kontrolle“, WP-49):
//!
//! Ein Agent läuft im YOLO-Mode in der Sandbox: Er kann `~/.ssh` nicht lesen und nur erlaubte
//! Hosts erreichen (`egress.blocked`), er sieht Secrets nur als `bt_cred_*`, `git push --force`
//! erzeugt eine Approval-Card (`force_push: ask` der User-Defaults), und bei Überschreitung des
//! Budgets wird gestoppt.
//!
//! Gegen `beton serve --dev` als eigenen Prozess mit Default-Sandbox (`auto`), der Fake-CLI als
//! `claude` in Stufe 1 (QA-002) und echten Bash-Aufrufen über `bt-exec` in Stufe 2. `HOME` ist
//! ein temporäres Fake-Home mit synthetischer Schlüsseldatei; Secrets liegen im Datei-Backend
//! (nie im Schlüsselbund); kein Netz außer Loopback.

#![allow(
    clippy::disallowed_methods,
    reason = "Testaufbau startet Prozesse direkt (SBX-001 gilt für Harness und Tools)"
)]
#![allow(clippy::unwrap_used)]
#![cfg(unix)]

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{Serve, fake_claude};
use serde_json::{Value, json};

/// Inhalt der synthetischen Schlüsseldatei im Fake-Home.
const KEY: &str = "FIXTURE-SSH-KEY-M2-DEMO-0815";
/// Wert des Credentials in der Umgebung des Daemons; darf nie in der Sandbox erscheinen.
const SECRET: &str = "ghp_FIXTUREm2demoSECRETvalue0815";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn bash(command: &str, gate: bool) -> Value {
    json!({ "tool_call": { "name": "Bash", "kind": "shell", "args": { "command": command } }, "gate": gate, "run": true })
}

async fn events(serve: &Serve, id: &str) -> Vec<Value> {
    common::events(&serve.client(), id).await
}

async fn wait_for(serve: &Serve, id: &str, pred: impl Fn(&Value) -> bool) -> Vec<Value> {
    let start = Instant::now();
    loop {
        let events = events(serve, id).await;
        if events.iter().any(&pred) {
            return events;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "Event fehlt: {}",
            serde_json::to_string_pretty(&events).unwrap()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Ergebnis des Tool-Calls mit diesem Kommando.
fn result_of<'a>(events: &'a [Value], command: &str) -> &'a Value {
    let call = events
        .iter()
        .find(|e| e["type"] == "tool.call.requested" && e["payload"]["args"]["command"] == command)
        .unwrap_or_else(|| panic!("kein Tool-Call {command}"));
    let id = call["payload"]["call_id"].clone();
    &events
        .iter()
        .find(|e| e["type"] == "tool.call.completed" && e["payload"]["call_id"] == id)
        .unwrap_or_else(|| panic!("kein Ergebnis für {command}"))["payload"]
}

#[tokio::test]
async fn m2_demo_yolo_in_the_sandbox_secrets_egress_force_push_and_budget() {
    let fx = tempfile::tempdir().unwrap();
    // Fake-Home mit synthetischer Schlüsseldatei.
    let home = fx.path().join("home");
    write(&home.join(".ssh/id_ed25519"), KEY);
    // Passphrase für das Datei-Backend der Secrets (beton-CA des Proxys).
    let pass = fx.path().join("pass");
    write(&pass, "m2-demo-passphrase-0815\n");
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&pass, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    // Projekt der Session; das Szenario liegt darin, damit die Fake-CLI es in Stufe 1 liest.
    let work = fx.path().join("projekt");
    std::fs::create_dir_all(&work).unwrap();
    // Ein Turn, der alles zeigt; dann ein zweiter, den das Budget stoppt.
    let scenario = work.join(".demo.yaml");
    let turn1 = json!({ "expect_input": "Zeig die Sandbox", "emit": [
        bash("cat ~/.ssh/id_ed25519", true),
        bash("curl -s -o /dev/null -m 10 -w '%{http_code}' http://blocked.example/", true),
        bash("env | grep '^GH_TOKEN='", true),
        bash("git push --force origin feature", true),
        { "message": "fertig" },
        // Abo-Nutzung zählt mit dem Listenpreis-Äquivalent (ADR-0034): weit über 0,50 USD.
        { "usage": { "model": "claude-sonnet-4-5", "input_tokens": 400000, "output_tokens": 100000 } },
    ]});
    let turn2 = json!({ "emit": [{ "message": "noch mehr" }] });
    write(
        &scenario,
        &serde_json::to_string(&json!({ "turns": [turn1, turn2] })).unwrap(),
    );
    let claude = fake_claude(&scenario);
    let config = "sandbox:\n  credentials:\n    - { name: github, type: github, source: { env: DEMO_GH_TOKEN }, env: [GH_TOKEN] }\n";
    let serve = Serve::start_with_config(
        config,
        &[
            ("HOME", home.to_str().unwrap()),
            ("BETON_CLAUDE_PATH", claude.as_str()),
            ("BETON_CFG_SECRETS__BACKEND", "file"),
            ("BETON_SECRETS_PASSPHRASE_FILE", pass.to_str().unwrap()),
            ("DEMO_GH_TOKEN", SECRET),
        ],
    );
    // Die User-Defaults (`beton setup`): `force_push: ask`.
    let defaults = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../policies/user-defaults.yaml"),
    )
    .unwrap();
    write(&serve.home().join("policies/defaults.yaml"), &defaults);

    // YOLO startet, weil Sandbox und Egress-Proxy aktiv sind (HAR-027, SBX-006).
    let client = serve.client();
    let created = client
        .create_session(&json!({
            "target": "claude",
            "cwd": work,
            "permission_mode": "yolo",
            "max_cost_usd": 0.5,
            // Mit Titel: kein Einmal-Aufruf für einen generierten Titel (der zählte mit).
            "title": "M2-Demo",
            // Modell mit Katalogpreis: Abo-Nutzung zählt mit dem Listenpreis-Äquivalent.
            "model": "claude-sonnet-4-5",
        }))
        .await;
    let created = match created {
        Ok(c) => c,
        Err(e) => {
            let sessions = client.all_sessions(true).await.unwrap_or_default();
            let mut dump = String::new();
            for s in &sessions {
                let id = s["id"].as_str().unwrap_or_default();
                dump.push_str(&serde_json::to_string_pretty(&events(&serve, id).await).unwrap());
            }
            panic!("{e}\n{dump}\n{}", serve.log());
        }
    };
    let id = created["id"].as_str().unwrap().to_owned();
    if let Err(e) = client.input(&id, "Zeig die Sandbox").await {
        let dump = serde_json::to_string_pretty(&events(&serve, &id).await).unwrap();
        panic!("{e}\n{dump}\n{}", serve.log());
    }

    // `git push --force` erzeugt eine Approval-Card; der Mensch lehnt ab.
    let events = wait_for(&serve, &id, |e| e["type"] == "approval.requested").await;
    let card = events
        .iter()
        .find(|e| e["type"] == "approval.requested")
        .unwrap();
    assert!(card.to_string().contains("git push --force"), "{card}");
    let approval = card["payload"]["approval_id"].as_str().unwrap();
    client
        .resolve_approval(&id, approval, false, Some("kein Force-Push"))
        .await
        .unwrap();
    let events = wait_for(&serve, &id, |e| e["type"] == "turn.completed").await;
    let text = serde_json::to_string(&events).unwrap();

    // In der Sandbox: Harness in Stufe 1, Bash in Stufe 2, beides mit echtem Backend.
    for stage in ["harness", "tools"] {
        assert!(
            events.iter().any(|e| e["type"] == "sandbox.started"
                && e["payload"]["stage"] == stage
                && e["payload"]["backend"] != "none"),
            "{stage}: {text}"
        );
    }
    // `~/.ssh` ist nicht lesbar.
    let key = result_of(&events, "cat ~/.ssh/id_ed25519");
    assert_ne!(key["status"], "ok", "{key}");
    // Der Grund ist die Sandbox (Seatbelt: „Operation not permitted“, Landlock: „Permission
    // denied“, Maske unter bwrap: nicht vorhanden), nicht ein anderer Fehler des Tools.
    let why = key["result"].to_string();
    assert!(
        [
            "Operation not permitted",
            "Permission denied",
            "No such file"
        ]
        .iter()
        .any(|m| why.contains(m)),
        "{key}"
    );
    assert!(!text.contains(KEY), "Schlüssel im Log");
    // Nicht erlaubter Host: blockiert und gemeldet.
    let curl = result_of(
        &events,
        "curl -s -o /dev/null -m 10 -w '%{http_code}' http://blocked.example/",
    );
    assert!(curl["result"].to_string().contains("403"), "{curl}");
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "egress.blocked" && e.to_string().contains("blocked.example")),
        "{text}"
    );
    // Secrets nur als Platzhalter.
    let env = result_of(&events, "env | grep '^GH_TOKEN='");
    assert!(
        env["result"].to_string().contains("GH_TOKEN=bt_cred_"),
        "{env}"
    );
    assert!(!text.contains(SECRET), "Secret im Log");
    // Force-Push: Card abgelehnt, nicht ausgeführt.
    let push = result_of(&events, "git push --force origin feature");
    assert_eq!(push["status"], "denied", "{push}");

    // Das Budget (0,50 USD) ist überschritten: Der nächste Turn startet nicht, abgelehnt von
    // der Budget-Regel der Session (`session-budget/max-cost`, API-006).
    // Abgelehnt beim Annehmen der Eingabe (403) oder vor der Modell-Anfrage des Turns.
    let _ = client.input(&id, "weiter").await;
    let is_deny = |e: &Value| {
        e["type"] == "policy.decision"
            && e["payload"]["phase"] == "model_request"
            && e["payload"]["outcome"] == "deny"
    };
    let events = wait_for(&serve, &id, is_deny).await;
    let denied = events.iter().rev().find(|e| is_deny(e)).unwrap();
    assert_eq!(
        denied["payload"]["matched"][0]["set"], "session-budget",
        "{denied}"
    );
    assert_eq!(
        denied["payload"]["matched"][0]["rule_id"], "max-cost",
        "{denied}"
    );
    assert!(!events.iter().any(|e| e.to_string().contains("noch mehr")));
    let all = serde_json::to_string(&events_final(&serve, &id).await).unwrap();
    assert!(!all.contains(SECRET) && !all.contains(KEY));
}

async fn events_final(serve: &Serve, id: &str) -> Vec<Value> {
    tokio::time::sleep(Duration::from_millis(300)).await;
    events(serve, id).await
}
