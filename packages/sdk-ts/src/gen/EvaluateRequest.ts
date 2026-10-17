// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Anfrage für `POST /v1/policies/evaluate`.
 */
export type EvaluateRequest = { 
/**
 * Phase: `session_start`, `model_request`, `tool_call`, `tool_result`, …
 */
phase: string, 
/**
 * Phasenspezifische Eingabe wie in Policy-Tests, z. B.
 * `{ "tool": { "name": "Bash", "args": { "command": "git push" } } }`.
 */
input: Record<string, unknown>, 
/**
 * Kontext dieser Session (Policies, Kosten, Zustand, Grants).
 */
session_id?: string, 
/**
 * Harness für Normalisierung und Durchsetzung; Default: der der Session bzw. `claude`.
 */
harness?: string, 
/**
 * Weitere Kontextwerte (wie `fixtures` in Policy-Tests); gelten vor denen der Session.
 */
fixtures?: Record<string, unknown>, };
