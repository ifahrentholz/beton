// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { Capabilities } from "./Capabilities";
import type { Mode } from "./Mode";
import type { ProbeReport } from "./ProbeReport";

/**
 * Ein Katalogeintrag für `GET /v1/harnesses?host=<id>` (HAR-002).
 */
export type HarnessInfo = { id: string, modes: Array<Mode>, 
/**
 * Je Modus, nach dem Probe verfeinert.
 */
capabilities: Array<Capabilities>, probe: ProbeReport, incompatible: boolean, incompatible_reason?: string, };
