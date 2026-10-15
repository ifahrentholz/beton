// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Wie Tool-Ausführungen in Stufe 2 landen (SBX-002): `inherited` – gar nicht, Tools laufen
 * im Prozess des Harness (Stufe 1, *degraded*, nur mit `require_tool_isolation: false`);
 * `shell_override` – über `bt-exec` als Shell des Harness; `beton` – beton führt die Tools
 * selbst aus (Direkt-API, ACP mit `terminal/*`).
 */
export type ToolIsolation = "inherited" | "shell_override" | "beton";
