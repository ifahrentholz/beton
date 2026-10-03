// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { MessageRole } from "./MessageRole";
import type { JsonValue } from "./serde_json/JsonValue";

export type MessageCompleted = { message_id: string, role: MessageRole, content: Array<JsonValue>, author?: `usr_${string}` | `sa_${string}`, };
