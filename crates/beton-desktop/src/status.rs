//! Was Tray, Dock-Badge und Beenden-Dialog aus Inbox und Session-Liste ableiten (DESK-002,
//! DESK-004, UX-001 AC5).

use std::collections::{BTreeMap, HashSet};

use beton_core::notifications::NotificationSettings;
use serde::Deserialize;

/// Ein Eintrag aus `GET /v1/inbox` (nur die Felder, die die App braucht).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InboxItem {
    pub id: String,
    pub session_id: String,
    #[serde(default)]
    pub session_title: String,
    /// `approval`, `question` oder `notice`.
    pub kind: String,
    /// `open`, `done` oder `info`.
    pub status: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub created_seq: u64,
}

impl InboxItem {
    pub fn is_open(&self) -> bool {
        self.status == "open" && matches!(self.kind.as_str(), "approval" | "question")
    }
}

/// Eine Session aus `GET /v1/sessions`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    #[serde(default)]
    pub title: String,
    pub status: String,
    #[serde(default)]
    pub harness: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub unread: bool,
    #[serde(default)]
    pub head_seq: u64,
    #[serde(default)]
    pub changed_at: String,
}

/// Anzeigename einer Session.
pub fn display_title(title: &str) -> String {
    let t = title.trim();
    if t.is_empty() {
        "Neue Session".into()
    } else {
        t.to_owned()
    }
}

/// Anzeigename eines Harness.
pub fn harness_name(id: &str) -> String {
    match id {
        "claude" => "Claude".into(),
        "codex" => "Codex".into(),
        "fake" => "Fake".into(),
        "direct" => "Direkt-API".into(),
        other => other.to_owned(),
    }
}

/// Zähler für Dock und Tray (DESK-004): offene Freigaben und Fragen, sofern ihr Typ den Kanal
/// „Dock-Badge“ an hat (UX-006), plus ungelesene Sessions ohne offenen Eintrag, wenn der Kanal
/// für „Agent ist fertig“ an ist. Mit den Defaults ist er genau der Zähler der Inbox
/// (UX-001 AC5), und eine Entscheidung verringert ihn sofort (DESK-004 AC1).
pub fn badge(
    open: &[InboxItem],
    sessions: &BTreeMap<String, SessionInfo>,
    settings: &NotificationSettings,
) -> u32 {
    let mut waiting = HashSet::new();
    let mut count = 0u32;
    for item in open.iter().filter(|i| i.is_open()) {
        waiting.insert(item.session_id.as_str());
        let on = match item.kind.as_str() {
            "question" => settings.events.question.badge,
            _ => settings.events.approval_requested.badge,
        };
        if on {
            count += 1;
        }
    }
    if !settings.events.turn_completed.badge {
        return count;
    }
    count
        + sessions
            .values()
            .filter(|s| s.unread && !s.archived && !waiting.contains(s.id.as_str()))
            .count() as u32
}

/// Eine Zeile im Tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayEntry {
    pub session_id: String,
    pub title: String,
    /// Rechts in der Zeile: was ansteht bzw. der Harness.
    pub note: String,
}

/// Inhalt des Tray-Menüs (Entwurf `desktop-tray`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrayModel {
    /// „Wartet auf dich“: offene Freigaben und Fragen.
    pub waiting: Vec<TrayEntry>,
    /// „Läuft“: Sessions mit laufendem Turn.
    pub running: Vec<TrayEntry>,
}

/// Höchstens so viele Zeilen je Gruppe.
pub const TRAY_LIMIT: usize = 8;

fn shorten(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or_default().trim();
    if line.chars().count() <= max {
        return line.to_owned();
    }
    let mut out: String = line.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

impl TrayModel {
    pub fn build(open: &[InboxItem], sessions: &BTreeMap<String, SessionInfo>) -> Self {
        let mut seen = HashSet::new();
        let mut waiting = Vec::new();
        for item in open.iter().filter(|i| i.is_open()) {
            if !seen.insert(item.session_id.clone()) || waiting.len() >= TRAY_LIMIT {
                continue;
            }
            let title = sessions
                .get(&item.session_id)
                .map(|s| s.title.as_str())
                .unwrap_or(&item.session_title);
            waiting.push(TrayEntry {
                session_id: item.session_id.clone(),
                title: shorten(&display_title(title), 48),
                note: if item.kind == "question" {
                    "Frage".into()
                } else {
                    shorten(&format!("{} freigeben", item.summary.trim()), 32)
                },
            });
        }
        let mut running: Vec<&SessionInfo> = sessions
            .values()
            .filter(|s| s.status == "running" && !s.archived && !seen.contains(&s.id))
            .collect();
        running.sort_by(|a, b| b.changed_at.cmp(&a.changed_at));
        let running = running
            .into_iter()
            .take(TRAY_LIMIT)
            .map(|s| TrayEntry {
                session_id: s.id.clone(),
                title: shorten(&display_title(&s.title), 48),
                note: harness_name(&s.harness),
            })
            .collect();
        Self { waiting, running }
    }
}

/// „seit 3 Std.“ für den Daemon-Status im Tray.
pub fn uptime(since: std::time::SystemTime, now: std::time::SystemTime) -> String {
    let secs = now.duration_since(since).map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        0..60 => "seit gerade eben".into(),
        60..3600 => format!("seit {} Min.", secs / 60),
        3600..86_400 => format!("seit {} Std.", secs / 3600),
        _ if secs < 2 * 86_400 => "seit 1 Tag".into(),
        _ => format!("seit {} Tagen", secs / 86_400),
    }
}

/// Kopfzeile des Tray-Menüs (Entwurf `desktop-tray`).
pub fn daemon_line(reachable: bool, version: &str, uptime: Option<&str>) -> String {
    if !reachable {
        return "Lokaler Daemon gestoppt – Sessions pausieren".into();
    }
    let mut parts = vec!["Lokaler Daemon läuft".to_owned()];
    parts.extend(uptime.map(str::to_owned));
    if !version.is_empty() {
        parts.push(format!("v{version}"));
    }
    parts.join(" · ")
}

/// Sessions, die gerade arbeiten: Beim Beenden fragt die App, ob der Daemon weiterlaufen soll
/// (DESK-002).
pub fn working(sessions: &BTreeMap<String, SessionInfo>) -> Vec<String> {
    let mut out: Vec<&SessionInfo> = sessions
        .values()
        .filter(|s| {
            !s.archived
                && matches!(
                    s.status.as_str(),
                    "running" | "waiting_approval" | "starting"
                )
        })
        .collect();
    out.sort_by(|a, b| b.changed_at.cmp(&a.changed_at));
    out.into_iter().map(|s| display_title(&s.title)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn session(id: &str, status: &str, unread: bool) -> SessionInfo {
        SessionInfo {
            id: id.into(),
            title: format!("Session {id}"),
            status: status.into(),
            harness: "claude".into(),
            archived: false,
            unread,
            head_seq: 3,
            changed_at: "2026-10-16T10:00:00.000Z".into(),
        }
    }

    pub(crate) fn item(id: &str, session: &str, kind: &str) -> InboxItem {
        InboxItem {
            id: id.into(),
            session_id: session.into(),
            session_title: format!("Session {session}"),
            kind: kind.into(),
            status: "open".into(),
            summary: "git push".into(),
            created_seq: 2,
        }
    }

    fn map(list: Vec<SessionInfo>) -> BTreeMap<String, SessionInfo> {
        list.into_iter().map(|s| (s.id.clone(), s)).collect()
    }

    #[test]
    fn ux_001_ac5_badge_mirrors_the_inbox_counter() {
        let s = NotificationSettings::default();
        let sessions = map(vec![
            session("a", "waiting_approval", true),
            session("b", "idle", false),
        ]);
        let open = vec![item("i1", "a", "approval"), item("i2", "a", "question")];
        assert_eq!(
            badge(&open, &sessions, &s),
            2,
            "Zähler = offene Freigaben und Fragen"
        );
        assert_eq!(badge(&[], &sessions, &s), 0, "entschieden: Zähler sinkt");
    }

    #[test]
    fn desk_004_badge_counts_open_approvals_plus_unread_sessions() {
        let mut s = NotificationSettings::default();
        s.events.turn_completed.badge = true;
        let sessions = map(vec![
            session("a", "waiting_approval", true),
            session("b", "idle", true),
            session("c", "idle", false),
        ]);
        let open = vec![item("i1", "a", "approval")];
        // a zählt einmal (offene Freigabe), b als ungelesen.
        assert_eq!(badge(&open, &sessions, &s), 2);
        s.events.approval_requested.badge = false;
        assert_eq!(badge(&open, &sessions, &s), 1, "Kanal Dock-Badge aus");
        let mut archived = sessions.clone();
        archived.get_mut("b").unwrap().archived = true;
        assert_eq!(badge(&[], &archived, &s), 1, "nur a, b ist archiviert");
        s.events.turn_completed.badge = false;
        assert_eq!(
            badge(&[], &sessions, &s),
            0,
            "ungelesene zählen nur mit Kanal an"
        );
    }

    #[test]
    fn desk_002_tray_header_shows_daemon_state() {
        let t0 = std::time::SystemTime::UNIX_EPOCH;
        let at = |s| t0 + std::time::Duration::from_secs(s);
        assert_eq!(uptime(t0, at(5)), "seit gerade eben");
        assert_eq!(uptime(t0, at(14 * 60)), "seit 14 Min.");
        assert_eq!(uptime(t0, at(3 * 3600 + 5)), "seit 3 Std.");
        assert_eq!(uptime(t0, at(3 * 86_400)), "seit 3 Tagen");
        assert_eq!(
            daemon_line(true, "0.1.0", Some("seit 3 Std.")),
            "Lokaler Daemon läuft · seit 3 Std. · v0.1.0"
        );
        assert_eq!(
            daemon_line(false, "0.1.0", None),
            "Lokaler Daemon gestoppt – Sessions pausieren"
        );
    }

    #[test]
    fn desk_004_tray_lists_waiting_then_running_sessions() {
        let sessions = map(vec![
            session("a", "waiting_approval", false),
            session("b", "running", false),
            session("c", "idle", false),
        ]);
        let t = TrayModel::build(
            &[item("i1", "a", "approval"), item("i2", "a", "question")],
            &sessions,
        );
        assert_eq!(t.waiting.len(), 1, "eine Zeile je Session");
        assert_eq!(t.waiting[0].note, "git push freigeben");
        assert_eq!(t.running.len(), 1);
        assert_eq!(t.running[0].session_id, "b");
        assert_eq!(t.running[0].note, "Claude");
        assert_eq!(working(&sessions), vec!["Session a", "Session b"]);
    }
}
