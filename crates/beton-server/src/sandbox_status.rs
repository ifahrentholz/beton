//! Sandbox-Status des Hosts (SBX-006, HAR-027): Grundlage für die Vorprüfung beim Anlegen
//! einer Session (kein Start ohne nutzbare Sandbox, `yolo` nur mit Sandbox und Proxy) und für
//! Clients (`GET /v1/info`, Mode-Picker).
//!
//! Quelle ist allein `sandbox:` der User-Konfiguration samt Probe des gewählten Backends; die
//! Projekt-Konfiguration kann den Schlüssel nicht setzen (#200), ein Agent nur verschärfen
//! (AGT-014). Maßgeblich bleibt der Runner: Er baut die Stufen und prüft erneut (fail closed).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use beton_harness::SandboxStatus;
use beton_sandbox::{BackendChoice, ProviderId, Registry, SandboxSpec};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

use crate::problem::{Problem, ProblemCode};

/// So lange gilt eine Probe; danach prüft der Server neu (z. B. nach `sysctl`).
const PROBE_TTL: Duration = Duration::from_secs(30);

/// Sandbox-Status des Hosts für Clients (`GET /v1/info`, SBX-006, HAR-027).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
pub struct SandboxInfo {
    /// Konfiguriertes Backend: `auto` (Default), `seatbelt`, `landlock`, `bwrap`, … oder
    /// `none` (ausdrücklich ohne Isolation, je Session zu bestätigen).
    pub backend: String,
    /// Gewähltes Backend dieses Rechners (bei `auto`); fehlt bei `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub provider: Option<String>,
    /// Sessions starten mit Sandbox: Das Backend ist auf diesem Rechner nutzbar.
    pub active: bool,
    /// Netz der Tools nur über den Egress-Proxy (oder gar keins, `allow_network: false`).
    pub proxy: bool,
    /// `yolo` ist startbar (für Harnesses, deren Tools in Stufe 2 laufen).
    pub yolo: bool,
    /// Was dem Backend fehlt, wenn es nicht nutzbar ist (z. B. `net`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    /// Hinweis zur Einrichtung, wenn keine Sandbox aktiv ist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hint: Option<String>,
}

impl SandboxInfo {
    /// Schutz für die Tools eines Harness; `inherited` heißt: Tools laufen im Harness-Prozess
    /// (Stufe 1, *degraded*), also keine Tool-Sandbox Stufe 2.
    pub fn status(&self, inherited_tools: bool) -> SandboxStatus {
        SandboxStatus {
            tool_sandbox: self.active && !inherited_tools,
            egress_proxy: self.active && self.proxy,
        }
    }

    /// `503 sandbox_unavailable` mit Backend, fehlenden Capabilities und Hinweis.
    pub fn unavailable(&self) -> Problem {
        let provider = self
            .provider
            .clone()
            .unwrap_or_else(|| self.backend.clone());
        let hint = self.hint.clone().unwrap_or_default();
        let missing = if self.missing.is_empty() {
            String::new()
        } else {
            format!(" (fehlt: {})", self.missing.join(", "))
        };
        Problem::new(ProblemCode::SandboxUnavailable)
            .detail(format!(
                "Session startet nur mit Sandbox; {provider} ist auf diesem Rechner nicht \
                 nutzbar: {hint}{missing}"
            ))
            .sandbox(provider, self.missing.clone(), self.hint.clone())
    }
}

/// Zwischenspeicher der Probe (die Linux-Probe startet Prozesse).
#[derive(Debug, Default)]
pub struct ProbeCache {
    last: Mutex<Option<(Instant, String, Probed)>>,
}

#[derive(Debug, Clone)]
struct Probed {
    provider: Option<ProviderId>,
    available: bool,
    missing: Vec<String>,
    reason: Option<String>,
}

impl ProbeCache {
    /// Status für `spec` (User-Konfiguration); `proxy_ca`: Der Daemon hat eine Quelle für die
    /// beton-CA (ohne sie kein Netz im Stufenmodell, PRX-002).
    pub async fn info(&self, spec: &SandboxSpec, proxy_ca: bool) -> SandboxInfo {
        let spec = SandboxSpec::merged(&[spec]);
        let choice = spec.backend.unwrap_or(BackendChoice::Auto);
        let backend = match choice {
            BackendChoice::Auto => "auto".to_owned(),
            BackendChoice::Provider(id) => id.as_str().to_owned(),
        };
        if spec.unsandboxed() {
            return SandboxInfo {
                backend,
                provider: None,
                active: false,
                proxy: false,
                yolo: false,
                missing: Vec::new(),
                hint: Some(
                    "`sandbox.backend: none`: jede Session braucht eine Bestätigung, YOLO \
                     bleibt gesperrt. Ohne `backend` (Default `auto`) laufen Sessions in der \
                     Sandbox (`beton sandbox probe`)."
                        .into(),
                ),
            };
        }
        let probed = self.probe(&backend, choice).await;
        let proxy = spec.allow_network == Some(false) || proxy_ca;
        let active = probed.available;
        let hint = if active {
            (!proxy).then(|| {
                "Keine Quelle für die beton-CA: Sessions mit Netz starten nicht (PRX-002)".into()
            })
        } else {
            Some(probed.reason.clone().unwrap_or_else(|| {
                "Backend nicht nutzbar; `beton doctor` bzw. `beton sandbox probe` nennen den \
                 Grund"
                    .into()
            }))
        };
        SandboxInfo {
            backend,
            provider: probed.provider.map(|p| p.as_str().to_owned()),
            active,
            proxy,
            yolo: active && proxy,
            missing: if active { Vec::new() } else { probed.missing },
            hint,
        }
    }

    async fn probe(&self, key: &str, choice: BackendChoice) -> Probed {
        if let Ok(last) = self.last.lock()
            && let Some((at, k, p)) = last.as_ref()
            && k == key
            && at.elapsed() < PROBE_TTL
        {
            return p.clone();
        }
        let probed = tokio::task::spawn_blocking(move || probe_now(choice))
            .await
            .unwrap_or_else(|e| Probed {
                provider: None,
                available: false,
                missing: Vec::new(),
                reason: Some(format!("Probe gescheitert: {e}")),
            });
        if let Ok(mut last) = self.last.lock() {
            *last = Some((Instant::now(), key.to_owned(), probed.clone()));
        }
        probed
    }
}

fn probe_now(choice: BackendChoice) -> Probed {
    let registry = Registry::platform();
    let provider = match registry.select(choice) {
        Ok(p) => p,
        Err(e) => {
            return Probed {
                provider: None,
                available: false,
                missing: beton_sandbox::caps::required_default()
                    .iter()
                    .map(|c| c.as_str().to_owned())
                    .collect(),
                reason: Some(e.to_string()),
            };
        }
    };
    let report = provider.probe();
    let missing: Vec<String> = beton_sandbox::caps::required_default()
        .iter()
        .filter(|c| !report.caps.contains(c))
        .map(|c| c.as_str().to_owned())
        .collect();
    Probed {
        provider: Some(provider.id()),
        // Die Pflicht-Capabilities muss das Backend erzwingen (SBX-006, kein Fallback).
        available: report.available && missing.is_empty(),
        missing,
        reason: report
            .unavailable_reason
            .clone()
            .or_else(|| report.degraded_reasons.values().next().cloned()),
    }
}
