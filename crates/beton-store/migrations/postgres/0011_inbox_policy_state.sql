-- 0011_inbox_policy_state: Inbox-Projektion (UX-001) und Policy-Zustand des Users (POL-009).
-- Folgt auf 0010_secrets_audit (WP-33).
-- Gegenstück: migrations/sqlite/0011_inbox_policy_state.sql.

-- Inbox: offene Freigaben und Fragen (`approval.requested`), Hinweise aus `notify`
-- (`policy.notified`). Projektion aus dem Event-Log, je Session neu aufbaubar; ältere Sessions
-- füllt `beton admin projections rebuild`.
CREATE TABLE inbox_items (
    id TEXT NOT NULL,
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    approval_id TEXT,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    detail JSONB NOT NULL,
    created_seq BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    resolved_seq BIGINT,
    resolved_at TIMESTAMPTZ,
    resolution JSONB,
    PRIMARY KEY (session_id, id)
);
CREATE UNIQUE INDEX inbox_items_id ON inbox_items (org_id, id);
CREATE INDEX inbox_items_status ON inbox_items (org_id, status, created_at);

-- Zustand der Scopes `user` (day = '') und `user_day` (day = JJJJ-MM-TT in der Zeitzone des
-- Users); ein neuer Tag beginnt leer. Werte als JSON, höchstens 4 KiB je Key.
CREATE TABLE policy_user_state (
    org_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    day TEXT NOT NULL,
    key TEXT NOT NULL,
    value JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (org_id, user_id, scope, day, key)
);
