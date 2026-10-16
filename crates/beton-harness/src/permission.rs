//! Permission-Modes (HAR-027): welche Modi ein Harness abbilden kann und wann `yolo` überhaupt
//! startbar ist.
//!
//! Fail closed: Ein Modus, den der Harness nicht deklariert, wird mit `capability_unsupported`
//! abgelehnt statt still ignoriert; `yolo` (keine Vendor-Rückfragen) braucht Tool-Sandbox
//! Stufe 2 und Egress-Proxy, sonst `sandbox_required` (SBX-006, PRX-008). Den Status liefert
//! allein das tatsächlich aufgebaute Stufenmodell der Session (Runner) bzw. die Sandbox der
//! User-Konfiguration samt Probe (Server); Projekt- und Agent-Konfiguration können ihn nie
//! setzen.

use crate::adapter::{HarnessError, PermissionMode};
use crate::capabilities::{Action, Capabilities, CapabilityUnsupported};

/// Schutz, der für die Tool-Aufrufe einer Session aktiv ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct SandboxStatus {
    /// Tool-Sandbox Stufe 2 (SBX-002, SBX-006): Tools laufen getrennt vom Harness.
    pub tool_sandbox: bool,
    /// Netz der Tools nur über den Egress-Proxy oder gar keins (PRX-008).
    pub egress_proxy: bool,
}

impl SandboxStatus {
    /// Ohne Sandbox (`backend: none` bzw. kein Stufenmodell).
    pub const NONE: Self = Self {
        tool_sandbox: false,
        egress_proxy: false,
    };

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
        let err = check_permission_mode(PermissionMode::Yolo, &caps(&ALL), SandboxStatus::NONE)
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
    fn har_027_without_sandbox_never_allows_yolo() {
        assert!(!SandboxStatus::NONE.allows_yolo());
        assert!(!SandboxStatus::default().allows_yolo());
    }

    #[test]
    fn har_027_undeclared_mode_is_capability_unsupported() {
        let c = caps(&[PermissionMode::Default]);
        let err = check_permission_mode(PermissionMode::Plan, &c, SandboxStatus::NONE).unwrap_err();
        assert_eq!(err.code(), "capability_unsupported");
        assert!(check_permission_mode(PermissionMode::Default, &c, SandboxStatus::NONE).is_ok());
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
