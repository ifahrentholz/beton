// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { Attachment } from "./Attachment";

export type QueueItem = { id: string, author: `usr_${string}` | `sa_${string}`, text: string, attachments: Array<Attachment>, created_at: string, };
