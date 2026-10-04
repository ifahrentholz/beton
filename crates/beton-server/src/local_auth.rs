//! Lokale Authentisierung (AUTH-001, AUTH-004).
//!
//! - Token-Datei `<data_dir>/auth/local.token` (256 bit, Datei 0600, Verzeichnis 0700). Der
//!   Start bricht ab, wenn Rechte zu weit sind oder Datei/Verzeichnis einem anderen User
//!   gehören.
//! - Einmal-Codes (128 bit, 60 s, single-use) für den Browser-Login, eingelöst gegen ein
//!   Session-Cookie. Codes leben nur im Speicher; Browser-Sessions stehen zusätzlich in
//!   `<data_dir>/auth/browser-sessions.json` (0600), damit sie einen Daemon-Neustart
//!   überleben (AUTH-004 AC4).
//!
//! Tokens, Codes und Cookies werden nur als SHA-256 gehalten bzw. gespeichert und nie geloggt.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Gültigkeit eines Einmal-Codes (AUTH-004).
pub const CODE_TTL: Duration = Duration::from_secs(60);
/// Gültigkeit eines Browser-Session-Cookies auf dem Server.
pub const COOKIE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Name des Session-Cookies.
pub const COOKIE_NAME: &str = "beton_session";

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error(
        "{path} hat die Rechte {mode:o}; erlaubt ist nur der Eigentümer ({expected:o}). \
         Abhilfe: chmod {expected:o} {path}"
    )]
    InsecurePermissions {
        path: String,
        mode: u32,
        expected: u32,
    },
    #[error("{path} gehört einem anderen Benutzer (uid {owner}); erwartet uid {current}")]
    ForeignOwner {
        path: String,
        owner: u32,
        current: u32,
    },
    #[error("{path} enthält kein gültiges Token")]
    Malformed { path: String },
    #[error("Token-Datei {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("Zufallsquelle nicht verfügbar")]
    Random,
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> AuthError + '_ {
    move |source| AuthError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// `n` zufällige Bytes als Hex.
pub fn random_hex(n: usize) -> Result<String, AuthError> {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).map_err(|_| AuthError::Random)?;
    Ok(hex::encode(bytes))
}

fn sha256(s: &str) -> [u8; 32] {
    Sha256::digest(s.as_bytes()).into()
}

/// Das lokale Token.
pub struct LocalToken {
    path: PathBuf,
    hash: Mutex<[u8; 32]>,
}

impl std::fmt::Debug for LocalToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalToken")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl LocalToken {
    pub fn path_in(data_dir: &Path) -> PathBuf {
        data_dir.join("auth").join("local.token")
    }

    /// Lädt das Token oder legt es an. Prüft Rechte und Eigentümer (AUTH-001 AC2).
    pub fn load_or_create(data_dir: &Path) -> Result<Self, AuthError> {
        let path = Self::path_in(data_dir);
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        if !dir.exists() {
            create_private_dir(&dir)?;
        }
        check_private(&dir, 0o700)?;
        if !path.exists() {
            write_private(&path, &random_hex(32)?)?;
        }
        check_private(&path, 0o600)?;
        let token = read_token(&path)?;
        Ok(Self {
            hash: Mutex::new(sha256(&token)),
            path,
        })
    }

    /// Erneuert das Token (`beton auth rotate-local`); laufende Clients lesen die Datei neu.
    pub fn rotate(&self) -> Result<(), AuthError> {
        let token = random_hex(32)?;
        let tmp = self.path.with_extension("token.new");
        let _ = std::fs::remove_file(&tmp);
        write_private(&tmp, &token)?;
        std::fs::rename(&tmp, &self.path).map_err(io_err(&self.path))?;
        if let Ok(mut h) = self.hash.lock() {
            *h = sha256(&token);
        }
        Ok(())
    }

    /// Vergleich in konstanter Zeit.
    pub fn verify(&self, candidate: &str) -> bool {
        let candidate = sha256(candidate.trim());
        self.hash
            .lock()
            .map(|h| bool::from(h.ct_eq(&candidate)))
            .unwrap_or(false)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn read_token(path: &Path) -> Result<String, AuthError> {
    let token = std::fs::read_to_string(path).map_err(io_err(path))?;
    let token = token.trim().to_owned();
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AuthError::Malformed {
            path: path.display().to_string(),
        });
    }
    Ok(token)
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> Result<(), AuthError> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(io_err(dir))
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> Result<(), AuthError> {
    std::fs::create_dir_all(dir).map_err(io_err(dir))
}

#[cfg(unix)]
fn write_private(path: &Path, content: &str) -> Result<(), AuthError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(io_err(path))?;
    f.write_all(content.as_bytes()).map_err(io_err(path))?;
    f.sync_all().map_err(io_err(path))
}

#[cfg(not(unix))]
fn write_private(path: &Path, content: &str) -> Result<(), AuthError> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_err(path))?;
    f.write_all(content.as_bytes()).map_err(io_err(path))?;
    f.sync_all().map_err(io_err(path))
}

/// Datei bzw. Verzeichnis darf nur dem aktuellen User gehören und keine Gruppen-/Fremdrechte
/// haben.
#[cfg(unix)]
pub(crate) fn check_private(path: &Path, expected: u32) -> Result<(), AuthError> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).map_err(io_err(path))?;
    let current = rustix::process::getuid().as_raw();
    if meta.uid() != current {
        return Err(AuthError::ForeignOwner {
            path: path.display().to_string(),
            owner: meta.uid(),
            current,
        });
    }
    let mode = meta.mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(AuthError::InsecurePermissions {
            path: path.display().to_string(),
            mode,
            expected,
        });
    }
    Ok(())
}

/// Windows: ACL-Prüfung folgt (Windows ist Beta); das Profilverzeichnis ist per Default privat.
#[cfg(not(unix))]
pub(crate) fn check_private(path: &Path, _expected: u32) -> Result<(), AuthError> {
    std::fs::metadata(path).map_err(io_err(path))?;
    Ok(())
}

/// Einmal-Codes (nur im Speicher) und Browser-Sessions (optional dauerhaft), nur als Hashes.
#[derive(Debug, Default)]
pub struct BrowserLogins {
    codes: Mutex<HashMap<[u8; 32], SystemTime>>,
    sessions: Mutex<HashMap<[u8; 32], SystemTime>>,
    /// Datei der Browser-Sessions; `None` hält sie nur im Speicher (Tests).
    file: Option<PathBuf>,
}

/// Inhalt von `browser-sessions.json`: Cookie-Hashes mit Ablauf in Unix-Sekunden.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct SessionFile {
    v: u32,
    sessions: Vec<StoredSession>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredSession {
    sha256: String,
    expires: u64,
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl BrowserLogins {
    pub fn path_in(data_dir: &Path) -> PathBuf {
        data_dir.join("auth").join("browser-sessions.json")
    }

    /// Lädt die gespeicherten Browser-Sessions. Zu weite Rechte verhindern den Start wie bei
    /// der Token-Datei; eine unlesbare oder kaputte Datei meldet niemanden an.
    pub fn persistent(data_dir: &Path) -> Result<Self, AuthError> {
        let path = Self::path_in(data_dir);
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        if !dir.exists() {
            create_private_dir(&dir)?;
        }
        check_private(&dir, 0o700)?;
        let mut sessions = HashMap::new();
        if path.exists() {
            check_private(&path, 0o600)?;
            let stored: SessionFile = std::fs::read_to_string(&path)
                .ok()
                .and_then(|c| serde_json::from_str(&c).ok())
                .unwrap_or_else(|| {
                    tracing::warn!(
                        "Browser-Sessions nicht lesbar; alle Browser müssen sich neu anmelden"
                    );
                    SessionFile::default()
                });
            for s in stored.sessions {
                let mut hash = [0u8; 32];
                if hex::decode_to_slice(&s.sha256, &mut hash).is_ok() {
                    sessions.insert(hash, UNIX_EPOCH + Duration::from_secs(s.expires));
                }
            }
        }
        Ok(Self {
            codes: Mutex::default(),
            sessions: Mutex::new(sessions),
            file: Some(path),
        })
    }

    /// Neuer Einmal-Code (128 bit), gültig für 60 s ab `now`.
    pub fn issue_code(&self, now: SystemTime) -> Result<String, AuthError> {
        let code = random_hex(16)?;
        if let Ok(mut codes) = self.codes.lock() {
            codes.retain(|_, expires| *expires > now);
            codes.insert(sha256(&code), now + CODE_TTL);
        }
        Ok(code)
    }

    /// Löst einen Code ein (genau einmal, innerhalb der Frist) und liefert den Cookie-Wert.
    /// Lässt sich die Session nicht speichern, gibt es kein Cookie (fail closed).
    pub fn redeem(&self, code: &str, now: SystemTime) -> Option<String> {
        let expires = self.codes.lock().ok()?.remove(&sha256(code))?;
        if now >= expires {
            return None;
        }
        let cookie = random_hex(32).ok()?;
        let mut sessions = self.sessions.lock().ok()?;
        sessions.retain(|_, e| *e > now);
        sessions.insert(sha256(&cookie), now + COOKIE_TTL);
        if let Some(path) = &self.file
            && let Err(e) = save_sessions(path, &sessions)
        {
            tracing::warn!("Browser-Session nicht gespeichert: {e}");
            sessions.remove(&sha256(&cookie));
            return None;
        }
        Some(cookie)
    }

    pub fn verify_cookie(&self, cookie: &str, now: SystemTime) -> bool {
        self.sessions
            .lock()
            .ok()
            .and_then(|s| s.get(&sha256(cookie)).copied())
            .is_some_and(|expires| now < expires)
    }
}

/// Schreibt die Sessions atomar (temporäre Datei 0600, dann `rename`).
fn save_sessions(path: &Path, sessions: &HashMap<[u8; 32], SystemTime>) -> Result<(), AuthError> {
    let file = SessionFile {
        v: 1,
        sessions: sessions
            .iter()
            .map(|(hash, expires)| StoredSession {
                sha256: hex::encode(hash),
                expires: unix_secs(*expires),
            })
            .collect(),
    };
    let json = serde_json::to_string(&file).map_err(|e| AuthError::Io {
        path: path.display().to_string(),
        source: std::io::Error::other(e),
    })?;
    let tmp = path.with_extension("json.new");
    let _ = std::fs::remove_file(&tmp);
    write_private(&tmp, &json)?;
    std::fs::rename(&tmp, path).map_err(io_err(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_001_token_file_is_created_private_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let token = LocalToken::load_or_create(dir.path()).unwrap();
        let content = std::fs::read_to_string(token.path()).unwrap();
        assert_eq!(content.len(), 64, "256 bit als Hex");
        assert!(token.verify(&content));
        assert!(!token.verify("falsch"));
        assert!(!token.verify(""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(token.path()), 0o600);
            assert_eq!(mode(token.path().parent().unwrap()), 0o700);
        }
        // Zweiter Start liest dasselbe Token.
        let again = LocalToken::load_or_create(dir.path()).unwrap();
        assert!(again.verify(&content));
    }

    #[cfg(unix)]
    #[test]
    fn auth_001_ac2_world_readable_token_refuses_start() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let token = LocalToken::load_or_create(dir.path()).unwrap();
        std::fs::set_permissions(token.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = LocalToken::load_or_create(dir.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            matches!(err, AuthError::InsecurePermissions { mode: 0o644, .. }),
            "{msg}"
        );
        assert!(msg.contains("chmod 600"), "{msg}");

        std::fs::set_permissions(token.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
        let auth_dir = token.path().parent().unwrap();
        std::fs::set_permissions(auth_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            LocalToken::load_or_create(dir.path()).unwrap_err(),
            AuthError::InsecurePermissions { mode: 0o755, .. }
        ));
    }

    #[test]
    fn rotation_invalidates_old_token() {
        let dir = tempfile::tempdir().unwrap();
        let token = LocalToken::load_or_create(dir.path()).unwrap();
        let old = std::fs::read_to_string(token.path()).unwrap();
        token.rotate().unwrap();
        let new = std::fs::read_to_string(token.path()).unwrap();
        assert_ne!(old, new);
        assert!(!token.verify(&old));
        assert!(token.verify(&new));
        assert!(
            LocalToken::load_or_create(dir.path()).is_ok(),
            "Rechte nach Rotation"
        );
    }

    #[test]
    fn auth_004_ac1_code_is_single_use_and_expires() {
        let logins = BrowserLogins::default();
        let t0 = SystemTime::now();
        let code = logins.issue_code(t0).unwrap();
        assert_eq!(code.len(), 32, "128 bit");
        let cookie = logins.redeem(&code, t0 + Duration::from_secs(5)).unwrap();
        assert!(logins.verify_cookie(&cookie, t0 + Duration::from_secs(6)));
        assert!(
            logins.redeem(&code, t0 + Duration::from_secs(6)).is_none(),
            "zweites Einlösen"
        );

        let late = logins.issue_code(t0).unwrap();
        assert!(logins.redeem(&late, t0 + CODE_TTL).is_none(), "nach 60 s");
        assert!(logins.redeem("unbekannt", t0).is_none());
        assert!(!logins.verify_cookie(&cookie, t0 + COOKIE_TTL + Duration::from_secs(6)));
    }

    #[test]
    fn auth_004_ac4_cookie_survives_daemon_restart() {
        let dir = tempfile::tempdir().unwrap();
        let t0 = SystemTime::now();
        let cookie = {
            let logins = BrowserLogins::persistent(dir.path()).unwrap();
            let code = logins.issue_code(t0).unwrap();
            logins.redeem(&code, t0).unwrap()
        };
        // Neuer Prozess: gleiche Datei, Cookie gilt weiter, aber nicht über die 24 h hinaus.
        let logins = BrowserLogins::persistent(dir.path()).unwrap();
        assert!(logins.verify_cookie(&cookie, t0 + Duration::from_secs(3600)));
        assert!(!logins.verify_cookie(&cookie, t0 + COOKIE_TTL));
        assert!(!logins.verify_cookie("unbekannt", t0));
        // Einmal-Codes überleben keinen Neustart.
        let before = BrowserLogins::persistent(dir.path()).unwrap();
        let code = before.issue_code(t0).unwrap();
        drop(before);
        let after = BrowserLogins::persistent(dir.path()).unwrap();
        assert!(after.redeem(&code, t0).is_none());
    }

    #[test]
    fn auth_004_ac4_session_file_is_private_and_holds_only_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let logins = BrowserLogins::persistent(dir.path()).unwrap();
        let t0 = SystemTime::now();
        let code = logins.issue_code(t0).unwrap();
        let cookie = logins.redeem(&code, t0).unwrap();
        let path = BrowserLogins::path_in(dir.path());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains(&cookie), "kein Klartext-Cookie");
        assert!(!content.contains(&code), "kein Einmal-Code");
        assert!(content.contains(&hex::encode(sha256(&cookie))));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn auth_004_ac4_readable_session_file_refuses_start() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let logins = BrowserLogins::persistent(dir.path()).unwrap();
        let t0 = SystemTime::now();
        let code = logins.issue_code(t0).unwrap();
        logins.redeem(&code, t0).unwrap();
        let path = BrowserLogins::path_in(dir.path());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            BrowserLogins::persistent(dir.path()).unwrap_err(),
            AuthError::InsecurePermissions { mode: 0o644, .. }
        ));
    }

    #[test]
    fn auth_004_ac4_damaged_or_expired_entries_grant_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let t0 = SystemTime::now();
        let cookie = {
            let logins = BrowserLogins::persistent(dir.path()).unwrap();
            let code = logins.issue_code(t0).unwrap();
            logins.redeem(&code, t0).unwrap()
        };
        let path = BrowserLogins::path_in(dir.path());
        // Kaputte Datei: niemand ist angemeldet, neue Anmeldungen funktionieren.
        std::fs::write(&path, "{kein json").unwrap();
        let logins = BrowserLogins::persistent(dir.path()).unwrap();
        assert!(!logins.verify_cookie(&cookie, t0));
        let code = logins.issue_code(t0).unwrap();
        let fresh = logins.redeem(&code, t0).unwrap();
        assert!(
            BrowserLogins::persistent(dir.path())
                .unwrap()
                .verify_cookie(&fresh, t0)
        );
        // Abgelaufene Einträge werden beim nächsten Schreiben entfernt.
        let later = t0 + COOKIE_TTL + Duration::from_secs(1);
        let code = logins.issue_code(later).unwrap();
        logins.redeem(&code, later).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            !content.contains(&hex::encode(sha256(&fresh))),
            "abgelaufen entfernt"
        );
    }
}
