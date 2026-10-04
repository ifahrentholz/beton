// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Eigenschaften des Workspace, die Clients vor weiteren Abfragen brauchen.
 */
export type WorkspaceInfo = { 
/**
 * Der Workspace liegt in einem Git-Repository: Die Sichten `uncommitted` und `branch`
 * stehen zur Verfügung (sonst `409 not_a_git_repo`, nur `turn`).
 */
git_repo: boolean, };
