//! Modellverzeichnis `~/.beton/models/whisper/` (VOI-002).
//!
//! Im Verzeichnis liegen nur vollständige, geprüfte Modelle unter ihrem Dateinamen aus der
//! gepinnten Liste. Halbe Downloads liegen in `.partial/` (fortsetzbar per HTTP-Range) und
//! werden bei falscher Prüfsumme gelöscht; ein Modell erscheint erst nach erfolgreicher Prüfung
//! per Umbenennen im Verzeichnis (atomar auf demselben Dateisystem).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::catalog::{Catalog, ModelEntry};

/// Unterverzeichnis für halbe Downloads.
pub(crate) const PARTIAL_DIR: &str = ".partial";

/// Ein Modell, wie `beton voice models list --json` es zeigt (VOI-002 AC4; Schema
/// `schemas/v1/voice-models.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelInfo {
    /// Name, z. B. `small` oder `large-v3-turbo-q5_0`.
    pub name: String,
    /// Dateiname im Modellverzeichnis.
    pub file: String,
    /// Größe in Bytes laut gepinnter Liste.
    pub size_bytes: u64,
    /// Liegt das Modell vollständig im Modellverzeichnis?
    pub installed: bool,
    /// SHA-256 laut gepinnter Liste (hex).
    pub sha256: String,
    /// Quantisierung, z. B. `q5_0`; `null` für volle Genauigkeit.
    pub quantization: Option<String>,
    /// Default-Modell für diese Hardware.
    pub recommended: bool,
}

/// Fehler der Modellverwaltung.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("unbekanntes Modell „{0}“ (beton voice models list)")]
    UnknownModel(String),
    #[error(
        "{path}: SHA-256 {sha256} passt zu keinem Modell der mitgelieferten Liste; nur geprüfte \
         Modelldateien lassen sich importieren"
    )]
    NotPinned { path: PathBuf, sha256: String },
    #[error("{0}: Modell ist nicht installiert")]
    NotInstalled(String),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> StoreError + '_ {
    move |source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// SHA-256 einer Datei (hex, klein).
pub fn sha256_file(path: &Path) -> Result<String, StoreError> {
    let mut f = std::fs::File::open(path).map_err(io(path))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(io(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Das Modellverzeichnis.
#[derive(Debug, Clone)]
pub struct ModelStore {
    dir: PathBuf,
}

impl ModelStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `<home>/models/whisper` (bei Default-Home `~/.beton/models/whisper/`).
    pub fn in_home(home: &Path) -> Self {
        Self::new(home.join("models").join("whisper"))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Pfad eines Modells im Verzeichnis (unabhängig davon, ob es installiert ist).
    pub fn path(&self, entry: &ModelEntry) -> PathBuf {
        self.dir.join(&entry.file)
    }

    pub(crate) fn partial_path(&self, entry: &ModelEntry) -> PathBuf {
        self.dir
            .join(PARTIAL_DIR)
            .join(format!("{}.part", entry.file))
    }

    /// Ist das Modell vollständig da? Geprüft wird die Größe; die Prüfsumme wurde beim
    /// Installieren (Download bzw. Import) geprüft.
    pub fn is_installed(&self, entry: &ModelEntry) -> bool {
        std::fs::metadata(self.path(entry))
            .is_ok_and(|m| m.is_file() && m.len() == entry.size_bytes)
    }

    /// Alle Modelle der Liste mit Installationsstatus (VOI-002 AC4).
    pub fn list(&self, catalog: &Catalog, recommended: &str) -> Vec<ModelInfo> {
        catalog
            .entries()
            .iter()
            .map(|e| ModelInfo {
                name: e.name.clone(),
                file: e.file.clone(),
                size_bytes: e.size_bytes,
                installed: self.is_installed(e),
                sha256: e.sha256.clone(),
                quantization: e.quantization.clone(),
                recommended: e.name == recommended,
            })
            .collect()
    }

    /// Übernimmt eine lokale Modelldatei ohne Netzwerk (VOI-002 AC5, ADR-0033): nur, wenn ihre
    /// SHA-256 zu einem Eintrag der gepinnten Liste passt. Die Datei wird kopiert (die Quelle
    /// bleibt unverändert) und erst nach erneuter Prüfung der Kopie ins Verzeichnis umbenannt.
    pub fn import(&self, catalog: &Catalog, file: &Path) -> Result<ModelEntry, StoreError> {
        let sha = sha256_file(file)?;
        let Some(entry) = catalog.by_sha256(&sha) else {
            return Err(StoreError::NotPinned {
                path: file.to_path_buf(),
                sha256: sha,
            });
        };
        let partial = self.dir.join(PARTIAL_DIR);
        std::fs::create_dir_all(&partial).map_err(io(&partial))?;
        let tmp = partial.join(format!("{}.import", entry.file));
        let result = (|| {
            let mut src = std::fs::File::open(file).map_err(io(file))?;
            let mut dst = std::fs::File::create(&tmp).map_err(io(&tmp))?;
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = src.read(&mut buf).map_err(io(file))?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                dst.write_all(&buf[..n]).map_err(io(&tmp))?;
            }
            dst.sync_all().map_err(io(&tmp))?;
            // Die Kopie selbst prüfen: die Quelle könnte sich zwischendurch geändert haben.
            let copied = hex::encode(hasher.finalize());
            if !copied.eq_ignore_ascii_case(&entry.sha256) {
                return Err(StoreError::NotPinned {
                    path: file.to_path_buf(),
                    sha256: copied,
                });
            }
            let target = self.path(entry);
            std::fs::rename(&tmp, &target).map_err(io(&target))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result.map(|()| entry.clone())
    }

    /// Entfernt ein installiertes Modell samt eventuellem halben Download.
    pub fn remove(&self, catalog: &Catalog, name: &str) -> Result<(), StoreError> {
        let entry = catalog
            .get(name)
            .ok_or_else(|| StoreError::UnknownModel(name.to_owned()))?;
        let partial = self.partial_path(entry);
        let _ = std::fs::remove_file(&partial);
        let path = self.path(entry);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(StoreError::NotInstalled(name.to_owned()))
            }
            Err(e) => Err(io(&path)(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, content: &[u8]) -> ModelEntry {
        ModelEntry {
            name: name.to_owned(),
            file: format!("ggml-{name}.bin"),
            url: format!("http://127.0.0.1:1/{name}"),
            size_bytes: content.len() as u64,
            sha256: hex::encode(Sha256::digest(content)),
            quantization: None,
        }
    }

    /// VOI-002 AC4: Liste mit Name, Größe, installiert ja/nein und Prüfsumme.
    #[test]
    fn voi_002_ac4_list_reports_size_installed_and_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let catalog = Catalog::for_tests(vec![entry("a", b"aaaa"), entry("b", b"bbbbbb")]);
        std::fs::write(dir.path().join("ggml-a.bin"), b"aaaa").unwrap();
        // Falsche Größe zählt nicht als installiert.
        std::fs::write(dir.path().join("ggml-b.bin"), b"bb").unwrap();
        let list = store.list(&catalog, "b");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "a");
        assert!(list[0].installed);
        assert_eq!(list[0].size_bytes, 4);
        assert_eq!(list[0].sha256, hex::encode(Sha256::digest(b"aaaa")));
        assert!(!list[0].recommended);
        assert!(!list[1].installed);
        assert!(list[1].recommended);
    }

    /// VOI-002 AC5: Import einer Datei mit passender Prüfsumme; ohne passenden Eintrag
    /// abgelehnt, und im Verzeichnis bleibt nichts liegen.
    #[test]
    fn voi_002_ac5_import_accepts_pinned_checksum_and_rejects_others() {
        let dir = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path().join("models"));
        let catalog = Catalog::for_tests(vec![entry("tiny", b"model-bytes")]);

        let good = src.path().join("irgendwas.bin");
        std::fs::write(&good, b"model-bytes").unwrap();
        let e = store.import(&catalog, &good).unwrap();
        assert_eq!(e.name, "tiny");
        assert_eq!(
            std::fs::read(dir.path().join("models/ggml-tiny.bin")).unwrap(),
            b"model-bytes"
        );
        assert!(good.exists(), "Quelle bleibt unverändert");
        assert!(store.is_installed(&e));

        let bad = src.path().join("fremd.bin");
        std::fs::write(&bad, b"anderes modell").unwrap();
        let err = store.import(&catalog, &bad).unwrap_err();
        assert!(matches!(err, StoreError::NotPinned { .. }), "{err}");
        let names: Vec<_> = std::fs::read_dir(dir.path().join("models"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.iter().filter(|n| !n.starts_with('.')).count(), 1);
        assert_eq!(
            std::fs::read_dir(dir.path().join("models").join(PARTIAL_DIR))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn remove_deletes_model_and_reports_missing() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let catalog = Catalog::for_tests(vec![entry("a", b"aaaa")]);
        std::fs::write(dir.path().join("ggml-a.bin"), b"aaaa").unwrap();
        store.remove(&catalog, "a").unwrap();
        assert!(!dir.path().join("ggml-a.bin").exists());
        assert!(matches!(
            store.remove(&catalog, "a"),
            Err(StoreError::NotInstalled(_))
        ));
        assert!(matches!(
            store.remove(&catalog, "x"),
            Err(StoreError::UnknownModel(_))
        ));
    }
}
