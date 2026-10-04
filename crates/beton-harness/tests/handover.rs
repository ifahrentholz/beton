//! Präambel-Rendering des Handover-Kontexts (HAR-018): deterministisch (Golden-Test) und
//! im Budget. Erwartungen neu schreiben: `BETON_BLESS=1 cargo test -p beton-harness --test
//! handover`.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use beton_core::event::{
    EventPayload, FsChange, FsChangeKind, FsChanged, MessageCompleted, MessageRole,
    ReasoningCompleted, ToolCallCompleted, ToolCallRequested, ToolSource, ToolStatus,
    TurnCompleted, TurnInterrupted, TurnStarted,
};
use beton_core::id::{PrincipalId, SessionId, TurnId, UserId};
use beton_harness::golden::BLESS_ENV;
use beton_harness::handover::{self, HandoverContext, HistoryEvent, Source};
use serde_json::{Value, json};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/handover")
}

/// Baut den Verlauf aus einfachen Bausteinen; `seq` zählt mit.
#[derive(Default)]
struct Log {
    events: Vec<HistoryEvent>,
    calls: u32,
}

impl Log {
    fn push(&mut self, payload: EventPayload) {
        let seq = self.events.len() as u64 + 2;
        self.events.push(HistoryEvent {
            seq,
            payload,
            turn_id: None,
            raw: None,
        });
    }

    fn user(&mut self, text: &str) {
        self.push(EventPayload::MessageCompleted(MessageCompleted {
            message_id: "u".into(),
            role: MessageRole::User,
            content: vec![json!({"type": "text", "text": text})],
            author: Some(PrincipalId::User(UserId::LOCAL)),
        }));
        self.push(EventPayload::TurnStarted(TurnStarted {
            turn_id: TurnId::from_ulid(ulid::Ulid(1)),
            input_id: None,
            author: PrincipalId::User(UserId::LOCAL),
        }));
    }

    fn agent(&mut self, text: &str) {
        self.push(EventPayload::MessageCompleted(MessageCompleted {
            message_id: "a".into(),
            role: MessageRole::Assistant,
            content: vec![json!({"type": "text", "text": text})],
            author: None,
        }));
    }

    fn reasoning(&mut self, text: &str) {
        self.push(EventPayload::ReasoningCompleted(ReasoningCompleted {
            message_id: "r".into(),
            summary: Some(text.into()),
            redacted: false,
        }));
    }

    fn tool(&mut self, tool: &str, args: Value, result: &str, status: ToolStatus) {
        self.calls += 1;
        let call_id = format!("call_{}", self.calls);
        self.push(EventPayload::ToolCallRequested(ToolCallRequested {
            call_id: call_id.clone(),
            tool: tool.into(),
            mcp_server: None,
            args,
            source: ToolSource::Harness,
        }));
        self.push(EventPayload::ToolCallCompleted(ToolCallCompleted {
            call_id,
            status,
            result: Some(Value::String(result.into())),
            result_ref: None,
            duration_ms: 3,
        }));
    }

    fn done(&mut self) {
        self.push(EventPayload::TurnCompleted(TurnCompleted {
            turn_id: TurnId::from_ulid(ulid::Ulid(1)),
            stop_reason: "end_turn".into(),
            usage_summary: json!({}),
        }));
    }

    fn context(&self) -> HandoverContext {
        HandoverContext::from_history(
            &self.events,
            Source {
                session: SessionId::from_ulid(ulid::Ulid(0x0123_4567_89ab_cdef)),
                title: Some("Rate-Limiter für die Login-API".into()),
                harness: "claude".into(),
                up_to_seq: self.events.last().map_or(1, |e| e.seq),
                worktree: Some(
                    "/home/dev/.beton/worktrees/shop-1a2b3c4d/beton-rate-limiter-7f3k".into(),
                ),
                branch: Some("beton/rate-limiter-7f3k".into()),
                agent_ref: None,
            },
        )
    }
}

fn sample() -> Log {
    let mut log = Log::default();
    log.user("Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP. Bitte mit Tests.");
    log.reasoning("Erst die Route lesen, dann eine Middleware bauen.");
    log.tool(
        "Read",
        json!({"file_path": "src/routes/auth.ts"}),
        "export function login() {}",
        ToolStatus::Ok,
    );
    log.tool(
        "TodoWrite",
        json!({"todos": [
            {"content": "Middleware bauen", "status": "completed"},
            {"content": "Tests schreiben", "status": "in_progress"},
            {"content": "Register-Route absichern", "status": "pending"},
        ]}),
        "ok",
        ToolStatus::Ok,
    );
    log.tool(
        "Edit",
        json!({"file_path": "src/middleware/rate-limit.ts"}),
        "ok",
        ToolStatus::Ok,
    );
    log.push(EventPayload::FsChanged(FsChanged {
        changes: vec![FsChange {
            path: "src/middleware/rate-limit.spec.ts".into(),
            change: FsChangeKind::Added,
            from: None,
        }],
        source: "watcher".into(),
    }));
    log.tool(
        "Bash",
        json!({"command": "pnpm vitest run auth"}),
        "10 passed",
        ToolStatus::Ok,
    );
    log.agent("Die Login-Route ist jetzt auf 5 Versuche pro Minute und IP begrenzt. Alle 10 Tests laufen.");
    log.done();
    log.user("Was passiert hinter einem Proxy?");
    log.tool(
        "Bash",
        json!({"command": "rm -rf node_modules"}),
        "nicht erlaubt",
        ToolStatus::Denied,
    );
    log.agent("Hinter einem Proxy ist `req.ip` die Adresse des Proxys; ich nutze `X-Forwarded-For` nur mit `trust proxy`.");
    log.push(EventPayload::TurnInterrupted(TurnInterrupted {
        turn_id: TurnId::from_ulid(ulid::Ulid(1)),
        by: PrincipalId::User(UserId::LOCAL),
    }));
    log
}

fn check(name: &str, actual: &str) {
    let path = golden_dir().join(name);
    if std::env::var(BLESS_ENV).is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (BETON_BLESS=1 erzeugt die Datei)", path.display()));
    assert_eq!(actual, expected, "{} weicht ab", path.display());
}

#[test]
fn har_018_ac2_preamble_rendering_is_deterministic() {
    let log = sample();
    let a = handover::render(&log.context(), "codex", 272_000, ".beton/handover/ses_x.md");
    let b = handover::render(
        &sample().context(),
        "codex",
        272_000,
        ".beton/handover/ses_x.md",
    );
    assert_eq!(a.document.as_bytes(), b.document.as_bytes(), "byte-gleich");
    assert_eq!(a.brief, b.brief);
    check("preamble.md", &a.document);
    check("brief.txt", &a.brief);
    let ctx = log.context();
    assert_eq!(
        ctx.open_todos,
        ["Tests schreiben", "Register-Route absichern"]
    );
    assert_eq!(
        ctx.changed_files,
        [
            "src/middleware/rate-limit.ts",
            "src/middleware/rate-limit.spec.ts"
        ]
    );
}

#[test]
fn har_018_ac3_one_megabyte_transcript_stays_within_budget() {
    let mut log = Log::default();
    let big = "x".repeat(6 * 1024);
    let mut turns = 0;
    while log
        .events
        .iter()
        .map(|e| serde_json::to_string(&e.payload).unwrap().len())
        .sum::<usize>()
        < 1024 * 1024
    {
        turns += 1;
        log.user(&format!("Frage {turns}: {}", "Kontext ".repeat(40)));
        log.reasoning(&"Überlegung ".repeat(80));
        log.tool(
            "Bash",
            json!({"command": format!("cargo test -p modul{turns}")}),
            &big,
            ToolStatus::Ok,
        );
        log.agent(&format!("Antwort {turns}: {}", "Erklärung ".repeat(60)));
        log.done();
    }
    // Die letzten drei Turns klein und eindeutig.
    for k in 1..=3 {
        log.user(&format!("Letzte Frage {k}"));
        log.tool(
            "Bash",
            json!({"command": format!("echo letzte-{k}")}),
            &format!("Ergebnis letzte-{k}"),
            ToolStatus::Ok,
        );
        log.agent(&format!("Letzte Antwort {k}"));
        log.done();
    }
    let ctx = log.context();
    let p = handover::render(&ctx, "codex", 128_000, ".beton/handover/ses_x.md");
    assert_eq!(p.budget, 51_200, "40 % von 128k");
    assert!(p.tokens <= p.budget, "{} > {}", p.tokens, p.budget);
    assert_eq!(p.turns, turns + 3);
    assert!(p.full_turns >= 3);
    for k in 1..=3 {
        for needle in [
            format!("**Nutzer:** Letzte Frage {k}"),
            format!("`Bash` echo letzte-{k} → ok"),
            format!("Ergebnis letzte-{k}"),
            format!("**Agent:** Letzte Antwort {k}"),
        ] {
            assert!(p.document.contains(&needle), "fehlt: {needle}");
        }
    }
    // Kürzungsregeln greifen sichtbar: gekürzte Turns oder ausgelassene mit Marker.
    assert!(p.document.contains("(gekürzt)") || p.document.contains("Turns ausgelassen"));
    if p.omitted_turns > 0 {
        assert!(
            p.document
                .contains(&format!("[… {} Turns ausgelassen …]", p.omitted_turns))
        );
    }
}

#[test]
fn har_018_rules_apply_in_order() {
    // Kleines Budget: Regel 1 (Tool-Results kürzen) reicht für ältere Turns.
    let mut log = Log::default();
    for k in 1..=5 {
        log.user(&format!("Frage {k}"));
        log.reasoning("Gedanke");
        log.tool(
            "Bash",
            json!({"command": "ls"}),
            &"y".repeat(8 * 1024),
            ToolStatus::Ok,
        );
        log.agent("ok");
        log.done();
    }
    let ctx = log.context();
    let full = handover::render(&ctx, "codex", 10_000_000, "d.md");
    assert_eq!(full.full_turns, 5);
    let budget = full.tokens - 2_000;
    let p = handover::render(&ctx, "codex", budget * 100 / 40 + 1, "d.md");
    assert!(p.tokens <= p.budget);
    assert!(p.document.contains("Bytes gekürzt"), "Regel 1");
    assert!(
        p.document.contains("**Überlegung:** Gedanke"),
        "Regel 2 noch nicht nötig"
    );
    assert_eq!(p.omitted_turns, 0);
}
