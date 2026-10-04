//! Strukturierte Logs für alle Komponenten (OBS-001).
//!
//! - Daemon, Host, Runner und CLI schreiben JSON-Lines nach `~/.beton/logs/<component>.log`.
//! - Pflichtfelder je Zeile: `ts`, `level`, `target`, `component`, `version`; aus dem
//!   Span-Kontext zusätzlich `session_id`, `runner_id`, `request_id`, `seq`, sofern gesetzt.
//! - Filter über `BETON_LOG` (EnvFilter-Syntax); ohne globale Angabe gilt `info`.
//! - Rotation täglich, Aufbewahrung 7 Tage bzw. 100 MB ([`Retention`]).
//! - Optional zusätzlich JSON auf stdout (Container) und menschenlesbar auf stderr (CLI).
//!
//! Regel für alle Aufrufer: Prompts und Tool-Inhalte werden nie auf `info` oder höher
//! geloggt (OBS-001, AGENTS.md).

mod json;
mod rolling;

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

pub use json::JsonLayer;
pub use rolling::{Clock, Retention, RollingFile, SystemClock};

/// Umgebungsvariable für den Log-Filter.
pub const FILTER_ENV: &str = "BETON_LOG";

/// Komponente, die loggt; bestimmt den Dateinamen und das Feld `component`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Daemon,
    Host,
    Runner,
    Cli,
}

impl Component {
    pub fn as_str(self) -> &'static str {
        match self {
            Component::Daemon => "daemon",
            Component::Host => "host",
            Component::Runner => "runner",
            Component::Cli => "cli",
        }
    }
}

/// Konfiguration für [`init`].
#[derive(Debug, Clone)]
pub struct LogConfig {
    pub component: Component,
    /// Verzeichnis der Log-Dateien, Standard: [`default_log_dir`].
    pub dir: PathBuf,
    /// Filter-Direktiven, Standard: Inhalt von `BETON_LOG`.
    pub filter: Option<String>,
    /// Zusätzlich JSON-Lines auf stdout (zentraler Betrieb im Container).
    pub stdout_json: bool,
    /// Zusätzlich menschenlesbare Ausgabe auf stderr (CLI).
    pub stderr_human: bool,
    pub retention: Retention,
}

impl LogConfig {
    /// Standardkonfiguration für eine Komponente; liest `BETON_LOG` und `BETON_HOME`.
    pub fn for_component(component: Component) -> Self {
        Self {
            component,
            dir: default_log_dir(),
            filter: std::env::var(FILTER_ENV).ok(),
            stdout_json: false,
            stderr_human: component == Component::Cli,
            retention: Retention::default(),
        }
    }
}

/// Hält die Hintergrund-Writer am Leben; beim Drop werden gepufferte Zeilen geschrieben.
#[must_use = "Beim Drop des Guards werden die Log-Writer beendet"]
pub struct LogGuard {
    _guards: Vec<WorkerGuard>,
}

/// `$BETON_HOME/logs`, sonst `~/.beton/logs`.
pub fn default_log_dir() -> PathBuf {
    beton_home().join("logs")
}

/// Datenverzeichnis: `$BETON_HOME`, sonst `~/.beton`.
pub fn beton_home() -> PathBuf {
    if let Some(home) = std::env::var_os("BETON_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    let user_home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    user_home.join(".beton")
}

/// Baut den Filter: `BETON_LOG`-Direktiven, ergänzt um `info` als Standard für alle
/// übrigen Targets. `BETON_LOG=beton_policy=debug` erhöht also nur dieses Target.
pub fn build_filter(directives: Option<&str>) -> anyhow::Result<EnvFilter> {
    let directives = directives.unwrap_or_default().trim();
    // `EnvFilter` schaltet ohne globale Direktive alle übrigen Targets ab; deshalb wird
    // `info` vorangestellt, sofern die Angabe kein eigenes globales Level enthält.
    let has_global = directives.split(',').map(str::trim).any(|d| {
        !d.is_empty() && !d.contains('=') && !d.contains('[') && d.parse::<LevelFilter>().is_ok()
    });
    let directives = if has_global {
        directives.to_owned()
    } else {
        format!("{},{directives}", LevelFilter::INFO)
    };
    EnvFilter::builder()
        .parse(&directives)
        .with_context(|| format!("ungültiger Wert in {FILTER_ENV}"))
}

/// Initialisiert das globale Logging für eine Komponente.
pub fn init(config: LogConfig) -> anyhow::Result<LogGuard> {
    let filter = build_filter(config.filter.as_deref())?;
    let file = RollingFile::open(
        &config.dir,
        config.component.as_str(),
        config.retention,
        SystemClock,
    )
    .with_context(|| format!("Log-Verzeichnis {} nicht nutzbar", config.dir.display()))?;
    let (file_writer, file_guard) = tracing_appender::non_blocking(file);
    let mut guards = vec![file_guard];

    let stdout_layer = if config.stdout_json {
        let (writer, guard) = tracing_appender::non_blocking(std::io::stdout());
        guards.push(guard);
        Some(JsonLayer::new(config.component, writer))
    } else {
        None
    };
    let stderr_layer = config.stderr_human.then(|| {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(false)
            .compact()
    });

    tracing_subscriber::registry()
        .with(filter)
        .with(JsonLayer::new(config.component, file_writer))
        .with(stdout_layer)
        .with(stderr_layer)
        .try_init()
        .context("Logging wurde bereits initialisiert")?;
    Ok(LogGuard { _guards: guards })
}

/// Pfad der aktuellen Log-Datei einer Komponente.
pub fn log_file(dir: &Path, component: Component) -> PathBuf {
    dir.join(format!("{}.log", component.as_str()))
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    /// Sammelt geschriebene Bytes im Speicher.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        fn lines(&self) -> Vec<serde_json::Value> {
            let bytes = self.0.lock().unwrap().clone();
            String::from_utf8(bytes)
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str(l).expect("jede Zeile ist valides JSON"))
                .collect()
        }
    }

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn with_capture(filter: Option<&str>, f: impl FnOnce()) -> Vec<serde_json::Value> {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::registry()
            .with(build_filter(filter).unwrap())
            .with(JsonLayer::new(Component::Daemon, capture.clone()));
        tracing::subscriber::with_default(subscriber, f);
        capture.lines()
    }

    #[test]
    fn obs_001_ac1_json_line_has_required_fields_and_span_context() {
        let lines = with_capture(None, || {
            let session = tracing::info_span!("session", session_id = "ses_123", seq = 7_u64);
            let _s = session.enter();
            let runner = tracing::info_span!("runner", runner_id = "run_9");
            let _r = runner.enter();
            tracing::info!(target: "beton_server", attempt = 2, "Runner gestartet");
        });
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        for key in ["ts", "level", "target", "component", "version", "message"] {
            assert!(line.get(key).is_some(), "Feld `{key}` fehlt in {line}");
        }
        assert_eq!(line["level"], "INFO");
        assert_eq!(line["target"], "beton_server");
        assert_eq!(line["component"], "daemon");
        assert_eq!(line["version"], crate::VERSION);
        assert_eq!(line["message"], "Runner gestartet");
        assert_eq!(line["session_id"], "ses_123");
        assert_eq!(line["runner_id"], "run_9");
        assert_eq!(line["seq"], 7);
        assert_eq!(line["fields"]["attempt"], 2);
        let ts = line["ts"].as_str().unwrap();
        assert!(
            time::OffsetDateTime::parse(ts, &time::format_description::well_known::Rfc3339).is_ok(),
            "ts ist kein RFC 3339: {ts}"
        );
    }

    #[test]
    fn obs_001_ac2_beton_log_raises_only_the_given_target() {
        let lines = with_capture(Some("beton_policy=debug"), || {
            tracing::debug!(target: "beton_policy", "policy debug");
            tracing::debug!(target: "beton_server", "server debug");
            tracing::info!(target: "beton_server", "server info");
        });
        let messages: Vec<_> = lines.iter().map(|l| l["message"].clone()).collect();
        assert_eq!(messages, vec!["policy debug", "server info"]);
    }

    #[test]
    fn obs_001_ac2_global_level_in_beton_log_is_respected() {
        let lines = with_capture(Some("warn,beton_policy=debug"), || {
            tracing::debug!(target: "beton_policy", "policy debug");
            tracing::info!(target: "beton_server", "server info");
            tracing::warn!(target: "beton_server", "server warn");
        });
        let messages: Vec<_> = lines.iter().map(|l| l["message"].clone()).collect();
        assert_eq!(messages, vec!["policy debug", "server warn"]);
    }

    #[test]
    fn obs_001_ac2_invalid_filter_is_reported() {
        assert!(build_filter(Some("beton_policy=lauter")).is_err());
    }

    #[test]
    fn obs_001_ac1_log_file_name_per_component() {
        let dir = Path::new("/tmp/x");
        assert_eq!(log_file(dir, Component::Daemon), dir.join("daemon.log"));
        assert_eq!(log_file(dir, Component::Runner), dir.join("runner.log"));
    }
}
