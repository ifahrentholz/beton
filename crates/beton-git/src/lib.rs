//! Git-Anbindung von beton (M1-Umfang): Worktree pro Session (SES-015, SES-016), Änderungen
//! und Diffs für die Workspace-API (SES-018) und Turn-Snapshots in einem Schatten-Repository.
//!
//! Git-Provider (GitHub, GitLab) folgen mit M4. Alle Funktionen sind synchron und rufen die
//! `git`-CLI auf; aus async-Code über `spawn_blocking` verwenden.

pub mod changes;
pub mod cmd;
pub mod snapshot;
pub mod worktree;

pub use crate::changes::{ChangeStatus, ChangedFile, DiffLine, FileDiff, Hunk, LineKind};
pub use crate::cmd::{Git, GitError};
pub use crate::snapshot::ShadowRepo;
