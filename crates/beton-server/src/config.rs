//! Server-Konfiguration für den lokalen Modus (AUTH-001, AUTH-002, AUTH-003).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;

/// Standard-Port des lokalen Daemons.
pub const DEFAULT_PORT: u16 = 7420;

/// Feste Origins der Desktop-App (Tauri 2).
pub const TAURI_ORIGINS: [&str; 2] = ["tauri://localhost", "http://tauri.localhost"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// Datenverzeichnis, z. B. `~/.beton` (Token-Datei, Socket, Datenbank).
    pub data_dir: PathBuf,
    /// TCP-Adressen; lokal nur Loopback (Default `127.0.0.1:7420` und `[::1]:7420`).
    pub listen: Vec<SocketAddr>,
    /// Unix-Socket (Default `<data_dir>/run/beton.sock`); `None` = keiner.
    pub socket: Option<PathBuf>,
    /// Zusätzlich erlaubte `Host`-Header (`server.allowed_hosts`).
    pub allowed_hosts: Vec<String>,
    /// Zusätzlich erlaubte Origins für WebSocket und Cookie-Requests (`auth.ws_allowed_origins`).
    pub allowed_origins: Vec<String>,
}

impl ServerConfig {
    /// Lokaler Modus mit Defaults.
    pub fn local(data_dir: PathBuf) -> Self {
        Self {
            listen: vec![
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_PORT),
                SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), DEFAULT_PORT),
            ],
            socket: cfg!(unix).then(|| data_dir.join("run").join("beton.sock")),
            allowed_hosts: Vec::new(),
            allowed_origins: Vec::new(),
            data_dir,
        }
    }

    /// Prüft die Konfiguration vor dem Start (AUTH-001 AC4): Im lokalen Modus ist nur
    /// Loopback erlaubt; Nicht-Loopback braucht Pairing/TLS (AUTH-009, ab M4).
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.listen.is_empty() && self.socket.is_none() {
            return Err(ConfigError::NothingToListenOn);
        }
        for addr in &self.listen {
            if !addr.ip().is_loopback() {
                return Err(ConfigError::NonLoopback(*addr));
            }
        }
        Ok(())
    }

    /// Erlaubte `Host`-Header für die tatsächlich gebundenen Ports (AUTH-002).
    pub fn host_allowlist(&self, ports: &[u16]) -> Vec<String> {
        let mut hosts = Vec::new();
        for port in ports {
            hosts.push(format!("127.0.0.1:{port}"));
            hosts.push(format!("[::1]:{port}"));
            hosts.push(format!("localhost:{port}"));
        }
        hosts.extend(self.allowed_hosts.iter().cloned());
        hosts
    }

    /// Erlaubte Origins für die gebundenen Ports: Loopback, Tauri und Konfiguration (AUTH-003).
    pub fn origin_allowlist(&self, ports: &[u16]) -> Vec<String> {
        let mut origins = Vec::new();
        for port in ports {
            origins.push(format!("http://127.0.0.1:{port}"));
            origins.push(format!("http://[::1]:{port}"));
            origins.push(format!("http://localhost:{port}"));
        }
        origins.extend(TAURI_ORIGINS.iter().map(|o| (*o).to_owned()));
        origins.extend(self.allowed_origins.iter().cloned());
        origins
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "server.listen {0} ist keine Loopback-Adresse. Im lokalen Modus bindet beton nur an \
         127.0.0.1 und ::1; Zugriff von anderen Geräten folgt mit Pairing und TLS (AUTH-009)."
    )]
    NonLoopback(SocketAddr),
    #[error("weder server.listen noch ein Socket konfiguriert")]
    NothingToListenOn,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_001_ac4_non_loopback_listen_is_rejected_in_local_mode() {
        let mut cfg = ServerConfig::local("/tmp/x".into());
        assert!(cfg.validate().is_ok());
        cfg.listen = vec!["0.0.0.0:7420".parse().unwrap()];
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, ConfigError::NonLoopback(_)));
        assert!(err.to_string().contains("AUTH-009"));
        cfg.listen = vec!["192.168.1.10:7420".parse().unwrap()];
        assert!(cfg.validate().is_err());
        cfg.listen = vec!["[::]:7420".parse().unwrap()];
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn default_port_and_allowlists() {
        let cfg = ServerConfig::local("/tmp/x".into());
        assert!(cfg.listen.iter().all(|a| a.port() == DEFAULT_PORT));
        let hosts = cfg.host_allowlist(&[7420]);
        assert!(hosts.contains(&"localhost:7420".to_owned()));
        assert!(!hosts.iter().any(|h| h == "localhost"));
        let origins = cfg.origin_allowlist(&[7420]);
        assert!(origins.contains(&"tauri://localhost".to_owned()));
        assert!(origins.contains(&"http://127.0.0.1:7420".to_owned()));
    }
}
