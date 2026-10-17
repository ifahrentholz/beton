// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Ruhezeiten: täglich von `start` bis `end` (`HH:MM`, Ortszeit des Geräts; über Mitternacht
 * erlaubt). In dieser Zeit bleiben Mitteilung und Ton stumm, die Inbox sammelt weiter.
 */
export type QuietHours = { enabled: boolean, start: string, end: string, 
/**
 * Freigaben (`approval_requested`) trotzdem melden (UX-006 AC2).
 */
allow_approvals: boolean, };
