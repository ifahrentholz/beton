// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ApprovalKind } from "./ApprovalKind";
import type { TimeoutAction } from "./TimeoutAction";
import type { JsonValue } from "./serde_json/JsonValue";

export type ApprovalRequested = { approval_id: `apr_${string}`, kind: ApprovalKind, subject: JsonValue, options: Array<string>, expires_at: string, on_timeout: TimeoutAction, };
