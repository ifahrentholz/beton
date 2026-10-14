-- 0009_usage_records: Usage-Projektion (USE-001, DATA-005). Je `cost.delta` ein Record mit
-- Session, Projekt, User, Harness, Modell, `auth_source`, Zeitpunkt und Kalendertag (UTC).
-- Der Tag in der Zeitzone des Users entsteht bei der Abfrage aus `ts`, damit ein Wechsel der
-- Zeitzone keine Records ändert. Team kommt mit M4.
-- `cost_micro` ist leer bei `cost_source` `none` (ohne Preis) und `subscription`
-- (`vendor_cli`, zählt nie zu Geld-Summen); `equivalent_micro` ist nur informativ.
-- Bestehende `cost.delta` werden übernommen wie beim Rebuild (ohne neue Bewertung):
-- `vendor_cli` → `subscription`, gemeldete Kosten → `harness`, sonst `none`. Aus Dateien
-- importierte Sessions haben keine Usage (DATA-010).
-- Gegenstück: migrations/postgres/0009_usage_records.sql.

CREATE TABLE usage_records (
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    seq INTEGER NOT NULL,
    ts TEXT NOT NULL,
    day TEXT NOT NULL,
    project_id TEXT,
    user_id TEXT NOT NULL,
    harness TEXT NOT NULL,
    model TEXT NOT NULL,
    auth_source TEXT NOT NULL,
    purpose TEXT,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    cost_micro INTEGER,
    cost_source TEXT NOT NULL,
    pricing_version TEXT,
    equivalent_micro INTEGER,
    PRIMARY KEY (session_id, seq)
);
CREATE INDEX usage_records_org_ts ON usage_records (org_id, ts);
CREATE INDEX usage_records_org_user_ts ON usage_records (org_id, user_id, ts);

INSERT INTO usage_records (org_id, session_id, seq, ts, day, project_id, user_id, harness,
    model, auth_source, purpose, input_tokens, output_tokens, cache_read_tokens,
    cache_write_tokens, cost_micro, cost_source, pricing_version, equivalent_micro)
SELECT e.org_id, e.session_id, e.seq, e.ts, substr(e.ts, 1, 10), s.project_id, s.owner_id,
    json_extract(e.payload, '$.harness'),
    json_extract(e.payload, '$.model'),
    json_extract(e.payload, '$.auth_source'),
    json_extract(e.payload, '$.purpose'),
    COALESCE(json_extract(e.payload, '$.input_tokens'), 0),
    COALESCE(json_extract(e.payload, '$.output_tokens'), 0),
    COALESCE(json_extract(e.payload, '$.cache_read_tokens'), 0),
    COALESCE(json_extract(e.payload, '$.cache_write_tokens'), 0),
    CASE WHEN json_extract(e.payload, '$.auth_source') = 'vendor_cli' THEN NULL
         ELSE json_extract(e.payload, '$.cost_micro') END,
    CASE WHEN json_extract(e.payload, '$.auth_source') = 'vendor_cli' THEN 'subscription'
         WHEN json_extract(e.payload, '$.cost_micro') IS NOT NULL THEN 'harness'
         ELSE 'none' END,
    NULL,
    NULL
FROM events e
JOIN sessions s ON s.id = e.session_id AND s.org_id = e.org_id
WHERE e.type = 'cost.delta' AND e.payload IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM session_file_imports f WHERE f.session_id = e.session_id);
