// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Worktree-Wunsch beim Anlegen einer Session (SES-015).
 */
export type WorktreeRequest = { 
/**
 * Branch-Name; Default `beton/<titel-slug>-<id4>`. Ein vorhandener Branch wird ausgecheckt.
 */
branch?: string, 
/**
 * Base; Default `origin/HEAD`, sonst der aktuelle Branch.
 */
base?: string, 
/**
 * `git fetch <remote> <branch>` vor dem Anlegen bei Remote-Tracking-Bases (Default `true`,
 * Timeout 15 s). Ist das Remote nicht erreichbar, entsteht der Worktree aus dem lokalen
 * Stand und die Session erhält einen Hinweis.
 */
fetch?: boolean, };
