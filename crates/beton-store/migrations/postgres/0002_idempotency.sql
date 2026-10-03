-- 0002_idempotency: gespeicherte Antworten für Idempotency-Key (PROTO-010 AC2), 24 h gültig.
-- Gegenstück: migrations/sqlite/0002_idempotency.sql.

CREATE TABLE idempotency_keys (
    org_id TEXT NOT NULL REFERENCES orgs (id),
    key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    status BIGINT NOT NULL,
    content_type TEXT NOT NULL,
    body BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (org_id, key)
);
CREATE INDEX idempotency_keys_created ON idempotency_keys (created_at);
