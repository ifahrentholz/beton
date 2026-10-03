// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { SessionTrigger } from "./SessionTrigger";

export type AsyncRunEvent = { run_id: string, trigger?: SessionTrigger, reason?: string, status?: string, cost_micro?: number, duration_s?: number, summary?: string, };
