//! `beton doctor` (OBS-005, CLI-005, HAR-003 AC3): Umgebung prüfen, nichts ändern.
//!
//! M0-Prüfungen: Version, Daemon, Konfiguration, Rechte von `~/.beton` und Token-Datei,
//! SQLite-Integrität, Harness-CLIs (Pfad, Version, Bereich, Quelle) und Login-Status über
//! das Statuskommando der CLI. Ab M1: verwaiste Worktrees ohne Session (SES-016 AC3). Ab M2:
//! Sandbox-Fähigkeiten (`sandbox.linux`, `sandbox.macos`). Ab M3: Spracheingabe (`voice`, Backend
//! und Real-Time-Factor, VOI-003). `doctor` schreibt nichts und kontaktiert nur den lokalen
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

/// Größe für Menschen, z. B. `12,3 MiB`.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit]).replace('.', ",")
    }
}

/// Worktree-Verzeichnisse unter `<home>/worktrees` ohne Session (SES-016 AC3).
async fn worktrees(home: &Path) -> Check {
    let root = home.join("worktrees");
    if !root.is_dir() {
        return check(
            "worktrees",
            CheckStatus::Ok,
            "keine Session-Worktrees",
            None,
        );
    }
    let db = home.join("beton.db");
    let known: Vec<std::path::PathBuf> = if db.exists() {
        match beton_store::worktree_paths(&db).await {
            Ok(paths) => paths.into_iter().map(Into::into).collect(),
            Err(e) => {
                return check(
                    "worktrees",
                    CheckStatus::Warn,
                    format!("verwaiste Worktrees nicht prüfbar ({e})"),
                    Some("Daemon einmal starten (`beton serve`), damit die Datenbank aktuell ist"),
                );
            }
        }
    } else {
        Vec::new()
    };
    let orphans = match beton_git::worktree::orphans(&root, &known) {
        Ok(o) => o,
        Err(e) => {
            return check(
                "worktrees",
                CheckStatus::Warn,
                format!("{}: nicht lesbar ({e})", root.display()),
                None,
            );
        }
    };
    if orphans.is_empty() {
        return check(
            "worktrees",
            CheckStatus::Ok,
            format!("{} Session-Worktree(s), keine verwaisten", known.len()),
            None,
        );
    }
    let list: Vec<String> = orphans
        .iter()
        .map(|o| format!("{} ({})", o.path.display(), human_size(o.size)))
        .collect();
    let mut c = check(
        "worktrees",
        CheckStatus::Warn,
        format!(
            "{} verwaiste(r) Worktree(s) ohne Session: {}",
            orphans.len(),
            list.join(", ")
        ),
        Some(
            "Inhalt prüfen, dann im Repository `git worktree remove <pfad>` und `git worktree prune`",
        ),
    );
    c.details = Some(json!({
        "orphans": orphans
            .iter()
            .map(|o| json!({ "path": o.path.display().to_string(), "size_bytes": o.size }))
            .collect::<Vec<_>>(),
    }));
    c
}

/// Secret-Store (SEC-002 AC2): welches Backend der Daemon nimmt. Prüft nur die Verfügbarkeit,
/// liest und ändert keinen Keychain-Eintrag.
pub fn secrets_store(
    choice: beton_secrets::select::BackendChoice,
    keychain: &beton_secrets::backend::keychain::Availability,
    home: &Path,
) -> Check {
    use beton_secrets::select::{Plan, plan};
    let file = beton_secrets::backend::FileBackend::default_path(home);
    let file_state = if file.exists() {
        format!("{} vorhanden", file.display())
    } else {
        format!(
            "{} wird mit `beton secrets unlock` angelegt",
            file.display()
        )
    };
    let mut c = match plan(choice, keychain) {
        Plan::Keychain => check(
            "secrets.store",
            CheckStatus::Ok,
            "OS-Keychain (Service `beton`)",
            None,
        ),
        Plan::File { reason: Some(_) } => check(
            "secrets.store",
            CheckStatus::Ok,
            format!("Keychain nicht verfügbar → Datei-Store ({file_state})"),
            Some("Entsperren: beton secrets unlock oder BETON_SECRETS_PASSPHRASE_FILE"),
        ),
        Plan::File { reason: None } => check(
            "secrets.store",
            CheckStatus::Ok,
            format!("Datei-Store (secrets.backend: file; {file_state})"),
            Some("Entsperren: beton secrets unlock oder BETON_SECRETS_PASSPHRASE_FILE"),
        ),
        Plan::None { reason } => check(
            "secrets.store",
            CheckStatus::Warn,
            format!("Secrets nicht verfügbar: {reason}"),
            Some("beton config set secrets.backend auto"),
        ),
    };
    if let beton_secrets::backend::keychain::Availability::Unavailable(why) = keychain {
        c.details = Some(json!({ "keychain": "unavailable", "reason": why }));
    }
    c
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
    let (registry, problems, provider_problems) = beton_runner::builtin_registry_with_options(
        layers,
        false,
        &beton_harness_direct::DirectOptions::default(),
    );
    let catalog = registry.catalog(env).await;
    let mut out = Vec::new();
    if !provider_problems.is_empty() {
        // HAR-011 AC4: ungültige Provider mit Pfad und Grund; die übrigen bleiben nutzbar.
        let texts: Vec<String> = provider_problems.iter().map(ToString::to_string).collect();
        let mut c = check(
            "config.providers",
            CheckStatus::Warn,
            texts.join("; "),
            Some("Eintrag unter providers korrigieren"),
        );
        c.details = Some(json!({ "skipped": texts }));
        out.push(c);
    }
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
        if id.starts_with("direct:") {
            // Direkt-API (HAR-011): optional, kein Binary; nur der Key muss da sein.
            let auth = adapter.auth_status(env).await;
            let endpoint = info.probe.path.clone().unwrap_or_default();
            let mut c = match auth {
                AuthStatus::LoggedOut => check(
                    &format!("harness.{id}"),
                    CheckStatus::Warn,
                    format!("{id}: Direkt-API {endpoint}, API-Key fehlt"),
                    Some(
                        "Die unter providers.<name>.api_key_env genannte Variable für den Daemon setzen",
                    ),
                ),
                _ => check(
                    &format!("harness.{id}"),
                    CheckStatus::Ok,
                    format!("{id}: Direkt-API {endpoint}"),
                    None,
                ),
            };
            c.details = Some(json!({ "endpoint": endpoint, "auth_status": auth }));
            out.push(c);
            continue;
        }
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

/// Preis-Katalog und eigene Preise (USE-002, USE-003 AC4). Im zentralen Betrieb
/// (`--server`) gelten nur die Overrides des Servers; lokale Einträge sind dann wirkungslos.
pub fn pricing(settings: Option<&crate::config::Settings>, central: bool) -> Check {
    let catalog = beton_core::pricing::Catalog::embedded();
    let overrides = settings.map_or(0, |s| s.price_overrides().len());
    let catalog_text = format!(
        "Preis-Katalog {} ({} Modelle, mitgeliefert, kein Online-Abruf)",
        catalog.version(),
        catalog.models().len()
    );
    let mut c = if central && overrides > 0 {
        check(
            "pricing",
            CheckStatus::Warn,
            format!(
                "{catalog_text}; eigene Preise der User-Konfiguration ({overrides}) werden im \
                 zentralen Betrieb ignoriert"
            ),
            Some(
                "Im zentralen Betrieb gelten nur die Preis-Overrides des Servers (Admin); \
                 lokale Einträge unter pricing.overrides bzw. providers.*.models[].pricing \
                 entfernen oder dort anlegen",
            ),
        )
    } else if central {
        check(
            "pricing",
            CheckStatus::Ok,
            format!("{catalog_text}; zentraler Betrieb: es gelten die Preise des Servers"),
            None,
        )
    } else {
        check(
            "pricing",
            CheckStatus::Ok,
            format!("{catalog_text}; eigene Preise: {overrides}"),
            None,
        )
    };
    c.details = Some(json!({
        "catalog_version": catalog.version(),
        "models": catalog.models().len(),
        "overrides": overrides,
        "central": central,
    }));
    c
}

/// Sandbox-Fähigkeiten (OBS-005, ab M2; SBX-008, SBX-009): Linux braucht Landlock (Kernel
/// ≥ 5.19, ABI ≥ 2) und dazu unprivilegierte User-Namespaces oder bwrap; macOS `sandbox-exec`.
/// Die Probe aller Backends (SBX-010, wie `beton sandbox probe --json`) steht in `details`.
pub fn sandbox_check(
    os: beton_sandbox::Os,
    reports: &[beton_sandbox::ProbeReport],
    landlock_abi: Option<u32>,
) -> Check {
    let mut c = sandbox_status(os, reports, landlock_abi);
    c.details = Some(json!({ "providers": reports }));
    c
}

fn sandbox_status(
    os: beton_sandbox::Os,
    reports: &[beton_sandbox::ProbeReport],
    landlock_abi: Option<u32>,
) -> Check {
    use beton_sandbox::{Os, ProviderId};
    let find = |id: ProviderId| reports.iter().find(|r| r.id == id);
    let reason = |id: ProviderId| {
        find(id)
            .and_then(|r| r.unavailable_reason.clone())
            .unwrap_or_else(|| "nicht registriert".into())
    };
    let ok = |id: ProviderId| find(id).filter(|r| r.available);
    match os {
        Os::Linux => {
            let id = "sandbox.linux";
            if landlock_abi.unwrap_or(0) < 2 {
                let abi = landlock_abi.map_or_else(|| "fehlt".to_owned(), |a| format!("ABI {a}"));
                return check(
                    id,
                    CheckStatus::Fail,
                    format!(
                        "Landlock {abi}: Die Linux-Sandbox (landlock, bwrap) ist nicht verfügbar"
                    ),
                    Some(
                        "Kernel ≥ 5.19 mit aktivem Landlock (Boot-Parameter `lsm=` muss landlock \
                         enthalten) oder ab M5 backend: docker",
                    ),
                );
            }
            if let Some(r) = ok(ProviderId::Landlock) {
                return check(
                    id,
                    CheckStatus::Ok,
                    format!("landlock ({})", r.version),
                    None,
                );
            }
            if let Some(r) = ok(ProviderId::Bwrap) {
                return check(
                    id,
                    CheckStatus::Ok,
                    format!(
                        "bwrap ({}); landlock nicht nutzbar: {}",
                        r.version,
                        reason(ProviderId::Landlock)
                    ),
                    None,
                );
            }
            check(
                id,
                CheckStatus::Fail,
                format!(
                    "keine Linux-Sandbox nutzbar: {}",
                    reason(ProviderId::Landlock)
                ),
                Some(
                    "unprivilegierte User-Namespaces erlauben (AppArmor-Profil deploy/apparmor/beton) \
                     oder bubblewrap ≥ 0.8 installieren",
                ),
            )
        }
        Os::MacOs => match ok(ProviderId::Seatbelt) {
            Some(r) => check(
                "sandbox.macos",
                CheckStatus::Ok,
                format!("seatbelt ({})", r.version),
                None,
            ),
            None => check(
                "sandbox.macos",
                CheckStatus::Fail,
                format!("Seatbelt nicht nutzbar: {}", reason(ProviderId::Seatbelt)),
                Some("beton nicht innerhalb einer anderen Sandbox starten"),
            ),
        },
        // SBX-006 (Windows-Beta bis SBX-016): Default „Sandbox erforderlich“ heißt hier, dass
        // keine Session ohne ausdrücklich bestätigtes `backend: none` startet.
        Os::Windows => check(
            "sandbox.windows",
            CheckStatus::Warn,
            "Die Windows-Sandbox (Beta) folgt mit M5 (SBX-016); bis dahin starten Sessions nur \
             mit `sandbox.backend: none` und Bestätigung je Session, YOLO bleibt gesperrt",
            Some(
                "`sandbox: { backend: none }` in ~/.beton/config.yaml und jede Session mit \
                 `beton run --unsandboxed` bestätigen (steht im Audit-Log); mit Sandbox: WSL2",
            ),
        ),
        Os::Other => check(
            "sandbox",
            CheckStatus::Warn,
            "für dieses Betriebssystem gibt es kein Sandbox-Backend",
            None,
        ),
    }
}

/// Spracheingabe (VOI-003): gewähltes Backend und Real-Time-Factor; gemessen wird nur mit
/// einem installierten Modell (ohne Download, ADR-0033).
pub fn voice_check(d: &beton_voice::Diagnosis) -> Check {
    let backend = d.backend.as_str();
    let device = d
        .gpu_note
        .as_deref()
        .filter(|_| d.backend.is_gpu())
        .map(|n| format!(" ({n})"))
        .unwrap_or_default();
    let measured = match (&d.model, d.rtf) {
        (Some(model), Some(rtf)) => format!(
            "{model}, Echtzeitfaktor {} (5 s Testsignal)",
            format!("{rtf:.2}").replace('.', ",")
        ),
        (Some(model), None) => format!("{model}, nicht messbar"),
        (None, _) => format!(
            "kein Modell installiert (Echtzeitfaktor nach `beton voice models pull {}`)",
            beton_voice::default_model()
        ),
    };
    let mut c = if d.gpu_fallback {
        check(
            "voice",
            CheckStatus::Warn,
            format!(
                "Spracheingabe: GPU nicht nutzbar ({}), Diktate laufen auf der CPU; {measured}",
                d.gpu_note.as_deref().unwrap_or("unbekannter Grund")
            ),
            Some("Details im Daemon-Log; mit `voice.acceleration: cpu` bewusst auf der CPU"),
        )
    } else if let Some(error) = &d.error {
        check(
            "voice",
            CheckStatus::Warn,
            format!("Spracheingabe: {backend}{device}; {measured}: {error}"),
            Some(
                "Modell neu laden: beton voice models remove <modell> && beton voice models pull <modell>",
            ),
        )
    } else {
        check(
            "voice",
            CheckStatus::Ok,
            format!("Spracheingabe: {backend}{device}; {measured}"),
            None,
        )
    };
    c.details = Some(json!({
        "backend": backend,
        "gpu": d.gpu_note,
        "fallback": d.gpu_fallback,
        "model": d.model,
        "rtf": d.rtf,
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
    let daemon_settings = layers.as_ref().and_then(|l| l.daemon_settings().ok());
    checks.push(pricing(
        daemon_settings.as_ref(),
        ctx.global.server.is_some(),
    ));
    let secrets_choice = daemon_settings
        .as_ref()
        .map(|s| s.secrets.backend)
        .unwrap_or_default();
    let voice_settings = daemon_settings
        .as_ref()
        .map(|s| s.voice.clone())
        .unwrap_or_default();
    let configured = daemon_settings.map(|s| s.features).unwrap_or_default();
    checks.push(features(
        &configured,
        std::env::var(beton_core::feature::ENV_VAR).ok().as_deref(),
    ));
    checks.push(permissions(&ctx.home));
    checks.push(database(&ctx.home).await);
    checks.push(worktrees(&ctx.home).await);
    let sandboxes = beton_sandbox::Registry::platform();
    let reports = tokio::task::spawn_blocking(move || sandboxes.probe_all())
        .await
        .unwrap_or_default();
    checks.push(sandbox_check(
        beton_sandbox::Os::current(),
        &reports,
        beton_sandbox::linux::kernel_landlock_abi(),
    ));
    let home = ctx.home.clone();
    if let Ok(d) =
        tokio::task::spawn_blocking(move || beton_voice::diagnose(&home, &voice_settings)).await
    {
        checks.push(voice_check(&d));
    }
    checks.push(secrets_store(
        secrets_choice,
        &beton_secrets::backend::KeychainBackend::availability(),
        &ctx.home,
    ));

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
    #[test]
    fn sec_002_ac2_doctor_reports_file_store_without_keychain() {
        use beton_secrets::backend::keychain::{Availability, availability_for};
        use beton_secrets::select::BackendChoice;
        let home = std::path::Path::new("/tmp/beton-home");
        // Linux ohne D-Bus-Session.
        let none = availability_for("linux", None);
        let c = secrets_store(BackendChoice::Auto, &none, home);
        assert_eq!(c.status, CheckStatus::Ok);
        assert!(
            c.message.contains("Keychain nicht verfügbar → Datei-Store"),
            "{}",
            c.message
        );
        assert!(c.message.contains("secrets/store.v1"), "{}", c.message);
        let c = secrets_store(BackendChoice::Auto, &Availability::Available, home);
        assert!(c.message.contains("OS-Keychain"), "{}", c.message);
        let c = secrets_store(BackendChoice::Keychain, &none, home);
        assert_eq!(c.status, CheckStatus::Warn);
    }

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

    #[test]
    fn use_003_ac4_doctor_warns_that_local_overrides_are_ignored_centrally() {
        let settings: crate::config::Settings = serde_json::from_value(json!({
            "pricing": {"overrides": [
                {"match": {"provider": "ollama-local"}, "free": true}
            ]}
        }))
        .unwrap();
        let local = pricing(Some(&settings), false);
        assert_eq!(local.status, CheckStatus::Ok);
        assert!(
            local.message.ends_with("eigene Preise: 1"),
            "{}",
            local.message
        );
        let central = pricing(Some(&settings), true);
        assert_eq!(central.status, CheckStatus::Warn);
        assert!(
            central.message.contains("im zentralen Betrieb ignoriert"),
            "{}",
            central.message
        );
        assert!(
            central
                .hint
                .unwrap()
                .contains("Preis-Overrides des Servers")
        );
        assert_eq!(central.details.unwrap()["overrides"], 1);
        assert_eq!(pricing(None, true).status, CheckStatus::Ok);
    }

    #[test]
    fn obs_005_ac3_linux_without_landlock_fails_with_kernel_hint() {
        use beton_sandbox::{Os, ProbeReport, ProviderId};
        let reports = [
            ProbeReport::unavailable(ProviderId::Landlock, "Landlock ist nicht aktiv"),
            ProbeReport::unavailable(ProviderId::Bwrap, "bwrap fehlt"),
        ];
        for abi in [None, Some(1)] {
            let c = sandbox_check(Os::Linux, &reports, abi);
            assert_eq!(c.id, "sandbox.linux");
            assert_eq!(c.status, CheckStatus::Fail, "{c:?}");
            let hint = c.hint.unwrap();
            assert!(hint.contains("Kernel") && hint.contains("5.19"), "{hint}");
            assert!(hint.contains("lsm="), "{hint}");
        }
        // Landlock vorhanden, aber User-Namespaces gesperrt und kein bwrap: ebenfalls Fehler,
        // mit dem Grund der Probe.
        let c = sandbox_check(Os::Linux, &reports, Some(4));
        assert_eq!(c.status, CheckStatus::Fail);
        assert!(c.message.contains("Landlock ist nicht aktiv"), "{c:?}");
        // Mit einem verfügbaren Backend ist alles in Ordnung, Fallback inklusive.
        let mut ok = reports.clone();
        ok[1].available = true;
        ok[1].unavailable_reason = None;
        let c = sandbox_check(Os::Linux, &ok, Some(4));
        assert_eq!(c.status, CheckStatus::Ok, "{c:?}");
        assert!(c.message.contains("bwrap"), "{c:?}");
        ok[0].available = true;
        ok[0].unavailable_reason = None;
        let c = sandbox_check(Os::Linux, &ok, Some(4));
        assert!(c.message.contains("landlock"), "{c:?}");
        // macOS: Seatbelt; Windows: Beta erst mit M5.
        let seatbelt = [ProbeReport::unavailable(
            ProviderId::Seatbelt,
            "sandbox-exec fehlt",
        )];
        let c = sandbox_check(Os::MacOs, &seatbelt, None);
        assert_eq!(
            (c.id.as_str(), c.status),
            ("sandbox.macos", CheckStatus::Fail)
        );
        let c = sandbox_check(Os::Windows, &[], None);
        assert_eq!(
            (c.id.as_str(), c.status),
            ("sandbox.windows", CheckStatus::Warn)
        );
    }

    /// SBX-006, Windows-Beta: Ohne Backend startet eine Session nur mit bestätigtem
    /// `backend: none`; `doctor` sagt das samt Weg.
    #[test]
    fn sbx_006_windows_doctor_explains_backend_none_with_confirmation() {
        let c = sandbox_check(beton_sandbox::Os::Windows, &[], None);
        assert!(c.message.contains("backend: none"), "{}", c.message);
        assert!(c.message.contains("YOLO"), "{}", c.message);
        let hint = c.hint.unwrap_or_default();
        assert!(hint.contains("--unsandboxed"), "{hint}");
        assert!(hint.contains("Audit-Log"), "{hint}");
    }

    /// SBX-010: `doctor` bindet die Probe ein; die Angaben stehen in `details` des Checks
    /// `sandbox.<os>` (wie `beton sandbox probe --json`).
    #[test]
    fn sbx_010_doctor_sandbox_check_carries_the_probe() {
        use beton_sandbox::{Os, ProbeReport, ProviderId};
        let reports = [
            ProbeReport::unavailable(ProviderId::Landlock, "Landlock ist nicht aktiv"),
            ProbeReport::unavailable(ProviderId::Bwrap, "bwrap fehlt"),
        ];
        for (os, abi) in [(Os::Linux, None), (Os::Linux, Some(4)), (Os::MacOs, None)] {
            let c = sandbox_check(os, &reports, abi);
            let details = c.details.unwrap_or_else(|| panic!("{os:?}: keine details"));
            let ids: Vec<&str> = details["providers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["id"].as_str().unwrap())
                .collect();
            assert_eq!(ids, ["landlock", "bwrap"], "{details}");
            assert_eq!(details["providers"][1]["available"], false);
        }
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1,5 KiB");
        assert_eq!(human_size(3 * 1024 * 1024), "3,0 MiB");
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

    fn diagnosis() -> beton_voice::Diagnosis {
        beton_voice::Diagnosis {
            backend: beton_voice::Backend::Metal,
            gpu_note: Some("Apple M3 Pro".into()),
            gpu_fallback: false,
            model: Some("large-v3-turbo".into()),
            rtf: Some(0.214),
            error: None,
        }
    }

    /// VOI-003: `doctor` meldet Backend und Real-Time-Factor in `details` des Checks `voice`.
    #[test]
    fn voi_003_doctor_reports_backend_and_rtf() {
        let c = voice_check(&diagnosis());
        assert_eq!(c.id, "voice");
        assert_eq!(c.status, CheckStatus::Ok);
        assert!(c.message.contains("metal (Apple M3 Pro)"), "{}", c.message);
        assert!(c.message.contains("Echtzeitfaktor 0,21"), "{}", c.message);
        let d = c.details.unwrap();
        assert_eq!(d["backend"], "metal");
        assert_eq!(d["rtf"], 0.214);
        assert_eq!(d["model"], "large-v3-turbo");
    }

    /// VOI-003 AC2: Rückfall auf die CPU ist eine Warnung mit Grund.
    #[test]
    fn voi_003_ac2_doctor_warns_about_cpu_fallback() {
        let c = voice_check(&beton_voice::Diagnosis {
            backend: beton_voice::Backend::Cpu,
            gpu_note: Some("metal ließ sich nicht initialisieren".into()),
            gpu_fallback: true,
            rtf: Some(0.9),
            ..diagnosis()
        });
        assert_eq!(c.status, CheckStatus::Warn);
        assert!(c.message.contains("auf der CPU"), "{}", c.message);
        assert!(c.message.contains("ließ sich nicht"), "{}", c.message);
        assert_eq!(c.details.unwrap()["backend"], "cpu");
    }

    #[test]
    fn voi_003_doctor_without_model_reports_backend_only() {
        let c = voice_check(&beton_voice::Diagnosis {
            model: None,
            rtf: None,
            ..diagnosis()
        });
        assert_eq!(c.status, CheckStatus::Ok);
        assert!(
            c.message.contains("kein Modell installiert"),
            "{}",
            c.message
        );
        assert_eq!(c.details.unwrap()["rtf"], serde_json::Value::Null);
    }
}
