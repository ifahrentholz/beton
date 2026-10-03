//! Inhaltsadressierter Blob-Store im Dateisystem (DATA-006).
//!
//! Dateien liegen unter `<dir>/sha256/<ab>/<cd>/<hash>` mit Rechten `0600`. Der Store kennt
//! nur Inhalte; wer welchen Blob sehen darf, entscheidet die referenzierende Ressource
//! (`blob_refs`, siehe [`crate::Store::session_blob`]). Es gibt bewusst keinen Zugriff
//! allein per Hash nach außen.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use beton_core::event::BlobRef;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Default-Obergrenze für einen Blob: 512 MiB.
pub const DEFAULT_MAX_BLOB_BYTES: u64 = 512 * 1024 * 1024;

/// Unreferenzierte Blobs werden erst nach dieser Frist gelöscht (DATA-006).
pub const GC_GRACE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
    max_bytes: u64,
}

/// Ergebnis eines GC-Laufs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    /// Gelöschte Blob-Einträge in der Datenbank.
    pub rows_deleted: u64,
    /// Gelöschte Dateien.
    pub files_deleted: u64,
}

impl BlobStore {
    pub fn open(root: &Path, max_bytes: u64) -> Result<Self> {
        std::fs::create_dir_all(root.join("sha256"))?;
        std::fs::create_dir_all(root.join("tmp"))?;
        Ok(Self {
            root: root.to_path_buf(),
            max_bytes,
        })
    }

    /// Speichert `bytes` atomar (temporäre Datei, fsync, rename) und liefert die Referenz.
    pub fn put(&self, bytes: &[u8]) -> Result<BlobRef> {
        let size = bytes.len() as u64;
        if size > self.max_bytes {
            return Err(Error::BlobTooLarge {
                size,
                max: self.max_bytes,
            });
        }
        let hex = hex::encode(Sha256::digest(bytes));
        let blob = BlobRef::from_hex(&hex).map_err(Error::corrupt)?;
        let path = self.path(&blob);
        if path.exists() {
            // Frische Änderungszeit schützt den Inhalt vor einem parallelen GC-Sweep.
            std::fs::File::options()
                .write(true)
                .open(&path)?
                .set_modified(SystemTime::now())?;
            return Ok(blob);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self
            .root
            .join("tmp")
            .join(format!("{hex}.{}", ulid_suffix()));
        {
            let mut file = create_private(&tmp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, &path)?;
        Ok(blob)
    }

    /// Liest einen Blob und prüft den Hash (DATA-006 AC3).
    pub(crate) fn read(&self, blob: &BlobRef) -> Result<Vec<u8>> {
        let bytes = match std::fs::read(self.path(blob)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::NotFound(format!("Blob {blob}")));
            }
            Err(e) => return Err(e.into()),
        };
        if hex::encode(Sha256::digest(&bytes)) != blob.hex() {
            return Err(Error::BlobCorrupt(blob.clone()));
        }
        Ok(bytes)
    }

    pub(crate) fn exists(&self, blob: &BlobRef) -> bool {
        self.path(blob).exists()
    }

    pub(crate) fn path(&self, blob: &BlobRef) -> PathBuf {
        let hex = blob.hex();
        self.root
            .join("sha256")
            .join(&hex[0..2])
            .join(&hex[2..4])
            .join(hex)
    }

    /// Alle gespeicherten Blobs mit Änderungszeit (für den Sweep).
    pub(crate) fn list(&self) -> Result<Vec<(BlobRef, SystemTime)>> {
        let mut out = Vec::new();
        let root = self.root.join("sha256");
        for a in read_dir(&root)? {
            for b in read_dir(&a)? {
                for file in read_dir(&b)? {
                    let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    if let Ok(blob) = BlobRef::from_hex(name) {
                        out.push((blob, std::fs::metadata(&file)?.modified()?));
                    }
                }
            }
        }
        Ok(out)
    }

    pub(crate) fn remove(&self, blob: &BlobRef) -> Result<bool> {
        match std::fs::remove_file(self.path(blob)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

fn read_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    std::fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()).map_err(Error::from))
        .collect()
}

fn ulid_suffix() -> String {
    beton_core::id::EventId::new().ulid().to_string()
}

#[cfg(unix)]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_006_blobs_are_content_addressed_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path(), 1024).unwrap();
        let a = store.put(b"hallo").unwrap();
        assert_eq!(store.put(b"hallo").unwrap(), a);
        assert_eq!(
            a.hex(),
            "d3751d33f9cd5049c4af2b462735457e4d3baf130bcbb87f389e349fbaeb20b9"
        );
        let path = store.path(&a);
        assert!(path.ends_with(format!("sha256/d3/75/{}", a.hex())));
        assert_eq!(store.read(&a).unwrap(), b"hallo");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn data_006_ac3_corrupt_blob_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path(), 1024).unwrap();
        let a = store.put(b"original").unwrap();
        std::fs::write(store.path(&a), b"manipuliert").unwrap();
        let err = store.read(&a).unwrap_err();
        assert_eq!(err.code(), "blob_corrupt");
    }

    #[test]
    fn blob_size_limit_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path(), 4).unwrap();
        assert_eq!(store.put(b"12345").unwrap_err().code(), "blob_too_large");
        assert!(store.put(b"1234").is_ok());
    }
}
