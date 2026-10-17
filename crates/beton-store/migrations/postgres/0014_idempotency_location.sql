-- 0014_idempotency_location: `Location` einer gespeicherten 201-Antwort für das Replay
-- (PROTO-010 AC2, #72). Gegenstück: migrations/sqlite/0014_idempotency_location.sql.

ALTER TABLE idempotency_keys ADD COLUMN location TEXT;
