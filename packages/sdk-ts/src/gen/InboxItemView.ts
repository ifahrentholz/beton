// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Ein Eintrag der Inbox.
 */
export type InboxItemView = { id: string, session_id: string, session_title: string, 
/**
 * `approval`, `question` oder `notice`.
 */
kind: string, 
/**
 * `open` (wartet auf dich), `done` (entschieden) oder `info` (Hinweis).
 */
status: string, approval_id?: string, title: string, summary: string, 
/**
 * Freigaben und Fragen: `subject`, `options`, `expires_at`, `on_timeout`,
 * `approval_kind`; Hinweise: `rule_id`, `scope`, `set`, `channels`.
 */
detail: unknown, created_seq: number, 
/**
 * RFC 3339.
 */
created_at: string, resolved_at?: string, 
/**
 * `decision`, `via`, `actor`, `comment?`, `answer?`, `remember?`, `on_timeout_applied?`.
 */
resolution?: unknown, };
