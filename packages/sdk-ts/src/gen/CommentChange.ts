// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { JsonValue } from "./serde_json/JsonValue";

export type CommentChange = { comment_id: `cmt_${string}`, thread_id: string, anchor: JsonValue, body?: string, author: `usr_${string}` | `sa_${string}`, };
