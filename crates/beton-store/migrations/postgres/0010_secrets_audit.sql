-- 0010_secrets_audit: Secret-Metadaten (SEC-001) und hash-verkettetes Audit-Log (SEC-012).
-- Folgt auf 0009_usage_records (WP-34).
-- Gegenstück: migrations/sqlite/0010_secrets_audit.sql.

-- Hash-Kette je Org: hash = SHA-256(prev_hash ‖ Eintrag), seq lückenlos ab 1. Bestehende
-- Einträge (DATA-008) verkettet der Store beim Öffnen nach.
ALTER TABLE audit_entries ADD COLUMN seq BIGINT;
ALTER TABLE audit_entries ADD COLUMN session_id TEXT;
ALTER TABLE audit_entries ADD COLUMN host_id TEXT;
ALTER TABLE audit_entries ADD COLUMN count BIGINT NOT NULL DEFAULT 1;
ALTER TABLE audit_entries ADD COLUMN prev_hash TEXT;
ALTER TABLE audit_entries ADD COLUMN hash TEXT;
CREATE UNIQUE INDEX audit_entries_org_seq ON audit_entries (org_id, seq);

-- Kopf der Kette: erkennt auch das Löschen des letzten Eintrags.
CREATE TABLE audit_heads (
    org_id TEXT NOT NULL PRIMARY KEY,
    seq BIGINT NOT NULL,
    hash TEXT NOT NULL
);

-- Offene 5-min-Fenster hochfrequenter Secret-Nutzung (Proxy) je (secret, session, host);
-- ein abgelaufenes Fenster wird als ein Audit-Eintrag mit Zähler versiegelt.
CREATE TABLE audit_usage_windows (
    org_id TEXT NOT NULL,
    secret_ref TEXT NOT NULL,
    session_id TEXT NOT NULL,
    host_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    purpose TEXT NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    last_at TIMESTAMPTZ NOT NULL,
    count BIGINT NOT NULL,
    PRIMARY KEY (org_id, secret_ref, session_id, host_id)
);

-- Metadaten der Secrets; Werte liegen nur im Backend (Keychain oder verschlüsselte Datei).
CREATE TABLE secrets (
    org_id TEXT NOT NULL,
    secret_ref TEXT NOT NULL,
    scope TEXT NOT NULL,
    provider TEXT NOT NULL,
    account TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    backend TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    last_used_at TIMESTAMPTZ,
    PRIMARY KEY (org_id, secret_ref)
);
