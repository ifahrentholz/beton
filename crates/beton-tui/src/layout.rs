//! Pane-Layout einer Session (TUI-002): Baum aus Teilungen, Fokus, Zoom. Das Layout wird pro
//! Session unter `<data_dir>/tui/layouts/<session>.json` gespeichert; PTYs und Scrollback
//! bleiben im Runner (AC2).

use std::path::{Path, PathBuf};

use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

/// Was in einem Pane läuft.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum PaneRef {
    /// Verlauf und Eingabe der Session (Pane 0).
    Agent,
    /// Ein Terminal der Session im Runner.
    Terminal { terminal_id: String },
}

impl PaneRef {
    pub fn terminal_id(&self) -> Option<&str> {
        match self {
            Self::Terminal { terminal_id } => Some(terminal_id),
            Self::Agent => None,
        }
    }
}

/// Richtung einer Teilung: `|` nebeneinander, `-` übereinander.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    /// Senkrecht geteilt: Panes nebeneinander (`^G |`).
    Columns,
    /// Waagerecht geteilt: Panes übereinander (`^G -`).
    Rows,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Node {
    Pane {
        pane: PaneRef,
    },
    Split {
        dir: Dir,
        first: Box<Node>,
        second: Box<Node>,
    },
}

impl Node {
    fn panes(&self, out: &mut Vec<PaneRef>) {
        match self {
            Self::Pane { pane } => out.push(pane.clone()),
            Self::Split { first, second, .. } => {
                first.panes(out);
                second.panes(out);
            }
        }
    }

    fn split(&mut self, target: &PaneRef, dir: Dir, new: &PaneRef) -> bool {
        match self {
            Self::Pane { pane } if pane == target => {
                let old = std::mem::replace(
                    self,
                    Self::Pane {
                        pane: PaneRef::Agent,
                    },
                );
                *self = Self::Split {
                    dir,
                    first: Box::new(old),
                    second: Box::new(Self::Pane { pane: new.clone() }),
                };
                true
            }
            Self::Pane { .. } => false,
            Self::Split { first, second, .. } => {
                first.split(target, dir, new) || second.split(target, dir, new)
            }
        }
    }

    /// Entfernt `target`; die Geschwister rücken auf. `false`, wenn es nicht vorkam.
    fn remove(&mut self, target: &PaneRef) -> bool {
        let Self::Split { first, second, .. } = self else {
            return false;
        };
        let hit = |n: &Node| matches!(n, Node::Pane { pane } if pane == target);
        if hit(first) {
            *self = (**second).clone();
            return true;
        }
        if hit(second) {
            *self = (**first).clone();
            return true;
        }
        first.remove(target) || second.remove(target)
    }

    fn rects(&self, area: Rect, out: &mut Vec<(PaneRef, Rect)>) {
        match self {
            Self::Pane { pane } => out.push((pane.clone(), area)),
            Self::Split { dir, first, second } => {
                let (a, b) = halves(area, *dir);
                first.rects(a, out);
                second.rects(b, out);
            }
        }
    }
}

fn halves(area: Rect, dir: Dir) -> (Rect, Rect) {
    match dir {
        Dir::Columns => {
            let w = area.width / 2;
            (
                Rect { width: w, ..area },
                Rect {
                    x: area.x + w,
                    width: area.width - w,
                    ..area
                },
            )
        }
        Dir::Rows => {
            let h = area.height / 2;
            (
                Rect { height: h, ..area },
                Rect {
                    y: area.y + h,
                    height: area.height - h,
                    ..area
                },
            )
        }
    }
}

/// Layout der Panes einer Session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub root: Node,
    pub focus: PaneRef,
    #[serde(default)]
    pub zoom: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            root: Node::Pane {
                pane: PaneRef::Agent,
            },
            focus: PaneRef::Agent,
            zoom: false,
        }
    }
}

impl Layout {
    /// Panes in Anzeigereihenfolge (links/oben zuerst).
    pub fn panes(&self) -> Vec<PaneRef> {
        let mut out = Vec::new();
        self.root.panes(&mut out);
        out
    }

    /// Nur der Verlauf, keine Terminals: die normale Session-Ansicht.
    pub fn is_single(&self) -> bool {
        matches!(&self.root, Node::Pane { .. })
    }

    /// Teilt das fokussierte Pane; das neue bekommt den Fokus.
    pub fn split(&mut self, dir: Dir, new: PaneRef) {
        let target = self.focus.clone();
        if !self.root.split(&target, dir, &new) {
            // Fokus zeigt ins Leere: an der Wurzel anfügen.
            let old = std::mem::replace(&mut self.root, Node::Pane { pane: new.clone() });
            self.root = Node::Split {
                dir,
                first: Box::new(old),
                second: Box::new(Node::Pane { pane: new.clone() }),
            };
        }
        self.focus = new;
        self.zoom = false;
    }

    /// Schließt ein Pane (nie den Verlauf).
    pub fn remove(&mut self, pane: &PaneRef) {
        if *pane == PaneRef::Agent {
            return;
        }
        let order = self.panes();
        let idx = order.iter().position(|p| p == pane);
        if self.root.remove(pane) && self.focus == *pane {
            let rest = self.panes();
            self.focus = idx
                .and_then(|i| rest.get(i.saturating_sub(1)).cloned())
                .unwrap_or(PaneRef::Agent);
            self.zoom = false;
        }
    }

    /// Behält nur Terminals, für die `keep` gilt (z. B. die noch im Runner laufen).
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        for p in self.panes() {
            if let Some(id) = p.terminal_id()
                && !keep(id)
            {
                self.remove(&p);
            }
        }
        if !self.panes().contains(&self.focus) {
            self.focus = PaneRef::Agent;
        }
    }

    /// Fokus auf das nächste Pane (`^G o`).
    pub fn focus_next(&mut self) {
        let panes = self.panes();
        let i = panes.iter().position(|p| *p == self.focus).unwrap_or(0);
        self.focus = panes[(i + 1) % panes.len()].clone();
        self.zoom = false;
    }

    /// Bereiche der sichtbaren Panes; gezoomt nur das fokussierte.
    pub fn rects(&self, area: Rect) -> Vec<(PaneRef, Rect)> {
        if self.zoom {
            return vec![(self.focus.clone(), area)];
        }
        let mut out = Vec::new();
        self.root.rects(area, &mut out);
        out
    }

    /// Nummer eines Panes wie im Prototyp (0 = Agent, Terminals ab 1).
    pub fn number(&self, pane: &PaneRef) -> usize {
        self.panes().iter().position(|p| p == pane).unwrap_or(0)
    }
}

/// Datei des Layouts einer Session.
pub fn layout_path(home: &Path, session: &str) -> PathBuf {
    home.join("tui")
        .join("layouts")
        .join(format!("{session}.json"))
}

/// Gespeichertes Layout; fehlt oder ist kaputt: `None`.
pub fn load(home: &Path, session: &str) -> Option<Layout> {
    let text = std::fs::read_to_string(layout_path(home, session)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Speichert das Layout (atomar über eine temporäre Datei).
pub fn save(home: &Path, session: &str, layout: &Layout) -> std::io::Result<()> {
    let path = layout_path(home, session);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(layout).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(id: &str) -> PaneRef {
        PaneRef::Terminal {
            terminal_id: id.into(),
        }
    }

    #[test]
    fn tui_002_split_focus_zoom_and_close() {
        let mut l = Layout::default();
        assert!(l.is_single());
        l.split(Dir::Columns, term("term_1"));
        l.split(Dir::Rows, term("term_2"));
        assert_eq!(l.panes(), [PaneRef::Agent, term("term_1"), term("term_2")]);
        assert_eq!(l.focus, term("term_2"));
        let area = Rect::new(0, 0, 100, 40);
        let rects = l.rects(area);
        assert_eq!(rects[0].1, Rect::new(0, 0, 50, 40));
        assert_eq!(rects[1].1, Rect::new(50, 0, 50, 20));
        assert_eq!(rects[2].1, Rect::new(50, 20, 50, 20));
        l.zoom = true;
        assert_eq!(l.rects(area), [(term("term_2"), area)]);
        l.focus_next();
        assert_eq!(l.focus, PaneRef::Agent);
        assert!(!l.zoom);
        l.remove(&term("term_1"));
        assert_eq!(l.panes(), [PaneRef::Agent, term("term_2")]);
        l.remove(&PaneRef::Agent);
        assert_eq!(l.panes().len(), 2, "der Verlauf bleibt immer");
        l.retain(|id| id != "term_2");
        assert!(l.is_single());
    }

    #[test]
    fn tui_002_ac2_layout_is_saved_per_session() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path(), "ses_a").is_none());
        let mut l = Layout::default();
        l.split(Dir::Columns, term("term_7"));
        save(dir.path(), "ses_a", &l).unwrap();
        assert_eq!(load(dir.path(), "ses_a"), Some(l));
        assert!(load(dir.path(), "ses_b").is_none());
    }
}
