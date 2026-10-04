// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ForkWorkspace } from "./ForkWorkspace";

/**
 * Fork-Anfrage (SES-006, SES-007).
 */
export type ForkRequest = { 
/**
 * Fork-Punkt; ohne Angabe das Ende der Session. Liegt er mitten in einem Turn, beginnt der
 * Fork am letzten vollständigen Turn-Ende davor (`effective_seq`).
 */
at_seq?: number, 
/**
 * Ziel-Harness, z. B. `codex`; ohne Angabe der Harness der Quelle. Ein anderer Harness
 * bekommt den Verlauf als Übergabe-Präambel (HAR-018).
 */
harness?: string, model?: string, 
/**
 * Reasoning-Effort des Forks (HAR-017), gegen den Ziel-Harness geprüft.
 */
effort?: string, 
/**
 * Permission-Mode des Forks (HAR-027); `yolo` ohne Sandbox: `409 sandbox_required`.
 */
permission_mode?: string, 
/**
 * `new_worktree` (Default, falls die Quelle in einem Git-Repository arbeitet), `shared`
 * oder `fresh`. Dateien werden nie auf den Stand von `at_seq` zurückgesetzt.
 */
workspace?: ForkWorkspace, title?: string, 
/**
 * Harness-spezifische Startoptionen (wie bei `POST /v1/sessions`); ohne Angabe die der
 * Quelle, falls der Harness gleich bleibt.
 */
harness_opts?: Record<string, unknown>, };
