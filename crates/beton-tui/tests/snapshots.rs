//! Snapshot-Tests der Layouts gegen das Test-Backend von ratatui (TUI-001, TUI-002, TUI-004,
//! TUI-005, USE-008): dieselben Zustände wie der Prototyp (`design/prototype/src/screens/tui/`).
//! Erwartungen neu schreiben mit `BETON_BLESS=1 cargo test -p beton-tui --test snapshots`.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use beton_core::event::{Actor, Event, EventPayload};
use beton_core::id::{ApprovalId, SessionId};
use beton_core::time::Timestamp;
use beton_sdk::Terminal as SdkTerminal;
use beton_sdk::channel::ChannelEvent;
use beton_tui::app::{App, AppOptions, Effect, Msg, WindowsHost};
use beton_tui::keys::Input;
use beton_tui::layout::{Dir, Layout, PaneRef};
use beton_tui::theme;
use beton_tui::view::{buffer_lines, render};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use serde_json::{Value, json};
use time::{OffsetDateTime, UtcOffset};

const SESSION: &str = "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C";
const OTHER: &str = "ses_01JB8Y2D0M3K4J5H6G7F8E9D0D";

fn sid() -> SessionId {
    SESSION.parse().unwrap()
}

fn at(min: i64, sec: i64) -> OffsetDateTime {
    // 07:41 UTC = 09:41 MESZ wie im Prototyp.
    OffsetDateTime::from_unix_timestamp(1_792_222_860).unwrap()
        + time::Duration::minutes(min)
        + time::Duration::seconds(sec)
}

fn ev(seq: u64, kind: &str, payload: Value) -> Event {
    let body: EventPayload =
        serde_json::from_value(json!({"type": kind, "payload": payload})).unwrap();
    let mut e = Event::new(sid(), seq, Actor::default(), body);
    e.ts = Timestamp::from(at(0, seq as i64));
    if e.body.event_type().persistence() == beton_core::event::Persistence::Transient {
        e.transient = true;
    }
    e
}

fn user(seq: u64, text: &str) -> Event {
    ev(
        seq,
        "message.completed",
        json!({"message_id": format!("u{seq}"), "role": "user", "content": [{"type": "text", "text": text}]}),
    )
}

fn assistant(seq: u64, text: &str) -> Event {
    ev(
        seq,
        "message.completed",
        json!({"message_id": format!("a{seq}"), "role": "assistant", "content": [{"type": "text", "text": text}]}),
    )
}

fn tool(seq: u64, id: &str, name: &str, kind: &str, args: Value, ms: Option<u64>) -> Vec<Event> {
    let mut out = vec![ev(
        seq,
        "tool.call.requested",
        json!({"call_id": id, "tool": name, "kind": kind, "args": args, "source": "harness"}),
    )];
    if let Some(ms) = ms {
        out.push(ev(
            seq + 1,
            "tool.call.completed",
            json!({"call_id": id, "status": "ok", "duration_ms": ms}),
        ));
    }
    out
}

/// Verlauf der Rate-Limiter-Session aus dem Prototyp.
fn history(streaming: bool) -> Vec<Event> {
    let mut e = vec![
        ev(
            1,
            "session.created",
            json!({"owner": "usr_local", "kind": "main", "harness": "claude", "model": "claude-opus-5-5", "cwd": "/Users/ingo/code/shop-frontend", "trigger": "user"}),
        ),
        ev(
            2,
            "session.title_changed",
            json!({"title": "Rate-Limiter für die Login-API", "source": "generated"}),
        ),
        user(
            3,
            "Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP. Bitte mit Tests.",
        ),
        assistant(
            4,
            "Ich lege eine Express-Middleware mit Token-Bucket pro IP an.",
        ),
    ];
    e.extend(tool(
        5,
        "c1",
        "Grep",
        "search",
        json!({"pattern": "rg 'auth.post' src/"}),
        Some(200),
    ));
    e.extend(tool(
        7,
        "c2",
        "Read",
        "file_read",
        json!({"file_path": "src/routes/auth.ts"}),
        Some(100),
    ));
    e.extend(tool(9, "c3", "Edit", "file_edit", json!({"file_path": "src/routes/auth.ts", "old_string": "auth.post('/login', login)", "new_string": "import { rateLimit } from '../middleware/rate-limit'\nauth.post('/login', rateLimit({ window: '1m', max: 5, key: 'ip' }), login)"}), Some(300)));
    e.extend(tool(
        11,
        "c4",
        "Bash",
        "shell",
        json!({"command": "pnpm vitest run auth"}),
        Some(4800),
    ));
    e.push(ev(13, "terminal.opened", json!({"terminal_id": "term_1", "channel_id": 1, "kind": "user", "cmd": "pnpm dev", "cols": 96, "rows": 14, "name": "dev-server"})));
    e.push(ev(
        14,
        "context.usage",
        json!({"used_tokens": 72_000, "window_tokens": 200_000, "source": "harness"}),
    ));
    e.push(ev(15, "usage.subscription", json!({"vendor": "anthropic", "window": "5h", "used_pct": 38.0, "resets_at": "2026-10-17T10:00:00Z"})));
    if streaming {
        e.push(ev(16, "session.status", json!({"status": "running"})));
        e.push(ev(17, "queue.updated", json!({"items": [{"id": "q1", "author": "usr_local", "text": "Danach bitte auch die Register-Route absichern", "attachments": [], "created_at": "2026-10-17T07:41:00Z"}], "paused": false})));
        e.push(ev(17, "message.delta", json!({"message_id": "m9", "text": "Die Login-Route ist auf 5 Versuche pro Minute und IP begrenzt; danach kommt 429 mit Retry-After. Ich schreibe noch den Test für den Header"})));
    } else {
        e.push(assistant(16, "Die Login-Route ist auf 5 Versuche pro Minute und IP begrenzt; danach kommt 429 mit Retry-After. Soll ich pushen und einen PR öffnen?"));
    }
    e
}

fn approval_events(id: ApprovalId) -> Vec<Event> {
    let mut e = vec![
        user(18, "Ja, push und PR."),
        ev(19, "session.status", json!({"status": "waiting_approval"})),
    ];
    e.extend(tool(
        20,
        "c5",
        "Bash",
        "shell",
        json!({"command": "git push -u origin beton/rate-limiter-7f3k"}),
        None,
    ));
    e.push(ev(21, "approval.requested", json!({
        "approval_id": id.to_string(), "kind": "policy",
        "subject": {"title": "Shell-Kommando bestätigen", "summary": "git push -u origin beton/rate-limiter-7f3k",
            "tool": {"name": "Bash", "kind": "shell", "args": {"command": "git push -u origin beton/rate-limiter-7f3k"}},
            "reasons": [{"rule_id": "git-push-fragen", "scope": "project", "set": "shop-frontend", "reason": "Pushes verlassen deinen Rechner. Die Projekt-Policy verlangt dafür deine Freigabe.", "fingerprint": "x"}],
            "call_id": "c5"},
        "options": ["approve", "approve_session", "deny"], "expires_at": "2026-10-17T08:11:00Z", "on_timeout": "deny"})));
    e
}

fn sessions(other_waiting: bool) -> Vec<Value> {
    let s = |id: &str, title: &str, status: &str, harness: &str, last: &str| json!({"id": id, "title": title, "status": status, "kind": "main", "harness": harness, "archived": false, "last_activity_at": last});
    vec![
        s(
            SESSION,
            "Rate-Limiter Login",
            "waiting_approval",
            "claude",
            "2026-10-17T07:41:06Z",
        ),
        s(
            OTHER,
            "Review: Rate-Limiter",
            if other_waiting {
                "waiting_approval"
            } else {
                "running"
            },
            "codex",
            "2026-10-17T07:41:05Z",
        ),
        s(
            "ses_01JB8Y2D0M3K4J5H6G7F8E9D0E",
            "Checkout a11y",
            "idle",
            "claude",
            "2026-10-17T07:41:04Z",
        ),
        s(
            "ses_01JB8Y2D0M3K4J5H6G7F8E9D0F",
            "Dependency-Update",
            "running",
            "claude",
            "2026-10-17T07:41:03Z",
        ),
        s(
            "ses_01JB8Y2D0M3K4J5H6G7F8E9D0G",
            "Flaky payment_spec",
            "failed",
            "claude",
            "2026-10-17T07:41:02Z",
        ),
        s(
            "ses_01JB8Y2D0M3K4J5H6G7F8E9D0H",
            "Terraform-Plan",
            "stopped",
            "acp:gemini",
            "2026-10-17T07:41:01Z",
        ),
    ]
}

fn app_with(size: (u16, u16), events: Vec<Event>, other_waiting: bool) -> App {
    let opts = AppOptions {
        server_label: "local".into(),
        offset: UtcOffset::from_hms(2, 0, 0).unwrap(),
        windows: None,
    };
    let mut app = App::new(opts, size);
    app.update(Msg::Tick(at(0, 30)));
    app.update(Msg::Sessions(sessions(other_waiting)));
    app.open_session(sid());
    app.update(Msg::Events {
        session: sid(),
        events,
    });
    app.update(Msg::Live { session: sid() });
    app.update(Msg::Changes {
        session: sid(),
        items: vec![
            json!({"path": "src/routes/auth.ts", "status": "modified", "additions": 2, "deletions": 1}),
            json!({"path": "src/middleware/rate-limit.ts", "status": "added", "additions": 12, "deletions": 0}),
        ],
    });
    if other_waiting {
        app.update(Msg::Inbox(vec![json!({"session_id": OTHER, "session_title": "Review: Rate-Limiter", "kind": "approval", "status": "open", "created_at": "2026-10-17T07:40:00Z"})]));
    }
    app
}

fn draw(app: &App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(app.size.0, app.size.1)).unwrap();
    t.draw(|f| render(app, f)).unwrap();
    t.backend().buffer().clone()
}

fn snapshot(name: &str, buf: &Buffer) -> String {
    let text = buffer_lines(buf).join("\n") + "\n";
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.txt"));
    if std::env::var_os("BETON_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Snapshot {name} fehlt (BETON_BLESS=1)"));
    assert_eq!(text, expected, "Snapshot {name} weicht ab:\n{text}");
    text
}

fn cell_fg(buf: &Buffer, needle: &str) -> ratatui::style::Color {
    let lines = buffer_lines(buf);
    for (y, line) in lines.iter().enumerate() {
        if let Some(byte) = line.find(needle) {
            let x = line[..byte].chars().count() as u16;
            return buf.cell(Position::new(x, y as u16)).unwrap().fg;
        }
    }
    panic!("{needle} nicht gefunden");
}

#[test]
fn tui_001_session_view_while_the_agent_streams() {
    let app = app_with((140, 36), history(true), false);
    let buf = draw(&app);
    let text = snapshot("session-streaming", &buf);
    assert!(text.contains("Sessions"));
    assert!(text.contains("Session: Rate-Limiter für die Login-API (claude · claude-opus-5-5)"));
    assert!(text.contains("● läuft"));
    assert!(text.contains("Queue: 1"));
    assert!(text.contains("Änderungen (2)"));
    assert!(text.contains("Ich schreibe noch den Test für den Header █"));
    assert_eq!(cell_fg(&buf, "Claude Code ▸"), theme::VOICE_CLAUDE);
}

#[test]
fn use_008_ac3_status_line_shows_the_context_level() {
    let app = app_with((140, 36), history(true), false);
    let buf = draw(&app);
    let status = &buffer_lines(&buf)[34];
    assert!(status.contains("Kontext 36 %"), "{status}");
    // 72 % gemeldet → Stufe „wird knapp“.
    let mut events = history(true);
    events.push(ev(
        30,
        "context.usage",
        json!({"used_tokens": 144_000, "window_tokens": 200_000, "source": "harness"}),
    ));
    let app = app_with((140, 36), events, false);
    let buf = draw(&app);
    assert!(buffer_lines(&buf)[34].contains("Kontext 72 % · wird knapp"));
    assert_eq!(cell_fg(&buf, "Kontext 72 %"), theme::WARN);
    // Ohne Fenster nur die Tokenzahl.
    let mut events = history(true);
    events.push(ev(
        30,
        "context.usage",
        json!({"used_tokens": 150_000, "window_tokens": 0, "source": "estimated"}),
    ));
    let app = app_with((140, 36), events, false);
    assert!(buffer_lines(&draw(&app))[34].contains("150 k Tokens"));
}

#[test]
fn tui_004_approval_card_with_keys() {
    let mut events = history(false);
    events.extend(approval_events(ApprovalId::new()));
    let app = app_with((140, 36), events, false);
    let buf = draw(&app);
    let text = snapshot("session-approval", &buf);
    assert!(text.contains("Freigabe nötig ─ wartet auf dich"));
    assert!(text.contains("git push -u origin beton/rate-limiter-7f3k"));
    assert!(text.contains("git-push-fragen (Projekt shop-frontend)"));
    assert!(text.contains(" y  erlauben"));
    assert!(text.contains(" s  für diese Session erlauben"));
    assert!(text.contains("y/n/s/d entscheiden · Esc zurück zum Verlauf"));
    assert!(text.contains("◆ 1 offen ^G a"));
    assert_eq!(cell_fg(&buf, "Freigabe nötig"), theme::SIGNAL);
}

#[test]
fn tui_004_ac2_approval_in_another_session_appears_in_the_status_line() {
    let app = app_with((140, 36), history(true), true);
    let text = snapshot("session-other-approval", &draw(&app));
    assert!(text.contains("◆ 1 Freigabe in „Review: Rate-Limiter“ · ^G a springt hin"));
}

#[test]
fn tui_001_ac3_below_80x24_shows_a_hint() {
    let app = app_with((64, 18), history(true), false);
    let text = snapshot("too-small", &draw(&app));
    assert!(text.contains("Terminal zu klein: 64 × 18"));
    assert!(text.contains("beton tui braucht mindestens 80 × 24 Zeichen."));
    // 79 × 24 ebenfalls zu klein, 80 × 23 auch.
    for size in [(79, 24), (80, 23)] {
        let app = app_with(size, history(true), false);
        assert!(
            buffer_lines(&draw(&app))
                .join("\n")
                .contains("Terminal zu klein")
        );
    }
}

#[test]
fn tui_001_ac3_80x24_keeps_a_working_layout() {
    let mut events = history(false);
    events.extend(approval_events(ApprovalId::new()));
    let app = app_with((80, 24), events, true);
    let text = snapshot("session-80x24", &draw(&app));
    assert!(!text.contains("Terminal zu klein"));
    assert!(text.contains("Session: Rate-Limiter"));
    assert!(text.contains("Freigabe nötig"));
    assert!(text.contains(" y  erlauben"));
    assert!(text.contains("Nachricht"));
    // Jede Zeile passt in 80 Spalten.
    assert!(
        text.lines()
            .all(|l| unicode_width::UnicodeWidthStr::width(l) <= 80)
    );
}

fn vite() -> Vec<u8> {
    b"\x1b[32mVITE v7.1.4\x1b[m  \x1b[2mready in 412 ms\x1b[m\r\n\r\n  \xe2\x9e\x9c  Local:   \x1b[36mhttp://localhost:5173/\x1b[m\r\n09:41:02 \x1b[36m[vite]\x1b[m hmr update /src/routes/auth.ts\r\n".to_vec()
}

fn htop() -> Vec<u8> {
    b"\x1b[?1049h\x1b[H\x1b[2J  1 [\x1b[32m||||||||||\x1b[m       42.1%]   Tasks: 312\r\n  2 [\x1b[32m|||||\x1b[m            21.4%]   Load average: 2.31\r\n\r\n\x1b[30;42m  PID USER   CPU% Command          \x1b[m\r\n48121 ingo   12.4 node vite\r\n".to_vec()
}

fn term(id: &str, name: &str, cmd: &str) -> SdkTerminal {
    SdkTerminal {
        terminal_id: id.into(),
        channel_id: 1,
        name: name.into(),
        kind: "user".into(),
        cmd: cmd.into(),
        cols: 96,
        rows: 14,
    }
}

fn panes_app(zoom: bool) -> App {
    let mut app = app_with((140, 36), history(true), false);
    let mut layout = Layout::default();
    layout.split(
        Dir::Columns,
        PaneRef::Terminal {
            terminal_id: "term_1".into(),
        },
    );
    layout.split(
        Dir::Rows,
        PaneRef::Terminal {
            terminal_id: "term_2".into(),
        },
    );
    layout.focus = PaneRef::Terminal {
        terminal_id: "term_1".into(),
    };
    layout.zoom = zoom;
    if zoom {
        layout.focus = PaneRef::Terminal {
            terminal_id: "term_2".into(),
        };
    }
    let fx = app.update(Msg::Restored {
        session: sid(),
        layout: Some(layout),
        terminals: vec![
            term("term_1", "dev-server", "pnpm dev"),
            term("term_2", "htop", "htop"),
        ],
    });
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Attach { terminal_id, .. } if terminal_id == "term_2"))
    );
    // PTY-Größe folgt der Fläche des Panes (TUI-002 AC1).
    let resized = fx.iter().any(|e| matches!(e, Effect::ResizeTerminal { terminal_id, cols: 68, rows: 14, .. } if terminal_id == "term_1"));
    assert_eq!(resized, !zoom, "nur sichtbare Panes passen ihr PTY an");
    for (id, bytes) in [("term_1", vite()), ("term_2", htop())] {
        app.update(Msg::Pane {
            terminal_id: id.into(),
            event: ChannelEvent::Opened {
                access: beton_proto::channel::ChannelAccess::Drive,
            },
        });
        let mut key = b"\x1bc".to_vec();
        key.extend(bytes);
        app.update(Msg::Pane {
            terminal_id: id.into(),
            event: ChannelEvent::Data {
                bytes: key,
                keyframe: true,
            },
        });
    }
    app
}

#[test]
fn tui_002_panes_split_without_tmux() {
    let app = panes_app(false);
    let buf = draw(&app);
    let text = snapshot("panes-split", &buf);
    assert!(text.contains("Rate-Limiter für die Login-API · 3 Panes"));
    assert!(text.contains("0 Agent · Claude Code"));
    assert!(text.contains("1 dev-server · pnpm dev"));
    assert!(text.contains("2 htop"));
    assert!(text.contains("PTY 68×14") && text.contains("PTY 68×15"));
    assert!(text.contains("VITE v7.1.4"));
    assert!(text.contains("PID USER"));
    assert!(text.contains(
        "✓ Wieder verbunden. 3 Panes wiederhergestellt; dev-server und htop liefen im Runner weiter"
    ));
}

#[test]
fn tui_002_prefix_overlay_and_zoom() {
    let mut app = panes_app(false);
    app.banner = None;
    app.input(Input::Key(KeyEvent::new(
        KeyCode::Char('g'),
        KeyModifiers::CONTROL,
    )));
    let text = snapshot("panes-prefix", &draw(&app));
    assert!(text.contains("^G gedrückt – nächste Taste"));
    assert!(text.contains("|   senkrecht teilen"));
    assert!(text.contains("^G  ^G an den Pane senden"));
    assert!(text.contains("Panes laufen im Runner (beton-pty) – kein tmux"));
    assert!(text.contains("Layout wird pro Session gespeichert"));
    let mut app = panes_app(true);
    app.banner = None;
    let text = snapshot("panes-zoom", &draw(&app));
    assert!(text.contains("[Z] 2 htop"));
    assert!(text.contains("^G z zurück"));
    assert!(!text.contains("dev-server"));
}

#[test]
fn tui_005_windows_title_names_the_terminal_and_conpty() {
    let mut app = panes_app(false);
    app.banner = None;
    app.opts.windows = Some(WindowsHost::WindowsTerminal);
    let text = snapshot("windows-panes", &draw(&app));
    assert!(text.contains("· Windows"));
    assert!(text.contains("Windows Terminal · ConPTY"));
    assert!(text.contains("crossterm · Panes über ConPTY"));
}
