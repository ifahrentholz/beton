// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { SubagentNode } from "./SubagentNode";

/**
 * Session-Baum unter einer Session (WEB-012).
 */
export type SubagentTree = { 
/**
 * Die angefragte Session.
 */
root: string, 
/**
 * Wurzel zuerst, dann nach Tiefe und Anlage.
 */
nodes: Array<SubagentNode>, };
