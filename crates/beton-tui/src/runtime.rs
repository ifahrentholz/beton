//! Laufzeit der TUI: Ereignisschleife mit gedrosselter Bildrate (30 fps, TUI-001 AC2),
//! Diff-Rendering über ratatui, Ausführung der [`Effect`]e über `beton-sdk`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use beton_core::event::Event;
use beton_core::id::SessionId;
use beton_proto::channel::ChannelKind;
use beton_sdk::channel::{Channel, ChannelInput};
use beton_sdk::ws::Update;
use beton_sdk::{Client, TerminalOpen};
use ratatui::Terminal;
use ratatui::backend::Backend;
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::app::{App, AppOptions, Effect, Msg};
use crate::keys::Input;
use crate::layout;

/// Abstand zweier Bilder: höchstens 30 pro Sekunde (TUI-001 AC2).
pub const FRAME: Duration = Duration::from_millis(33);
/// So oft fragt die TUI Session-Liste und Inbox ab (wie die Web-UI, ≤ 1 s).
pub const POLL: Duration = Duration::from_millis(1000);

/// Rückruf mit den Events, die die TUI anwendet (Tests von TUI-001 AC1).
pub type EventHook = Arc<dyn Fn(&[Event]) + Send + Sync>;

/// Start-Optionen.
#[derive(Clone)]
pub struct Options {
    /// Datenverzeichnis (`$BETON_HOME`), dort liegen die Layouts.
    pub home: PathBuf,
    /// Session, die zuerst geöffnet wird; ohne: die zuletzt aktive.
    pub session: Option<SessionId>,
    pub app: AppOptions,
    pub on_events: Option<EventHook>,
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("home", &self.home)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

/// Ein Zeichenziel mit optionaler synchronisierter Ausgabe (kein Flackern beim Zeichnen).
pub trait Surface: Backend {
    fn begin_frame(&mut self) {}
    fn end_frame(&mut self) {}
}

impl<W: std::io::Write> Surface for ratatui::backend::CrosstermBackend<W> {
    fn begin_frame(&mut self) {
        let _ = crossterm::queue!(self, crossterm::terminal::BeginSynchronizedUpdate);
    }

    fn end_frame(&mut self) {
        let _ = crossterm::execute!(self, crossterm::terminal::EndSynchronizedUpdate);
    }
}

impl Surface for ratatui::backend::TestBackend {}

fn io(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

struct Executor {
    client: Client,
    home: PathBuf,
    tx: mpsc::UnboundedSender<Msg>,
    subscription: Option<JoinHandle<()>>,
    channels: HashMap<String, (ChannelInput, JoinHandle<()>)>,
}

impl Executor {
    fn spawn<F>(&self, f: F)
    where
        F: std::future::Future<Output = Option<Msg>> + Send + 'static,
    {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Some(msg) = f.await {
                let _ = tx.send(msg);
            }
        });
    }

    fn run(&mut self, effect: Effect) {
        let c = self.client.clone();
        match effect {
            Effect::Subscribe(id) => self.subscribe(id),
            Effect::Send { session, text } => self.spawn(async move {
                c.input(&session.to_string(), &text)
                    .await
                    .err()
                    .map(|e| Msg::Error(format!("Senden fehlgeschlagen: {e}")))
            }),
            Effect::Interrupt(session) => self.spawn(async move {
                c.interrupt(&session.to_string())
                    .await
                    .err()
                    .map(|e| Msg::Error(format!("Unterbrechen fehlgeschlagen: {e}")))
            }),
            Effect::Resolve {
                session,
                approval,
                allow,
            } => self.spawn(async move {
                let reason = (!allow).then_some("in der TUI abgelehnt");
                c.resolve_approval(&session.to_string(), &approval.to_string(), allow, reason)
                    .await
                    .err()
                    .map(|e| Msg::Error(format!("Freigabe fehlgeschlagen: {e}")))
            }),
            Effect::AllowForSession { session, approval } => self.spawn(async move {
                c.resolve_approval_for_session(&session.to_string(), &approval.to_string())
                    .await
                    .err()
                    .map(|e| Msg::Error(format!("Freigabe fehlgeschlagen: {e}")))
            }),
            Effect::Answer {
                session,
                approval,
                text,
            } => self.spawn(async move {
                c.answer_question(&session.to_string(), &approval.to_string(), &text)
                    .await
                    .err()
                    .map(|e| Msg::Error(format!("Antwort fehlgeschlagen: {e}")))
            }),
            Effect::OpenTerminal {
                session,
                dir,
                cols,
                rows,
            } => self.spawn(async move {
                let req = TerminalOpen {
                    cols: cols.max(2),
                    rows: rows.max(2),
                    ..TerminalOpen::default()
                };
                Some(match c.open_terminal(&session.to_string(), &req).await {
                    Ok(terminal) => Msg::TerminalOpened {
                        session,
                        dir,
                        terminal,
                    },
                    Err(e) => Msg::Error(format!("Terminal nicht geöffnet: {e}")),
                })
            }),
            Effect::Attach {
                session,
                terminal_id,
            } => self.attach(session, terminal_id),
            Effect::Detach { terminal_id } => {
                if let Some((_, task)) = self.channels.remove(&terminal_id) {
                    task.abort();
                }
            }
            Effect::CloseTerminal {
                session,
                terminal_id,
            } => self.spawn(async move {
                c.close_terminal(&session.to_string(), &terminal_id)
                    .await
                    .err()
                    .map(|e| Msg::Error(format!("Terminal nicht geschlossen: {e}")))
            }),
            Effect::ResizeTerminal {
                session,
                terminal_id,
                cols,
                rows,
            } => self.spawn(async move {
                c.resize_terminal(&session.to_string(), &terminal_id, cols, rows)
                    .await
                    .err()
                    .map(|e| Msg::Error(format!("Größe nicht übernommen: {e}")))
            }),
            Effect::PaneInput { terminal_id, bytes } => {
                if let Some((input, _)) = self.channels.get(&terminal_id) {
                    input.send(bytes);
                }
            }
            Effect::SaveLayout { session, layout } => {
                if let Err(e) = layout::save(&self.home, &session.to_string(), &layout) {
                    tracing::warn!(error = %e, "Layout nicht gespeichert");
                }
            }
            Effect::RefreshChanges(session) => self.spawn(async move {
                match c.workspace_changes(&session.to_string()).await {
                    Ok(page) => Some(Msg::Changes {
                        session,
                        items: page.items,
                    }),
                    // Ohne Worktree gibt es keine Änderungen; das ist kein Fehler.
                    Err(_) => None,
                }
            }),
            Effect::Quit => {}
        }
    }

    fn subscribe(&mut self, id: SessionId) {
        if let Some(old) = self.subscription.take() {
            old.abort();
        }
        for (_, (_, task)) in self.channels.drain() {
            task.abort();
        }
        let client = self.client.clone();
        let tx = self.tx.clone();
        self.subscription = Some(tokio::spawn(async move {
            let mut sub = match client.subscribe(id, 0).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(Msg::Error(format!("Session nicht abonniert: {e}")));
                    return;
                }
            };
            loop {
                let msg = match sub.next().await {
                    Ok(Update::Events(events)) => Msg::Events {
                        session: id,
                        events,
                    },
                    Ok(Update::Live { .. }) => Msg::Live { session: id },
                    Ok(Update::Reset) => Msg::Reset { session: id },
                    Ok(Update::Reconnecting { .. }) => continue,
                    Err(e) => {
                        let _ = tx.send(Msg::Error(format!("Verbindung verloren: {e}")));
                        return;
                    }
                };
                if tx.send(msg).is_err() {
                    return;
                }
            }
        }));
        let client = self.client.clone();
        let home = self.home.clone();
        self.spawn(async move {
            let saved = layout::load(&home, &id.to_string());
            let terminals = if saved.is_some() {
                client.terminals(&id.to_string()).await.unwrap_or_default()
            } else {
                Vec::new()
            };
            Some(Msg::Restored {
                session: id,
                layout: saved,
                terminals,
            })
        });
    }

    fn attach(&mut self, session: SessionId, terminal_id: String) {
        if self.channels.contains_key(&terminal_id) {
            return;
        }
        let channel = Channel::open(&self.client, session, ChannelKind::Terminal, &terminal_id);
        let (input, mut events) = channel.split();
        let tx = self.tx.clone();
        let id = terminal_id.clone();
        let task = tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                let msg = Msg::Pane {
                    terminal_id: id.clone(),
                    event,
                };
                if tx.send(msg).is_err() {
                    return;
                }
            }
        });
        self.channels.insert(terminal_id, (input, task));
    }

    fn shutdown(&mut self) {
        if let Some(s) = self.subscription.take() {
            s.abort();
        }
        for (_, (_, task)) in self.channels.drain() {
            task.abort();
        }
    }
}

/// Liste und Inbox regelmäßig abfragen.
fn spawn_pollers(client: &Client, tx: &mpsc::UnboundedSender<Msg>) -> Vec<JoinHandle<()>> {
    let c = client.clone();
    let t = tx.clone();
    let lists = tokio::spawn(async move {
        loop {
            if let Ok(page) = c.sessions(Some(200), None, false).await
                && t.send(Msg::Sessions(page.items)).is_err()
            {
                return;
            }
            if let Ok(inbox) = c.inbox(Some("open"), 200).await {
                let items = inbox["items"].as_array().cloned().unwrap_or_default();
                if t.send(Msg::Inbox(items)).is_err() {
                    return;
                }
            }
            tokio::time::sleep(POLL).await;
        }
    });
    let t = tx.clone();
    let clock = tokio::spawn(async move {
        loop {
            if t.send(Msg::Tick(OffsetDateTime::now_utc())).is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    vec![lists, clock]
}

/// Führt die TUI auf `terminal` aus, bis der Benutzer sie beendet oder `input` endet.
pub async fn run_with<B>(
    terminal: &mut Terminal<B>,
    client: Client,
    opts: Options,
    mut input: mpsc::Receiver<Input>,
) -> std::io::Result<()>
where
    B: Surface,
    B::Error: std::fmt::Display,
{
    let size = terminal.size().map_err(io)?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = App::new(opts.app.clone(), (size.width, size.height));
    app.now = OffsetDateTime::now_utc();
    let mut exec = Executor {
        client: client.clone(),
        home: opts.home.clone(),
        tx: tx.clone(),
        subscription: None,
        channels: HashMap::new(),
    };
    let pollers = spawn_pollers(&client, &tx);
    let mut want_first = opts.session.is_none();
    if let Some(id) = opts.session {
        for e in app.open_session(id) {
            exec.run(e);
        }
    }
    let mut dirty = true;
    let mut last_draw = Instant::now() - FRAME;
    let result = loop {
        let next_frame = last_draw + FRAME;
        let effects = tokio::select! {
            biased;
            () = tokio::time::sleep_until(next_frame), if dirty => {
                terminal.backend_mut().begin_frame();
                let drawn = terminal.draw(|f| crate::view::render(&app, f)).map(|_| ()).map_err(io);
                terminal.backend_mut().end_frame();
                if let Err(e) = drawn {
                    break Err(e);
                }
                dirty = false;
                last_draw = Instant::now();
                continue;
            }
            event = input.recv() => match event {
                Some(Input::Resize(c, r)) => {
                    // Neue Größe: einmal vollständig neu zeichnen.
                    let _ = terminal.autoresize();
                    let _ = terminal.clear();
                    app.input(Input::Resize(c, r))
                }
                Some(i) => app.input(i),
                None => break Ok(()),
            },
            msg = rx.recv() => match msg {
                Some(msg) => {
                    if let (Some(hook), Msg::Events { events, session }) = (&opts.on_events, &msg)
                        && app.session_id() == Some(*session)
                    {
                        hook(events);
                    }
                    let first = want_first && matches!(msg, Msg::Sessions(_));
                    let mut fx = app.update(msg);
                    if first {
                        want_first = false;
                        if app.session.is_none()
                            && let Some(id) = app.sessions.first().and_then(|s| s.id.parse().ok())
                        {
                            fx.extend(app.open_session(id));
                        }
                    }
                    fx
                }
                None => break Ok(()),
            },
        };
        dirty = true;
        for e in effects {
            exec.run(e);
        }
        if app.quit {
            break Ok(());
        }
    };
    exec.shutdown();
    for p in pollers {
        p.abort();
    }
    result
}

/// Startet die TUI im echten Terminal (crossterm: Unix, Windows Terminal und Konsole).
pub async fn run(client: Client, opts: Options) -> std::io::Result<()> {
    use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste, EventStream};
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };
    use futures_util::StreamExt as _;

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    if let Err(e) = crossterm::execute!(stdout, EnterAlternateScreen, EnableBracketedPaste) {
        let _ = disable_raw_mode();
        return Err(e);
    }
    let restore = || {
        let _ = crossterm::execute!(
            std::io::stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
    };
    // Auch bei einem Panic das Terminal zurücksetzen.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::execute!(
            std::io::stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
        hook(info);
    }));
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            restore();
            return Err(e);
        }
    };
    let (tx, rx) = mpsc::channel(256);
    let reader = tokio::spawn(async move {
        let mut events = EventStream::new();
        while let Some(Ok(ev)) = events.next().await {
            if let Some(i) = crate::keys::from_crossterm(ev)
                && tx.send(i).await.is_err()
            {
                return;
            }
        }
    });
    let result = run_with(&mut terminal, client, opts, rx).await;
    reader.abort();
    let _ = terminal.show_cursor();
    restore();
    result
}
