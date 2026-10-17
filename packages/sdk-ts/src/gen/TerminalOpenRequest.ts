// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Neues Terminal (`POST /v1/sessions/{id}/terminals`, WS `terminal.open`).
 */
export type TerminalOpenRequest = { 
/**
 * Anzeigename des Tabs, z. B. `Dev-Server`; Default `Shell <n>`.
 */
name?: string, 
/**
 * Programm und Argumente; Default ist die Login-Shell des Benutzers (`SHELL`).
 */
command?: Array<string>, cols: number, rows: number, };
