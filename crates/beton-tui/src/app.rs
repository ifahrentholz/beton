//! Zustand und Tastenlogik der TUI, ohne IO: Eingaben und Nachrichten der Laufzeit ändern den
//! Zustand und liefern [`Effect`]e, die die Laufzeit über `beton-sdk` ausführt. Die TUI
//! entscheidet nichts selbst (keine Policy-, keine Kostenlogik); Freigaben gehen an die
//! Approval-API (POL-010).

use std::collections::{HashMap, HashSet};

use beton_core::event::{ApprovalKind, Event};
use beton_core::id::{ApprovalId, SessionId};
use beton_pty::Screen;
use beton_sdk::Terminal;
use beton_sdk::channel::ChannelEvent;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use serde_json::Value;
use time::{OffsetDateTime, UtcOffset};

use crate::keys::{self, Input};
use crate::layout::{Dir, Layout, PaneRef};
use crate::model::{ApprovalCard, Item, SessionModel};

/// Kleinste unterstützte Terminalgröße (TUI-001 AC3).
pub const MIN_COLS: u16 = 80;
pub const MIN_ROWS: u16 = 24;

/// Einstellungen der Darstellung.
#[derive(Debug, Clone)]
pub struct AppOptions {
    /// Linke Seite der Titelleiste, z. B. `local`.
    pub server_label: String,
    /// Versatz der lokalen Zeitzone (Uhrzeit, Zeitpunkte von Entscheidungen).
    pub offset: UtcOffset,
    /// Läuft die TUI unter Windows (Titel nennt Windows Terminal bzw. Konsole, TUI-005)?
    pub windows: Option<WindowsHost>,
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            server_label: "local".into(),
            offset: UtcOffset::UTC,
            windows: None,
        }
    }
}

/// Terminal-Host unter Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsHost {
    WindowsTerminal,
    Console,
}

impl WindowsHost {
    /// Erkennung über `WT_SESSION` (Windows Terminal setzt sie für jede Shell).
    pub fn detect(wt_session: Option<&str>) -> Self {
        if wt_session.is_some_and(|v| !v.is_empty()) {
            Self::WindowsTerminal
        } else {
            Self::Console
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::WindowsTerminal => "Windows Terminal · ConPTY",
            Self::Console => "Konsole · ConPTY",
        }
    }
}

/// Eintrag der Session-Liste (`GET /v1/sessions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    pub id: String,
    pub title: String,
    pub status: String,
    pub harness: String,
    pub last_activity_at: String,
    pub worktree: Option<String>,
}

impl SessionEntry {
    pub fn from_json(v: &Value) -> Option<Self> {
        if v["archived"] == true || v["kind"].as_str().is_some_and(|k| k != "main") {
            return None;
        }
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_owned();
        Some(Self {
            id: v["id"].as_str()?.to_owned(),
            title: s("title"),
            status: s("status"),
            harness: s("harness"),
            last_activity_at: s("last_activity_at"),
            worktree: v["worktree"]["path"].as_str().map(str::to_owned),
        })
    }
}

/// Offene Freigabe einer Session aus der Inbox (`GET /v1/inbox?status=open`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxEntry {
    pub session_id: String,
    pub session_title: String,
    pub created_at: String,
}

/// Eine geänderte Datei (`workspace/changes`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub status: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
}

/// Fokus in der Session-Ansicht (`Tab` wechselt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sessions,
    Chat,
    Composer,
}

/// Überlagerung über der Ansicht.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    None,
    /// `^G` gedrückt: nächste Taste wählt das Kommando.
    Prefix,
    /// `^G ?`: dieselbe Liste als Hilfe.
    Help,
    /// `^G s`: Session wählen.
    Picker {
        selected: usize,
    },
}

/// Verbindungszustand eines Terminal-Panes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneStatus {
    Connecting,
    Open,
    Reconnecting,
    Closed(String),
}

/// Ein Terminal-Pane: emulierter Bildschirm des PTY im Runner.
#[derive(Debug)]
pub struct Pane {
    pub terminal: Terminal,
    pub screen: Screen,
    pub status: PaneStatus,
    /// Zeilen zurück im Scrollback (0: aktuell).
    pub scroll: usize,
    /// Seit dem Wiederverbinden schon ein Keyframe gesehen?
    pub restored: bool,
}

/// Was die Laufzeit ausführen soll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Events der Session abonnieren, Layout laden und Terminals verbinden.
    Subscribe(SessionId),
    Send {
        session: SessionId,
        text: String,
    },
    Interrupt(SessionId),
    Resolve {
        session: SessionId,
        approval: ApprovalId,
        allow: bool,
    },
    AllowForSession {
        session: SessionId,
        approval: ApprovalId,
    },
    Answer {
        session: SessionId,
        approval: ApprovalId,
        text: String,
    },
    OpenTerminal {
        session: SessionId,
        dir: Dir,
        cols: u16,
        rows: u16,
    },
    /// Binärkanal eines Terminals öffnen.
    Attach {
        session: SessionId,
        terminal_id: String,
    },
    /// Binärkanal schließen (Prozess läuft im Runner weiter).
    Detach {
        terminal_id: String,
    },
    CloseTerminal {
        session: SessionId,
        terminal_id: String,
    },
    ResizeTerminal {
        session: SessionId,
        terminal_id: String,
        cols: u16,
        rows: u16,
    },
    PaneInput {
        terminal_id: String,
        bytes: Vec<u8>,
    },
    SaveLayout {
        session: SessionId,
        layout: Layout,
    },
    RefreshChanges(SessionId),
    Quit,
}

/// Nachrichten der Laufzeit an die App.
#[derive(Debug)]
pub enum Msg {
    Events {
        session: SessionId,
        events: Vec<Event>,
    },
    Live {
        session: SessionId,
    },
    Reset {
        session: SessionId,
    },
    Sessions(Vec<Value>),
    Inbox(Vec<Value>),
    Changes {
        session: SessionId,
        items: Vec<Value>,
    },
    /// Gespeichertes Layout und laufende Terminals nach dem Öffnen einer Session.
    Restored {
        session: SessionId,
        layout: Option<Layout>,
        terminals: Vec<Terminal>,
    },
    TerminalOpened {
        session: SessionId,
        dir: Dir,
        terminal: Terminal,
    },
    Pane {
        terminal_id: String,
        event: ChannelEvent,
    },
    /// Fehler einer Aktion (Statuszeile).
    Error(String),
    Tick(OffsetDateTime),
}

/// Der gesamte Zustand der TUI.
#[derive(Debug)]
pub struct App {
    pub opts: AppOptions,
    /// `(cols, rows)` des Terminals.
    pub size: (u16, u16),
    pub sessions: Vec<SessionEntry>,
    pub session: Option<SessionModel>,
    /// Replay abgeschlossen, Events kommen live.
    pub live: bool,
    pub focus: Focus,
    pub composer: String,
    /// Cursor im Composer (Zeichenindex).
    pub cursor: usize,
    pub overlay: Overlay,
    /// Mit `Esc` weggeklickte Freigabe (bleibt offen, Karte versteckt).
    pub hidden_approval: Option<ApprovalId>,
    pub details: bool,
    /// Antwort auf eine Frage des Agents.
    pub answer: String,
    /// In dieser TUI per Taste entschieden (für „Erlaubt mit y“).
    pub decided: HashMap<ApprovalId, char>,
    /// Ausgewählter Eintrag im Verlauf (Index in `items`).
    pub selected: Option<usize>,
    /// Zeilen über dem Ende des Verlaufs.
    pub scroll: usize,
    /// Ausgewählte Session in der Liste.
    pub list_selected: usize,
    pub inbox: Vec<InboxEntry>,
    pub changes: Vec<Change>,
    pub layout: Layout,
    pub panes: HashMap<String, Pane>,
    /// Hinweis nach dem Wiederherstellen der Panes (AC2).
    pub banner: Option<String>,
    /// Kurzer Hinweis bzw. Fehler in der Statuszeile.
    pub flash: Option<String>,
    pub now: OffsetDateTime,
    pub quit: bool,
    /// Nach dem Laden der Session die Freigabe zeigen (`^G a` in eine andere Session).
    show_approval_on_load: bool,
    changes_version: u64,
    restored_terminals: HashSet<String>,
}

impl App {
    pub fn new(opts: AppOptions, size: (u16, u16)) -> Self {
        Self {
            opts,
            size,
            sessions: Vec::new(),
            session: None,
            live: false,
            focus: Focus::Composer,
            composer: String::new(),
            cursor: 0,
            overlay: Overlay::None,
            hidden_approval: None,
            details: false,
            answer: String::new(),
            decided: HashMap::new(),
            selected: None,
            scroll: 0,
            list_selected: 0,
            inbox: Vec::new(),
            changes: Vec::new(),
            layout: Layout::default(),
            panes: HashMap::new(),
            banner: None,
            flash: None,
            now: OffsetDateTime::UNIX_EPOCH,
            quit: false,
            show_approval_on_load: false,
            changes_version: 0,
            restored_terminals: HashSet::new(),
        }
    }

    pub fn too_small(&self) -> bool {
        self.size.0 < MIN_COLS || self.size.1 < MIN_ROWS
    }

    pub fn session_id(&self) -> Option<SessionId> {
        self.session.as_ref().map(|s| s.id)
    }

    /// Öffnet eine Session (Abo, Layout, Terminals).
    pub fn open_session(&mut self, id: SessionId) -> Vec<Effect> {
        let mut effects: Vec<Effect> = self
            .panes
            .keys()
            .map(|t| Effect::Detach {
                terminal_id: t.clone(),
            })
            .collect();
        self.panes.clear();
        self.restored_terminals.clear();
        let mut model = SessionModel::new(id);
        if let Some(e) = self.sessions.iter().find(|s| s.id == id.to_string()) {
            model.title.clone_from(&e.title);
            model.harness.clone_from(&e.harness);
        }
        self.session = Some(model);
        self.live = false;
        self.layout = Layout::default();
        self.selected = None;
        self.scroll = 0;
        self.details = false;
        self.hidden_approval = None;
        self.changes.clear();
        self.changes_version = 0;
        self.banner = None;
        self.focus = Focus::Composer;
        effects.push(Effect::Subscribe(id));
        effects
    }

    /// Offene Freigabe der aktuellen Session, die die Karte zeigt.
    pub fn visible_approval(&self) -> Option<&ApprovalCard> {
        let card = self.session.as_ref()?.pending_approvals().next()?;
        (self.hidden_approval != Some(card.approval_id)).then_some(card)
    }

    /// Fängt die Karte die Tasten (`y`/`n`/`s`/`d`)? Nur, wenn der Verlauf den Fokus hat
    /// (in der Pane-Ansicht nicht, solange ein Terminal-Pane fokussiert ist).
    pub fn approval_captures_keys(&self) -> bool {
        self.visible_approval().is_some() && self.layout.focus == PaneRef::Agent
    }

    /// Offene Freigaben in anderen Sessions (TUI-004 AC2).
    pub fn other_approvals(&self) -> Vec<&InboxEntry> {
        let current = self.session_id().map(|s| s.to_string());
        self.inbox
            .iter()
            .filter(|e| Some(&e.session_id) != current.as_ref())
            .collect()
    }

    /// Eine Nachricht der Laufzeit anwenden.
    pub fn update(&mut self, msg: Msg) -> Vec<Effect> {
        match msg {
            Msg::Events { session, events } => {
                let Some(model) = self.session.as_mut().filter(|m| m.id == session) else {
                    return Vec::new();
                };
                let before = model.workspace_version;
                model.apply_all(&events);
                let mut out = Vec::new();
                if model.workspace_version != before && self.live {
                    out.push(Effect::RefreshChanges(session));
                }
                if self.show_approval_on_load && model.pending_count() > 0 {
                    self.show_approval_on_load = false;
                    self.hidden_approval = None;
                }
                out
            }
            Msg::Live { session } => {
                if self.session_id() != Some(session) {
                    return Vec::new();
                }
                self.live = true;
                vec![Effect::RefreshChanges(session)]
            }
            Msg::Reset { session } => {
                if let Some(m) = self.session.as_mut().filter(|m| m.id == session) {
                    m.reset();
                }
                Vec::new()
            }
            Msg::Sessions(items) => {
                let mut list: Vec<SessionEntry> =
                    items.iter().filter_map(SessionEntry::from_json).collect();
                list.sort_by(|a, b| b.last_activity_at.cmp(&a.last_activity_at));
                if let Some(m) = self.session.as_mut()
                    && let Some(e) = list.iter().find(|e| e.id == m.id.to_string())
                    && m.title.is_empty()
                {
                    m.title.clone_from(&e.title);
                }
                self.sessions = list;
                self.list_selected = self
                    .list_selected
                    .min(self.sessions.len().saturating_sub(1));
                Vec::new()
            }
            Msg::Inbox(items) => {
                self.inbox = items
                    .iter()
                    .filter(|i| {
                        i["status"] == "open"
                            && matches!(i["kind"].as_str(), Some("approval" | "question"))
                    })
                    .map(|i| InboxEntry {
                        session_id: i["session_id"].as_str().unwrap_or_default().to_owned(),
                        session_title: i["session_title"].as_str().unwrap_or_default().to_owned(),
                        created_at: i["created_at"].as_str().unwrap_or_default().to_owned(),
                    })
                    .collect();
                // Älteste zuerst: `^G a` springt zur am längsten wartenden.
                self.inbox.sort_by(|a, b| a.created_at.cmp(&b.created_at));
                Vec::new()
            }
            Msg::Changes { session, items } => {
                if self.session_id() == Some(session) {
                    self.changes = items
                        .iter()
                        .map(|c| Change {
                            path: c["path"].as_str().unwrap_or_default().to_owned(),
                            status: c["status"].as_str().unwrap_or_default().to_owned(),
                            additions: c["additions"].as_u64(),
                            deletions: c["deletions"].as_u64(),
                        })
                        .collect();
                }
                Vec::new()
            }
            Msg::Restored {
                session,
                layout,
                terminals,
            } => self.restore(session, layout, terminals),
            Msg::TerminalOpened {
                session,
                dir,
                terminal,
            } => {
                if self.session_id() != Some(session) {
                    return Vec::new();
                }
                let id = terminal.terminal_id.clone();
                self.add_pane(terminal);
                self.layout.split(
                    dir,
                    PaneRef::Terminal {
                        terminal_id: id.clone(),
                    },
                );
                let mut out = vec![Effect::Attach {
                    session,
                    terminal_id: id,
                }];
                out.extend(self.layout_changed());
                out
            }
            Msg::Pane { terminal_id, event } => self.pane_event(&terminal_id, event),
            Msg::Error(e) => {
                self.flash = Some(e);
                Vec::new()
            }
            Msg::Tick(now) => {
                self.now = now;
                Vec::new()
            }
        }
    }

    fn add_pane(&mut self, terminal: Terminal) {
        let screen = Screen::new(terminal.cols.max(2), terminal.rows.max(2));
        self.panes.insert(
            terminal.terminal_id.clone(),
            Pane {
                terminal,
                screen,
                status: PaneStatus::Connecting,
                scroll: 0,
                restored: false,
            },
        );
    }

    fn restore(
        &mut self,
        session: SessionId,
        layout: Option<Layout>,
        terminals: Vec<Terminal>,
    ) -> Vec<Effect> {
        if self.session_id() != Some(session) {
            return Vec::new();
        }
        let Some(mut layout) = layout else {
            return Vec::new();
        };
        let running: HashSet<&str> = terminals.iter().map(|t| t.terminal_id.as_str()).collect();
        layout.retain(|id| running.contains(id));
        let mut out = Vec::new();
        for t in terminals {
            if layout
                .panes()
                .iter()
                .any(|p| p.terminal_id() == Some(&t.terminal_id))
            {
                self.restored_terminals.insert(t.terminal_id.clone());
                out.push(Effect::Attach {
                    session,
                    terminal_id: t.terminal_id.clone(),
                });
                self.add_pane(t);
            }
        }
        self.layout = layout;
        out.extend(self.sync_sizes());
        out
    }

    fn pane_event(&mut self, terminal_id: &str, event: ChannelEvent) -> Vec<Effect> {
        let Some(pane) = self.panes.get_mut(terminal_id) else {
            return Vec::new();
        };
        match event {
            ChannelEvent::Opened { .. } => pane.status = PaneStatus::Open,
            ChannelEvent::Data { bytes, keyframe } => {
                if keyframe {
                    // Vollständiger Zustand ersetzt den bisherigen (Scrollback inklusive).
                    let (cols, rows) = (pane.terminal.cols.max(2), pane.terminal.rows.max(2));
                    pane.screen = Screen::new(cols, rows);
                    pane.restored = true;
                }
                pane.screen.feed(&bytes);
                if keyframe && self.restored_terminals.contains(terminal_id) {
                    self.update_banner();
                }
            }
            ChannelEvent::Reconnecting { .. } => pane.status = PaneStatus::Reconnecting,
            ChannelEvent::Closed { reason } => {
                pane.status = PaneStatus::Closed(reason);
                let p = PaneRef::Terminal {
                    terminal_id: terminal_id.to_owned(),
                };
                self.panes.remove(terminal_id);
                self.layout.remove(&p);
                return self.layout_changed();
            }
        }
        Vec::new()
    }

    /// „Wieder verbunden“ (Prototyp `tui-panes`, Zustand `reattach`).
    fn update_banner(&mut self) {
        let order: Vec<String> = self
            .layout
            .panes()
            .iter()
            .filter_map(|p| p.terminal_id().map(str::to_owned))
            .filter(|id| self.restored_terminals.contains(id))
            .collect();
        if order
            .iter()
            .any(|id| self.panes.get(id).is_some_and(|p| !p.restored))
        {
            return;
        }
        let mut restored: Vec<&mut Pane> = Vec::new();
        let mut rest: Vec<(&String, &mut Pane)> = self.panes.iter_mut().collect();
        for id in &order {
            if let Some(i) = rest.iter().position(|(k, _)| *k == id) {
                restored.push(rest.swap_remove(i).1);
            }
        }
        let n = restored.len();
        let mut names = Vec::new();
        let mut scrollback = 0;
        for p in restored {
            names.push(p.terminal.name.clone());
            scrollback += p.screen.scrollback_len();
        }
        let since = self
            .session
            .as_ref()
            .and_then(|m| {
                m.terminals
                    .iter()
                    .filter(|t| names.contains(&t.name))
                    .map(|t| t.opened_at)
                    .min()
            })
            .map(|t| {
                let mins = (self.now - t.as_offset_date_time()).whole_minutes().max(0);
                format!(" (seit {mins} Min.)")
            })
            .unwrap_or_default();
        let verb = if names.len() == 1 { "lief" } else { "liefen" };
        let panes_label = if n + 1 == 1 {
            "Pane".to_owned()
        } else {
            format!("{} Panes", n + 1)
        };
        self.banner = Some(format!(
            "{panes_label} wiederhergestellt; {} {verb} im Runner weiter{since} · Scrollback {} Zeilen.",
            join_names(&names),
            thousands(scrollback as u64),
        ));
        self.restored_terminals.clear();
    }

    /// Nach jeder Änderung am Layout: Größen anpassen, Layout speichern.
    fn layout_changed(&mut self) -> Vec<Effect> {
        let mut out = self.sync_sizes();
        if let Some(session) = self.session_id() {
            out.push(Effect::SaveLayout {
                session,
                layout: self.layout.clone(),
            });
        }
        out
    }

    /// Bringt die PTY-Größe jedes sichtbaren Terminal-Panes auf seine Fläche (TUI-002 AC1).
    pub fn sync_sizes(&mut self) -> Vec<Effect> {
        let Some(session) = self.session_id() else {
            return Vec::new();
        };
        if self.layout.is_single() || self.too_small() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (pane, rect) in self.layout.rects(panes_area(self.size)) {
            let Some(id) = pane.terminal_id() else {
                continue;
            };
            let Some(p) = self.panes.get_mut(id) else {
                continue;
            };
            let (cols, rows) = inner_size(rect);
            if (cols, rows) != (p.terminal.cols, p.terminal.rows) && cols >= 2 && rows >= 2 {
                p.terminal.cols = cols;
                p.terminal.rows = rows;
                p.screen.resize(cols, rows);
                out.push(Effect::ResizeTerminal {
                    session,
                    terminal_id: id.to_owned(),
                    cols,
                    rows,
                });
            }
        }
        out
    }

    /// Eine Eingabe verarbeiten.
    pub fn input(&mut self, input: Input) -> Vec<Effect> {
        match input {
            Input::Resize(cols, rows) => {
                self.size = (cols, rows);
                self.sync_sizes()
            }
            Input::Paste(text) => self.paste(&text),
            Input::Key(k) => self.key(k),
        }
    }

    fn paste(&mut self, text: &str) -> Vec<Effect> {
        if let Some(id) = self.focused_terminal() {
            let bracketed = self
                .panes
                .get(&id)
                .is_some_and(|p| p.screen.vt().bracketed_paste());
            return vec![Effect::PaneInput {
                terminal_id: id,
                bytes: keys::paste_bytes(text, bracketed),
            }];
        }
        if self
            .visible_approval()
            .is_some_and(|a| a.kind == ApprovalKind::Question)
        {
            self.answer.push_str(text);
            return Vec::new();
        }
        for c in text.chars() {
            self.insert(c);
        }
        Vec::new()
    }

    /// Fokussiertes Terminal-Pane in der Pane-Ansicht.
    pub fn focused_terminal(&self) -> Option<String> {
        if self.layout.is_single() {
            return None;
        }
        self.layout.focus.terminal_id().map(str::to_owned)
    }

    fn key(&mut self, k: KeyEvent) -> Vec<Effect> {
        self.flash = None;
        if self.overlay == Overlay::Prefix {
            self.overlay = Overlay::None;
            return self.prefix_command(k);
        }
        if keys::is_prefix(&k) {
            self.overlay = Overlay::Prefix;
            return Vec::new();
        }
        match self.overlay {
            Overlay::Help => {
                if matches!(k.code, KeyCode::Esc | KeyCode::Char('q' | '?')) {
                    self.overlay = Overlay::None;
                }
                return Vec::new();
            }
            Overlay::Picker { selected } => return self.picker_key(k, selected),
            Overlay::Prefix | Overlay::None => {}
        }
        if let Some(id) = self.focused_terminal() {
            return self.terminal_key(&id, k);
        }
        if self.approval_captures_keys() {
            return self.approval_key(k);
        }
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            if self.composer.is_empty() {
                self.flash = Some("^G q beendet die TUI; die Session läuft weiter.".into());
            }
            self.composer.clear();
            self.cursor = 0;
            return Vec::new();
        }
        match k.code {
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Sessions => Focus::Chat,
                    Focus::Chat => Focus::Composer,
                    Focus::Composer if self.layout.is_single() => Focus::Sessions,
                    Focus::Composer => Focus::Chat,
                };
                return Vec::new();
            }
            KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Sessions => Focus::Composer,
                    Focus::Chat if self.layout.is_single() => Focus::Sessions,
                    Focus::Chat => Focus::Composer,
                    Focus::Composer => Focus::Chat,
                };
                return Vec::new();
            }
            KeyCode::Esc => {
                if let Some(m) = &self.session
                    && m.busy()
                {
                    return vec![Effect::Interrupt(m.id)];
                }
                return Vec::new();
            }
            KeyCode::PageUp => {
                self.scroll += usize::from(self.size.1 / 2);
                return Vec::new();
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(usize::from(self.size.1 / 2));
                return Vec::new();
            }
            _ => {}
        }
        match self.focus {
            Focus::Composer => self.composer_key(k),
            Focus::Chat => self.chat_key(k),
            Focus::Sessions => self.list_key(k),
        }
    }

    fn insert(&mut self, c: char) {
        let at = self
            .composer
            .char_indices()
            .nth(self.cursor)
            .map_or(self.composer.len(), |(i, _)| i);
        self.composer.insert(at, c);
        self.cursor += 1;
    }

    fn composer_key(&mut self, k: KeyEvent) -> Vec<Effect> {
        let len = self.composer.chars().count();
        match k.code {
            KeyCode::Enter => {
                let text = self.composer.trim().to_owned();
                let Some(session) = self.session_id() else {
                    return Vec::new();
                };
                if text.is_empty() {
                    return Vec::new();
                }
                self.composer.clear();
                self.cursor = 0;
                self.scroll = 0;
                return vec![Effect::Send { session, text }];
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => self.insert(c),
            KeyCode::Backspace if self.cursor > 0 => {
                if let Some((i, _)) = self.composer.char_indices().nth(self.cursor - 1) {
                    self.composer.remove(i);
                    self.cursor -= 1;
                }
            }
            KeyCode::Delete if self.cursor < len => {
                if let Some((i, _)) = self.composer.char_indices().nth(self.cursor) {
                    self.composer.remove(i);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            KeyCode::Up => self.scroll += 1,
            KeyCode::Down => self.scroll = self.scroll.saturating_sub(1),
            _ => {}
        }
        Vec::new()
    }

    /// Indizes der aufklappbaren Tool-Cards im Verlauf.
    fn tool_indices(&self) -> Vec<usize> {
        self.session
            .as_ref()
            .map(|m| {
                m.items
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| matches!(i, Item::Tool(_)))
                    .map(|(n, _)| n)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn chat_key(&mut self, k: KeyEvent) -> Vec<Effect> {
        let tools = self.tool_indices();
        let pos = self
            .selected
            .and_then(|s| tools.iter().position(|t| *t == s));
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = match pos {
                    Some(0) => tools.first().copied(),
                    Some(p) => tools.get(p - 1).copied(),
                    None => tools.last().copied(),
                };
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = match pos {
                    Some(p) if p + 1 < tools.len() => tools.get(p + 1).copied(),
                    Some(_) => None,
                    None => None,
                };
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let (Some(i), Some(m)) = (self.selected, self.session.as_mut()) {
                    m.toggle_tool(i);
                }
            }
            KeyCode::End => {
                self.scroll = 0;
                self.selected = None;
            }
            _ => {}
        }
        Vec::new()
    }

    fn list_key(&mut self, k: KeyEvent) -> Vec<Effect> {
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.list_selected = self.list_selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.list_selected =
                    (self.list_selected + 1).min(self.sessions.len().saturating_sub(1));
            }
            KeyCode::Enter => return self.switch_to(self.list_selected),
            _ => {}
        }
        Vec::new()
    }

    fn switch_to(&mut self, index: usize) -> Vec<Effect> {
        let Some(id) = self
            .sessions
            .get(index)
            .and_then(|e| e.id.parse::<SessionId>().ok())
        else {
            return Vec::new();
        };
        if self.session_id() == Some(id) {
            return Vec::new();
        }
        self.open_session(id)
    }

    fn picker_key(&mut self, k: KeyEvent, selected: usize) -> Vec<Effect> {
        let max = self.sessions.len().saturating_sub(1);
        match k.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Up | KeyCode::Char('k') => {
                self.overlay = Overlay::Picker {
                    selected: selected.saturating_sub(1),
                };
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.overlay = Overlay::Picker {
                    selected: (selected + 1).min(max),
                };
            }
            KeyCode::Enter => {
                self.overlay = Overlay::None;
                self.list_selected = selected;
                return self.switch_to(selected);
            }
            _ => {}
        }
        Vec::new()
    }

    fn terminal_key(&mut self, id: &str, k: KeyEvent) -> Vec<Effect> {
        let Some(pane) = self.panes.get_mut(id) else {
            return Vec::new();
        };
        if k.modifiers.contains(KeyModifiers::SHIFT) {
            match k.code {
                KeyCode::PageUp => {
                    pane.scroll += usize::from(pane.terminal.rows / 2).max(1);
                    pane.screen.set_scrollback(pane.scroll);
                    pane.scroll = pane.screen.vt().scrollback();
                    return Vec::new();
                }
                KeyCode::PageDown => {
                    pane.scroll = pane
                        .scroll
                        .saturating_sub(usize::from(pane.terminal.rows / 2).max(1));
                    pane.screen.set_scrollback(pane.scroll);
                    return Vec::new();
                }
                _ => {}
            }
        }
        if pane.scroll > 0 {
            pane.scroll = 0;
            pane.screen.set_scrollback(0);
        }
        let bytes = keys::key_bytes(&k, pane.screen.vt().application_cursor());
        if bytes.is_empty() {
            return Vec::new();
        }
        vec![Effect::PaneInput {
            terminal_id: id.to_owned(),
            bytes,
        }]
    }

    fn approval_key(&mut self, k: KeyEvent) -> Vec<Effect> {
        let (Some(session), Some(card)) = (self.session_id(), self.visible_approval().cloned())
        else {
            return Vec::new();
        };
        let id = card.approval_id;
        if card.kind == ApprovalKind::Question {
            match k.code {
                KeyCode::Esc => self.hidden_approval = Some(id),
                KeyCode::Enter if !self.answer.trim().is_empty() => {
                    let text = std::mem::take(&mut self.answer);
                    return vec![Effect::Answer {
                        session,
                        approval: id,
                        text: text.trim().to_owned(),
                    }];
                }
                KeyCode::Backspace => {
                    self.answer.pop();
                }
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.answer.push(c);
                }
                _ => {}
            }
            return Vec::new();
        }
        match k.code {
            KeyCode::Char('y') => {
                self.decided.insert(id, 'y');
                self.details = false;
                vec![Effect::Resolve {
                    session,
                    approval: id,
                    allow: true,
                }]
            }
            KeyCode::Char('n') => {
                self.decided.insert(id, 'n');
                self.details = false;
                vec![Effect::Resolve {
                    session,
                    approval: id,
                    allow: false,
                }]
            }
            KeyCode::Char('s') if card.allows_session() => {
                self.decided.insert(id, 's');
                self.details = false;
                vec![Effect::AllowForSession {
                    session,
                    approval: id,
                }]
            }
            KeyCode::Char('s') => {
                self.flash = Some("Diese Freigabe lässt sich nicht für die Session merken.".into());
                Vec::new()
            }
            KeyCode::Char('d') => {
                self.details = !self.details;
                Vec::new()
            }
            KeyCode::Esc => {
                self.hidden_approval = Some(id);
                self.details = false;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Kommando nach `^G`.
    fn prefix_command(&mut self, k: KeyEvent) -> Vec<Effect> {
        if keys::is_prefix(&k) {
            // `^G ^G` schickt `^G` an das fokussierte Pane.
            return match self.focused_terminal() {
                Some(id) => vec![Effect::PaneInput {
                    terminal_id: id,
                    bytes: vec![0x07],
                }],
                None => Vec::new(),
            };
        }
        let KeyCode::Char(c) = k.code else {
            return Vec::new();
        };
        match c {
            '|' | '-' | 'c' => {
                let Some(session) = self.session_id() else {
                    return Vec::new();
                };
                let dir = match c {
                    '|' => Dir::Columns,
                    '-' => Dir::Rows,
                    _ => self.auto_dir(),
                };
                let (cols, rows) = self.size_after_split(dir);
                vec![Effect::OpenTerminal {
                    session,
                    dir,
                    cols,
                    rows,
                }]
            }
            'o' => {
                self.layout.focus_next();
                self.layout_changed()
            }
            'z' => {
                if !self.layout.is_single() {
                    self.layout.zoom = !self.layout.zoom;
                }
                self.layout_changed()
            }
            'x' => {
                let (Some(session), Some(id)) = (self.session_id(), self.focused_terminal()) else {
                    self.flash = Some("Der Verlauf lässt sich nicht schließen.".into());
                    return Vec::new();
                };
                vec![Effect::CloseTerminal {
                    session,
                    terminal_id: id,
                }]
            }
            'd' | 'q' => {
                self.quit = true;
                let mut out = Vec::new();
                if let Some(session) = self.session_id() {
                    out.push(Effect::SaveLayout {
                        session,
                        layout: self.layout.clone(),
                    });
                }
                out.push(Effect::Quit);
                out
            }
            's' => {
                let current = self
                    .session_id()
                    .and_then(|id| self.sessions.iter().position(|s| s.id == id.to_string()))
                    .unwrap_or(0);
                self.overlay = Overlay::Picker { selected: current };
                Vec::new()
            }
            'a' => self.jump_to_approval(),
            '?' => {
                self.overlay = Overlay::Help;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// `^G a`: zur offenen Freigabe dieser Session, sonst zur ältesten in einer anderen.
    fn jump_to_approval(&mut self) -> Vec<Effect> {
        if self.session.as_ref().is_some_and(|m| m.pending_count() > 0) {
            self.hidden_approval = None;
            self.layout.focus = PaneRef::Agent;
            self.layout.zoom = false;
            return self.layout_changed();
        }
        let Some(target) = self
            .other_approvals()
            .first()
            .and_then(|e| e.session_id.parse::<SessionId>().ok())
        else {
            self.flash = Some("Keine offenen Freigaben.".into());
            return Vec::new();
        };
        self.show_approval_on_load = true;
        self.open_session(target)
    }

    /// Teilungsrichtung für `^G c`: entlang der längeren Seite des fokussierten Panes.
    fn auto_dir(&self) -> Dir {
        let rect = self
            .layout
            .rects(panes_area(self.size))
            .into_iter()
            .find(|(p, _)| *p == self.layout.focus)
            .map(|(_, r)| r)
            .unwrap_or_else(|| panes_area(self.size));
        // Zeichen sind etwa doppelt so hoch wie breit.
        if rect.width >= rect.height * 2 {
            Dir::Columns
        } else {
            Dir::Rows
        }
    }

    /// Größe des neuen Panes nach einer Teilung.
    fn size_after_split(&self, dir: Dir) -> (u16, u16) {
        let mut probe = self.layout.clone();
        let new = PaneRef::Terminal {
            terminal_id: "term_neu".into(),
        };
        probe.split(dir, new.clone());
        probe
            .rects(panes_area(self.size))
            .into_iter()
            .find(|(p, _)| *p == new)
            .map_or((80, 24), |(_, r)| inner_size(r))
    }
}

/// Fläche der Panes: alles außer Titel-, Status- und Tastenleiste.
pub fn panes_area((cols, rows): (u16, u16)) -> Rect {
    Rect::new(0, 1, cols, rows.saturating_sub(3))
}

/// Innenmaß eines Panes ohne Rahmen.
pub fn inner_size(r: Rect) -> (u16, u16) {
    (r.width.saturating_sub(2), r.height.saturating_sub(2))
}

fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} und {last}", init.join(", ")),
    }
}

/// `4812` → `4.812`.
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> Input {
        Input::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn ctrl_g() -> Input {
        Input::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL))
    }

    #[test]
    fn tui_002_prefix_keys_open_split_and_quit() {
        let mut app = App::new(AppOptions::default(), (120, 40));
        let id: SessionId = "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C".parse().unwrap();
        app.open_session(id);
        assert!(app.input(ctrl_g()).is_empty());
        assert_eq!(app.overlay, Overlay::Prefix);
        let fx = app.input(key('|'));
        assert_eq!(app.overlay, Overlay::None);
        assert!(matches!(
            fx.as_slice(),
            [Effect::OpenTerminal {
                dir: Dir::Columns,
                cols: 58,
                rows: 35,
                ..
            }]
        ));
        app.input(ctrl_g());
        let fx = app.input(key('q'));
        assert!(app.quit);
        assert!(matches!(fx.last(), Some(Effect::Quit)));
        assert_eq!(thousands(4812), "4.812");
        assert_eq!(thousands(12), "12");
    }

    #[test]
    fn tui_005_windows_host_is_detected_from_wt_session() {
        assert_eq!(
            WindowsHost::detect(Some("abc")),
            WindowsHost::WindowsTerminal
        );
        assert_eq!(WindowsHost::detect(None), WindowsHost::Console);
        assert_eq!(WindowsHost::detect(Some("")), WindowsHost::Console);
    }
}
