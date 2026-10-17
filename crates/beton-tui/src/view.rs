//! Darstellung nach dem abgenommenen Prototyp (`design/prototype/src/screens/tui/`, Screens
//! `tui-session`, `tui-panes`, `tui-windows`). Reine Funktion des Zustands: dieselbe App ergibt
//! denselben Buffer (Snapshot-Tests gegen das Test-Backend von ratatui).

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget as _};
use unicode_width::UnicodeWidthStr as _;

use crate::app::{App, Focus, MIN_COLS, MIN_ROWS, Overlay, PaneStatus, panes_area};
use crate::context::{ContextLevel, status_text};
use crate::layout::PaneRef;
use crate::model::{
    ApprovalCard, Glyph, Item, SessionModel, Tone, ToolItem, ToolState, Voice, arg_summary,
    edit_lines, harness_name, seconds, tool_label,
};
use crate::text::{line_width, markdown, pad, truncate, wrap};
use crate::{theme, vt};

/// Tastenleiste der Session-Ansicht.
pub const KEYS_MAIN: &[(&str, &str)] = &[
    ("^G", "Prefix"),
    ("Tab", "Fokus"),
    ("↵", "Senden"),
    ("Esc", "Unterbrechen"),
    ("^G s", "Session"),
    ("^G a", "Freigaben"),
    ("^G ?", "Hilfe"),
    ("^G q", "Beenden"),
];

/// Tastenleiste der Pane-Ansicht.
pub const KEYS_PANES: &[(&str, &str)] = &[
    ("^G |", "teilen"),
    ("^G -", "teilen"),
    ("^G o", "nächster"),
    ("^G z", "zoomen"),
    ("^G c", "Terminal"),
    ("^G d", "trennen"),
    ("^G ?", "Hilfe"),
];

/// Liste nach `^G` (Prototyp `tui-panes`, Zustand `prefix`).
pub const PREFIX_KEYS: &[(&str, &str)] = &[
    ("|", "senkrecht teilen"),
    ("-", "waagerecht teilen"),
    ("o", "nächster Pane"),
    ("z", "Pane zoomen"),
    ("c", "neues Terminal"),
    ("x", "Pane schließen"),
    ("d", "trennen (alles läuft weiter)"),
    ("s", "Session wechseln"),
    ("a", "zur nächsten Freigabe"),
    ("^G", "^G an den Pane senden"),
];

/// Zeichnet die ganze TUI.
pub fn render(app: &App, f: &mut Frame<'_>) {
    let area = f.area();
    if area.width < MIN_COLS || area.height < MIN_ROWS {
        too_small(area, f.buffer_mut());
        return;
    }
    let cursor = if app.layout.is_single() {
        session_view(app, area, f.buffer_mut())
    } else {
        panes_view(app, area, f.buffer_mut())
    };
    let main = Rect::new(0, 1, area.width, area.height.saturating_sub(3));
    let cursor = match app.overlay {
        Overlay::Prefix => {
            prefix_overlay(main, f.buffer_mut(), "^G gedrückt – nächste Taste");
            None
        }
        Overlay::Help => {
            prefix_overlay(main, f.buffer_mut(), "Hilfe ─ erst ^G, dann");
            None
        }
        Overlay::Picker { selected } => {
            picker(app, main, f.buffer_mut(), selected);
            None
        }
        Overlay::None => cursor,
    };
    if let Some(p) = cursor {
        f.set_cursor_position(p);
    }
}

/// Hinweis statt kaputtem Layout unter 80 × 24 (TUI-001 AC3).
fn too_small(area: Rect, buf: &mut Buffer) {
    let w = 52.min(area.width);
    let h = 5.min(area.height);
    let rect = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    let block = Block::bordered().border_style(Style::default().fg(theme::BORDER));
    let inner = block.inner(rect);
    block.render(rect, buf);
    let lines = [
        Line::styled(
            format!("Terminal zu klein: {} × {}", area.width, area.height),
            theme::bold(),
        ),
        Line::raw(format!(
            "beton tui braucht mindestens {MIN_COLS} × {MIN_ROWS} Zeichen."
        )),
        Line::styled(
            "Vergrößere das Fenster – die Session läuft weiter.",
            theme::dim(),
        ),
    ];
    draw_lines(buf, inner.inner_x(1), &lines);
}

trait RectExt {
    fn inner_x(self, n: u16) -> Rect;
}

impl RectExt for Rect {
    fn inner_x(self, n: u16) -> Rect {
        Rect {
            x: self.x + n.min(self.width),
            width: self.width.saturating_sub(2 * n),
            ..self
        }
    }
}

fn draw_lines(buf: &mut Buffer, area: Rect, lines: &[Line<'_>]) {
    for (i, line) in lines.iter().enumerate().take(usize::from(area.height)) {
        buf.set_line(area.x, area.y + i as u16, line, area.width);
    }
}

/// Kopfzeile: `beton tui`, links Kontext, rechts Uhrzeit bzw. Zustand.
fn title_bar(buf: &mut Buffer, area: Rect, left: &str, right: &str) {
    let row = Rect::new(area.x, area.y, area.width, 1);
    buf.set_style(row, Style::default().bg(theme::TITLE_BG));
    let line = Line::from(vec![
        Span::styled(" beton tui", theme::bold()),
        Span::raw("  "),
        Span::styled(left.to_owned(), theme::dim()),
    ]);
    buf.set_line(row.x, row.y, &line, row.width);
    let rw = right.width() as u16;
    if rw + 2 < row.width {
        buf.set_string(row.x + row.width - rw - 1, row.y, right, theme::dim());
    }
}

/// Untere Tastenleiste: Taste invertiert, Bedeutung daneben.
fn key_bar(buf: &mut Buffer, area: Rect, keys: &[(&str, &str)]) {
    let mut spans = vec![Span::raw(" ")];
    let mut used = 1;
    for (k, label) in keys {
        let w = k.width() + label.width() + 4;
        if used + w > usize::from(area.width) {
            break;
        }
        used += w;
        spans.push(Span::styled(format!(" {k} "), theme::key()));
        spans.push(Span::styled(format!(" {label}"), theme::dim()));
        spans.push(Span::raw(" "));
    }
    buf.set_line(area.x, area.y, &Line::from(spans), area.width);
}

fn frame_block<'a>(title: Line<'a>, right: Option<Line<'a>>, focused: bool) -> Block<'a> {
    let mut b = Block::bordered()
        .border_style(Style::default().fg(if focused { theme::FG } else { theme::BORDER }))
        .title(title);
    if let Some(r) = right {
        b = b.title_top(r.right_aligned());
    }
    b
}

fn title_style(focused: bool) -> Style {
    if focused { theme::bold() } else { theme::dim() }
}

/// Projektname: letzter Teil des Arbeitsverzeichnisses der Session.
fn project_label(model: Option<&SessionModel>) -> String {
    model
        .and_then(|m| m.cwd.as_deref())
        .map(|c| {
            c.trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(c)
                .to_owned()
        })
        .unwrap_or_default()
}

fn clock(app: &App) -> String {
    let t = app.now.to_offset(app.opts.offset);
    format!("{:02}:{:02}", t.hour(), t.minute())
}

/// Session-Ansicht (Prototyp `tui-session`). Liefert die Cursor-Position.
fn session_view(app: &App, area: Rect, buf: &mut Buffer) -> Option<Position> {
    let model = app.session.as_ref();
    let mut left = app.opts.server_label.clone();
    let project = project_label(model);
    if !project.is_empty() {
        left.push_str(" · ");
        left.push_str(&project);
    }
    if app.opts.windows.is_some() {
        left.push_str(" · Windows");
    }
    let right = match app.opts.windows {
        Some(w) => w.label().to_owned(),
        None => clock(app),
    };
    title_bar(buf, area, &left, &right);
    let main_h = area.height.saturating_sub(6);
    let main = Rect::new(area.x, area.y + 1, area.width, main_h);
    let show_list = area.width >= 100;
    let show_rail = area.width >= 120;
    let list_w = if show_list { 27 } else { 0 };
    let rail_w = if show_rail { 32 } else { 0 };
    if show_list {
        session_list(app, Rect::new(main.x, main.y, list_w, main.height), buf);
    }
    let chat = Rect::new(
        main.x + list_w,
        main.y,
        main.width - list_w - rail_w,
        main.height,
    );
    let approval = app.visible_approval();
    chat_block(
        app,
        chat,
        buf,
        approval.is_none() && app.focus == Focus::Chat,
    );
    if show_rail {
        rail(
            app,
            Rect::new(chat.x + chat.width, main.y, rail_w, main.height),
            buf,
        );
    }
    let mut cursor = None;
    if let Some(card) = approval {
        cursor = approval_overlay(app, card, chat, buf);
    }
    let composer = Rect::new(area.x, main.y + main.height, area.width, 3);
    let c = composer_block(
        app,
        composer,
        buf,
        approval.is_none() && app.focus == Focus::Composer,
    );
    if approval.is_none() {
        cursor = c;
    }
    status_line(
        app,
        Rect::new(area.x, composer.y + 3, area.width, 1),
        buf,
        approval.is_some(),
    );
    key_bar(
        buf,
        Rect::new(area.x, area.y + area.height - 1, area.width, 1),
        KEYS_MAIN,
    );
    cursor
}

fn session_list(app: &App, area: Rect, buf: &mut Buffer) {
    let focused = app.focus == Focus::Sessions && app.visible_approval().is_none();
    let block = frame_block(
        Line::styled(" Sessions ", title_style(focused)),
        None,
        focused,
    );
    let inner = block.inner(area).inner_x(1);
    block.render(area, buf);
    let mut lines = Vec::new();
    let project = project_label(app.session.as_ref());
    lines.push(Line::styled(
        if project.is_empty() {
            "Sessions".into()
        } else {
            project
        },
        theme::dim(),
    ));
    let current = app.session_id().map(|s| s.to_string());
    let width = usize::from(inner.width);
    let legend = 3;
    let max_items = usize::from(inner.height).saturating_sub(1 + legend).max(1);
    let start = app
        .list_selected
        .saturating_sub(max_items.saturating_sub(1));
    for (i, s) in app.sessions.iter().enumerate().skip(start).take(max_items) {
        let active = current.as_deref() == Some(s.id.as_str());
        let waiting = app.inbox.iter().any(|e| e.session_id == s.id);
        let glyph = if waiting {
            Glyph::Waiting
        } else {
            Glyph::of_status(&s.status)
        };
        let title = if s.title.is_empty() {
            s.id.clone()
        } else {
            s.title.clone()
        };
        let name_w = width.saturating_sub(4);
        let mut style = Style::default();
        if active {
            style = style.add_modifier(Modifier::BOLD).bg(theme::SELECTED_BG);
        }
        if focused && i == app.list_selected {
            style = style
                .bg(theme::SELECTED_BG)
                .add_modifier(Modifier::UNDERLINED);
        }
        lines.push(Line::from(vec![
            Span::styled(if active { "> " } else { "  " }, style),
            Span::styled(pad(&title, name_w), style),
            Span::styled(" ", style),
            Span::styled(glyph.symbol(), theme::glyph(glyph).patch(style)),
        ]));
    }
    if app.sessions.is_empty() {
        lines.push(Line::styled("keine Sessions", theme::dim()));
    }
    draw_lines(buf, inner, &lines);
    if inner.height as usize > lines.len() + legend {
        let y = inner.y + inner.height - legend as u16;
        let legend_lines = [
            Line::styled("─".repeat(width.min(21)), theme::dim()),
            Line::styled("◆ wartet  ● läuft", theme::dim()),
            Line::styled("○ bereit  ✕ Fehler", theme::dim()),
        ];
        draw_lines(
            buf,
            Rect::new(inner.x, y, inner.width, legend as u16),
            &legend_lines,
        );
    }
}

fn chat_title(model: Option<&SessionModel>) -> Line<'static> {
    let Some(m) = model else {
        return Line::styled(" Session ", theme::bold());
    };
    let title = if m.title.is_empty() {
        m.id.to_string()
    } else {
        m.title.clone()
    };
    let mut spans = vec![
        Span::styled(format!(" Session: {title} ("), theme::bold()),
        Span::styled(
            if m.harness.is_empty() {
                "…".to_owned()
            } else {
                m.harness.clone()
            },
            theme::voice(Voice::of(&m.harness)),
        ),
    ];
    if let Some(model) = &m.model {
        spans.push(Span::styled(format!(" · {model}"), theme::bold()));
    }
    spans.push(Span::styled(") ", theme::bold()));
    Line::from(spans)
}

fn status_mark(m: &SessionModel) -> Line<'static> {
    let g = m.glyph();
    Line::from(Span::styled(
        format!(" {} {} ", g.symbol(), g.label()),
        if g == Glyph::Waiting {
            theme::signal()
        } else {
            theme::dim()
        },
    ))
}

fn chat_block(app: &App, area: Rect, buf: &mut Buffer, focused: bool) {
    let model = app.session.as_ref();
    let block = frame_block(chat_title(model), model.map(status_mark), true);
    let inner = block.inner(area).inner_x(1);
    block.render(area, buf);
    let lines = chat_lines(app, inner.width, inner.height, focused);
    draw_lines(buf, inner, &lines);
}

/// Zeilen des Verlaufs, unten bündig, mit `app.scroll` Zeilen Abstand zum Ende.
pub fn chat_lines(app: &App, width: u16, height: u16, selecting: bool) -> Vec<Line<'static>> {
    let Some(m) = app.session.as_ref() else {
        return vec![Line::styled(
            "Keine Session. Starte eine mit `beton run` oder wähle eine mit ^G s.",
            theme::dim(),
        )];
    };
    let mut blocks: Vec<Vec<Line<'static>>> = Vec::new();
    let need = usize::from(height) + app.scroll;
    let mut total = 0;
    // Laufende Nachrichten stehen immer am Ende (wie in der Web-UI).
    for (_, text) in m.streaming.iter().rev() {
        let lines = message_lines(
            &harness_name(&m.harness),
            Voice::of(&m.harness),
            text,
            true,
            width,
        );
        total += lines.len() + 1;
        blocks.push(with_gap(lines));
    }
    for (_, text) in m.reasoning.iter().rev() {
        let line = Line::from(vec![
            Span::styled("  ▸ Überlegt … ", theme::dim()),
            Span::styled(
                truncate(last_line(text), usize::from(width).saturating_sub(16)),
                theme::dim(),
            ),
        ]);
        total += 1;
        blocks.push(vec![line]);
    }
    for (index, item) in m.items.iter().enumerate().rev() {
        if total >= need {
            break;
        }
        let selected = selecting && app.selected == Some(index);
        let lines = item_lines(app, m, item, width, selected);
        if lines.is_empty() {
            continue;
        }
        total += lines.len();
        blocks.push(lines);
    }
    let mut lines: Vec<Line<'static>> = blocks.into_iter().rev().flatten().collect();
    if lines.first().is_some_and(|l| line_width(l) == 0) {
        lines.remove(0);
    }
    if m.items.is_empty() && m.streaming.is_empty() {
        return vec![Line::styled(
            if app.live {
                "Noch kein Verlauf. Schreib unten eine Nachricht."
            } else {
                "Lade Verlauf …"
            },
            theme::dim(),
        )];
    }
    let end = lines
        .len()
        .saturating_sub(app.scroll.min(lines.len().saturating_sub(1)));
    let start = end.saturating_sub(usize::from(height));
    lines[start..end].to_vec()
}

fn with_gap(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    lines.insert(0, Line::raw(""));
    lines
}

fn last_line(text: &str) -> &str {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
}

fn message_lines(
    label: &str,
    voice: Voice,
    text: &str,
    streaming: bool,
    width: u16,
) -> Vec<Line<'static>> {
    let mut md = markdown(text, Style::default());
    if md.is_empty() {
        md.push(Line::raw(""));
    }
    if streaming && let Some(last) = md.last_mut() {
        last.spans.push(Span::raw(" "));
        last.spans
            .push(Span::styled("█", Style::default().fg(theme::FG)));
    }
    let mut first = vec![Span::styled(format!("{label} ▸ "), theme::voice(voice))];
    first.append(&mut md[0].spans);
    md[0] = Line::from(first);
    md.iter().flat_map(|l| wrap(l, width, 0)).collect()
}

fn user_lines(text: &str, width: u16) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for (i, raw) in text.split('\n').enumerate() {
        let line = if i == 0 {
            Line::from(vec![
                Span::styled("Du ▸ ", theme::bold()),
                Span::raw(raw.to_owned()),
            ])
        } else {
            Line::raw(raw.to_owned())
        };
        out.extend(wrap(&line, width, 0));
    }
    out
}

fn item_lines(
    app: &App,
    m: &SessionModel,
    item: &Item,
    width: u16,
    selected: bool,
) -> Vec<Line<'static>> {
    match item {
        Item::User { text } => with_gap(user_lines(text, width)),
        Item::Assistant { text, .. } => with_gap(message_lines(
            &harness_name(&m.harness),
            Voice::of(&m.harness),
            text,
            false,
            width,
        )),
        Item::Reasoning { text } => vec![Line::from(vec![
            Span::styled("  ▸ Überlegung · ", theme::dim()),
            Span::styled(
                truncate(
                    text.lines().next().unwrap_or(""),
                    usize::from(width).saturating_sub(18),
                ),
                theme::dim(),
            ),
        ])],
        Item::Tool(t) => tool_lines(m, t, width, selected),
        Item::Approval(card) => approval_line(app, card, width),
        Item::Note { text, tone } => {
            let style = match tone {
                Tone::Ok => theme::ok(),
                Tone::Warn => theme::warn(),
                Tone::Plain => theme::dim(),
            };
            vec![Line::styled(format!("── {text} ──"), style)]
        }
        Item::Error { title, detail } => {
            let text = match detail {
                Some(d) => format!("✕ {title}: {d}"),
                None => format!("✕ {title}"),
            };
            wrap(&Line::styled(text, theme::deny()), width, 2)
        }
    }
}

/// Tool-Call als Zeile mit festen Spalten (Pfeil, Name, Ziel, Ergebnis), aufgeklappt mit Diff
/// bzw. Ausgabe (Prototyp `ToolLine`).
fn tool_lines(m: &SessionModel, t: &ToolItem, width: u16, selected: bool) -> Vec<Line<'static>> {
    let width = usize::from(width);
    let open = t.is_expanded();
    let waiting = t.state == ToolState::Requested && m.waiting_for_approval(&t.call_id);
    let (result, result_style) = if waiting {
        ("◆ wartet auf dich".to_owned(), theme::signal())
    } else {
        match t.state {
            ToolState::Requested => ("… angefragt".to_owned(), theme::dim()),
            ToolState::Running => ("● läuft".to_owned(), theme::ok()),
            ToolState::Completed => (
                format!("✓ {}", seconds(t.duration_ms.unwrap_or(0))),
                theme::ok(),
            ),
            ToolState::Failed => (
                format!("✕ {}", seconds(t.duration_ms.unwrap_or(0))),
                theme::deny(),
            ),
            ToolState::Denied => ("✕ abgelehnt".to_owned(), theme::deny()),
        }
    };
    let edits = if t.is_edit() {
        edit_lines(&t.args)
    } else {
        None
    };
    let mut target = arg_summary(&t.args);
    if let Some((del, add)) = &edits {
        target.push_str(&format!(" (+{} −{})", add.len(), del.len()));
    }
    let dim = !open && !waiting;
    let base = if dim { theme::dim() } else { Style::default() };
    let base = if selected {
        base.bg(theme::SELECTED_BG)
    } else {
        base
    };
    let result_w = 20.min(width / 3);
    let target_w = width.saturating_sub(4 + 12 + result_w + 1);
    let mut lines = vec![Line::from(vec![
        Span::styled(if open { "  ▾ " } else { "  ▸ " }, base),
        Span::styled(pad(&tool_label(&t.tool, t.kind.as_deref()), 12), base),
        Span::styled(pad(&target, target_w), base),
        Span::styled(" ", base),
        Span::styled(
            pad(&result, result_w),
            result_style.patch(if selected {
                Style::default().bg(theme::SELECTED_BG)
            } else {
                Style::default()
            }),
        ),
    ])];
    if !open {
        return lines;
    }
    if let Some((del, add)) = edits {
        let max = 12;
        for l in del.iter().take(max) {
            lines.push(Line::styled(
                truncate(&format!("      − {l}"), width),
                theme::deny(),
            ));
        }
        for l in add.iter().take(max) {
            lines.push(Line::styled(
                truncate(&format!("      + {l}"), width),
                theme::ok(),
            ));
        }
        return lines;
    }
    let output = if t.output.is_empty() {
        t.result
            .as_ref()
            .map(|r| match r {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default()
    } else {
        t.output.clone()
    };
    let out_lines: Vec<&str> = output.lines().collect();
    let skip = out_lines.len().saturating_sub(10);
    for l in &out_lines[skip..] {
        lines.push(Line::styled(
            truncate(&format!("      {l}"), width),
            theme::dim(),
        ));
    }
    lines
}

/// Vermerk einer entschiedenen Freigabe, z. B. „✓ Erlaubt mit y · Du, TUI · 09:41:12“.
fn approval_line(app: &App, card: &ApprovalCard, width: u16) -> Vec<Line<'static>> {
    use beton_core::event::{ApprovalDecision, ResolvedVia};
    let Some(r) = &card.resolution else {
        if card.call_id.is_none() {
            return vec![Line::styled(
                truncate(
                    &format!(
                        "  ◆ {}",
                        if card.title.is_empty() {
                            "Freigabe nötig"
                        } else {
                            &card.title
                        }
                    ),
                    usize::from(width),
                ),
                theme::signal(),
            )];
        }
        return Vec::new();
    };
    let key = app.decided.get(&card.approval_id);
    let with = key.map(|k| format!(" mit {k}")).unwrap_or_default();
    let who = if r.by_local_user {
        if key.is_some() {
            "Du, TUI".to_owned()
        } else {
            "Du".to_owned()
        }
    } else {
        match r.via {
            ResolvedVia::Timeout => "Zeit abgelaufen".to_owned(),
            ResolvedVia::Policy => "Policy".to_owned(),
            _ => "System".to_owned(),
        }
    };
    let t = r.at.as_offset_date_time().to_offset(app.opts.offset);
    let at = format!("{:02}:{:02}:{:02}", t.hour(), t.minute(), t.second());
    let (text, style) = match r.decision {
        ApprovalDecision::Allow if r.remember => {
            (format!("  ✓ Für diese Session erlaubt{with}"), theme::ok())
        }
        ApprovalDecision::Allow => (format!("  ✓ Erlaubt{with}"), theme::ok()),
        ApprovalDecision::Answer => ("  ✓ Beantwortet".to_owned(), theme::ok()),
        ApprovalDecision::Deny => (format!("  ✕ Abgelehnt{with}"), theme::deny()),
        ApprovalDecision::Abort => ("  ✕ Abgebrochen".to_owned(), theme::deny()),
    };
    vec![Line::from(vec![
        Span::styled(text, style),
        Span::styled(format!(" · {who} · {at}"), theme::dim()),
    ])]
}

fn scope_label(scope: &str, set: &str) -> String {
    let s = match scope {
        "org" => "Org",
        "team" => "Team",
        "user" => "User",
        "project" => "Projekt",
        "agent" => "Agent",
        other => other,
    };
    [s, set]
        .iter()
        .filter(|x| !x.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Freigabe als Karte über dem Verlauf (TUI-004, Prototyp `tui-session`, Zustand `approval`).
fn approval_overlay(
    app: &App,
    card: &ApprovalCard,
    chat: Rect,
    buf: &mut Buffer,
) -> Option<Position> {
    use beton_core::event::ApprovalKind;
    let width = chat.width.saturating_sub(4).max(20);
    let inner_w = width.saturating_sub(4);
    let label_w = 9usize;
    let value_w = usize::from(inner_w).saturating_sub(label_w + 1) as u16;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let row = |lines: &mut Vec<Line<'static>>, label: &str, value: String, style: Style| {
        let wrapped = wrap(&Line::styled(value, style), value_w, 0);
        for (i, w) in wrapped.into_iter().enumerate() {
            let mut spans = vec![Span::styled(
                pad(if i == 0 { label } else { "" }, label_w + 1),
                theme::dim(),
            )];
            spans.extend(w.spans);
            lines.push(Line::from(spans));
        }
    };
    let mut cursor = None;
    let title;
    if card.kind == ApprovalKind::Question {
        title = " Frage des Agents ─ wartet auf dich ";
        let q = card.question.clone().unwrap_or_default();
        for l in wrap(&Line::raw(q), inner_w, 0) {
            lines.push(l);
        }
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled("> ", theme::bold()),
            Span::raw(app.answer.clone()),
        ]));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled(" ↵ ", theme::key_signal()),
            Span::raw(" antworten   "),
            Span::styled(" Esc ", theme::key()),
            Span::raw(" zurück zum Verlauf"),
        ]));
    } else {
        title = " Freigabe nötig ─ wartet auf dich ";
        row(
            &mut lines,
            "Tool",
            tool_label(&card.tool, card.tool_kind.as_deref()),
            Style::default(),
        );
        let what = match card.tool_kind.as_deref() {
            Some("file_read" | "file_edit" | "file_write") => "Datei",
            _ => "Befehl",
        };
        if !card.summary.is_empty() {
            row(&mut lines, what, card.summary.clone(), theme::bold());
        }
        if let Some(r) = card.reasons.first() {
            let mut rule = r.rule_id.clone();
            let origin = scope_label(&r.scope, &r.set);
            if !origin.is_empty() {
                rule.push_str(&format!(" ({origin})"));
            }
            if card.reasons.len() > 1 {
                rule.push_str(&format!(" · +{} weitere", card.reasons.len() - 1));
            }
            row(&mut lines, "Regel", rule, Style::default());
            if !r.reason.is_empty() {
                row(&mut lines, "Grund", r.reason.clone(), Style::default());
            }
        } else if !card.title.is_empty() {
            row(&mut lines, "Grund", card.title.clone(), Style::default());
        }
        if app.details {
            let args = serde_json::to_string_pretty(&card.args).unwrap_or_default();
            for (i, l) in args.lines().take(10).enumerate() {
                row(
                    &mut lines,
                    if i == 0 { "Argumente" } else { "" },
                    l.to_owned(),
                    theme::code(),
                );
            }
            for r in card.reasons.iter().skip(1) {
                row(
                    &mut lines,
                    "Regel",
                    format!(
                        "{} ({}) {}",
                        r.rule_id,
                        scope_label(&r.scope, &r.set),
                        r.reason
                    ),
                    Style::default(),
                );
            }
            let left = (card.expires_at.as_offset_date_time() - app.now).whole_minutes();
            if left > 0 {
                row(
                    &mut lines,
                    "Ablauf",
                    format!("läuft in {left} Min. ab"),
                    theme::dim(),
                );
            }
        }
        lines.push(Line::raw(""));
        let details = if app.details { "weniger" } else { "Details" };
        let build = |session: &str, gap: &str| {
            let mut keys = vec![
                Span::styled(" y ", theme::key_signal()),
                Span::raw(format!(" erlauben{gap}")),
                Span::styled(" n ", theme::key()),
                Span::raw(format!(" ablehnen{gap}")),
            ];
            if card.allows_session() {
                keys.push(Span::styled(" s ", theme::key()));
                keys.push(Span::raw(format!(" {session}{gap}")));
            }
            keys.push(Span::styled(" d ", theme::key()));
            keys.push(Span::raw(format!(" {details}")));
            Line::from(keys)
        };
        let full = build("für diese Session erlauben", "  ");
        let keys = if line_width(&full) <= usize::from(inner_w) {
            full
        } else {
            // Schmales Terminal: kürzere Beschriftung, alle Tasten bleiben sichtbar.
            build("Session", "  ")
        };
        lines.push(keys);
    }
    let max_h = chat.height.saturating_sub(2);
    let height = (lines.len() as u16 + 2).min(max_h);
    let rect = Rect::new(
        chat.x + (chat.width - width) / 2,
        chat.y + chat.height - 1 - height,
        width,
        height,
    );
    // Die Zeilen unter der Karte ganz freimachen, damit kein Verlauf durchscheint.
    Clear.render(
        Rect::new(
            chat.x + 1,
            rect.y,
            chat.width.saturating_sub(2),
            rect.height,
        ),
        buf,
    );
    let block = Block::bordered()
        .border_type(BorderType::Thick)
        .border_style(theme::signal())
        .title(Line::styled(
            title,
            theme::signal().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect).inner_x(1);
    block.render(rect, buf);
    let skip = lines.len().saturating_sub(usize::from(inner.height));
    // Bei wenig Platz bleiben die Tasten sichtbar.
    let shown: Vec<Line<'static>> = if skip > 0 {
        let keys = lines.pop();
        let mut l: Vec<Line<'static>> = lines
            .into_iter()
            .take(usize::from(inner.height).saturating_sub(1))
            .collect();
        l.extend(keys);
        l
    } else {
        lines
    };
    draw_lines(buf, inner, &shown);
    if card.kind == ApprovalKind::Question {
        let y = inner.y + shown.len().saturating_sub(3) as u16;
        let x = inner.x + 2 + app.answer.width() as u16;
        if x < inner.x + inner.width {
            cursor = Some(Position::new(x, y));
        }
    }
    cursor
}

fn rail(app: &App, area: Rect, buf: &mut Buffer) {
    let block = frame_block(Line::styled(" Rail ", theme::dim()), None, false);
    let inner = block.inner(area).inner_x(1);
    block.render(area, buf);
    let w = usize::from(inner.width);
    let mut lines = vec![Line::styled(
        format!("Änderungen ({})", app.changes.len()),
        theme::bold(),
    )];
    let budget = usize::from(inner.height).saturating_sub(8).max(1);
    for c in app.changes.iter().take(budget) {
        let (letter, style) = match c.status.as_str() {
            "added" => ("A", theme::ok()),
            "deleted" => ("D", theme::deny()),
            "renamed" => ("R", theme::code()),
            _ => ("M", theme::code()),
        };
        let stat = match (c.additions, c.deletions) {
            (Some(a), Some(d)) if d > 0 => format!("+{a} −{d}"),
            (Some(a), _) => format!("+{a}"),
            (None, Some(d)) => format!("−{d}"),
            (None, None) => String::new(),
        };
        let path_w = w.saturating_sub(3 + stat.width());
        lines.push(Line::from(vec![
            Span::styled(letter, style),
            Span::raw(" "),
            Span::raw(truncate(&c.path, path_w)),
            Span::raw(" "),
            Span::styled(stat, theme::dim()),
        ]));
    }
    if app.changes.len() > budget {
        lines.push(Line::styled(
            format!("… {} weitere", app.changes.len() - budget),
            theme::dim(),
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("Terminals", theme::bold()));
    let terminals = app
        .session
        .as_ref()
        .map(|m| m.terminals.as_slice())
        .unwrap_or_default();
    if terminals.is_empty() {
        lines.push(Line::styled("keine", theme::dim()));
    }
    for (i, t) in terminals.iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled("● ", theme::ok()),
            Span::raw(truncate(
                &format!("{} {}", i + 1, t.name),
                w.saturating_sub(2),
            )),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("Freigaben", theme::bold()));
    let pending = app.session.as_ref().map_or(0, SessionModel::pending_count);
    if pending > 0 {
        lines.push(Line::from(vec![
            Span::styled("◆ ", theme::signal()),
            Span::styled(format!("{pending} offen"), theme::signal()),
            Span::styled(" ^G a", theme::dim()),
        ]));
    } else {
        lines.push(Line::styled("keine offen", theme::dim()));
    }
    draw_lines(buf, inner, &lines);
}

fn composer_block(app: &App, area: Rect, buf: &mut Buffer, focused: bool) -> Option<Position> {
    let block = frame_block(
        Line::styled(" Nachricht ", title_style(focused)),
        None,
        focused,
    );
    let inner = block.inner(area).inner_x(1);
    block.render(area, buf);
    composer_line(app, inner, buf, focused)
}

/// `> Text` mit Cursor; laufender Turn: Text gedämpft (wird eingereiht).
fn composer_line(app: &App, inner: Rect, buf: &mut Buffer, focused: bool) -> Option<Position> {
    let busy = app.session.as_ref().is_some_and(SessionModel::busy);
    let avail = usize::from(inner.width).saturating_sub(3);
    let chars: Vec<char> = app.composer.chars().collect();
    let start = app.cursor.saturating_sub(avail);
    let shown: String = chars.iter().skip(start).take(avail).collect();
    let style = if busy { theme::dim() } else { Style::default() };
    let line = Line::from(vec![
        Span::styled("> ", theme::bold()),
        Span::styled(shown, style),
    ]);
    buf.set_line(inner.x, inner.y, &line, inner.width);
    let before: String = chars[start..app.cursor.min(chars.len())].iter().collect();
    focused.then(|| Position::new(inner.x + 2 + before.width() as u16, inner.y))
}

/// Statuszeile: Queue, Kontext (USE-008 AC3), Abo, Freigaben anderer Sessions (TUI-004 AC2),
/// rechts der Hinweis.
fn status_line(app: &App, area: Rect, buf: &mut Buffer, approval: bool) {
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    let sep = |spans: &mut Vec<Span<'static>>| {
        if spans.len() > 1 {
            spans.push(Span::raw("  "));
        }
    };
    if let Some(m) = &app.session {
        if m.queue_len > 0 {
            sep(&mut spans);
            spans.push(Span::raw(format!("Queue: {}", m.queue_len)));
        }
        if let Some(c) = &m.context {
            let (text, level) = status_text(c);
            let style = match level {
                ContextLevel::Full => theme::deny().add_modifier(Modifier::BOLD),
                ContextLevel::Warn | ContextLevel::High => {
                    theme::warn().add_modifier(Modifier::BOLD)
                }
                ContextLevel::Ok | ContextLevel::Plain => Style::default(),
            };
            sep(&mut spans);
            spans.push(Span::styled(text, style));
        }
        if let Some(sub) = m
            .subscription
            .iter()
            .max_by(|a, b| a.used_pct.total_cmp(&b.used_pct))
        {
            sep(&mut spans);
            spans.push(Span::raw(format!(
                "{} {:.0} %",
                harness_name(&m.harness),
                sub.used_pct
            )));
        }
    }
    let others = app.other_approvals();
    if !others.is_empty() {
        sep(&mut spans);
        let text = if others.len() == 1 {
            format!(
                "◆ 1 Freigabe in „{}“ · ^G a springt hin",
                others[0].session_title
            )
        } else {
            format!(
                "◆ {} Freigaben in anderen Sessions · ^G a springt hin",
                others.len()
            )
        };
        spans.push(Span::styled(text, theme::signal()));
    }
    if let Some(flash) = &app.flash {
        sep(&mut spans);
        spans.push(Span::styled(flash.clone(), theme::warn()));
    }
    let hint = if approval {
        "y/n/s/d entscheiden · Esc zurück zum Verlauf"
    } else {
        "^G ? Hilfe"
    };
    buf.set_line(area.x, area.y, &Line::from(spans), area.width);
    let hw = hint.width() as u16;
    let used = line_width(&Line::from(buf_text(buf, area))) as u16;
    if hw + 2 + used < area.width {
        buf.set_string(area.x + area.width - hw - 1, area.y, hint, theme::dim());
    }
}

fn buf_text(buf: &Buffer, area: Rect) -> String {
    let mut s = String::new();
    for x in area.x..area.x + area.width {
        s.push_str(
            buf.cell(Position::new(x, area.y))
                .map_or(" ", |c| c.symbol()),
        );
    }
    s.trim_end().to_owned()
}

/// Pane-Ansicht (Prototyp `tui-panes`).
fn panes_view(app: &App, area: Rect, buf: &mut Buffer) -> Option<Position> {
    let title = app
        .session
        .as_ref()
        .map(|m| {
            if m.title.is_empty() {
                m.id.to_string()
            } else {
                m.title.clone()
            }
        })
        .unwrap_or_default();
    let n = app.layout.panes().len();
    let mut left = format!("{title} · {n} Panes");
    if app.opts.windows.is_some() {
        left.push_str(" · Windows");
    }
    let right = if app.layout.zoom {
        format!("[Z] {}", pane_name(app, &app.layout.focus))
    } else {
        match app.opts.windows {
            Some(w) => w.label().to_owned(),
            None => clock(app),
        }
    };
    title_bar(buf, area, &left, &right);
    let mut cursor = None;
    let rects = app.layout.rects(panes_area((area.width, area.height)));
    let mut agent_rect = None;
    for (pane, rect) in &rects {
        let focused = *pane == app.layout.focus;
        match pane {
            PaneRef::Agent => {
                agent_rect = Some(*rect);
                let c = agent_pane(app, *rect, buf, focused);
                if focused {
                    cursor = c;
                }
            }
            PaneRef::Terminal { terminal_id } => {
                let c = terminal_pane(app, terminal_id, *rect, buf, focused);
                if focused {
                    cursor = c;
                }
            }
        }
    }
    if let (Some(card), Some(rect)) = (app.visible_approval(), agent_rect)
        && app.layout.focus == PaneRef::Agent
    {
        cursor = approval_overlay(app, card, rect, buf).or(None);
    }
    let footer = Rect::new(area.x, area.y + area.height - 2, area.width, 1);
    let mut spans = vec![Span::raw(" ")];
    if let Some(b) = &app.banner {
        spans.push(Span::styled("✓ Wieder verbunden. ", theme::ok()));
        spans.push(Span::raw(b.clone()));
    } else if let Some(f) = &app.flash {
        spans.push(Span::styled(f.clone(), theme::warn()));
    } else if app.session.as_ref().is_some_and(|m| m.pending_count() > 0)
        && app.layout.focus != PaneRef::Agent
    {
        spans.push(Span::styled(
            "◆ Freigabe offen · ^G a zeigt sie",
            theme::signal(),
        ));
    } else if !app.other_approvals().is_empty() {
        spans.push(Span::styled(
            format!(
                "◆ {} Freigabe(n) in anderen Sessions · ^G a springt hin",
                app.other_approvals().len()
            ),
            theme::signal(),
        ));
    } else {
        spans.push(Span::raw("Panes laufen im Runner (beton-pty) – kein tmux"));
    }
    buf.set_line(footer.x, footer.y, &Line::from(spans), footer.width);
    let hint = match app.opts.windows {
        Some(_) => "crossterm · Panes über ConPTY",
        None => "Layout wird pro Session gespeichert",
    };
    let used = buf_text(buf, footer).width() as u16;
    let hw = hint.width() as u16;
    if used + hw + 3 < footer.width {
        buf.set_string(
            footer.x + footer.width - hw - 1,
            footer.y,
            hint,
            theme::dim(),
        );
    }
    key_bar(
        buf,
        Rect::new(area.x, area.y + area.height - 1, area.width, 1),
        KEYS_PANES,
    );
    cursor
}

fn pane_name(app: &App, pane: &PaneRef) -> String {
    let n = app.layout.number(pane);
    match pane {
        PaneRef::Agent => format!("{n} Agent"),
        PaneRef::Terminal { terminal_id } => {
            let name = app
                .panes
                .get(terminal_id)
                .map_or_else(|| terminal_id.clone(), |p| p.terminal.name.clone());
            format!("{n} {name}")
        }
    }
}

fn agent_pane(app: &App, rect: Rect, buf: &mut Buffer, focused: bool) -> Option<Position> {
    let m = app.session.as_ref();
    let harness = m.map(|m| m.harness.clone()).unwrap_or_default();
    let n = app.layout.number(&PaneRef::Agent);
    let zoom = if app.layout.zoom { "[Z] " } else { "" };
    let title = Line::from(vec![
        Span::styled(format!(" {zoom}{n} Agent · "), title_style(focused)),
        Span::styled(harness_name(&harness), theme::voice(Voice::of(&harness))),
        Span::raw(" "),
    ]);
    let right = m.map(status_mark);
    let block = frame_block(title, right, focused);
    let inner = block.inner(rect).inner_x(1);
    block.render(rect, buf);
    if inner.height < 2 {
        return None;
    }
    let chat = Rect {
        height: inner.height - 1,
        ..inner
    };
    let lines = chat_lines(app, chat.width, chat.height, false);
    draw_lines(buf, chat, &lines);
    let input = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
    composer_line(app, input, buf, focused && app.visible_approval().is_none())
}

fn terminal_pane(
    app: &App,
    id: &str,
    rect: Rect,
    buf: &mut Buffer,
    focused: bool,
) -> Option<Position> {
    let pane = app.panes.get(id);
    let n = app.layout.number(&PaneRef::Terminal {
        terminal_id: id.to_owned(),
    });
    let (name, cmd) = pane.map_or((id.to_owned(), String::new()), |p| {
        (p.terminal.name.clone(), p.terminal.cmd.clone())
    });
    let zoom = if app.layout.zoom { "[Z] " } else { "" };
    let mut title = format!(" {zoom}{n} {name}");
    if !cmd.is_empty() && cmd != name {
        title.push_str(&format!(" · {cmd}"));
    }
    title.push(' ');
    let right = match pane.map(|p| &p.status) {
        Some(PaneStatus::Reconnecting) => Line::styled(" verbinde neu … ", theme::signal()),
        _ if app.layout.zoom => Line::styled(" ^G z zurück ", theme::dim()),
        Some(_) if pane.is_some_and(|x| x.scroll > 0) => Line::styled(
            format!(" Scrollback −{} ", pane.map_or(0, |x| x.scroll)),
            theme::signal(),
        ),
        _ => Line::styled(
            format!(
                " PTY {}×{} ",
                pane.map_or(0, |p| p.terminal.cols),
                pane.map_or(0, |p| p.terminal.rows)
            ),
            theme::dim(),
        ),
    };
    let block = frame_block(
        Line::styled(
            truncate(&title, usize::from(rect.width).saturating_sub(16)),
            title_style(focused),
        ),
        Some(right),
        focused,
    );
    let inner = block.inner(rect);
    block.render(rect, buf);
    let p = pane?;
    if p.status == PaneStatus::Connecting && !p.restored {
        buf.set_string(inner.x + 1, inner.y, "verbinde …", theme::dim());
        return None;
    }
    let cursor = vt::render(&p.screen, inner, buf);
    if focused { cursor } else { None }
}

fn prefix_overlay(main: Rect, buf: &mut Buffer, title: &str) {
    let w = 46.min(main.width);
    let h = (PREFIX_KEYS.len() as u16 + 2).min(main.height);
    let rect = Rect::new(
        main.x + main.width - w - 2,
        main.y + main.height - h - 1,
        w,
        h,
    );
    Clear.render(rect, buf);
    let block = Block::bordered()
        .border_type(BorderType::Thick)
        .border_style(theme::signal())
        .title(Line::styled(
            format!(" {title} "),
            theme::signal().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect).inner_x(1);
    block.render(rect, buf);
    let lines: Vec<Line<'static>> = PREFIX_KEYS
        .iter()
        .map(|(k, l)| {
            Line::from(vec![
                Span::styled(pad(k, 4), theme::signal()),
                Span::raw((*l).to_owned()),
            ])
        })
        .collect();
    draw_lines(buf, inner, &lines);
}

fn picker(app: &App, main: Rect, buf: &mut Buffer, selected: usize) {
    let w = 56.min(main.width);
    let h = (app.sessions.len() as u16 + 4).clamp(5, main.height);
    let rect = Rect::new(
        main.x + (main.width - w) / 2,
        main.y + (main.height - h) / 2,
        w,
        h,
    );
    Clear.render(rect, buf);
    let block = Block::bordered()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::FG))
        .title(Line::styled(" Session wechseln ", theme::bold()));
    let inner = block.inner(rect).inner_x(1);
    block.render(rect, buf);
    let rows = usize::from(inner.height).saturating_sub(1);
    let start = selected.saturating_sub(rows.saturating_sub(1));
    let mut lines: Vec<Line<'static>> = app
        .sessions
        .iter()
        .enumerate()
        .skip(start)
        .take(rows)
        .map(|(i, s)| {
            let g = if app.inbox.iter().any(|e| e.session_id == s.id) {
                Glyph::Waiting
            } else {
                Glyph::of_status(&s.status)
            };
            let style = if i == selected {
                Style::default()
                    .bg(theme::SELECTED_BG)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::from(vec![
                Span::styled(if i == selected { "> " } else { "  " }, style),
                Span::styled(g.symbol(), theme::glyph(g)),
                Span::styled(" ", style),
                Span::styled(
                    pad(
                        if s.title.is_empty() { &s.id } else { &s.title },
                        usize::from(inner.width).saturating_sub(4),
                    ),
                    style,
                ),
            ])
        })
        .collect();
    if app.sessions.is_empty() {
        lines.push(Line::styled("keine Sessions", theme::dim()));
    }
    lines.push(Line::styled(
        "↑↓ wählen · ↵ öffnen · Esc zurück",
        theme::dim(),
    ));
    draw_lines(buf, inner, &lines);
}

/// Text eines Buffers zeilenweise (für Tests und die Leistungsmessung).
pub fn buffer_lines(buf: &Buffer) -> Vec<String> {
    let area = buf.area;
    (area.y..area.y + area.height)
        .map(|y| {
            let mut s = String::new();
            for x in area.x..area.x + area.width {
                s.push_str(buf.cell(Position::new(x, y)).map_or(" ", |c| c.symbol()));
            }
            s.trim_end().to_owned()
        })
        .collect()
}
