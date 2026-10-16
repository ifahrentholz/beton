//! `git`-Shim der Stufe 1 (SBX-003, #311).

/// Absoluter Pfad des echten `git` im Environment der Stufe 1 (von Stufe 0 beim Start
/// ermittelt).
pub const ENV_GIT: &str = "BETON_GIT";
/// Aufrufname des Shims.
pub const NAME: &str = "git";
