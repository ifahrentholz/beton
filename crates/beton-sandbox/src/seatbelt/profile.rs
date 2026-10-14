//! Generator des Seatbelt-Profils (SBX-007): festes, versioniertes Template plus Parameter.
//!
//! Kein Pfad und kein Muster gelangt als Text ins SBPL. Jede generierte Zeile besteht aus
//! festen Bausteinen und einem generierten Schlüssel (`[A-Z0-9_]+`); die Werte gehen als
//! `-D KEY=VALUE` an `sandbox-exec` und werden im Profil nur über `(param "KEY")` gelesen.
//! Damit kann ein Workspace-Name mit `"`, `)` oder Zeilenumbruch das Profil nicht verändern
//! (AC3). Reguläre Ausdrücke für Muster-Masken entstehen aus maskierten Pfaden
//! ([`regex_escape`]); sie sind ebenfalls nur Parameterwerte.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::SandboxError;
use crate::resolve::{Mask, NetMode, ResolvedPolicy, Stage};

/// Version des Templates (erscheint in Probe und `explain`).
pub const PROFILE_VERSION: u32 = 1;

/// Das Template.
pub const TEMPLATE: &str = include_str!("profile.v1.sbpl");

/// Höchstgröße von Profil plus Parametern (SBX-007).
pub const MAX_PROFILE_BYTES: usize = 256 * 1024;

/// Mach-Services des Keychains (nur Stufe 1 mit `keychain: true`).
pub const KEYCHAIN_SERVICES: [&str; 2] = ["com.apple.SecurityServer", "com.apple.securityd"];

/// Profiltext und Parameter für einen Start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub text: String,
    pub params: Vec<(String, String)>,
}

impl Profile {
    /// Gesamtgröße (Text, Schlüssel, Werte).
    pub fn size(&self) -> usize {
        self.text.len()
            + self
                .params
                .iter()
                .map(|(k, v)| k.len() + v.len() + 1)
                .sum::<usize>()
    }
}

/// Maskiert alle Sonderzeichen regulärer Ausdrücke (POSIX ERE) mit `\`.
pub fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Ein Pfadsegment-Muster (`*`, `?`) als regulärer Ausdruck ohne `/`.
pub fn glob_regex(glob: &str) -> String {
    let mut out = String::new();
    for c in glob.chars() {
        match c {
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            c => out.push_str(&regex_escape(&c.to_string())),
        }
    }
    out
}

fn utf8(p: &Path) -> Result<String, SandboxError> {
    p.to_str().map(str::to_owned).ok_or_else(|| {
        SandboxError::invalid(
            "paths",
            format!("{} ist kein gültiges UTF-8", p.display()),
        )
    })
}

/// Regex für einen Pfad samt Inhalt unter `root`.
fn root_regex(root: &Path) -> Result<String, SandboxError> {
    Ok(format!("^{}", regex_escape(utf8(root)?.trim_end_matches('/'))))
}

struct Builder {
    params: Vec<(String, String)>,
}

impl Builder {
    fn param(&mut self, prefix: &str, value: String) -> String {
        let key = format!("{prefix}_{}", self.params.len());
        self.params.push((key.clone(), value));
        key
    }

    fn path(&mut self, prefix: &str, p: &Path) -> Result<String, SandboxError> {
        let v = utf8(p)?;
        Ok(self.param(prefix, v))
    }
}

fn within_any(p: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|r| p.starts_with(r))
}

/// Erzeugt Profil und Parameter für `policy`.
pub fn render(policy: &ResolvedPolicy) -> Result<Profile, SandboxError> {
        todo!("SBX-007")
    }
