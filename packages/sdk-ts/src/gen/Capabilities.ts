// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ApprovalMechanism } from "./ApprovalMechanism";
import type { AuthSource } from "./AuthSource";
import type { CompactionSupport } from "./CompactionSupport";
import type { ForkHistory } from "./ForkHistory";
import type { InstructionsDelivery } from "./InstructionsDelivery";
import type { Mode } from "./Mode";
import type { ResumeSupport } from "./ResumeSupport";
import type { Subagents } from "./Subagents";
import type { SwitchSupport } from "./SwitchSupport";
import type { ToolCallGate } from "./ToolCallGate";
import type { Transport } from "./Transport";
import type { UsageReporting } from "./UsageReporting";

/**
 * Fähigkeiten eines Harness in einem Modus; ggf. nach `probe()` versionsabhängig verfeinert.
 */
export type Capabilities = { mode: Mode, transport: Transport, 
/**
 * Getestete CLI-Versionen als SemVer-Bereich, z. B. `>=2.0.0`.
 */
version_range?: string, auth_sources: Array<AuthSource>, approval: ApprovalMechanism, tool_call_gate: ToolCallGate, model_switch: SwitchSupport, effort_switch: SwitchSupport, resume: ResumeSupport, fork_history: ForkHistory, interrupt: boolean, steering: boolean, subagents: Subagents, usage_reporting: UsageReporting, compaction: CompactionSupport, instructions_delivery: InstructionsDelivery, mcp_injection: boolean, images: boolean, transcript_import: boolean, 
/**
 * Wählbare Modelle (Aliase der CLI), z. B. für den Composer-Picker (WEB-004).
 */
models?: Array<string>, 
/**
 * Wählbare Effort-Stufen; leer, wenn der Harness keine kennt.
 */
efforts?: Array<string>, };
