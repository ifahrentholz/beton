//! Verschlüsselter Datei-Store `~/.beton/secrets/store.v1` (SEC-003).
//!
//! Format (alle Zahlen big-endian):
//!
//! | Offset | Länge | Inhalt |
//! |---|---|---|
//! | 0 | 8 | Magic `BETONSV1` |
//! | 8 | 1 | Formatversion `1` |
//! | 9 | 1 | KDF `1` = Argon2id (Version 0x13) |
//! | 10 | 12 | `m` (KiB), `t`, `p` als `u32` |
//! | 22 | 16 | Salt (zufällig, neu bei jedem Passphrase-Wechsel) |
//! | 38 | 16 | Key-Check-Value (KCV) |
//! | 54 | 12 | Nonce (zufällig, neu bei jedem Schreiben) |
//! | 66 | n | AES-256-GCM-Chiffrat samt Tag; AAD = Bytes 0..66 |
//! | 66+n | 32 | SHA-256 über alles davor |
//!
//! Schlüssel: `master = Argon2id(passphrase, salt, m, t, p)` (32 Byte), daraus per
//! HKDF-SHA256 der Datenschlüssel (`enc`) und der KCV (`kcv`). Der KCV unterscheidet eine
//! falsche Passphrase (`secrets_locked`) von einer beschädigten Datei (`secrets_corrupt`); die
//! Prüfsumme am Ende sorgt dafür, dass **jede** Veränderung der Datei als Integritätsfehler
//! erkannt wird, bevor Argon2 läuft. Gegen gezielte Manipulation schützt das GCM-Tag (der
//! Header ist AAD).
//!
//! Geschrieben wird atomar: Temp-Datei (0600) im selben Verzeichnis, `fsync`, `rename`,
//! `fsync` des Verzeichnisses (AC4). Im Speicher liegt nur der abgeleitete Schlüssel, die
//! Klartexte nur für die Dauer eines Zugriffs (`Zeroizing`).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use aes_gcm::aead::{Aead as _, KeyInit as _, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use hkdf::Hkdf;
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use zeroize::Zeroizing;

use super::{Backend, BackendKind};
use crate::error::{Error, Result};
use crate::{SecretRef, SecretValue};

const MAGIC: &[u8; 8] = b"BETONSV1";
const FORMAT_VERSION: u8 = 1;
const KDF_ARGON2ID: u8 = 1;
const SALT_LEN: usize = 16;
const KCV_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const HEADER_LEN: usize = 66;
const TAG_LEN: usize = 16;
const SUM_LEN: usize = 32;
const HKDF_SALT: &[u8] = b"beton-secrets-store-v1";

/// Argon2id-Parameter (SEC-003: m = 64 MiB, t = 3, p = 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    /// Speicher in KiB.
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            m_kib: 64 * 1024,
            t: 3,
            p: 1,
        }
    }
}

impl KdfParams {
    /// Obergrenzen gegen einen manipulierten Header (Speicher, Laufzeit).
    fn plausible(self) -> bool {
        (8..=4 * 1024 * 1024).contains(&self.m_kib)
            && (1..=64).contains(&self.t)
            && (1..=16).contains(&self.p)
            && self.m_kib >= 8 * self.p
    }
}

/// SHA-256 über die Datei ohne die Prüfsumme.
pub fn checksum(bytes: &[u8]) -> [u8; SUM_LEN] {
    Sha256::digest(bytes).into()
}

struct Key {
    enc: Zeroizing<[u8; 32]>,
    kcv: [u8; KCV_LEN],
    salt: [u8; SALT_LEN],
    params: KdfParams,
}

fn derive(passphrase: &[u8], salt: &[u8; SALT_LEN], params: KdfParams) -> Result<Key> {
    let argon = Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(params.m_kib, params.t, params.p, Some(32))
            .map_err(|e| Error::Invalid(format!("Argon2-Parameter: {e}")))?,
    );
    let mut master = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(passphrase, salt, master.as_mut())
        .map_err(|e| Error::Invalid(format!("Argon2: {e}")))?;
    let hk = Hkdf::<Sha256>::new(Some(HKDF_SALT), master.as_ref());
    let mut enc = Zeroizing::new([0u8; 32]);
    let mut kcv = [0u8; KCV_LEN];
    hk.expand(b"enc", enc.as_mut())
        .and_then(|()| hk.expand(b"kcv", &mut kcv))
        .map_err(|_| Error::Invalid("HKDF-Länge".into()))?;
    Ok(Key {
        enc,
        kcv,
        salt: *salt,
        params,
    })
}

/// Gelesene, auf Integrität geprüfte Datei.
struct Parsed {
    params: KdfParams,
    salt: [u8; SALT_LEN],
    kcv: [u8; KCV_LEN],
    nonce: [u8; NONCE_LEN],
    header: Vec<u8>,
    ciphertext: Vec<u8>,
}

fn corrupt(why: &str) -> Error {
    Error::Corrupt(format!("Secret-Datei beschädigt oder manipuliert ({why})"))
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&bytes[at..at + 4]);
    u32::from_be_bytes(b)
}

fn parse(bytes: &[u8]) -> Result<Parsed> {
    if bytes.len() < HEADER_LEN + TAG_LEN + SUM_LEN {
        return Err(corrupt("zu kurz"));
    }
    let (body, sum) = bytes.split_at(bytes.len() - SUM_LEN);
    if !bool::from(checksum(body).ct_eq(sum)) {
        return Err(corrupt("Prüfsumme"));
    }
    if &body[..8] != MAGIC || body[8] != FORMAT_VERSION || body[9] != KDF_ARGON2ID {
        return Err(corrupt("unbekanntes Format"));
    }
    let params = KdfParams {
        m_kib: u32_at(body, 10),
        t: u32_at(body, 14),
        p: u32_at(body, 18),
    };
    if !params.plausible() {
        return Err(corrupt("KDF-Parameter"));
    }
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&body[22..38]);
    let mut kcv = [0u8; KCV_LEN];
    kcv.copy_from_slice(&body[38..54]);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&body[54..66]);
    Ok(Parsed {
        params,
        salt,
        kcv,
        nonce,
        header: body[..HEADER_LEN].to_vec(),
        ciphertext: body[HEADER_LEN..].to_vec(),
    })
}

type Entries = BTreeMap<String, SecretValue>;

fn encode(entries: &Entries) -> Result<Zeroizing<Vec<u8>>> {
    let mut out = Zeroizing::new(Vec::new());
    let len = |n: usize| u32::try_from(n).map_err(|_| Error::Invalid("Eintrag zu groß".into()));
    out.extend_from_slice(&len(entries.len())?.to_be_bytes());
    for (k, v) in entries {
        out.extend_from_slice(&len(k.len())?.to_be_bytes());
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(&len(v.len())?.to_be_bytes());
        out.extend_from_slice(v.expose());
    }
    Ok(out)
}

fn decode(plain: &[u8]) -> Result<Entries> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Result<&[u8]> {
        let end = at.checked_add(n).filter(|e| *e <= plain.len());
        let end = end.ok_or_else(|| corrupt("Inhalt"))?;
        let s = &plain[at..end];
        at = end;
        Ok(s)
    };
    let read_len = |s: &[u8]| -> usize {
        let mut b = [0u8; 4];
        b.copy_from_slice(s);
        u32::from_be_bytes(b) as usize
    };
    let count = read_len(take(4)?);
    let mut entries = Entries::new();
    for _ in 0..count {
        let n = read_len(take(4)?);
        let key = String::from_utf8(take(n)?.to_vec()).map_err(|_| corrupt("Inhalt"))?;
        let n = read_len(take(4)?);
        let value = SecretValue::from_bytes(take(n)?.to_vec());
        entries.insert(key, value);
    }
    Ok(entries)
}

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| Error::Unavailable(format!("Zufallsquelle: {e}")))?;
    Ok(b)
}

/// Der verschlüsselte Datei-Store.
pub struct FileBackend {
    path: PathBuf,
    /// Parameter für neu angelegte Dateien bzw. Passphrase-Wechsel.
    params: KdfParams,
    key: Mutex<Option<Key>>,
}

impl std::fmt::Debug for FileBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileBackend")
            .field("path", &self.path)
            .field("locked", &self.locked())
            .finish()
    }
}

impl FileBackend {
    /// Store unter `path`; gesperrt, bis [`Backend::unlock`] die Passphrase bekommt.
    pub fn new(path: PathBuf, params: KdfParams) -> Self {
        Self {
            path,
            params,
            key: Mutex::new(None),
        }
    }

    /// Standardpfad unter dem Datenverzeichnis: `<home>/secrets/store.v1`.
    pub fn default_path(home: &Path) -> PathBuf {
        home.join("secrets").join("store.v1")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    fn guard(&self) -> Result<MutexGuard<'_, Option<Key>>> {
        self.key
            .lock()
            .map_err(|_| Error::Unavailable("Datei-Store vergiftet".into()))
    }

    fn read(&self) -> Result<Parsed> {
        let bytes = std::fs::read(&self.path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => corrupt("Datei fehlt"),
            _ => Error::Io(format!("{}: {e}", self.path.display())),
        })?;
        parse(&bytes)
    }

    /// Prüft die Passphrase gegen eine gelesene Datei; falsch → `secrets_locked`.
    fn open_with(passphrase: &[u8], parsed: &Parsed) -> Result<Key> {
        let key = derive(passphrase, &parsed.salt, parsed.params)?;
        if !bool::from(key.kcv.ct_eq(&parsed.kcv)) {
            return Err(Error::Locked);
        }
        Ok(key)
    }

    fn decrypt(key: &Key, parsed: &Parsed) -> Result<Entries> {
        // Eine Datei mit fremdem Schlüssel (z. B. von einem anderen Prozess neu angelegt):
        // gesperrt, nicht beschädigt.
        if parsed.salt != key.salt || !bool::from(key.kcv.ct_eq(&parsed.kcv)) {
            return Err(Error::Locked);
        }
        let cipher = Aes256Gcm::new_from_slice(key.enc.as_ref())
            .map_err(|_| Error::Invalid("Schlüssellänge".into()))?;
        let nonce = Nonce::try_from(&parsed.nonce[..]).map_err(|_| corrupt("Nonce"))?;
        let plain = Zeroizing::new(
            cipher
                .decrypt(
                    &nonce,
                    Payload {
                        msg: &parsed.ciphertext,
                        aad: &parsed.header,
                    },
                )
                .map_err(|_| corrupt("Authentifizierung"))?,
        );
        decode(&plain)
    }

    fn write(&self, key: &Key, entries: &Entries) -> Result<()> {
        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(MAGIC);
        header.push(FORMAT_VERSION);
        header.push(KDF_ARGON2ID);
        header.extend_from_slice(&key.params.m_kib.to_be_bytes());
        header.extend_from_slice(&key.params.t.to_be_bytes());
        header.extend_from_slice(&key.params.p.to_be_bytes());
        header.extend_from_slice(&key.salt);
        header.extend_from_slice(&key.kcv);
        let nonce_bytes: [u8; NONCE_LEN] = random()?;
        header.extend_from_slice(&nonce_bytes);
        let plain = encode(entries)?;
        let cipher = Aes256Gcm::new_from_slice(key.enc.as_ref())
            .map_err(|_| Error::Invalid("Schlüssellänge".into()))?;
        let nonce = Nonce::try_from(&nonce_bytes[..]).map_err(|_| corrupt("Nonce"))?;
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: &plain,
                    aad: &header,
                },
            )
            .map_err(|_| Error::Invalid("Verschlüsselung".into()))?;
        let mut file = header;
        file.extend_from_slice(&ciphertext);
        let sum = checksum(&file);
        file.extend_from_slice(&sum);
        write_atomic(&self.path, &file)
    }

    /// Liest, ändert und schreibt die Einträge unter dem Lock.
    fn modify<T>(&self, f: impl FnOnce(&mut Entries) -> T) -> Result<T> {
        let guard = self.guard()?;
        let key = guard.as_ref().ok_or(Error::Locked)?;
        let mut entries = Self::decrypt(key, &self.read()?)?;
        let out = f(&mut entries);
        self.write(key, &entries)?;
        Ok(out)
    }
}

impl Backend for FileBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::File
    }

    fn locked(&self) -> bool {
        self.key.lock().map_or(true, |k| k.is_none())
    }

    fn put(&self, r: &SecretRef, value: &SecretValue) -> Result<()> {
        self.modify(|e| {
            e.insert(
                r.to_string(),
                SecretValue::from_bytes(value.expose().to_vec()),
            );
        })
    }

    fn get(&self, r: &SecretRef) -> Result<SecretValue> {
        let guard = self.guard()?;
        let key = guard.as_ref().ok_or(Error::Locked)?;
        let mut entries = Self::decrypt(key, &self.read()?)?;
        entries
            .remove(&r.to_string())
            .ok_or_else(|| Error::NotFound(r.to_string()))
    }

    fn delete(&self, r: &SecretRef) -> Result<bool> {
        let guard = self.guard()?;
        let key = guard.as_ref().ok_or(Error::Locked)?;
        let mut entries = Self::decrypt(key, &self.read()?)?;
        if entries.remove(&r.to_string()).is_none() {
            return Ok(false);
        }
        self.write(key, &entries)?;
        Ok(true)
    }

    /// Entsperrt eine vorhandene Datei bzw. legt sie mit dieser Passphrase an. Eine falsche
    /// Passphrase liefert `secrets_locked` und lässt die Datei unverändert (AC1).
    fn unlock(&self, passphrase: &SecretValue) -> Result<()> {
        let mut guard = self.guard()?;
        if !self.path.exists() {
            if passphrase.is_empty() {
                return Err(Error::Invalid("Passphrase darf nicht leer sein".into()));
            }
            let key = derive(passphrase.expose(), &random()?, self.params)?;
            self.write(&key, &Entries::new())?;
            *guard = Some(key);
            return Ok(());
        }
        let parsed = self.read()?;
        let key = Self::open_with(passphrase.expose(), &parsed)?;
        // Erst wenn auch das Chiffrat stimmt, gilt der Store als entsperrt.
        Self::decrypt(&key, &parsed)?;
        *guard = Some(key);
        Ok(())
    }

    /// Verschlüsselt mit neuem Salt und neuer Passphrase neu (AC3).
    fn change_passphrase(&self, old: &SecretValue, new: &SecretValue) -> Result<()> {
        if new.is_empty() {
            return Err(Error::Invalid("Passphrase darf nicht leer sein".into()));
        }
        let mut guard = self.guard()?;
        let parsed = self.read()?;
        let old_key = Self::open_with(old.expose(), &parsed)?;
        let entries = Self::decrypt(&old_key, &parsed)?;
        let new_key = derive(new.expose(), &random()?, self.params)?;
        self.write(&new_key, &entries)?;
        *guard = Some(new_key);
        Ok(())
    }
}

/// Schreibt atomar: Temp-Datei (0600), `fsync`, `rename`, `fsync` des Verzeichnisses.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| Error::Io("Pfad ohne Verzeichnis".into()))?;
    create_private_dir(dir)?;
    let suffix: [u8; 8] = random()?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("store"),
        suffix
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ));
    let result = (|| -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        let (first, rest) = bytes.split_at(bytes.len() / 2);
        file.write_all(first)?;
        #[cfg(test)]
        kill_hook::pause(1);
        file.write_all(rest)?;
        file.sync_all()?;
        drop(file);
        #[cfg(test)]
        kill_hook::pause(2);
        std::fs::rename(&tmp, path)?;
        #[cfg(test)]
        kill_hook::pause(3);
        #[cfg(unix)]
        std::fs::File::open(dir)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn create_private_dir(dir: &Path) -> Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    Ok(())
}

/// Env-Variable des Test-Builds: Phase (1–3), in der `write_atomic` anhält (SEC-003 AC4).
#[cfg(test)]
pub(crate) const PHASE_ENV: &str = "BETON_SECRETS_TEST_KILL_PHASE";

/// Haltepunkte im Test-Build, damit der Kill-Test sicher mitten im Schreiben tötet.
#[cfg(test)]
mod kill_hook {
    use std::io::Write as _;

    pub(super) fn pause(phase: u8) {
        let wanted = std::env::var(super::PHASE_ENV)
            .ok()
            .and_then(|v| v.parse::<u8>().ok());
        if wanted == Some(phase) {
            println!("PHASE {phase}");
            let _ = std::io::stdout().flush();
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    }
}
