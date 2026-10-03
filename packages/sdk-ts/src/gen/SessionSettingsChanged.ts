// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { SettingsMechanism } from "./SettingsMechanism";

export type SessionSettingsChanged = { model?: string, effort?: string, requested_effort?: string, permission_mode?: string, mechanism?: SettingsMechanism, effective_from_turn?: `trn_${string}`, };
