-- 0009_usage_records: Usage-Projektion (USE-001, DATA-005). Je `cost.delta` ein Record mit
-- Session, Projekt, User, Harness, Modell, `auth_source`, Zeitpunkt und Kalendertag (UTC).
-- Der Tag in der Zeitzone des Users entsteht bei der Abfrage aus `ts`, damit ein Wechsel der
-- Zeitzone keine Records ändert. Team kommt mit M4.
-- `cost_micro` ist leer bei `cost_source` `none` (ohne Preis) und `subscription`
-- (`vendor_cli`, zählt nie zu Geld-Summen); `equivalent_micro` ist nur informativ.
-- Bestehende `cost.delta` werden übernommen wie beim Rebuild (ohne neue Bewertung):
-- `vendor_cli` → `subscription`, gemeldete Kosten → `harness`, sonst `none`. Aus Dateien
-- importierte Sessions haben keine Usage (DATA-010).
-- Gegenstück: migrations/sqlite/0009_usage_records.sql.

CREATE TABLE usage_records (
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    seq BIGINT NOT NULL,
    ts TIMESTAMPTZ NOT NULL,
    day TEXT NOT NULL,
    project_id TEXT,
    user_id TEXT NOT NULL,
    harness TEXT NOT NULL,
    model TEXT NOT NULL,
    auth_source TEXT NOT NULL,
    purpose TEXT,
    input_tokens BIGINT NOT NULL,
    output_tokens BIGINT NOT NULL,
    cache_read_tokens BIGINT NOT NULL,
    cache_write_tokens BIGINT NOT NULL,
    cost_micro BIGINT,
    cost_source TEXT NOT NULL,
    pricing_version TEXT,
    equivalent_micro BIGINT,
    PRIMARY KEY (session_id, seq)
);
CREATE INDEX usage_records_org_ts ON usage_records (org_id, ts);
CREATE INDEX usage_records_org_user_ts ON usage_records (org_id, user_id, ts);

INSERT INTO usage_records (org_id, session_id, seq, ts, day, project_id, user_id, harness,
    model, auth_source, purpose, input_tokens, output_tokens, cache_read_tokens,
    cache_write_tokens, cost_micro, cost_source, pricing_version, equivalent_micro)
SELECT e.org_id, e.session_id, e.seq, e.ts, to_char(e.ts AT TIME ZONE 'UTC', 'YYYY-MM-DD'),
    s.project_id, s.owner_id,
    e.payload ->> 'harness',
    e.payload ->> 'model',
    e.payload ->> 'auth_source',
    e.payload ->> 'purpose',
    COALESCE((e.payload ->> 'input_tokens')::bigint, 0),
    COALESCE((e.payload ->> 'output_tokens')::bigint, 0),
    COALESCE((e.payload ->> 'cache_read_tokens')::bigint, 0),
    COALESCE((e.payload ->> 'cache_write_tokens')::bigint, 0),
    CASE WHEN e.payload ->> 'auth_source' = 'vendor_cli' THEN NULL
         ELSE (e.payload ->> 'cost_micro')::bigint END,
    CASE WHEN e.payload ->> 'auth_source' = 'vendor_cli' THEN 'subscription'
         WHEN e.payload ->> 'cost_micro' IS NOT NULL THEN 'harness'
         ELSE 'none' END,
    NULL,
    NULL
FROM events e
JOIN sessions s ON s.id = e.session_id AND s.org_id = e.org_id
WHERE e.type = 'cost.delta' AND e.payload IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM session_file_imports f WHERE f.session_id = e.session_id);
