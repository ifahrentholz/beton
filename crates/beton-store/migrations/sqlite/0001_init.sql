-- 0001_init: lokale Kernentitäten (DATA-001), Event-Log (DATA-002), Projektionen (DATA-005),
-- Blob-Referenzen (DATA-006), Tombstones und Audit (DATA-008).
-- Zeitstempel sind RFC-3339-Strings in UTC mit Millisekunden (lexikografisch sortierbar).
-- Gegenstück: migrations/postgres/0001_init.sql (Paritätstest DATA-003 AC1).

CREATE TABLE orgs (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE users (
    id TEXT NOT NULL PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES orgs (id),
    display_name TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE nodes (
    id TEXT NOT NULL PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES orgs (id),
    name TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE sessions (
    id TEXT NOT NULL PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES orgs (id),
    owner_id TEXT NOT NULL REFERENCES users (id),
    project_id TEXT,
    parent_id TEXT REFERENCES sessions (id),
    kind TEXT NOT NULL,
    harness TEXT NOT NULL,
    home_node_id TEXT NOT NULL REFERENCES nodes (id),
    epoch INTEGER NOT NULL,
    head_seq INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    -- Projektion (DATA-005): aus Events abgeleitet, per Rebuild neu aufbaubar.
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    archived INTEGER NOT NULL,
    cost_micro INTEGER NOT NULL,
    last_activity_at TEXT NOT NULL
);
CREATE INDEX sessions_org_activity ON sessions (org_id, last_activity_at);
CREATE INDEX sessions_parent ON sessions (parent_id);

CREATE TABLE events (
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    seq INTEGER NOT NULL,
    id TEXT NOT NULL,
    ts TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    actor_id TEXT,
    actor TEXT NOT NULL,
    type TEXT NOT NULL,
    payload TEXT,
    payload_ref TEXT,
    turn_id TEXT,
    causation_id TEXT,
    epoch INTEGER NOT NULL,
    redacted INTEGER NOT NULL,
    PRIMARY KEY (session_id, seq),
    CHECK ((payload IS NULL) <> (payload_ref IS NULL))
);
CREATE UNIQUE INDEX events_id ON events (id);

CREATE TABLE event_raw (
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    raw TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (session_id, seq),
    FOREIGN KEY (session_id, seq) REFERENCES events (session_id, seq)
);

CREATE TABLE blobs (
    org_id TEXT NOT NULL REFERENCES orgs (id),
    sha256 TEXT NOT NULL,
    size INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (org_id, sha256)
);

CREATE TABLE blob_refs (
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    sha256 TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (session_id, sha256),
    FOREIGN KEY (org_id, sha256) REFERENCES blobs (org_id, sha256)
);
CREATE INDEX blob_refs_blob ON blob_refs (org_id, sha256);

CREATE TABLE approvals (
    id TEXT NOT NULL PRIMARY KEY,
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    requested_seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    subject TEXT NOT NULL,
    options TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    on_timeout TEXT NOT NULL,
    status TEXT NOT NULL,
    decision TEXT,
    resolved_seq INTEGER
);
CREATE INDEX approvals_org_status ON approvals (org_id, status);
CREATE INDEX approvals_session ON approvals (session_id);

CREATE TABLE usage_daily (
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    day TEXT NOT NULL,
    harness TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    cost_micro INTEGER NOT NULL,
    events INTEGER NOT NULL,
    PRIMARY KEY (session_id, day, harness, model)
);
CREATE INDEX usage_daily_org_day ON usage_daily (org_id, day);

CREATE TABLE tombstones (
    org_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    deleted_at TEXT NOT NULL,
    deleted_by TEXT NOT NULL,
    PRIMARY KEY (kind, id)
);
CREATE INDEX tombstones_org_deleted ON tombstones (org_id, deleted_at);

CREATE TABLE audit_entries (
    id TEXT NOT NULL PRIMARY KEY,
    org_id TEXT NOT NULL,
    at TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    details TEXT NOT NULL
);
CREATE INDEX audit_entries_org_at ON audit_entries (org_id, at);
