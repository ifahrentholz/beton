// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { NotificationChannels } from "./NotificationChannels";

/**
 * Kanäle je Ereignistyp. Defaults wie im Entwurf `settings-notifications`.
 */
export type NotificationMatrix = { approval_requested: NotificationChannels, question: NotificationChannels, turn_completed: NotificationChannels, async_done: NotificationChannels, mention: NotificationChannels, runner_lost: NotificationChannels, budget_threshold: NotificationChannels, rate_limit_high: NotificationChannels, };
