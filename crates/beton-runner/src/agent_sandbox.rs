//! `sandbox:` eines Agents (AGT-014): Die Agent-Ebene verschärft die Sandbox der Session nur
//! (#200). Sie kann ein bestimmtes Backend verlangen (fehlt es, startet der Agent nicht), das
//! Netz der Tools abschalten, den Workspace nur lesbar machen, Masken ergänzen, Limits senken
//! und Stufe 2 erzwingen; Grants (Pfade, Egress-Regeln, Credentials, Env) und `backend: none`
//! sind ein Startfehler.

use beton_agents::spec as agent;
use beton_sandbox::{
    BackendChoice, Preset, ProviderId, SandboxError, SandboxSpec, WorkspaceAccess,
};

/// Die Sandbox der Session mit den Anforderungen des Agents. `user`: `sandbox:` der
/// User-Konfiguration; `auto`: das Backend, das `auto` auf diesem Rechner wählt.
pub fn tighten(
    user: &SandboxSpec,
    a: &agent::Sandbox,
    auto: Option<ProviderId>,
) -> Result<SandboxSpec, SandboxError> {
    let loosen = |field: &str| SandboxError::InvalidSpec {
        path: format!("agent.sandbox.{field}"),
        message: "Die Agent-Ebene kann die Sandbox nur verschärfen, keine Freigaben geben \
                  (AGT-014, #200)"
            .into(),
    };
    let grants = [
        ("read_paths", !a.read_paths.is_empty()),
        ("write_paths", !a.write_paths.is_empty()),
        ("allow_hidden", !a.allow_hidden.is_empty()),
        ("egress_rules", !a.egress_rules.is_empty()),
        ("egress_allow_private", !a.egress_allow_private.is_empty()),
        ("credentials", !a.credentials.is_empty()),
        ("env", a.env.is_some()),
        ("harness", a.harness.is_some()),
        ("windows", a.windows.is_some()),
        ("allow_network", a.allow_network == Some(true)),
        (
            "require_tool_isolation",
            a.require_tool_isolation == Some(false),
        ),
        ("workspace", a.workspace == Some(agent::WorkspaceAccess::Rw)),
    ];
    if let Some((field, _)) = grants.iter().find(|(_, set)| *set) {
        return Err(loosen(field));
    }
    if matches!(
        a.preset,
        Some(agent::SandboxPreset::Default | agent::SandboxPreset::Dev)
    ) {
        return Err(loosen("preset"));
    }
    let user_merged = SandboxSpec::merged(&[user]);
    let mut out = user.clone();
    // Backend: eine Anforderung, nie `none`; ohne Sandbox der Session startet der Agent nicht.
    let wanted = match a.backend {
        None => None,
        Some(agent::SandboxBackend::None) => return Err(loosen("backend")),
        Some(agent::SandboxBackend::Auto) => Some(BackendChoice::Auto),
        Some(b) => Some(BackendChoice::Provider(provider(b))),
    };
    if user_merged.unsandboxed() {
        return Err(SandboxError::Unavailable {
            provider: ProviderId::None,
            missing: beton_sandbox::caps::required_default(),
            hint: "Der Agent verlangt eine Sandbox, die Session läuft ohne \
                   (`sandbox.backend: none`)"
                .into(),
        });
    }
    // Ein Agent wählt kein anderes Backend, als die Session ohnehin nutzt: Andere Backends
    // setzen Masken und Grenzen anders durch (z. B. `bwrap` statt `landlock`). Er kann nur
    // verlangen, dass es dieses ist; sonst startet er nicht (Security-Review #327).
    if let Some(BackendChoice::Provider(p)) = wanted {
        let session = match user_merged.backend.unwrap_or(BackendChoice::Auto) {
            BackendChoice::Provider(u) => Some(u),
            BackendChoice::Auto => auto,
        };
        match session {
            Some(u) if u == p => out.backend = Some(BackendChoice::Provider(p)),
            Some(u) => {
                return Err(SandboxError::Unavailable {
                    provider: p,
                    missing: beton_sandbox::caps::required_default(),
                    hint: format!(
                        "Der Agent verlangt das Backend `{p}`, die Session nutzt `{u}`; ein \
                         Agent wechselt das Backend nicht"
                    ),
                });
            }
            None => {
                return Err(SandboxError::Unavailable {
                    provider: p,
                    missing: beton_sandbox::caps::required_default(),
                    hint: format!(
                        "Der Agent verlangt das Backend `{p}`, auf diesem Rechner gibt es kein \
                         Sandbox-Backend"
                    ),
                });
            }
        }
    }
    if a.preset == Some(agent::SandboxPreset::Readonly) {
        out.preset = Some(Preset::Readonly);
    }
    if a.allow_network == Some(false) {
        out.allow_network = Some(false);
    }
    if a.workspace == Some(agent::WorkspaceAccess::Ro) {
        out.workspace = Some(WorkspaceAccess::Ro);
    }
    if a.require_tool_isolation == Some(true) {
        out.require_tool_isolation = Some(true);
    }
    // Masken werden vereinigt; ohne `masks` der User-Ebene gelten deren Standard-Masken
    // (`.env`, `.env.*`), die zusätzliche Agent-Masken nicht aufheben (Security-Review #327).
    // Die übrigen Listen (Pfade, Egress, Credentials, Env) setzt die Agent-Ebene nie (oben).
    if !a.masks.is_empty() {
        let mut masks = out.masks.clone().unwrap_or_else(|| {
            beton_sandbox::masks::DEFAULT_WORKSPACE_MASKS
                .iter()
                .map(|m| (*m).to_owned())
                .collect()
        });
        masks.extend(a.masks.iter().cloned());
        out.masks = Some(masks);
    }
    if let Some(l) = &a.limits {
        let mut limits = out.limits.unwrap_or_default();
        let lower = |cur: Option<u64>, new: Option<u64>| match (cur, new) {
            (Some(c), Some(n)) => Some(c.min(n)),
            (c, n) => n.or(c),
        };
        limits.memory_mb = lower(limits.memory_mb, l.memory_mb);
        limits.cpu_seconds = lower(limits.cpu_seconds, l.cpu_seconds);
        limits.pids =
            lower(limits.pids.map(u64::from), l.pids).map(|p| u32::try_from(p).unwrap_or(u32::MAX));
        out.limits = Some(limits);
    }
    Ok(out)
}

/// Das Backend, das `auto` auf diesem Rechner wählt; nur abgefragt, wenn der Agent ein
/// bestimmtes Backend verlangt (die Probe startet unter Linux Prozesse).
pub fn auto_choice(a: &agent::Sandbox) -> Option<ProviderId> {
    let wants = a
        .backend
        .is_some_and(|b| !matches!(b, agent::SandboxBackend::Auto | agent::SandboxBackend::None));
    if !wants {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    beton_sandbox::Registry::with_launcher(&exe).auto_choice()
}

fn provider(b: agent::SandboxBackend) -> ProviderId {
    match b {
        agent::SandboxBackend::Seatbelt => ProviderId::Seatbelt,
        agent::SandboxBackend::Landlock => ProviderId::Landlock,
        agent::SandboxBackend::Bwrap => ProviderId::Bwrap,
        agent::SandboxBackend::Windows => ProviderId::Windows,
        agent::SandboxBackend::Docker => ProviderId::Docker,
        agent::SandboxBackend::Podman => ProviderId::Podman,
        agent::SandboxBackend::Auto | agent::SandboxBackend::None => ProviderId::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(yaml: &str) -> agent::Sandbox {
        serde_norway::from_str(yaml).unwrap()
    }

    /// `auto` wählt auf dem Testrechner Seatbelt.
    fn tighten_on(u: &SandboxSpec, a: &agent::Sandbox) -> Result<SandboxSpec, SandboxError> {
        tighten(u, a, Some(ProviderId::Seatbelt))
    }

    fn user(yaml: &str) -> SandboxSpec {
        SandboxSpec::from_yaml(yaml).unwrap()
    }

    #[test]
    fn agt_014_agent_sandbox_only_tightens() {
        let out = tighten_on(
            &user("allow_network: true\nmasks: [.env]\n"),
            &a("allow_network: false\nworkspace: ro\nmasks: [secrets/]\npreset: readonly\nlimits: { pids: 64 }\n"),
        )
        .unwrap();
        assert_eq!(out.allow_network, Some(false));
        assert_eq!(out.workspace, Some(WorkspaceAccess::Ro));
        assert_eq!(out.preset, Some(Preset::Readonly));
        assert_eq!(out.masks, Some(vec![".env".into(), "secrets/".into()]));
        assert_eq!(out.limits.unwrap().pids, Some(64));
        // Höhere Limits der User-Ebene bleiben die niedrigeren.
        let out = tighten_on(
            &user("limits: { pids: 32 }\n"),
            &a("limits: { pids: 64 }\n"),
        )
        .unwrap();
        assert_eq!(out.limits.unwrap().pids, Some(32));
    }

    #[test]
    fn agt_014_agent_sandbox_grants_are_errors() {
        for yaml in [
            "read_paths: [\"~\"]\n",
            "write_paths: [/tmp/x]\n",
            "egress_rules: [\"* evil.example/**\"]\n",
            "allow_network: true\n",
            "workspace: rw\n",
            "preset: dev\n",
            "backend: none\n",
            "require_tool_isolation: false\n",
            "env: { passthrough: [AWS_SECRET_ACCESS_KEY] }\n",
            "credentials: [{ name: x }]\n",
        ] {
            let err = tighten_on(&user("{}\n"), &a(yaml)).unwrap_err();
            assert_eq!(err.code(), "invalid_spec", "{yaml}: {err}");
        }
    }

    #[test]
    fn agt_014_ac3_backend_requirement_is_honoured_or_unavailable() {
        let out = tighten_on(&user("{}\n"), &a("backend: seatbelt\n")).unwrap();
        assert_eq!(
            out.backend,
            Some(BackendChoice::Provider(ProviderId::Seatbelt))
        );
        let err = tighten_on(&user("backend: landlock\n"), &a("backend: seatbelt\n")).unwrap_err();
        assert_eq!(err.code(), "sandbox_unavailable", "{err}");
        // Ohne Sandbox der Session startet ein Agent mit Sandbox-Anforderung nicht.
        let err = tighten_on(&user("backend: none\n"), &a("allow_network: false\n")).unwrap_err();
        assert_eq!(err.code(), "sandbox_unavailable", "{err}");
    }

    /// Security-Review #327 B1: Ohne `masks` der User-Ebene gelten die Standard-Masken
    /// (`.env`, `.env.*`); zusätzliche Masken des Agents heben sie nicht auf.
    #[test]
    fn agt_014_agent_masks_without_user_masks_keep_the_default_masks() {
        let out = tighten_on(&user("{}\n"), &a("masks: [node_modules]\n")).unwrap();
        let masks = out.masks.unwrap();
        for m in [".env", ".env.*", "node_modules"] {
            assert!(masks.iter().any(|x| x == m), "{m} fehlt: {masks:?}");
        }
        // Eigene Masken der User-Ebene bleiben ebenso.
        let out = tighten_on(&user("masks: [secrets/]\n"), &a("masks: [node_modules]\n")).unwrap();
        assert_eq!(
            out.masks,
            Some(vec!["secrets/".into(), "node_modules".into()])
        );
        // Ohne Agent-Masken bleibt `None` (= Standard-Masken).
        let out = tighten_on(&user("{}\n"), &a("allow_network: false\n")).unwrap();
        assert_eq!(out.masks, None);
    }

    /// Security-Review #327: Ein Agent kann kein anderes Backend erzwingen, als die Session
    /// ohnehin nutzt (z. B. `bwrap` statt des von `auto` gewählten `landlock`); seine
    /// Garantien wären andere. Das ist ein Startfehler, keine Lockerung.
    #[test]
    fn agt_014_agent_cannot_switch_the_backend_auto_selects() {
        let err = tighten(
            &user("{}\n"),
            &a("backend: bwrap\n"),
            Some(ProviderId::Landlock),
        )
        .unwrap_err();
        assert_eq!(err.code(), "sandbox_unavailable", "{err}");
        let err = tighten(&user("{}\n"), &a("backend: seatbelt\n"), None).unwrap_err();
        assert_eq!(err.code(), "sandbox_unavailable", "{err}");
        // Dasselbe Backend, das `auto` wählt: erfüllt.
        let out = tighten(
            &user("{}\n"),
            &a("backend: landlock\n"),
            Some(ProviderId::Landlock),
        )
        .unwrap();
        assert_eq!(
            out.backend,
            Some(BackendChoice::Provider(ProviderId::Landlock))
        );
        // `backend: auto` des Agents verlangt nur irgendeine Sandbox.
        let out = tighten(
            &user("{}\n"),
            &a("backend: auto\n"),
            Some(ProviderId::Landlock),
        )
        .unwrap();
        assert_eq!(out.backend, None);
    }
}
