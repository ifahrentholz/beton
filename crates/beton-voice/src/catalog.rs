//! Gepinnte Modell-Liste (VOI-002, VOI-008 AC3, ADR-0033).
//!
//! Jedes Modell hat eine feste Download-URL auf einen festen Commit des Hugging-Face-Repos
//! `ggerganov/whisper.cpp`, die erwartete Größe und die SHA-256 der Datei. Die Werte sind im
//! Binary eingebettet; es gibt keinen Weg, zur Laufzeit andere URLs einzuschleusen. Ein
//! Download folgt Weiterleitungen nur per HTTPS zu den Hosts in [`Catalog::redirect_hosts`]
//! (Hugging Face leitet auf sein CDN um); die Prüfsumme sichert den Inhalt unabhängig davon.

use serde::Serialize;

/// Commit des Hugging-Face-Repos, auf den alle URLs zeigen.
pub const PINNED_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";

/// Ein Eintrag der gepinnten Liste.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelEntry {
    /// Name für CLI und Einstellungen, z. B. `small` oder `large-v3-turbo-q5_0`.
    pub name: String,
    /// Dateiname im Modellverzeichnis, z. B. `ggml-small.bin`.
    pub file: String,
    /// Feste Download-URL.
    pub url: String,
    /// Erwartete Größe in Bytes.
    pub size_bytes: u64,
    /// SHA-256 der Datei (hex, klein).
    pub sha256: String,
    /// Quantisierung, z. B. `q5_0`; `None` für die volle Genauigkeit (f16).
    pub quantization: Option<String>,
}

/// Rohdaten der eingebetteten Liste: Name, Datei, Größe, SHA-256, Quantisierung.
const PINNED: [(&str, &str, u64, &str, Option<&str>); 6] = [
    (
        "small",
        "ggml-small.bin",
        487_601_967,
        "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        None,
    ),
    (
        "small-q5_1",
        "ggml-small-q5_1.bin",
        190_085_487,
        "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        Some("q5_1"),
    ),
    (
        "medium",
        "ggml-medium.bin",
        1_533_763_059,
        "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        None,
    ),
    (
        "medium-q5_0",
        "ggml-medium-q5_0.bin",
        539_212_467,
        "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
        Some("q5_0"),
    ),
    (
        "large-v3-turbo",
        "ggml-large-v3-turbo.bin",
        1_624_555_275,
        "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        None,
    ),
    (
        "large-v3-turbo-q5_0",
        "ggml-large-v3-turbo-q5_0.bin",
        574_041_195,
        "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        Some("q5_0"),
    ),
];

/// Hosts, zu denen ein Download weitergeleitet werden darf (nur HTTPS): Hugging Face und sein
/// CDN (`*.hf.co`).
const PINNED_REDIRECT_HOSTS: [&str; 2] = ["huggingface.co", "hf.co"];

/// Die Modell-Liste samt erlaubter Weiterleitungsziele.
#[derive(Debug, Clone)]
pub struct Catalog {
    entries: Vec<ModelEntry>,
    redirect_hosts: Vec<String>,
    /// Nur Tests: HTTP auf Loopback statt HTTPS (Mock-Server).
    allow_loopback_http: bool,
}

impl Catalog {
    /// Die im Binary eingebettete, gepinnte Liste (VOI-002, VOI-008 AC3).
    pub fn pinned() -> Self {
        let entries = PINNED
            .iter()
            .map(|(name, file, size, sha, quant)| ModelEntry {
                name: (*name).to_owned(),
                file: (*file).to_owned(),
                url: format!(
                    "https://huggingface.co/ggerganov/whisper.cpp/resolve/{PINNED_REVISION}/{file}"
                ),
                size_bytes: *size,
                sha256: (*sha).to_owned(),
                quantization: quant.map(str::to_owned),
            })
            .collect();
        Self {
            entries,
            redirect_hosts: PINNED_REDIRECT_HOSTS.map(str::to_owned).to_vec(),
            allow_loopback_http: false,
        }
    }

    /// Liste für Unit-Tests mit einem Mock-Server auf Loopback.
    #[cfg(test)]
    pub(crate) fn for_tests(entries: Vec<ModelEntry>) -> Self {
        Self {
            entries,
            redirect_hosts: vec!["127.0.0.1".to_owned()],
            allow_loopback_http: true,
        }
    }

    pub fn entries(&self) -> &[ModelEntry] {
        &self.entries
    }

    /// Eintrag nach Name.
    pub fn get(&self, name: &str) -> Option<&ModelEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Eintrag nach Prüfsumme (Import, VOI-002 AC5).
    pub fn by_sha256(&self, sha256: &str) -> Option<&ModelEntry> {
        self.entries
            .iter()
            .find(|e| e.sha256.eq_ignore_ascii_case(sha256))
    }

    pub fn redirect_hosts(&self) -> &[String] {
        &self.redirect_hosts
    }

    /// Darf ein Download diese URL ansteuern? Die Start-URL muss exakt aus der Liste stammen;
    /// Weiterleitungen nur per HTTPS zu einem erlaubten Host oder einer seiner Subdomains.
    pub fn allows_redirect(&self, url: &reqwest::Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        let scheme_ok = url.scheme() == "https"
            || (self.allow_loopback_http && url.scheme() == "http" && host == "127.0.0.1");
        scheme_ok
            && self
                .redirect_hosts
                .iter()
                .any(|h| host == h || host.ends_with(&format!(".{h}")))
    }

    /// Default-Modell je Hardware (VOI-002): `large-v3-turbo` auf Apple Silicon bzw. mit
    /// GPU-Beschleunigung, sonst `small`.
    pub fn default_model(gpu: bool) -> &'static str {
        if gpu || cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            "large-v3-turbo"
        } else {
            "small"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VOI-008 AC3: Die eingebettete Liste zeigt nur auf HTTPS-URLs von Hugging Face mit
    /// festem Commit; jeder Eintrag hat Größe und SHA-256.
    #[test]
    fn voi_008_ac3_pinned_list_only_has_https_urls_on_a_fixed_revision() {
        let catalog = Catalog::pinned();
        assert!(!catalog.entries().is_empty());
        for e in catalog.entries() {
            assert_eq!(
                e.url,
                format!(
                    "https://huggingface.co/ggerganov/whisper.cpp/resolve/{PINNED_REVISION}/{}",
                    e.file
                )
            );
            assert_eq!(e.sha256.len(), 64, "{}", e.name);
            assert!(e.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(e.size_bytes > 0);
        }
        // Die Spec nennt small, medium und large-v3-turbo, jeweils optional quantisiert.
        for name in ["small", "medium", "large-v3-turbo"] {
            assert!(catalog.get(name).is_some(), "{name}");
            assert!(
                catalog
                    .entries()
                    .iter()
                    .any(|e| e.name.starts_with(name) && e.quantization.is_some()),
                "{name} quantisiert"
            );
        }
    }

    /// VOI-008 AC3: Weiterleitungen nur per HTTPS zu Hugging Face bzw. dessen CDN.
    #[test]
    fn voi_008_ac3_redirects_only_to_pinned_hosts_over_https() {
        let catalog = Catalog::pinned();
        let ok = |u: &str| catalog.allows_redirect(&reqwest::Url::parse(u).unwrap());
        assert!(ok("https://huggingface.co/x"));
        assert!(ok("https://cdn-lfs.hf.co/x"));
        assert!(ok("https://us.aws.cdn.hf.co/xet-bridge-us/1"));
        assert!(!ok("http://huggingface.co/x"));
        assert!(!ok("https://evil.example/x"));
        assert!(!ok("https://huggingface.co.evil.example/x"));
        assert!(!ok("https://nothf.co/x"));
        assert!(!ok("http://127.0.0.1/x"));
    }
}
