-- 0003_approvals_per_session: Approval-IDs sind nur pro Session eindeutig. Der Fake-Harness
-- vergibt sie deterministisch aus dem Szenario (HAR-026 AC1), zwei Sessions mit demselben
-- Szenario teilen also IDs. Schlüssel wird (session_id, id).
-- Gegenstück: migrations/sqlite/0003_approvals_per_session.sql.

CREATE TABLE approvals_v3 (
    id TEXT NOT NULL,
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    requested_seq BIGINT NOT NULL,
    kind TEXT NOT NULL,
    subject JSONB NOT NULL,
    options JSONB NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    on_timeout TEXT NOT NULL,
    status TEXT NOT NULL,
    decision TEXT,
    resolved_seq BIGINT,
    PRIMARY KEY (session_id, id)
);
INSERT INTO approvals_v3 (id, org_id, session_id, requested_seq, kind, subject, options,
    expires_at, on_timeout, status, decision, resolved_seq)
    SELECT id, org_id, session_id, requested_seq, kind, subject, options, expires_at,
    on_timeout, status, decision, resolved_seq FROM approvals;
DROP TABLE approvals;
ALTER TABLE approvals_v3 RENAME TO approvals;
CREATE INDEX approvals_org_status ON approvals (org_id, status);
CREATE INDEX approvals_session ON approvals (session_id);
