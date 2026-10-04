//! Lokale Authentisierung (AUTH-001, AUTH-004).
//!
//! - Token-Datei `<data_dir>/auth/local.token` (256 bit, Datei 0600, Verzeichnis 0700). Der
//!   Start bricht ab, wenn Rechte zu weit sind oder Datei/Verzeichnis einem anderen User
//!   gehören.
//! - Einmal-Codes (128 bit, 60 s, single-use) für den Browser-Login, eingelöst gegen ein
//!   Session-Cookie.
//!
//! Tokens, Codes und Cookies werden nur gehasht im Speicher gehalten und nie geloggt.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

/// Das lokale Token. Die Datei ist die Quelle der Wahrheit: Ändert sie sich (z. B. durch
/// `beton auth rotate-local` in einem anderen Prozess), liest `verify` sie neu ein. Fehlt sie,
/// ist sie unlesbar oder sind ihre Rechte zu weit, wird jedes Token abgelehnt (fail closed).
pub struct LocalToken {
    path: PathBuf,
    state: Mutex<TokenState>,
}

struct TokenState {
    /// `None` = derzeit kein gültiges Token.
    hash: Option<[u8; 32]>,
    stamp: Option<FileStamp>,
}

/// Merkmale der Datei, an denen eine Änderung erkannt wird.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    modified: Option<std::time::SystemTime>,
    len: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(unix)]
    ctime_ns: i64,
}

fn stamp_of(path: &Path) -> Option<FileStamp> {
    let m = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Some(FileStamp {
        modified: m.modified().ok(),
        len: m.len(),
        #[cfg(unix)]
        ino: m.ino(),
        #[cfg(unix)]
        ctime_ns: m.ctime().saturating_mul(1_000_000_000) + m.ctime_nsec(),
    })
}

/// Liest das Token mit Rechteprüfung von Datei und Verzeichnis.
fn load_checked(path: &Path) -> Result<String, AuthError> {
    if let Some(dir) = path.parent() {
        check_private(dir, 0o700)?;
    }
    check_private(path, 0o600)?;
    read_token(path)
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
        let stamp = stamp_of(&path);
        let token = load_checked(&path)?;
        Ok(Self {
            state: Mutex::new(TokenState {
                hash: Some(sha256(&token)),
                stamp,
            }),
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
        if let Ok(mut st) = self.state.lock() {
            st.hash = Some(sha256(&token));
            st.stamp = stamp_of(&self.path);
        }
        Ok(())
    }

    /// Vergleich in konstanter Zeit gegen den aktuellen Inhalt der Token-Datei.
    pub fn verify(&self, candidate: &str) -> bool {
        let candidate = sha256(candidate.trim());
        let Ok(mut st) = self.state.lock() else {
            return false;
        };
        let stamp = stamp_of(&self.path);
        if stamp != st.stamp {
            st.hash = match &stamp {
                Some(_) => match load_checked(&self.path) {
                    Ok(token) => Some(sha256(&token)),
                    Err(e) => {
                        tracing::warn!("lokales Token abgelehnt: {e}");
                        None
                    }
                },
                None => None,
            };
            st.stamp = stamp;
        }
        st.hash
            .as_ref()
            .is_some_and(|h| bool::from(h.ct_eq(&candidate)))
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

/// Einmal-Codes und Browser-Sessions im Speicher (nur Hashes).
#[derive(Debug, Default)]
pub struct BrowserLogins {
    codes: Mutex<HashMap<[u8; 32], Instant>>,
    sessions: Mutex<HashMap<[u8; 32], Instant>>,
}

impl BrowserLogins {
    /// Neuer Einmal-Code (128 bit), gültig für 60 s ab `now`.
    pub fn issue_code(&self, now: Instant) -> Result<String, AuthError> {
        let code = random_hex(16)?;
        if let Ok(mut codes) = self.codes.lock() {
            codes.retain(|_, expires| *expires > now);
            codes.insert(sha256(&code), now + CODE_TTL);
        }
        Ok(code)
    }

    /// Löst einen Code ein (genau einmal, innerhalb der Frist) und liefert den Cookie-Wert.
    pub fn redeem(&self, code: &str, now: Instant) -> Option<String> {
        let expires = self.codes.lock().ok()?.remove(&sha256(code))?;
        if now >= expires {
            return None;
        }
        let cookie = random_hex(32).ok()?;
        let mut sessions = self.sessions.lock().ok()?;
        sessions.retain(|_, e| *e > now);
        sessions.insert(sha256(&cookie), now + COOKIE_TTL);
        Some(cookie)
    }

    pub fn verify_cookie(&self, cookie: &str, now: Instant) -> bool {
        self.sessions
            .lock()
            .ok()
            .and_then(|s| s.get(&sha256(cookie)).copied())
            .is_some_and(|expires| now < expires)
    }
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
    fn auth_001_rotation_by_another_process_takes_effect_in_the_daemon() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = LocalToken::load_or_create(dir.path()).unwrap();
        let old = std::fs::read_to_string(daemon.path()).unwrap();
        assert!(daemon.verify(&old));
        // `beton auth rotate-local` läuft als eigener Prozess.
        LocalToken::load_or_create(dir.path())
            .unwrap()
            .rotate()
            .unwrap();
        let new = std::fs::read_to_string(daemon.path()).unwrap();
        assert!(!daemon.verify(&old), "altes Token ist sofort ungültig");
        assert!(daemon.verify(&new));
    }

    #[test]
    fn auth_001_missing_or_broken_token_file_rejects_everything() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = LocalToken::load_or_create(dir.path()).unwrap();
        let token = std::fs::read_to_string(daemon.path()).unwrap();
        std::fs::remove_file(daemon.path()).unwrap();
        assert!(!daemon.verify(&token), "fail closed ohne Datei");
        write_private(daemon.path(), "kaputt").unwrap();
        assert!(!daemon.verify(&token));
        assert!(!daemon.verify("kaputt"));
    }

    #[cfg(unix)]
    #[test]
    fn auth_001_token_file_with_loose_permissions_rejects_everything() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let daemon = LocalToken::load_or_create(dir.path()).unwrap();
        let token = std::fs::read_to_string(daemon.path()).unwrap();
        // Neues Token mit zu weiten Rechten unterschieben.
        let other = random_hex(32).unwrap();
        std::fs::write(daemon.path(), &other).unwrap();
        std::fs::set_permissions(daemon.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!daemon.verify(&other));
        assert!(!daemon.verify(&token));
    }

    #[test]
    fn auth_004_ac1_code_is_single_use_and_expires() {
        let logins = BrowserLogins::default();
        let t0 = Instant::now();
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
}
