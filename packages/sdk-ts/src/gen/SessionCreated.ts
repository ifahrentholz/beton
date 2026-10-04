// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { SessionKind } from "./SessionKind";
import type { SessionTrigger } from "./SessionTrigger";
import type { JsonValue } from "./serde_json/JsonValue";

export type SessionCreated = { owner: `usr_${string}` | `sa_${string}`, kind: SessionKind, harness: string, agent_ref?: string, model?: string, cwd: string, project_id?: `prj_${string}`, parent_session_id?: `ses_${string}`, trigger: SessionTrigger, 
/**
 * Harness-spezifische Startoptionen aus `POST /v1/sessions` (z. B. Fake-Szenario).
 */
harness_opts?: JsonValue, };
