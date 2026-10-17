//! Kopflose TUI für Tests: ratatui-Test-Backend mit Zählern (Bilder, geschriebene Zellen,
//! Löschvorgänge), Tastatureingaben und Warten auf Bildschirminhalte. Wird von den
//! Integrationstests gegen einen echten Daemon mit Fake-Harness genutzt (TUI-001 … TUI-004).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beton_sdk::Client;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::{Backend, ClearType, TestBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::keys::Input;
use crate::runtime::{Options, Surface, run_with};

/// Was das Test-Backend beobachtet hat.
#[derive(Debug, Clone, Default)]
pub struct Stats {
    /// Bildschirm nach dem letzten Bild, zeilenweise.
    pub lines: Vec<String>,
    /// Anzahl gezeichneter Bilder.
    pub frames: u64,
    /// Geänderte Zellen je Bild (Diff-Rendering von ratatui).
    pub cells: Vec<usize>,
    /// Zeitpunkte der Bilder.
    pub times: Vec<Instant>,
    /// Vollständige Löschvorgänge des Bildschirms.
    pub clears: u64,
    /// Cursor nach dem letzten Bild, falls sichtbar.
    pub cursor: Option<Position>,
    /// Vom Test gewünschte neue Größe (wie ein vergrößertes Fenster).
    pub want_size: Option<(u16, u16)>,
}

/// Test-Backend, das jedes Bild in [`Stats`] festhält.
#[derive(Debug)]
pub struct RecordingBackend {
    inner: TestBackend,
    stats: Arc<Mutex<Stats>>,
    pending: usize,
    cursor_visible: bool,
}

impl RecordingBackend {
    pub fn new(cols: u16, rows: u16, stats: Arc<Mutex<Stats>>) -> Self {
        Self {
            inner: TestBackend::new(cols, rows),
            stats,
            pending: 0,
            cursor_visible: false,
        }
    }

    fn apply_resize(&mut self) {
        let want = self.stats.lock().ok().and_then(|s| s.want_size);
        if let Some((c, r)) = want {
            let size = self.inner.size().unwrap_or_default();
            if (size.width, size.height) != (c, r) {
                self.inner.resize(c, r);
            }
        }
    }
}

impl Backend for RecordingBackend {
    type Error = std::convert::Infallible;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.apply_resize();
        let cells: Vec<(u16, u16, &Cell)> = content.collect();
        self.pending += cells.len();
        self.inner.draw(cells.into_iter())
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.cursor_visible = false;
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.cursor_visible = true;
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        if let Ok(mut s) = self.stats.lock() {
            s.clears += 1;
        }
        self.apply_resize();
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        if let Some((width, height)) = self.stats.lock().ok().and_then(|s| s.want_size) {
            return Ok(Size { width, height });
        }
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        let lines = crate::view::buffer_lines(self.inner.buffer());
        let cursor = if self.cursor_visible {
            self.inner.get_cursor_position().ok()
        } else {
            None
        };
        if let Ok(mut s) = self.stats.lock() {
            s.lines = lines;
            s.frames += 1;
            s.cells.push(std::mem::take(&mut self.pending));
            s.times.push(Instant::now());
            s.cursor = cursor;
        }
        self.inner.flush()
    }
}

impl Surface for RecordingBackend {}

/// Eine laufende TUI ohne echtes Terminal.
pub struct Headless {
    input: mpsc::Sender<Input>,
    stats: Arc<Mutex<Stats>>,
    task: JoinHandle<std::io::Result<()>>,
}

impl std::fmt::Debug for Headless {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Headless").finish_non_exhaustive()
    }
}

impl Headless {
    /// Startet die TUI mit `cols` × `rows` gegen `client`.
    pub fn start(client: Client, opts: Options, cols: u16, rows: u16) -> Self {
        let stats = Arc::new(Mutex::new(Stats::default()));
        let backend = RecordingBackend::new(cols, rows, stats.clone());
        let (tx, rx) = mpsc::channel(256);
        let task = tokio::spawn(async move {
            let mut terminal = Terminal::new(backend).map_err(std::io::Error::other)?;
            run_with(&mut terminal, client, opts, rx).await
        });
        Self {
            input: tx,
            stats,
            task,
        }
    }

    pub async fn send(&self, input: Input) {
        let _ = self.input.send(input).await;
    }

    pub async fn key(&self, code: KeyCode) {
        self.send(Input::Key(KeyEvent::new(code, KeyModifiers::NONE)))
            .await;
    }

    pub async fn ctrl(&self, c: char) {
        self.send(Input::Key(KeyEvent::new(
            KeyCode::Char(c),
            KeyModifiers::CONTROL,
        )))
        .await;
    }

    /// `^G` und danach `c`.
    pub async fn prefix(&self, c: char) {
        self.ctrl('g').await;
        self.key(KeyCode::Char(c)).await;
    }

    /// Neue Fenstergröße, wie beim Vergrößern des Terminals.
    pub async fn resize(&self, cols: u16, rows: u16) {
        if let Ok(mut s) = self.stats.lock() {
            s.want_size = Some((cols, rows));
        }
        self.send(Input::Resize(cols, rows)).await;
    }

    pub async fn type_text(&self, text: &str) {
        for c in text.chars() {
            self.key(KeyCode::Char(c)).await;
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Bildschirmtext des letzten Bildes.
    pub fn screen(&self) -> String {
        self.stats().lines.join("\n")
    }

    /// Wartet, bis `check` auf den Bildschirmtext zutrifft; sonst Panic mit dem Bildschirm.
    pub async fn wait_for(
        &self,
        what: &str,
        timeout: Duration,
        check: impl Fn(&str) -> bool,
    ) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            let screen = self.screen();
            if check(&screen) {
                return screen;
            }
            assert!(
                Instant::now() < deadline,
                "Zeitüberschreitung beim Warten auf {what}; Bildschirm:\n{screen}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Beendet die TUI (`^G q`) und wartet auf das Ende.
    pub async fn quit(self) -> std::io::Result<()> {
        self.prefix('q').await;
        match tokio::time::timeout(Duration::from_secs(10), self.task).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => Err(std::io::Error::other(e)),
            Err(_) => Err(std::io::Error::other("TUI endet nicht")),
        }
    }
}
