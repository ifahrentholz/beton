//! `beton tui` gegen einen echten Daemon mit Fake-Harness (TUI-001, TUI-002, TUI-004). Die TUI
//! läuft kopflos auf dem Test-Backend von ratatui (`beton_tui::testing`); Terminals laufen im
//! Runner des Daemons. Keine echten Vendor-CLIs, kein Netz außer Loopback.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beton_core::event::{Event, EventPayload};
use beton_core::id::SessionId;
use beton_sdk::TerminalOpen;
use beton_sdk::ws::ServerMsg;
use beton_tui::app::AppOptions;
use beton_tui::layout::{Dir, Layout, PaneRef};
use beton_tui::testing::Headless;
use common::{Serve, events};
use crossterm::event::KeyCode;
use serde_json::json;

const WAIT: Duration = Duration::from_secs(30);

fn options(
    home: &Path,
    session: &str,
    hook: Option<beton_tui::runtime::EventHook>,
) -> beton_tui::Options {
    beton_tui::Options {
        home: home.to_path_buf(),
        session: Some(session.parse().unwrap()),
        app: AppOptions::default(),
        on_events: hook,
    }
}

async fn session_with(serve: &Serve, scenario: &str) -> String {
    let scenario = serve.scenario(scenario);
    let created = serve
        .client()
        .create_session(&json!({
            "target": "fake",
            "cwd": serve.work.path(),
            "harness_opts": {"scenario": scenario},
            "confirm_unsandboxed": true,
        }))
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_owned();
    let client = serve.client();
    common::wait_until("Session idle", || {
        let client = client.clone();
        let id = id.clone();
        async move { common::status(&client, &id).await == "idle" }
    })
    .await;
    id
}

/// Ein Event als Vergleichsschlüssel: dauerhafte über `seq`, transiente über `tseq`.
fn key(e: &Event) -> (bool, u64, Option<u64>, &'static str) {
    (e.transient, e.seq, e.tseq, e.type_name())
}

#[tokio::test]
async fn tui_001_ac1_turn_shows_the_same_event_sequence_as_the_web_ui() {
    let serve = Serve::start();
    let id = session_with(
        &serve,
        "turns:\n  - emit:\n      - { message_delta: \"Ich lese die Datei und antworte dann.\", chunk: 3 }\n      - { tool_call: { name: Read, kind: file_read, args: { file_path: \"src/main.rs\" } } }\n      - { tool_result: \"fn main() {}\" }\n      - { message: \"Fertig: die Datei ist leer.\" }\n",
    )
    .await;
    let seen = Arc::new(Mutex::new(Vec::<Event>::new()));
    let hook_seen = seen.clone();
    let hook: beton_tui::runtime::EventHook =
        Arc::new(move |events: &[Event]| hook_seen.lock().unwrap().extend_from_slice(events));
    let tui = Headless::start(
        serve.client(),
        options(serve.home(), &id, Some(hook)),
        140,
        36,
    );
    tui.wait_for("Session geladen", WAIT, |s| s.contains("Session:"))
        .await;
    // Zweiter Client wie die Web-UI: dieselbe WebSocket-API, Attach mit transienten Events.
    let session: SessionId = id.parse().unwrap();
    let mut web = serve.client().connect_ws().await.unwrap();
    web.attach(session, 0).await.unwrap();
    let web_events = Arc::new(Mutex::new(Vec::<Event>::new()));
    let collect = web_events.clone();
    let reader = tokio::spawn(async move {
        while let Some(Ok(msg)) = web.next().await {
            if let ServerMsg::Events { events, .. } = msg {
                collect.lock().unwrap().extend(events);
            }
        }
    });
    tui.type_text("Lies die Datei").await;
    tui.key(KeyCode::Enter).await;
    let screen = tui
        .wait_for("Antwort", WAIT, |s| {
            s.contains("Fertig: die Datei ist leer.")
        })
        .await;
    assert!(screen.contains("Du ▸ Lies die Datei"), "{screen}");
    assert!(
        screen.contains("Lesen") && screen.contains("src/main.rs"),
        "{screen}"
    );
    let done = |v: &Arc<Mutex<Vec<Event>>>| {
        v.lock()
            .unwrap()
            .iter()
            .any(|e| e.type_name() == "turn.completed")
    };
    let deadline = Instant::now() + WAIT;
    while !(done(&seen) && done(&web_events)) {
        assert!(Instant::now() < deadline, "turn.completed fehlt");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    reader.abort();
    let tui_keys: Vec<_> = seen
        .lock()
        .unwrap()
        .iter()
        .map(key)
        .map(|k| (k.0, k.1, k.2, k.3.to_owned()))
        .collect();
    let web_keys: Vec<_> = web_events
        .lock()
        .unwrap()
        .iter()
        .map(key)
        .map(|k| (k.0, k.1, k.2, k.3.to_owned()))
        .collect();
    assert_eq!(
        tui_keys, web_keys,
        "TUI und Web-UI sehen dieselbe Event-Folge"
    );
    assert!(tui_keys.iter().any(|k| k.3 == "message.delta"));
    assert!(tui_keys.iter().any(|k| k.3 == "tool.call.completed"));
    tui.quit().await.unwrap();
}

#[tokio::test]
async fn tui_001_ac2_500_tokens_per_second_render_at_most_30_fps_with_diffs() {
    let serve = Serve::start();
    // 40 Zeichen (≈ 10 Tokens) alle 20 ms ≈ 500 Tokens/s, gut 3 s lang.
    let words: Vec<String> = (0..1600).map(|i| format!("w{i:04}")).collect();
    let text = words.join(" ");
    let id = session_with(
        &serve,
        &format!("turns:\n  - emit:\n      - {{ message_delta: \"{text}\", chunk: 40, chunk_delay_ms: 20 }}\n"),
    )
    .await;
    let deltas = Arc::new(Mutex::new(Vec::<(Instant, usize)>::new()));
    let d = deltas.clone();
    let hook: beton_tui::runtime::EventHook = Arc::new(move |events: &[Event]| {
        for e in events {
            if let Some(EventPayload::MessageDelta(t)) = e.payload() {
                d.lock().unwrap().push((Instant::now(), t.text.len()));
            }
        }
    });
    let tui = Headless::start(
        serve.client(),
        options(serve.home(), &id, Some(hook)),
        140,
        36,
    );
    tui.wait_for("Session geladen", WAIT, |s| s.contains("Session:"))
        .await;
    tui.type_text("los").await;
    tui.key(KeyCode::Enter).await;
    tui.wait_for("Ende des Streams", WAIT, |s| s.contains("w1599"))
        .await;
    let deltas = deltas.lock().unwrap().clone();
    let (first, last) = (deltas.first().unwrap().0, deltas.last().unwrap().0);
    let secs = (last - first).as_secs_f64();
    let chars: usize = deltas.iter().map(|d| d.1).sum();
    let tokens_per_s = chars as f64 / 4.0 / secs;
    assert!(
        tokens_per_s >= 400.0,
        "der Fake-Harness liefert nur {tokens_per_s:.0} Tokens/s"
    );
    let stats = tui.stats();
    let frames: Vec<usize> = stats
        .times
        .iter()
        .zip(&stats.cells)
        .filter(|(t, _)| **t >= first && **t <= last)
        .map(|(_, c)| *c)
        .collect();
    let fps = frames.len() as f64 / secs;
    assert!(
        fps <= 31.0,
        "{fps:.1} Bilder/s bei {tokens_per_s:.0} Tokens/s"
    );
    assert!(
        fps >= 10.0,
        "nur {fps:.1} Bilder/s: die Anzeige hängt hinterher"
    );
    // Diff-Rendering: kein Bild zeichnet den ganzen Bildschirm neu, und es wird nie gelöscht.
    let total = 140 * 36;
    let max = frames.iter().copied().max().unwrap();
    assert!(
        max < total * 7 / 10,
        "ein Bild schrieb {max} von {total} Zellen"
    );
    let avg = frames.iter().sum::<usize>() / frames.len();
    assert!(
        avg < total / 2,
        "im Schnitt {avg} von {total} Zellen je Bild"
    );
    assert_eq!(stats.clears, 0, "kein Löschen des Bildschirms (Flackern)");
    tui.quit().await.unwrap();
}

const PUSH_ASK: &str = "capabilities: { approval: native_request }\nturns:\n  - emit:\n      - { message_delta: \"Ich pushe jetzt.\", chunk: 4 }\n      - { tool_call: { name: Bash, kind: shell, args: { command: \"git push origin main\" } }, gate: true }\n      - { on_gate: { allow: [{ tool_result: \"ok\" }, { message: \"Gepusht.\" }], deny: [{ message: \"Push abgelehnt.\" }] } }\n";

#[tokio::test]
async fn tui_004_ac1_y_resolves_the_approval_with_the_tui_user_as_actor() {
    let serve = Serve::start();
    let id = session_with(&serve, PUSH_ASK).await;
    let tui = Headless::start(serve.client(), options(serve.home(), &id, None), 120, 32);
    tui.wait_for("Session geladen", WAIT, |s| s.contains("Session:"))
        .await;
    tui.type_text("Bitte pushen").await;
    tui.key(KeyCode::Enter).await;
    let card = tui
        .wait_for("Freigabe-Karte", WAIT, |s| s.contains("Freigabe nötig"))
        .await;
    assert!(card.contains("git push origin main"), "{card}");
    assert!(card.contains(" y  erlauben"), "{card}");
    // Eine frisch erschienene Karte nimmt Tasten erst nach kurzer Zeit an.
    tokio::time::sleep(beton_tui::app::ARM_DELAY * 2).await;
    tui.key(KeyCode::Char('y')).await;
    tui.wait_for("Ergebnis", WAIT, |s| s.contains("Gepusht."))
        .await;
    let screen = tui
        .wait_for("Vermerk", WAIT, |s| s.contains("✓ Erlaubt mit y · Du, TUI"))
        .await;
    assert!(!screen.contains("Freigabe nötig"), "{screen}");
    let resolved: Vec<_> = events(&serve.client(), &id)
        .await
        .into_iter()
        .filter(|e| e["type"] == "approval.resolved")
        .collect();
    assert_eq!(resolved.len(), 1, "{resolved:?}");
    let p = &resolved[0]["payload"];
    assert_eq!(p["decision"], "allow");
    assert_eq!(p["via"], "user");
    assert_eq!(p["actor"], json!({"kind": "user", "id": "usr_local"}));
    tui.quit().await.unwrap();
}

#[tokio::test]
async fn tui_004_ac2_approval_in_a_hidden_session_counts_and_ctrl_g_a_jumps_there() {
    let serve = Serve::start();
    let shown = session_with(&serve, "turns: []").await;
    let other = session_with(&serve, PUSH_ASK).await;
    let client = serve.client();
    client
        .patch_session(&other, &json!({"title": "Review: Rate-Limiter"}))
        .await
        .unwrap();
    client
        .patch_session(&shown, &json!({"title": "Checkout a11y"}))
        .await
        .unwrap();
    let tui = Headless::start(serve.client(), options(serve.home(), &shown, None), 140, 36);
    tui.wait_for("Session geladen", WAIT, |s| {
        s.contains("Session: Checkout a11y")
    })
    .await;
    assert!(!tui.screen().contains("Freigabe in"));
    client.input(&other, "Bitte pushen").await.unwrap();
    tui.wait_for("Zähler in der Statuszeile", WAIT, |s| {
        s.contains("◆ 1 Freigabe in „Review: Rate-Limiter“ · ^G a springt hin")
    })
    .await;
    tui.prefix('a').await;
    let screen = tui
        .wait_for("Sprung zur Freigabe", WAIT, |s| {
            s.contains("Session: Review: Rate-Limiter") && s.contains("Freigabe nötig")
        })
        .await;
    assert!(screen.contains("git push origin main"), "{screen}");
    tokio::time::sleep(beton_tui::app::ARM_DELAY * 2).await;
    tui.key(KeyCode::Char('n')).await;
    tui.wait_for("Ablehnung", WAIT, |s| s.contains("Push abgelehnt."))
        .await;
    tui.quit().await.unwrap();
}

/// Programm wie htop: alternativer Bildschirm, Farben, Positionierung; meldet seine Größe
/// beim Start und nach jedem `SIGWINCH`.
const FULLSCREEN: &str = "printf '\\033[?1049h\\033[H\\033[2J\\033[1;32mVOLLBILD\\033[m'; \
show() { printf '\\033[2;1H\\033[KGROESSE %s' \"$(stty size)\"; }; trap show WINCH; show; \
while :; do sleep 0.05; done";

fn layout_with(terminal_id: &str) -> Layout {
    let mut l = Layout::default();
    l.split(
        Dir::Columns,
        PaneRef::Terminal {
            terminal_id: terminal_id.into(),
        },
    );
    l
}

#[tokio::test]
async fn tui_002_ac1_fullscreen_program_renders_and_resize_reaches_the_pty() {
    let serve = Serve::start();
    let id = session_with(&serve, "turns: []").await;
    let client = serve.client();
    let t = client
        .open_terminal(
            &id,
            &TerminalOpen {
                name: Some("vollbild".into()),
                command: Some(vec!["/bin/sh".into(), "-c".into(), FULLSCREEN.into()]),
                cols: 80,
                rows: 24,
            },
        )
        .await
        .unwrap();
    beton_tui::layout::save(serve.home(), &id, &layout_with(&t.terminal_id)).unwrap();
    let tui = Headless::start(serve.client(), options(serve.home(), &id, None), 120, 40);
    // 120 × 40: Pane-Fläche 120 × 37, rechte Hälfte 60 × 37, innen 58 × 35.
    let screen = tui
        .wait_for("Vollbildprogramm in Pane-Größe", WAIT, |s| {
            s.contains("VOLLBILD") && s.contains("GROESSE 35 58")
        })
        .await;
    assert!(screen.contains("1 vollbild"), "{screen}");
    assert!(screen.contains("PTY 58×35"), "{screen}");
    tui.resize(160, 50).await;
    // 160 × 50: rechte Hälfte 80 × 47, innen 78 × 45.
    tui.wait_for("neue PTY-Größe", WAIT, |s| s.contains("GROESSE 45 78"))
        .await;
    let terminals = client.terminals(&id).await.unwrap();
    assert_eq!((terminals[0].cols, terminals[0].rows), (78, 45));
    tui.quit().await.unwrap();
}

#[tokio::test]
async fn tui_002_ac1_htop_renders_in_a_pane() {
    let htop = [
        "/usr/bin/htop",
        "/opt/homebrew/bin/htop",
        "/usr/local/bin/htop",
    ]
    .into_iter()
    .find(|p| Path::new(p).is_file());
    let Some(htop) = htop else {
        eprintln!("htop ist nicht installiert; das Vollbildprogramm-Szenario deckt AC1 ab");
        return;
    };
    let serve = Serve::start_with(&[("TERM", "xterm-256color")]);
    let id = session_with(&serve, "turns: []").await;
    let t = serve
        .client()
        .open_terminal(
            &id,
            &TerminalOpen {
                name: Some("htop".into()),
                command: Some(vec![htop.into()]),
                cols: 80,
                rows: 24,
            },
        )
        .await
        .unwrap();
    beton_tui::layout::save(serve.home(), &id, &layout_with(&t.terminal_id)).unwrap();
    let tui = Headless::start(serve.client(), options(serve.home(), &id, None), 140, 40);
    tui.wait_for("htop-Kopf", WAIT, |s| {
        s.contains("PID") && s.contains("CPU")
    })
    .await;
    tui.quit().await.unwrap();
}

/// Daemon, dessen Terminals `/bin/sh` starten; mit `fakebin` vorn im `PATH`.
fn shell_daemon(fakebin: Option<&Path>) -> Serve {
    let path = match fakebin {
        Some(dir) => format!(
            "{}:{}",
            dir.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
        None => std::env::var("PATH").unwrap_or_default(),
    };
    Serve::start_with(&[("SHELL", "/bin/sh"), ("PATH", &path), ("ENV", "")])
}

/// PID aus der Ausgabezeile `PID=<n>` (nicht aus der Befehlszeile mit `PID=$$`).
fn pid_in(screen: &str) -> Option<String> {
    screen
        .lines()
        .filter_map(|l| l.split("PID=").nth(1))
        .map(|r| {
            r.split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap_or("")
                .to_owned()
        })
        .find(|p| !p.is_empty())
}

async fn shell_line(tui: &Headless, command: &str) {
    tui.type_text(command).await;
    tui.key(KeyCode::Enter).await;
}

#[tokio::test]
async fn tui_002_ac2_reopening_restores_panes_processes_and_scrollback() {
    let serve = shell_daemon(None);
    let id = session_with(&serve, "turns: []").await;
    let tui = Headless::start(serve.client(), options(serve.home(), &id, None), 120, 40);
    tui.wait_for("Session geladen", WAIT, |s| s.contains("Session:"))
        .await;
    tui.prefix('|').await;
    tui.wait_for("neues Pane", WAIT, |s| {
        s.contains("2 Panes") && s.contains("PTY 58×35")
    })
    .await;
    shell_line(&tui, "echo MARKE-$((40+2)); echo PID=$$").await;
    let first = tui
        .wait_for("Ausgabe im Pane", WAIT, |s| {
            s.contains("MARKE-42") && pid_in(s).is_some()
        })
        .await;
    let pid = pid_in(&first).unwrap_or_default();
    assert!(!pid.is_empty(), "{first}");
    tui.quit().await.unwrap();
    assert!(beton_tui::layout::layout_path(serve.home(), &id).is_file());

    let tui = Headless::start(serve.client(), options(serve.home(), &id, None), 120, 40);
    let screen = tui
        .wait_for("wiederhergestellt", WAIT, |s| {
            s.contains("✓ Wieder verbunden.") && s.contains("MARKE-42")
        })
        .await;
    assert!(screen.contains("2 Panes wiederhergestellt"), "{screen}");
    shell_line(&tui, "echo NOCH=$$").await;
    tui.wait_for("derselbe Prozess", WAIT, |s| {
        s.contains(&format!("NOCH={pid}"))
    })
    .await;
    tui.quit().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn tui_002_ac3_no_tmux_binary_is_called() {
    use std::os::unix::fs::PermissionsExt as _;
    let fakebin = tempfile::tempdir().unwrap();
    let trace = fakebin.path().join("tmux-called");
    let tmux = fakebin.path().join("tmux");
    std::fs::write(
        &tmux,
        format!("#!/bin/sh\necho \"$@\" >> {}\nexit 1\n", trace.display()),
    )
    .unwrap();
    std::fs::set_permissions(&tmux, std::fs::Permissions::from_mode(0o755)).unwrap();
    let serve = shell_daemon(Some(fakebin.path()));
    let id = session_with(&serve, "turns: []").await;
    let tui = Headless::start(serve.client(), options(serve.home(), &id, None), 120, 40);
    tui.wait_for("Session geladen", WAIT, |s| s.contains("Session:"))
        .await;
    // Teilen, zoomen, wechseln, schließen, trennen und neu verbinden: alles ohne tmux.
    tui.prefix('|').await;
    tui.wait_for("Pane 1", WAIT, |s| s.contains("2 Panes"))
        .await;
    tui.prefix('-').await;
    tui.wait_for("Pane 2", WAIT, |s| s.contains("3 Panes"))
        .await;
    // Der Prozess-Trace: `command -v tmux` sieht nur den Fake, der jeden Aufruf protokolliert.
    shell_line(&tui, "command -v tmux").await;
    tui.wait_for("Fake-tmux im PATH", WAIT, |s| s.contains("/tmux"))
        .await;
    tui.prefix('z').await;
    tui.wait_for("gezoomt", WAIT, |s| s.contains("[Z]")).await;
    tui.prefix('o').await;
    tui.prefix('x').await;
    tui.quit().await.unwrap();
    let tui = Headless::start(serve.client(), options(serve.home(), &id, None), 120, 40);
    tui.wait_for("neu verbunden", WAIT, |s| s.contains("Panes"))
        .await;
    tui.quit().await.unwrap();
    assert!(
        !trace.exists(),
        "tmux wurde aufgerufen: {}",
        std::fs::read_to_string(&trace).unwrap_or_default()
    );
    assert!(!serve.log().contains("tmux"), "{}", serve.log());
}

#[test]
fn tui_001_cli_refuses_without_a_terminal() {
    let home = tempfile::tempdir().unwrap();
    let out = common::run(common::beton(home.path()).arg("tui"));
    assert_eq!(out.status.code(), Some(2), "{}", common::stderr(&out));
    assert!(common::stderr(&out).contains("braucht ein Terminal"));
}
