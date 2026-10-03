# @ifahrentholz/beton-sdk

TypeScript-SDK für die beton-API. Das Paket selbst (Client, Transport) entsteht mit API-004 (WP-13).

`src/gen/` enthält die aus den Rust-Typen generierten Typen (PROTO-013) – derzeit das Event-Modell (`Event`, `EventPayload`, alle Payloads). Diese Dateien werden ausschließlich mit `cargo xtask codegen` erzeugt und nie von Hand geändert; die CI prüft das mit `cargo xtask codegen --check`.
