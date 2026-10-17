//! Modell-Download nach ausdrücklicher Zustimmung (VOI-002, VOI-008, ADR-0033).
//!
//! Das ist die einzige Stelle, an der `beton-voice` eine Netzverbindung aufbaut. Ohne
//! [`Consent`] gibt es keinen Download; die URL stammt immer aus der gepinnten Liste
//! ([`Catalog`]), Weiterleitungen nur zu deren erlaubten Hosts. Der Download landet zuerst in
//! `.partial/` und wird per HTTP-Range fortgesetzt; nach dem letzten Byte prüft beton die
//! SHA-256 und verwirft die Datei bei Abweichung (fail closed). Erst eine geprüfte Datei wird ins
//! Modellverzeichnis umbenannt.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::catalog::Catalog;
use crate::store::ModelStore;

/// Ausdrückliche Zustimmung des Users zu einem Download (VOI-002 AC3). Nur die Oberfläche, die
/// den User gefragt hat (CLI-Rückfrage, `--yes`, Klick in den Einstellungen), erzeugt sie.
#[derive(Debug)]
pub struct Consent(());

impl Consent {
    /// Der User hat dem Download dieses Modells zugestimmt.
    pub fn given_by_user() -> Self {
        Self(())
    }
}

/// Fortschritt eines Downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Bytes auf der Platte (inklusive fortgesetzter).
    pub downloaded: u64,
    pub total: u64,
    /// Ab hier wurde fortgesetzt (0 bei neuem Download).
    pub resumed_from: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("unbekanntes Modell „{0}“ (beton voice models list)")]
    UnknownModel(String),
    #[error(
        "Prüfsumme stimmt nicht (erwartet {expected}, erhalten {actual}); die Datei wurde \
         verworfen"
    )]
    ChecksumMismatch { expected: String, actual: String },
    #[error("Server lieferte mehr als die erwarteten {expected} Bytes; die Datei wurde verworfen")]
    TooLarge { expected: u64 },
    #[error("Weiterleitung zu {0} ist nicht erlaubt (nur gepinnte Hosts per HTTPS)")]
    RedirectRefused(String),
    #[error("HTTP {0}")]
    Status(u16),
    #[error("Download unterbrochen nach {downloaded} von {total} Bytes; erneut starten setzt fort")]
    Interrupted { downloaded: u64, total: u64 },
    #[error("Netzwerk: {0}")]
    Network(String),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> DownloadError + '_ {
    move |source| DownloadError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Lädt ein Modell der gepinnten Liste (VOI-002). Ist es schon installiert, passiert nichts.
pub async fn pull(
    catalog: &Catalog,
    store: &ModelStore,
    name: &str,
    _consent: Consent,
    mut progress: impl FnMut(Progress) + Send,
) -> Result<PathBuf, DownloadError> {
    let entry = catalog
        .get(name)
        .ok_or_else(|| DownloadError::UnknownModel(name.to_owned()))?;
    let target = store.path(entry);
    if store.is_installed(entry) {
        return Ok(target);
    }
    let part = store.partial_path(entry);
    if let Some(parent) = part.parent() {
        std::fs::create_dir_all(parent).map_err(io(parent))?;
    }

    let refused: Arc<Mutex<Option<String>>> = Arc::default();
    let policy = {
        let catalog = catalog.clone();
        let refused = refused.clone();
        reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("zu viele Weiterleitungen")
            } else if catalog.allows_redirect(attempt.url()) {
                attempt.follow()
            } else {
                if let Ok(mut r) = refused.lock() {
                    *r = Some(attempt.url().to_string());
                }
                attempt.error("Weiterleitung nicht erlaubt")
            }
        })
    };
    let mut builder = reqwest::Client::builder()
        .redirect(policy)
        .connect_timeout(Duration::from_secs(30))
        .user_agent(concat!("beton/", env!("CARGO_PKG_VERSION")));
    if entry.url.starts_with("http://127.0.0.1") {
        builder = builder.no_proxy();
    }
    let client = builder
        .build()
        .map_err(|e| DownloadError::Network(e.to_string()))?;
    let network = |e: reqwest::Error| {
        if let Some(url) = refused.lock().ok().and_then(|r| r.clone()) {
            DownloadError::RedirectRefused(url)
        } else {
            DownloadError::Network(e.to_string())
        }
    };

    let mut have = std::fs::metadata(&part).map_or(0, |m| m.len());
    if have > entry.size_bytes {
        std::fs::remove_file(&part).map_err(io(&part))?;
        have = 0;
    }
    let mut request = client.get(&entry.url);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut response = request.send().await.map_err(network)?;
    let status = response.status().as_u16();
    let resumed_from = match status {
        206 if have > 0 && content_range_start(&response) == Some(have) => have,
        // Server ignoriert Range: von vorn.
        200 => 0,
        // Der Teil ist schon vollständig (oder größer): unten prüfen bzw. neu laden.
        416 if have == entry.size_bytes => have,
        s => {
            if s == 416 {
                let _ = std::fs::remove_file(&part);
            }
            return Err(DownloadError::Status(s));
        }
    };

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&part)
        .map_err(io(&part))?;
    let mut hasher = Sha256::new();
    if resumed_from > 0 {
        // Bereits geladenen Teil in die Prüfsumme aufnehmen.
        let mut buf = vec![0u8; 1 << 20];
        let mut left = resumed_from;
        file.seek(SeekFrom::Start(0)).map_err(io(&part))?;
        while left > 0 {
            let n = file.read(&mut buf).map_err(io(&part))?;
            if n == 0 {
                break;
            }
            let n = n.min(usize::try_from(left).unwrap_or(usize::MAX));
            hasher.update(&buf[..n]);
            left -= n as u64;
        }
    }
    file.set_len(resumed_from).map_err(io(&part))?;
    file.seek(SeekFrom::Start(resumed_from))
        .map_err(io(&part))?;

    let mut downloaded = resumed_from;
    progress(Progress {
        downloaded,
        total: entry.size_bytes,
        resumed_from,
    });
    if status != 416 {
        loop {
            let chunk = match response.chunk().await {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(_) => {
                    // Teil behalten: ein neuer Aufruf setzt per Range fort (VOI-002 AC2).
                    let _ = file.sync_all();
                    return Err(DownloadError::Interrupted {
                        downloaded,
                        total: entry.size_bytes,
                    });
                }
            };
            downloaded += chunk.len() as u64;
            if downloaded > entry.size_bytes {
                drop(file);
                let _ = std::fs::remove_file(&part);
                return Err(DownloadError::TooLarge {
                    expected: entry.size_bytes,
                });
            }
            hasher.update(&chunk);
            file.write_all(&chunk).map_err(io(&part))?;
            progress(Progress {
                downloaded,
                total: entry.size_bytes,
                resumed_from,
            });
        }
    }
    file.sync_all().map_err(io(&part))?;
    drop(file);
    if downloaded < entry.size_bytes {
        return Err(DownloadError::Interrupted {
            downloaded,
            total: entry.size_bytes,
        });
    }
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(&entry.sha256) {
        // Fail closed (VOI-002 AC1): nichts Halbfertiges oder Falsches bleibt liegen.
        let _ = std::fs::remove_file(&part);
        return Err(DownloadError::ChecksumMismatch {
            expected: entry.sha256.clone(),
            actual,
        });
    }
    std::fs::rename(&part, &target).map_err(io(&target))?;
    Ok(target)
}

fn content_range_start(response: &reqwest::Response) -> Option<u64> {
    let value = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    let rest = value.strip_prefix("bytes ")?;
    rest.split('-').next()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::catalog::ModelEntry;

    /// Verhalten des Mock-Servers auf Loopback.
    #[derive(Clone)]
    struct Behaviour {
        body: Arc<Vec<u8>>,
        /// Beim ersten Request nach so vielen Bytes die Verbindung trennen.
        cut_first_after: Option<usize>,
        /// Statt des Inhalts auf diese URL weiterleiten.
        redirect_to: Option<String>,
    }

    struct Mock {
        base: String,
        requests: Arc<AtomicUsize>,
        ranges: Arc<Mutex<Vec<Option<String>>>>,
    }

    async fn mock(b: Behaviour) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let ranges: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
        let (req, rng) = (requests.clone(), ranges.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let n = req.fetch_add(1, Ordering::SeqCst);
                let b = b.clone();
                let rng = rng.clone();
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if sock.read(&mut byte).await.unwrap_or(0) == 0 {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).to_string();
                    let range = head
                        .lines()
                        .find_map(|l| l.strip_prefix("range: ").or(l.strip_prefix("Range: ")))
                        .map(str::to_owned);
                    rng.lock().unwrap().push(range.clone());
                    if let Some(to) = &b.redirect_to {
                        let resp = format!(
                            "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\n\r\n"
                        );
                        let _ = sock.write_all(resp.as_bytes()).await;
                        return;
                    }
                    let start: usize = range
                        .as_deref()
                        .and_then(|r| r.strip_prefix("bytes="))
                        .and_then(|r| r.trim_end_matches('-').parse().ok())
                        .unwrap_or(0);
                    let body = &b.body[start.min(b.body.len())..];
                    let head = if start > 0 {
                        format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{}\r\n\r\n",
                            body.len(),
                            b.body.len() - 1,
                            b.body.len()
                        )
                    } else {
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len())
                    };
                    let _ = sock.write_all(head.as_bytes()).await;
                    match b.cut_first_after {
                        Some(cut) if n == 0 => {
                            let _ = sock.write_all(&body[..cut]).await;
                            let _ = sock.flush().await;
                            // Verbindung hart trennen: der Client sieht einen abgebrochenen Body.
                        }
                        _ => {
                            let _ = sock.write_all(body).await;
                        }
                    }
                });
            }
        });
        Mock {
            base,
            requests,
            ranges,
        }
    }

    fn content() -> Vec<u8> {
        (0..300_000u32).map(|i| (i % 251) as u8).collect()
    }

    fn entry(url: String, body: &[u8], sha_of: &[u8]) -> ModelEntry {
        ModelEntry {
            name: "test".into(),
            file: "ggml-test.bin".into(),
            url,
            size_bytes: body.len() as u64,
            sha256: hex::encode(Sha256::digest(sha_of)),
            quantization: None,
        }
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push(p.strip_prefix(dir).unwrap().display().to_string());
                }
            }
        }
        out
    }

    #[tokio::test]
    async fn voi_002_download_with_matching_checksum_installs_the_model() {
        let body = content();
        let m = mock(Behaviour {
            body: Arc::new(body.clone()),
            cut_first_after: None,
            redirect_to: None,
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let catalog = Catalog::for_tests(vec![entry(format!("{}/m", m.base), &body, &body)]);
        let mut last = None;
        let path = pull(&catalog, &store, "test", Consent::given_by_user(), |p| {
            last = Some(p);
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), body);
        assert_eq!(last.unwrap().downloaded, body.len() as u64);
        assert_eq!(files_in(dir.path()), vec!["ggml-test.bin".to_owned()]);
    }

    /// VOI-002 AC1: falsche Prüfsumme → verworfen, nichts im Modellverzeichnis.
    #[tokio::test]
    async fn voi_002_ac1_wrong_checksum_is_discarded() {
        let body = content();
        let m = mock(Behaviour {
            body: Arc::new(body.clone()),
            cut_first_after: None,
            redirect_to: None,
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let catalog = Catalog::for_tests(vec![entry(
            format!("{}/m", m.base),
            &body,
            b"etwas anderes",
        )]);
        let err = pull(&catalog, &store, "test", Consent::given_by_user(), |_| {})
            .await
            .unwrap_err();
        assert!(
            matches!(err, DownloadError::ChecksumMismatch { .. }),
            "{err}"
        );
        assert!(
            files_in(dir.path()).is_empty(),
            "{:?}",
            files_in(dir.path())
        );
        assert!(!store.is_installed(&catalog.entries()[0]));
    }

    /// VOI-002 AC2: Ein abgebrochener Download wird per HTTP-Range fortgesetzt.
    #[tokio::test]
    async fn voi_002_ac2_interrupted_download_resumes_with_range() {
        let body = content();
        let m = mock(Behaviour {
            body: Arc::new(body.clone()),
            cut_first_after: Some(100_000),
            redirect_to: None,
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let catalog = Catalog::for_tests(vec![entry(format!("{}/m", m.base), &body, &body)]);
        let err = pull(&catalog, &store, "test", Consent::given_by_user(), |_| {})
            .await
            .unwrap_err();
        assert!(matches!(err, DownloadError::Interrupted { .. }), "{err}");
        let part = store.partial_path(&catalog.entries()[0]);
        let have = std::fs::metadata(&part).unwrap().len();
        assert!(have > 0 && have < body.len() as u64, "{have}");
        assert!(!store.is_installed(&catalog.entries()[0]));

        let mut resumed = None;
        let path = pull(&catalog, &store, "test", Consent::given_by_user(), |p| {
            resumed = Some(p.resumed_from);
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), body);
        assert_eq!(resumed, Some(have));
        let ranges = m.ranges.lock().unwrap().clone();
        assert_eq!(ranges, vec![None, Some(format!("bytes={have}-"))]);
        assert_eq!(m.requests.load(Ordering::SeqCst), 2);
        assert!(!part.exists());
    }

    /// VOI-008 AC3: Eine Weiterleitung zu einem Host außerhalb der gepinnten Liste wird nicht
    /// verfolgt; der Ziel-Server sieht keinen Request.
    #[tokio::test]
    async fn voi_008_ac3_redirect_outside_the_pinned_hosts_is_refused() {
        let body = content();
        let target = mock(Behaviour {
            body: Arc::new(body.clone()),
            cut_first_after: None,
            redirect_to: None,
        })
        .await;
        // Gleicher Rechner, aber als `localhost` statt der gepinnten `127.0.0.1`.
        let port = target.base.rsplit(':').next().unwrap().to_owned();
        let redirect = mock(Behaviour {
            body: Arc::new(Vec::new()),
            cut_first_after: None,
            redirect_to: Some(format!("http://localhost:{port}/m")),
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let catalog = Catalog::for_tests(vec![entry(format!("{}/m", redirect.base), &body, &body)]);
        let err = pull(&catalog, &store, "test", Consent::given_by_user(), |_| {})
            .await
            .unwrap_err();
        assert!(matches!(err, DownloadError::RedirectRefused(_)), "{err}");
        assert_eq!(target.requests.load(Ordering::SeqCst), 0);
        assert!(!store.is_installed(&catalog.entries()[0]));
    }

    /// VOI-008 AC3: Nur Namen der Liste; eine URL lässt sich nicht übergeben.
    #[tokio::test]
    async fn voi_008_ac3_unknown_model_names_are_rejected_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let err = pull(
            &Catalog::pinned(),
            &store,
            "https://evil.example/model.bin",
            Consent::given_by_user(),
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::UnknownModel(_)), "{err}");
        assert!(files_in(dir.path()).is_empty());
    }
}
