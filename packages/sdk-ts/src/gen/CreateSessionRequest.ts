// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
export type CreateSessionRequest = { 
/**
 * Harness-ID, z. B. `claude` oder (nur mit `--dev`) `fake`.
 */
target: string, 
/**
 * Arbeitsverzeichnis (Projekt oder Worktree).
 */
cwd: string, title?: string, model?: string, 
/**
 * Harness-spezifische Optionen, z. B. `{"scenario": "…"}` beim Fake-Harness.
 */
harness_opts?: Record<string, unknown>, };
