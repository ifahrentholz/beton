//! Permission-Modes der Claude-Code-CLI (HAR-027).
//!
//! beton setzt den Modus immer ausdrücklich (`--permission-mode`, Control-Request
//! `set_permission_mode`); sonst könnte ein Repository ihn über `.claude/settings.json`
//! (`permissions.defaultMode`) lockern – gegen 2.1.285 geprüft: `acceptEdits` aus den
//! Projekt-Einstellungen wirkt ohne Flag, mit `--permission-mode default` nicht. Zusätzlich
//! prüft der [`ModeGuard`] jeden Modus, den die CLI meldet (Initialize-Antwort
//! `current_permission_mode`, `system`-Zeilen mit `permissionMode`): Ist er freizügiger als
//! der von beton gesetzte, beendet der Adapter die Session (fail closed).

use beton_harness::PermissionMode;

/// Name des Modus in der CLI (`--permission-mode`, `set_permission_mode`).
pub fn vendor_mode(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Plan => "plan",
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Yolo => "bypassPermissions",
    }
}

/// beton-Modus zu einem Modus der CLI (Umkehrung von [`vendor_mode`]).
pub fn beton_mode(vendor: &str) -> Option<PermissionMode> {
    PermissionMode::ALL
        .into_iter()
        .find(|m| vendor_mode(*m) == vendor)
}

/// Wie viel ein Modus ohne Rückfrage erlaubt; unbekannte Modi gelten als am freizügigsten.
fn rank(vendor: &str) -> u8 {
    match vendor {
        "plan" => 0,
        // `dontAsk` lehnt alles ab, was nicht vorab erlaubt ist, fragt also nie zusätzlich.
        "default" | "manual" | "dontAsk" => 1,
        "acceptEdits" => 2,
        "auto" => 3,
        "bypassPermissions" => 4,
        _ => u8::MAX,
    }
}

/// Welche Modi die CLI melden darf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeGuard {
    /// Zuletzt von der CLI bestätigter Modus (Start oder `set_permission_mode`).
    current: String,
    /// Modus vor `plan`: ein freigegebenes `ExitPlanMode` stellt ihn in der CLI wieder her.
    pre_plan: String,
    /// Gesendete, noch unbestätigte Wechsel (Request-ID, Modus) in Sendereihenfolge.
    pending: std::collections::VecDeque<(String, String)>,
}

impl Default for ModeGuard {
    fn default() -> Self {
        Self::new(vendor_mode(PermissionMode::Default))
    }
}

impl ModeGuard {
    pub fn new(vendor: &str) -> Self {
        Self {
            current: vendor.to_owned(),
            pre_plan: vendor_mode(PermissionMode::Default).to_owned(),
            pending: std::collections::VecDeque::new(),
        }
    }

    /// beton sendet `set_permission_mode` mit `request_id`: Bis die CLI antwortet, darf sie
    /// den alten oder den neuen Modus melden.
    pub fn allow(&mut self, request_id: &str, vendor: &str) {
        self.pending
            .push_back((request_id.to_owned(), vendor.to_owned()));
    }

    /// Antwort der CLI auf einen Wechsel: bei Erfolg gilt der neue Modus, sonst bleibt der alte.
    /// Liefert `false`, wenn `request_id` kein offener Wechsel ist.
    pub fn answered(&mut self, request_id: &str, success: bool) -> bool {
        let Some(pos) = self.pending.iter().position(|(id, _)| id == request_id) else {
            return false;
        };
        // Die CLI antwortet in Reihenfolge; ältere Wechsel sind damit erledigt.
        let mut done: Vec<(String, String)> = self.pending.drain(..=pos).collect();
        if success && let Some((_, mode)) = done.pop() {
            if mode == "plan" && self.current != "plan" {
                self.pre_plan = std::mem::take(&mut self.current);
            }
            self.current = mode;
        }
        true
    }

    /// Der Nutzer hat `ExitPlanMode` freigegeben: Die CLI verlässt `plan` zum Modus davor, den
    /// beton gesetzt hatte (nie freizügiger). Liefert ihn, falls die Session in `plan` war.
    pub fn plan_exit_approved(&mut self) -> Option<String> {
        if self.current != "plan" || !self.pending.is_empty() {
            return None;
        }
        self.current = self.pre_plan.clone();
        Some(self.current.clone())
    }

    fn ceiling(&self) -> u8 {
        self.pending
            .iter()
            .map(|(_, m)| rank(m))
            .fold(rank(&self.current), u8::max)
    }

    /// Prüft einen gemeldeten Modus. `Err` mit Begründung, wenn er freizügiger ist als erlaubt.
    pub fn check(&self, reported: &str) -> Result<(), String> {
        if rank(reported) > self.ceiling() {
            let set = self.pending.back().map_or(&self.current, |(_, m)| m);
            return Err(format!(
                "Claude Code meldet den Permission-Mode `{reported}`, beton hat `{set}` gesetzt"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn har_027_beton_modes_map_to_cli_modes() {
        assert_eq!(vendor_mode(PermissionMode::Plan), "plan");
        assert_eq!(vendor_mode(PermissionMode::Default), "default");
        assert_eq!(vendor_mode(PermissionMode::AcceptEdits), "acceptEdits");
        assert_eq!(vendor_mode(PermissionMode::Yolo), "bypassPermissions");
    }

    #[test]
    fn har_027_more_permissive_report_than_requested_fails_closed() {
        let g = ModeGuard::new("default");
        assert!(g.check("default").is_ok());
        assert!(g.check("plan").is_ok(), "strenger ist erlaubt");
        for worse in ["acceptEdits", "auto", "bypassPermissions", "unbekannt"] {
            assert!(g.check(worse).is_err(), "{worse}");
        }
        assert!(ModeGuard::new("plan").check("default").is_err());
    }

    #[test]
    fn har_027_switch_allows_old_and_new_until_answered() {
        let mut g = ModeGuard::new("plan");
        g.allow("beton_1", "acceptEdits");
        g.allow("beton_2", "default");
        g.allow("beton_3", "plan");
        // Verspätete Meldungen der Zwischenstände sind kein Fehler, solange Wechsel offen sind.
        assert!(g.check("plan").is_ok());
        assert!(g.check("acceptEdits").is_ok());
        assert!(g.answered("beton_1", true));
        assert!(g.check("acceptEdits").is_ok(), "beton_2/3 noch offen");
        assert!(g.answered("beton_3", true));
        assert!(
            g.check("acceptEdits").is_err(),
            "alle Wechsel bestätigt: nur noch plan"
        );
        assert!(!g.answered("beton_9", true));
    }

    #[test]
    fn har_027_approved_plan_exit_returns_to_the_previous_mode_only() {
        let mut g = ModeGuard::new("plan");
        assert_eq!(g.plan_exit_approved().as_deref(), Some("default"));
        assert!(g.check("default").is_ok());
        assert!(g.check("acceptEdits").is_err());
        // Vorher `acceptEdits`, dann `plan`: zurück zu `acceptEdits`, nicht weiter.
        let mut g = ModeGuard::new("default");
        g.allow("beton_1", "acceptEdits");
        g.answered("beton_1", true);
        g.allow("beton_2", "plan");
        g.answered("beton_2", true);
        assert!(g.check("acceptEdits").is_err(), "in plan");
        assert_eq!(g.plan_exit_approved().as_deref(), Some("acceptEdits"));
        assert!(g.check("acceptEdits").is_ok());
        assert!(g.check("bypassPermissions").is_err());
        // Außerhalb von `plan` ändert eine Freigabe nichts.
        assert_eq!(g.plan_exit_approved(), None);
    }

    #[test]
    fn har_027_failed_switch_keeps_the_old_mode() {
        let mut g = ModeGuard::new("default");
        g.allow("beton_1", "acceptEdits");
        assert!(g.answered("beton_1", false));
        assert!(g.check("acceptEdits").is_err());
        assert!(g.check("default").is_ok());
    }
}
