//! `beton doctor` (OBS-005, CLI-005, HAR-003 AC3): Umgebung prüfen, nichts ändern.
//!
//! M0-Prüfungen: Version, Daemon, Konfiguration, Rechte von `~/.beton` und Token-Datei,
//! SQLite-Integrität, Harness-CLIs (Pfad, Version, Bereich, Quelle) und Login-Status über
//! das Statuskommando der CLI. `doctor` schreibt nichts und kontaktiert nur den lokalen
//! Daemon über Loopback (ADR-0033). Exit-Code 0 (ok), 1 (Warnungen), 2 (Fehler).

use std::path::Path;

use beton_harness::registry::{HarnessLayers, resolve_binary};
use beton_harness::{AuthStatus, HostEnv};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::Ctx;
use crate::setup::VENDOR_CLIS;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

/// Eine Prüfung.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// Stabile Kennung, z. B. `daemon` oder `harness.claude`.
    pub id: String,
    pub status: CheckStatus,
    pub message: String,
    /// Handlungsempfehlung; `null`, wenn nichts zu tun ist.
    pub hint: Option<String>,
    /// Zusätzliche Angaben, z. B. Pfad und Version eines Harness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Ausgabe von `beton doctor --json` (Schema: `schemas/v1/doctor.schema.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DoctorReport {
    pub version: String,
    /// Schlechtester Status aller Prüfungen.
    pub status: CheckStatus,
    pub checks: Vec<Check>,
}

impl DoctorReport {
    /// Exit-Code laut OBS-005.
    pub fn exit_code(&self) -> u8 {
        match self.status {
            CheckStatus::Ok => 0,
            CheckStatus::Warn => 1,
            CheckStatus::Fail => 2,
        }
    }
}

fn check(id: &str, status: CheckStatus, message: impl Into<String>, hint: Option<&str>) -> Check {
    Check {
        id: id.to_owned(),
        status,
        message: message.into(),
        hint: hint.map(str::to_owned),
        details: None,
    }
}

/// Rechte einer Datei bzw. eines Verzeichnisses: nur der Eigentümer (Unix).
#[cfg(unix)]
fn private(path: &Path, max: u32) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let mode = meta.mode() & 0o777;
    if mode & !max != 0 {
        return Err(format!("Rechte {mode:o}, erlaubt {max:o}"));
    }
    let uid = rustix::process::getuid().as_raw();
    if meta.uid() != uid {
        return Err(format!("gehört uid {}, erwartet {uid}", meta.uid()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn private(path: &Path, _max: u32) -> Result<(), String> {
    std::fs::metadata(path)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn permissions(home: &Path) -> Check {
    let token = beton_sdk::local_token_path(home);
    let auth_dir = token.parent().map(Path::to_path_buf).unwrap_or_default();
    if !token.exists() {
        return check(
            "home.permissions",
            CheckStatus::Ok,
            format!(
                "{}: noch kein lokales Token (legt `beton serve` an)",
                home.display()
            ),
            None,
        );
    }
    let problems: Vec<String> = [(&auth_dir, 0o700), (&token, 0o600)]
        .iter()
        .filter_map(|(p, max)| {
            private(p, *max)
                .err()
                .map(|e| format!("{}: {e}", p.display()))
        })
        .collect();
    if problems.is_empty() {
        check(
            "home.permissions",
            CheckStatus::Ok,
            "Token-Datei und Verzeichnis nur für den Eigentümer lesbar",
            None,
        )
    } else {
        check(
            "home.permissions",
            CheckStatus::Fail,
            problems.join("; "),
            Some(&format!(
                "chmod 700 {} && chmod 600 {}",
                auth_dir.display(),
                token.display()
            )),
        )
    }
}

async fn database(home: &Path) -> Check {
    let db = home.join("beton.db");
    if !db.exists() {
        return check(
            "database",
            CheckStatus::Ok,
            "Noch keine Datenbank (legt `beton serve` an)",
            None,
        );
    }
    match beton_store::quick_check(&db).await {
        Ok(problems) if problems.is_empty() => check(
            "database",
            CheckStatus::Ok,
            format!("{}: quick_check ok", db.display()),
            None,
        ),
        Ok(problems) => check(
            "database",
            CheckStatus::Fail,
            format!("{}: {}", db.display(), problems.join("; ")),
            Some("Daemon stoppen und die letzte Sicherung `beton.db.bak-*` zurückspielen"),
        ),
        Err(e) => check(
            "database",
            CheckStatus::Warn,
            format!("{}: nicht prüfbar ({e})", db.display()),
            Some("Rechte des Datenverzeichnisses prüfen"),
        ),
    }
}

async fn daemon(home: &Path) -> Check {
    match crate::daemon::running(home).await {
        Some(client) => {
            let pid = beton_sdk::DaemonInfo::read(home).map(|d| d.pid);
            let mut c = check(
                "daemon",
                CheckStatus::Ok,
                format!("läuft auf {}", client.base_url()),
                None,
            );
            c.details = Some(json!({ "url": client.base_url(), "pid": pid }));
            c
        }
        None => check(
            "daemon",
            CheckStatus::Warn,
            "kein lokaler Daemon erreichbar",
            Some("beton serve (startet ihn im Hintergrund; `beton run` tut das automatisch)"),
        ),
    }
}

fn auth_word(s: AuthStatus) -> &'static str {
    match s {
        AuthStatus::LoggedIn => "angemeldet",
        AuthStatus::LoggedOut => "nicht angemeldet",
        AuthStatus::Unknown | AuthStatus::NotApplicable => "unbekannt",
    }
}

async fn harnesses(env: &HostEnv, layers: &HarnessLayers) -> Vec<Check> {
    let (registry, problems) = beton_runner::builtin_registry_with_problems(layers, false);
    let catalog = registry.catalog(env).await;
    let mut out = Vec::new();
    if !problems.is_empty() {
        // HAR-008 AC3: übersprungene Einträge mit Datei und Zeile.
        let mut c = check(
            "config.acp",
            CheckStatus::Warn,
            problems
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
            Some("Eintrag unter harnesses.acp.agents korrigieren"),
        );
        c.details = Some(
            json!({ "skipped": problems.iter().map(ToString::to_string).collect::<Vec<_>>() }),
        );
        out.push(c);
    }
    for info in catalog {
        let id = info.id.to_string();
        let Some(adapter) = registry.get(&info.id) else {
            continue;
        };
        let cli = VENDOR_CLIS.iter().find(|c| c.id == id);
        let source =
            resolve_binary(&info.id, cli.map_or(id.as_str(), |c| c.command), env).map(|b| b.source);
        let range = info
            .capabilities
            .first()
            .and_then(|c| c.version_range.clone());
        if !info.probe.installed && id.starts_with("acp:") {
            // Presets sind optional und nur aktiv, wenn ihr Binary gefunden wird (HAR-008).
            out.push(check(
                &format!("harness.{id}"),
                CheckStatus::Ok,
                format!("{id}: nicht installiert (optional)"),
                None,
            ));
            continue;
        }
        if !info.probe.installed {
            out.push(check(
                &format!("harness.{id}"),
                CheckStatus::Warn,
                format!(
                    "{id}: nicht gefunden (PATH, {}, harnesses.{id}.command)",
                    info.id.path_env_var()
                ),
                Some(&format!(
                    "Installieren mit: {}",
                    cli.map_or("siehe Dokumentation des Vendors", |c| c.install)
                )),
            ));
            continue;
        }
        let auth = adapter.auth_status(env).await;
        let details = json!({
            "path": info.probe.path,
            "version": info.probe.version,
            "version_range": range,
            "compatible": !info.incompatible,
            "source": source,
            "auth_status": auth,
        });
        let summary = format!(
            "{id} {} ({}, Quelle {}), {}",
            info.probe.version.as_deref().unwrap_or("Version unbekannt"),
            info.probe.path.as_deref().unwrap_or("?"),
            source.map_or("?".into(), |s| serde_json::to_value(s)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()),
            if info.incompatible {
                "inkompatibel"
            } else {
                "kompatibel"
            },
        );
        let mut c = if info.incompatible {
            check(
                &format!("harness.{id}"),
                CheckStatus::Fail,
                format!(
                    "{summary}: {}",
                    info.incompatible_reason
                        .as_deref()
                        .unwrap_or("Version passt nicht")
                ),
                Some(&format!(
                    "Version im Bereich {} installieren",
                    range.as_deref().unwrap_or("?")
                )),
            )
        } else if let Some(failed) = &info.probe.probe_failed {
            check(
                &format!("harness.{id}"),
                CheckStatus::Warn,
                format!("{summary}: {failed}"),
                Some("Installation der CLI prüfen"),
            )
        } else {
            check(&format!("harness.{id}"), CheckStatus::Ok, summary, None)
        };
        c.details = Some(details);
        out.push(c);

        let login = cli.map(|c| format!("{} {}", c.command, c.login.join(" ")));
        out.push(match auth {
            AuthStatus::LoggedOut => check(
                &format!("auth.{id}"),
                CheckStatus::Warn,
                format!("{id}: {}", auth_word(auth)),
                login.as_deref(),
            ),
            _ => check(
                &format!("auth.{id}"),
                CheckStatus::Ok,
                format!("{id}: {}", auth_word(auth)),
                None,
            ),
        });
    }
    out
}

/// Feature-Flags aus Config und `BETON_FEATURES` (UX-007 AC2, AC3): Unbekanntes und
/// Entferntes ist eine Warnung, kein Fehler.
pub fn features(config: &[String], env: Option<&str>) -> Check {
    use beton_core::feature::FeatureSet;
    let set = FeatureSet::from_sources(config, env);
    let active: Vec<Value> = set
        .active()
        .map(|(id, source)| json!({"id": id, "source": source}))
        .collect();
    let problems = set.problems();
    let mut c = if problems.is_empty() {
        let names: Vec<&str> = set.active().map(|(id, _)| id).collect();
        check(
            "features",
            CheckStatus::Ok,
            if names.is_empty() {
                "keine experimentellen Funktionen eingeschaltet".to_owned()
            } else {
                format!("{} aktiv", names.join(", "))
            },
            None,
        )
    } else {
        let hint = problems
            .iter()
            .find_map(|p| p.suggestion.map(|s| format!("Meintest du „{s}“?")))
            .unwrap_or_else(|| {
                "Namen in BETON_FEATURES bzw. `features:` der Konfiguration prüfen".into()
            });
        check(
            "features",
            CheckStatus::Warn,
            problems
                .iter()
                .map(beton_core::feature::FlagProblem::message)
                .collect::<Vec<_>>()
                .join("; "),
            Some(&hint),
        )
    };
    c.details = Some(json!({
        "active": active,
        "unknown": problems.iter().filter(|p| !p.removed).map(|p| &p.id).collect::<Vec<_>>(),
        "removed": problems.iter().filter(|p| p.removed).map(|p| &p.id).collect::<Vec<_>>(),
    }));
    c
}

/// Führt alle M0-Prüfungen aus.
pub async fn run(ctx: &Ctx) -> DoctorReport {
    let mut checks = vec![check(
        "version",
        CheckStatus::Ok,
        format!(
            "beton {} (lokaler Build; keine Online-Update-Prüfung)",
            crate::VERSION
        ),
        None,
    )];
    checks.push(daemon(&ctx.home).await);
    let layers = match ctx.layers() {
        Ok(layers) => {
            checks.push(check(
                "config",
                CheckStatus::Ok,
                format!(
                    "gültig ({}, {})",
                    layers.paths.user.display(),
                    layers.paths.project.display()
                ),
                None,
            ));
            Some(layers)
        }
        Err(e) => {
            checks.push(check(
                "config",
                CheckStatus::Fail,
                e.to_string(),
                Some("beton config edit"),
            ));
            None
        }
    };
    let configured = layers
        .as_ref()
        .and_then(|l| l.daemon_settings().ok())
        .map(|s| s.features)
        .unwrap_or_default();
    checks.push(features(
        &configured,
        std::env::var(beton_core::feature::ENV_VAR).ok().as_deref(),
    ));
    checks.push(permissions(&ctx.home));
    checks.push(database(&ctx.home).await);

    let harness_layers = layers
        .as_ref()
        .and_then(|l| l.harness_layers().ok())
        .unwrap_or_default();
    let env = HostEnv {
        user: harness_layers.user.clone(),
        project: harness_layers.project.clone(),
        ..HostEnv::from_process()
    };
    checks.extend(harnesses(&env, &harness_layers).await);

    let status = checks
        .iter()
        .map(|c| c.status)
        .max()
        .unwrap_or(CheckStatus::Ok);
    DoctorReport {
        version: crate::VERSION.to_owned(),
        status,
        checks,
    }
}

/// Tabelle für das Terminal.
pub fn render(report: &DoctorReport, out: &mut dyn std::io::Write) -> std::io::Result<()> {
    for c in &report.checks {
        let mark = match c.status {
            CheckStatus::Ok => "ok  ",
            CheckStatus::Warn => "warn",
            CheckStatus::Fail => "FAIL",
        };
        writeln!(out, "{mark}  {:<18} {}", c.id, c.message)?;
        if let Some(hint) = &c.hint {
            writeln!(out, "      {:<18} → {hint}", "")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worst_status_sets_exit_code() {
        let mut r = DoctorReport {
            version: "0".into(),
            status: CheckStatus::Ok,
            checks: Vec::new(),
        };
        assert_eq!(r.exit_code(), 0);
        r.status = CheckStatus::Warn;
        assert_eq!(r.exit_code(), 1);
        r.status = CheckStatus::Fail;
        assert_eq!(r.exit_code(), 2);
    }

    #[test]
    fn ux_007_ac2_unknown_flag_is_a_warning_with_suggestion() {
        let c = features(&[], Some("unknown_flag"));
        assert_eq!(c.status, CheckStatus::Warn);
        assert!(
            c.message
                .contains("BETON_FEATURES enthält unbekanntes Flag „unknown_flag“")
        );
        let c = features(&[], Some("fake_harnes"));
        assert_eq!(c.hint.as_deref(), Some("Meintest du „fake_harness“?"));
        let c = features(&["fake_harness".into()], None);
        assert_eq!(c.status, CheckStatus::Ok);
        assert_eq!(c.message, "fake_harness aktiv");
        assert_eq!(c.details.unwrap()["active"][0]["source"], "config");
    }

    #[cfg(unix)]
    #[test]
    fn obs_005_loose_token_permissions_fail() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            permissions(dir.path()).status,
            CheckStatus::Ok,
            "ohne Token ok"
        );
        std::fs::create_dir_all(dir.path().join("auth")).unwrap();
        std::fs::set_permissions(
            dir.path().join("auth"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let token = dir.path().join("auth/local.token");
        std::fs::write(&token, "x").unwrap();
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(permissions(dir.path()).status, CheckStatus::Ok);
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
        let c = permissions(dir.path());
        assert_eq!(c.status, CheckStatus::Fail);
        assert!(c.hint.unwrap().contains("chmod 600"));
    }
}
