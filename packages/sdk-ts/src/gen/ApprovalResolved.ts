// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { Actor } from "./Actor";
import type { ApprovalDecision } from "./ApprovalDecision";
import type { RememberScope } from "./RememberScope";
import type { ResolvedVia } from "./ResolvedVia";
import type { TimeoutAction } from "./TimeoutAction";

export type ApprovalResolved = { approval_id: `apr_${string}`, decision: ApprovalDecision, answer?: string, actor: Actor, via: ResolvedVia, remember?: RememberScope, comment?: string, on_timeout_applied?: TimeoutAction, };
