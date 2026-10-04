// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
export type WorkspaceFile = { path: string, size: number, 
/**
 * SHA-256 des Inhalts; zugleich das `ETag` für `If-Match`.
 */
sha256: string, 
/**
 * Inhalt ist kein UTF-8-Text.
 */
binary: boolean, 
/**
 * Größer als 5 MiB: kein Inline-Inhalt, Download über `?download=true`.
 */
too_large: boolean, 
/**
 * Text der Datei (nur UTF-8 und höchstens 5 MiB).
 */
content?: string, };
