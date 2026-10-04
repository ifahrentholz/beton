// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Worktree einer Session (aus `git.worktree_created`).
 */
export type SessionWorktree = { 
/**
 * Verzeichnis des Worktrees (Workspace der Session).
 */
path: string, branch: string, 
/**
 * Base, von der der Branch abzweigt, z. B. `origin/main`.
 */
base: string, 
/**
 * Commit der Base beim Anlegen.
 */
base_sha: string, };
