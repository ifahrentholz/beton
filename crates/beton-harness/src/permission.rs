//! Permission-Modes (HAR-027): welche Modi ein Harness abbilden kann und wann `yolo` überhaupt
//! startbar ist.
//!
//! Fail closed: Ein Modus, den der Harness nicht deklariert, wird mit `capability_unsupported`
//! abgelehnt statt still ignoriert; `yolo` (keine Vendor-Rückfragen) braucht Tool-Sandbox
//! Stufe 2 und Egress-Proxy, sonst `sandbox_required`. Beides kommt erst mit M2 (SBX-006,
//! PRX-008), bis dahin ist `yolo` nie startbar.

use crate::adapter::{HarnessError, PermissionMode};
use crate::capabilities::{Action, Capabilities, CapabilityUnsupported};

/// Schutz, der auf diesem Host für Tool-Aufrufe aktiv ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SandboxStatus {
    /// Tool-Sandbox Stufe 2 (SBX-006).
    pub tool_sandbox: bool,
    /// Egress-Proxy (PRX-008).
    pub egress_proxy: bool,
}

impl SandboxStatus {
    /// Stand dieses Builds: Weder Tool-Sandbox Stufe 2 noch Egress-Proxy gibt es vor M2. Es gibt
    /// bewusst keinen Schalter (Konfiguration, Umgebung, Projekt-Datei), der das ändert.
    pub const fn current() -> Self {
        Self {
            tool_sandbox: false,
            egress_proxy: false,
        }
    }

    /// `yolo` braucht beides (HAR-027).
    pub fn allows_yolo(&self) -> bool {
        self.tool_sandbox && self.egress_proxy
    }
}

/// Darf `mode` für einen Harness mit `caps` gesetzt werden (Start oder Wechsel)?
pub fn check_permission_mode(
    mode: PermissionMode,
    caps: &Capabilities,
    sandbox: SandboxStatus,
) -> Result<(), HarnessError> {
    if mode == PermissionMode::Yolo && !sandbox.allows_yolo() {
        return Err(HarnessError::SandboxRequired(
            "YOLO startet nur mit Tool-Sandbox (Stufe 2) und Egress-Proxy; beides ist auf diesem \
             Rechner nicht aktiv"
                .into(),
        ));
    }
    if !caps.permission_modes.contains(&mode) {
        return Err(CapabilityUnsupported(Action::PermissionMode).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{Mode, Transport};

    fn caps(modes: &[PermissionMode]) -> Capabilities {
        let mut c = Capabilities::minimal(Mode::Native, Transport::Native);
        c.permission_modes = modes.to_vec();
        c
    }

    const ALL: [PermissionMode; 4] = [
        PermissionMode::Plan,
        PermissionMode::Default,
        PermissionMode::AcceptEdits,
        PermissionMode::Yolo,
    ];

    #[test]
    fn har_027_ac1_yolo_without_sandbox_is_sandbox_required() {
        let err =
            check_permission_mode(PermissionMode::Yolo, &caps(&ALL), SandboxStatus::current())
                .unwrap_err();
        assert_eq!(err.code(), "sandbox_required");
    }

    #[test]
    fn har_027_yolo_needs_sandbox_and_proxy() {
        for partial in [
            SandboxStatus {
                tool_sandbox: true,
                egress_proxy: false,
            },
            SandboxStatus {
                tool_sandbox: false,
                egress_proxy: true,
            },
        ] {
            let err =
                check_permission_mode(PermissionMode::Yolo, &caps(&ALL), partial).unwrap_err();
            assert_eq!(err.code(), "sandbox_required", "{partial:?}");
        }
        let full = SandboxStatus {
            tool_sandbox: true,
            egress_proxy: true,
        };
        assert!(check_permission_mode(PermissionMode::Yolo, &caps(&ALL), full).is_ok());
    }

    #[test]
    fn har_027_this_build_never_allows_yolo() {
        assert!(!SandboxStatus::current().allows_yolo());
        assert!(!SandboxStatus::default().allows_yolo());
    }

    #[test]
    fn har_027_undeclared_mode_is_capability_unsupported() {
        let c = caps(&[PermissionMode::Default]);
        let err =
            check_permission_mode(PermissionMode::Plan, &c, SandboxStatus::current()).unwrap_err();
        assert_eq!(err.code(), "capability_unsupported");
        assert!(
            check_permission_mode(PermissionMode::Default, &c, SandboxStatus::current()).is_ok()
        );
    }

    #[test]
    fn har_027_modes_parse_only_beton_names() {
        for m in ALL {
            assert_eq!(m.as_str().parse::<PermissionMode>().unwrap(), m);
        }
        for bad in ["bypassPermissions", "acceptEdits", "YOLO", "", "auto"] {
            assert!(bad.parse::<PermissionMode>().is_err(), "{bad}");
        }
    }
}
