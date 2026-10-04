// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { SessionSkill } from "./SessionSkill";

/**
 * Skills einer Session.
 */
export type SessionSkills = { 
/**
 * Name des aktiven Agents, falls die Session einen hat.
 */
agent?: string, items: Array<SessionSkill>, 
/**
 * Immer leer: die Liste ist vollständig.
 */
next_cursor: string | null, };
