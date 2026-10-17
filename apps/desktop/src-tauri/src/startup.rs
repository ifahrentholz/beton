//! Startbildschirm (DESK-002, Entwurf `desktop-daemon` Zustände „Start“ und „Daemon
//! inkompatibel“): Schritte aus dem Fortschritt des Daemon-Starts.

use beton_desktop::daemon::{Step, Versions};
use serde::Serialize;

#[derive(Debug, Clone, Default)]
pub struct Startup {
    /// Version des mitgelieferten Binarys.
    pub sidecar: Option<String>,
    /// `Some(true)`: Daemon lief schon; `Some(false)`: App startet einen.
    pub existing: Option<bool>,
    pub connected: bool,
    pub error: Option<String>,
    pub incompatible: Option<Versions>,
    pub upgrading: bool,
    /// Sessions, auf die ein Upgrade wartet.
    pub waiting: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepView {
    pub text: String,
    /// `done`, `active`, `todo` oder `failed`.
    pub state: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IncompatibleView {
    pub daemon: String,
    pub daemon_protocol: String,
    pub app: String,
    pub app_protocol: String,
    pub upgrading: bool,
    pub waiting: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupView {
    pub home: String,
    /// „beton startet“ bzw. „beton startet nicht“.
    pub title: &'static str,
    /// Zustand „Start fehlgeschlagen“: Erneut versuchen, Logs anzeigen, Beenden.
    pub failed: bool,
    pub steps: Vec<StepView>,
    pub incompatible: Option<IncompatibleView>,
}

fn step(text: impl Into<String>, state: &'static str) -> StepView {
    StepView {
        text: text.into(),
        state,
    }
}

impl Startup {
    /// Neuer Versuch nach einem fehlgeschlagenen Start.
    pub fn retry(&mut self) {
        self.error = None;
        self.existing = None;
        self.connected = false;
    }

    pub fn apply(&mut self, s: &Step) {
        match s {
            Step::SidecarFound { version } => self.sidecar = Some(version.clone()),
            Step::Starting => self.existing = Some(false),
            Step::Connected { started } => {
                self.existing.get_or_insert(!started);
                self.connected = true;
                self.error = None;
            }
        }
    }

    pub fn view(&self, home: &str) -> StartupView {
        let mut steps = Vec::new();
        if let Some(v) = &self.sidecar {
            steps.push(step(format!("beton {v} gefunden (mitgeliefert)"), "done"));
        }
        steps.push(match self.existing {
            Some(false) => step("Kein laufender Daemon – starte einen neuen", "done"),
            Some(true) => step("Laufenden Daemon gefunden", "done"),
            None if self.incompatible.is_some() => step("Laufenden Daemon gefunden", "done"),
            None if self.error.is_some() => step("Kein laufender Daemon", "done"),
            None => step("Suche laufenden Daemon", "active"),
        });
        steps.push(match (&self.error, self.connected) {
            (Some(e), _) => step(e.clone(), "failed"),
            (None, true) => step(
                "Lokaler Daemon auf 127.0.0.1, nur von diesem Rechner erreichbar",
                "done",
            ),
            (None, false) if self.incompatible.is_some() => step(
                "Lokaler Daemon auf 127.0.0.1, nur von diesem Rechner erreichbar",
                "todo",
            ),
            (None, false) => step(
                "Lokaler Daemon auf 127.0.0.1, nur von diesem Rechner erreichbar",
                "active",
            ),
        });
        steps.push(step(
            "Profil „local“ öffnen",
            if self.connected { "active" } else { "todo" },
        ));
        StartupView {
            home: home.to_owned(),
            title: if self.error.is_some() {
                "beton startet nicht"
            } else {
                "beton startet"
            },
            failed: self.error.is_some(),
            steps,
            incompatible: self.incompatible.as_ref().map(|v| IncompatibleView {
                daemon: v.daemon.clone(),
                daemon_protocol: format!("{}", v.daemon_protocol),
                app: v.app.clone(),
                app_protocol: format!("{}", v.app_protocol),
                upgrading: self.upgrading,
                waiting: self.waiting,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states(s: &Startup) -> Vec<&'static str> {
        s.view("~/.beton/").steps.iter().map(|s| s.state).collect()
    }

    #[test]
    fn desk_002_ac1_start_screen_walks_through_the_steps_to_connected() {
        let mut s = Startup::default();
        s.apply(&Step::SidecarFound {
            version: "0.1.0".into(),
        });
        assert_eq!(states(&s), ["done", "active", "active", "todo"]);
        s.apply(&Step::Starting);
        let v = s.view("~/.beton/");
        assert_eq!(v.steps[0].text, "beton 0.1.0 gefunden (mitgeliefert)");
        assert_eq!(
            v.steps[1].text,
            "Kein laufender Daemon – starte einen neuen"
        );
        s.apply(&Step::Connected { started: true });
        assert_eq!(states(&s), ["done", "done", "done", "active"]);
    }

    #[test]
    fn desk_002_ac2_existing_daemon_is_named_as_such() {
        let mut s = Startup::default();
        s.apply(&Step::Connected { started: false });
        assert_eq!(s.view("").steps[0].text, "Laufenden Daemon gefunden");
    }

    #[test]
    fn desk_002_ac3_incompatible_daemon_shows_the_dialog_instead_of_an_error() {
        let s = Startup {
            incompatible: Some(Versions {
                app: "0.3.2".into(),
                app_protocol: 1,
                daemon: "0.1.0".into(),
                daemon_protocol: 1,
            }),
            ..Startup::default()
        };
        let v = s.view("");
        assert!(!v.steps.iter().any(|s| s.state == "failed"));
        let inc = v.incompatible.unwrap();
        assert_eq!((inc.daemon.as_str(), inc.app.as_str()), ("0.1.0", "0.3.2"));
    }

    #[test]
    fn desk_002_start_failure_offers_retry_logs_and_quit() {
        let mut s = Startup {
            sidecar: Some("0.1.0".into()),
            existing: Some(false),
            error: Some("Daemon-Start fehlgeschlagen: Port 7420 ist belegt".into()),
            ..Startup::default()
        };
        let v = s.view("~/.beton/");
        assert!(v.failed, "Zustand „Start fehlgeschlagen“");
        assert_eq!(v.title, "beton startet nicht");
        assert_eq!(states(&s), ["done", "done", "failed", "todo"]);
        s.retry();
        let v = s.view("~/.beton/");
        assert!(!v.failed);
        assert_eq!(v.title, "beton startet");
        assert_eq!(states(&s), ["done", "active", "active", "todo"]);
    }

    #[test]
    fn desk_002_start_failure_is_shown_in_the_step_list() {
        let s = Startup {
            error: Some("Daemon-Start fehlgeschlagen: Port belegt".into()),
            ..Startup::default()
        };
        let v = s.view("");
        assert!(
            v.steps
                .iter()
                .any(|s| s.state == "failed" && s.text.contains("Port belegt"))
        );
    }
}
