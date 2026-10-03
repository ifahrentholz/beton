// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { SystemComponent } from "./SystemComponent";

/**
 * Wer ein Event ausgelöst hat.
 */
export type Actor = { "kind": "user", id: `usr_${string}` | `sa_${string}`, device_id?: `dev_${string}`, } | { "kind": "agent", id?: `agt_${string}`, 
/**
 * Harness-ID, z. B. `claude`, `codex`, `acp:gemini-cli`.
 */
harness: string, agent_ref?: string, } | { "kind": "system", component: SystemComponent, };
