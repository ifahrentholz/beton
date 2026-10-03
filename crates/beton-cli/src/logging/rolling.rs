//! Log-Datei mit täglicher Rotation und Aufbewahrungsgrenzen (OBS-001).
//!
//! Die aktuelle Datei heißt immer `<prefix>.log`. Beim Tageswechsel oder wenn sie
//! `max_file_bytes` überschreiten würde, wird sie nach `<prefix>.<YYYY-MM-DD>.<n>.log`
//! umbenannt. Danach werden archivierte Dateien gelöscht, die älter als `max_age_days`
//! sind, und – älteste zuerst – so lange weitere, bis alle Dateien zusammen höchstens
//! `max_total_bytes` belegen.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use time::{Date, Duration, OffsetDateTime};

/// Aufbewahrungsgrenzen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub max_age_days: u32,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
}

impl Default for Retention {
    /// 7 Tage bzw. 100 MB insgesamt (inklusive der aktuellen Datei); eine einzelne Datei
    /// wird ab 20 MB rotiert.
    fn default() -> Self {
        Self {
            max_age_days: 7,
            max_total_bytes: 100 * 1024 * 1024,
            max_file_bytes: 20 * 1024 * 1024,
        }
    }
}

/// Zeitquelle, in Tests ersetzbar.
pub trait Clock: Send + 'static {
    fn now(&self) -> OffsetDateTime;
}

/// Systemzeit in UTC.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

/// Rotierende Log-Datei; implementiert [`Write`].
pub struct RollingFile<C: Clock = SystemClock> {
    dir: PathBuf,
    prefix: String,
    retention: Retention,
    clock: C,
    file: File,
    date: Date,
    size: u64,
}

impl<C: Clock> RollingFile<C> {
    /// Öffnet bzw. erstellt `<dir>/<prefix>.log`. Stammt eine vorhandene Datei von
    /// einem früheren Tag, wird sie beim ersten Schreiben rotiert.
    pub fn open(dir: &Path, prefix: &str, retention: Retention, clock: C) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(format!("{prefix}.log"));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let meta = file.metadata()?;
        let date = meta
            .modified()
            .map(|m| OffsetDateTime::from(m).date())
            .unwrap_or_else(|_| clock.now().date());
        let mut rolling = Self {
            dir: dir.to_path_buf(),
            prefix: prefix.to_owned(),
            retention,
            clock,
            file,
            date,
            size: meta.len(),
        };
        rolling.prune()?;
        Ok(rolling)
    }

    fn current_path(&self) -> PathBuf {
        self.dir.join(format!("{}.log", self.prefix))
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        // Fortlaufende Nummer pro Tag; gelöschte Nummern werden nicht wiederverwendet,
        // damit die Reihenfolge der Archive immer der Entstehung entspricht.
        let n = self
            .archived()?
            .iter()
            .filter(|(date, ..)| *date == self.date)
            .map(|(_, n, ..)| n + 1)
            .max()
            .unwrap_or(0);
        let target = self
            .dir
            .join(format!("{}.{}.{n}.log", self.prefix, self.date));
        fs::rename(self.current_path(), target)?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.current_path())?;
        self.size = 0;
        self.date = self.clock.now().date();
        self.prune()
    }

    /// Archivierte Dateien als `(Datum, Nummer, Pfad, Größe)`, älteste zuerst.
    fn archived(&self) -> io::Result<Vec<(Date, u32, PathBuf, u64)>> {
        let mut files = Vec::new();
        let format = time::macros::format_description!("[year]-[month]-[day]");
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(rest) = name
                .strip_prefix(&self.prefix)
                .and_then(|r| r.strip_prefix('.'))
                .and_then(|r| r.strip_suffix(".log"))
            else {
                continue;
            };
            let Some((date, n)) = rest.rsplit_once('.') else {
                continue;
            };
            let (Ok(date), Ok(n)) = (Date::parse(date, &format), n.parse::<u32>()) else {
                continue;
            };
            files.push((date, n, entry.path(), entry.metadata()?.len()));
        }
        files.sort_by_key(|f| (f.0, f.1));
        Ok(files)
    }

    fn prune(&mut self) -> io::Result<()> {
        let oldest_kept =
            self.clock.now().date() - Duration::days(self.retention.max_age_days.into());
        let mut kept = Vec::new();
        for (date, _, path, size) in self.archived()? {
            if date < oldest_kept {
                fs::remove_file(path)?;
            } else {
                kept.push((path, size));
            }
        }
        // Platz für die aktuelle Datei bis zu ihrer Rotationsgrenze freihalten, damit alle
        // Dateien zusammen `max_total_bytes` nie überschreiten.
        let reserved = self.retention.max_file_bytes.max(self.size);
        let budget = self.retention.max_total_bytes.saturating_sub(reserved);
        let mut archived_total: u64 = kept.iter().map(|(_, s)| s).sum();
        for (path, size) in kept {
            if archived_total <= budget {
                break;
            }
            fs::remove_file(path)?;
            archived_total -= size;
        }
        Ok(())
    }
}

impl<C: Clock> Write for RollingFile<C> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let new_day = self.clock.now().date() != self.date;
        let too_big = self.size > 0 && self.size + buf.len() as u64 > self.retention.max_file_bytes;
        if new_day || too_big {
            self.rotate()?;
        }
        self.file.write_all(buf)?;
        self.size += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use time::macros::datetime;

    use super::*;

    #[derive(Clone)]
    struct FakeClock(Arc<Mutex<OffsetDateTime>>);

    impl FakeClock {
        fn new(start: OffsetDateTime) -> Self {
            Self(Arc::new(Mutex::new(start)))
        }
        fn advance_days(&self, days: i64) {
            *self.0.lock().unwrap() += Duration::days(days);
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> OffsetDateTime {
            *self.0.lock().unwrap()
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn obs_001_ac3_rotates_daily_and_removes_files_older_than_7_days() {
        let tmp = tempfile::tempdir().unwrap();
        // Startdatum in der Zukunft, damit die mtime der neuen Datei „gestern“ ist.
        let clock = FakeClock::new(datetime!(2099-01-01 12:00 UTC));
        let mut log =
            RollingFile::open(tmp.path(), "daemon", Retention::default(), clock.clone()).unwrap();

        for _ in 0..10 {
            log.write_all(b"{\"msg\":1}\n").unwrap();
            clock.advance_days(1);
        }
        log.write_all(b"{\"msg\":2}\n").unwrap();

        let names = names(tmp.path());
        assert!(names.contains(&"daemon.log".to_owned()));
        let archived: Vec<_> = names.iter().filter(|n| *n != "daemon.log").collect();
        // heute = 2099-01-11; aufbewahrt werden Archive ab 2099-01-04 (7 Tage)
        assert_eq!(
            archived.first().map(|s| s.as_str()),
            Some("daemon.2099-01-04.0.log")
        );
        assert_eq!(
            archived.last().map(|s| s.as_str()),
            Some("daemon.2099-01-10.0.log")
        );
        assert_eq!(archived.len(), 7);
    }

    #[test]
    fn obs_001_ac3_removes_oldest_files_when_total_size_exceeds_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::new(datetime!(2099-01-01 12:00 UTC));
        let retention = Retention {
            max_age_days: 7,
            max_total_bytes: 250,
            max_file_bytes: 100,
        };
        let mut log = RollingFile::open(tmp.path(), "runner", retention, clock).unwrap();
        let line = [b'x'; 60];
        for _ in 0..10 {
            log.write_all(&line).unwrap();
        }
        let total: u64 = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().metadata().unwrap().len())
            .sum();
        assert!(total <= 250, "insgesamt {total} Bytes");
        let names = names(tmp.path());
        assert!(names.contains(&"runner.log".to_owned()));
        // Die jüngsten Archive bleiben erhalten, die ältesten wurden entfernt.
        assert!(!names.contains(&"runner.2099-01-01.0.log".to_owned()));
        assert!(names.contains(&"runner.2099-01-01.8.log".to_owned()));
    }

    #[test]
    fn obs_001_ac3_ignores_unrelated_files() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("host.2000-01-01.0.log"), b"fremd").unwrap();
        fs::write(tmp.path().join("notes.txt"), b"x").unwrap();
        let clock = FakeClock::new(datetime!(2099-01-01 12:00 UTC));
        let _log = RollingFile::open(tmp.path(), "daemon", Retention::default(), clock).unwrap();
        let names = names(tmp.path());
        assert!(names.contains(&"host.2000-01-01.0.log".to_owned()));
        assert!(names.contains(&"notes.txt".to_owned()));
    }
}
