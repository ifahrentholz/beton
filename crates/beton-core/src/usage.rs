//! Subscription-Usage (USE-004): gemeinsame Namen der Abo-Fenster und die Schwelle für den
//! Inbox-Hinweis.
//!
//! Die Vendor-CLIs benennen ihre Fenster unterschiedlich: Claude Code nach Art
//! (`five_hour`, `seven_day`, `seven_day_opus`), Codex nach Länge in Minuten (300, 10080).
//! beton führt beide auf eine Schreibweise zurück: `5h`, `7d`, `7d_opus`.

/// Ab diesem genutzten Anteil (Prozent) eines Abo-Fensters erzeugt beton einen Inbox-Hinweis
/// (USE-004 AC3).
pub const RATE_LIMIT_NOTICE_PCT: f64 = 90.0;

/// Länge in Minuten als Fenstername: `300` → `5h`, `10080` → `7d`, `90` → `90m`.
pub fn window_from_minutes(minutes: u64) -> String {
    match minutes {
        0 => "0m".into(),
        m if m % 1440 == 0 => format!("{}d", m / 1440),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{m}m"),
    }
}

/// Einheitlicher Fenstername aus der Schreibweise einer CLI; auch für Events, die vor der
/// Vereinheitlichung gespeichert wurden (`five_hour`, `300m`). Unbekanntes bleibt.
pub fn window_name(raw: &str) -> String {
    for (prefix, short) in [("five_hour", "5h"), ("seven_day", "7d")] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            return format!("{short}{rest}");
        }
    }
    match raw.strip_suffix('m').map(str::parse::<u64>) {
        Some(Ok(minutes)) => window_from_minutes(minutes),
        _ => raw.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_004_window_names_are_unified() {
        assert_eq!(window_name("five_hour"), "5h");
        assert_eq!(window_name("seven_day"), "7d");
        assert_eq!(window_name("seven_day_opus"), "7d_opus");
        assert_eq!(window_name("300m"), "5h");
        assert_eq!(window_name("10080m"), "7d");
        assert_eq!(window_name("90m"), "90m");
        assert_eq!(window_name("5h"), "5h");
        assert_eq!(window_name("overage"), "overage");
        assert_eq!(window_from_minutes(1440), "1d");
    }
}
