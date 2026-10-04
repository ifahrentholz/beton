// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Ein Skill im Slash-Menü.
 */
export type SessionSkill = { 
/**
 * Aufruf als `/<name>`.
 */
name: string, description: string, 
/**
 * Herkunft: `agent`, `project`, `user` oder `builtin`.
 */
origin: string, };
