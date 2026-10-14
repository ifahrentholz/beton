// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { UsageRecordView } from "./UsageRecordView";
import type { UsageTotals } from "./UsageTotals";

/**
 * Antwort von `GET /v1/sessions/{id}/usage` und `beton usage --session <id> --json`.
 */
export type SessionUsageView = { session_id: string, records: Array<UsageRecordView>, totals: UsageTotals, };
