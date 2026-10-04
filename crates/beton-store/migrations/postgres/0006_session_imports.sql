-- 0006_session_imports: Herkunft importierter Sessions (SES-008). Dedup-Schlüssel ist
-- (Host, Harness, Vendor-Session-ID); ein zweiter Import derselben Vendor-Session ohne `force`
-- wird übersprungen. Beim Löschen der Session wird der Eintrag mitgelöscht.
-- Gegenstück: migrations/sqlite/0006_session_imports.sql.

CREATE TABLE session_imports (
    org_id TEXT NOT NULL,
    host_id TEXT NOT NULL REFERENCES nodes (id),
    harness TEXT NOT NULL,
    vendor_session_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    imported_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (org_id, host_id, harness, vendor_session_id)
);
CREATE INDEX session_imports_session ON session_imports (session_id);
