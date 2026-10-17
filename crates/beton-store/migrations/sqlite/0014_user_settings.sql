-- 0014_user_settings: Einstellungen je User, die auf allen Geräten gelten (UX-006 AC3), z. B.
-- die Benachrichtigungs-Einstellungen unter dem Schlüssel `notifications`.
-- Folgt auf 0013_policy_snapshots_shadow. Gegenstück: migrations/postgres/0014_user_settings.sql.

-- Ein Dokument (JSON) je User und Schlüssel; der Server prüft es gegen den Rust-Typ, bevor er
-- es schreibt.
CREATE TABLE user_settings (
    org_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (org_id, user_id, key)
);
