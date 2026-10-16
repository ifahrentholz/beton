-- 0015_user_settings: Einstellungen je User, die auf allen Geräten gelten (UX-006 AC3), z. B.
-- die Benachrichtigungs-Einstellungen unter dem Schlüssel `notifications`.
-- Folgt auf 0014 (WP-51). Gegenstück: migrations/sqlite/0015_user_settings.sql.

-- Ein Dokument (JSON) je User und Schlüssel; der Server prüft es gegen den Rust-Typ, bevor er
-- es schreibt.
CREATE TABLE user_settings (
    org_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (org_id, user_id, key)
);
